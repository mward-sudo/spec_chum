import AppKit
import Foundation
import GameController
import IOSurface
import UniformTypeIdentifiers
import CSpecChumHost

/// Temporary input-latency probe (`SPEC_CHUM_INPUT_LATENCY=1` → `/tmp/spec-input-latency.log`).
private enum InputLatencyProbe {
    static let enabled =
        ProcessInfo.processInfo.environment["SPEC_CHUM_INPUT_LATENCY"] == "1"
    private static var tKey: CFAbsoluteTime = 0
    private static var pending = false

    static func noteKey() {
        guard enabled else { return }
        tKey = CFAbsoluteTimeGetCurrent()
        pending = true
        write("key t0")
    }

    static func noteFbPublish() {
        guard enabled, pending else { return }
        write(String(format: "fb_publish +%.1fms", (CFAbsoluteTimeGetCurrent() - tKey) * 1000))
    }

    static func noteRoomPresent() {
        guard enabled, pending else { return }
        write(String(format: "room_present +%.1fms", (CFAbsoluteTimeGetCurrent() - tKey) * 1000))
        pending = false
    }

    static func noteScroll(steps: Int32) {
        guard enabled else { return }
        write("scroll steps=\(steps) t=\(CFAbsoluteTimeGetCurrent())")
    }

    private static func write(_ msg: String) {
        let line = String(format: "%.6f %@\n", CFAbsoluteTimeGetCurrent(), msg)
        fputs(line, stderr)
        let url = URL(fileURLWithPath: "/tmp/spec-input-latency.log")
        if let h = try? FileHandle(forWritingTo: url) {
            h.seekToEndOfFile()
            h.write(Data(line.utf8))
            try? h.close()
        } else {
            try? Data(line.utf8).write(to: url)
        }
    }
}

/// Thin Swift wrapper around the Spec Chum C host API.
final class HostBridge: ObservableObject {
    /// Joystick presentation — matches `sc_set_joystick_mode` (default Kempston).
    enum JoystickMode: UInt32, CaseIterable, Identifiable {
        case kempston = 0
        case sinclairLeft = 1
        case sinclairRight = 2
        case cursor = 3

        var id: UInt32 { rawValue }

        var title: String {
            switch self {
            case .kempston: "Kempston"
            case .sinclairLeft: "Sinclair left (1–5)"
            case .sinclairRight: "Sinclair right (6–0)"
            case .cursor: "Cursor"
            }
        }
    }

    /// Digital stick threshold for left thumbstick axes (~egui gilrs parity).
    private static let stickThreshold: Float = 0.5
    enum Model: UInt32, CaseIterable, Identifiable {
        case spectrum48 = 0
        case spectrum128 = 1
        case spectrumPlus3 = 2
        case spectrumPlus2A = 3
        case spectrumPlus2 = 4
        case spectrum16K = 5
        case pentagon128 = 6
        case timexTC2048 = 7
        case timexTS2068 = 8

        /// Canonical UI order (matches `machine::ALL_MODELS` / egui Machine menu).
        static let pickerOrder: [Model] = [
            .spectrum16K, .spectrum48, .spectrum128, .spectrumPlus2, .spectrumPlus2A, .spectrumPlus3, .pentagon128, .timexTC2048, .timexTS2068,
        ]

        var id: UInt32 { rawValue }

        var title: String {
            switch self {
            case .spectrum16K: "Spectrum 16K"
            case .spectrum48: "Spectrum 48K"
            case .spectrum128: "Spectrum 128K"
            case .spectrumPlus2: "Spectrum +2 (grey)"
            case .spectrumPlus3: "Spectrum +3"
            case .spectrumPlus2A: "Spectrum +2A"
            case .pentagon128: "Pentagon 128"
            case .timexTC2048: "Timex TC2048"
            case .timexTS2068: "Timex TS2068"
            }
        }

        /// Short label for window titles.
        var shortTitle: String {
            switch self {
            case .spectrum16K: "16K"
            case .spectrum48: "48K"
            case .spectrum128: "128K"
            case .spectrumPlus2: "+2"
            case .spectrumPlus3: "+3"
            case .spectrumPlus2A: "+2A"
            case .pentagon128: "Pentagon"
            case .timexTC2048: "TC2048"
            case .timexTS2068: "TS2068"
            }
        }

        /// Prefs key segment (`spectrum48`, `pentagon128`, …) — matches host_api JSON v2.
        var prefSlug: String {
            switch self {
            case .spectrum16K: "spectrum16_k"
            case .spectrum48: "spectrum48"
            case .spectrum128: "spectrum128"
            case .spectrumPlus2: "spectrum_plus2"
            case .spectrumPlus2A: "spectrum_plus2_a"
            case .spectrumPlus3: "spectrum_plus3"
            case .pentagon128: "pentagon128"
            case .timexTC2048: "timex_tc2048"
            case .timexTS2068: "timex_ts2068"
            }
        }

        /// Default ROM from fetch script present (#188).
        var romAvailable: Bool { sc_model_rom_available(rawValue) != 0 }

        /// Models whose ROM dumps are never auto-fetched (user must supply paths).
        var requiresUserProvidedRoms: Bool { sc_model_requires_user_rom(rawValue) != 0 }

        /// +3 has floppy; toolbar/File Open may include `.dsk`.
        var supportsDisk: Bool { self == .spectrumPlus3 }

        /// Beta Disk / TR-DOS on 48K-class and Sinclair 128K (not Amstrad +2/+2A/+3).
        var supportsBeta: Bool {
            self == .spectrum16K || self == .spectrum48 || self == .timexTC2048 || self == .timexTS2068 || self == .spectrum128 || self == .spectrumPlus2 || self == .pentagon128
        }

        /// Toolbar machine picker: fit the longest `title` ("Spectrum 128K") plus chevron.
        static let toolbarPickerMinWidth: CGFloat = 148
        static let toolbarPickerMaxWidth: CGFloat = 196
    }

    @Published private(set) var status: String = "Starting…"
    @Published private(set) var tapePlaying: Bool = false
    @Published private(set) var hasTape: Bool = false
    /// Last opened media filename for the window title (tape / snapshot / RZX / disk / ROM).
    @Published private(set) var mediaTitle: String?
    /// Flash-load mirror of host options. Instant turns this on ephemerally; Play forces it off.
    @Published var instantLoad: Bool = false {
        didSet { pushTapeLoadOptions() }
    }
    /// EAR speed (1x…20x): while Play is active, that many Spectrum frames per host tick.
    @Published var tapeSpeed: UInt32 = HostBridge.loadPersistedTapeSpeed() {
        didSet {
            if !suppressTapeOptsPush, tapeSpeed != 0 {
                experienceLoad = false
                instantLoad = false
            }
            pushTapeLoadOptions()
            if !suppressPrefsPersist {
                UserDefaults.standard.set(Int(tapeSpeed), forKey: Self.tapeSpeedDefaultsKey)
            }
        }
    }
    @Published var experienceLoad: Bool = HostBridge.loadPersistedExperience() {
        didSet {
            pushTapeLoadOptions()
            if !suppressPrefsPersist {
                UserDefaults.standard.set(experienceLoad, forKey: Self.experienceDefaultsKey)
            }
        }
    }
    /// Host PCM output gain 0…1 (what the user hears). Does not affect EAR / flash-load.
    @Published var outputVolume: Float = HostBridge.loadPersistedVolume() {
        didSet {
            let clamped = max(0, min(1, outputVolume))
            if clamped != outputVolume {
                outputVolume = clamped
                return
            }
            audio.volume = clamped
            UserDefaults.standard.set(Double(clamped), forKey: Self.volumeDefaultsKey)
        }
    }
    /// Host output mute (mixer gain 0). Independent of Spectrum EAR bit.
    @Published var outputMuted: Bool = HostBridge.loadPersistedMuted() {
        didSet {
            audio.muted = outputMuted
            UserDefaults.standard.set(outputMuted, forKey: Self.mutedDefaultsKey)
        }
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
    /// When true, center view is the Bevy living room (SwiftUI chrome stays). Default off — experimental.
    /// Set `SPEC_CHUM_LIVING_ROOM=1` to start in living-room mode (automation / perf capture).
    @Published var livingRoomMode: Bool = ProcessInfo.processInfo.environment["SPEC_CHUM_LIVING_ROOM"] == "1" {
        didSet {
            if livingRoomMode {
                ensureLivingRoom()
            } else {
                destroyLivingRoom(syncTeardown: false)
            }
        }
    }
    /// Weak flat-mode present view — refreshed directly from `runFrame` (no @Published churn).
    private weak var spectrumPresentView: SpectrumNSView?
    /// Bevy/room FFI handle — only touched on `livingRoomQueue` (see header thread affinity).
    private var livingRoomHandle: UnsafeMutableRawPointer?
    /// Main-thread mirror: room created and ready for enqueue.
    private var livingRoomReady = false
    /// Main-thread: create already queued (avoid sync waits / duplicate creates).
    private var livingRoomCreateInFlight = false
    /// Serial queue for all `sc_room_*` on the embed handle (Bevy must not block AppKit).
    private let livingRoomQueue = DispatchQueue(label: "dev.specchum.living-room", qos: .userInteractive)
    /// Coalesce: at most one set_fb+tick in flight on the room queue.
    private var roomTickInFlight = false
    /// When `roomTickInFlight` was set; used to recover if Bevy/GPU stalls on the room queue.
    private var roomTickStartedUptime: TimeInterval = 0
    /// Bumped on watchdog recovery / teardown so stale queue completions cannot clear a newer gate.
    private var roomTickGeneration: UInt64 = 0
    private static let roomTickStuckSeconds: TimeInterval = 3.0
    private let roomTickLock = NSLock()
    /// Coalesce stepped IOSurface rebinds on the room queue (full resize was freezing the UI).
    private var roomPresentBindInFlight = false
    /// When `roomPresentBindInFlight` was set; used to recover if the room queue stalls.
    private var roomPresentBindStartedUptime: TimeInterval = 0
    /// Bumped on watchdog recovery / teardown so stale bind completions cannot replay pending work.
    private var roomPresentBindGeneration: UInt64 = 0
    private var roomPresentBindPending: (surface: IOSurface, width: UInt32, height: UInt32)?
    /// Latest Spectrum RGBA published on main; consumed on room queue (DisplayLink).
    private let roomFbLock = NSLock()
    private var roomFbPublished: [UInt8] = []
    private var roomFbGeneration: UInt64 = 0
    /// Only touched on `livingRoomQueue`.
    private var roomFbLastUploadedGen: UInt64 = 0
    /// Strong IOSurface retain for the async bind + room texture lifetime (queue only).
    private var livingRoomBoundSurface: IOSurface?
    /// Weak present view — refresh CALayer.contents on main after each room tick (no @Published).
    private weak var livingRoomPresentView: LivingRoomNSView?
    private var scrollZoomAccum: CGFloat = 0
    /// Trackpad pixels per preset step (matches standalone Bevy `SCROLL_PIXELS_PER_STEP`).
    private static let scrollZoomStepPx: CGFloat = 64
    /// Active present size (stepped); updated on resize.
    private(set) var roomPresentWidth: UInt32 = 1920
    private(set) var roomPresentHeight: UInt32 = 1080
    /// 50 Hz host clock (owns Spectrum pacing; SwiftUI only presents).
    private var frameTimer: DispatchSourceTimer?
    @Published private(set) var debugPc: UInt16 = 0
    @Published private(set) var debugSp: UInt16 = 0
    @Published private(set) var debugAf: UInt16 = 0
    @Published private(set) var inspectJsonPreview: String = ""
    @Published var joystickMode: JoystickMode = HostBridge.loadPersistedJoystickMode() {
        didSet {
            guard oldValue != joystickMode else { return }
            _ = applyJoystickMode(joystickMode)
            if !suppressPrefsPersist {
                UserDefaults.standard.set(Int(joystickMode.rawValue), forKey: Self.joystickDefaultsKey)
            }
        }
    }
    /// When true, Spectrum / living-room pointer motion feeds the Kempston mouse (egui parity).
    @Published var kempstonMouse: Bool = HostBridge.loadPersistedKempstonMouse() {
        didSet {
            guard oldValue != kempstonMouse else { return }
            if !kempstonMouse {
                pendingMouseDx = 0
                pendingMouseDy = 0
                mouseLeft = false
                mouseRight = false
                mouseMiddle = false
                clearGuestMouseButtons()
            }
            if !suppressPrefsPersist {
                UserDefaults.standard.set(kempstonMouse, forKey: Self.kempstonMouseDefaultsKey)
            }
        }
    }
    @Published var model: Model = HostBridge.loadPersistedModel() {
        didSet {
            guard oldValue != model else { return }
            if !suppressActiveConfigClear {
                activeConfigId = nil
                persistActiveConfigId()
            }
            if !suppressPrefsPersist {
                UserDefaults.standard.set(Int(model.rawValue), forKey: Self.modelDefaultsKey)
                if activeConfigId == nil {
                    UserDefaults.standard.set(Int(model.rawValue), forKey: Self.lastBuiltinModelKey)
                }
            }
            if activeConfigId == nil {
                romSetupModel = model
            }
            guard let handle, !suppressModelPush else { return }
            _ = sc_set_model(handle, model.rawValue)
            tryAutoloadRom()
            pushTapeLoadOptions()
            refreshStatus()
            refreshRomSetupQuiet()
            maybeAutoPresentRomSetup()
        }
    }

    /// Saved user machine profiles (#187). Empty when using a built-in model only.
    @Published private(set) var customConfigs: [UserMachineConfig] = HostBridge.loadPersistedCustomConfigs()
    /// When set, the active profile is custom (not a built-in toolbar/menu pick).
    @Published var activeConfigId: String? = UserDefaults.standard.string(forKey: HostBridge.activeConfigIdKey)
    @Published var showMachineConfigEditor = false
    @Published var machineConfigEditorDraft: UserMachineConfig?
    @Published var machineConfigEditorIsNew = true

    /// ROM setup sheet (#188) — shown when built-in model ROMs are incomplete.
    @Published var showRomSetup = false
    @Published private(set) var romSetupPayload: RomSetupPayload?
    @Published private(set) var romSetupError: String?
    /// Model the ROM dialog is configuring (may differ while picking from menu).
    @Published var romSetupModel: Model = .spectrum48

    /// True when the active built-in model still needs ROM files on disk.
    var needsRomSetup: Bool {
        guard activeConfigId == nil else { return false }
        if let payload = romSetupPayload, !payload.complete {
            return true
        }
        return !model.romAvailable
    }

    /// Toolbar ROMs affordance — same gate as Machine → ROMs… (built-in only).
    var showRomsToolbarButton: Bool {
        activeConfigId == nil
    }

    /// True when a saved custom profile (not a built-in model pick) is active.
    var isCustomConfigActive: Bool {
        guard let id = activeConfigId else { return false }
        return customConfigs.contains { $0.id == id }
    }

    /// Toolbar / window label: custom profile name or built-in short title.
    var machineDisplayTitle: String {
        if let id = activeConfigId,
           let cfg = customConfigs.first(where: { $0.id == id })
        {
            return cfg.name
        }
        return model.shortTitle
    }

    /// Recent media paths (most recent first); reopen from File menu — not auto-inserted on launch.
    @Published private(set) var recentFiles: [URL] = HostBridge.loadPersistedRecentFiles()

    /// Document-style window title: media + machine (HIG).
    var windowTitle: String {
        if let mediaTitle, !mediaTitle.isEmpty {
            return "\(mediaTitle) — \(machineDisplayTitle)"
        }
        return "Spec Chum — \(machineDisplayTitle)"
    }

    private var handle: UnsafeMutableRawPointer?
    private let romSearchRoots: [URL]
    /// Wall-clock gate so SwiftUI over-scheduling cannot turbo the Spectrum.
    private var lastFrameUptime: TimeInterval = 0
    private static let framePeriod: TimeInterval = 1.0 / 50.0
    /// After a hitch, advance at most this many Spectrum frames per host tick.
    private static let maxCatchUpFrames = 2
    /// `SPEC_CHUM_ROOM_PERF=1` — stderr + footer HUD for Bevy/host hitch diagnosis.
    private let roomPerfEnabled =
        ProcessInfo.processInfo.environment["SPEC_CHUM_ROOM_PERF"].map { $0 != "0" && !$0.isEmpty }
            ?? false
    @Published private(set) var roomPerfLine: String = ""
    private var roomPerfHostFrames: UInt64 = 0
    private var roomPerfHostSumMs: Double = 0
    private var roomPerfHostMaxMs: Double = 0
    private var roomPerfRoomSumMs: Double = 0
    private var roomPerfRoomMaxMs: Double = 0
    private var roomPerfRoomTicks: UInt64 = 0
    private var roomPerfSkippedBusy: UInt64 = 0
    private var roomPerfSpectrumFrames: UInt64 = 0
    private var roomPerfHitches: UInt64 = 0
    private var roomPerfLastHudUptime: TimeInterval = 0
    private var roomPerfLastSnap: ScRoomPerfSnapshot?
    private let audio = TapeAudioPlayer()
    /// Publish progress at most ~4 Hz to avoid SwiftUI churn.
    private var progressPublishCounter: UInt32 = 0
    /// Arrows + Tab → Kempston bits (egui parity); OR’d with gamepad each frame.
    private(set) var keyboardJoystickMask: UInt32 = 0
    private var joystickModeApplied = false
    /// Accumulated host pointer delta since last `pushMouse` (egui frame clamp parity).
    private var pendingMouseDx: CGFloat = 0
    private var pendingMouseDy: CGFloat = 0
    private var mouseLeft = false
    private var mouseRight = false
    private var mouseMiddle = false
    private var connectObserver: NSObjectProtocol?
    private var disconnectObserver: NSObjectProtocol?
    private var ensureAudioObserver: NSObjectProtocol?
    /// First successful post-activation audio arm (force-rebuild once).
    private var audioOutputArmed = false
    /// Frame-scripted `LOAD ""` [CODE] (egui KeyScript parity); advanced in `runFrame`.
    private var keyScript: LoadKeyScript?
    /// After Instant Type LOAD finishes, auto-Play with flash-load still on.
    private var pendingInstantPlay = false
    /// Instant left flash-load on; clear it when the deck next stops (or Pause/Play/Rewind).
    private var instantFlashActive = false
    /// When syncing `model` from `sc_get_model` after snapshot load, skip `sc_set_model`.
    private var suppressModelPush = false
    /// Skip clearing `activeConfigId` when syncing model from a custom profile apply.
    private var suppressActiveConfigClear = false
    /// Skip UserDefaults writes while restoring published prefs in bulk.
    private var suppressPrefsPersist = false

    /// Toolbar / File label: include Disk only on +3.
    var openMediaTitle: String {
        model.supportsDisk ? "Open Tape / Disk" : "Open Tape"
    }

    /// Menu item with ellipsis.
    var openMediaMenuTitle: String { "\(openMediaTitle)…" }

    init(romSearchRoots: [URL] = HostBridge.defaultRomRoots()) {
        self.romSearchRoots = romSearchRoots
        let restoredCustom = Self.loadPersistedCustomConfigs()
        customConfigs = restoredCustom
        activeConfigId = UserDefaults.standard.string(forKey: Self.activeConfigIdKey)
        romSetupModel = model
        syncModelRomPathsToHost()
        // Restore model before create so the first machine matches last session (#186).
        handle = sc_create(model.rawValue, 1)
        if handle == nil {
            status = HostBridge.takeLastError() ?? "Failed to create host session"
            return
        }
        sc_debug_init_from_env()
        if let id = activeConfigId,
           let cfg = customConfigs.first(where: { $0.id == id }),
           applyCustomConfiguration(cfg)
        {
            // Custom profile restored.
        } else {
            activeConfigId = nil
            persistActiveConfigId()
            tryAutoloadRom()
        }
        refreshRomSetupQuiet()
        maybeAutoPresentRomSetup()
        refreshStatus()
        // Prefer persisted tape prefs over host defaults after create.
        pushTapeLoadOptions()
        _ = applyJoystickMode(joystickMode)
        startGamepadDiscovery()
        ensureAudioObserver = NotificationCenter.default.addObserver(
            forName: .specChumEnsureAudio,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            self?.ensureAudioOutput()
        }
        audio.volume = outputVolume
        audio.muted = outputMuted
        // Do not start AudioQueue here — pre-activation start can leave a zombie
        // queue that accepts enqueues but never reaches the device.
        // `applicationDidBecomeActive` → `ensureAudioOutput()` does the real start.
        if livingRoomMode {
            ensureLivingRoom()
        } else {
            scheduleLivingRoomPreload()
        }
        startFrameTimer()
        scheduleAutomationOpenTapeIfRequested()
        // If we already missed the first becomeActive (observer registered late), arm audio now.
        DispatchQueue.main.async { [weak self] in
            self?.ensureAudioOutput()
        }
    }

    /// `SPEC_CHUM_OPEN_TAPE=/path/to.tap` (+ optional `SPEC_CHUM_AUTO_PLAY_TAPE=1`) for headless audio capture.
    /// Never adjust macOS system output volume (no `osascript` `set volume` / CoreAudio device gain).
    private func scheduleAutomationOpenTapeIfRequested() {
        guard let path = ProcessInfo.processInfo.environment["SPEC_CHUM_OPEN_TAPE"], !path.isEmpty
        else { return }
        let autoPlay = ProcessInfo.processInfo.environment["SPEC_CHUM_AUTO_PLAY_TAPE"] == "1"
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.6) { [weak self] in
            guard let self else { return }
            let url = URL(fileURLWithPath: path)
            self.openTape(at: url)
            if autoPlay, self.hasTape {
                self.playTape()
                NSLog(
                    "spec-chum-audio: automation opened+played tape %@",
                    url.lastPathComponent
                )
            } else {
                NSLog("spec-chum-audio: automation opened tape %@", url.lastPathComponent)
            }
        }
    }

    private static let volumeDefaultsKey = "specChum.outputVolume"
    private static let mutedDefaultsKey = "specChum.outputMuted"
    private static let modelDefaultsKey = "specChum.model"
    private static let lastBuiltinModelKey = "specChum.lastBuiltinModel"
    private static let customConfigsKey = "specChum.customConfigs"
    static let activeConfigIdKey = "specChum.activeConfigId"
    private static let maxCustomConfigs = 32
    private static let tapeSpeedDefaultsKey = "specChum.tapeEarSpeed"
    private static let experienceDefaultsKey = "specChum.tapeExperience"
    private static let joystickDefaultsKey = "specChum.joystickMode"
    private static let kempstonMouseDefaultsKey = "specChum.kempstonMouse"
    private static let recentFilesDefaultsKey = "specChum.recentFiles"
    private static let modelRomPathsKey = "specChum.modelRomPaths"
    private static let maxRecentFiles = 12

    /// Persisted ROM paths keyed `{pref_model_slug}_{slot}` (mirrors host_api v2 JSON).
    private var modelRomPaths: [String: String] = HostBridge.loadPersistedModelRomPaths()

    private static func loadPersistedVolume() -> Float {
        let defaults = UserDefaults.standard
        if defaults.object(forKey: volumeDefaultsKey) == nil {
            return 1.0
        }
        return max(0, min(1, Float(defaults.double(forKey: volumeDefaultsKey))))
    }

    private static func loadPersistedMuted() -> Bool {
        UserDefaults.standard.bool(forKey: mutedDefaultsKey)
    }

    private static func loadPersistedModel() -> Model {
        let defaults = UserDefaults.standard
        if let id = defaults.string(forKey: activeConfigIdKey),
           let data = defaults.data(forKey: customConfigsKey),
           let configs = try? JSONDecoder().decode([UserMachineConfig].self, from: data),
           configs.contains(where: { $0.id == id })
        {
            // Model field reflects base while custom profile is active; pick last built-in fallback.
            let raw = UInt32(defaults.integer(forKey: lastBuiltinModelKey))
            if raw == 0, defaults.object(forKey: lastBuiltinModelKey) == nil {
                return .spectrum48
            }
            return Model(rawValue: raw) ?? .spectrum48
        }
        let raw = UInt32(defaults.integer(forKey: modelDefaultsKey))
        return Model(rawValue: raw) ?? .spectrum48
    }

    private static func loadPersistedCustomConfigs() -> [UserMachineConfig] {
        guard let data = UserDefaults.standard.data(forKey: customConfigsKey),
              let decoded = try? JSONDecoder().decode([UserMachineConfig].self, from: data)
        else {
            return []
        }
        return Array(decoded.prefix(maxCustomConfigs))
    }

    private func persistCustomConfigs() {
        guard let data = try? JSONEncoder().encode(customConfigs) else { return }
        UserDefaults.standard.set(data, forKey: Self.customConfigsKey)
    }

    private func persistActiveConfigId() {
        if let activeConfigId {
            UserDefaults.standard.set(activeConfigId, forKey: Self.activeConfigIdKey)
        } else {
            UserDefaults.standard.removeObject(forKey: Self.activeConfigIdKey)
        }
    }

    /// Select a built-in model (clears active custom profile).
    func selectBuiltinModel(_ pick: Model) {
        activeConfigId = nil
        persistActiveConfigId()
        romSetupModel = pick
        if model != pick {
            model = pick
        } else {
            guard let handle else { return }
            _ = sc_set_model(handle, pick.rawValue)
            tryAutoloadRom()
            pushTapeLoadOptions()
            refreshStatus()
            refreshRomSetupQuiet()
            maybeAutoPresentRomSetup()
        }
        reclaimKeyboardFocus()
    }

    /// Return keyboard focus to the Spectrum / living-room NSView after chrome interaction.
    func reclaimKeyboardFocus() {
        if livingRoomMode {
            livingRoomPresentView?.claimFocus()
        } else {
            spectrumPresentView?.claimFocus()
        }
        FocusSpectrumView.postDelayed()
    }

    /// Open ROM setup manually or after a built-in model pick when files are missing.
    func presentRomSetup(auto: Bool = false) {
        if activeConfigId == nil {
            romSetupModel = model
        }
        scheduleRomSetupSheet(auto: auto, force: true)
    }

    func refreshRomSetup() {
        romSetupError = nil
        romSetupPayload = RomSetupCodec.fetch(model: romSetupModel)
        if romSetupPayload == nil {
            romSetupError = HostBridge.takeLastError() ?? "ROM setup unavailable"
        }
    }

    /// Present the ROM sheet on the next run loop (SwiftUI may miss `true` set during init).
    private func scheduleRomSetupSheet(auto: Bool, force: Bool) {
        guard activeConfigId == nil else {
            showRomSetup = false
            return
        }
        syncModelRomPathsToHost()
        refreshRomSetup()
        let shouldShow = force || needsRomSetup
        guard shouldShow else {
            showRomSetup = false
            return
        }
        DispatchQueue.main.async { [weak self] in
            guard let self, self.activeConfigId == nil else { return }
            self.syncModelRomPathsToHost()
            self.refreshRomSetup()
            guard force || self.needsRomSetup else {
                self.showRomSetup = false
                return
            }
            self.showRomSetup = true
            if auto, let payload = self.romSetupPayload, !payload.complete {
                self.status = "ROMs required for \(payload.modelTitle)"
            }
        }
    }

    /// After a built-in model change: auto-open ROM sheet when paths are unset or files invalid.
    private func maybeAutoPresentRomSetup() {
        scheduleRomSetupSheet(auto: true, force: false)
    }

    /// Update cached payload without opening the sheet (model changes / init).
    private func refreshRomSetupQuiet() {
        guard activeConfigId == nil else {
            romSetupPayload = nil
            return
        }
        romSetupModel = model
        refreshRomSetup()
    }

    func pickRomForSlot(_ slotId: String) {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.allowedContentTypes = [
            UTType(filenameExtension: "rom") ?? .data,
            UTType(filenameExtension: "bin") ?? .data,
        ]
        panel.title = "Choose ROM file"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        installRomSlot(slotId: slotId, from: url)
    }

    func installRomSlot(slotId: String, from url: URL) {
        romSetupError = nil
        let ok = url.path.withCString { source in
            slotId.withCString { slot in
                sc_install_model_rom(romSetupModel.rawValue, slot, source)
            }
        }
        if ok != 0 {
            romSetupError = HostBridge.takeLastError() ?? "ROM install failed"
            refreshRomSetup()
            return
        }
        pullModelRomPathsFromHost()
        refreshRomSetup()
        if romSetupModel == model, romSetupPayload?.complete == true {
            finishRomSetup(loadMachine: true)
        } else {
            status = "Installed \(url.lastPathComponent) → roms/"
        }
    }

    func finishRomSetup(loadMachine: Bool) {
        if loadMachine, romSetupModel == model {
            tryAutoloadRom()
            pushTapeLoadOptions()
            refreshStatus()
        }
        showRomSetup = false
        romSetupError = nil
        reclaimKeyboardFocus()
    }

    func selectCustomConfiguration(id: String) {
        guard let cfg = customConfigs.first(where: { $0.id == id }) else { return }
        showRomSetup = false
        if applyCustomConfiguration(cfg) {
            activeConfigId = id
            persistActiveConfigId()
        }
    }

    func beginNewConfiguration() {
        let base = activeConfigId == nil ? model : (customConfigs.first { $0.id == activeConfigId }?.base.hostModel ?? model)
        var draft = UserMachineConfig.newNamed("My Spectrum", base: base)
        draft.joystickMode = PrefJoystickSlug.from(joystickMode)
        draft.kempstonMouse = kempstonMouse
        machineConfigEditorDraft = draft
        machineConfigEditorIsNew = true
        showMachineConfigEditor = true
    }

    func beginEditConfiguration(id: String) {
        guard let cfg = customConfigs.first(where: { $0.id == id }) else { return }
        machineConfigEditorDraft = cfg
        machineConfigEditorIsNew = false
        showMachineConfigEditor = true
    }

    func beginEditActiveConfiguration() {
        guard let id = activeConfigId, isCustomConfigActive else { return }
        beginEditConfiguration(id: id)
    }

    func deleteConfiguration(id: String) {
        guard customConfigs.contains(where: { $0.id == id }) else { return }
        customConfigs.removeAll { $0.id == id }
        persistCustomConfigs()
        if activeConfigId == id {
            activeConfigId = nil
            persistActiveConfigId()
            let fallback = Model(
                rawValue: UInt32(UserDefaults.standard.integer(forKey: Self.lastBuiltinModelKey))
            ) ?? .spectrum48
            selectBuiltinModel(fallback)
        }
    }

    func deleteActiveConfiguration() {
        guard let id = activeConfigId, isCustomConfigActive else { return }
        deleteConfiguration(id: id)
    }

    @discardableResult
    func saveCustomConfiguration(_ config: UserMachineConfig, isNew: Bool) -> Bool {
        if isNew, customConfigs.count >= Self.maxCustomConfigs {
            status = "Cannot save more than \(Self.maxCustomConfigs) configurations"
            return false
        }
        guard applyCustomConfiguration(config) else { return false }
        if let idx = customConfigs.firstIndex(where: { $0.id == config.id }) {
            customConfigs[idx] = config
        } else {
            customConfigs.append(config)
        }
        persistCustomConfigs()
        activeConfigId = config.id
        persistActiveConfigId()
        return true
    }

    @discardableResult
    private func applyCustomConfiguration(_ config: UserMachineConfig) -> Bool {
        guard let handle else { return false }
        guard let data = try? JSONEncoder().encode(config),
              let json = String(data: data, encoding: .utf8)
        else {
            status = "Failed to encode configuration"
            return false
        }
        let ok = json.withCString { sc_apply_user_config_json(handle, $0) }
        if ok != 0 {
            status = HostBridge.takeLastError() ?? "Configuration apply failed"
            return false
        }
        suppressActiveConfigClear = true
        suppressModelPush = true
        model = config.base.hostModel
        suppressModelPush = false
        suppressActiveConfigClear = false
        joystickMode = config.joystickMode.hostMode
        kempstonMouse = config.kempstonMouse
        pushTapeLoadOptions()
        refreshStatus()
        return true
    }

    private static func loadPersistedTapeSpeed() -> UInt32 {
        let defaults = UserDefaults.standard
        if defaults.object(forKey: tapeSpeedDefaultsKey) == nil {
            return 1
        }
        let speed = UInt32(max(1, min(defaults.integer(forKey: tapeSpeedDefaultsKey), 64)))
        let offered: Set<UInt32> = [1, 2, 5, 10, 20]
        return offered.contains(speed) ? speed : 1
    }

    private static func loadPersistedExperience() -> Bool {
        UserDefaults.standard.bool(forKey: experienceDefaultsKey)
    }

    private static func loadPersistedJoystickMode() -> JoystickMode {
        let raw = UInt32(UserDefaults.standard.integer(forKey: joystickDefaultsKey))
        return JoystickMode(rawValue: raw) ?? .kempston
    }

    private static func loadPersistedKempstonMouse() -> Bool {
        UserDefaults.standard.bool(forKey: kempstonMouseDefaultsKey)
    }

    private static func loadPersistedRecentFiles() -> [URL] {
        let paths = UserDefaults.standard.stringArray(forKey: recentFilesDefaultsKey) ?? []
        return paths.prefix(maxRecentFiles).map { URL(fileURLWithPath: $0) }
    }

    private static func loadPersistedModelRomPaths() -> [String: String] {
        guard let data = UserDefaults.standard.data(forKey: modelRomPathsKey),
              let decoded = try? JSONDecoder().decode([String: String].self, from: data)
        else {
            return [:]
        }
        return decoded.filter { !$0.value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
    }

    private func persistModelRomPaths() {
        guard let data = try? JSONEncoder().encode(modelRomPaths) else { return }
        UserDefaults.standard.set(data, forKey: Self.modelRomPathsKey)
    }

    private func syncModelRomPathsToHost() {
        guard let json = try? JSONEncoder().encode(modelRomPaths),
              let text = String(data: json, encoding: .utf8)
        else {
            return
        }
        _ = text.withCString { sc_sync_model_rom_paths_json($0) }
    }

    private func pullModelRomPathsFromHost() {
        guard let cstr = sc_model_rom_paths_json() else { return }
        defer { sc_string_free(cstr) }
        let text = String(cString: cstr)
        guard let data = text.data(using: .utf8),
              let decoded = try? JSONDecoder().decode([String: String].self, from: data)
        else {
            return
        }
        modelRomPaths = decoded
        persistModelRomPaths()
    }

    private func modelRomPath(model: Model, slot: String) -> String? {
        modelRomPaths["\(model.prefSlug)_\(slot)"]
    }

    private func persistRecentFiles() {
        let paths = recentFiles.prefix(Self.maxRecentFiles).map(\.path)
        UserDefaults.standard.set(Array(paths), forKey: Self.recentFilesDefaultsKey)
    }

    func noteRecentFile(_ url: URL) {
        var next = recentFiles.filter { $0.standardizedFileURL != url.standardizedFileURL }
        next.insert(url, at: 0)
        if next.count > Self.maxRecentFiles {
            next = Array(next.prefix(Self.maxRecentFiles))
        }
        recentFiles = next
        persistRecentFiles()
    }

    /// Reopen a recent path; missing files are dropped from the list without crashing.
    func openRecentFile(_ url: URL) {
        guard FileManager.default.isReadableFile(atPath: url.path) else {
            status = "Recent file missing: \(url.lastPathComponent)"
            recentFiles.removeAll { $0.standardizedFileURL == url.standardizedFileURL }
            persistRecentFiles()
            return
        }
        switch url.pathExtension.lowercased() {
        case "sna", "z80":
            openSnapshot(at: url)
        case "rzx":
            openRzx(at: url)
        default:
            openMedia(at: url)
        }
    }

    deinit {
        if let connectObserver {
            NotificationCenter.default.removeObserver(connectObserver)
        }
        if let disconnectObserver {
            NotificationCenter.default.removeObserver(disconnectObserver)
        }
        if let ensureAudioObserver {
            NotificationCenter.default.removeObserver(ensureAudioObserver)
        }
        GCController.stopWirelessControllerDiscovery()
        audio.stop()
        stopFrameTimer()
        destroyLivingRoom(syncTeardown: true)
        if let handle {
            sc_destroy(handle)
        }
    }

    var rawHandle: UnsafeMutableRawPointer? { handle }

    /// (Re)start AudioQueue once the app/window is active. Safe to call repeatedly.
    /// First call force-rebuilds so any pre-activation zombie queue is discarded.
    func ensureAudioOutput() {
        guard let handle else { return }
        let rate = Double(sc_audio_sample_rate(handle))
        let force = !audioOutputArmed
        if audio.ensureStarted(sampleRate: rate > 0 ? rate : 44_100, force: force) {
            audioOutputArmed = true
        }
    }

    /// Register the IOSurface present NSView (weak). Called from LivingRoomDisplayView.
    func attachLivingRoomPresentView(_ view: LivingRoomNSView) {
        livingRoomPresentView = view
    }

    /// Register the flat Spectrum NSView (weak). Called from SpectrumDisplayView.
    func attachSpectrumPresentView(_ view: SpectrumNSView) {
        spectrumPresentView = view
    }

    func skipLivingRoomIntro() {
        guard livingRoomMode, livingRoomReady else { return }
        livingRoomQueue.async { [weak self] in
            guard let self, let room = self.livingRoomHandle else { return }
            let rc = sc_room_skip_intro(room)
            // Pose applies on next DisplayLink tick — no forced Bevy frame here.
            DispatchQueue.main.async {
                if rc != 0 {
                    self.status = HostBridge.takeRoomLastError() ?? "Living room skip intro failed"
                }
            }
        }
    }

    /// Trackpad / scroll: positive `deltaY` (scroll up) zooms toward the CRT.
    /// One intentional flick → one preset step (64px threshold); Rust cooldown debounces inertia.
    func nudgeLivingRoomZoom(deltaY: CGFloat) {
        guard livingRoomMode, livingRoomReady else { return }
        let stepPx = Self.scrollZoomStepPx
        scrollZoomAccum += deltaY
        var steps: Int32 = 0
        if scrollZoomAccum >= stepPx {
            steps = -1
        } else if scrollZoomAccum <= -stepPx {
            steps = 1
        }
        guard steps != 0 else { return }
        // One step max per burst — discard remainder so leftover pixels don't re-trigger.
        scrollZoomAccum = 0
        InputLatencyProbe.noteScroll(steps: steps)
        livingRoomQueue.async { [weak self] in
            guard let self, let room = self.livingRoomHandle else { return }
            _ = sc_room_nudge_zoom(room, steps)
            // Pose eases on DisplayLink ticks — no forced sc_room_tick here.
        }
    }

    /// Bind a shared IOSurface for zero-copy present (room queue, never blocks AppKit main).
    /// Captures `surface` strongly until the bind runs; the room queue also retains it
    /// for the texture lifetime (resize must not free the surface mid-bind).
    func bindLivingRoomPresent(surface: IOSurface, width: UInt32, height: UInt32) {
        ensureLivingRoom()
        guard livingRoomReady else { return }
        let bindW = width == 0 ? roomPresentWidth : width
        let bindH = height == 0 ? roomPresentHeight : height
        if roomPresentBindInFlight,
           roomPresentBindStartedUptime > 0,
           ProcessInfo.processInfo.systemUptime - roomPresentBindStartedUptime
               >= Self.roomTickStuckSeconds
        {
            roomPresentBindGeneration &+= 1
            roomPresentBindInFlight = false
            roomPresentBindStartedUptime = 0
            roomPresentBindPending = nil
        }
        if roomPresentBindInFlight {
            roomPresentBindPending = (surface, bindW, bindH)
            return
        }
        roomPresentBindGeneration &+= 1
        let bindGen = roomPresentBindGeneration
        roomPresentBindInFlight = true
        roomPresentBindStartedUptime = ProcessInfo.processInfo.systemUptime
        enqueueLivingRoomPresentBind(
            surface: surface,
            width: bindW,
            height: bindH,
            generation: bindGen
        )
    }

    /// Must run on `livingRoomQueue` only via `enqueueLivingRoomPresentBind`.
    private func performLivingRoomPresentBind(surface: IOSurface, width: UInt32, height: UInt32) {
        guard let room = livingRoomHandle else { return }
        livingRoomBoundSurface = surface
        let ptr = Unmanaged.passUnretained(surface).toOpaque()
        var bindError: String?
        if width != roomPresentWidth || height != roomPresentHeight {
            if sc_room_resize(room, width, height) != 0 {
                bindError = HostBridge.takeRoomLastError() ?? "Living room resize failed"
            } else {
                _ = sc_room_skip_intro(room)
                roomPresentWidth = width
                roomPresentHeight = height
            }
        }
        if bindError == nil, sc_room_set_present_iosurface(room, ptr, width, height) != 0 {
            _ = sc_room_set_present_iosurface(room, nil, 0, 0)
            livingRoomBoundSurface = nil
            bindError = HostBridge.takeRoomLastError() ?? "Living room IOSurface bind failed"
        }
        if let bindError {
            DispatchQueue.main.async { [weak self] in
                self?.status = bindError
            }
        }
    }

    private func enqueueLivingRoomPresentBind(
        surface: IOSurface,
        width: UInt32,
        height: UInt32,
        generation: UInt64
    ) {
        let retained = surface
        livingRoomQueue.async { [weak self] in
            guard let self else { return }
            self.performLivingRoomPresentBind(surface: retained, width: width, height: height)
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                guard generation == self.roomPresentBindGeneration else { return }
                self.roomPresentBindInFlight = false
                self.roomPresentBindStartedUptime = 0
                guard let pending = self.roomPresentBindPending else { return }
                self.roomPresentBindPending = nil
                self.roomPresentBindGeneration &+= 1
                let nextGen = self.roomPresentBindGeneration
                self.roomPresentBindInFlight = true
                self.roomPresentBindStartedUptime = ProcessInfo.processInfo.systemUptime
                self.enqueueLivingRoomPresentBind(
                    surface: pending.surface,
                    width: pending.width,
                    height: pending.height,
                    generation: nextGen
                )
            }
        }
    }

    func clearLivingRoomPresent() {
        livingRoomQueue.async { [weak self] in
            guard let self else { return }
            self.livingRoomBoundSurface = nil
            guard let room = self.livingRoomHandle else { return }
            _ = sc_room_set_present_iosurface(room, nil, 0, 0)
        }
    }

    /// Non-blocking: create Bevy on `livingRoomQueue` and mark ready on main.
    private func ensureLivingRoom() {
        if livingRoomReady || livingRoomCreateInFlight { return }
        livingRoomCreateInFlight = true
        livingRoomQueue.async { [weak self] in
            guard let self else { return }
            let createError = self.createLivingRoomOnQueue(warmupTicks: 0)
            let ok = self.livingRoomHandle != nil
            DispatchQueue.main.async {
                self.livingRoomCreateInFlight = false
                guard ok else {
                    self.status = createError
                        ?? "Living room renderer failed — run cargo build -p living_room --release --no-default-features"
                    if self.livingRoomMode {
                        self.livingRoomMode = false
                    }
                    self.livingRoomReady = false
                    return
                }
                self.livingRoomReady = true
                self.roomTickInFlight = false
                self.roomTickStartedUptime = 0
                self.roomTickGeneration &+= 1
                self.scrollZoomAccum = 0
                self.roomFbLock.lock()
                self.roomFbPublished = []
                self.roomFbGeneration = 0
                self.roomFbLock.unlock()
                if let createError {
                    self.status = createError
                }
                // First bind may have been skipped while create was in flight.
                self.livingRoomPresentView?.syncPresentTargetIfNeeded()
            }
        }
    }

    /// Warm Bevy + GPU pipelines in the background while flat mode runs (issue #146).
    private func scheduleLivingRoomPreload() {
        if livingRoomReady || livingRoomCreateInFlight { return }
        livingRoomCreateInFlight = true
        livingRoomQueue.async { [weak self] in
            guard let self else { return }
            let createError = self.createLivingRoomOnQueue(warmupTicks: 4)
            let ok = self.livingRoomHandle != nil
            DispatchQueue.main.async {
                self.livingRoomCreateInFlight = false
                guard ok else { return }
                self.livingRoomReady = true
                self.roomTickInFlight = false
                self.roomTickStartedUptime = 0
                self.roomTickGeneration &+= 1
                self.scrollZoomAccum = 0
                if let createError, self.livingRoomMode {
                    self.status = createError
                    self.livingRoomMode = false
                    self.livingRoomReady = false
                } else if self.livingRoomMode {
                    self.livingRoomPresentView?.syncPresentTargetIfNeeded()
                }
            }
        }
    }

    /// Must run on `livingRoomQueue`. Creates handle, skips intro, optional warmup ticks.
    private func createLivingRoomOnQueue(warmupTicks: Int) -> String? {
        guard livingRoomHandle == nil else { return nil }
        livingRoomHandle = sc_room_create(roomPresentWidth, roomPresentHeight)
        guard livingRoomHandle != nil else {
            return HostBridge.takeRoomLastError()
                ?? "Living room renderer failed — run cargo build -p living_room --release --no-default-features"
        }
        if sc_room_skip_intro(livingRoomHandle) != 0 {
            return HostBridge.takeRoomLastError() ?? "Living room skip intro failed"
        }
        roomFbLastUploadedGen = 0
        if warmupTicks > 0, let room = livingRoomHandle {
            for _ in 0..<warmupTicks {
                sc_room_set_frame_delta_seconds(room, 1.0 / 60.0)
                _ = sc_room_tick(room)
            }
        }
        return nil
    }

    /// Tear down the room. `syncTeardown` is only for `deinit` (handle must not outlive HostBridge).
    private func destroyLivingRoom(syncTeardown: Bool) {
        livingRoomReady = false
        livingRoomCreateInFlight = false
        roomTickInFlight = false
        roomTickStartedUptime = 0
        roomTickGeneration &+= 1
        scrollZoomAccum = 0
        roomPresentBindInFlight = false
        roomPresentBindStartedUptime = 0
        roomPresentBindGeneration &+= 1
        roomPresentBindPending = nil
        livingRoomPresentView = nil
        roomFbLock.lock()
        roomFbPublished = []
        roomFbGeneration = 0
        roomFbLock.unlock()
        roomPerfLastSnap = nil
        let teardown = { [weak self] in
            guard let self else { return }
            self.roomFbLastUploadedGen = 0
            self.livingRoomBoundSurface = nil
            if let livingRoomHandle = self.livingRoomHandle {
                _ = sc_room_set_present_iosurface(livingRoomHandle, nil, 0, 0)
                sc_room_destroy(livingRoomHandle)
            }
            self.livingRoomHandle = nil
        }
        if syncTeardown {
            livingRoomQueue.sync(execute: teardown)
        } else {
            livingRoomQueue.async(execute: teardown)
        }
        roomPresentWidth = 1920
        roomPresentHeight = 1080
    }

    /// Main: copy latest Spectrum RGBA into the publish slot (DisplayLink consumes).
    private func publishLivingRoomFramebuffer() {
        guard livingRoomMode, livingRoomReady, let handle else { return }
        guard let fb = sc_framebuffer_ptr(handle) else { return }
        let w = sc_framebuffer_width(handle)
        let h = sc_framebuffer_height(handle)
        guard w > 0, h > 0 else { return }
        let byteLen = Int(w * h * 4)
        roomFbLock.lock()
        if roomFbPublished.count != byteLen {
            roomFbPublished = [UInt8](repeating: 0, count: byteLen)
        }
        roomFbPublished.withUnsafeMutableBytes { dst in
            guard let base = dst.baseAddress else { return }
            base.copyMemory(from: fb, byteCount: byteLen)
        }
        roomFbGeneration &+= 1
        roomFbLock.unlock()
        InputLatencyProbe.noteFbPublish()
        if roomPerfEnabled {
            roomPerfSpectrumFrames &+= 1
        }
    }

    /// After keyboard matrix sync: run one Spectrum frame immediately (don't wait for 50 Hz timer).
    func flushInputFrame() {
        InputLatencyProbe.noteKey()
        _ = runFrame()
        roomTickLock.lock()
        let busy = roomTickInFlight
        roomTickLock.unlock()
        guard livingRoomMode, livingRoomReady, !busy else { return }
        onLivingRoomDisplayTick(deltaSeconds: 1.0 / 60.0)
    }

    /// DisplayLink → room queue: sample latest FB, tick Bevy at refresh rate (coalesce if busy).
    func onLivingRoomDisplayTick(deltaSeconds: CFTimeInterval) {
        guard livingRoomMode, livingRoomReady else { return }
        recoverStuckRoomTickIfNeeded()
        roomTickLock.lock()
        if roomTickInFlight {
            roomTickLock.unlock()
            roomPerfSkippedBusy &+= 1
            return
        }
        roomTickGeneration &+= 1
        let tickGen = roomTickGeneration
        roomTickInFlight = true
        roomTickStartedUptime = ProcessInfo.processInfo.systemUptime
        roomTickLock.unlock()
        if let window = livingRoomPresentView?.window ?? NSApp.keyWindow ?? NSApp.mainWindow,
           !window.occlusionState.contains(.visible)
        {
            finishRoomTick(generation: tickGen)
            return
        }
        let dt = Float(max(deltaSeconds, 1.0 / 240.0))
        livingRoomQueue.async { [weak self] in
            defer { self?.finishRoomTick(generation: tickGen) }
            guard let self,
                  self.shouldRunRoomTick(generation: tickGen),
                  let room = self.livingRoomHandle
            else { return }
            sc_room_perf_set_thread_hint(2)
            sc_room_set_frame_delta_seconds(room, dt)

            self.roomFbLock.lock()
            let gen = self.roomFbGeneration
            let shouldUpload = gen != self.roomFbLastUploadedGen && !self.roomFbPublished.isEmpty
            let setRc: Int32 = if shouldUpload {
                self.roomFbPublished.withUnsafeBufferPointer { buf -> Int32 in
                    guard let base = buf.baseAddress else { return -1 }
                    return sc_room_set_framebuffer(room, base, UInt32(buf.count))
                }
            } else {
                0
            }
            if setRc == 0, shouldUpload {
                self.roomFbLastUploadedGen = gen
            }
            self.roomFbLock.unlock()

            let t0 = ProcessInfo.processInfo.systemUptime
            _ = sc_room_tick(room)
            let roomMs = (ProcessInfo.processInfo.systemUptime - t0) * 1000.0
            var snap = ScRoomPerfSnapshot()
            let gotSnap = self.roomPerfEnabled && sc_room_perf_snapshot(room, &snap) == 0
            DispatchQueue.main.async {
                if self.roomPerfEnabled {
                    self.roomPerfRoomTicks &+= 1
                    self.roomPerfRoomSumMs += roomMs
                    self.roomPerfRoomMaxMs = max(self.roomPerfRoomMaxMs, roomMs)
                    if gotSnap { self.roomPerfLastSnap = snap }
                }
                self.livingRoomPresentView?.refreshLayerContents()
                InputLatencyProbe.noteRoomPresent()
            }
        }
    }

    /// True when this tick is still the active coalesced operation (not superseded by recovery).
    private func shouldRunRoomTick(generation: UInt64) -> Bool {
        roomTickLock.lock()
        defer { roomTickLock.unlock() }
        return roomTickInFlight && generation == roomTickGeneration
    }

    /// Clear coalesce flag after a room tick finishes or is abandoned.
    private func finishRoomTick(generation: UInt64) {
        roomTickLock.lock()
        defer { roomTickLock.unlock() }
        guard generation == roomTickGeneration else { return }
        roomTickInFlight = false
        roomTickStartedUptime = 0
    }

    /// If Bevy/GPU blocked the room queue too long, drop the coalesce gate so DisplayLink can resume.
    private func recoverStuckRoomTickIfNeeded() {
        roomTickLock.lock()
        let stuck = roomTickInFlight
        let started = roomTickStartedUptime
        roomTickLock.unlock()
        guard stuck, started > 0 else { return }
        let elapsed = ProcessInfo.processInfo.systemUptime - started
        guard elapsed >= Self.roomTickStuckSeconds else { return }
        if roomPerfEnabled {
            NSLog("roomperf: room tick in-flight %.1fs — forcing coalesce reset", elapsed)
        }
        roomTickLock.lock()
        roomTickGeneration &+= 1
        roomTickInFlight = false
        roomTickStartedUptime = 0
        roomTickLock.unlock()
    }

    private func noteHostFrame(elapsedMs: Double) {
        guard roomPerfEnabled else { return }
        roomPerfHostFrames &+= 1
        roomPerfHostSumMs += elapsedMs
        roomPerfHostMaxMs = max(roomPerfHostMaxMs, elapsedMs)
        if elapsedMs >= 25.0 {
            roomPerfHitches &+= 1
        }
        let now = ProcessInfo.processInfo.systemUptime
        if roomPerfLastHudUptime == 0 || now - roomPerfLastHudUptime >= 1.0 {
            roomPerfLastHudUptime = now
            publishRoomPerfHud()
        }
    }

    private func publishRoomPerfHud() {
        guard roomPerfEnabled else { return }
        let hostAvg = roomPerfHostFrames > 0
            ? roomPerfHostSumMs / Double(roomPerfHostFrames) : 0
        let roomAvg = roomPerfRoomTicks > 0
            ? roomPerfRoomSumMs / Double(roomPerfRoomTicks) : 0
        // Counts over the last ~1s window ≈ Hz.
        let roomHz = Double(roomPerfRoomTicks)
        let specHz = Double(roomPerfSpectrumFrames)
        var rustBit = ""
        if let snap = roomPerfLastSnap {
            let thr: String = {
                switch snap.thread_hint {
                case 1: return "main"
                case 2: return "roomQ"
                default: return "?"
                }
            }()
            rustBit = String(
                format: " bevy last=%.1f avg=%.1f max=%.1fms \(thr) %ux%u z%u present=%u",
                Double(snap.last_tick_us) / 1000.0,
                Double(snap.avg_window_us) / 1000.0,
                Double(snap.max_window_us) / 1000.0,
                snap.width,
                snap.height,
                snap.zoom_preset,
                UInt(snap.has_present)
            )
        }
        let line = String(
            format:
                "roomperf host avg=%.1f max=%.1fms hitches=%llu | roomHz=%.0f specHz=%.0f wall avg=%.1f max=%.1fms skip=%llu linear %ux%u%@",
            hostAvg,
            roomPerfHostMaxMs,
            roomPerfHitches,
            roomHz,
            specHz,
            roomAvg,
            roomPerfRoomMaxMs,
            roomPerfSkippedBusy,
            roomPresentWidth,
            roomPresentHeight,
            rustBit
        )
        roomPerfLine = line
        NSLog("%@", line)
        roomPerfHostFrames = 0
        roomPerfHostSumMs = 0
        roomPerfHostMaxMs = 0
        roomPerfRoomSumMs = 0
        roomPerfRoomMaxMs = 0
        roomPerfRoomTicks = 0
        roomPerfSkippedBusy = 0
        roomPerfSpectrumFrames = 0
        roomPerfHitches = 0
    }

    private func startFrameTimer() {
        stopFrameTimer()
        let timer = DispatchSource.makeTimerSource(queue: .main)
        timer.schedule(
            deadline: .now(),
            repeating: Self.framePeriod,
            leeway: .milliseconds(4)
        )
        timer.setEventHandler { [weak self] in
            self?.runFrame()
        }
        timer.resume()
        frameTimer = timer
    }

    private func stopFrameTimer() {
        frameTimer?.cancel()
        frameTimer = nil
    }

    /// Run Spectrum frame(s) capped to ~50 Hz wall clock (egui throttle parity).
    /// Returns whether at least one `sc_run_frame` ran.
    @discardableResult
    func runFrame() -> Bool {
        guard let handle else { return false }
        let hostT0 = ProcessInfo.processInfo.systemUptime
        defer {
            if roomPerfEnabled {
                noteHostFrame(elapsedMs: (ProcessInfo.processInfo.systemUptime - hostT0) * 1000.0)
            }
        }
        let now = ProcessInfo.processInfo.systemUptime
        if lastFrameUptime == 0 {
            lastFrameUptime = now
            pushJoystick()
            pushMouse()
            tickKeyScript()
            sc_run_frame(handle)
            syncTapePublished()
            enqueueAudio()
            publishLivingRoomFramebuffer()
            // Flat spectrum presents via direct NSView refresh; living room uses DisplayLink + IOSurface.
            if !livingRoomMode {
                spectrumPresentView?.presentFrame()
            }
            return true
        }

        var ran = 0
        while now - lastFrameUptime >= Self.framePeriod, ran < Self.maxCatchUpFrames {
            pushJoystick()
            pushMouse()
            tickKeyScript()
            sc_run_frame(handle)
            // Enqueue each frame — sc_run_frame replaces PCM; skipping mid catch-up
            // underruns AudioQueue (especially under living-room hitching).
            enqueueAudio()
            lastFrameUptime += Self.framePeriod
            ran += 1
        }
        // If we stalled longer than the catch-up window, resync to wall clock.
        if now - lastFrameUptime > Self.framePeriod * Double(Self.maxCatchUpFrames) {
            lastFrameUptime = now
        }
        if ran > 0 {
            syncTapePublished()
            publishLivingRoomFramebuffer()
            if !livingRoomMode {
                spectrumPresentView?.presentFrame()
            }
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

    private func loadRom(at url: URL) {
        guard let handle else { return }
        let ok = url.path.withCString { sc_load_rom(handle, $0) }
        if ok != 0 {
            status = HostBridge.takeLastError() ?? "ROM load failed"
        } else {
            mediaTitle = url.lastPathComponent
            // Custom ROM into a running machine is not a hot-swap — reset like real hardware.
            if sc_reset(handle) != 0 {
                status = HostBridge.takeLastError() ?? "ROM loaded but reset failed"
            }
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

    func openSnapshot(at url: URL) {
        guard let handle else { return }
        let ok = url.path.withCString { sc_load_snapshot(handle, $0) }
        if ok != 0 {
            status = HostBridge.takeLastError() ?? "Snapshot load failed"
        } else {
            mediaTitle = url.lastPathComponent
            syncModelFromHost()
            pushTapeLoadOptions()
            refreshStatus()
            noteRecentFile(url)
        }
    }

    /// Read host model after snapshot load without calling `sc_set_model` again.
    private func syncModelFromHost() {
        guard let handle else { return }
        let raw = sc_get_model(handle)
        guard raw != UInt32.max, let m = Model(rawValue: raw), m != model else { return }
        suppressModelPush = true
        model = m
        suppressModelPush = false
    }

    func openRzx(at url: URL) {
        guard let handle else { return }
        let ok = url.path.withCString { sc_load_rzx(handle, $0) }
        if ok != 0 {
            status = HostBridge.takeLastError() ?? "RZX load failed"
        } else {
            mediaTitle = url.lastPathComponent
            refreshStatus()
            noteRecentFile(url)
        }
    }

    func openDsk(at url: URL) {
        guard let handle else { return }
        let ok = url.path.withCString { sc_load_dsk(handle, $0) }
        if ok != 0 {
            status = HostBridge.takeLastError() ?? "DSK load failed"
        } else {
            mediaTitle = url.lastPathComponent
            // Prefer a clear +3DOS hint over the raw host status string.
            status = "DSK inserted — use +3 Loader / +3DOS"
            noteRecentFile(url)
        }
    }

    func openTrd(at url: URL) {
        guard let handle else { return }
        let ok = url.path.withCString { sc_load_trd(handle, $0) }
        if ok != 0 {
            status = HostBridge.takeLastError() ?? "TRD load failed"
        } else {
            mediaTitle = url.lastPathComponent
            status = "TRD inserted — attach TR-DOS ROM, then RANDOMIZE USR 15616"
        }
    }

    func loadTrdosRom(at url: URL) {
        guard let handle else { return }
        let ok = url.path.withCString { sc_load_trdos_rom(handle, $0) }
        if ok != 0 {
            status = HostBridge.takeLastError() ?? "TR-DOS ROM load failed"
        } else {
            refreshStatus()
        }
    }

    /// Queue egui-parity `LOAD ""` [CODE] via `sc_set_key`.
    /// 128K/+3: menu → 48 BASIC (+3 disk Loader — do not Enter alone).
    /// +2A: menu Loader is tape — Enter alone for PROGRAM.
    func typeLoadQuotes(withCode: Bool = false) {
        beginTypeLoadQuotes(withCode: withCode, pendingPlay: false)
    }

    /// Instant: always prompt for an image, then flash-load + Type LOAD "" + Play.
    /// Never silently reuses the currently inserted tape. Play alone stays EAR-only.
    func instantLoadTape() {
        presentInstantMediaPanel()
    }

    /// Tape-only filters; Instant is flash Type LOAD. Use Open Tape / Disk for `.dsk`.
    private func presentInstantMediaPanel() {
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
    private func beginInstantLoadAfterInsert() {
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

    private func beginTypeLoadQuotes(withCode: Bool, pendingPlay: Bool) {
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

    /// NSOpenPanel for tape (and `.dsk` on +3). Snapshots/RZX stay separate File items.
    func presentOpenMediaPanel() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        var types: [UTType] = [
            UTType(filenameExtension: "tap") ?? .data,
            UTType(filenameExtension: "tzx") ?? .data,
        ]
        if model.supportsDisk {
            types.append(UTType(filenameExtension: "dsk") ?? .data)
            panel.title = "Open TAP / TZX / DSK"
        } else {
            panel.title = "Open TAP / TZX"
        }
        panel.allowedContentTypes = types
        if panel.runModal() == .OK, let url = panel.url {
            openMedia(at: url)
        }
    }

    /// Route by extension: `.dsk` → disk (+3), else tape.
    func openMedia(at url: URL) {
        if url.pathExtension.lowercased() == "dsk" {
            openDsk(at: url)
        } else {
            openTape(at: url)
        }
    }

    /// Play always forces flash-load off (EAR path at the selected speed).
    func playTape() {
        instantFlashActive = false
        setFlashLoad(false)
        playTapeKeepingFlash()
        if tapePlaying {
            status = "Tape playing (EAR)"
        }
    }

    /// Start the deck without clearing flash-load (Instant only).
    private func playTapeKeepingFlash() {
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

    private var suppressTapeOptsPush = false
    private var suppressPausedPush = false

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

    private func setFlashLoad(_ on: Bool) {
        if on {
            experienceLoad = false
        }
        guard instantLoad != on else {
            pushTapeLoadOptions()
            return
        }
        instantLoad = on
    }

    private func pushTapeLoadOptions() {
        guard let handle, !suppressTapeOptsPush else { return }
        _ = sc_tape_set_load_options_ex(
            handle,
            instantLoad ? 1 : 0,
            max(1, min(tapeSpeed, 64)),
            experienceLoad ? 1 : 0
        )
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

    /// Batch matrix update for one key edge (single recompose path in Rust via setKey loop).
    func syncKeyboardMatrix(
        modifiers: [(UInt32, UInt32)],
        held: [(UInt32, UInt32)],
        joystickMask: UInt32
    ) {
        clearKeys()
        for (row, bit) in modifiers {
            setKey(row: row, bit: bit, pressed: true)
        }
        for (row, bit) in held {
            setKey(row: row, bit: bit, pressed: true)
        }
        setKeyboardJoystickMask(joystickMask)
        flushInputFrame()
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
            if joystickMode != mode {
                joystickMode = mode
            }
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

    /// NSEvent deltas: positive `deltaY` is up; Kempston/egui use positive dy = down.
    func noteMouseDelta(deltaX: CGFloat, deltaY: CGFloat) {
        guard kempstonMouse else { return }
        pendingMouseDx += deltaX
        pendingMouseDy -= deltaY
    }

    /// `buttonNumber`: 0=left, 1=right, 2=middle (AppKit).
    func noteMouseButton(buttonNumber: Int, pressed: Bool) {
        guard kempstonMouse else { return }
        switch buttonNumber {
        case 0: mouseLeft = pressed
        case 1: mouseRight = pressed
        case 2: mouseMiddle = pressed
        default: break
        }
    }

    func clearMouseButtons() {
        pendingMouseDx = 0
        pendingMouseDy = 0
        mouseLeft = false
        mouseRight = false
        mouseMiddle = false
        clearGuestMouseButtons()
    }

    private func clearGuestMouseButtons() {
        guard let handle else { return }
        _ = sc_set_mouse_buttons(handle, 0, 0, 0)
    }

    /// Clamp accumulated motion to i8 and push buttons (egui per-frame parity).
    private func pushMouse() {
        guard kempstonMouse, let handle else { return }
        let dx = Int32(max(CGFloat(Int8.min), min(CGFloat(Int8.max), pendingMouseDx.rounded())))
        let dy = Int32(max(CGFloat(Int8.min), min(CGFloat(Int8.max), pendingMouseDy.rounded())))
        pendingMouseDx -= CGFloat(dx)
        pendingMouseDy -= CGFloat(dy)
        if dx != 0 || dy != 0 {
            _ = sc_set_mouse_delta(handle, Int32(dx), Int32(dy))
        }
        _ = sc_set_mouse_buttons(
            handle,
            mouseLeft ? 1 : 0,
            mouseRight ? 1 : 0,
            mouseMiddle ? 1 : 0
        )
    }

    func attachMultiface(at url: URL) {
        guard let handle else { return }
        let ok = url.path.withCString { sc_attach_multiface(handle, $0) }
        if ok != 0 {
            status = HostBridge.takeLastError() ?? "Multiface attach failed"
        } else {
            refreshStatus()
        }
    }

    func multifaceNmi() {
        guard let handle else { return }
        if sc_multiface_nmi(handle) != 0 {
            status = HostBridge.takeLastError() ?? "Multiface NMI failed"
        } else {
            refreshStatus()
        }
    }

    func attachInterface1() {
        guard let handle else { return }
        if sc_attach_interface1(handle) != 0 {
            status = HostBridge.takeLastError() ?? "Interface 1 attach failed"
        } else {
            refreshStatus()
        }
    }

    func loadInterface1Rom(at url: URL) {
        guard let handle else { return }
        let ok = url.path.withCString { sc_load_interface1_rom(handle, $0) }
        if ok != 0 {
            status = HostBridge.takeLastError() ?? "IF1 ROM load failed"
        } else {
            refreshStatus()
        }
    }

    func insertMdr(at url: URL) {
        guard let handle else { return }
        let ok = url.path.withCString { sc_insert_mdr(handle, $0) }
        if ok != 0 {
            status = HostBridge.takeLastError() ?? "MDR insert failed"
        } else {
            refreshStatus()
        }
    }

    func attachDivmmc() {
        guard let handle else { return }
        if sc_attach_divmmc(handle) != 0 {
            status = HostBridge.takeLastError() ?? "DivMMC attach failed"
        } else {
            refreshStatus()
        }
    }

    func loadDivmmcSd(at url: URL) {
        guard let handle else { return }
        let ok = url.path.withCString { sc_load_divmmc_sd(handle, $0) }
        if ok != 0 {
            status = HostBridge.takeLastError() ?? "DivMMC SD load failed"
        } else {
            refreshStatus()
        }
    }

    func loadDivmmcEeprom(at url: URL) {
        guard let handle else { return }
        let ok = url.path.withCString { sc_load_divmmc_eeprom(handle, $0) }
        if ok != 0 {
            status = HostBridge.takeLastError() ?? "DivMMC EEPROM load failed"
        } else {
            refreshStatus()
        }
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
        syncModelRomPathsToHost()
        if let main = modelRomPath(model: model, slot: "main"),
           FileManager.default.isReadableFile(atPath: main)
        {
            loadRom(at: URL(fileURLWithPath: main))
            return
        }
        let candidates: [String] = {
            switch model {
            case .spectrum16K, .spectrum48:
                return ["roms/spec48.rom"]
            case .spectrum128:
                return ["roms/128/spec128uk.rom"]
            case .spectrumPlus2:
                return ["roms/plus2/plus2uk.rom"]
            case .spectrumPlus3:
                return ["roms/plus3/plus3.rom"]
            case .spectrumPlus2A:
                return ["roms/plus2a/plus2a.rom", "roms/plus3/plus3.rom"]
            case .pentagon128:
                return ["roms/pentagon/pentagon.rom", "roms/pentagon/128p.rom"]
            case .timexTC2048:
                return ["roms/timex/tc2048.rom"]
            case .timexTS2068:
                return ["roms/timex/tc2068-0.rom"]
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

    /// Apply one frame of the Type LOAD script (clear + chord), egui `tick_key_script` parity.
    private func tickKeyScript() {
        guard let handle, var script = keyScript else { return }
        if script.stepIndex >= script.steps.count {
            keyScript = nil
            clearKeys()
            finishInstantPlayIfPending()
            return
        }
        if script.framesLeft == 0 {
            script.framesLeft = max(1, script.steps[script.stepIndex].frames)
        }
        let chord = script.steps[script.stepIndex].keys
        clearKeys()
        for (row, bit) in chord {
            _ = sc_set_key(handle, row, bit, 1)
        }
        script.framesLeft -= 1
        if script.framesLeft == 0 {
            script.stepIndex += 1
        }
        if script.stepIndex >= script.steps.count {
            keyScript = nil
            finishInstantPlayIfPending()
        } else {
            keyScript = script
        }
    }

    private func finishInstantPlayIfPending() {
        guard pendingInstantPlay else { return }
        pendingInstantPlay = false
        playTapeKeepingFlash()
        status = "Instant: flash-loading after LOAD \"\""
    }

    private static func takeLastError() -> String? {
        guard let cstr = sc_last_error() else { return nil }
        let s = String(cString: cstr)
        sc_string_free(cstr)
        return s
    }

    private static func takeRoomLastError() -> String? {
        guard let cstr = sc_room_last_error() else { return nil }
        let s = String(cString: cstr)
        sc_room_string_free(cstr)
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

/// Keyword-mode `LOAD ""` [CODE] — mirrors egui `KeyScript` / ROM debounce.
private struct LoadKeyScript {
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
