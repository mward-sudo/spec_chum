//! Host → ZX Spectrum keyboard matrix mapping (ANSI key codes).
//!
//! Canonical table for the macOS `SwiftUI` shell (`SpectrumKeymap.swift`). The egui
//! app uses the same chords via `app::keymap` (logical `egui::Key` ids). Matrix
//! rows are active-low bits 0–4, matching `bus::Keyboard`.

/// Caps Shift (row 0, bit 0).
pub const CAPS: (usize, u8) = (0, 0);
/// Symbol Shift (row 7, bit 1).
pub const SYM: (usize, u8) = (7, 1);

/// A Spectrum key chord: one or more matrix positions held together.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chord {
    pub keys: Vec<(usize, u8)>,
}

impl Chord {
    #[must_use]
    pub fn single(row: usize, bit: u8) -> Self {
        Self {
            keys: vec![(row, bit)],
        }
    }

    #[must_use]
    pub fn with_caps(row: usize, bit: u8) -> Self {
        Self {
            keys: vec![CAPS, (row, bit)],
        }
    }

    #[must_use]
    pub fn with_sym(row: usize, bit: u8) -> Self {
        Self {
            keys: vec![SYM, (row, bit)],
        }
    }
}

/// ANSI US Mac key codes (Carbon / `HIToolbox`) — keep in sync with `SpectrumKeymap.swift`.
pub mod ansi {
    pub const LEFT: u16 = 123;
    pub const DOWN: u16 = 125;
    pub const UP: u16 = 126;
    pub const RIGHT: u16 = 124;
    pub const DELETE: u16 = 51;
    pub const TAB: u16 = 48;
    pub const RETURN: u16 = 36;
    pub const SPACE: u16 = 49;
    pub const QUOTE: u16 = 39;
    pub const SEMICOLON: u16 = 41;
    pub const COMMA: u16 = 43;
    pub const PERIOD: u16 = 47;
    pub const SLASH: u16 = 44;
    pub const MINUS: u16 = 27;
    pub const EQUALS: u16 = 24;
    pub const OPEN_BRACKET: u16 = 33;
    pub const CLOSE_BRACKET: u16 = 30;
    pub const BACKSLASH: u16 = 42;
    pub const BACKTICK: u16 = 50;
}

/// Keys routed to the host joystick stick (arrows + Tab fire), not the matrix.
/// Matches egui `sync_keyboard` skipping `Arrow*` / `Tab` for matrix chords.
#[must_use]
pub fn is_joystick_routing_key(key_code: u16) -> bool {
    matches!(
        key_code,
        ansi::LEFT | ansi::RIGHT | ansi::UP | ansi::DOWN | ansi::TAB
    )
}

/// Kempston mask from held ANSI key codes (egui: arrows + Tab fire).
/// Bits: 0=right, 1=left, 2=down, 3=up, 4=fire.
#[must_use]
pub fn kempston_mask(held: &[u16]) -> u8 {
    let mut mask = 0u8;
    for &code in held {
        match code {
            ansi::RIGHT => mask |= 1 << 0,
            ansi::LEFT => mask |= 1 << 1,
            ansi::DOWN => mask |= 1 << 2,
            ansi::UP => mask |= 1 << 3,
            ansi::TAB => mask |= 1 << 4,
            _ => {}
        }
    }
    mask
}

/// Map an ANSI key + modifiers to Spectrum matrix chords.
///
/// Returns `None` for unmapped keys (e.g. F-keys). Joystick-routing keys return
/// `None` — they must go through `sc_set_joystick`, not the matrix.
#[must_use]
pub fn chord_for_ansi(key_code: u16, shift: bool) -> Option<Chord> {
    if is_joystick_routing_key(key_code) {
        return None;
    }

    // Backspace / cursor-delete → Caps+0 (not joystick-routed).
    if key_code == ansi::DELETE {
        return Some(Chord::with_caps(4, 0));
    }

    if let Some(ch) = punct_chord_ansi(key_code, shift) {
        return Some(ch);
    }

    letter_digit_ansi(key_code).map(|(row, bit)| Chord::single(row, bit))
}

/// Modifier keys alone (when no punctuation override is active).
#[must_use]
pub fn modifier_keys(
    shift: bool,
    option: bool,
    control: bool,
    suppress_caps: bool,
) -> Vec<(usize, u8)> {
    let mut out = Vec::new();
    if shift && !suppress_caps {
        out.push(CAPS);
    }
    if option || control {
        out.push(SYM);
    }
    out
}

/// True when this key owns Symbol/Caps itself (punctuation / delete).
#[must_use]
pub fn suppresses_modifier_caps(key_code: u16) -> bool {
    if key_code == ansi::DELETE {
        return true;
    }
    punct_chord_ansi(key_code, false).is_some() || punct_chord_ansi(key_code, true).is_some()
}

/// Rebuild matrix keys + modifiers + Kempston mask from a held set (macOS `syncMatrix` parity).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncKeys {
    pub modifiers: Vec<(usize, u8)>,
    pub matrix: Vec<(usize, u8)>,
    pub kempston_mask: u8,
}

#[must_use]
pub fn sync_keys(held: &[u16], shift: bool, option: bool, control: bool) -> SyncKeys {
    let has_non_joystick_held = held
        .iter()
        .copied()
        .any(|code| !is_joystick_routing_key(code));
    let suppress_caps = if has_non_joystick_held {
        held.iter().copied().any(suppresses_modifier_caps)
    } else {
        held.iter()
            .copied()
            .any(|code| is_joystick_routing_key(code) || suppresses_modifier_caps(code))
    };
    let modifiers = modifier_keys(shift, option, control, suppress_caps);
    let mut matrix = Vec::new();
    for &code in held {
        if is_joystick_routing_key(code) {
            continue;
        }
        let Some(chord) = chord_for_ansi(code, shift) else {
            continue;
        };
        if suppresses_modifier_caps(code) {
            matrix.extend(chord.keys);
        } else {
            matrix.extend(chord.keys.into_iter().filter(|&k| k != CAPS && k != SYM));
        }
    }
    // Dedupe shared positions (e.g. SYM in modifiers and matrix) — Swift syncMatrix parity.
    let mut seen = std::collections::HashSet::new();
    let modifiers = modifiers.into_iter().filter(|k| seen.insert(*k)).collect();
    let matrix = matrix.into_iter().filter(|k| seen.insert(*k)).collect();
    SyncKeys {
        modifiers,
        matrix,
        kempston_mask: kempston_mask(held),
    }
}

fn letter_digit_ansi(key_code: u16) -> Option<(usize, u8)> {
    Some(match key_code {
        18 => (3, 0), // 1
        19 => (3, 1), // 2
        20 => (3, 2), // 3
        21 => (3, 3), // 4
        23 => (3, 4), // 5
        22 => (4, 4), // 6
        26 => (4, 3), // 7
        28 => (4, 2), // 8
        25 => (4, 1), // 9
        29 => (4, 0), // 0
        12 => (2, 0), // Q
        13 => (2, 1), // W
        14 => (2, 2), // E
        15 => (2, 3), // R
        17 => (2, 4), // T
        16 => (5, 4), // Y
        32 => (5, 3), // U
        34 => (5, 2), // I
        31 => (5, 1), // O
        35 => (5, 0), // P
        0 => (1, 0),  // A
        1 => (1, 1),  // S
        2 => (1, 2),  // D
        3 => (1, 3),  // F
        5 => (1, 4),  // G
        4 => (6, 4),  // H
        38 => (6, 3), // J
        40 => (6, 2), // K
        37 => (6, 1), // L
        6 => (0, 1),  // Z
        7 => (0, 2),  // X
        8 => (0, 3),  // C
        9 => (0, 4),  // V
        11 => (7, 4), // B
        45 => (7, 3), // N
        46 => (7, 2), // M
        ansi::RETURN => (6, 0),
        ansi::SPACE => (7, 0),
        _ => return None,
    })
}

fn punct_chord_ansi(key_code: u16, shift: bool) -> Option<Chord> {
    match key_code {
        ansi::QUOTE => {
            if shift {
                Some(Chord::with_sym(5, 0)) // P → "
            } else {
                Some(Chord::with_sym(4, 3)) // 7 → '
            }
        }
        ansi::SEMICOLON => {
            if shift {
                Some(Chord::with_sym(0, 1)) // Z → :
            } else {
                Some(Chord::with_sym(5, 1)) // O → ;
            }
        }
        ansi::COMMA => {
            if shift {
                Some(Chord::with_sym(2, 3)) // R → <
            } else {
                Some(Chord::with_sym(7, 3)) // N → ,
            }
        }
        ansi::PERIOD => {
            if shift {
                Some(Chord::with_sym(2, 4)) // T → >
            } else {
                Some(Chord::with_sym(7, 2)) // M → .
            }
        }
        ansi::SLASH => {
            if shift {
                Some(Chord::with_sym(0, 3)) // C → ?
            } else {
                Some(Chord::with_sym(0, 4)) // V → /
            }
        }
        ansi::MINUS => {
            if shift {
                Some(Chord::with_sym(4, 0)) // 0 → _
            } else {
                Some(Chord::with_sym(6, 3)) // J → -
            }
        }
        ansi::EQUALS => {
            if shift {
                Some(Chord::with_sym(6, 2)) // K → +
            } else {
                Some(Chord::with_sym(6, 1)) // L → =
            }
        }
        ansi::OPEN_BRACKET => {
            if shift {
                Some(Chord::with_sym(1, 3)) // F → {
            } else {
                Some(Chord::with_sym(5, 4)) // Y → [
            }
        }
        ansi::CLOSE_BRACKET => {
            if shift {
                Some(Chord::with_sym(1, 4)) // G → }
            } else {
                Some(Chord::with_sym(5, 3)) // U → ]
            }
        }
        ansi::BACKSLASH => {
            if shift {
                Some(Chord::with_sym(1, 1)) // S → |
            } else {
                Some(Chord::with_sym(1, 2)) // D
            }
        }
        ansi::BACKTICK => {
            if shift {
                Some(Chord::with_sym(1, 0)) // A → ~
            } else {
                Some(Chord::with_sym(0, 2)) // X
            }
        }
        _ => None,
    }
}

/// Human-readable mapping notes for Help / docs.
pub const MAPPING_DOC: &str = "\
Mac → Spectrum keyboard
• Letters/digits: direct matrix; Shift = Caps Shift; Option/Alt = Symbol Shift
• \" (Shift+Quote) = Symbol + P    ' (Quote) = Symbol + 7
• Arrows + Tab fire = host joystick (Settings: Kempston / Sinclair / Cursor)
• Backspace = Caps + 0 (DELETE)
• ; , . / - = = Symbol + O N M V J L (Shift variants for : < > ? _ +)
• LOAD \"\": Tape → Type LOAD \"\" / Type LOAD \"\" CODE (all models)
• 128K/+3: Type LOAD selects 48 BASIC then keywords — +3 menu Loader is disk, not tape
• +2A: Type LOAD selects menu Loader (tape) for PROGRAM; CODE uses 48 BASIC
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_shift_is_symbol_p() {
        let c = chord_for_ansi(ansi::QUOTE, true).unwrap();
        assert_eq!(c.keys, vec![SYM, (5, 0)]);
        assert!(suppresses_modifier_caps(ansi::QUOTE));
    }

    #[test]
    fn quote_alone_is_symbol_7() {
        let c = chord_for_ansi(ansi::QUOTE, false).unwrap();
        assert_eq!(c.keys, vec![SYM, (4, 3)]);
    }

    #[test]
    fn arrows_are_joystick_routing_not_matrix() {
        assert!(is_joystick_routing_key(ansi::LEFT));
        assert!(chord_for_ansi(ansi::LEFT, false).is_none());
        let sync = sync_keys(&[ansi::LEFT], false, false, false);
        assert!(sync.matrix.is_empty());
        assert_eq!(sync.kempston_mask, 0x02);
    }

    #[test]
    fn delete_is_caps_zero_not_joystick() {
        assert!(!is_joystick_routing_key(ansi::DELETE));
        let c = chord_for_ansi(ansi::DELETE, false).unwrap();
        assert_eq!(c.keys, vec![CAPS, (4, 0)]);
    }

    #[test]
    fn letter_j_is_load_key() {
        assert_eq!(chord_for_ansi(38, false).unwrap().keys, vec![(6, 3)]);
    }

    #[test]
    fn shift_modifier_is_caps_unless_suppressed() {
        assert_eq!(modifier_keys(true, false, false, false), vec![CAPS]);
        assert!(modifier_keys(true, false, false, true).is_empty());
    }

    #[test]
    fn tab_is_fire_only_via_kempston_mask() {
        let sync = sync_keys(&[ansi::TAB], false, false, false);
        assert!(sync.matrix.is_empty());
        assert_eq!(sync.kempston_mask, 0x10);
    }

    #[test]
    fn shift_arrow_suppresses_caps_modifier() {
        let sync = sync_keys(&[ansi::LEFT], true, false, false);
        assert!(sync.modifiers.is_empty());
        assert!(sync.matrix.is_empty());
        assert_eq!(sync.kempston_mask, 0x02);
    }

    #[test]
    fn shift_tab_suppresses_caps_modifier() {
        let sync = sync_keys(&[ansi::TAB], true, false, false);
        assert!(sync.modifiers.is_empty());
        assert!(sync.matrix.is_empty());
        assert_eq!(sync.kempston_mask, 0x10);
    }

    #[test]
    fn shift_left_a_preserves_caps_for_matrix_key() {
        let sync = sync_keys(&[ansi::LEFT, 0], true, false, false);
        assert_eq!(sync.modifiers, vec![CAPS]);
        assert_eq!(sync.matrix, vec![(1, 0)]);
        assert_eq!(sync.kempston_mask, 0x02);
    }

    #[test]
    fn symbol_layer_punctuation_table() {
        let cases = [
            (ansi::SEMICOLON, false, vec![SYM, (5, 1)]),
            (ansi::SEMICOLON, true, vec![SYM, (0, 1)]),
            (ansi::COMMA, false, vec![SYM, (7, 3)]),
            (ansi::COMMA, true, vec![SYM, (2, 3)]),
            (ansi::PERIOD, false, vec![SYM, (7, 2)]),
            (ansi::PERIOD, true, vec![SYM, (2, 4)]),
            (ansi::SLASH, false, vec![SYM, (0, 4)]),
            (ansi::SLASH, true, vec![SYM, (0, 3)]),
            (ansi::MINUS, false, vec![SYM, (6, 3)]),
            (ansi::MINUS, true, vec![SYM, (4, 0)]),
            (ansi::EQUALS, false, vec![SYM, (6, 1)]),
            (ansi::EQUALS, true, vec![SYM, (6, 2)]),
            (ansi::OPEN_BRACKET, false, vec![SYM, (5, 4)]),
            (ansi::OPEN_BRACKET, true, vec![SYM, (1, 3)]),
            (ansi::CLOSE_BRACKET, false, vec![SYM, (5, 3)]),
            (ansi::CLOSE_BRACKET, true, vec![SYM, (1, 4)]),
            (ansi::BACKSLASH, false, vec![SYM, (1, 2)]),
            (ansi::BACKSLASH, true, vec![SYM, (1, 1)]),
            (ansi::BACKTICK, false, vec![SYM, (0, 2)]),
            (ansi::BACKTICK, true, vec![SYM, (1, 0)]),
        ];
        for (code, shift, want) in cases {
            assert_eq!(
                chord_for_ansi(code, shift).unwrap().keys,
                want,
                "key {code} shift={shift}"
            );
        }
    }

    #[test]
    fn sync_j_plus_shift_is_caps_and_j() {
        let sync = sync_keys(&[38], true, false, false);
        assert_eq!(sync.modifiers, vec![CAPS]);
        assert_eq!(sync.matrix, vec![(6, 3)]);
    }

    #[test]
    fn sync_dedupes_modifier_positions_in_matrix() {
        // Option+Quote: SYM in modifiers and in chord — must not appear twice.
        let sync = sync_keys(&[ansi::QUOTE], false, true, false);
        assert_eq!(sync.modifiers, vec![SYM]);
        assert_eq!(sync.matrix, vec![(4, 3)]); // Symbol+7 for '
    }
}
