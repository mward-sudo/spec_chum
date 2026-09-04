import AppKit
import Foundation
import GameController
import IOSurface
import UniformTypeIdentifiers
import CSpecChumHost

/// Temporary input-latency probe (`SPEC_CHUM_INPUT_LATENCY=1` → `/tmp/spec-input-latency.log`).
enum InputLatencyProbe {
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
