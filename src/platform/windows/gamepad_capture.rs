use base::message_proto::GamepadInput;
use hbb_common::{tokio::sync::watch, ResultType};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

pub(crate) struct XInputState {
    pub packet_number: u32,
    pub input: GamepadInput,
}

pub(crate) trait XInputProvider {
    fn get_state(&self, user_index: u32) -> Option<XInputState>;
}

#[derive(Default)]
struct CaptureSampler {
    last_packet: Option<Option<u32>>,
    stopped: bool,
}

impl CaptureSampler {
    fn poll(&mut self, provider: &impl XInputProvider) -> Option<GamepadInput> {
        if self.stopped {
            return None;
        }
        let state = provider.get_state(0);
        let packet = state.as_ref().map(|state| state.packet_number);
        if self.last_packet == Some(packet) {
            return None;
        }
        self.last_packet = Some(packet);
        Some(state.map(|state| state.input).unwrap_or_default())
    }

    fn stop(&mut self) -> Option<GamepadInput> {
        if self.stopped {
            return None;
        }
        self.stopped = true;
        Some(GamepadInput::default())
    }
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
struct WindowsXInput;

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
impl XInputProvider for WindowsXInput {
    fn get_state(&self, user_index: u32) -> Option<XInputState> {
        use winapi::{
            shared::winerror::ERROR_DEVICE_NOT_CONNECTED,
            um::xinput::{XInputGetState, XINPUT_STATE},
        };
        let mut state: XINPUT_STATE = unsafe { std::mem::zeroed() };
        let result = unsafe { XInputGetState(user_index, &mut state) };
        if result != 0 {
            if result != ERROR_DEVICE_NOT_CONNECTED {
                hbb_common::throttled_log!(
                    Duration::from_secs(5),
                    warn,
                    "XInputGetState failed: {}",
                    result
                );
            }
            return None;
        }
        let pad = state.Gamepad;
        Some(XInputState {
            packet_number: state.dwPacketNumber,
            input: GamepadInput {
                connected: true,
                buttons: (pad.wButtons & 0xf3ff) as u32,
                left_trigger: pad.bLeftTrigger as u32,
                right_trigger: pad.bRightTrigger as u32,
                thumb_lx: pad.sThumbLX as i32,
                thumb_ly: pad.sThumbLY as i32,
                thumb_rx: pad.sThumbRX as i32,
                thumb_ry: pad.sThumbRY as i32,
                ..Default::default()
            },
        })
    }
}

pub(crate) struct GamepadCapture {
    stopped: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl GamepadCapture {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    pub fn start(sender: watch::Sender<Option<GamepadInput>>) -> ResultType<Self> {
        Self::start_with_provider(sender, WindowsXInput)
    }

    fn start_with_provider(
        sender: watch::Sender<Option<GamepadInput>>,
        provider: impl XInputProvider + Send + 'static,
    ) -> ResultType<Self> {
        let stopped = Arc::new(AtomicBool::new(false));
        let worker_stopped = Arc::clone(&stopped);
        let worker = thread::Builder::new()
            .name("gamepad-capture".into())
            .spawn(move || {
                let mut sampler = CaptureSampler::default();
                while !worker_stopped.load(Ordering::Acquire) && !sender.is_closed() {
                    if let Some(input) = sampler.poll(&provider) {
                        sender.send_replace(Some(input));
                    }
                    thread::park_timeout(Duration::from_millis(8));
                }
                if let Some(input) = sampler.stop() {
                    sender.send_replace(Some(input));
                }
            })?;
        Ok(Self {
            stopped,
            worker: Some(worker),
        })
    }

    pub fn stop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.thread().unpark();
            if worker.join().is_err() {
                hbb_common::log::error!("Gamepad capture thread panicked");
            }
        }
    }
}

impl Drop for GamepadCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::VecDeque};

    struct FakeProvider(RefCell<VecDeque<Option<XInputState>>>);

    impl XInputProvider for FakeProvider {
        fn get_state(&self, user_index: u32) -> Option<XInputState> {
            assert_eq!(user_index, 0);
            self.0.borrow_mut().pop_front().expect("poll fixture")
        }
    }

    fn connected(packet_number: u32, buttons: u32) -> Option<XInputState> {
        Some(XInputState {
            packet_number,
            input: GamepadInput {
                connected: true,
                buttons,
                left_trigger: 128,
                right_trigger: 255,
                thumb_lx: -32768,
                thumb_ly: 32767,
                thumb_rx: -12,
                thumb_ry: 34,
                ..Default::default()
            },
        })
    }

    #[test]
    fn gamepad_capture_reports_changes_disconnect_and_replug() {
        let provider = FakeProvider(RefCell::new(VecDeque::from([
            connected(0, 1),
            connected(0, 1),
            connected(1, 2),
            None,
            None,
            connected(0, 4),
        ])));
        let mut sampler = CaptureSampler::default();
        let first = sampler.poll(&provider).expect("first connection snapshot");
        assert_eq!((first.slot, first.connected, first.buttons), (0, true, 1));
        assert_eq!((first.left_trigger, first.right_trigger), (128, 255));
        assert_eq!(
            (
                first.thumb_lx,
                first.thumb_ly,
                first.thumb_rx,
                first.thumb_ry
            ),
            (-32768, 32767, -12, 34)
        );
        assert!(sampler.poll(&provider).is_none());
        assert_eq!(sampler.poll(&provider).unwrap().buttons, 2);
        assert_eq!(sampler.poll(&provider).unwrap(), GamepadInput::default());
        assert!(sampler.poll(&provider).is_none());
        assert_eq!(sampler.poll(&provider).unwrap().buttons, 4);
    }

    #[test]
    fn gamepad_capture_stop_emits_final_disconnect_once() {
        let provider = FakeProvider(RefCell::new(VecDeque::from([connected(0, 1)])));
        let mut sampler = CaptureSampler::default();
        assert!(sampler.poll(&provider).is_some());
        assert_eq!(sampler.stop(), Some(GamepadInput::default()));
        assert!(sampler.stop().is_none());
        assert!(sampler.poll(&provider).is_none());
    }

    #[test]
    fn gamepad_capture_worker_keeps_latest_and_stops_idempotently() {
        use std::sync::{atomic::AtomicU32, mpsc};
        struct AdvancingProvider {
            packet: AtomicU32,
            sampled: mpsc::SyncSender<()>,
        }
        impl XInputProvider for AdvancingProvider {
            fn get_state(&self, user_index: u32) -> Option<XInputState> {
                assert_eq!(user_index, 0);
                let packet = self.packet.fetch_add(1, Ordering::Relaxed) + 1;
                if packet == 4 {
                    self.sampled.send(()).unwrap();
                }
                connected(packet, packet)
            }
        }
        let (sampled, received) = mpsc::sync_channel(1);
        let (sender, mut receiver) = watch::channel(None);
        let mut capture = GamepadCapture::start_with_provider(
            sender,
            AdvancingProvider {
                packet: AtomicU32::new(0),
                sampled,
            },
        )
        .unwrap();
        received.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(receiver.borrow().as_ref().unwrap().buttons >= 3);
        capture.stop();
        assert_eq!(*receiver.borrow_and_update(), Some(GamepadInput::default()));
        capture.stop();
        assert!(!receiver.has_changed().unwrap_or(false));
        drop(capture);
    }
}
