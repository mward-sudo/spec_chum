import AppKit
import Foundation
import GameController
import IOSurface
import UniformTypeIdentifiers
import CSpecChumHost

/// Opt-in input timing probe (`SPEC_CHUM_INPUT_LATENCY=1` → `/tmp/spec-input-latency.log`).
/// Samples should be spaced apart; overlapping samples are reported as dropped.
enum InputLatencyProbe {
    private struct Sample {
        let id: Int
        let started: TimeInterval
        let mode: String
        var stage: Int
        var coreFrameStarted: TimeInterval?
    }

    static let enabled =
        ProcessInfo.processInfo.environment["SPEC_CHUM_INPUT_LATENCY"] == "1"
    private static let lock = NSLock()
    private static let logQueue = DispatchQueue(label: "dev.specchum.input-latency-log", qos: .utility)
    private static var nextSample = 0
    private static var sample: Sample?

    static func noteKey(mode: String) {
        guard enabled else { return }
        let now = ProcessInfo.processInfo.systemUptime
        lock.lock()
        if let previous = sample {
            enqueueLog("sample=\(previous.id) dropped=overlap")
        }
        nextSample &+= 1
        sample = Sample(id: nextSample, started: now, mode: mode, stage: 0, coreFrameStarted: nil)
        enqueueLog(String(format: "sample=%d mode=%@ stage=key_down", nextSample, mode))
        lock.unlock()
    }

    static func noteHostInputApplied() { advance("host_input_applied", to: 1) }

    static func noteCoreFrameStarted() {
        guard enabled else { return }
        lock.lock()
        guard var current = sample, current.stage == 1, current.coreFrameStarted == nil else {
            lock.unlock()
            return
        }
        let now = ProcessInfo.processInfo.systemUptime
        current.coreFrameStarted = now
        sample = current
        recordLocked("core_frame_started", current: current)
        lock.unlock()
    }

    static func noteCoreFrame() {
        guard enabled else { return }
        lock.lock()
        guard var current = sample, current.stage < 2 else {
            lock.unlock()
            return
        }
        current.stage = 2
        sample = current
        let frameMs = current.coreFrameStarted.map { (ProcessInfo.processInfo.systemUptime - $0) * 1000 } ?? 0
        recordLocked("core_frame", current: current, detail: String(format: " frame_ms=%.2f", frameMs))
        lock.unlock()
    }

    static func noteFbPublish() { advance("living_room_fb_published", to: 3, mode: "living_room") }

    static func noteFbUploaded() { advance("living_room_fb_uploaded", to: 4, mode: "living_room") }
    static func noteRoomTickStarted() { advance("room_tick_started", to: 5, mode: "living_room") }
    static func noteRoomTickFinished() { advance("room_tick_finished", to: 6, mode: "living_room") }

    static func noteFlatDraw() { finish("flat_draw", mode: "flat", after: 2, at: 3) }

    static func noteRoomPresent() {
        finish("living_room_layer_refresh", mode: "living_room", after: 6, at: 7)
    }

    static func noteScroll(steps: Int32) {
        guard enabled else { return }
        enqueueLog("scroll steps=\(steps)")
    }

    private static func advance(_ stage: String, to nextStage: Int, mode: String? = nil) {
        guard enabled else { return }
        lock.lock()
        guard var current = sample,
              current.stage == nextStage - 1,
              mode == nil || current.mode == mode
        else {
            lock.unlock()
            return
        }
        current.stage = nextStage
        sample = current
        recordLocked(stage, current: current)
        lock.unlock()
    }

    private static func finish(_ stage: String, mode: String, after expectedStage: Int, at finalStage: Int) {
        guard enabled else { return }
        lock.lock()
        guard var current = sample, current.stage == expectedStage, current.mode == mode else {
            lock.unlock()
            return
        }
        current.stage = finalStage
        recordLocked(stage, current: current)
        sample = nil
        lock.unlock()
    }

    private static func recordLocked(_ stage: String, current: Sample, detail: String = "") {
        let elapsedMs = (ProcessInfo.processInfo.systemUptime - current.started) * 1000
        enqueueLog(String(format: "sample=%d mode=%@ stage=%@ elapsed_ms=%.2f%@", current.id, current.mode, stage, elapsedMs, detail))
    }

    /// Keep disk I/O off the AppKit and render queues so the diagnostic does not stall input.
    private static func enqueueLog(_ message: String) {
        logQueue.async {
            let line = "\(ProcessInfo.processInfo.systemUptime) \(message)\n"
            let url = URL(fileURLWithPath: "/tmp/spec-input-latency.log")
            if let handle = try? FileHandle(forWritingTo: url) {
                defer { try? handle.close() }
                _ = try? handle.seekToEnd()
                try? handle.write(contentsOf: Data(line.utf8))
            } else {
                try? Data(line.utf8).write(to: url)
            }
        }
    }
}

extension HostBridge {
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

    func clearGuestMouseButtons() {
        guard let handle else { return }
        _ = sc_set_mouse_buttons(handle, 0, 0, 0)
    }

    /// Clamp accumulated motion to i8 and push buttons (egui per-frame parity).
    func pushMouse() {
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

    func pushJoystick() {
        guard let handle else { return }
        if !joystickModeApplied {
            _ = sc_set_joystick_mode(handle, JoystickMode.kempston.rawValue)
            joystickModeApplied = true
        }
        let mask = keyboardJoystickMask | Self.gamepadMask()
        _ = sc_set_joystick(handle, mask)
    }

    func startGamepadDiscovery() {
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
    static func gamepadMask() -> UInt32 {
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

}
