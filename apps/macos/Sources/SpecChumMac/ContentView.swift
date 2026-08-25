import AppKit
import SwiftUI

struct ContentView: View {
    @ObservedObject var host: HostBridge

    /// Stable epoch — do **not** use `Date.now` here; parent re-renders would
    /// rebuild the schedule and fire frames far above 50 Hz.
    private static let frameTimeline = PeriodicTimelineSchedule(
        from: Date(timeIntervalSinceReferenceDate: 0),
        by: 1.0 / 50.0
    )

    var body: some View {
        TimelineView(Self.frameTimeline) { timeline in
            VStack(spacing: 0) {
                SpectrumDisplayView(host: host, tick: UInt64(timeline.date.timeIntervalSinceReferenceDate * 1000))
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .background(Color.black)
                    .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
                    .padding(.horizontal, 12)
                    .padding(.top, 8)
                    .padding(.bottom, 8)
                    // No SwiftUI `.focusable()` — that drew a blue ring without making us key.
                    .onTapGesture {
                        activateSpecChum()
                        FocusSpectrumView.post()
                    }

                statusFooter
            }
            .onChange(of: timeline.date) { _, _ in
                // HostBridge also wall-clock gates to ~50 Hz.
                host.runFrame()
            }
        }
        .background(WindowBackground())
        .background(WindowTitleBinder(title: host.windowTitle))
        .toolbar {
            ToolbarItemGroup(placement: .navigation) {
                Button {
                    chromeAction { host.presentOpenMediaPanel() }
                } label: {
                    Label(host.openMediaTitle, systemImage: "opticaldiscdrive")
                }
                .help(host.openMediaMenuTitle)
                .accessibilityLabel(host.openMediaTitle)

                Button {
                    chromeAction { host.instantLoadTape() }
                } label: {
                    Label("Instant", systemImage: "bolt.fill")
                }
                .disabled(!host.hasTape)
                .help("Flash-load: Type LOAD \"\" then Play (skip to program)")
                .accessibilityLabel("Instant load")
            }

            ToolbarItemGroup(placement: .primaryAction) {
                Button {
                    chromeAction {
                        if host.tapePlaying {
                            host.pauseTape()
                        } else {
                            host.playTape()
                        }
                    }
                } label: {
                    Label(
                        host.tapePlaying ? "Pause" : "Play",
                        systemImage: host.tapePlaying ? "pause.fill" : "play.fill"
                    )
                }
                .disabled(!host.hasTape && !host.tapePlaying)
                .help(host.tapePlaying ? "Pause tape" : "Play tape")
                .accessibilityLabel(host.tapePlaying ? "Pause tape" : "Play tape")

                Button {
                    chromeAction { host.rewindTape() }
                } label: {
                    Label("Rewind", systemImage: "backward.end.fill")
                }
                .disabled(!host.hasTape)
                .help("Rewind tape")
                .accessibilityLabel("Rewind tape")
            }

            ToolbarItem(placement: .automatic) {
                if let frac = host.tapeFraction {
                    VStack(alignment: .leading, spacing: 1) {
                        ProgressView(value: frac)
                            .controlSize(.small)
                            .frame(minWidth: 100, maxWidth: 160)
                        Text(host.tapeBlockLabel)
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                    .accessibilityElement(children: .combine)
                    .accessibilityLabel("Tape progress")
                    .accessibilityValue(host.tapeBlockLabel)
                }
            }

            ToolbarItemGroup(placement: .automatic) {
                Button {
                    chromeAction { host.reset() }
                } label: {
                    Label("Reset", systemImage: "arrow.counterclockwise")
                }
                .help("Reset machine")
                .accessibilityLabel("Reset machine")
            }
        }
        .onAppear {
            host.runFrame()
            activateSpecChum()
            FocusSpectrumView.post()
        }
        .sheet(isPresented: $host.showInspector) {
            DebugInspectorView(host: host)
        }
        .onChange(of: host.showInspector) { _, showing in
            if !showing {
                FocusSpectrumView.post()
            }
        }
    }

    /// Secondary status — caption style so it does not compete with the display.
    private var statusFooter: some View {
        Text(host.status)
            .font(.caption)
            .foregroundStyle(.secondary)
            .lineLimit(2)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 14)
            .padding(.vertical, 8)
            .glassBarBackground()
            .padding(.horizontal, 12)
            .padding(.bottom, 10)
            .accessibilityLabel("Status")
            .accessibilityValue(host.status)
    }

    /// After chrome clicks, return key focus to the Spectrum view.
    private func chromeAction(_ work: () -> Void) {
        work()
        FocusSpectrumView.post()
    }
}

struct DebugInspectorView: View {
    @ObservedObject var host: HostBridge

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("Debug Inspector")
                    .font(.headline)
                Spacer()
                Button("Done") {
                    host.showInspector = false
                }
                .keyboardShortcut(.cancelAction)
            }
            HStack(spacing: 16) {
                Text(String(format: "PC %04X", host.debugPc))
                Text(String(format: "SP %04X", host.debugSp))
                Text(String(format: "AF %04X", host.debugAf))
            }
            .font(.system(.body, design: .monospaced))
            HStack {
                Button(host.paused ? "Continue" : "Pause") {
                    host.setPaused(!host.paused)
                }
                Button("Step") {
                    if !host.paused {
                        host.setPaused(true)
                    }
                    host.step()
                }
            }
            ScrollView {
                Text(host.inspectJsonPreview)
                    .font(.system(.caption, design: .monospaced))
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .padding()
        .frame(minWidth: 480, minHeight: 320)
        .onAppear {
            host.refreshInspector()
        }
    }
}

/// Ask the embedded Spectrum `NSView` to take first responder (SwiftUI focus alone is not enough).
enum FocusSpectrumView {
    static let name = Notification.Name("SpecChumFocusSpectrumView")

    static func post() {
        NotificationCenter.default.post(name: name, object: nil)
    }
}

/// Keep the NSWindow title in sync with media + machine (document-style HIG).
private struct WindowTitleBinder: NSViewRepresentable {
    var title: String

    func makeNSView(context: Context) -> NSView {
        let view = NSView()
        DispatchQueue.main.async { apply(to: view) }
        return view
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        apply(to: nsView)
    }

    private func apply(to view: NSView) {
        guard let window = view.window else { return }
        if window.title != title {
            window.title = title
        }
    }
}

private struct WindowBackground: View {
    var body: some View {
        if #available(macOS 26, *) {
            Color.clear
                .glassEffect(.clear, in: .rect)
                .ignoresSafeArea()
        } else {
            LinearGradient(
                colors: [
                    Color(nsColor: .windowBackgroundColor),
                    Color(nsColor: .underPageBackgroundColor),
                ],
                startPoint: .topLeading,
                endPoint: .bottomTrailing
            )
            .ignoresSafeArea()
        }
    }
}
