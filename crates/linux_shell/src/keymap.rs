//! GDK keyval → Spectrum matrix chords for the Linux GTK4 shell.
//!
//! Mirrors [`app::keymap`] / [`windows_shell::keymap`] semantics: Ctrl → Symbol,
//! Shift → Caps (letters), Symbol+digit for shifted digits, arrows/Tab → selected joystick.

use spec_chum_host::keymap::Chord;

#[cfg(test)]
use spec_chum_host::keymap::{CAPS, SYM};

/// Subset of GDK keyvals used by the shell (US layout).
pub mod key {
    pub const BACKSPACE: u32 = 0xff08;
    pub const TAB: u32 = 0xff09;
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
    // Shifted digit Unicode (!@#…) must be Symbol+digit, not Caps+digit.
    if let Some(ch) = shifted_digit_chord(keyval) {
        return Some(ch);
    }

    let kv = canonical_keyval(keyval);
    if is_joystick_routing_key(kv) {
        return None;
    }
    if kv == key::BACKSPACE {
        return Some(Chord::with_caps(4, 0));
    }

    if let Some(ch) = punct_chord(kv, shift) {
        return Some(ch);
    }

    // Host Shift+digit → Symbol+digit (e.g. Shift+1 → !), not Caps+digit (EDIT).
    if shift {
        if let Some((row, bit)) = letter_digit(kv) {
            if (0x030..=0x039).contains(&kv) {
                return Some(Chord::with_sym(row, bit));
            }
        }
    }

    letter_digit(kv).map(|(row, bit)| Chord::single(row, bit))
}

/// Arrow directions and Tab fire are routed through the selected host joystick mode.
#[must_use]
pub fn is_joystick_routing_key(keyval: u32) -> bool {
    matches!(
        canonical_keyval(keyval),
        key::LEFT | key::RIGHT | key::UP | key::DOWN | key::TAB
    )
}

/// Kempston-compatible direction/fire mask from held GDK keyvals.
#[must_use]
pub fn kempston_mask(held: &[u32]) -> u8 {
    let mut mask = 0;
    for &keyval in held {
        match canonical_keyval(keyval) {
            key::RIGHT => mask |= 1 << 0,
            key::LEFT => mask |= 1 << 1,
            key::DOWN => mask |= 1 << 2,
            key::UP => mask |= 1 << 3,
            key::TAB => mask |= 1 << 4,
            _ => {}
        }
    }
    mask
}

/// Modifier keys alone (when no punctuation override is active).
#[must_use]
pub fn modifier_keys(shift: bool, alt: bool, ctrl: bool, suppress_caps: bool) -> Vec<(usize, u8)> {
    spec_chum_host::keymap::modifier_keys(shift, alt, ctrl, suppress_caps)
}

/// True when this key owns Symbol/Caps itself (punctuation / Backspace).
///
/// Digits suppress Caps when `shift` is held so Shift+1 becomes Symbol+1, not Caps+1.
#[must_use]
pub fn suppresses_modifier_caps(keyval: u32, shift: bool) -> bool {
    if shifted_digit_chord(keyval).is_some() {
        return true;
    }
    let kv = canonical_keyval(keyval);
    if shift && (0x030..=0x039).contains(&kv) {
        return true;
    }
    matches!(
        kv,
        key::BACKSPACE
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

fn shifted_digit_chord(keyval: u32) -> Option<Chord> {
    // US Shift+digit punctuation → Spectrum Symbol layer (not Caps+digit).
    Some(match keyval {
        0x021 => Chord::with_sym(3, 0), // !
        0x040 => Chord::with_sym(3, 1), // @
        0x023 => Chord::with_sym(3, 2), // #
        0x024 => Chord::with_sym(3, 3), // $
        0x025 => Chord::with_sym(3, 4), // %
        0x05e => Chord::with_sym(4, 4), // ^
        0x026 => Chord::with_sym(4, 3), // &
        0x02a => Chord::with_sym(4, 2), // *
        0x028 => Chord::with_sym(4, 1), // (
        0x029 => Chord::with_sym(4, 0), // )
        _ => return None,
    })
}

fn canonical_keyval(keyval: u32) -> u32 {
    // GDK may deliver the shifted Unicode keyval for punctuation keys (", :, etc.).
    // Digits' shifted forms are handled by [`shifted_digit_chord`] before this runs.
    let base = match keyval {
        0x022 => key::APOSTROPHE,   // "
        0x03a => key::SEMICOLON,    // :
        0x03c => key::COMMA,        // <
        0x03e => key::PERIOD,       // >
        0x03f => key::SLASH,        // ?
        0x05f => key::MINUS,        // _
        0x02b => key::EQUAL,        // +
        0x07b => key::BRACKETLEFT,  // {
        0x07d => key::BRACKETRIGHT, // }
        0x07c => key::BACKSLASH,    // |
        0x07e => key::GRAVE,        // ~
        other => other,
    };
    normalize_letter(base)
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
    fn shifted_exclam_is_symbol_one() {
        let ch = chord_for_keyval(0x021, true).expect("!");
        assert!(ch.keys.contains(&SYM));
        assert!(ch.keys.contains(&(3, 0)));
        assert!(!ch.keys.contains(&CAPS));
    }

    #[test]
    fn shifted_exclam_suppresses_caps_modifier() {
        assert!(suppresses_modifier_caps(0x021, false));
        assert!(suppresses_modifier_caps(0x021, true));
    }

    #[test]
    fn host_shift_digit_one_is_symbol_not_caps() {
        let ch = chord_for_keyval(0x031, true).expect("Shift+1");
        assert!(ch.keys.contains(&SYM));
        assert!(ch.keys.contains(&(3, 0)));
        assert!(!ch.keys.contains(&CAPS));
        assert!(suppresses_modifier_caps(0x031, true));
        assert!(!suppresses_modifier_caps(0x031, false));
    }

    #[test]
    fn shifted_digit_contract_maps_all_digits_to_symbol_without_caps() {
        let cases = [
            (0x031, (3, 0)),
            (0x032, (3, 1)),
            (0x033, (3, 2)),
            (0x034, (3, 3)),
            (0x035, (3, 4)),
            (0x036, (4, 4)),
            (0x037, (4, 3)),
            (0x038, (4, 2)),
            (0x039, (4, 1)),
            (0x030, (4, 0)),
        ];
        for (keyval, digit) in cases {
            assert_eq!(
                chord_for_keyval(keyval, true).unwrap().keys,
                vec![SYM, digit],
                "keyval {keyval:#x}"
            );
            assert!(suppresses_modifier_caps(keyval, true), "keyval {keyval:#x}");
            assert!(
                !suppresses_modifier_caps(keyval, false),
                "keyval {keyval:#x}"
            );
        }
    }

    #[test]
    fn punctuation_suppresses_caps_only_without_a_plain_letter_chord() {
        let quote_owns_modifier = suppresses_modifier_caps(key::APOSTROPHE, true);
        let letter_needs_modifier =
            chord_for_keyval(0x061, true).is_some() && !suppresses_modifier_caps(0x061, true);
        assert!(spec_chum_host::keymap::caps_modifier_suppressed(
            quote_owns_modifier,
            false,
            false
        ));
        assert!(!spec_chum_host::keymap::caps_modifier_suppressed(
            quote_owns_modifier,
            letter_needs_modifier,
            false
        ));
    }

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
    fn arrows_and_tab_are_joystick_routed() {
        assert!(is_joystick_routing_key(key::LEFT));
        assert!(chord_for_keyval(key::LEFT, false).is_none());
        assert_eq!(kempston_mask(&[key::LEFT, key::TAB]), 0x12);
    }

    #[test]
    fn ctrl_maps_to_symbol_modifier() {
        let mods = modifier_keys(false, false, true, false);
        assert!(mods.contains(&SYM));
        assert!(!mods.contains(&CAPS));
    }
}
