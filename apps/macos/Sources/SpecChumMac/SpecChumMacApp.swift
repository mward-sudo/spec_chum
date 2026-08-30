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

                Button("Open TRD…") {
                    openTrd()
                }
                .disabled(!host.model.supportsBeta)

                if !host.recentFiles.isEmpty {
                    Divider()
                    Menu("Open Recent") {
                        ForEach(Array(host.recentFiles.enumerated()), id: \.offset) { _, url in
                            Button(url.lastPathComponent) {
                                host.openRecentFile(url)
                            }
                        }
                    }
                }
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

            // Machine — built-in models, custom profiles, reset
            CommandMenu("Machine") {
                Menu("Built-in models") {
                    Text("Select only — default ROMs. Session hardware via Hardware menu.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    ForEach(HostBridge.Model.pickerOrder) { pick in
                        Button {
                            host.selectBuiltinModel(pick)
                        } label: {
                            if host.activeConfigId == nil && host.model == pick {
                                Text("✓ \(pick.title)")
                            } else {
                                Text(pick.title)
                            }
                        }
                        .disabled(!pick.romAvailable)
                    }
                }
                Divider()
                Menu("My configurations") {
                    Button("+ New configuration…") {
                        host.beginNewConfiguration()
                    }
                    if host.customConfigs.isEmpty {
                        Text("(none saved yet)")
                    }
                    ForEach(host.customConfigs) { cfg in
                        Button {
                            host.selectCustomConfiguration(id: cfg.id)
                        } label: {
                            if host.activeConfigId == cfg.id {
                                Text("✓ \(cfg.name)")
                            } else {
                                Text(cfg.name)
                            }
                        }
                    }
                    if !host.customConfigs.isEmpty {
                        Menu("Manage configuration…") {
                            ForEach(host.customConfigs) { cfg in
                                Button("Edit “\(cfg.name)”…") {
                                    host.beginEditConfiguration(id: cfg.id)
                                }
                                Button("Delete “\(cfg.name)”", role: .destructive) {
                                    host.deleteConfiguration(id: cfg.id)
                                }
                            }
                        }
                    }
                    if host.isCustomConfigActive {
                        Divider()
                        Button("Edit configuration…") {
                            host.beginEditActiveConfiguration()
                        }
                        Button("Delete configuration") {
                            host.deleteActiveConfiguration()
                        }
                    }
                }
                Divider()
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
                .disabled(!host.model.supportsBeta)
                Button("Load Interface 1 ROM…") {
                    openInterface1Rom()
                }
                .disabled(!host.model.supportsBeta)
                Button("Open Microdrive MDR…") {
                    openMdr()
                }
                .disabled(!host.model.supportsBeta)
                Divider()
                Button("Attach DivMMC") {
                    host.attachDivmmc()
                }
                .disabled(!host.model.supportsBeta)
                Button("Open DivMMC SD image…") {
                    openDivmmcSd()
                }
                .disabled(!host.model.supportsBeta)
                Button("Open DivMMC EEPROM (ESXDOS)…") {
                    openDivmmcEeprom()
                }
                .disabled(!host.model.supportsBeta)
                Divider()
                Button("Load TR-DOS ROM…") {
                    openTrdosRom()
                }
                .disabled(!host.model.supportsBeta)
                Button("Beta: use egui (stubs)") {}
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
                Button("ROM Copyright Notice") {
                    if let url = URL(string: "https://github.com/mward-sudo/spec_chum/blob/main/docs/ROMS.md") {
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

    private func openTrd() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.allowedContentTypes = [
            UTType(filenameExtension: "trd") ?? .data,
        ]
        panel.title = "Open TR-DOS disk (TRD)"
        if panel.runModal() == .OK, let url = panel.url {
            host.openTrd(at: url)
        }
    }

    private func openDivmmcSd() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.allowedContentTypes = [
            UTType(filenameExtension: "img") ?? .data,
            UTType(filenameExtension: "bin") ?? .data,
            UTType(filenameExtension: "mmc") ?? .data,
            UTType(filenameExtension: "sd") ?? .data,
        ]
        panel.title = "Open DivMMC SD image"
        if panel.runModal() == .OK, let url = panel.url {
            host.loadDivmmcSd(at: url)
        }
    }

    private func openDivmmcEeprom() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.allowedContentTypes = [
            UTType(filenameExtension: "rom") ?? .data,
            UTType(filenameExtension: "bin") ?? .data,
            UTType(filenameExtension: "eeprom") ?? .data,
        ]
        panel.title = "Open DivMMC EEPROM (ESXDOS, 8 KiB+)"
        if panel.runModal() == .OK, let url = panel.url {
            host.loadDivmmcEeprom(at: url)
        }
    }

    private func openTrdosRom() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.allowedContentTypes = [
            UTType(filenameExtension: "rom") ?? .data,
            UTType(filenameExtension: "bin") ?? .data,
        ]
        panel.title = "Load TR-DOS ROM (16 KiB)"
        if panel.runModal() == .OK, let url = panel.url {
            host.loadTrdosRom(at: url)
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
}
