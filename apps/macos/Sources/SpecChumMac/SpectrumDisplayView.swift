import AppKit
import CSpecChumHost
import SwiftUI

struct SpectrumDisplayView: NSViewRepresentable {
    @ObservedObject var host: HostBridge

    func makeNSView(context: Context) -> SpectrumNSView {
        let view = SpectrumNSView()
        view.host = host
        host.attachSpectrumPresentView(view)
        DispatchQueue.main.async {
            view.claimFocus()
        }
        return view
    }

    func updateNSView(_ nsView: SpectrumNSView, context: Context) {
        nsView.host = host
        host.attachSpectrumPresentView(nsView)
    }
}

/// Spectrum framebuffer view that owns keyboard focus and injects matrix keys.
///
/// SwiftUI focus rings are decorative and do not make the process key. We activate
/// `NSApp`, make the window key, and become first responder. Prefer first-responder
/// `keyDown` / `keyUp` / `flagsChanged`. A local monitor is only a backup when a
/// SwiftUI host steals first responder while the window stays key — it must not
/// inject when we already own focus (would double-deliver presses).
///
/// Autorepeat (`isARepeat`) is ignored: Spectrum keeps a key held until keyUp;
/// OS repeats would look like press edges and spam 48K keywords (e.g. LOAD).
final class SpectrumNSView: NSView {
    weak var host: HostBridge?
    private var bitmapRep: NSBitmapImageRep?
    private var bitmapWidth = 0
    private var bitmapHeight = 0
    private var held: Set<UInt16> = []
    private var keyMonitor: Any?
    private var becomeKeyObserver: NSObjectProtocol?
    private var resignKeyObserver: NSObjectProtocol?
    private var focusRequestObserver: NSObjectProtocol?
    private var menuEndTrackingObserver: NSObjectProtocol?

    override var acceptsFirstResponder: Bool { true }
    /// Do not join the key-view loop — arrow keys must reach `keyDown`, not `moveLeft:` et al.
    override var canBecomeKeyView: Bool { false }
    override var isFlipped: Bool { false }

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        focusRingType = .none
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    override func drawFocusRingMask() {
        // No decorative focus ring — keyboard focus is still accepted.
    }

    override var focusRingMaskBounds: NSRect { .zero }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        KempstonMouseTracking.installTrackingArea(on: self)
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        tearDownObservers()
        removeKeyMonitor()
        guard let window else { return }
        claimFocus()
        installKeyMonitor()
        becomeKeyObserver = NotificationCenter.default.addObserver(
            forName: NSWindow.didBecomeKeyNotification,
            object: window,
            queue: .main
        ) { [weak self] _ in
            self?.claimFocus()
        }
        resignKeyObserver = NotificationCenter.default.addObserver(
            forName: NSWindow.didResignKeyNotification,
            object: window,
            queue: .main
        ) { [weak self] _ in
            self?.releaseAllKeys()
        }
        focusRequestObserver = NotificationCenter.default.addObserver(
            forName: FocusSpectrumView.name,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            self?.claimFocus()
        }
        menuEndTrackingObserver = NotificationCenter.default.addObserver(
            forName: NSMenu.didEndTrackingNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            self?.claimFocus()
        }
    }

    deinit {
        tearDownObservers()
        removeKeyMonitor()
    }

    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override func mouseDown(with event: NSEvent) {
        claimFocus()
        if KempstonMouseTracking.handleButton(host: host, buttonNumber: 0, pressed: true) {
            return
        }
        super.mouseDown(with: event)
    }

    override func mouseUp(with event: NSEvent) {
        if KempstonMouseTracking.handleButton(host: host, buttonNumber: 0, pressed: false) {
            return
        }
        super.mouseUp(with: event)
    }

    override func rightMouseDown(with event: NSEvent) {
        claimFocus()
        if KempstonMouseTracking.handleButton(host: host, buttonNumber: 1, pressed: true) {
            return
        }
        super.rightMouseDown(with: event)
    }

    override func rightMouseUp(with event: NSEvent) {
        if KempstonMouseTracking.handleButton(host: host, buttonNumber: 1, pressed: false) {
            return
        }
        super.rightMouseUp(with: event)
    }

    override func otherMouseDown(with event: NSEvent) {
        claimFocus()
        if KempstonMouseTracking.handleButton(host: host, buttonNumber: Int(event.buttonNumber), pressed: true) {
            return
        }
        super.otherMouseDown(with: event)
    }

    override func otherMouseUp(with event: NSEvent) {
        if KempstonMouseTracking.handleButton(host: host, buttonNumber: Int(event.buttonNumber), pressed: false) {
            return
        }
        super.otherMouseUp(with: event)
    }

    override func mouseMoved(with event: NSEvent) {
        KempstonMouseTracking.handleMotion(host: host, event: event)
        super.mouseMoved(with: event)
    }

    override func mouseDragged(with event: NSEvent) {
        KempstonMouseTracking.handleMotion(host: host, event: event)
        super.mouseDragged(with: event)
    }

    override func rightMouseDragged(with event: NSEvent) {
        KempstonMouseTracking.handleMotion(host: host, event: event)
        super.rightMouseDragged(with: event)
    }

    override func otherMouseDragged(with event: NSEvent) {
        KempstonMouseTracking.handleMotion(host: host, event: event)
        super.otherMouseDragged(with: event)
    }

    override func becomeFirstResponder() -> Bool {
        let ok = super.becomeFirstResponder()
        needsDisplay = true
        return ok
    }

    /// Called from the 50 Hz host timer — avoids @Published SwiftUI invalidation every frame.
    func presentFrame() {
        if Thread.isMainThread {
            needsDisplay = true
        } else {
            DispatchQueue.main.async { [weak self] in
                self?.needsDisplay = true
            }
        }
    }

    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        NSColor.black.setFill()
        bounds.fill()

        guard let handle = host?.rawHandle,
              let src = sc_framebuffer_ptr(handle)
        else { return }
        let w = Int(sc_framebuffer_width(handle))
        let h = Int(sc_framebuffer_height(handle))
        guard w > 0, h > 0, let rep = ensureBitmap(width: w, height: h),
              let dst = rep.bitmapData
        else { return }

        let byteLen = w * h * 4
        dst.update(from: src, count: byteLen)

        guard let cgImage = rep.cgImage else { return }
        let imgSize = NSSize(width: w, height: h)
        let scale = min(bounds.width / imgSize.width, bounds.height / imgSize.height)
        let drawW = imgSize.width * scale
        let drawH = imgSize.height * scale
        let rect = NSRect(
            x: (bounds.width - drawW) / 2,
            y: (bounds.height - drawH) / 2,
            width: drawW,
            height: drawH
        )

        guard let ctx = NSGraphicsContext.current?.cgContext else { return }
        ctx.interpolationQuality = .none
        ctx.draw(cgImage, in: rect)
    }

    private func ensureBitmap(width: Int, height: Int) -> NSBitmapImageRep? {
        if let rep = bitmapRep, bitmapWidth == width, bitmapHeight == height {
            return rep
        }
        guard let rep = NSBitmapImageRep(
            bitmapDataPlanes: nil,
            pixelsWide: width,
            pixelsHigh: height,
            bitsPerSample: 8,
            samplesPerPixel: 4,
            hasAlpha: true,
            isPlanar: false,
            colorSpaceName: .deviceRGB,
            bytesPerRow: width * 4,
            bitsPerPixel: 32
        ) else { return nil }
        bitmapRep = rep
        bitmapWidth = width
        bitmapHeight = height
        return rep
    }

    override func keyDown(with event: NSEvent) {
        if event.modifierFlags.contains(.command) {
            releaseAllKeys()
            super.keyDown(with: event)
            return
        }
        // Ignore OS autorepeat — hold until keyUp.
        guard !event.isARepeat else { return }
        applyKey(code: event.keyCode, pressed: true, flags: event.modifierFlags)
    }

    override func keyUp(with event: NSEvent) {
        if event.modifierFlags.contains(.command) {
            // keyUp for a letter after ⌘chord still carries .command — clear stuck bits.
            releaseAllKeys()
            super.keyUp(with: event)
            return
        }
        applyKey(code: event.keyCode, pressed: false, flags: event.modifierFlags)
    }

    override func flagsChanged(with event: NSEvent) {
        if event.modifierFlags.contains(.command) {
            releaseAllKeys()
            return
        }
        syncMatrix(flags: event.modifierFlags)
    }

    // MARK: - Focus / monitor

    func claimFocus() {
        activateSpecChum()
        guard let window else { return }
        window.makeKeyAndOrderFront(nil)
        window.makeFirstResponder(self)
        // Toolbar Machine pickers often steal first responder until the menu closes.
        DispatchQueue.main.async { [weak self] in
            guard let self, let window = self.window, window.isKeyWindow else { return }
            if window.firstResponder !== self {
                window.makeFirstResponder(self)
            }
        }
    }

    /// Backup capture when the window is key but AppKit would not deliver `keyDown` here.
    private func shouldMonitorCapture(for event: NSEvent) -> Bool {
        guard let window, window.isKeyWindow else { return false }
        if FocusSpectrumView.isMenuTracking { return false }
        // Arrows / Tab: always inject — even as first responder AppKit may route via key-view loop.
        if event.type == .keyDown || event.type == .keyUp,
           SpectrumKeymap.isJoystickRoutingKey(keyCode: event.keyCode)
        {
            return true
        }
        guard let fr = window.firstResponder else { return true }
        if fr === self { return false }
        // Only defer to real text fields — toolbar buttons/menus are NSControls too.
        if fr is NSTextView || fr is NSTextField { return false }
        return true
    }

    private func installKeyMonitor() {
        removeKeyMonitor()
        keyMonitor = NSEvent.addLocalMonitorForEvents(matching: [.keyDown, .keyUp, .flagsChanged]) {
            [weak self] event in
            guard let self else { return event }

            if event.modifierFlags.contains(.command) {
                self.releaseAllKeys()
                return event
            }

            if FocusSpectrumView.isMenuTracking {
                return event
            }

            guard self.shouldMonitorCapture(for: event) else { return event }

            switch event.type {
            case .keyDown:
                if !event.isARepeat {
                    self.applyKey(code: event.keyCode, pressed: true, flags: event.modifierFlags)
                }
                // Consume to avoid AppKit beep when we are not first responder.
                return nil
            case .keyUp:
                self.applyKey(code: event.keyCode, pressed: false, flags: event.modifierFlags)
                return nil
            case .flagsChanged:
                self.syncMatrix(flags: event.modifierFlags)
                return event
            default:
                return event
            }
        }
    }

    private func removeKeyMonitor() {
        if let keyMonitor {
            NSEvent.removeMonitor(keyMonitor)
            self.keyMonitor = nil
        }
    }

    private func tearDownObservers() {
        if let becomeKeyObserver {
            NotificationCenter.default.removeObserver(becomeKeyObserver)
            self.becomeKeyObserver = nil
        }
        if let resignKeyObserver {
            NotificationCenter.default.removeObserver(resignKeyObserver)
            self.resignKeyObserver = nil
        }
        if let focusRequestObserver {
            NotificationCenter.default.removeObserver(focusRequestObserver)
            self.focusRequestObserver = nil
        }
        if let menuEndTrackingObserver {
            NotificationCenter.default.removeObserver(menuEndTrackingObserver)
            self.menuEndTrackingObserver = nil
        }
    }

    private func releaseAllKeys() {
        held.removeAll()
        host?.clearKeys()
        host?.setKeyboardJoystickMask(0)
        host?.clearMouseButtons()
    }

    // MARK: - Matrix sync

    private func applyKey(code: UInt16, pressed: Bool, flags: NSEvent.ModifierFlags) {
        if pressed {
            // Duplicate keyDown (or monitor + view) while already held: keep matrix stable.
            if !held.insert(code).inserted {
                return
            }
        } else if held.remove(code) == nil {
            return
        }
        syncMatrix(flags: flags)
    }

    /// Rebuild matrix from `held` + modifiers. Only on real press/release/flags
    /// changes so `run_frame` never observes a cleared intermediate state.
    private func syncMatrix(flags: NSEvent.ModifierFlags) {
        host?.clearKeys()
        let hasNonJoystickHeld = held.contains { !SpectrumKeymap.isJoystickRoutingKey(keyCode: $0) }
        let suppressCaps: Bool
        if hasNonJoystickHeld {
            suppressCaps = held.contains { SpectrumKeymap.suppressesModifierCaps(keyCode: $0) }
        } else {
            suppressCaps = held.contains { code in
                SpectrumKeymap.isJoystickRoutingKey(keyCode: code)
                    || SpectrumKeymap.suppressesModifierCaps(keyCode: code)
            }
        }
        for (row, bit) in SpectrumKeymap.modifierKeys(flags: flags, suppressCaps: suppressCaps) {
            host?.setKey(row: row, bit: bit, pressed: true)
        }
        for code in held {
            if let cursor = SpectrumKeymap.cursorChord(keyCode: code) {
                for (row, bit) in cursor {
                    host?.setKey(row: row, bit: bit, pressed: true)
                }
            }
            if SpectrumKeymap.isJoystickRoutingKey(keyCode: code) {
                continue
            }
            let chord = SpectrumKeymap.chords(keyCode: code, flags: flags)
            if SpectrumKeymap.suppressesModifierCaps(keyCode: code) {
                for (row, bit) in chord {
                    host?.setKey(row: row, bit: bit, pressed: true)
                }
            } else {
                for (row, bit) in chord where (row, bit) != SpectrumKeymap.caps && (row, bit) != SpectrumKeymap.sym {
                    host?.setKey(row: row, bit: bit, pressed: true)
                }
            }
        }
        // Kempston mirror (egui: arrows + Tab fire) — OR’d with GCController in HostBridge.
        host?.setKeyboardJoystickMask(SpectrumKeymap.kempstonMask(held: held))
        host?.flushInputFrame()
    }
}
