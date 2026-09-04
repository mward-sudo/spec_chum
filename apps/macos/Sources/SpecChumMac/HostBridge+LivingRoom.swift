import AppKit
import Foundation
import GameController
import IOSurface
import UniformTypeIdentifiers
import CSpecChumHost

extension HostBridge {
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
    func performLivingRoomPresentBind(surface: IOSurface, width: UInt32, height: UInt32) {
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

    func enqueueLivingRoomPresentBind(
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
    func ensureLivingRoom() {
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
    func scheduleLivingRoomPreload() {
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
    func createLivingRoomOnQueue(warmupTicks: Int) -> String? {
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
    func destroyLivingRoom(syncTeardown: Bool) {
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
    func publishLivingRoomFramebuffer() {
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
    func shouldRunRoomTick(generation: UInt64) -> Bool {
        roomTickLock.lock()
        defer { roomTickLock.unlock() }
        return roomTickInFlight && generation == roomTickGeneration
    }

    /// Clear coalesce flag after a room tick finishes or is abandoned.
    func finishRoomTick(generation: UInt64) {
        roomTickLock.lock()
        defer { roomTickLock.unlock() }
        guard generation == roomTickGeneration else { return }
        roomTickInFlight = false
        roomTickStartedUptime = 0
    }

    /// If Bevy/GPU blocked the room queue too long, drop the coalesce gate so DisplayLink can resume.
    func recoverStuckRoomTickIfNeeded() {
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

    func noteHostFrame(elapsedMs: Double) {
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

    func publishRoomPerfHud() {
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

}
