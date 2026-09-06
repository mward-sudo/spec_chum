//! Host (macOS/egui) → ZX Spectrum keyboard matrix mapping.
//!
//! Shared matrix constants and Symbol-layer tables live in [`spec_chum_host::keymap`]
//! (ANSI codes for the `SwiftUI` shell). This module maps [`egui::Key`] + modifiers for the egui host.

use eframe::egui::{Key, Modifiers};

pub use spec_chum_host::keymap::{Chord, CAPS, MAPPING_DOC, SYM};

/// Map an egui key + modifiers to Spectrum matrix chords.
///
/// Returns `None` when the key is unused (e.g. F-keys). Punctuation that needs
/// Symbol Shift overrides Caps from the host Shift key. Arrow keys and Tab are
/// joystick-routed in `sync_keyboard` (matrix injection skipped); direct
/// `chord_for` calls still return Caps cursor chords for arrows.
#[must_use]
pub fn chord_for(key: Key, modifiers: Modifiers) -> Option<Chord> {
    // Arrow keys → Spectrum cursor (Caps + 5/6/7/8), regardless of Shift.
    // Matrix injection is skipped in `sync_keyboard`; joystick routing applies them.
    match key {
        Key::ArrowLeft => return Some(Chord::with_caps(3, 4)), // 5
        Key::ArrowDown => return Some(Chord::with_caps(4, 4)), // 6
        Key::ArrowUp => return Some(Chord::with_caps(4, 3)),   // 7
        Key::ArrowRight => return Some(Chord::with_caps(4, 2)), // 8
        Key::Backspace => return Some(Chord::with_caps(4, 0)), // Caps+0 delete
        _ => {}
    }

    // Symbol-layer punctuation (host Shift/Option must not inject Caps).
    if let Some(ch) = punct_chord(key, modifiers) {
        return Some(ch);
    }

    // Alphanumeric + Enter/Space — plain matrix; Caps/Sym come from modifiers.
    letter_digit(key).map(|(row, bit)| Chord::single(row, bit))
}

/// Modifier keys alone (when no punctuation override is active).
#[must_use]
pub fn modifier_keys(modifiers: Modifiers, suppress_caps: bool) -> Vec<(usize, u8)> {
    spec_chum_host::keymap::modifier_keys(
        modifiers.shift,
        modifiers.alt,
        modifiers.ctrl,
        suppress_caps,
    )
}

/// True when this key event owns Symbol/Caps itself (punctuation / arrows).
#[must_use]
pub fn suppresses_modifier_caps(key: Key) -> bool {
    matches!(
        key,
        Key::ArrowLeft
            | Key::ArrowRight
            | Key::ArrowUp
            | Key::ArrowDown
            | Key::Backspace
            | Key::Quote
            | Key::Semicolon
            | Key::Comma
            | Key::Period
            | Key::Slash
            | Key::Minus
            | Key::Equals
            | Key::OpenBracket
            | Key::CloseBracket
            | Key::Backslash
            | Key::Backtick
    )
}

fn letter_digit(key: Key) -> Option<(usize, u8)> {
    Some(match key {
        Key::Num1 => (3, 0),
        Key::Num2 => (3, 1),
        Key::Num3 => (3, 2),
        Key::Num4 => (3, 3),
        Key::Num5 => (3, 4),
        Key::Num6 => (4, 4),
        Key::Num7 => (4, 3),
        Key::Num8 => (4, 2),
        Key::Num9 => (4, 1),
        Key::Num0 => (4, 0),
        Key::Q => (2, 0),
        Key::W => (2, 1),
        Key::E => (2, 2),
        Key::R => (2, 3),
        Key::T => (2, 4),
        Key::Y => (5, 4),
        Key::U => (5, 3),
        Key::I => (5, 2),
        Key::O => (5, 1),
        Key::P => (5, 0),
        Key::A => (1, 0),
        Key::S => (1, 1),
        Key::D => (1, 2),
        Key::F => (1, 3),
        Key::G => (1, 4),
        Key::H => (6, 4),
        Key::J => (6, 3),
        Key::K => (6, 2),
        Key::L => (6, 1),
        Key::Enter => (6, 0),
        Key::Z => (0, 1),
        Key::X => (0, 2),
        Key::C => (0, 3),
        Key::V => (0, 4),
        Key::B => (7, 4),
        Key::N => (7, 3),
        Key::M => (7, 2),
        Key::Space => (7, 0),
        _ => return None,
    })
}

fn punct_chord(key: Key, modifiers: Modifiers) -> Option<Chord> {
    // US Mac layout approximations → Spectrum Symbol layer.
    match key {
        // ' → Sym+7 ; " → Sym+P
        Key::Quote => {
            if modifiers.shift {
                Some(Chord::with_sym(5, 0)) // P → "
            } else {
                Some(Chord::with_sym(4, 3)) // 7 → '
            }
        }
        // ; → Sym+O ; : → Sym+Z
        Key::Semicolon => {
            if modifiers.shift {
                Some(Chord::with_sym(0, 1)) // Z → :
            } else {
                Some(Chord::with_sym(5, 1)) // O → ;
            }
        }
        // , → Sym+N ; < → Sym+R
        Key::Comma => {
            if modifiers.shift {
                Some(Chord::with_sym(2, 3)) // R → <
            } else {
                Some(Chord::with_sym(7, 3)) // N → ,
            }
        }
        // . → Sym+M ; > → Sym+T
        Key::Period => {
            if modifiers.shift {
                Some(Chord::with_sym(2, 4)) // T → >
            } else {
                Some(Chord::with_sym(7, 2)) // M → .
            }
        }
        // / → Sym+V ; ? → Sym+C
        Key::Slash => {
            if modifiers.shift {
                Some(Chord::with_sym(0, 3)) // C → ?
            } else {
                Some(Chord::with_sym(0, 4)) // V → /
            }
        }
        // - → Sym+J ; _ → Sym+0
        Key::Minus => {
            if modifiers.shift {
                Some(Chord::with_sym(4, 0)) // 0 → _
            } else {
                Some(Chord::with_sym(6, 3)) // J → -
            }
        }
        // = → Sym+L ; + → Sym+K
        Key::Equals => {
            if modifiers.shift {
                Some(Chord::with_sym(6, 2)) // K → +
            } else {
                Some(Chord::with_sym(6, 1)) // L → =
            }
        }
        // [ → Sym+Y ; { → Sym+F (approx)
        Key::OpenBracket => {
            if modifiers.shift {
                Some(Chord::with_sym(1, 3)) // F → {
            } else {
                Some(Chord::with_sym(5, 4)) // Y → [
            }
        }
        // ] → Sym+U ; } → Sym+G
        Key::CloseBracket => {
            if modifiers.shift {
                Some(Chord::with_sym(1, 4)) // G → }
            } else {
                Some(Chord::with_sym(5, 3)) // U → ]
            }
        }
        // \ → Sym+D (approx ★ on Spectrum) — map to Sym+D; | → Sym+S
        Key::Backslash => {
            if modifiers.shift {
                Some(Chord::with_sym(1, 1)) // S → |
            } else {
                Some(Chord::with_sym(1, 2)) // D
            }
        }
        // ` → Sym+X (₤/© variants vary); ~ → Sym+A
        Key::Backtick => {
            if modifiers.shift {
                Some(Chord::with_sym(1, 0)) // A → ~
            } else {
                Some(Chord::with_sym(0, 2)) // X
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mods_shift() -> Modifiers {
        Modifiers {
            shift: true,
            ..Modifiers::default()
        }
    }

    #[test]
    fn quote_shift_is_symbol_p() {
        let c = chord_for(Key::Quote, mods_shift()).unwrap();
        assert_eq!(c.keys, vec![SYM, (5, 0)]);
        assert!(suppresses_modifier_caps(Key::Quote));
    }

    #[test]
    fn quote_alone_is_symbol_7() {
        let c = chord_for(Key::Quote, Modifiers::default()).unwrap();
        assert_eq!(c.keys, vec![SYM, (4, 3)]);
    }

    #[test]
    fn arrows_are_caps_cursor() {
        assert_eq!(
            chord_for(Key::ArrowLeft, Modifiers::default())
                .unwrap()
                .keys,
            vec![CAPS, (3, 4)]
        );
        assert_eq!(
            chord_for(Key::ArrowDown, Modifiers::default())
                .unwrap()
                .keys,
            vec![CAPS, (4, 4)]
        );
        assert_eq!(
            chord_for(Key::ArrowUp, Modifiers::default()).unwrap().keys,
            vec![CAPS, (4, 3)]
        );
        assert_eq!(
            chord_for(Key::ArrowRight, Modifiers::default())
                .unwrap()
                .keys,
            vec![CAPS, (4, 2)]
        );
    }

    #[test]
    fn letter_j_is_load_key() {
        assert_eq!(
            chord_for(Key::J, Modifiers::default()).unwrap().keys,
            vec![(6, 3)]
        );
    }

    #[test]
    fn shift_modifier_is_caps_unless_suppressed() {
        let m = mods_shift();
        assert_eq!(modifier_keys(m, false), vec![CAPS]);
        assert!(modifier_keys(m, true).is_empty());
    }
}
