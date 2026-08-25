import SwiftUI

/// Liquid glass when the SDK/OS supports it; otherwise ultra-thin material.
/// Used for the status footer (and any residual chrome). Primary actions use
/// the system `.toolbar` — not custom glass chips — per HIG.
struct GlassBarBackground: ViewModifier {
    func body(content: Content) -> some View {
        if #available(macOS 26, *) {
            content
                .glassEffect(.regular, in: .rect(cornerRadius: 10))
        } else {
            content
                .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        }
    }
}

extension View {
    func glassBarBackground() -> some View {
        modifier(GlassBarBackground())
    }
}
