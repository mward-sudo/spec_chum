import AppKit
import Foundation
import GameController
import IOSurface
import UniformTypeIdentifiers
import CSpecChumHost

extension HostBridge {
    func syncTapePublished() {
        guard let handle else { return }
        let playing = sc_tape_playing(handle) != 0
        let tape = sc_has_tape(handle) != 0
        // Avoid @Published writes every tick — they re-enter SwiftUI and can
        // reset TimelineView(.periodic(from: .now)) into a turbo frame loop.
        if playing != tapePlaying {
            if tapePlaying && !playing && instantFlashActive {
                instantFlashActive = false
                setFlashLoad(false)
            }
            tapePlaying = playing
        }
        if tape != hasTape {
            hasTape = tape
        }
        progressPublishCounter &+= 1
        if progressPublishCounter % 12 == 0 || !tape {
            refreshTapeProgress()
        }
    }

    func refreshTapeProgress() {
        guard let handle, sc_has_tape(handle) != 0 else {
            if tapeFraction != nil {
                tapeFraction = nil
                tapeBlockLabel = ""
            }
            return
        }
        var block: UInt32 = 0
        var blocks: UInt32 = 0
        var pulse: UInt32 = 0
        var pulses: UInt32 = 0
        guard sc_tape_progress(handle, &block, &blocks, &pulse, &pulses) == 0, blocks > 0 else {
            return
        }
        let within = pulses == 0 ? 0.0 : Double(min(pulse, pulses)) / Double(pulses)
        let frac = min(1.0, (Double(min(block, blocks)) + within) / Double(blocks))
        let label = "Tape \(min(block + 1, blocks))/\(blocks)"
        if tapeFraction.map({ abs($0 - frac) > 0.002 }) ?? true {
            tapeFraction = frac
        }
        if label != tapeBlockLabel {
            tapeBlockLabel = label
        }
    }
    func openTape(at url: URL) {
        guard let handle else { return }
        let ok = url.path.withCString { sc_open_tape(handle, $0) }
        if ok != 0 {
            status = HostBridge.takeLastError() ?? "Tape open failed"
        } else {
            mediaTitle = url.lastPathComponent
            // Insert always leaves flash-load off; Instant turns it on ephemerally.
            instantFlashActive = false
            setFlashLoad(false)
            refreshStatus()
            hasTape = true
            tapePlaying = false
            refreshTapeProgress()
            noteRecentFile(url)
        }
    }

    func typeLoadQuotes(withCode: Bool = false) {
        beginTypeLoadQuotes(withCode: withCode, pendingPlay: false)
    }

    /// Instant: always prompt for an image, then flash-load + Type LOAD "" + Play.
    /// Never silently reuses the currently inserted tape. Play alone stays EAR-only.
    func instantLoadTape() {
        presentInstantMediaPanel()
    }

    /// Tape-only filters; Instant is flash Type LOAD. Use Open for `.dsk` / `.trd`.
    func presentInstantMediaPanel() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.allowedContentTypes = [
            UTType(filenameExtension: "tap") ?? .data,
            UTType(filenameExtension: "tzx") ?? .data,
        ]
        panel.title = "Instant — Open TAP / TZX"
        guard panel.runModal() == .OK, let url = panel.url else {
            status = "Instant cancelled"
            return
        }
        // Defensive: Instant is tape-oriented; never fake Type LOAD for disks.
        if url.pathExtension.lowercased() == "dsk" {
            openDsk(at: url)
            status = "DSK inserted — use +3 Loader / +3DOS"
            return
        }
        openTape(at: url)
        guard hasTape else { return }
        beginInstantLoadAfterInsert()
    }

    /// Flash on → Type LOAD "" → Play (flash cleared when deck stops / Pause / Play).
    func beginInstantLoadAfterInsert() {
        instantFlashActive = true
        setFlashLoad(true)

        if let r = regs(), r.pc == 0x056C {
            pendingInstantPlay = false
            playTapeKeepingFlash()
            status = "Instant: flash-loading at LD-BYTES"
            return
        }

        beginTypeLoadQuotes(withCode: false, pendingPlay: true)
        status = "Instant: typing LOAD \"\" then flash-load Play"
    }

    func beginTypeLoadQuotes(withCode: Bool, pendingPlay: Bool) {
        pendingInstantPlay = pendingPlay
        switch model {
        case .spectrum16K, .spectrum48, .timexTC2048, .timexTS2068:
            keyScript = LoadKeyScript.loadQuotes48k(withCode: withCode)
            status = withCode
                ? "Typing LOAD \"\" CODE — press Tape → Play when border goes red/cyan"
                : "Typing LOAD \"\" — press Tape → Play when the border goes red/cyan"
        case .spectrumPlus2A:
            keyScript = LoadKeyScript.loadQuotesPlus2A(withCode: withCode)
            status = withCode
                ? "Typing 48 BASIC LOAD \"\" CODE — press Tape → Play when border goes red/cyan"
                : "Selecting +2A tape Loader — press Tape → Play when border goes red/cyan"
        case .spectrum128, .spectrumPlus2, .spectrumPlus3, .pentagon128:
            keyScript = LoadKeyScript.loadQuotes128OrPlus3(withCode: withCode)
            status = withCode
                ? "Typing 48 BASIC LOAD \"\" CODE — press Tape → Play when border goes red/cyan"
                : "Typing 48 BASIC LOAD \"\" — press Tape → Play when the border goes red/cyan"
        }
    }

    /// NSOpenPanel for tape (`.dsk` on +3, `.trd` on Beta-capable models).

    func playTape() {
        instantFlashActive = false
        setFlashLoad(false)
        playTapeKeepingFlash()
        if tapePlaying {
            status = "Tape playing (EAR)"
        }
    }

    /// Start the deck without clearing flash-load (Instant only).
    func playTapeKeepingFlash() {
        guard let handle else { return }
        ensureAudioOutput()
        if sc_tape_play(handle) != 0 {
            status = HostBridge.takeLastError() ?? "Play failed"
        } else {
            refreshStatus()
            tapePlaying = true
        }
    }

    func pauseTape() {
        guard let handle else { return }
        _ = sc_tape_pause(handle)
        instantFlashActive = false
        setFlashLoad(false)
        refreshStatus()
        tapePlaying = false
    }

    func rewindTape() {
        guard let handle else { return }
        _ = sc_tape_rewind(handle)
        instantFlashActive = false
        setFlashLoad(false)
        refreshStatus()
        tapePlaying = false
    }

    func syncTapeLoadOptionsFromHost() {
        guard let handle else { return }
        var flash: Int32 = 0
        var speed: UInt32 = 1
        var experience: Int32 = 0
        guard sc_tape_get_load_options_ex(handle, &flash, &speed, &experience) == 0 else { return }
        suppressTapeOptsPush = true
        instantLoad = flash != 0
        tapeSpeed = max(1, min(speed, 64))
        experienceLoad = experience != 0
        suppressTapeOptsPush = false
    }

    func setFlashLoad(_ on: Bool) {
        if on {
            experienceLoad = false
        }
        guard instantLoad != on else {
            pushTapeLoadOptions()
            return
        }
        instantLoad = on
    }

    func pushTapeLoadOptions() {
        guard let handle, !suppressTapeOptsPush else { return }
        _ = sc_tape_set_load_options_ex(
            handle,
            instantLoad ? 1 : 0,
            max(1, min(tapeSpeed, 64)),
            experienceLoad ? 1 : 0
        )
        refreshStatus()
    }

}

/// Keyword-mode `LOAD ""` [CODE] — mirrors egui `KeyScript` / ROM debounce.
struct LoadKeyScript {
    struct Step {
        let keys: [(UInt32, UInt32)]
        let frames: UInt32
    }

    var steps: [Step]
    var stepIndex: Int = 0
    var framesLeft: UInt32 = 0

    private static let press: UInt32 = 10
    private static let gap: UInt32 = 5
    /// Idle after menu → 48 BASIC Enter (egui `MENU_TO_48_BASIC_WAIT`).
    private static let menuTo48BasicWait: UInt32 = 120
    private static let caps: (UInt32, UInt32) = (0, 0)
    private static let sym: (UInt32, UInt32) = (7, 1)

    static func loadQuotes48k(withCode: Bool) -> LoadKeyScript {
        LoadKeyScript(steps: loadQuotes48kSteps(withCode: withCode))
    }

    /// 128K / +3 boot menu → 48 BASIC, then keyword LOAD (egui parity; #144).
    static func loadQuotes128OrPlus3(withCode: Bool) -> LoadKeyScript {
        let empty: [(UInt32, UInt32)] = []
        let cursorDown: [(UInt32, UInt32)] = [caps, (4, 4)]
        let enter: [(UInt32, UInt32)] = [(6, 0)]
        var steps: [Step] = []
        for _ in 0..<3 {
            steps.append(Step(keys: cursorDown, frames: press))
            steps.append(Step(keys: empty, frames: gap))
        }
        steps.append(Step(keys: enter, frames: press))
        steps.append(Step(keys: empty, frames: menuTo48BasicWait))
        steps.append(contentsOf: loadQuotes48kSteps(withCode: withCode))
        return LoadKeyScript(steps: steps)
    }

    /// +2A: menu Loader is tape — Enter alone for PROGRAM; CODE via 48 BASIC (#145).
    static func loadQuotesPlus2A(withCode: Bool) -> LoadKeyScript {
        if withCode {
            return loadQuotes128OrPlus3(withCode: true)
        }
        let empty: [(UInt32, UInt32)] = []
        let enter: [(UInt32, UInt32)] = [(6, 0)]
        return LoadKeyScript(steps: [
            Step(keys: enter, frames: press),
            Step(keys: empty, frames: menuTo48BasicWait),
        ])
    }

    private static func loadQuotes48kSteps(withCode: Bool) -> [Step] {
        let j: [(UInt32, UInt32)] = [(6, 3)]
        let quote: [(UInt32, UInt32)] = [sym, (5, 0)]
        let extend: [(UInt32, UInt32)] = [caps, sym]
        let codeI: [(UInt32, UInt32)] = [(5, 2)]
        let enter: [(UInt32, UInt32)] = [(6, 0)]
        let empty: [(UInt32, UInt32)] = []
        var steps: [Step] = [
            Step(keys: j, frames: press),
            Step(keys: empty, frames: gap),
            Step(keys: quote, frames: press),
            Step(keys: empty, frames: gap),
            Step(keys: quote, frames: press),
            Step(keys: empty, frames: gap),
        ]
        if withCode {
            steps.append(Step(keys: extend, frames: press))
            steps.append(Step(keys: empty, frames: gap))
            steps.append(Step(keys: codeI, frames: press))
            steps.append(Step(keys: empty, frames: gap))
        }
        steps.append(Step(keys: enter, frames: press))
        steps.append(Step(keys: empty, frames: 15))
        return steps
    }
}
