import AppKit
import SwiftUI
import UniformTypeIdentifiers

@main
struct SpecChumMacApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @StateObject private var host = HostBridge()

    init() {
        // Before SwiftUI creates NSWindows — prevents the document tab strip ("Spec Chum" + "+").
        NSWindow.allowsAutomaticWindowTabbing = false
        UserDefaults.standard.register(defaults: ["NSWindowTabbingEnabled": false])
    }

    var body: some Scene {
        // Single main window (not WindowGroup — that invites macOS title-bar tabs).
        Window("Spec Chum", id: "main") {
            ContentView(host: host)
                .frame(minWidth: 640, minHeight: 520)
        }
        .defaultSize(width: 780, height: 640)
        // Toolbar band is the draggable titlebar region (unified over full-bleed content).
        .windowStyle(.hiddenTitleBar)
        .windowToolbarStyle(.unified)
        .commands {
            // File — Open… counterparts to toolbar (standard once); other media kinds here only
            CommandGroup(replacing: .newItem) {}
            CommandGroup(after: .newItem) {
                Button(host.openMediaMenuTitle) {
                    host.presentOpenMediaPanel()
                }
                .keyboardShortcut("o", modifiers: .command)

                Button("Open Snapshot…") {
                    openSnapshot()
                }
                .keyboardShortcut("o", modifiers: [.command, .option])

                Divider()

                Button("Open RZX…") {
                    openRzx()
                }

                Button("Open Disk…") {
                    openDsk()
                }
                .disabled(!host.model.supportsDisk)

                Button("Open ROM…") {
                    openRom()
                }
                .keyboardShortcut("o", modifiers: [.command, .shift])
            }

            // View — inspector (system View menu also keeps toolbar / full screen)
            CommandGroup(after: .toolbar) {
                Button("Show Inspector") {
                    host.showInspector = true
                    host.refreshInspector()
                }
                .keyboardShortcut("i", modifiers: [.command, .option])
            }

            // Tape — Type LOAD only; Play / Instant / Rewind / speed live on the toolbar
            CommandMenu("Tape") {
                Button("Type LOAD \"\"") {
                    host.typeLoadQuotes(withCode: false)
                }
                Button("Type LOAD \"\" CODE") {
                    host.typeLoadQuotes(withCode: true)
                }
            }

            // Machine — Reset (+ model in Settings)
            CommandMenu("Machine") {
                Button("Reset") { host.reset() }
                    .keyboardShortcut("r", modifiers: .command)
            }

            // Hardware — Multiface / IF1; Joystick mode lives in Settings
            CommandMenu("Hardware") {
                Button("Attach Multiface 1 ROM…") {
                    openMultifaceRom()
                }
                Button("Multiface NMI") {
                    host.multifaceNmi()
                }
                Divider()
                Button("Attach Interface 1") {
                    host.attachInterface1()
                }
                Button("Load Interface 1 ROM…") {
                    openInterface1Rom()
                }
                Button("Open Microdrive MDR…") {
                    openMdr()
                }
                Divider()
                Button("DivMMC / Beta: use egui (stubs)") {}
                    .disabled(true)
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
                Button("Add Breakpoint at PC") {
                    host.addBreakpointAtPc()
                }
                Button("Dump JSON to Desktop") {
                    host.dumpTraceJsonToDesktop()
                }
                Button("Show Inspector") {
                    host.showInspector = true
                    host.refreshInspector()
                }
                Divider()
                Button("Enable Default Trace") {
                    host.enableDefaultTrace()
                }
                Button("Dump Trace to Desktop…") {
                    host.dumpTraceToDesktop()
                }
                Button("Dump Trace to File…") {
                    host.dumpTracePanel()
                }
                Button("Clear Trace Ring") {
                    host.clearTrace()
                }
                Divider()
                Button("Env: SPEC_CHUM_DEBUG=1 or SPEC_CHUM_TRACE=tape,cpu") {}
                    .disabled(true)
            }

            CommandGroup(replacing: .help) {
                Button("Spec Chum Help") {
                    if let url = URL(string: "https://github.com/mward-sudo/spec_chum/blob/main/docs/MACOS_NATIVE.md") {
                        NSWorkspace.shared.open(url)
                    }
                }
            }
        }

        Settings {
            SpecChumSettingsView(host: host)
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

    private func openMultifaceRom() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.allowedContentTypes = [
            UTType(filenameExtension: "rom") ?? .data,
            UTType(filenameExtension: "bin") ?? .data,
        ]
        panel.title = "Attach Multiface 1 ROM (8 KiB, 48K)"
        if panel.runModal() == .OK, let url = panel.url {
            host.attachMultiface(at: url)
        }
    }

    private func openInterface1Rom() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.allowedContentTypes = [
            UTType(filenameExtension: "rom") ?? .data,
            UTType(filenameExtension: "bin") ?? .data,
        ]
        panel.title = "Load Interface 1 ROM (8 KiB)"
        if panel.runModal() == .OK, let url = panel.url {
            host.loadInterface1Rom(at: url)
        }
    }

    private func openMdr() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.allowedContentTypes = [
            UTType(filenameExtension: "mdr") ?? .data,
        ]
        panel.title = "Open Microdrive MDR"
        if panel.runModal() == .OK, let url = panel.url {
            host.insertMdr(at: url)
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
