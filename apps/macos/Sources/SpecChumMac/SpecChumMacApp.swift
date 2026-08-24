import AppKit
import SwiftUI
import UniformTypeIdentifiers

@main
struct SpecChumMacApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
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

                Button("Open Snapshot…") {
                    openSnapshot()
                }
                .keyboardShortcut("o", modifiers: [.command, .option])

                Button("Open RZX…") {
                    openRzx()
                }

                Button("Open Disk…") {
                    openDsk()
                }

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
                Divider()
                Button("Type LOAD \"\"") {
                    host.typeLoadQuotes(withCode: false)
                }
                Button("Type LOAD \"\" CODE") {
                    host.typeLoadQuotes(withCode: true)
                }
            }

            CommandMenu("Machine") {
                Button("Reset") { host.reset() }
                    .keyboardShortcut("r", modifiers: .command)
                Divider()
                Button("Type LOAD \"\"") {
                    host.typeLoadQuotes(withCode: false)
                }
                Button("Type LOAD \"\" CODE") {
                    host.typeLoadQuotes(withCode: true)
                }
                Divider()
                ForEach(HostBridge.Model.allCases) { model in
                    Button(model.title) {
                        host.model = model
                    }
                }
            }

            CommandMenu("Debug") {
                Button(host.paused ? "Continue" : "Pause") {
                    host.setPaused(!host.paused)
                }
                .keyboardShortcut("p", modifiers: [.command, .option])
                Button("Step") {
                    if !host.paused {
                        host.setPaused(true)
                    }
                    host.step()
                }
                .keyboardShortcut("s", modifiers: [.command, .option])
                Button("Add breakpoint at PC") {
                    host.addBreakpointAtPc()
                }
                Button("Dump JSON to Desktop") {
                    host.dumpTraceJsonToDesktop()
                }
                Button("Show inspector") {
                    host.showInspector = true
                    host.refreshInspector()
                }
                Divider()
                Button("Enable default trace") {
                    host.enableDefaultTrace()
                }
                Button("Dump trace to Desktop…") {
                    host.dumpTraceToDesktop()
                }
                Button("Dump trace to file…") {
                    host.dumpTracePanel()
                }
                Button("Clear trace ring") {
                    host.clearTrace()
                }
                Divider()
                Button("Env: SPEC_CHUM_DEBUG=1 or SPEC_CHUM_TRACE=tape,cpu") {}
                    .disabled(true)
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
        panel.title = "Open TAP / TZX"
        if panel.runModal() == .OK, let url = panel.url {
            host.openTape(at: url)
        }
    }

    private func openSnapshot() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.allowedContentTypes = [
            UTType(filenameExtension: "sna") ?? .data,
            UTType(filenameExtension: "z80") ?? .data,
        ]
        panel.title = "Open SNA / Z80 Snapshot"
        if panel.runModal() == .OK, let url = panel.url {
            host.openSnapshot(at: url)
        }
    }

    private func openRzx() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.allowedContentTypes = [
            UTType(filenameExtension: "rzx") ?? .data,
        ]
        panel.title = "Open RZX"
        if panel.runModal() == .OK, let url = panel.url {
            host.openRzx(at: url)
        }
    }

    private func openDsk() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.allowedContentTypes = [
            UTType(filenameExtension: "dsk") ?? .data,
        ]
        panel.title = "Open Disk (DSK)"
        if panel.runModal() == .OK, let url = panel.url {
            host.openDsk(at: url)
        }
    }

    private func openRom() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.allowedContentTypes = [UTType(filenameExtension: "rom") ?? .data]
        panel.title = "Open ROM"
        if panel.runModal() == .OK, let url = panel.url {
            host.loadRom(at: url)
        }
    }
}
