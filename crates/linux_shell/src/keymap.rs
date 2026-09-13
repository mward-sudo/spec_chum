//! GDK keyval → Spectrum matrix chords for the Linux GTK4 shell.
//!
//! Mirrors [`app::keymap`] / [`windows_shell::keymap`] semantics: Ctrl → Symbol,
//! Shift → Caps (unless punctuation owns Symbol), arrows → Caps cursor.

use spec_chum_host::keymap::Chord;

#[cfg(test)]
use spec_chum_host::keymap::{CAPS, SYM};

/// Subset of GDK keyvals used by the shell (US layout).
pub mod key {
    pub const BACKSPACE: u32 = 0xff08;
    pub const RETURN: u32 = 0xff0d;
    pub const LEFT: u32 = 0xff51;
    pub const UP: u32 = 0xff52;
    pub const RIGHT: u32 = 0xff53;
    pub const DOWN: u32 = 0xff54;
    pub const SHIFT_L: u32 = 0xffe1;
    pub const SHIFT_R: u32 = 0xffe2;
    pub const CONTROL_L: u32 = 0xffe3;
    pub const CONTROL_R: u32 = 0xffe4;
    pub const ALT_L: u32 = 0xffe9;
    pub const ALT_R: u32 = 0xffea;
    pub const SPACE: u32 = 0x020;
    pub const APOSTROPHE: u32 = 0x027;
    pub const COMMA: u32 = 0x02c;
    pub const MINUS: u32 = 0x02d;
    pub const PERIOD: u32 = 0x02e;
    pub const SLASH: u32 = 0x02f;
    pub const SEMICOLON: u32 = 0x03b;
    pub const EQUAL: u32 = 0x03d;
    pub const BRACKETLEFT: u32 = 0x05b;
    pub const BACKSLASH: u32 = 0x05c;
    pub const BRACKETRIGHT: u32 = 0x05d;
    pub const GRAVE: u32 = 0x060;
}

/// Map a GDK keyval + Shift to Spectrum matrix chords.
#[must_use]
pub fn chord_for_keyval(keyval: u32, shift: bool) -> Option<Chord> {
    let kv = normalize_letter(keyval);
    match kv {
        key::LEFT => return Some(Chord::with_caps(3, 4)),
        key::DOWN => return Some(Chord::with_caps(4, 4)),
        key::UP => return Some(Chord::with_caps(4, 3)),
        key::RIGHT => return Some(Chord::with_caps(4, 2)),
        key::BACKSPACE => return Some(Chord::with_caps(4, 0)),
        _ => {}
    }

    if let Some(ch) = punct_chord(kv, shift) {
        return Some(ch);
    }

    letter_digit(kv).map(|(row, bit)| Chord::single(row, bit))
}

/// Modifier keys alone (when no punctuation override is active).
#[must_use]
pub fn modifier_keys(shift: bool, alt: bool, ctrl: bool, suppress_caps: bool) -> Vec<(usize, u8)> {
    spec_chum_host::keymap::modifier_keys(shift, alt, ctrl, suppress_caps)
}

/// True when this key owns Symbol/Caps itself (punctuation / arrows / Backspace).
#[must_use]
pub fn suppresses_modifier_caps(keyval: u32) -> bool {
    let kv = normalize_letter(keyval);
    matches!(
        kv,
        key::LEFT
            | key::RIGHT
            | key::UP
            | key::DOWN
            | key::BACKSPACE
            | key::APOSTROPHE
            | key::SEMICOLON
            | key::COMMA
            | key::PERIOD
            | key::SLASH
            | key::MINUS
            | key::EQUAL
            | key::BRACKETLEFT
            | key::BRACKETRIGHT
            | key::BACKSLASH
            | key::GRAVE
    )
}

/// True for Shift / Ctrl / Alt keyvals (handled as modifiers, not held chords).
#[must_use]
pub fn is_modifier_keyval(keyval: u32) -> bool {
    matches!(
        keyval,
        key::SHIFT_L | key::SHIFT_R | key::CONTROL_L | key::CONTROL_R | key::ALT_L | key::ALT_R
    )
}

/// Apply a chord press/release onto the host matrix.
pub fn apply_chord(set_key: &mut dyn FnMut(usize, u8, bool), chord: &Chord, pressed: bool) {
    for &(row, bit) in &chord.keys {
        set_key(row, bit, pressed);
    }
}

/// Apply standalone modifiers (Caps / Sym).
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

fn normalize_letter(keyval: u32) -> u32 {
    // GDK uppercase Latin letters → lowercase for a single match table.
    if (0x041..=0x05a).contains(&keyval) {
        keyval + 0x20
    } else {
        keyval
    }
}

fn letter_digit(keyval: u32) -> Option<(usize, u8)> {
    Some(match keyval {
        0x031 => (3, 0), // 1
        0x032 => (3, 1),
        0x033 => (3, 2),
        0x034 => (3, 3),
        0x035 => (3, 4),
        0x036 => (4, 4),
        0x037 => (4, 3),
        0x038 => (4, 2),
        0x039 => (4, 1),
        0x030 => (4, 0),
        0x071 => (2, 0), // q
        0x077 => (2, 1), // w
        0x065 => (2, 2), // e
        0x072 => (2, 3), // r
        0x074 => (2, 4), // t
        0x079 => (5, 4), // y
        0x075 => (5, 3), // u
        0x069 => (5, 2), // i
        0x06f => (5, 1), // o
        0x070 => (5, 0), // p
        0x061 => (1, 0), // a
        0x073 => (1, 1), // s
        0x064 => (1, 2), // d
        0x066 => (1, 3), // f
        0x067 => (1, 4), // g
        0x068 => (6, 4), // h
        0x06a => (6, 3), // j
        0x06b => (6, 2), // k
        0x06c => (6, 1), // l
        key::RETURN => (6, 0),
        0x07a => (0, 1), // z
        0x078 => (0, 2), // x
        0x063 => (0, 3), // c
        0x076 => (0, 4), // v
        0x062 => (7, 4), // b
        0x06e => (7, 3), // n
        0x06d => (7, 2), // m
        key::SPACE => (7, 0),
        _ => return None,
    })
}

fn punct_chord(keyval: u32, shift: bool) -> Option<Chord> {
    match keyval {
        key::APOSTROPHE => {
            if shift {
                Some(Chord::with_sym(5, 0)) // "
            } else {
                Some(Chord::with_sym(4, 3)) // '
            }
        }
        key::SEMICOLON => {
            if shift {
                Some(Chord::with_sym(0, 1)) // :
            } else {
                Some(Chord::with_sym(5, 1)) // ;
            }
        }
        key::COMMA => {
            if shift {
                Some(Chord::with_sym(2, 3)) // <
            } else {
                Some(Chord::with_sym(7, 3)) // ,
            }
        }
        key::PERIOD => {
            if shift {
                Some(Chord::with_sym(2, 4)) // >
            } else {
                Some(Chord::with_sym(7, 2)) // .
            }
        }
        key::SLASH => {
            if shift {
                Some(Chord::with_sym(0, 3)) // ?
            } else {
                Some(Chord::with_sym(0, 4)) // /
            }
        }
        key::MINUS => {
            if shift {
                Some(Chord::with_sym(4, 0)) // _
            } else {
                Some(Chord::with_sym(6, 3)) // -
            }
        }
        key::EQUAL => {
            if shift {
                Some(Chord::with_sym(6, 2)) // +
            } else {
                Some(Chord::with_sym(6, 1)) // =
            }
        }
        key::BRACKETLEFT => {
            if shift {
                Some(Chord::with_sym(1, 3)) // {
            } else {
                Some(Chord::with_sym(5, 4)) // [
            }
        }
        key::BRACKETRIGHT => {
            if shift {
                Some(Chord::with_sym(1, 4)) // }
            } else {
                Some(Chord::with_sym(5, 3)) // ]
            }
        }
        key::BACKSLASH => {
            if shift {
                Some(Chord::with_sym(1, 1)) // |
            } else {
                Some(Chord::with_sym(1, 2)) // \
            }
        }
        key::GRAVE => {
            if shift {
                Some(Chord::with_sym(1, 0)) // ~
            } else {
                Some(Chord::with_sym(0, 2)) // `
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letter_j_is_load_keyword_row() {
        let ch = chord_for_keyval(0x06a, false).expect("j");
        assert_eq!(ch.keys, vec![(6, 3)]);
    }

    #[test]
    fn uppercase_j_normalizes() {
        let ch = chord_for_keyval(0x04a, false).expect("J");
        assert_eq!(ch.keys, vec![(6, 3)]);
    }

    #[test]
    fn quote_uses_symbol_layer() {
        let ch = chord_for_keyval(key::APOSTROPHE, true).expect("\"");
        assert!(ch.keys.contains(&SYM));
        assert!(ch.keys.contains(&(5, 0)));
    }

    #[test]
    fn arrow_left_is_caps_5() {
        let ch = chord_for_keyval(key::LEFT, false).expect("left");
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
