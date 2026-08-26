//! Bevy KeyCode → Spectrum matrix chords (adapted from `crates/app` keymap).

use bevy::prelude::{ButtonInput, KeyCode};

pub const CAPS: (usize, u8) = (0, 0);
pub const SYM: (usize, u8) = (7, 1);

/// Collect pressed matrix positions for the current host keyboard state.
#[must_use]
pub fn matrix_from_bevy(keys: &ButtonInput<KeyCode>) -> Vec<(usize, u8)> {
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    let alt = keys.pressed(KeyCode::AltLeft) || keys.pressed(KeyCode::AltRight);
    let ctrl = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);

    let mut out = Vec::new();
    let mut suppress_caps = false;

    for code in keys.get_pressed() {
        if let Some(chord) = chord_for(*code, shift, alt || ctrl) {
            if chord_suppresses_caps(*code) {
                suppress_caps = true;
            }
            for (row, bit) in chord {
                push_unique(&mut out, (row, bit));
            }
        }
    }

    if shift && !suppress_caps {
        push_unique(&mut out, CAPS);
    }
    if alt || ctrl {
        push_unique(&mut out, SYM);
    }

    out
}

fn push_unique(out: &mut Vec<(usize, u8)>, key: (usize, u8)) {
    if !out.contains(&key) {
        out.push(key);
    }
}

fn chord_suppresses_caps(code: KeyCode) -> bool {
    matches!(
        code,
        KeyCode::ArrowLeft
            | KeyCode::ArrowRight
            | KeyCode::ArrowUp
            | KeyCode::ArrowDown
            | KeyCode::Backspace
            | KeyCode::Quote
            | KeyCode::Semicolon
            | KeyCode::Comma
            | KeyCode::Period
            | KeyCode::Slash
            | KeyCode::Minus
            | KeyCode::Equal
            | KeyCode::BracketLeft
            | KeyCode::BracketRight
            | KeyCode::Backslash
            | KeyCode::Backquote
    )
}

fn chord_for(code: KeyCode, shift: bool, _sym_mod: bool) -> Option<Vec<(usize, u8)>> {
    match code {
        KeyCode::ArrowLeft => return Some(vec![CAPS, (3, 4)]),
        KeyCode::ArrowDown => return Some(vec![CAPS, (4, 4)]),
        KeyCode::ArrowUp => return Some(vec![CAPS, (4, 3)]),
        KeyCode::ArrowRight => return Some(vec![CAPS, (4, 2)]),
        KeyCode::Backspace => return Some(vec![CAPS, (4, 0)]),
        KeyCode::Quote => {
            return Some(if shift {
                vec![SYM, (5, 0)] // "
            } else {
                vec![SYM, (4, 3)] // '
            });
        }
        KeyCode::Semicolon => {
            return Some(if shift {
                vec![SYM, (0, 1)] // :
            } else {
                vec![SYM, (5, 1)] // ;
            });
        }
        KeyCode::Comma => {
            return Some(if shift {
                vec![SYM, (2, 3)] // <
            } else {
                vec![SYM, (7, 3)] // ,
            });
        }
        KeyCode::Period => {
            return Some(if shift {
                vec![SYM, (2, 4)] // >
            } else {
                vec![SYM, (7, 2)] // .
            });
        }
        KeyCode::Slash => {
            return Some(if shift {
                vec![SYM, (0, 3)] // ? (Sym+C)
            } else {
                // Sym+V → /  (row0 bit4). (7,4) is B → * — wrong.
                vec![SYM, (0, 4)]
            });
        }
        _ => {}
    }

    letter_digit(code).map(|(row, bit)| vec![(row, bit)])
}

fn letter_digit(code: KeyCode) -> Option<(usize, u8)> {
    Some(match code {
        KeyCode::Digit1 => (3, 0),
        KeyCode::Digit2 => (3, 1),
        KeyCode::Digit3 => (3, 2),
        KeyCode::Digit4 => (3, 3),
        KeyCode::Digit5 => (3, 4),
        KeyCode::Digit6 => (4, 4),
        KeyCode::Digit7 => (4, 3),
        KeyCode::Digit8 => (4, 2),
        KeyCode::Digit9 => (4, 1),
        KeyCode::Digit0 => (4, 0),
        KeyCode::KeyQ => (2, 0),
        KeyCode::KeyW => (2, 1),
        KeyCode::KeyE => (2, 2),
        KeyCode::KeyR => (2, 3),
        KeyCode::KeyT => (2, 4),
        KeyCode::KeyY => (5, 4),
        KeyCode::KeyU => (5, 3),
        KeyCode::KeyI => (5, 2),
        KeyCode::KeyO => (5, 1),
        KeyCode::KeyP => (5, 0),
        KeyCode::KeyA => (1, 0),
        KeyCode::KeyS => (1, 1),
        KeyCode::KeyD => (1, 2),
        KeyCode::KeyF => (1, 3),
        KeyCode::KeyG => (1, 4),
        KeyCode::KeyH => (6, 4),
        KeyCode::KeyJ => (6, 3),
        KeyCode::KeyK => (6, 2),
        KeyCode::KeyL => (6, 1),
        KeyCode::Enter => (6, 0),
        KeyCode::KeyZ => (0, 1),
        KeyCode::KeyX => (0, 2),
        KeyCode::KeyC => (0, 3),
        KeyCode::KeyV => (0, 4),
        KeyCode::KeyB => (7, 4),
        KeyCode::KeyN => (7, 3),
        KeyCode::KeyM => (7, 2),
        KeyCode::Space => (7, 0),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letter_q_is_row2_bit0() {
        assert_eq!(letter_digit(KeyCode::KeyQ), Some((2, 0)));
    }

    #[test]
    fn space_is_row7_bit0() {
        assert_eq!(letter_digit(KeyCode::Space), Some((7, 0)));
    }

    #[test]
    fn quote_shift_is_sym_p() {
        let chord = chord_for(KeyCode::Quote, true, false).expect("chord");
        assert!(chord.contains(&SYM));
        assert!(chord.contains(&(5, 0)));
    }
}
