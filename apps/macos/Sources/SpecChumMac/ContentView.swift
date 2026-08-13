import AppKit
import SwiftUI
import UniformTypeIdentifiers

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
                glassToolbar
                    .padding(.horizontal, 12)
                    .padding(.vertical, 10)

                SpectrumDisplayView(host: host, tick: UInt64(timeline.date.timeIntervalSinceReferenceDate * 1000))
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .background(Color.black)
                    .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
                    .padding(.horizontal, 12)
                    .padding(.bottom, 8)
                    // No SwiftUI `.focusable()` — that drew a blue ring without making us key.
                    .onTapGesture {
                        activateSpecChum()
                        FocusSpectrumView.post()
                    }

                Text(host.status)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 16)
                    .padding(.bottom, 12)
                    .glassBarBackground()
                    .padding(.horizontal, 12)
                    .padding(.bottom, 10)
            }
            .onChange(of: timeline.date) { _, _ in
                // HostBridge also wall-clock gates to ~50 Hz.
                host.runFrame()
            }
        }
        .background(WindowBackground())
        .onAppear {
            host.runFrame()
            activateSpecChum()
            FocusSpectrumView.post()
        }
    }

    private var glassToolbar: some View {
        HStack(spacing: 10) {
            GlassToolbarButton(title: "Open Tape", systemImage: "opticaldiscdrive") {
                openTapePanel()
            }
            GlassToolbarButton(
                title: host.tapePlaying ? "Pause" : "Play",
                systemImage: host.tapePlaying ? "pause.fill" : "play.fill"
            ) {
                if host.tapePlaying {
                    host.pauseTape()
                } else {
                    host.playTape()
                }
            }
            .disabled(!host.hasTape && !host.tapePlaying)

            GlassToolbarButton(title: "Rewind", systemImage: "backward.end.fill") {
                host.rewindTape()
            }
            .disabled(!host.hasTape)

            if let frac = host.tapeFraction {
                VStack(alignment: .leading, spacing: 2) {
                    ProgressView(value: frac)
                        .progressViewStyle(.linear)
                        .frame(minWidth: 120, maxWidth: 180)
                    Text(host.tapeBlockLabel)
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
                .accessibilityLabel("Tape progress")
            }

            Toggle("Instant", isOn: $host.instantLoad)
                .toggleStyle(.checkbox)
                .help("Flash-load TAP/standard TZX at LD-BYTES (near-instant)")

            Picker("Speed", selection: $host.tapeSpeed) {
                Text("1x").tag(UInt32(1))
                Text("2x").tag(UInt32(2))
                Text("5x").tag(UInt32(5))
                Text("10x").tag(UInt32(10))
                Text("20x").tag(UInt32(20))
            }
            .pickerStyle(.menu)
            .frame(maxWidth: 90)
            .help("EAR bitstream speed when Instant is off (also speeds TZX pulse tapes)")
            .disabled(host.instantLoad)

            Spacer()

            Picker("Model", selection: $host.model) {
                ForEach(HostBridge.Model.allCases) { m in
                    Text(m.title).tag(m)
                }
            }
            .pickerStyle(.menu)
            .frame(maxWidth: 180)

            GlassToolbarButton(title: "Reset", systemImage: "arrow.counterclockwise") {
                host.reset()
            }
        }
        .padding(8)
        .glassBarBackground()
    }

    private func openTapePanel() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.allowedContentTypes = [
            UTType(filenameExtension: "tap") ?? .data,
            UTType(filenameExtension: "tzx") ?? .data,
        ]
        panel.title = "Open TAP / TZX"
        if panel.runModal() == .OK, let url = panel.url {
            host.openTape(at: url)
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
