import AppKit
import Foundation
import GameController
import IOSurface
import UniformTypeIdentifiers
import CSpecChumHost

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
    static let stickThreshold: Float = 0.5
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

        /// Timex dock `.dck` cartridges (TS2068 / TC2068 horizontal MMU).
        var supportsTimexDock: Bool { self == .timexTS2068 }

        /// Toolbar machine picker: fit the longest `title` ("Spectrum 128K") plus chevron.
        static let toolbarPickerMinWidth: CGFloat = 148
        static let toolbarPickerMaxWidth: CGFloat = 196
    }

    @Published var status: String = "Starting…"
    @Published var tapePlaying: Bool = false
    @Published var hasTape: Bool = false
    /// Last opened media filename for the window title (tape / snapshot / RZX / disk / ROM).
    @Published var mediaTitle: String?
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
    @Published var tapeFraction: Double?
    @Published var tapeBlockLabel: String = ""
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
    weak var spectrumPresentView: SpectrumNSView?
    /// Bevy/room FFI handle — only touched on `livingRoomQueue` (see header thread affinity).
    var livingRoomHandle: UnsafeMutableRawPointer?
    /// Main-thread mirror: room created and ready for enqueue.
    var livingRoomReady = false
    /// Main-thread: create already queued (avoid sync waits / duplicate creates).
    var livingRoomCreateInFlight = false
    /// Serial queue for all `sc_room_*` on the embed handle (Bevy must not block AppKit).
    let livingRoomQueue = DispatchQueue(label: "dev.specchum.living-room", qos: .userInteractive)
    /// Coalesce: at most one set_fb+tick in flight on the room queue.
    var roomTickInFlight = false
    /// When `roomTickInFlight` was set; used to recover if Bevy/GPU stalls on the room queue.
    var roomTickStartedUptime: TimeInterval = 0
    /// Bumped on watchdog recovery / teardown so stale queue completions cannot clear a newer gate.
    var roomTickGeneration: UInt64 = 0
    static let roomTickStuckSeconds: TimeInterval = 3.0
    let roomTickLock = NSLock()
    /// Coalesce stepped IOSurface rebinds on the room queue (full resize was freezing the UI).
    var roomPresentBindInFlight = false
    /// When `roomPresentBindInFlight` was set; used to recover if the room queue stalls.
    var roomPresentBindStartedUptime: TimeInterval = 0
    /// Bumped on watchdog recovery / teardown so stale bind completions cannot replay pending work.
    var roomPresentBindGeneration: UInt64 = 0
    var roomPresentBindPending: (surface: IOSurface, width: UInt32, height: UInt32)?
    /// Latest Spectrum RGBA published on main; consumed on room queue (DisplayLink).
    let roomFbLock = NSLock()
    var roomFbPublished: [UInt8] = []
    var roomFbGeneration: UInt64 = 0
    /// Only touched on `livingRoomQueue`.
    var roomFbLastUploadedGen: UInt64 = 0
    /// Strong IOSurface retain for the async bind + room texture lifetime (queue only).
    var livingRoomBoundSurface: IOSurface?
    /// Weak present view — refresh CALayer.contents on main after each room tick (no @Published).
    weak var livingRoomPresentView: LivingRoomNSView?
    var scrollZoomAccum: CGFloat = 0
    /// Trackpad pixels per preset step (matches standalone Bevy `SCROLL_PIXELS_PER_STEP`).
    static let scrollZoomStepPx: CGFloat = 64
    /// Active present size (stepped); updated on resize.
    var roomPresentWidth: UInt32 = 1920
    var roomPresentHeight: UInt32 = 1080
    /// 50 Hz host clock (owns Spectrum pacing; SwiftUI only presents).
    var frameTimer: DispatchSourceTimer?
    @Published var debugPc: UInt16 = 0
    @Published var debugSp: UInt16 = 0
    @Published var debugAf: UInt16 = 0
    @Published var inspectJsonPreview: String = ""
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
    @Published var customConfigs: [UserMachineConfig] = HostBridge.loadPersistedCustomConfigs()
    /// When set, the active profile is custom (not a built-in toolbar/menu pick).
    @Published var activeConfigId: String? = UserDefaults.standard.string(forKey: HostBridge.activeConfigIdKey)
    @Published var showMachineConfigEditor = false
    @Published var machineConfigEditorDraft: UserMachineConfig?
    @Published var machineConfigEditorIsNew = true

    /// ROM setup sheet (#188) — shown when built-in model ROMs are incomplete.
    @Published var showRomSetup = false
    @Published var romSetupPayload: RomSetupPayload?
    @Published var romSetupError: String?
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
    @Published var recentFiles: [URL] = HostBridge.loadPersistedRecentFiles()

    /// Document-style window title: media + machine (HIG).
    var windowTitle: String {
        if let mediaTitle, !mediaTitle.isEmpty {
            return "\(mediaTitle) — \(machineDisplayTitle)"
        }
        return "Spec Chum — \(machineDisplayTitle)"
    }

    var handle: UnsafeMutableRawPointer?
    let romSearchRoots: [URL]
    /// Wall-clock gate so SwiftUI over-scheduling cannot turbo the Spectrum.
    var lastFrameUptime: TimeInterval = 0
    static let framePeriod: TimeInterval = 1.0 / 50.0
    /// After a hitch, advance at most this many Spectrum frames per host tick.
    static let maxCatchUpFrames = 2
    /// `SPEC_CHUM_ROOM_PERF=1` — stderr + footer HUD for Bevy/host hitch diagnosis.
    let roomPerfEnabled =
        ProcessInfo.processInfo.environment["SPEC_CHUM_ROOM_PERF"].map { $0 != "0" && !$0.isEmpty }
            ?? false
    @Published var roomPerfLine: String = ""
    var roomPerfHostFrames: UInt64 = 0
    var roomPerfHostSumMs: Double = 0
    var roomPerfHostMaxMs: Double = 0
    var roomPerfRoomSumMs: Double = 0
    var roomPerfRoomMaxMs: Double = 0
    var roomPerfRoomTicks: UInt64 = 0
    var roomPerfSkippedBusy: UInt64 = 0
    var roomPerfSpectrumFrames: UInt64 = 0
    var roomPerfHitches: UInt64 = 0
    var roomPerfLastHudUptime: TimeInterval = 0
    var roomPerfLastSnap: ScRoomPerfSnapshot?
    let audio = TapeAudioPlayer()
    /// Publish progress at most ~4 Hz to avoid SwiftUI churn.
    var progressPublishCounter: UInt32 = 0
    /// Arrows + Tab → Kempston bits (egui parity); OR’d with gamepad each frame.
    var keyboardJoystickMask: UInt32 = 0
    var joystickModeApplied = false
    /// Accumulated host pointer delta since last `pushMouse` (egui frame clamp parity).
    var pendingMouseDx: CGFloat = 0
    var pendingMouseDy: CGFloat = 0
    var mouseLeft = false
    var mouseRight = false
    var mouseMiddle = false
    var connectObserver: NSObjectProtocol?
    var disconnectObserver: NSObjectProtocol?
    var ensureAudioObserver: NSObjectProtocol?
    /// First successful post-activation audio arm (force-rebuild once).
    var audioOutputArmed = false
    /// Frame-scripted `LOAD ""` [CODE] (egui KeyScript parity); advanced in `runFrame`.
    var keyScript: LoadKeyScript?
    /// After Instant Type LOAD finishes, auto-Play with flash-load still on.
    var pendingInstantPlay = false
    /// Instant left flash-load on; clear it when the deck next stops (or Pause/Play/Rewind).
    var instantFlashActive = false
    /// When syncing `model` from `sc_get_model` after snapshot load, skip `sc_set_model`.
    var suppressModelPush = false
    /// Skip clearing `activeConfigId` when syncing model from a custom profile apply.
    var suppressActiveConfigClear = false
    /// Skip UserDefaults writes while restoring published prefs in bulk.
    var suppressPrefsPersist = false

    /// Toolbar / File label: Disk on +3, TRD on Beta-capable models.
    var openMediaTitle: String {
        if model.supportsDisk {
            "Open Tape / Disk"
        } else if model.supportsBeta {
            "Open Tape / TRD"
        } else {
            "Open Tape"
        }
    }

    /// Menu item with ellipsis.
    var openMediaMenuTitle: String { "\(openMediaTitle)…" }

    var rawHandle: UnsafeMutableRawPointer? { handle }

    var modelRomPaths: [String: String] = HostBridge.loadPersistedModelRomPaths()
    var suppressTapeOptsPush = false
    var suppressPausedPush = false

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
        maybeEmbedAgentServer()
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

    func scheduleAutomationOpenTapeIfRequested() {
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

    /// When `SPEC_CHUM_AGENT=1`, embed loopback HTTP on the live session (parity with egui #221).
    func maybeEmbedAgentServer() {
        guard let handle else { return }
        guard ProcessInfo.processInfo.environment["SPEC_CHUM_AGENT"] == "1" else { return }
        if sc_agent_embed_start(handle) == 0 {
            let port = ProcessInfo.processInfo.environment["SPEC_CHUM_AGENT_PORT"] ?? "17384"
            status = "\(status) — agent http://127.0.0.1:\(port)"
            publishAgentHostView()
        } else {
            let detail = HostBridge.takeLastError() ?? "agent embed failed (need SPEC_CHUM_AGENT_TOKEN or SPEC_CHUM_AGENT_INSECURE=1)"
            status = "\(status) — \(detail)"
        }
    }

    /// Publish own-window id + display panel size for `/v1/host/*` (#239).
    ///

    func publishAgentHostView() {
        guard let handle else { return }
        guard ProcessInfo.processInfo.environment["SPEC_CHUM_AGENT"] == "1" else { return }
        let present: NSView? = spectrumPresentView ?? livingRoomPresentView
        if let window = present?.window {
            // windowNumber == CGWindowID; capture path verifies owning PID == self.
            _ = sc_agent_set_host_window_id(handle, UInt32(window.windowNumber))
        }
        if let view = present {
            let b = view.bounds
            let w = UInt32(max(1, b.width.rounded()))
            let h = UInt32(max(1, b.height.rounded()))
            _ = sc_agent_set_display_panel_size(handle, w, h)
        }
    }

    static let volumeDefaultsKey = "specChum.outputVolume"
    static let mutedDefaultsKey = "specChum.outputMuted"
    static let modelDefaultsKey = "specChum.model"
    static let lastBuiltinModelKey = "specChum.lastBuiltinModel"
    static let customConfigsKey = "specChum.customConfigs"
    static let activeConfigIdKey = "specChum.activeConfigId"
    static let maxCustomConfigs = 32
    static let tapeSpeedDefaultsKey = "specChum.tapeEarSpeed"
    static let experienceDefaultsKey = "specChum.tapeExperience"
    static let joystickDefaultsKey = "specChum.joystickMode"
    static let kempstonMouseDefaultsKey = "specChum.kempstonMouse"
    static let recentFilesDefaultsKey = "specChum.recentFiles"
    static let modelRomPathsKey = "specChum.modelRomPaths"
    static let maxRecentFiles = 12

    func ensureAudioOutput() {
        guard let handle else { return }
        let rate = Double(sc_audio_sample_rate(handle))
        let force = !audioOutputArmed
        if audio.ensureStarted(sampleRate: rate > 0 ? rate : 44_100, force: force) {
            audioOutputArmed = true
        }
    }

    func flushInputFrame() {
        InputLatencyProbe.noteKey()
        _ = runFrame()
        roomTickLock.lock()
        let busy = roomTickInFlight
        roomTickLock.unlock()
        guard livingRoomMode, livingRoomReady, !busy else { return }
        onLivingRoomDisplayTick(deltaSeconds: 1.0 / 60.0)
    }

    func startFrameTimer() {
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

    func stopFrameTimer() {
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
            publishAgentHostView()
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
            publishAgentHostView()
            if !livingRoomMode {
                spectrumPresentView?.presentFrame()
            }
            return true
        }
        return false
    }

    func enqueueAudio() {
        guard let handle else { return }
        let n = Int(sc_audio_frames(handle))
        guard n > 0, let ptr = sc_audio_ptr(handle) else { return }
        audio.schedule(samples: ptr, count: n)
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

    func syncModelFromHost() {
        guard let handle else { return }
        let raw = sc_get_model(handle)
        guard raw != UInt32.max, let m = Model(rawValue: raw), m != model else { return }
        suppressModelPush = true
        model = m
        suppressModelPush = false
    }

    func reset() {
        guard let handle else { return }
        if sc_reset(handle) != 0 {
            status = HostBridge.takeLastError() ?? "Reset failed"
        } else {
            refreshStatus()
        }
    }

    func refreshStatus() {
        guard let handle else { return }
        if let cstr = sc_status(handle) {
            status = String(cString: cstr)
            sc_string_free(cstr)
        }
    }

    /// Apply one frame of the Type LOAD script (clear + chord), egui `tick_key_script` parity.
    func tickKeyScript() {
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

    func finishInstantPlayIfPending() {
        guard pendingInstantPlay else { return }
        pendingInstantPlay = false
        playTapeKeepingFlash()
        status = "Instant: flash-loading after LOAD \"\""
    }

    static func takeLastError() -> String? {
        guard let cstr = sc_last_error() else { return nil }
        let s = String(cString: cstr)
        sc_string_free(cstr)
        return s
    }

    static func takeRoomLastError() -> String? {
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
            _ = sc_agent_embed_stop(handle)
            sc_destroy(handle)
        }
    }
}
