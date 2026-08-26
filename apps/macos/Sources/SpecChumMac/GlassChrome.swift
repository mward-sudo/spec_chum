import SwiftUI

/// Liquid glass when the SDK/OS supports it; otherwise ultra-thin material.
/// Floating chrome over the living-room scene (status). Primary actions use
/// the system `.toolbar` — not custom glass chips — per HIG.
struct GlassBarBackground: ViewModifier {
    func body(content: Content) -> some View {
        if #available(macOS 26, *) {
            content
                .glassEffect(.regular, in: .rect(cornerRadius: 12))
        } else {
            content
                .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                .overlay(
                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                        .strokeBorder(.white.opacity(0.12), lineWidth: 0.5)
                )
        }
    }
}

extension View {
    func glassBarBackground() -> some View {
        modifier(GlassBarBackground())
    }
}
