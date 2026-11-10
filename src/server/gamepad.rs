use base::message_proto::GamepadInput;
use hbb_common::{anyhow::Context, bail, ResultType};

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

#[cfg(test)]
mod tests {
    use super::*;

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
