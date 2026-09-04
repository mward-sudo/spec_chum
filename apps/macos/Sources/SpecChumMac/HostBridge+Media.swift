import AppKit
import Foundation
import GameController
import IOSurface
import UniformTypeIdentifiers
import CSpecChumHost

extension HostBridge {
    func noteRecentFile(_ url: URL) {
        var next = recentFiles.filter { $0.standardizedFileURL != url.standardizedFileURL }
        next.insert(url, at: 0)
        if next.count > Self.maxRecentFiles {
            next = Array(next.prefix(Self.maxRecentFiles))
        }
        recentFiles = next
        persistRecentFiles()
    }

    /// Reopen a recent path; missing files are dropped from the list without crashing.
    func openRecentFile(_ url: URL) {
        guard FileManager.default.isReadableFile(atPath: url.path) else {
            status = "Recent file missing: \(url.lastPathComponent)"
            recentFiles.removeAll { $0.standardizedFileURL == url.standardizedFileURL }
            persistRecentFiles()
            return
        }
        switch url.pathExtension.lowercased() {
        case "sna", "z80":
            openSnapshot(at: url)
        case "rzx":
            openRzx(at: url)
        case "trd":
            openTrd(at: url)
        default:
            openMedia(at: url)
        }
    }

    func loadRom(at url: URL) {
        guard let handle else { return }
        let ok = url.path.withCString { sc_load_rom(handle, $0) }
        if ok != 0 {
            status = HostBridge.takeLastError() ?? "ROM load failed"
        } else {
            mediaTitle = url.lastPathComponent
            // Custom ROM into a running machine is not a hot-swap — reset like real hardware.
            if sc_reset(handle) != 0 {
                status = HostBridge.takeLastError() ?? "ROM loaded but reset failed"
            }
            pushTapeLoadOptions()
            refreshStatus()
        }
    }

    func openSnapshot(at url: URL) {
        guard let handle else { return }
        let ok = url.path.withCString { sc_load_snapshot(handle, $0) }
        if ok != 0 {
            status = HostBridge.takeLastError() ?? "Snapshot load failed"
        } else {
            mediaTitle = url.lastPathComponent
            syncModelFromHost()
            pushTapeLoadOptions()
            refreshStatus()
            noteRecentFile(url)
        }
    }

    func openRzx(at url: URL) {
        guard let handle else { return }
        let ok = url.path.withCString { sc_load_rzx(handle, $0) }
        if ok != 0 {
            status = HostBridge.takeLastError() ?? "RZX load failed"
        } else {
            mediaTitle = url.lastPathComponent
            refreshStatus()
            noteRecentFile(url)
        }
    }

    func openDsk(at url: URL) {
        guard let handle else { return }
        let ok = url.path.withCString { sc_load_dsk(handle, $0) }
        if ok != 0 {
            status = HostBridge.takeLastError() ?? "DSK load failed"
        } else {
            mediaTitle = url.lastPathComponent
            // Prefer a clear +3DOS hint over the raw host status string.
            status = "DSK inserted — use +3 Loader / +3DOS"
            noteRecentFile(url)
        }
    }

    func openTrd(at url: URL) {
        guard let handle else { return }
        let ok = url.path.withCString { sc_load_trd(handle, $0) }
        if ok != 0 {
            status = HostBridge.takeLastError() ?? "TRD load failed"
        } else {
            mediaTitle = url.lastPathComponent
            refreshStatus()
            noteRecentFile(url)
        }
    }

    func loadTrdosRom(at url: URL) {
        guard let handle else { return }
        let ok = url.path.withCString { sc_load_trdos_rom(handle, $0) }
        if ok != 0 {
            status = HostBridge.takeLastError() ?? "TR-DOS ROM load failed"
        } else {
            refreshStatus()
        }
    }

    func attachBeta() {
        guard let handle else { return }
        if sc_attach_beta(handle) != 0 {
            status = HostBridge.takeLastError() ?? "Beta Disk attach failed"
        } else {
            refreshStatus()
        }
    }

    var hasBeta: Bool {
        guard let handle else { return false }
        return sc_has_beta(handle) != 0
    }

    /// Queue egui-parity `LOAD ""` [CODE] via `sc_set_key`.
    /// 128K/+3: menu → 48 BASIC (+3 disk Loader — do not Enter alone).

    func presentOpenMediaPanel() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        var types: [UTType] = [
            UTType(filenameExtension: "tap") ?? .data,
            UTType(filenameExtension: "tzx") ?? .data,
        ]
        if model.supportsDisk {
            types.append(UTType(filenameExtension: "dsk") ?? .data)
            panel.title = "Open TAP / TZX / DSK"
        } else if model.supportsBeta {
            types.append(UTType(filenameExtension: "trd") ?? .data)
            panel.title = "Open TAP / TZX / TRD"
        } else {
            panel.title = "Open TAP / TZX"
        }
        panel.allowedContentTypes = types
        if panel.runModal() == .OK, let url = panel.url {
            openMedia(at: url)
        }
    }

    /// Route by extension: `.dsk` → +3 disk, `.trd` → Beta, else tape.
    func openMedia(at url: URL) {
        switch url.pathExtension.lowercased() {
        case "dsk":
            openDsk(at: url)
        case "trd":
            openTrd(at: url)
        default:
            openTape(at: url)
        }
    }

    func attachMultiface(at url: URL) {
        guard let handle else { return }
        let ok = url.path.withCString { sc_attach_multiface(handle, $0) }
        if ok != 0 {
            status = HostBridge.takeLastError() ?? "Multiface attach failed"
        } else {
            refreshStatus()
        }
    }

    func multifaceNmi() {
        guard let handle else { return }
        if sc_multiface_nmi(handle) != 0 {
            status = HostBridge.takeLastError() ?? "Multiface NMI failed"
        } else {
            refreshStatus()
        }
    }

    func attachInterface1() {
        guard let handle else { return }
        if sc_attach_interface1(handle) != 0 {
            status = HostBridge.takeLastError() ?? "Interface 1 attach failed"
        } else {
            refreshStatus()
        }
    }

    func loadInterface1Rom(at url: URL) {
        guard let handle else { return }
        let ok = url.path.withCString { sc_load_interface1_rom(handle, $0) }
        if ok != 0 {
            status = HostBridge.takeLastError() ?? "IF1 ROM load failed"
        } else {
            refreshStatus()
        }
    }

    func insertMdr(at url: URL) {
        guard let handle else { return }
        let ok = url.path.withCString { sc_insert_mdr(handle, $0) }
        if ok != 0 {
            status = HostBridge.takeLastError() ?? "MDR insert failed"
        } else {
            refreshStatus()
        }
    }

    func insertDck(at url: URL) {
        guard let handle else { return }
        let ok = url.path.withCString { sc_insert_dck(handle, $0) }
        if ok != 0 {
            status = HostBridge.takeLastError() ?? "DCK insert failed"
        } else {
            mediaTitle = url.lastPathComponent
            refreshStatus()
        }
    }

    func ejectDck() {
        guard let handle else { return }
        if sc_eject_dck(handle) != 0 {
            status = HostBridge.takeLastError() ?? "DCK eject failed"
        } else {
            refreshStatus()
        }
    }

    var hasTimexDock: Bool {
        guard let handle else { return false }
        return sc_has_timex_dock(handle) != 0
    }

    func attachDivmmc() {
        guard let handle else { return }
        if sc_attach_divmmc(handle) != 0 {
            status = HostBridge.takeLastError() ?? "DivMMC attach failed"
        } else {
            refreshStatus()
        }
    }

    func loadDivmmcSd(at url: URL) {
        guard let handle else { return }
        let ok = url.path.withCString { sc_load_divmmc_sd(handle, $0) }
        if ok != 0 {
            status = HostBridge.takeLastError() ?? "DivMMC SD load failed"
        } else {
            refreshStatus()
        }
    }

    func loadDivmmcEeprom(at url: URL) {
        guard let handle else { return }
        let ok = url.path.withCString { sc_load_divmmc_eeprom(handle, $0) }
        if ok != 0 {
            status = HostBridge.takeLastError() ?? "DivMMC EEPROM load failed"
        } else {
            refreshStatus()
        }
    }

}
