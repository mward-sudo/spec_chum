import AppKit

/// Mac → Spectrum matrix mapping (parity with `crates/app/src/keymap.rs`).
///
/// Uses hardware key codes so mapping works even when synthetic / empty
/// `characters` are present (flagsChanged rebuilds, local monitors).
enum SpectrumKeymap {
    static let caps: (UInt32, UInt32) = (0, 0)
    static let sym: (UInt32, UInt32) = (7, 1)

    /// Matrix chords for one physical key + current modifiers.
    static func chords(keyCode: UInt16, flags: NSEvent.ModifierFlags) -> [(UInt32, UInt32)] {
        let shift = flags.contains(.shift)

        // Arrows / delete → Caps + digit (ignore host Shift as extra Caps).
        switch keyCode {
        case 123: return [caps, (3, 4)] // left → Caps+5
        case 125: return [caps, (4, 4)] // down → Caps+6
        case 126: return [caps, (4, 3)] // up → Caps+7
        case 124: return [caps, (4, 2)] // right → Caps+8
        case 51: return [caps, (4, 0)] // delete → Caps+0
        default: break
        }

        // Symbol-layer punctuation (host Shift must not also inject Caps).
        if let punct = punctChord(keyCode: keyCode, shift: shift) {
            return punct
        }

        var keys: [(UInt32, UInt32)] = []
        if shift {
            keys.append(caps)
        }
        if flags.contains(.option) || flags.contains(.control) {
            keys.append(sym)
        }

        switch keyCode {
        case 36: // return
            keys.append((6, 0))
        case 49: // space
            keys.append((7, 0))
        default:
            if let pair = letterDigit(keyCode: keyCode) {
                keys.append(pair)
            }
        }
        return keys
    }

    /// Caps / Symbol when held alone (no punctuation/arrow owning the chord).
    static func modifierKeys(flags: NSEvent.ModifierFlags, suppressCaps: Bool) -> [(UInt32, UInt32)] {
        var out: [(UInt32, UInt32)] = []
        if flags.contains(.shift), !suppressCaps {
            out.append(caps)
        }
        if flags.contains(.option) || flags.contains(.control) {
            out.append(sym)
        }
        return out
    }

    static func suppressesModifierCaps(keyCode: UInt16) -> Bool {
        switch keyCode {
        case 123, 124, 125, 126, 51: // arrows, delete
            return true
        case 39, 41, 43, 47, 44, 27, 24, 33, 30, 42, 50: // punct
            return true
        default:
            return false
        }
    }

    /// Kempston mask from held ANSI key codes (egui: arrows + Tab fire).
    /// Bits: 0=right, 1=left, 2=down, 3=up, 4=fire.
    static func kempstonMask(held: Set<UInt16>) -> UInt32 {
        var mask: UInt32 = 0
        if held.contains(124) { mask |= 1 << 0 } // right
        if held.contains(123) { mask |= 1 << 1 } // left
        if held.contains(125) { mask |= 1 << 2 } // down
        if held.contains(126) { mask |= 1 << 3 } // up
        if held.contains(48) { mask |= 1 << 4 } // Tab fire
        return mask
    }

    // MARK: - Private

    private static func letterDigit(keyCode: UInt16) -> (UInt32, UInt32)? {
        // ANSI US key codes (same as Carbon / HIToolbox).
        switch keyCode {
        case 18: return (3, 0) // 1
        case 19: return (3, 1) // 2
        case 20: return (3, 2) // 3
        case 21: return (3, 3) // 4
        case 23: return (3, 4) // 5
        case 22: return (4, 4) // 6
        case 26: return (4, 3) // 7
        case 28: return (4, 2) // 8
        case 25: return (4, 1) // 9
        case 29: return (4, 0) // 0
        case 12: return (2, 0) // Q
        case 13: return (2, 1) // W
        case 14: return (2, 2) // E
        case 15: return (2, 3) // R
        case 17: return (2, 4) // T
        case 16: return (5, 4) // Y
        case 32: return (5, 3) // U
        case 34: return (5, 2) // I
        case 31: return (5, 1) // O
        case 35: return (5, 0) // P
        case 0: return (1, 0) // A
        case 1: return (1, 1) // S
        case 2: return (1, 2) // D
        case 3: return (1, 3) // F
        case 5: return (1, 4) // G
        case 4: return (6, 4) // H
        case 38: return (6, 3) // J
        case 40: return (6, 2) // K
        case 37: return (6, 1) // L
        case 6: return (0, 1) // Z
        case 7: return (0, 2) // X
        case 8: return (0, 3) // C
        case 9: return (0, 4) // V
        case 11: return (7, 4) // B
        case 45: return (7, 3) // N
        case 46: return (7, 2) // M
        default: return nil
        }
    }

    private static func punctChord(keyCode: UInt16, shift: Bool) -> [(UInt32, UInt32)]? {
        switch keyCode {
        case 39: // ' / "
            return shift ? [sym, (5, 0)] : [sym, (4, 3)] // P / 7
        case 41: // ; / :
            return shift ? [sym, (0, 1)] : [sym, (5, 1)] // Z / O
        case 43: // , / <
            return shift ? [sym, (2, 3)] : [sym, (7, 3)] // R / N
        case 47: // . / >
            return shift ? [sym, (2, 4)] : [sym, (7, 2)] // T / M
        case 44: // / / ?
            return shift ? [sym, (0, 3)] : [sym, (0, 4)] // C / V
        case 27: // - / _
            return shift ? [sym, (4, 0)] : [sym, (6, 3)] // 0 / J
        case 24: // = / +
            return shift ? [sym, (6, 2)] : [sym, (6, 1)] // K / L
        case 33: // [ / {
            return shift ? [sym, (1, 3)] : [sym, (5, 4)] // F / Y
        case 30: // ] / }
            return shift ? [sym, (1, 4)] : [sym, (5, 3)] // G / U
        case 42: // \ / |
            return shift ? [sym, (1, 1)] : [sym, (1, 2)] // S / D
        case 50: // ` / ~
            return shift ? [sym, (1, 0)] : [sym, (0, 2)] // A / X
        default:
            return nil
        }
    }
}
