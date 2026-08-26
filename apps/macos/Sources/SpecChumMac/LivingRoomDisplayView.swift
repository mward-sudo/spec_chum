import AppKit
import CSpecChumHost
import CoreVideo
import IOSurface
import QuartzCore
import SwiftUI

/// Full-window Bevy living-room present via shared IOSurface (GPU blit, no CGImage).
///
/// Bevy is paced by `CADisplayLink` (monitor refresh). Present long-edge tracks
/// a stepped size (60 Hz refresh, up to 2560 backing pixels). Layer filters are **linear**; phosphor
/// texels stay nearest in Rust.
struct LivingRoomDisplayView: NSViewRepresentable {
    @ObservedObject var host: HostBridge

    func makeNSView(context: Context) -> LivingRoomNSView {
        let view = LivingRoomNSView()
        view.host = host
        host.attachLivingRoomPresentView(view)
        return view
    }

    func updateNSView(_ nsView: LivingRoomNSView, context: Context) {
        nsView.host = host
        host.attachLivingRoomPresentView(nsView)
        nsView.syncPresentTargetIfNeeded()
        nsView.scheduleSteppedResizeIfNeeded()
    }
}

final class LivingRoomNSView: NSView {
    weak var host: HostBridge? {
        didSet {
            if host?.livingRoomMode == true {
                syncPresentTargetIfNeeded()
                startDisplayLinkIfNeeded()
            } else {
                stopDisplayLink()
                presentBound = false
                presentSurface = nil
                layer?.contents = nil
            }
        }
    }
    private var held: Set<UInt16> = []
    private var keyMonitor: Any?
    private var becomeKeyObserver: NSObjectProtocol?
    private var resignKeyObserver: NSObjectProtocol?
    private var focusRequestObserver: NSObjectProtocol?
    private var presentSurface: IOSurface?
    private var presentBound = false
    private var presentWidth = 1920
    private var presentHeight = 1080
    private var displayLink: CADisplayLink?
    private var resizeWorkItem: DispatchWorkItem?

    override var acceptsFirstResponder: Bool { true }
    override var canBecomeKeyView: Bool { true }
    override var isFlipped: Bool { false }

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        focusRingType = .none
        wantsLayer = true
        // We set `layer.contents` to a live IOSurface — never route through `-draw`.
        layerContentsRedrawPolicy = .never
        layer?.contentsGravity = .resizeAspect
        // Linear: upscale the composited 3D frame (phosphor stays nearest in Bevy).
        layer?.magnificationFilter = .linear
        layer?.minificationFilter = .linear
        layer?.backgroundColor = NSColor.black.cgColor
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    deinit {
        stopDisplayLink()
        tearDownObservers()
        removeKeyMonitor()
        releaseAllKeys()
        host?.clearLivingRoomPresent()
    }

    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        tearDownObservers()
        removeKeyMonitor()
        stopDisplayLink()
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
        syncPresentTargetIfNeeded()
        startDisplayLinkIfNeeded()
        scheduleSteppedResizeIfNeeded()
    }

    override func layout() {
        super.layout()
        scheduleSteppedResizeIfNeeded()
    }

    override func drawFocusRingMask() {}
    override var focusRingMaskBounds: NSRect { .zero }

    func refreshLayerContents() {
        guard let presentSurface else { return }
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        // Re-assign after in-place Metal blit so Core Animation picks up new pixels.
        layer?.contents = nil
        layer?.contents = presentSurface
        CATransaction.commit()
        needsDisplay = true
    }

    /// Bind IOSurface at current stepped size (or after living-room toggle).
    func syncPresentTargetIfNeeded() {
        guard host?.livingRoomMode == true else { return }
        if presentBound, presentSurface != nil {
            refreshLayerContents()
            startDisplayLinkIfNeeded()
            return
        }
        rebindPresentSurface(width: presentWidth, height: presentHeight)
        startDisplayLinkIfNeeded()
    }

    /// Debounced stepped resize from view backing pixels.
    func scheduleSteppedResizeIfNeeded() {
        guard host?.livingRoomMode == true, window != nil else { return }
        let target = Self.steppedSize(for: self)
        guard target.width != presentWidth || target.height != presentHeight else { return }
        resizeWorkItem?.cancel()
        let work = DispatchWorkItem { [weak self] in
            guard let self else { return }
            let again = Self.steppedSize(for: self)
            guard again.width != self.presentWidth || again.height != self.presentHeight else { return }
            self.rebindPresentSurface(width: again.width, height: again.height)
        }
        resizeWorkItem = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.35, execute: work)
    }

    private func rebindPresentSurface(width: Int, height: Int) {
        guard host?.livingRoomMode == true else { return }
        guard let surface = Self.makeIOSurface(width: width, height: height) else { return }
        presentWidth = width
        presentHeight = height
        presentSurface = surface
        presentBound = true
        host?.bindLivingRoomPresent(surface: surface, width: UInt32(width), height: UInt32(height))
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        layer?.contents = surface
        layer?.contentsScale = 1
        CATransaction.commit()
    }

    /// Stepped long-edge budget, aspect matched to the view (no 16:9 letterboxing).
    /// Cap at **2560** backing pixels; CALayer linear-upscales when the view is larger.
    static func steppedSize(for view: NSView) -> (width: Int, height: Int) {
        let scale = max(view.window?.backingScaleFactor ?? 2.0, 1.0)
        let bw = max(view.bounds.width * scale, 1)
        let bh = max(view.bounds.height * scale, 1)
        let aspect = bw / bh
        let budget: CGFloat = min(max(bw, bh), 2560)
        var w: Int
        var h: Int
        if aspect >= 1 {
            w = Int(budget.rounded())
            h = max(2, Int((budget / aspect).rounded()))
        } else {
            h = Int(budget.rounded())
            w = max(2, Int((budget * aspect).rounded()))
        }
        // Even dimensions are friendlier for GPU blit / IOSurface.
        w &= ~1
        h &= ~1
        return (max(w, 2), max(h, 2))
    }

    private static func makeIOSurface(width: Int, height: Int) -> IOSurface? {
        let bytesPerElement = 4
        let bytesPerRow = width * bytesPerElement
        let props: [IOSurfacePropertyKey: Any] = [
            .width: width,
            .height: height,
            .bytesPerElement: bytesPerElement,
            .bytesPerRow: bytesPerRow,
            .pixelFormat: Int(kCVPixelFormatType_32BGRA),
        ]
        return IOSurface(properties: props)
    }

    private func startDisplayLinkIfNeeded() {
        guard host?.livingRoomMode == true, window != nil, displayLink == nil else { return }
        guard let screen = window?.screen else { return }
        let link = screen.displayLink(target: self, selector: #selector(displayLinkFired(_:)))
        // Prefer the panel max (120 on ProMotion, 60 elsewhere).
        if #available(macOS 14.0, *) {
            let maxHz = Float(screen.maximumFramesPerSecond)
            link.preferredFrameRateRange = CAFrameRateRange(
                minimum: min(60, maxHz),
                maximum: maxHz,
                preferred: maxHz
            )
        }
        link.add(to: .main, forMode: .common)
        displayLink = link
    }

    private func stopDisplayLink() {
        displayLink?.invalidate()
        displayLink = nil
    }

    @objc private func displayLinkFired(_ link: CADisplayLink) {
        host?.onLivingRoomDisplayTick(deltaSeconds: link.duration)
    }

    override func mouseDown(with event: NSEvent) {
        claimFocus()
        host?.skipLivingRoomIntro()
        super.mouseDown(with: event)
    }

    override func scrollWheel(with event: NSEvent) {
        claimFocus()
        let dy = event.hasPreciseScrollingDeltas ? event.scrollingDeltaY : event.scrollingDeltaY * 12
        host?.nudgeLivingRoomZoom(deltaY: dy)
    }

    override func keyDown(with event: NSEvent) {
        if event.modifierFlags.contains(.command) {
            releaseAllKeys()
            super.keyDown(with: event)
            return
        }
        if event.isARepeat { return }
        if event.keyCode == 49 || event.keyCode == 53 {
            host?.skipLivingRoomIntro()
        }
        applyKey(code: event.keyCode, pressed: true, flags: event.modifierFlags)
    }

    override func keyUp(with event: NSEvent) {
        if event.modifierFlags.contains(.command) {
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
        applyKey(code: event.keyCode, pressed: false, flags: event.modifierFlags)
        for code in held {
            applyKey(code: code, pressed: true, flags: event.modifierFlags)
        }
    }

    // MARK: - Matrix sync

    private func applyKey(code: UInt16, pressed: Bool, flags: NSEvent.ModifierFlags) {
        if pressed {
            if !held.insert(code).inserted {
                return
            }
        } else if held.remove(code) == nil {
            return
        }
        syncMatrix(flags: flags)
    }

    private func syncMatrix(flags: NSEvent.ModifierFlags) {
        let suppressCaps = held.contains { SpectrumKeymap.suppressesModifierCaps(keyCode: $0) }
        var modifiers = SpectrumKeymap.modifierKeys(flags: flags, suppressCaps: suppressCaps)
        var matrixHeld: [(UInt32, UInt32)] = []
        for code in held {
            let chord = SpectrumKeymap.chords(keyCode: code, flags: flags)
            if SpectrumKeymap.suppressesModifierCaps(keyCode: code) {
                matrixHeld.append(contentsOf: chord)
            } else {
                matrixHeld.append(contentsOf: chord.filter { (row, bit) in
                    (row, bit) != SpectrumKeymap.caps && (row, bit) != SpectrumKeymap.sym
                })
            }
        }
        // Dedupe while preserving order.
        var seen = Set<[UInt32]>()
        modifiers = modifiers.filter { seen.insert([$0.0, $0.1]).inserted }
        matrixHeld = matrixHeld.filter { seen.insert([$0.0, $0.1]).inserted }
        host?.syncKeyboardMatrix(
            modifiers: modifiers,
            held: matrixHeld,
            joystickMask: SpectrumKeymap.kempstonMask(held: held)
        )
    }

    private func releaseAllKeys() {
        held.removeAll()
        host?.clearKeys()
        host?.setKeyboardJoystickMask(0)
    }

    private func claimFocus() {
        activateSpecChum()
        guard let window else { return }
        window.makeKeyAndOrderFront(nil)
        window.makeFirstResponder(self)
    }

    /// Backup capture when we are not first responder but the window is key.
    private func shouldMonitorCapture() -> Bool {
        guard let window, window.isKeyWindow else { return false }
        guard let fr = window.firstResponder else { return true }
        if fr === self { return false }
        if fr is NSTextView || fr is NSTextField { return false }
        if fr is NSControl { return false }
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

            guard self.shouldMonitorCapture() else { return event }

            switch event.type {
            case .keyDown:
                if !event.isARepeat { self.keyDown(with: event) }
                return nil
            case .keyUp:
                self.keyUp(with: event)
                return nil
            case .flagsChanged:
                self.flagsChanged(with: event)
                return nil
            default:
                return event
            }
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

    private func removeKeyMonitor() {
        if let keyMonitor {
            NSEvent.removeMonitor(keyMonitor)
            self.keyMonitor = nil
        }
    }
}
