//! Windows Virtual-Key → Spectrum matrix chords.
//!
//! Mirrors [`app::keymap`] / [`spec_chum_host::keymap`] semantics: Ctrl → Symbol,
//! Shift → Caps (unless punctuation owns Symbol), arrows → Caps cursor.

use spec_chum_host::keymap::Chord;

#[cfg(test)]
use spec_chum_host::keymap::{CAPS, SYM};

/// Windows Virtual-Key codes used by the shell (subset of Winuser.h).
pub mod vk {
    pub const BACK: u16 = 0x08;
    pub const RETURN: u16 = 0x0D;
    pub const SHIFT: u16 = 0x10;
    pub const CONTROL: u16 = 0x11;
    pub const MENU: u16 = 0x12; // Alt
    pub const SPACE: u16 = 0x20;
    pub const LEFT: u16 = 0x25;
    pub const UP: u16 = 0x26;
    pub const RIGHT: u16 = 0x27;
    pub const DOWN: u16 = 0x28;
    pub const OEM_1: u16 = 0xBA; // ;:
    pub const OEM_PLUS: u16 = 0xBB; // =+
    pub const OEM_COMMA: u16 = 0xBC;
    pub const OEM_MINUS: u16 = 0xBD;
    pub const OEM_PERIOD: u16 = 0xBE;
    pub const OEM_2: u16 = 0xBF; // /?
    pub const OEM_3: u16 = 0xC0; // `~
    pub const OEM_4: u16 = 0xDB; // [{
    pub const OEM_5: u16 = 0xDC; // \|
    pub const OEM_6: u16 = 0xDD; // ]}
    pub const OEM_7: u16 = 0xDE; // '"
}

/// Map a Virtual-Key + modifiers to Spectrum matrix chords.
#[must_use]
pub fn chord_for_vk(vk: u16, shift: bool) -> Option<Chord> {
    match vk {
        vk::LEFT => return Some(Chord::with_caps(3, 4)),
        vk::DOWN => return Some(Chord::with_caps(4, 4)),
        vk::UP => return Some(Chord::with_caps(4, 3)),
        vk::RIGHT => return Some(Chord::with_caps(4, 2)),
        vk::BACK => return Some(Chord::with_caps(4, 0)),
        _ => {}
    }

    if let Some(ch) = punct_chord(vk, shift) {
        return Some(ch);
    }

    letter_digit(vk).map(|(row, bit)| Chord::single(row, bit))
}

/// Modifier keys alone (when no punctuation override is active).
#[must_use]
pub fn modifier_keys(shift: bool, alt: bool, ctrl: bool, suppress_caps: bool) -> Vec<(usize, u8)> {
    spec_chum_host::keymap::modifier_keys(shift, alt, ctrl, suppress_caps)
}

/// True when this key owns Symbol/Caps itself (punctuation / arrows / Backspace).
#[must_use]
pub fn suppresses_modifier_caps(vk: u16) -> bool {
    matches!(
        vk,
        vk::LEFT
            | vk::RIGHT
            | vk::UP
            | vk::DOWN
            | vk::BACK
            | vk::OEM_1
            | vk::OEM_PLUS
            | vk::OEM_COMMA
            | vk::OEM_MINUS
            | vk::OEM_PERIOD
            | vk::OEM_2
            | vk::OEM_3
            | vk::OEM_4
            | vk::OEM_5
            | vk::OEM_6
            | vk::OEM_7
    )
}

fn letter_digit(vk: u16) -> Option<(usize, u8)> {
    Some(match vk {
        0x31 => (3, 0), // 1
        0x32 => (3, 1),
        0x33 => (3, 2),
        0x34 => (3, 3),
        0x35 => (3, 4),
        0x36 => (4, 4),
        0x37 => (4, 3),
        0x38 => (4, 2),
        0x39 => (4, 1),
        0x30 => (4, 0),
        0x51 => (2, 0), // Q
        0x57 => (2, 1), // W
        0x45 => (2, 2), // E
        0x52 => (2, 3), // R
        0x54 => (2, 4), // T
        0x59 => (5, 4), // Y
        0x55 => (5, 3), // U
        0x49 => (5, 2), // I
        0x4F => (5, 1), // O
        0x50 => (5, 0), // P
        0x41 => (1, 0), // A
        0x53 => (1, 1), // S
        0x44 => (1, 2), // D
        0x46 => (1, 3), // F
        0x47 => (1, 4), // G
        0x48 => (6, 4), // H
        0x4A => (6, 3), // J
        0x4B => (6, 2), // K
        0x4C => (6, 1), // L
        vk::RETURN => (6, 0),
        0x5A => (0, 1), // Z
        0x58 => (0, 2), // X
        0x43 => (0, 3), // C
        0x56 => (0, 4), // V
        0x42 => (7, 4), // B
        0x4E => (7, 3), // N
        0x4D => (7, 2), // M
        vk::SPACE => (7, 0),
        _ => return None,
    })
}

fn punct_chord(vk: u16, shift: bool) -> Option<Chord> {
    match vk {
        vk::OEM_7 => {
            if shift {
                Some(Chord::with_sym(5, 0)) // "
            } else {
                Some(Chord::with_sym(4, 3)) // '
            }
        }
        vk::OEM_1 => {
            if shift {
                Some(Chord::with_sym(0, 1)) // :
            } else {
                Some(Chord::with_sym(5, 1)) // ;
            }
        }
        vk::OEM_COMMA => {
            if shift {
                Some(Chord::with_sym(2, 3)) // <
            } else {
                Some(Chord::with_sym(7, 3)) // ,
            }
        }
        vk::OEM_PERIOD => {
            if shift {
                Some(Chord::with_sym(2, 4)) // >
            } else {
                Some(Chord::with_sym(7, 2)) // .
            }
        }
        vk::OEM_2 => {
            if shift {
                Some(Chord::with_sym(0, 3)) // ?
            } else {
                Some(Chord::with_sym(0, 4)) // /
            }
        }
        vk::OEM_MINUS => {
            if shift {
                Some(Chord::with_sym(4, 0)) // _
            } else {
                Some(Chord::with_sym(6, 3)) // -
            }
        }
        vk::OEM_PLUS => {
            if shift {
                Some(Chord::with_sym(6, 2)) // +
            } else {
                Some(Chord::with_sym(6, 1)) // =
            }
        }
        vk::OEM_4 => {
            if shift {
                Some(Chord::with_sym(1, 3)) // {
            } else {
                Some(Chord::with_sym(5, 4)) // [
            }
        }
        vk::OEM_6 => {
            if shift {
                Some(Chord::with_sym(1, 4)) // }
            } else {
                Some(Chord::with_sym(5, 3)) // ]
            }
        }
        vk::OEM_5 => {
            if shift {
                Some(Chord::with_sym(1, 1)) // |
            } else {
                Some(Chord::with_sym(1, 2)) // \
            }
        }
        vk::OEM_3 => {
            if shift {
                Some(Chord::with_sym(1, 0)) // ~
            } else {
                Some(Chord::with_sym(0, 2)) // `
            }
        }
        _ => None,
    }
}

/// Apply a chord press/release onto the host matrix.
pub fn apply_chord(set_key: &mut dyn FnMut(usize, u8, bool), chord: &Chord, pressed: bool) {
    for &(row, bit) in &chord.keys {
        set_key(row, bit, pressed);
    }
}

/// Apply standalone modifiers (Caps / Sym). Does not clear other matrix bits —
/// callers should [`HostSession::clear_keys`] (or equivalent) first.
pub fn apply_modifiers(
    set_key: &mut dyn FnMut(usize, u8, bool),
    shift: bool,
    alt: bool,
    ctrl: bool,
    suppress_caps: bool,
) {
    for &(row, bit) in &modifier_keys(shift, alt, ctrl, suppress_caps) {
        set_key(row, bit, true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letter_j_is_load_keyword_row() {
        let ch = chord_for_vk(0x4A, false).expect("J");
        assert_eq!(ch.keys, vec![(6, 3)]);
    }

    #[test]
    fn quote_uses_symbol_layer() {
        let ch = chord_for_vk(vk::OEM_7, true).expect("\"");
        assert!(ch.keys.contains(&SYM));
        assert!(ch.keys.contains(&(5, 0)));
    }

    #[test]
    fn arrow_left_is_caps_5() {
        let ch = chord_for_vk(vk::LEFT, false).expect("left");
        assert!(ch.keys.contains(&CAPS));
        assert!(ch.keys.contains(&(3, 4)));
    }

    #[test]
    fn ctrl_maps_to_symbol_modifier() {
        let mods = modifier_keys(false, false, true, false);
        assert!(mods.contains(&SYM));
        assert!(!mods.contains(&CAPS));
    }
}
