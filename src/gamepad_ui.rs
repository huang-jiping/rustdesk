use base::message_proto::GamepadStatusKind;

#[derive(Default)]
pub(crate) struct GamepadUiState {
    pub enabled: bool,
    pub status: GamepadStatusKind,
    pub local_connected: bool,
}

impl GamepadUiState {
    pub fn update(&mut self, status: Option<GamepadStatusKind>, local_connected: bool) {
        if let Some(status) = status {
            self.status = status;
            if !matches!(status, GamepadStatusKind::READY | GamepadStatusKind::ACTIVE) {
                self.enabled = false;
            }
        }
        self.local_connected = local_connected;
    }
}

#[cfg(test)]
mod tests_gamepad {
    use super::*;

    #[test]
    fn gamepad_status_preserves_intent_until_stopped_or_failed() {
        let mut state = GamepadUiState::default();
        assert!(!state.enabled);
        state.enabled = true;
        state.update(Some(GamepadStatusKind::READY), false);
        assert!(state.enabled);
        assert_eq!(state.status, GamepadStatusKind::READY);
        state.update(Some(GamepadStatusKind::ACTIVE), true);
        assert!(state.local_connected);
        state.update(Some(GamepadStatusKind::DRIVER_MISSING), false);
        assert!(!state.enabled);
        assert_eq!(state.status, GamepadStatusKind::DRIVER_MISSING);
        assert!(!state.local_connected);
    }

    #[test]
    fn gamepad_local_connection_edge_keeps_remote_status() {
        let mut state = GamepadUiState::default();
        state.update(Some(GamepadStatusKind::ACTIVE), true);
        state.update(None, false);
        assert_eq!(state.status, GamepadStatusKind::ACTIVE);
        assert!(!state.local_connected);
    }
}
