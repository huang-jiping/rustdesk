use crate::server::gamepad::{GamepadBackend, GamepadState};
use base::message_proto::GamepadStatusKind;
use hbb_common::ResultType;
use std::{error::Error, fmt, sync::Arc};
use vigem_client::{Client, TargetId, XGamepad, Xbox360Wired};

#[derive(Debug)]
pub(crate) struct GamepadBackendError {
    pub kind: GamepadStatusKind,
}

impl fmt::Display for GamepadBackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Gamepad backend: {:?}", self.kind)
    }
}

impl Error for GamepadBackendError {}

impl From<vigem_client::Error> for GamepadBackendError {
    fn from(error: vigem_client::Error) -> Self {
        Self {
            kind: map_vigem_error(error),
        }
    }
}

fn map_vigem_error(error: vigem_client::Error) -> GamepadStatusKind {
    match error {
        vigem_client::Error::BusNotFound => GamepadStatusKind::DRIVER_MISSING,
        vigem_client::Error::BusVersionMismatch => GamepadStatusKind::VERSION_MISMATCH,
        vigem_client::Error::NoFreeSlot => GamepadStatusKind::NO_FREE_SLOT,
        vigem_client::Error::BusAccessFailed(5) | vigem_client::Error::WinError(5) => {
            GamepadStatusKind::ACCESS_DENIED
        }
        _ => GamepadStatusKind::INTERNAL_ERROR,
    }
}

fn to_xgamepad(state: &GamepadState) -> XGamepad {
    XGamepad {
        buttons: state.buttons.into(),
        left_trigger: state.left_trigger,
        right_trigger: state.right_trigger,
        thumb_lx: state.thumb_lx,
        thumb_ly: state.thumb_ly,
        thumb_rx: state.thumb_rx,
        thumb_ry: state.thumb_ry,
    }
}

fn submit_report(
    report: &XGamepad,
    mut submit: impl FnMut(&XGamepad) -> Result<(), vigem_client::Error>,
) -> Result<(), vigem_client::Error> {
    match submit(report) {
        // The driver drops this report while its USB input queue has no pending request.
        Err(vigem_client::Error::WinError(259)) => {
            std::thread::sleep(std::time::Duration::from_millis(1));
            submit(report)
        }
        result => result,
    }
}

pub(crate) struct VigemBackend {
    client: Arc<Client>,
    target: Option<Xbox360Wired<Arc<Client>>>,
}

impl VigemBackend {
    pub fn connect() -> Result<Self, GamepadBackendError> {
        Ok(Self {
            client: Arc::new(Client::connect().map_err(GamepadBackendError::from)?),
            target: None,
        })
    }

    fn check_slot(slot: u8) -> Result<(), GamepadBackendError> {
        if slot != 0 {
            return Err(GamepadBackendError {
                kind: GamepadStatusKind::INTERNAL_ERROR,
            });
        }
        Ok(())
    }

    fn clear_target(&mut self) -> Result<(), GamepadBackendError> {
        let Some(mut target) = self.target.take() else {
            return Ok(());
        };
        let neutral = submit_report(&XGamepad::default(), |report| target.update(report));
        let unplug = target.unplug();
        if let Err(error) = unplug {
            if neutral.is_err() {
                hbb_common::throttled_log!(
                    std::time::Duration::from_secs(1),
                    warn,
                    "Gamepad unplug failed: {}",
                    error
                );
            }
        }
        neutral.and(unplug).map_err(GamepadBackendError::from)
    }
}

pub(crate) fn create_gamepad_backend() -> Result<Box<dyn GamepadBackend>, GamepadBackendError> {
    Ok(Box::new(VigemBackend::connect()?))
}

impl GamepadBackend for VigemBackend {
    fn attach(&mut self, slot: u8) -> ResultType<()> {
        Self::check_slot(slot)?;
        if self.target.is_some() {
            return Ok(());
        }
        let mut target = Xbox360Wired::new(Arc::clone(&self.client), TargetId::XBOX360_WIRED);
        target.plugin().map_err(GamepadBackendError::from)?;
        self.target = Some(target);
        if let Some(target) = self.target.as_mut() {
            if let Err(error) = target.wait_ready() {
                self.reset_all();
                return Err(GamepadBackendError::from(error).into());
            }
        }
        Ok(())
    }

    fn update(&mut self, slot: u8, state: &GamepadState) -> ResultType<()> {
        Self::check_slot(slot)?;
        let result = match self.target.as_mut() {
            Some(target) => submit_report(&to_xgamepad(state), |report| target.update(report)),
            None => Err(vigem_client::Error::NotPluggedIn),
        };
        if let Err(error) = result {
            self.reset_all();
            return Err(GamepadBackendError::from(error).into());
        }
        Ok(())
    }

    fn detach(&mut self, slot: u8) -> ResultType<()> {
        Self::check_slot(slot)?;
        self.clear_target()?;
        Ok(())
    }

    fn reset_all(&mut self) {
        if let Err(error) = self.clear_target() {
            hbb_common::throttled_log!(
                std::time::Duration::from_secs(1),
                warn,
                "Gamepad cleanup failed: {}",
                error
            );
        }
    }
}

impl Drop for VigemBackend {
    fn drop(&mut self) {
        self.reset_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::gamepad::GamepadState;
    use base::message_proto::GamepadStatusKind;

    #[test]
    fn maps_complete_xbox_report() {
        let state = GamepadState {
            connected: true,
            buttons: 0xa139,
            left_trigger: 17,
            right_trigger: 255,
            thumb_lx: -32768,
            thumb_ly: 32767,
            thumb_rx: -1234,
            thumb_ry: 5678,
        };
        let report = to_xgamepad(&state);
        assert_eq!(report.buttons.raw, 0xa139);
        assert_eq!((report.left_trigger, report.right_trigger), (17, 255));
        assert_eq!((report.thumb_lx, report.thumb_ly), (-32768, 32767));
        assert_eq!((report.thumb_rx, report.thumb_ry), (-1234, 5678));
        let neutral = to_xgamepad(&GamepadState::default());
        assert_eq!(neutral, vigem_client::XGamepad::default());
    }

    #[test]
    fn maps_backend_failures_to_stable_status() {
        for (error, expected) in [
            (
                vigem_client::Error::BusNotFound,
                GamepadStatusKind::DRIVER_MISSING,
            ),
            (
                vigem_client::Error::BusVersionMismatch,
                GamepadStatusKind::VERSION_MISMATCH,
            ),
            (
                vigem_client::Error::NoFreeSlot,
                GamepadStatusKind::NO_FREE_SLOT,
            ),
            (
                vigem_client::Error::BusAccessFailed(5),
                GamepadStatusKind::ACCESS_DENIED,
            ),
            (
                vigem_client::Error::WinError(5),
                GamepadStatusKind::ACCESS_DENIED,
            ),
            (
                vigem_client::Error::BusAccessFailed(6),
                GamepadStatusKind::INTERNAL_ERROR,
            ),
            (
                vigem_client::Error::TargetNotReady,
                GamepadStatusKind::INTERNAL_ERROR,
            ),
        ] {
            assert_eq!(map_vigem_error(error), expected);
            let error = GamepadBackendError::from(error);
            assert_eq!(error.kind, expected);
        }
    }

    #[test]
    fn retries_no_more_items_once_with_the_complete_report() {
        let report = XGamepad {
            buttons: 0xa139.into(),
            left_trigger: 17,
            right_trigger: 255,
            thumb_lx: -32768,
            thumb_ly: 32767,
            thumb_rx: -1234,
            thumb_ry: 5678,
        };
        let mut submitted = Vec::new();
        let result = submit_report(&report, |actual| {
            submitted.push(*actual);
            if submitted.len() == 1 {
                Err(vigem_client::Error::WinError(259))
            } else {
                Ok(())
            }
        });
        assert_eq!(result, Ok(()));
        assert_eq!(submitted, [report, report]);
    }

    #[test]
    fn propagates_the_second_submission_failure_without_retrying_again() {
        for error in [
            vigem_client::Error::WinError(259),
            vigem_client::Error::BusAccessFailed(5),
        ] {
            let mut calls = 0;
            let result = submit_report(&XGamepad::default(), |_| {
                calls += 1;
                Err(if calls == 1 {
                    vigem_client::Error::WinError(259)
                } else {
                    error
                })
            });
            assert_eq!(result, Err(error));
            assert_eq!(calls, 2);
        }
    }

    #[test]
    fn does_not_retry_success_or_other_errors() {
        for expected in [
            Ok(()),
            Err(vigem_client::Error::WinError(5)),
            Err(vigem_client::Error::BusNotFound),
            Err(vigem_client::Error::TargetNotReady),
        ] {
            let mut calls = 0;
            let result = submit_report(&XGamepad::default(), |_| {
                calls += 1;
                expected
            });
            assert_eq!(result, expected);
            assert_eq!(calls, 1);
        }
    }

    #[test]
    #[ignore = "requires official ViGEmBus v1.22.0 and RUSTDESK_TEST_VIGEM=1"]
    fn connects_to_vigem_1_22_0() {
        use winapi::um::xinput::{XInputGetState, XINPUT_STATE};

        assert_eq!(std::env::var("RUSTDESK_TEST_VIGEM").as_deref(), Ok("1"));
        let read = |index| {
            let mut state: XINPUT_STATE = unsafe { std::mem::zeroed() };
            match unsafe { XInputGetState(index, &mut state) } {
                0 => Ok(XGamepad::from(state.Gamepad)),
                code => Err(code),
            }
        };
        let disconnected = |index| {
            for _ in 0..20 {
                if read(index) == Err(1167) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            panic!("virtual target still visible after cleanup");
        };
        let mut backend = VigemBackend::connect().expect("connect to ViGEmBus v1.22.0");
        for cycle in 0..2 {
            backend.attach(0).expect("plug in and wait ready");
            backend
                .update(
                    0,
                    &GamepadState {
                        connected: true,
                        buttons: 0x1000,
                        left_trigger: 128,
                        thumb_lx: -16384,
                        ..Default::default()
                    },
                )
                .expect("submit complete report");
            let index = backend
                .target
                .as_mut()
                .expect("attached target")
                .get_user_index()
                .expect("virtual target user index");
            assert_eq!(
                read(index),
                Ok(XGamepad {
                    buttons: 0x1000.into(),
                    left_trigger: 128,
                    thumb_lx: -16384,
                    ..Default::default()
                })
            );
            backend
                .update(0, &GamepadState::default())
                .expect("submit neutral report");
            assert_eq!(read(index), Ok(XGamepad::default()));
            if cycle == 0 {
                backend.detach(0).expect("submit neutral report and unplug");
            } else {
                drop(backend);
                disconnected(index);
                break;
            }
            disconnected(index);
        }
    }
}
