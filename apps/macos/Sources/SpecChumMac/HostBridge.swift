import AppKit
import CSpecChumHost
import Foundation
import GameController
import UniformTypeIdentifiers

/// Thin Swift wrapper around the Spec Chum C host API.
final class HostBridge: ObservableObject {
    /// Joystick presentation — matches `sc_set_joystick_mode` (default Kempston).
    enum JoystickMode: UInt32 {
        case kempston = 0
        case sinclairLeft = 1
        case sinclairRight = 2
        case cursor = 3
    }

    /// Digital stick threshold for left thumbstick axes (~egui gilrs parity).
    private static let stickThreshold: Float = 0.5
    enum Model: UInt32, CaseIterable, Identifiable {
        case spectrum48 = 0
        case spectrum128 = 1
        case spectrumPlus3 = 2

        var id: UInt32 { rawValue }

        var title: String {
            switch self {
            case .spectrum48: "Spectrum 48K"
            case .spectrum128: "Spectrum 128K"
            case .spectrumPlus3: "Spectrum +2A/+3"
            }
        }
    }

    @Published private(set) var status: String = "Starting…"
    @Published private(set) var tapePlaying: Bool = false
    @Published private(set) var hasTape: Bool = false
    /// Instant flash-load when true; EAR bitstream when false.
    @Published var instantLoad: Bool = true {
        didSet { pushTapeLoadOptions() }
    }
    /// EAR speed multiplier presets (also applied when instant is off).
    @Published var tapeSpeed: UInt32 = 1 {
        didSet { pushTapeLoadOptions() }
    }
    /// 0...1 tape position for ProgressView; nil when no tape.
    @Published private(set) var tapeFraction: Double?
    @Published private(set) var tapeBlockLabel: String = ""
    @Published var paused: Bool = false {
        didSet {
            guard let handle, !suppressPausedPush else { return }
            sc_set_paused(handle, paused ? 1 : 0)
        }
    }
    @Published var showInspector: Bool = false
    @Published private(set) var debugPc: UInt16 = 0
    @Published private(set) var debugSp: UInt16 = 0
    @Published private(set) var debugAf: UInt16 = 0
    @Published private(set) var inspectJsonPreview: String = ""
    @Published var model: Model = .spectrum48 {
        didSet {
            guard let handle, oldValue != model else { return }
            _ = sc_set_model(handle, model.rawValue)
            tryAutoloadRom()
            pushTapeLoadOptions()
            refreshStatus()
        }
    }

    private var handle: UnsafeMutableRawPointer?
    private let romSearchRoots: [URL]
    /// Wall-clock gate so SwiftUI over-scheduling cannot turbo the Spectrum.
    private var lastFrameUptime: TimeInterval = 0
    private static let framePeriod: TimeInterval = 1.0 / 50.0
    /// After a hitch, advance at most this many Spectrum frames per host tick.
    private static let maxCatchUpFrames = 2
    private let audio = TapeAudioPlayer()
    /// Publish progress at most ~4 Hz to avoid SwiftUI churn.
    private var progressPublishCounter: UInt32 = 0
    /// Arrows + Tab → Kempston bits (egui parity); OR’d with gamepad each frame.
    private(set) var keyboardJoystickMask: UInt32 = 0
    private var joystickModeApplied = false
    private var connectObserver: NSObjectProtocol?
    private var disconnectObserver: NSObjectProtocol?

    init(romSearchRoots: [URL] = HostBridge.defaultRomRoots()) {
        self.romSearchRoots = romSearchRoots
        handle = sc_create(Model.spectrum48.rawValue, 1)
        if handle == nil {
            status = HostBridge.takeLastError() ?? "Failed to create host session"
            return
        }
        sc_debug_init_from_env()
        tryAutoloadRom()
        refreshStatus()
        syncTapeLoadOptionsFromHost()
        applyJoystickMode(.kempston)
        startGamepadDiscovery()
        let rate = Double(sc_audio_sample_rate(handle))
        audio.ensureStarted(sampleRate: rate > 0 ? rate : 44100)
    }

    deinit {
        if let connectObserver {
            NotificationCenter.default.removeObserver(connectObserver)
        }
        if let disconnectObserver {
            NotificationCenter.default.removeObserver(disconnectObserver)
        }
        GCController.stopWirelessControllerDiscovery()
        audio.stop()
        if let handle {
            sc_destroy(handle)
        }
    }

    /// Run Spectrum frame(s) capped to ~50 Hz wall clock (egui throttle parity).
    /// Returns whether at least one `sc_run_frame` ran.
    @discardableResult
    func runFrame() -> Bool {
        guard let handle else { return false }
        let now = ProcessInfo.processInfo.systemUptime
        if lastFrameUptime == 0 {
            lastFrameUptime = now
            pushJoystick()
            sc_run_frame(handle)
            syncTapePublished()
            enqueueAudio()
            return true
        }

        var ran = 0
        while now - lastFrameUptime >= Self.framePeriod, ran < Self.maxCatchUpFrames {
            pushJoystick()
            sc_run_frame(handle)
            lastFrameUptime += Self.framePeriod
            ran += 1
        }
        // If we stalled longer than the catch-up window, resync to wall clock.
        if now - lastFrameUptime > Self.framePeriod * Double(Self.maxCatchUpFrames) {
            lastFrameUptime = now
        }
        if ran > 0 {
            syncTapePublished()
            enqueueAudio()
            return true
        }
        return false
    }

    private func enqueueAudio() {
        guard let handle else { return }
        let n = Int(sc_audio_frames(handle))
        guard n > 0, let ptr = sc_audio_ptr(handle) else { return }
        audio.schedule(samples: ptr, count: n)
    }

    private func syncTapePublished() {
        guard let handle else { return }
        let playing = sc_tape_playing(handle) != 0
        let tape = sc_has_tape(handle) != 0
        // Avoid @Published writes every tick — they re-enter SwiftUI and can
        // reset TimelineView(.periodic(from: .now)) into a turbo frame loop.
        if playing != tapePlaying {
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

    private func refreshTapeProgress() {
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

    /// Copy current RGBA framebuffer into an `NSImage` (nearest-neighbor friendly).
    func makeFrameImage() -> NSImage? {
        guard let handle else { return nil }
        let ptr = sc_framebuffer_ptr(handle)
        let w = Int(sc_framebuffer_width(handle))
        let h = Int(sc_framebuffer_height(handle))
        guard let ptr, w > 0, h > 0 else { return nil }

        let bytesPerRow = w * 4
        let data = Data(bytes: ptr, count: w * h * 4)
        guard let provider = CGDataProvider(data: data as CFData) else { return nil }
        guard let cgImage = CGImage(
            width: w,
            height: h,
            bitsPerComponent: 8,
            bitsPerPixel: 32,
            bytesPerRow: bytesPerRow,
            space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedLast.rawValue),
            provider: provider,
            decode: nil,
            shouldInterpolate: false,
            intent: .defaultIntent
        ) else { return nil }
        let size = NSSize(width: w, height: h)
        return NSImage(cgImage: cgImage, size: size)
    }

    func loadRom(at url: URL) {
        guard let handle else { return }
        let ok = url.path.withCString { sc_load_rom(handle, $0) }
        if ok != 0 {
            status = HostBridge.takeLastError() ?? "ROM load failed"
        } else {
            pushTapeLoadOptions()
            refreshStatus()
        }
    }

    func openTape(at url: URL) {
        guard let handle else { return }
        let ok = url.path.withCString { sc_open_tape(handle, $0) }
        if ok != 0 {
            status = HostBridge.takeLastError() ?? "Tape open failed"
        } else {
            refreshStatus()
            hasTape = true
            tapePlaying = false
            refreshTapeProgress()
        }
    }

    func playTape() {
        guard let handle else { return }
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
        refreshStatus()
        tapePlaying = false
    }

    func rewindTape() {
        guard let handle else { return }
        _ = sc_tape_rewind(handle)
        refreshStatus()
        tapePlaying = false
    }

    private var suppressTapeOptsPush = false
    private var suppressPausedPush = false

    func syncTapeLoadOptionsFromHost() {
        guard let handle else { return }
        var flash: Int32 = 1
        var speed: UInt32 = 1
        guard sc_tape_get_load_options(handle, &flash, &speed) == 0 else { return }
        suppressTapeOptsPush = true
        instantLoad = flash != 0
        tapeSpeed = max(1, min(speed, 64))
        suppressTapeOptsPush = false
    }

    private func pushTapeLoadOptions() {
        guard let handle, !suppressTapeOptsPush else { return }
        _ = sc_tape_set_load_options(handle, instantLoad ? 1 : 0, max(1, min(tapeSpeed, 64)))
        refreshStatus()
    }

    func reset() {
        guard let handle else { return }
        if sc_reset(handle) != 0 {
            status = HostBridge.takeLastError() ?? "Reset failed"
        } else {
            refreshStatus()
        }
    }

    func setKey(row: UInt32, bit: UInt32, pressed: Bool) {
        guard let handle else { return }
        _ = sc_set_key(handle, row, bit, pressed ? 1 : 0)
    }

    func clearKeys() {
        guard let handle else { return }
        _ = sc_clear_keys(handle)
    }

    /// Update arrow/Tab → Kempston mask from the Spectrum key view (`held` set).
    func setKeyboardJoystickMask(_ mask: UInt32) {
        keyboardJoystickMask = mask
    }

    @discardableResult
    func applyJoystickMode(_ mode: JoystickMode) -> Bool {
        guard let handle else { return false }
        let ok = sc_set_joystick_mode(handle, mode.rawValue) == 0
        if ok {
            joystickModeApplied = true
        }
        return ok
    }

    @discardableResult
    func setJoystick(mask: UInt32) -> Bool {
        guard let handle else { return false }
        return sc_set_joystick(handle, mask) == 0
    }

    @discardableResult
    func clearJoystick() -> Bool {
        guard let handle else { return false }
        keyboardJoystickMask = 0
        return sc_clear_joystick(handle) == 0
    }

    /// OR keyboard Kempston bits with GCController digital stick + A fire.
    private func pushJoystick() {
        guard let handle else { return }
        if !joystickModeApplied {
            _ = sc_set_joystick_mode(handle, JoystickMode.kempston.rawValue)
            joystickModeApplied = true
        }
        let mask = keyboardJoystickMask | Self.gamepadMask()
        _ = sc_set_joystick(handle, mask)
    }

    private func startGamepadDiscovery() {
        GCController.startWirelessControllerDiscovery(completionHandler: nil)
        connectObserver = NotificationCenter.default.addObserver(
            forName: .GCControllerDidConnect,
            object: nil,
            queue: .main
        ) { _ in }
        disconnectObserver = NotificationCenter.default.addObserver(
            forName: .GCControllerDidDisconnect,
            object: nil,
            queue: .main
        ) { _ in }
    }

    /// Bits: 0=right, 1=left, 2=down, 3=up, 4=fire (matches `sc_set_joystick`).
    private static func gamepadMask() -> UInt32 {
        var mask: UInt32 = 0
        for controller in GCController.controllers() {
            guard let pad = controller.extendedGamepad else { continue }
            let x = pad.leftThumbstick.xAxis.value
            let y = pad.leftThumbstick.yAxis.value
            if pad.dpad.right.isPressed || x >= stickThreshold { mask |= 1 << 0 }
            if pad.dpad.left.isPressed || x <= -stickThreshold { mask |= 1 << 1 }
            if pad.dpad.down.isPressed || y <= -stickThreshold { mask |= 1 << 2 }
            if pad.dpad.up.isPressed || y >= stickThreshold { mask |= 1 << 3 }
            if pad.buttonA.isPressed { mask |= 1 << 4 }
        }
        return mask
    }

    /// Default categories: bus|tape|ula|machine (bits 2|4|8|16 = 30).
    func enableDefaultTrace() {
        sc_debug_set_categories(2 | 4 | 8 | 16)
        status = "Trace enabled (default categories), events=\(sc_debug_event_count())"
    }

    func clearTrace() {
        sc_debug_clear()
        status = "Trace ring cleared"
    }

    func dumpTraceToDesktop() {
        let dir = FileManager.default.urls(for: .desktopDirectory, in: .userDomainMask).first
            ?? URL(fileURLWithPath: NSTemporaryDirectory())
        let url = dir.appendingPathComponent("spec_chum_trace.txt")
        dumpTrace(to: url)
    }

    func dumpTracePanel() {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = "spec_chum_trace.txt"
        panel.allowedContentTypes = [.plainText]
        if panel.runModal() == .OK, let url = panel.url {
            dumpTrace(to: url)
        }
    }

    private func dumpTrace(to url: URL) {
        let ok = url.path.withCString { sc_debug_dump_to_file($0) }
        if ok != 0 {
            status = HostBridge.takeLastError() ?? "Trace dump failed"
        } else {
            status = "Trace dump → \(url.path) (\(sc_debug_event_count()) events in ring)"
        }
    }

    func peek(addr: UInt32) -> UInt8? {
        guard let handle else { return nil }
        var out: UInt8 = 0
        guard sc_peek(handle, addr, &out) == 0 else { return nil }
        return out
    }

    @discardableResult
    func poke(addr: UInt32, value: UInt8) -> Bool {
        guard let handle else { return false }
        return sc_poke(handle, addr, value) == 0
    }

    func inspectJson() -> String? {
        guard let handle, let cstr = sc_inspect_json(handle) else { return nil }
        let s = String(cString: cstr)
        sc_string_free(cstr)
        return s
    }

    func regs() -> (pc: UInt16, sp: UInt16, af: UInt16, bc: UInt16, de: UInt16, hl: UInt16, ix: UInt16, iy: UInt16)? {
        guard let handle else { return nil }
        var pc: UInt16 = 0
        var sp: UInt16 = 0
        var af: UInt16 = 0
        var bc: UInt16 = 0
        var de: UInt16 = 0
        var hl: UInt16 = 0
        var ix: UInt16 = 0
        var iy: UInt16 = 0
        guard sc_regs(handle, &pc, &sp, &af, &bc, &de, &hl, &ix, &iy) == 0 else {
            return nil
        }
        return (pc, sp, af, bc, de, hl, ix, iy)
    }

    func step() {
        guard let handle else { return }
        if sc_step(handle) != 0 {
            status = HostBridge.takeLastError() ?? "Step failed"
            return
        }
        refreshInspector()
    }

    func setPaused(_ value: Bool) {
        paused = value
        status = value ? "Paused" : "Running"
        refreshInspector()
    }

    @discardableResult
    func addBreakpoint(pc: UInt32) -> Bool {
        guard let handle else { return false }
        if sc_add_breakpoint(handle, pc) != 0 {
            status = HostBridge.takeLastError() ?? "Breakpoint failed"
            return false
        }
        status = String(format: "Breakpoint at PC %04X", pc)
        return true
    }

    func addBreakpointAtPc() {
        guard let r = regs() else {
            status = HostBridge.takeLastError() ?? "No machine"
            return
        }
        _ = addBreakpoint(pc: UInt32(r.pc))
        refreshInspector()
    }

    @discardableResult
    func runUntilBreak(maxInsns: UInt32 = 1_000_000) -> Int32 {
        guard let handle else { return -1 }
        let reason = sc_run_until_break(handle, maxInsns)
        if reason < 0 {
            status = HostBridge.takeLastError() ?? "run-until-break failed"
        } else {
            status = "Break reason \(reason)"
            if reason == 1 || reason == 2 || reason == 3 || reason == 4 {
                suppressPausedPush = true
                paused = true
                suppressPausedPush = false
            }
        }
        refreshInspector()
        return reason
    }

    func dumpTraceJson() -> String? {
        guard let cstr = sc_debug_dump_json() else { return nil }
        let s = String(cString: cstr)
        sc_string_free(cstr)
        return s
    }

    func dumpTraceJsonToDesktop() {
        let dir = FileManager.default.urls(for: .desktopDirectory, in: .userDomainMask).first
            ?? URL(fileURLWithPath: NSTemporaryDirectory())
        let url = dir.appendingPathComponent("spec_chum_trace.json")
        guard let json = dumpTraceJson() else {
            status = "JSON dump failed"
            return
        }
        do {
            try json.write(to: url, atomically: true, encoding: .utf8)
            status = "Trace JSON → \(url.path) (\(sc_debug_event_count()) events in ring)"
        } catch {
            status = "Trace JSON write failed: \(error.localizedDescription)"
        }
    }

    func refreshInspector() {
        if let r = regs() {
            debugPc = r.pc
            debugSp = r.sp
            debugAf = r.af
        }
        if let json = inspectJson() {
            inspectJsonPreview = String(json.prefix(2000))
        } else {
            inspectJsonPreview = "(no machine)"
        }
    }

    func tryAutoloadRom() {
        let candidates: [String] = {
            switch model {
            case .spectrum48:
                return ["roms/spec48.rom"]
            case .spectrum128:
                return ["roms/128/spec128uk.rom"]
            case .spectrumPlus3:
                return ["roms/plus3/plus3.rom", "roms/plus2a/plus2a.rom"]
            }
        }()
        for root in romSearchRoots {
            for rel in candidates {
                let url = root.appendingPathComponent(rel)
                if FileManager.default.isReadableFile(atPath: url.path) {
                    loadRom(at: url)
                    return
                }
            }
        }
        status = "Missing ROM — run ./scripts/fetch_roms.sh"
    }

    private func refreshStatus() {
        guard let handle else { return }
        if let cstr = sc_status(handle) {
            status = String(cString: cstr)
            sc_string_free(cstr)
        }
    }

    private static func takeLastError() -> String? {
        guard let cstr = sc_last_error() else { return nil }
        let s = String(cString: cstr)
        sc_string_free(cstr)
        return s
    }

    static func defaultRomRoots() -> [URL] {
        var roots: [URL] = []
        roots.append(URL(fileURLWithPath: FileManager.default.currentDirectoryPath))
        if let env = ProcessInfo.processInfo.environment["SPEC_CHUM_ROOT"] {
            roots.append(URL(fileURLWithPath: env))
        }
        if let exe = Bundle.main.executableURL?.deletingLastPathComponent() {
            var dir = exe
            for _ in 0 ..< 8 {
                let probe = dir.appendingPathComponent("roms/spec48.rom")
                if FileManager.default.isReadableFile(atPath: probe.path) {
                    roots.append(dir)
                    break
                }
                dir = dir.deletingLastPathComponent()
            }
        }
        return roots
    }
}
