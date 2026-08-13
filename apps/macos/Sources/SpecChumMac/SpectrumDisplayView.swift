import AppKit
import SwiftUI

struct SpectrumDisplayView: NSViewRepresentable {
    @ObservedObject var host: HostBridge
    var tick: UInt64

    func makeNSView(context: Context) -> SpectrumNSView {
        let view = SpectrumNSView()
        view.host = host
        return view
    }

    func updateNSView(_ nsView: SpectrumNSView, context: Context) {
        nsView.host = host
        _ = tick
        nsView.needsDisplay = true
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
    private var held: Set<UInt16> = []
    private var keyMonitor: Any?
    private var becomeKeyObserver: NSObjectProtocol?
    private var resignKeyObserver: NSObjectProtocol?
    private var focusRequestObserver: NSObjectProtocol?

    override var acceptsFirstResponder: Bool { true }
    override var canBecomeKeyView: Bool { true }
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
    }

    deinit {
        tearDownObservers()
        removeKeyMonitor()
    }

    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override func mouseDown(with event: NSEvent) {
        claimFocus()
        super.mouseDown(with: event)
    }

    override func becomeFirstResponder() -> Bool {
        let ok = super.becomeFirstResponder()
        needsDisplay = true
        return ok
    }

    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        NSColor.black.setFill()
        bounds.fill()

        guard let image = host?.makeFrameImage() else { return }
        let imgSize = image.size
        guard imgSize.width > 0, imgSize.height > 0 else { return }

        let scale = min(bounds.width / imgSize.width, bounds.height / imgSize.height)
        let drawW = imgSize.width * scale
        let drawH = imgSize.height * scale
        let rect = NSRect(
            x: (bounds.width - drawW) / 2,
            y: (bounds.height - drawH) / 2,
            width: drawW,
            height: drawH
        )

        NSGraphicsContext.current?.imageInterpolation = .none
        image.draw(
            in: rect,
            from: NSRect(origin: .zero, size: imgSize),
            operation: .copy,
            fraction: 1.0,
            respectFlipped: true,
            hints: [.interpolation: NSImageInterpolation.none]
        )
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
    }

    /// Backup capture only when we are not first responder but the window is key.
    private func shouldMonitorCapture() -> Bool {
        guard let window, window.isKeyWindow else { return false }
        guard let fr = window.firstResponder else { return true }
        if fr === self { return false }
        if fr is NSTextView || fr is NSTextField { return false }
        if fr is NSControl { return false }
        // SwiftUI hosting views are plain NSViews — monitor fills the gap.
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

            // Prefer the first-responder path when we own focus.
            guard self.shouldMonitorCapture() else { return event }

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
    }

    private func releaseAllKeys() {
        held.removeAll()
        host?.clearKeys()
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
        let suppressCaps = held.contains { SpectrumKeymap.suppressesModifierCaps(keyCode: $0) }
        for (row, bit) in SpectrumKeymap.modifierKeys(flags: flags, suppressCaps: suppressCaps) {
            host?.setKey(row: row, bit: bit, pressed: true)
        }
        for code in held {
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
    }
}
