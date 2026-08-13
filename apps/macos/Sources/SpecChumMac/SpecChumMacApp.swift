import AppKit
import SwiftUI
import UniformTypeIdentifiers

@main
struct SpecChumMacApp: App {
    @StateObject private var host = HostBridge()

    var body: some Scene {
        WindowGroup("Spec Chum") {
            ContentView(host: host)
                .frame(minWidth: 640, minHeight: 520)
        }
        .defaultSize(width: 780, height: 640)
        .commands {
            CommandGroup(replacing: .newItem) {}
            CommandGroup(after: .newItem) {
                Button("Open Tape…") {
                    openTape()
                }
                .keyboardShortcut("o", modifiers: .command)

                Button("Open ROM…") {
                    openRom()
                }
                .keyboardShortcut("o", modifiers: [.command, .shift])
            }

            CommandMenu("Tape") {
                Button("Play") { host.playTape() }
                    .keyboardShortcut("p", modifiers: [.command, .shift])
                Button("Pause") { host.pauseTape() }
                Button("Rewind") { host.rewindTape() }
            }

            CommandMenu("Machine") {
                Button("Reset") { host.reset() }
                    .keyboardShortcut("r", modifiers: .command)
                Divider()
                ForEach(HostBridge.Model.allCases) { model in
                    Button(model.title) {
                        host.model = model
                    }
                }
            }
        }
    }

    private func openTape() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.allowedContentTypes = [
            UTType(filenameExtension: "tap") ?? .data,
            UTType(filenameExtension: "tzx") ?? .data,
        ]
        if panel.runModal() == .OK, let url = panel.url {
            host.openTape(at: url)
        }
    }

    private func openRom() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.allowedContentTypes = [UTType(filenameExtension: "rom") ?? .data]
        if panel.runModal() == .OK, let url = panel.url {
            host.loadRom(at: url)
        }
    }
}
