import AppKit

/// Ensures SpecChum becomes the key application after launch (Terminal must not keep keys).
final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationWillFinishLaunching(_ notification: Notification) {
        // Belt-and-suspenders before the first window is created.
        NSWindow.allowsAutomaticWindowTabbing = false
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.regular)
        FocusSpectrumView.installMenuTrackingObservers()
        for window in NSApp.windows {
            WindowChrome.applyTabBarPolicy(to: window)
        }
        activateSpecChum()
    }

    func applicationDidBecomeActive(_ notification: Notification) {
        NotificationCenter.default.post(name: .specChumEnsureAudio, object: nil)
        FocusSpectrumView.post()
    }
}

extension Notification.Name {
    /// Posted when the app becomes active — HostBridge retries AudioQueue startup.
    static let specChumEnsureAudio = Notification.Name("SpecChumEnsureAudio")
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
