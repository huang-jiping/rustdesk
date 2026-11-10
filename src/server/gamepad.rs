use base::message_proto::{GamepadInput, GamepadStatusKind};
use hbb_common::{anyhow::Context, bail, ResultType};
use std::{
    sync::{mpsc, Arc, Condvar, Mutex, Weak},
    time::Duration,
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct GamepadState {
    pub connected: bool,
    pub buttons: u16,
    pub left_trigger: u8,
    pub right_trigger: u8,
    pub thumb_lx: i16,
    pub thumb_ly: i16,
    pub thumb_rx: i16,
    pub thumb_ry: i16,
}

impl GamepadState {
    pub fn try_from_input(input: &GamepadInput) -> ResultType<Self> {
        if input.slot != 0 {
            bail!("Unsupported gamepad slot: {}", input.slot);
        }
        if input.buttons & !0xf3ff != 0 {
            bail!("Undefined gamepad button bits: {:#x}", input.buttons);
        }
        Ok(Self {
            connected: input.connected,
            buttons: input.buttons as u16,
            left_trigger: u8::try_from(input.left_trigger).context("Invalid left trigger")?,
            right_trigger: u8::try_from(input.right_trigger).context("Invalid right trigger")?,
            thumb_lx: i16::try_from(input.thumb_lx).context("Invalid left X axis")?,
            thumb_ly: i16::try_from(input.thumb_ly).context("Invalid left Y axis")?,
            thumb_rx: i16::try_from(input.thumb_rx).context("Invalid right X axis")?,
            thumb_ry: i16::try_from(input.thumb_ry).context("Invalid right Y axis")?,
        })
    }
}

pub(crate) trait GamepadBackend: Send {
    fn attach(&mut self, slot: u8) -> ResultType<()>;
    fn update(&mut self, slot: u8, state: &GamepadState) -> ResultType<()>;
    fn detach(&mut self, slot: u8) -> ResultType<()>;
    fn reset_all(&mut self);
}

enum WorkerCommand {
    Input(GamepadState),
    Stop(GamepadStatusKind),
}

#[derive(Default)]
struct MailboxState {
    command: Option<WorkerCommand>,
    stopped: bool,
}

#[derive(Default)]
struct GamepadMailbox {
    state: Mutex<MailboxState>,
    wake: Condvar,
}

impl GamepadMailbox {
    fn input(&self, input: GamepadState) -> ResultType<()> {
        let mut state = self.state.lock().unwrap();
        if state.stopped {
            bail!("Gamepad session is stopped");
        }
        state.command = Some(WorkerCommand::Input(input));
        self.wake.notify_one();
        Ok(())
    }

    fn stop(&self, final_status: GamepadStatusKind) {
        let mut state = self.state.lock().unwrap();
        if !state.stopped {
            state.stopped = true;
            state.command = Some(WorkerCommand::Stop(final_status));
            self.wake.notify_one();
        }
    }

    fn next(&self) -> WorkerCommand {
        let mut state = self.state.lock().unwrap();
        loop {
            if let Some(command) = state.command.take() {
                return command;
            }
            state = self.wake.wait(state).unwrap();
        }
    }
}

static GAMEPAD_OWNER: Mutex<Option<Weak<GamepadMailbox>>> = Mutex::new(None);

pub(crate) struct GamepadOwnership(());

impl GamepadOwnership {
    fn acquire(mailbox: &Arc<GamepadMailbox>) -> Option<Self> {
        let mut owner = GAMEPAD_OWNER.lock().unwrap();
        if owner.is_some() {
            return None;
        }
        *owner = Some(Arc::downgrade(mailbox));
        Some(Self(()))
    }
}

impl Drop for GamepadOwnership {
    fn drop(&mut self) {
        *GAMEPAD_OWNER.lock().unwrap() = None;
    }
}

pub(crate) fn revoke_active_gamepad() {
    let mailbox = GAMEPAD_OWNER
        .lock()
        .unwrap()
        .as_ref()
        .and_then(Weak::upgrade);
    if let Some(mailbox) = mailbox {
        mailbox.stop(GamepadStatusKind::DISABLED_BY_REMOTE);
    }
}

#[derive(Default)]
pub(crate) struct GamepadSession {
    mailbox: Mutex<Option<Arc<GamepadMailbox>>>,
}

impl GamepadSession {
    pub fn enable(
        &self,
        status_sender: impl Fn(GamepadStatusKind) + Send + 'static,
        backend_factory: impl FnOnce() -> Result<Box<dyn GamepadBackend>, GamepadStatusKind>
            + Send
            + 'static,
    ) -> GamepadStatusKind {
        let mailbox = Arc::new(GamepadMailbox::default());
        let ownership = {
            let mut current = self.mailbox.lock().unwrap();
            let Some(ownership) = GamepadOwnership::acquire(&mailbox) else {
                return GamepadStatusKind::BUSY;
            };
            *current = Some(Arc::clone(&mailbox));
            ownership
        };
        let (initialized, result) = mpsc::sync_channel(1);
        let worker_mailbox = Arc::clone(&mailbox);
        let worker = std::thread::Builder::new()
            .name("gamepad".into())
            .spawn(move || {
                let backend = match backend_factory() {
                    Ok(backend) => backend,
                    Err(kind) => {
                        worker_mailbox.stop(kind);
                        drop(ownership);
                        if initialized.send(kind).is_err() {
                            hbb_common::log::trace!("Gamepad initialization receiver closed");
                        }
                        return;
                    }
                };
                let mut worker = GamepadWorker {
                    backend,
                    _ownership: ownership,
                    mailbox: worker_mailbox,
                };
                let ready = {
                    let state = worker.mailbox.state.lock().unwrap();
                    !state.stopped && initialized.send(GamepadStatusKind::READY).is_ok()
                };
                if !ready {
                    worker.mailbox.stop(GamepadStatusKind::STOPPED);
                }
                let final_status = worker.run(&status_sender);
                drop(worker);
                status_sender(final_status);
                if !ready && initialized.send(final_status).is_err() {
                    hbb_common::log::trace!("Gamepad initialization receiver closed");
                }
            });
        if let Err(error) = worker {
            log_gamepad_error(&error);
            mailbox.stop(GamepadStatusKind::INTERNAL_ERROR);
            return GamepadStatusKind::INTERNAL_ERROR;
        }
        match result.recv() {
            Ok(kind) => kind,
            Err(error) => {
                log_gamepad_error(&error);
                mailbox.stop(GamepadStatusKind::INTERNAL_ERROR);
                GamepadStatusKind::INTERNAL_ERROR
            }
        }
    }

    pub fn handle_input(&self, input: GamepadInput) -> ResultType<()> {
        let state = GamepadState::try_from_input(&input)?;
        let mailbox = self.mailbox.lock().unwrap();
        match mailbox.as_ref() {
            Some(mailbox) => mailbox.input(state),
            None => bail!("Gamepad session is not enabled"),
        }
    }

    pub fn disable(&self, final_status: GamepadStatusKind) {
        if let Some(mailbox) = self.mailbox.lock().unwrap().as_ref() {
            mailbox.stop(final_status);
        }
    }
}

impl Drop for GamepadSession {
    fn drop(&mut self) {
        self.disable(GamepadStatusKind::STOPPED);
    }
}

struct GamepadWorker {
    // Field drop order keeps ownership until the backend client is dropped.
    backend: Box<dyn GamepadBackend>,
    _ownership: GamepadOwnership,
    mailbox: Arc<GamepadMailbox>,
}

impl GamepadWorker {
    fn run(&mut self, status_sender: &impl Fn(GamepadStatusKind)) -> GamepadStatusKind {
        let mut attached = false;
        let final_status = loop {
            let result = match self.mailbox.next() {
                WorkerCommand::Stop(kind) => break kind,
                WorkerCommand::Input(state) if state.connected => {
                    let first_report = !attached;
                    let attach = if first_report {
                        attached = true;
                        self.backend.attach(0)
                    } else {
                        Ok(())
                    };
                    attach
                        .and_then(|_| self.backend.update(0, &state))
                        .map(|_| {
                            if first_report {
                                status_sender(GamepadStatusKind::ACTIVE);
                            }
                        })
                }
                WorkerCommand::Input(_) => {
                    let was_attached = attached;
                    self.clear_target(&mut attached).map(|_| {
                        if was_attached {
                            status_sender(GamepadStatusKind::READY);
                        }
                    })
                }
            };
            if let Err(error) = result {
                log_gamepad_error(&error);
                break GamepadStatusKind::INTERNAL_ERROR;
            }
        };
        self.mailbox.stop(final_status);
        if let Err(error) = self.clear_target(&mut attached) {
            log_gamepad_error(&error);
        }
        final_status
    }

    fn clear_target(&mut self, attached: &mut bool) -> ResultType<()> {
        if !std::mem::take(attached) {
            return Ok(());
        }
        let neutral = self.backend.update(0, &GamepadState::default());
        let detach = self.backend.detach(0);
        if neutral.is_err() {
            if let Err(error) = &detach {
                log_gamepad_error(error);
            }
        }
        neutral.and(detach)
    }
}

impl Drop for GamepadWorker {
    fn drop(&mut self) {
        self.mailbox.stop(GamepadStatusKind::INTERNAL_ERROR);
        self.backend.reset_all();
    }
}

fn log_gamepad_error(error: &dyn std::fmt::Display) {
    hbb_common::throttled_log!(
        Duration::from_secs(1),
        warn,
        "Gamepad worker failed: {}",
        error
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use base::message_proto::GamepadStatusKind;
    use std::{
        sync::{mpsc, Mutex},
        time::Duration,
    };

    static SESSION_TEST: Mutex<()> = Mutex::new(());

    #[derive(Debug, PartialEq)]
    enum Event {
        Attach(u8),
        Update(u8, GamepadState),
        Detach(u8),
        Reset,
        Dropping,
        Dropped,
        Status(GamepadStatusKind),
    }

    struct MockBackend {
        events: mpsc::SyncSender<Event>,
        fail_update: bool,
        update_gate: Option<mpsc::Receiver<()>>,
        drop_gate: Option<mpsc::Receiver<()>>,
    }

    impl GamepadBackend for MockBackend {
        fn attach(&mut self, slot: u8) -> ResultType<()> {
            self.events.send(Event::Attach(slot)).unwrap();
            Ok(())
        }

        fn update(&mut self, slot: u8, state: &GamepadState) -> ResultType<()> {
            self.events.send(Event::Update(slot, *state)).unwrap();
            if let Some(gate) = self.update_gate.take() {
                gate.recv_timeout(Duration::from_secs(5)).unwrap();
            }
            if std::mem::take(&mut self.fail_update) {
                bail!("mock update failed");
            }
            Ok(())
        }

        fn detach(&mut self, slot: u8) -> ResultType<()> {
            self.events.send(Event::Detach(slot)).unwrap();
            Ok(())
        }

        fn reset_all(&mut self) {
            self.events.send(Event::Reset).unwrap();
        }
    }

    impl Drop for MockBackend {
        fn drop(&mut self) {
            self.events.send(Event::Dropping).unwrap();
            if let Some(gate) = self.drop_gate.take() {
                gate.recv_timeout(Duration::from_secs(5)).unwrap();
            }
            self.events.send(Event::Dropped).unwrap();
        }
    }

    fn mock_backend(events: &mpsc::SyncSender<Event>) -> MockBackend {
        MockBackend {
            events: events.clone(),
            fail_update: false,
            update_gate: None,
            drop_gate: None,
        }
    }

    fn enable(
        session: &GamepadSession,
        events: &mpsc::SyncSender<Event>,
        backend: MockBackend,
    ) -> GamepadStatusKind {
        let events = events.clone();
        let caller = std::thread::current().id();
        session.enable(
            move |kind| events.send(Event::Status(kind)).unwrap(),
            move || {
                assert_ne!(std::thread::current().id(), caller);
                Ok(Box::new(backend) as Box<dyn GamepadBackend>)
            },
        )
    }

    fn expect_events(receiver: &mpsc::Receiver<Event>, expected: &[Event]) {
        for event in expected {
            assert_eq!(
                receiver.recv_timeout(Duration::from_secs(5)).unwrap(),
                *event
            );
        }
    }

    fn connected_input(value: i32) -> GamepadInput {
        GamepadInput {
            connected: true,
            buttons: 0x1000,
            left_trigger: 17,
            right_trigger: 255,
            thumb_lx: value,
            thumb_ly: -1234,
            thumb_rx: 32767,
            thumb_ry: -32768,
            ..Default::default()
        }
    }

    #[test]
    fn session_keeps_single_owner_until_backend_drop_finishes() {
        let _serial = SESSION_TEST.lock().unwrap();
        let (events, receiver) = mpsc::sync_channel(32);
        let (release, gate) = mpsc::sync_channel(1);
        let session = GamepadSession::default();
        let mut backend = mock_backend(&events);
        backend.drop_gate = Some(gate);
        assert_eq!(enable(&session, &events, backend), GamepadStatusKind::READY);
        let other = GamepadSession::default();
        assert_eq!(
            other.enable(|_| {}, || panic!("busy factory must not run")),
            GamepadStatusKind::BUSY
        );
        session.handle_input(connected_input(42)).unwrap();
        expect_events(
            &receiver,
            &[
                Event::Attach(0),
                Event::Update(
                    0,
                    GamepadState::try_from_input(&connected_input(42)).unwrap(),
                ),
                Event::Status(GamepadStatusKind::ACTIVE),
            ],
        );
        session.handle_input(GamepadInput::default()).unwrap();
        expect_events(
            &receiver,
            &[
                Event::Update(0, GamepadState::default()),
                Event::Detach(0),
                Event::Status(GamepadStatusKind::READY),
            ],
        );
        assert_eq!(
            other.enable(|_| {}, || panic!("disconnect must retain owner")),
            GamepadStatusKind::BUSY
        );
        session.handle_input(connected_input(43)).unwrap();
        expect_events(
            &receiver,
            &[
                Event::Attach(0),
                Event::Update(
                    0,
                    GamepadState::try_from_input(&connected_input(43)).unwrap(),
                ),
                Event::Status(GamepadStatusKind::ACTIVE),
            ],
        );
        session.disable(GamepadStatusKind::STOPPED);
        expect_events(
            &receiver,
            &[
                Event::Update(0, GamepadState::default()),
                Event::Detach(0),
                Event::Reset,
                Event::Dropping,
            ],
        );
        assert_eq!(
            other.enable(|_| {}, || panic!("cleanup must retain owner")),
            GamepadStatusKind::BUSY
        );
        assert!(session.handle_input(connected_input(44)).is_err());
        release.send(()).unwrap();
        expect_events(
            &receiver,
            &[Event::Dropped, Event::Status(GamepadStatusKind::STOPPED)],
        );
        assert_eq!(
            enable(&other, &events, mock_backend(&events)),
            GamepadStatusKind::READY
        );
        other.disable(GamepadStatusKind::STOPPED);
        expect_events(
            &receiver,
            &[
                Event::Reset,
                Event::Dropping,
                Event::Dropped,
                Event::Status(GamepadStatusKind::STOPPED),
            ],
        );
    }

    #[test]
    fn initialization_and_update_errors_release_owner_after_cleanup() {
        let _serial = SESSION_TEST.lock().unwrap();
        let (events, receiver) = mpsc::sync_channel(32);
        let session = GamepadSession::default();
        assert_eq!(
            session.enable(
                |_| panic!("initialization failure is returned directly"),
                || Err(GamepadStatusKind::DRIVER_MISSING),
            ),
            GamepadStatusKind::DRIVER_MISSING
        );
        assert!(session.handle_input(connected_input(42)).is_err());
        let mut backend = mock_backend(&events);
        backend.fail_update = true;
        assert_eq!(enable(&session, &events, backend), GamepadStatusKind::READY);
        session.handle_input(connected_input(42)).unwrap();
        expect_events(
            &receiver,
            &[
                Event::Attach(0),
                Event::Update(
                    0,
                    GamepadState::try_from_input(&connected_input(42)).unwrap(),
                ),
                Event::Update(0, GamepadState::default()),
                Event::Detach(0),
                Event::Reset,
                Event::Dropping,
                Event::Dropped,
                Event::Status(GamepadStatusKind::INTERNAL_ERROR),
            ],
        );
        assert_eq!(
            enable(&session, &events, mock_backend(&events)),
            GamepadStatusKind::READY
        );
        session.disable(GamepadStatusKind::STOPPED);
        expect_events(
            &receiver,
            &[
                Event::Reset,
                Event::Dropping,
                Event::Dropped,
                Event::Status(GamepadStatusKind::STOPPED),
            ],
        );
    }

    #[test]
    fn mailbox_overwrites_a_thousand_states_and_worker_observes_last_complete_report() {
        let _serial = SESSION_TEST.lock().unwrap();
        let (events, receiver) = mpsc::sync_channel(32);
        let (release, gate) = mpsc::sync_channel(1);
        let session = GamepadSession::default();
        let mut backend = mock_backend(&events);
        backend.update_gate = Some(gate);
        assert_eq!(enable(&session, &events, backend), GamepadStatusKind::READY);
        session.handle_input(connected_input(0)).unwrap();
        expect_events(
            &receiver,
            &[
                Event::Attach(0),
                Event::Update(
                    0,
                    GamepadState::try_from_input(&connected_input(0)).unwrap(),
                ),
            ],
        );
        for value in 1..=1000 {
            session
                .handle_input(GamepadInput {
                    connected: true,
                    buttons: value as u32,
                    left_trigger: value as u32 % 256,
                    right_trigger: (value as u32 + 17) % 256,
                    thumb_lx: value,
                    thumb_ly: -value,
                    thumb_rx: value * 2,
                    thumb_ry: -value * 2,
                    ..Default::default()
                })
                .unwrap();
            let mailbox = session.mailbox.lock().unwrap();
            let state = mailbox.as_ref().unwrap().state.lock().unwrap();
            assert!(matches!(state.command, Some(WorkerCommand::Input(_))));
        }
        release.send(()).unwrap();
        expect_events(
            &receiver,
            &[
                Event::Status(GamepadStatusKind::ACTIVE),
                Event::Update(
                    0,
                    GamepadState {
                        connected: true,
                        buttons: 1000,
                        left_trigger: 232,
                        right_trigger: 249,
                        thumb_lx: 1000,
                        thumb_ly: -1000,
                        thumb_rx: 2000,
                        thumb_ry: -2000,
                    },
                ),
            ],
        );
        session.disable(GamepadStatusKind::STOPPED);
        expect_events(
            &receiver,
            &[
                Event::Update(0, GamepadState::default()),
                Event::Detach(0),
                Event::Reset,
                Event::Dropping,
                Event::Dropped,
                Event::Status(GamepadStatusKind::STOPPED),
            ],
        );
    }

    #[test]
    fn revoke_and_session_drop_stop_and_wake_idle_worker() {
        let _serial = SESSION_TEST.lock().unwrap();
        let (events, receiver) = mpsc::sync_channel(32);
        let session = GamepadSession::default();
        assert_eq!(
            enable(&session, &events, mock_backend(&events)),
            GamepadStatusKind::READY
        );
        revoke_active_gamepad();
        expect_events(
            &receiver,
            &[
                Event::Reset,
                Event::Dropping,
                Event::Dropped,
                Event::Status(GamepadStatusKind::DISABLED_BY_REMOTE),
            ],
        );
        assert_eq!(
            enable(&session, &events, mock_backend(&events)),
            GamepadStatusKind::READY
        );
        drop(session);
        expect_events(
            &receiver,
            &[
                Event::Reset,
                Event::Dropping,
                Event::Dropped,
                Event::Status(GamepadStatusKind::STOPPED),
            ],
        );
    }

    #[test]
    fn revoke_during_initialization_returns_disabled_after_cleanup() {
        let _serial = SESSION_TEST.lock().unwrap();
        let (events, receiver) = mpsc::sync_channel(32);
        let (started, waiting) = mpsc::sync_channel(1);
        let (release, gate) = mpsc::sync_channel(1);
        let session = Arc::new(GamepadSession::default());
        let enabling = Arc::clone(&session);
        let backend = mock_backend(&events);
        let thread = std::thread::spawn(move || {
            enabling.enable(
                move |kind| events.send(Event::Status(kind)).unwrap(),
                move || {
                    started.send(()).unwrap();
                    gate.recv_timeout(Duration::from_secs(5)).unwrap();
                    Ok(Box::new(backend) as Box<dyn GamepadBackend>)
                },
            )
        });
        waiting.recv_timeout(Duration::from_secs(5)).unwrap();
        revoke_active_gamepad();
        assert!(session.handle_input(connected_input(42)).is_err());
        release.send(()).unwrap();
        expect_events(
            &receiver,
            &[
                Event::Reset,
                Event::Dropping,
                Event::Dropped,
                Event::Status(GamepadStatusKind::DISABLED_BY_REMOTE),
            ],
        );
        assert_eq!(
            thread.join().unwrap(),
            GamepadStatusKind::DISABLED_BY_REMOTE
        );
    }

    #[test]
    fn accepts_xinput_boundary_values() {
        let input = GamepadInput {
            connected: true,
            buttons: 0xf3ff,
            left_trigger: 0,
            right_trigger: 255,
            thumb_lx: -32768,
            thumb_ly: 32767,
            thumb_rx: 32767,
            thumb_ry: -32768,
            ..Default::default()
        };
        let state = GamepadState::try_from_input(&input).unwrap();
        assert_eq!(state.buttons, 0xf3ff);
        assert_eq!((state.left_trigger, state.right_trigger), (0, 255));
        assert_eq!((state.thumb_lx, state.thumb_ly), (-32768, 32767));
        assert_eq!((state.thumb_rx, state.thumb_ry), (32767, -32768));
        assert!(state.connected);
        let state = GamepadState::try_from_input(&GamepadInput::default()).unwrap();
        assert_eq!(state, GamepadState::default());
    }

    #[test]
    fn rejects_nonzero_slot() {
        for slot in [1, 3, u32::MAX] {
            let input = GamepadInput {
                slot,
                ..Default::default()
            };
            assert!(GamepadState::try_from_input(&input).is_err());
        }
    }

    #[test]
    fn rejects_trigger_above_255() {
        for (left_trigger, right_trigger) in [(256, 0), (0, 256), (u32::MAX, u32::MAX)] {
            let input = GamepadInput {
                left_trigger,
                right_trigger,
                ..Default::default()
            };
            assert!(GamepadState::try_from_input(&input).is_err());
        }
    }

    #[test]
    fn rejects_axis_out_of_range() {
        for value in [-32769, 32768, i32::MIN, i32::MAX] {
            for axis in 0..4 {
                let mut input = GamepadInput::default();
                match axis {
                    0 => input.thumb_lx = value,
                    1 => input.thumb_ly = value,
                    2 => input.thumb_rx = value,
                    _ => input.thumb_ry = value,
                }
                assert!(GamepadState::try_from_input(&input).is_err());
            }
        }
    }

    #[test]
    fn rejects_undefined_button_bits() {
        for buttons in [0x0400, 0x0800, 0x10000, u32::MAX] {
            let input = GamepadInput {
                buttons,
                ..Default::default()
            };
            assert!(GamepadState::try_from_input(&input).is_err());
        }
    }
}
