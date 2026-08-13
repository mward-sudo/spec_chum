import AppKit

/// Ensures SpecChum becomes the key application after launch (Terminal must not keep keys).
final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.regular)
        activateSpecChum()
    }

    func applicationDidBecomeActive(_ notification: Notification) {
        FocusSpectrumView.post()
    }
}

/// Bring SpecChum to the front as the key app and order its main window key.
func activateSpecChum() {
    // Prefer the modern API; fall back so we still steal key from the launch Terminal.
    if #available(macOS 14, *) {
        NSApp.activate()
    }
    NSApp.activate(ignoringOtherApps: true)
    if let window = NSApp.keyWindow ?? NSApp.windows.first(where: \.isVisible) ?? NSApp.windows.first {
        window.makeKeyAndOrderFront(nil)
    }
}
