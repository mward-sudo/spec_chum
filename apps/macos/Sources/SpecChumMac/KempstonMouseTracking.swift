import AppKit

/// NSTrackingArea + button/motion forwarding for Kempston mouse (Spectrum / living-room views).
enum KempstonMouseTracking {
    static func installTrackingArea(on view: NSView) {
        for area in view.trackingAreas {
            view.removeTrackingArea(area)
        }
        let options: NSTrackingArea.Options = [
            .activeInKeyWindow,
            .mouseMoved,
            .enabledDuringMouseDrag,
            .inVisibleRect,
        ]
        view.addTrackingArea(NSTrackingArea(rect: .zero, options: options, owner: view, userInfo: nil))
    }

    static func handleMotion(host: HostBridge?, event: NSEvent) {
        guard host?.kempstonMouse == true else { return }
        host?.noteMouseDelta(deltaX: event.deltaX, deltaY: event.deltaY)
    }

    /// Returns true when the event was consumed for Kempston buttons.
    @discardableResult
    static func handleButton(
        host: HostBridge?,
        buttonNumber: Int,
        pressed: Bool
    ) -> Bool {
        guard host?.kempstonMouse == true else { return false }
        host?.noteMouseButton(buttonNumber: buttonNumber, pressed: pressed)
        return true
    }
}
