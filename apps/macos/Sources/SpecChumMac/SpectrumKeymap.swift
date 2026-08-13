import AppKit
import SwiftUI

/// Spectrum matrix helpers for a minimal Mac keymap (vertical slice).
enum SpectrumKeymap {
    static let caps: (UInt32, UInt32) = (0, 0)
    static let sym: (UInt32, UInt32) = (7, 1)

    /// Map a Mac key code / characters to matrix positions held while the key is down.
    static func chords(for event: NSEvent) -> [(UInt32, UInt32)] {
        var keys: [(UInt32, UInt32)] = []
        let flags = event.modifierFlags

        if flags.contains(.shift) {
            keys.append(caps)
        }
        if flags.contains(.option) || flags.contains(.control) {
            keys.append(sym)
        }

        switch event.keyCode {
        case 123: // left
            return [caps, (3, 4)]
        case 125: // down
            return [caps, (4, 4)]
        case 126: // up
            return [caps, (4, 3)]
        case 124: // right
            return [caps, (4, 2)]
        case 51: // delete
            return [caps, (4, 0)]
        case 36: // return
            keys.append((6, 0))
            return keys
        case 49: // space
            keys.append((7, 0))
            return keys
        default:
            break
        }

        if let chars = event.charactersIgnoringModifiers?.lowercased(), let ch = chars.first {
            if let pair = letterDigit(ch) {
                keys.append(pair)
            }
        }
        return keys
    }

    private static func letterDigit(_ ch: Character) -> (UInt32, UInt32)? {
        switch ch {
        case "z": return (0, 1)
        case "x": return (0, 2)
        case "c": return (0, 3)
        case "v": return (0, 4)
        case "a": return (1, 0)
        case "s": return (1, 1)
        case "d": return (1, 2)
        case "f": return (1, 3)
        case "g": return (1, 4)
        case "q": return (2, 0)
        case "w": return (2, 1)
        case "e": return (2, 2)
        case "r": return (2, 3)
        case "t": return (2, 4)
        case "1": return (3, 0)
        case "2": return (3, 1)
        case "3": return (3, 2)
        case "4": return (3, 3)
        case "5": return (3, 4)
        case "0": return (4, 0)
        case "9": return (4, 1)
        case "8": return (4, 2)
        case "7": return (4, 3)
        case "6": return (4, 4)
        case "p": return (5, 0)
        case "o": return (5, 1)
        case "i": return (5, 2)
        case "u": return (5, 3)
        case "y": return (5, 4)
        case "l": return (6, 1)
        case "k": return (6, 2)
        case "j": return (6, 3)
        case "h": return (6, 4)
        case "m": return (7, 2)
        case "n": return (7, 3)
        case "b": return (7, 4)
        default: return nil
        }
    }
}
