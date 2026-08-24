//! Host joystick → Kempston / Sinclair / Cursor mapping.

use bus::{Kempston, Keyboard};

/// Logical digital stick (active when true).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct JoystickState {
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
    pub fire: bool,
}

impl JoystickState {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            up: false,
            down: false,
            left: false,
            right: false,
            fire: false,
        }
    }

    /// Bit0=right, 1=left, 2=down, 3=up, 4=fire (matches Kempston / `sc_set_joystick`).
    #[must_use]
    pub fn from_mask(mask: u8) -> Self {
        Self {
            right: mask & 0x01 != 0,
            left: mask & 0x02 != 0,
            down: mask & 0x04 != 0,
            up: mask & 0x08 != 0,
            fire: mask & 0x10 != 0,
        }
    }

    #[must_use]
    pub fn to_mask(self) -> u8 {
        let mut m = 0u8;
        if self.right {
            m |= 0x01;
        }
        if self.left {
            m |= 0x02;
        }
        if self.down {
            m |= 0x04;
        }
        if self.up {
            m |= 0x08;
        }
        if self.fire {
            m |= 0x10;
        }
        m
    }
}

/// How a host stick is presented to software.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum JoystickMode {
    #[default]
    Kempston = 0,
    /// Interface 2 left stick: keys 1–5.
    SinclairLeft = 1,
    /// Interface 2 right stick: keys 6–0.
    SinclairRight = 2,
    /// Cursor keys (Caps + 5/6/7/8) + 0 fire.
    Cursor = 3,
}

impl JoystickMode {
    #[must_use]
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Kempston),
            1 => Some(Self::SinclairLeft),
            2 => Some(Self::SinclairRight),
            3 => Some(Self::Cursor),
            _ => None,
        }
    }
}

/// Clear every matrix key that any joystick mode may press.
pub fn clear_joystick_matrix(kb: &mut Keyboard) {
    // Sinclair left 1–5: row 3 bits 0–4
    for bit in 0..=4u8 {
        kb.set_key(3, bit, false);
    }
    // Sinclair right / cursor digits on row 4: 0,9,8,7,6
    for bit in 0..=4u8 {
        kb.set_key(4, bit, false);
    }
    // Caps Shift (cursor)
    kb.set_key(0, 0, false);
}

/// Apply `state` under `mode` to Kempston and/or the keyboard matrix.
///
/// Kempston mode only touches the Kempston port so host keyboard matrix chords
/// (e.g. arrow → Caps+5/6/7/8) stay intact — matches egui. Sinclair/Cursor
/// modes clear joystick matrix keys first so releases stick.
pub fn apply_joystick(
    mode: JoystickMode,
    state: JoystickState,
    k: &mut Kempston,
    kb: &mut Keyboard,
) {
    k.reset();

    match mode {
        JoystickMode::Kempston => {
            k.right = state.right;
            k.left = state.left;
            k.down = state.down;
            k.up = state.up;
            k.fire = state.fire;
        }
        JoystickMode::SinclairLeft => {
            clear_joystick_matrix(kb);
            // 1=left, 2=right, 3=down, 4=up, 5=fire
            kb.set_key(3, 0, state.left);
            kb.set_key(3, 1, state.right);
            kb.set_key(3, 2, state.down);
            kb.set_key(3, 3, state.up);
            kb.set_key(3, 4, state.fire);
        }
        JoystickMode::SinclairRight => {
            clear_joystick_matrix(kb);
            // 6=left, 7=right, 8=down, 9=up, 0=fire
            kb.set_key(4, 4, state.left);
            kb.set_key(4, 3, state.right);
            kb.set_key(4, 2, state.down);
            kb.set_key(4, 1, state.up);
            kb.set_key(4, 0, state.fire);
        }
        JoystickMode::Cursor => {
            clear_joystick_matrix(kb);
            // Caps + 5/6/7/8; fire = 0
            let any_dir = state.left || state.right || state.up || state.down;
            kb.set_key(0, 0, any_dir);
            kb.set_key(3, 4, state.left); // 5
            kb.set_key(4, 4, state.down); // 6
            kb.set_key(4, 3, state.up); // 7
            kb.set_key(4, 2, state.right); // 8
            kb.set_key(4, 0, state.fire); // 0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kempston_mask_roundtrip() {
        let s = JoystickState {
            up: true,
            fire: true,
            ..JoystickState::empty()
        };
        assert_eq!(s.to_mask(), 0x18);
        assert_eq!(JoystickState::from_mask(0x18), s);
    }

    #[test]
    fn kempston_mode_sets_port_bits() {
        let mut k = Kempston::new();
        let mut kb = Keyboard::new();
        apply_joystick(
            JoystickMode::Kempston,
            JoystickState {
                left: true,
                fire: true,
                ..JoystickState::empty()
            },
            &mut k,
            &mut kb,
        );
        assert_eq!(k.read(), 0x12);
        assert_eq!(kb.rows, [0x1f; 8]);
    }

    #[test]
    fn sinclair_left_presses_12345_row() {
        let mut k = Kempston::new();
        let mut kb = Keyboard::new();
        apply_joystick(
            JoystickMode::SinclairLeft,
            JoystickState {
                left: true,
                right: true,
                down: true,
                up: true,
                fire: true,
            },
            &mut k,
            &mut kb,
        );
        assert_eq!(k.read(), 0);
        assert_eq!(kb.rows[3] & 0x1f, 0);
    }

    #[test]
    fn sinclair_right_presses_67890_row() {
        let mut k = Kempston::new();
        let mut kb = Keyboard::new();
        apply_joystick(
            JoystickMode::SinclairRight,
            JoystickState {
                left: true,
                fire: true,
                ..JoystickState::empty()
            },
            &mut k,
            &mut kb,
        );
        assert_eq!(kb.rows[4] & (1 << 4), 0); // 6 down (active low)
        assert_eq!(kb.rows[4] & (1 << 0), 0); // 0 fire
        assert_ne!(kb.rows[4] & (1 << 3), 0); // 7 not pressed
    }

    #[test]
    fn cursor_uses_caps_and_5678() {
        let mut k = Kempston::new();
        let mut kb = Keyboard::new();
        apply_joystick(
            JoystickMode::Cursor,
            JoystickState {
                up: true,
                ..JoystickState::empty()
            },
            &mut k,
            &mut kb,
        );
        assert_eq!(kb.rows[0] & 1, 0); // caps
        assert_eq!(kb.rows[4] & (1 << 3), 0); // 7
    }

    #[test]
    fn release_clears_previous_matrix() {
        let mut k = Kempston::new();
        let mut kb = Keyboard::new();
        apply_joystick(
            JoystickMode::SinclairLeft,
            JoystickState {
                fire: true,
                ..JoystickState::empty()
            },
            &mut k,
            &mut kb,
        );
        apply_joystick(
            JoystickMode::SinclairLeft,
            JoystickState::empty(),
            &mut k,
            &mut kb,
        );
        assert_eq!(kb.rows[3], 0x1f);
    }
}
