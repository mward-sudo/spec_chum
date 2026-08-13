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

final class SpectrumNSView: NSView {
    weak var host: HostBridge?
    private var held: Set<UInt16> = []

    override var acceptsFirstResponder: Bool { true }
    override var isFlipped: Bool { false }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        window?.makeFirstResponder(self)
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

        // Nearest-neighbor when scaling up Spectrum pixels.
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
        apply(event: event, pressed: true)
    }

    override func keyUp(with event: NSEvent) {
        apply(event: event, pressed: false)
    }

    override func flagsChanged(with event: NSEvent) {
        // Rebuild matrix from currently tracked keys on modifier changes.
        host?.clearKeys()
        for code in held {
            if let synth = NSEvent.keyEvent(
                with: .keyDown,
                location: .zero,
                modifierFlags: event.modifierFlags,
                timestamp: 0,
                windowNumber: window?.windowNumber ?? 0,
                context: nil,
                characters: "",
                charactersIgnoringModifiers: "",
                isARepeat: false,
                keyCode: code
            ) {
                for (row, bit) in SpectrumKeymap.chords(for: synth) {
                    host?.setKey(row: row, bit: bit, pressed: true)
                }
            }
        }
        // Still apply shift/option alone.
        let flags = event.modifierFlags
        if flags.contains(.shift) {
            host?.setKey(row: SpectrumKeymap.caps.0, bit: SpectrumKeymap.caps.1, pressed: true)
        }
        if flags.contains(.option) || flags.contains(.control) {
            host?.setKey(row: SpectrumKeymap.sym.0, bit: SpectrumKeymap.sym.1, pressed: true)
        }
    }

    private func apply(event: NSEvent, pressed: Bool) {
        if pressed {
            held.insert(event.keyCode)
        } else {
            held.remove(event.keyCode)
        }
        host?.clearKeys()
        for code in held {
            if let synth = NSEvent.keyEvent(
                with: .keyDown,
                location: .zero,
                modifierFlags: event.modifierFlags,
                timestamp: 0,
                windowNumber: window?.windowNumber ?? 0,
                context: nil,
                characters: event.characters ?? "",
                charactersIgnoringModifiers: event.charactersIgnoringModifiers ?? "",
                isARepeat: false,
                keyCode: code
            ) {
                for (row, bit) in SpectrumKeymap.chords(for: synth) {
                    host?.setKey(row: row, bit: bit, pressed: true)
                }
            }
        }
    }
}
