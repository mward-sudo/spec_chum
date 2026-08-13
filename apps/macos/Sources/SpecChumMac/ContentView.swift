import AppKit
import SwiftUI
import UniformTypeIdentifiers

struct ContentView: View {
    @ObservedObject var host: HostBridge

    var body: some View {
        TimelineView(.periodic(from: .now, by: 1.0 / 50.0)) { timeline in
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
                host.runFrame()
            }
        }
        .background(WindowBackground())
        .onAppear {
            host.runFrame()
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
