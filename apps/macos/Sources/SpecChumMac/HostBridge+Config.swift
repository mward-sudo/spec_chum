import AppKit
import Foundation
import GameController
import IOSurface
import UniformTypeIdentifiers
import CSpecChumHost

extension HostBridge {
    static func loadPersistedVolume() -> Float {
        let defaults = UserDefaults.standard
        if defaults.object(forKey: volumeDefaultsKey) == nil {
            return 1.0
        }
        return max(0, min(1, Float(defaults.double(forKey: volumeDefaultsKey))))
    }

    static func loadPersistedMuted() -> Bool {
        UserDefaults.standard.bool(forKey: mutedDefaultsKey)
    }

    static func loadPersistedModel() -> Model {
        let defaults = UserDefaults.standard
        if let id = defaults.string(forKey: activeConfigIdKey),
           let data = defaults.data(forKey: customConfigsKey),
           let configs = try? JSONDecoder().decode([UserMachineConfig].self, from: data),
           configs.contains(where: { $0.id == id })
        {
            // Model field reflects base while custom profile is active; pick last built-in fallback.
            let raw = UInt32(defaults.integer(forKey: lastBuiltinModelKey))
            if raw == 0, defaults.object(forKey: lastBuiltinModelKey) == nil {
                return .spectrum48
            }
            return Model(rawValue: raw) ?? .spectrum48
        }
        let raw = UInt32(defaults.integer(forKey: modelDefaultsKey))
        return Model(rawValue: raw) ?? .spectrum48
    }

    static func loadPersistedCustomConfigs() -> [UserMachineConfig] {
        guard let data = UserDefaults.standard.data(forKey: customConfigsKey),
              let decoded = try? JSONDecoder().decode([UserMachineConfig].self, from: data)
        else {
            return []
        }
        return Array(decoded.prefix(maxCustomConfigs))
    }

    func persistCustomConfigs() {
        guard let data = try? JSONEncoder().encode(customConfigs) else { return }
        UserDefaults.standard.set(data, forKey: Self.customConfigsKey)
    }

    func persistActiveConfigId() {
        if let activeConfigId {
            UserDefaults.standard.set(activeConfigId, forKey: Self.activeConfigIdKey)
        } else {
            UserDefaults.standard.removeObject(forKey: Self.activeConfigIdKey)
        }
    }

    /// Select a built-in model (clears active custom profile).
    func selectBuiltinModel(_ pick: Model) {
        activeConfigId = nil
        persistActiveConfigId()
        romSetupModel = pick
        if model != pick {
            model = pick
        } else {
            guard let handle else { return }
            _ = sc_set_model(handle, pick.rawValue)
            tryAutoloadRom()
            pushTapeLoadOptions()
            refreshStatus()
            refreshRomSetupQuiet()
            maybeAutoPresentRomSetup()
        }
        reclaimKeyboardFocus()
    }

    /// Return keyboard focus to the Spectrum / living-room NSView after chrome interaction.
    func reclaimKeyboardFocus() {
        if livingRoomMode {
            livingRoomPresentView?.claimFocus()
        } else {
            spectrumPresentView?.claimFocus()
        }
        FocusSpectrumView.postDelayed()
    }

    /// Open ROM setup manually or after a built-in model pick when files are missing.
    func presentRomSetup(auto: Bool = false) {
        if activeConfigId == nil {
            romSetupModel = model
        }
        scheduleRomSetupSheet(auto: auto, force: true)
    }

    func refreshRomSetup() {
        romSetupError = nil
        romSetupPayload = RomSetupCodec.fetch(model: romSetupModel)
        if romSetupPayload == nil {
            romSetupError = HostBridge.takeLastError() ?? "ROM setup unavailable"
        }
    }

    /// Present the ROM sheet on the next run loop (SwiftUI may miss `true` set during init).
    func scheduleRomSetupSheet(auto: Bool, force: Bool) {
        guard activeConfigId == nil else {
            showRomSetup = false
            return
        }
        syncModelRomPathsToHost()
        refreshRomSetup()
        let shouldShow = force || needsRomSetup
        guard shouldShow else {
            showRomSetup = false
            return
        }
        DispatchQueue.main.async { [weak self] in
            guard let self, self.activeConfigId == nil else { return }
            self.syncModelRomPathsToHost()
            self.refreshRomSetup()
            guard force || self.needsRomSetup else {
                self.showRomSetup = false
                return
            }
            self.showRomSetup = true
            if auto, let payload = self.romSetupPayload, !payload.complete {
                self.status = "ROMs required for \(payload.modelTitle)"
            }
        }
    }

    /// After a built-in model change: auto-open ROM sheet when paths are unset or files invalid.
    func maybeAutoPresentRomSetup() {
        scheduleRomSetupSheet(auto: true, force: false)
    }

    /// Update cached payload without opening the sheet (model changes / init).
    func refreshRomSetupQuiet() {
        guard activeConfigId == nil else {
            romSetupPayload = nil
            return
        }
        romSetupModel = model
        refreshRomSetup()
    }

    func pickRomForSlot(_ slotId: String) {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.allowedContentTypes = [
            UTType(filenameExtension: "rom") ?? .data,
            UTType(filenameExtension: "bin") ?? .data,
        ]
        panel.title = "Choose ROM file"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        installRomSlot(slotId: slotId, from: url)
    }

    func installRomSlot(slotId: String, from url: URL) {
        romSetupError = nil
        let ok = url.path.withCString { source in
            slotId.withCString { slot in
                sc_install_model_rom(romSetupModel.rawValue, slot, source)
            }
        }
        if ok != 0 {
            romSetupError = HostBridge.takeLastError() ?? "ROM install failed"
            refreshRomSetup()
            return
        }
        pullModelRomPathsFromHost()
        refreshRomSetup()
        if romSetupModel == model, romSetupPayload?.complete == true {
            finishRomSetup(loadMachine: true)
        } else {
            status = "Installed \(url.lastPathComponent) → roms/"
        }
    }

    func finishRomSetup(loadMachine: Bool) {
        if loadMachine, romSetupModel == model {
            tryAutoloadRom()
            pushTapeLoadOptions()
            refreshStatus()
        }
        showRomSetup = false
        romSetupError = nil
        reclaimKeyboardFocus()
    }

    func selectCustomConfiguration(id: String) {
        guard let cfg = customConfigs.first(where: { $0.id == id }) else { return }
        showRomSetup = false
        if applyCustomConfiguration(cfg) {
            activeConfigId = id
            persistActiveConfigId()
        }
    }

    func beginNewConfiguration() {
        let base = activeConfigId == nil ? model : (customConfigs.first { $0.id == activeConfigId }?.base.hostModel ?? model)
        var draft = UserMachineConfig.newNamed("My Spectrum", base: base)
        draft.joystickMode = PrefJoystickSlug.from(joystickMode)
        draft.kempstonMouse = kempstonMouse
        machineConfigEditorDraft = draft
        machineConfigEditorIsNew = true
        showMachineConfigEditor = true
    }

    func beginEditConfiguration(id: String) {
        guard let cfg = customConfigs.first(where: { $0.id == id }) else { return }
        machineConfigEditorDraft = cfg
        machineConfigEditorIsNew = false
        showMachineConfigEditor = true
    }

    func beginEditActiveConfiguration() {
        guard let id = activeConfigId, isCustomConfigActive else { return }
        beginEditConfiguration(id: id)
    }

    func deleteConfiguration(id: String) {
        guard customConfigs.contains(where: { $0.id == id }) else { return }
        customConfigs.removeAll { $0.id == id }
        persistCustomConfigs()
        if activeConfigId == id {
            activeConfigId = nil
            persistActiveConfigId()
            let fallback = Model(
                rawValue: UInt32(UserDefaults.standard.integer(forKey: Self.lastBuiltinModelKey))
            ) ?? .spectrum48
            selectBuiltinModel(fallback)
        }
    }

    func deleteActiveConfiguration() {
        guard let id = activeConfigId, isCustomConfigActive else { return }
        deleteConfiguration(id: id)
    }

    @discardableResult
    func saveCustomConfiguration(_ config: UserMachineConfig, isNew: Bool) -> Bool {
        if isNew, customConfigs.count >= Self.maxCustomConfigs {
            status = "Cannot save more than \(Self.maxCustomConfigs) configurations"
            return false
        }
        guard applyCustomConfiguration(config) else { return false }
        if let idx = customConfigs.firstIndex(where: { $0.id == config.id }) {
            customConfigs[idx] = config
        } else {
            customConfigs.append(config)
        }
        persistCustomConfigs()
        activeConfigId = config.id
        persistActiveConfigId()
        return true
    }

    @discardableResult
    func applyCustomConfiguration(_ config: UserMachineConfig) -> Bool {
        guard let handle else { return false }
        guard let data = try? JSONEncoder().encode(config),
              let json = String(data: data, encoding: .utf8)
        else {
            status = "Failed to encode configuration"
            return false
        }
        let ok = json.withCString { sc_apply_user_config_json(handle, $0) }
        if ok != 0 {
            status = HostBridge.takeLastError() ?? "Configuration apply failed"
            return false
        }
        suppressActiveConfigClear = true
        suppressModelPush = true
        model = config.base.hostModel
        suppressModelPush = false
        suppressActiveConfigClear = false
        joystickMode = config.joystickMode.hostMode
        kempstonMouse = config.kempstonMouse
        pushTapeLoadOptions()
        refreshStatus()
        return true
    }

    static func loadPersistedTapeSpeed() -> UInt32 {
        let defaults = UserDefaults.standard
        if defaults.object(forKey: tapeSpeedDefaultsKey) == nil {
            return 1
        }
        let speed = UInt32(max(1, min(defaults.integer(forKey: tapeSpeedDefaultsKey), 64)))
        let offered: Set<UInt32> = [1, 2, 5, 10, 20]
        return offered.contains(speed) ? speed : 1
    }

    static func loadPersistedExperience() -> Bool {
        UserDefaults.standard.bool(forKey: experienceDefaultsKey)
    }

    static func loadPersistedJoystickMode() -> JoystickMode {
        let raw = UInt32(UserDefaults.standard.integer(forKey: joystickDefaultsKey))
        return JoystickMode(rawValue: raw) ?? .kempston
    }

    static func loadPersistedKempstonMouse() -> Bool {
        UserDefaults.standard.bool(forKey: kempstonMouseDefaultsKey)
    }

    static func loadPersistedRecentFiles() -> [URL] {
        let paths = UserDefaults.standard.stringArray(forKey: recentFilesDefaultsKey) ?? []
        return paths.prefix(maxRecentFiles).map { URL(fileURLWithPath: $0) }
    }

    static func loadPersistedModelRomPaths() -> [String: String] {
        guard let data = UserDefaults.standard.data(forKey: modelRomPathsKey),
              let decoded = try? JSONDecoder().decode([String: String].self, from: data)
        else {
            return [:]
        }
        return decoded.filter { !$0.value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
    }

    func persistModelRomPaths() {
        guard let data = try? JSONEncoder().encode(modelRomPaths) else { return }
        UserDefaults.standard.set(data, forKey: Self.modelRomPathsKey)
    }

    func syncModelRomPathsToHost() {
        guard let json = try? JSONEncoder().encode(modelRomPaths),
              let text = String(data: json, encoding: .utf8)
        else {
            return
        }
        _ = text.withCString { sc_sync_model_rom_paths_json($0) }
    }

    func pullModelRomPathsFromHost() {
        guard let cstr = sc_model_rom_paths_json() else { return }
        defer { sc_string_free(cstr) }
        let text = String(cString: cstr)
        guard let data = text.data(using: .utf8),
              let decoded = try? JSONDecoder().decode([String: String].self, from: data)
        else {
            return
        }
        modelRomPaths = decoded
        persistModelRomPaths()
    }

    func modelRomPath(model: Model, slot: String) -> String? {
        modelRomPaths["\(model.prefSlug)_\(slot)"]
    }

    func persistRecentFiles() {
        let paths = recentFiles.prefix(Self.maxRecentFiles).map(\.path)
        UserDefaults.standard.set(Array(paths), forKey: Self.recentFilesDefaultsKey)
    }

    func tryAutoloadRom() {
        syncModelRomPathsToHost()
        if let main = modelRomPath(model: model, slot: "main"),
           FileManager.default.isReadableFile(atPath: main)
        {
            loadRom(at: URL(fileURLWithPath: main))
            return
        }
        let candidates: [String] = {
            switch model {
            case .spectrum16K, .spectrum48:
                return ["roms/spec48.rom"]
            case .spectrum128:
                return ["roms/128/spec128uk.rom"]
            case .spectrumPlus2:
                return ["roms/plus2/plus2uk.rom"]
            case .spectrumPlus3:
                return ["roms/plus3/plus3.rom"]
            case .spectrumPlus2A:
                return ["roms/plus2a/plus2a.rom", "roms/plus3/plus3.rom"]
            case .pentagon128:
                return ["roms/pentagon/pentagon.rom", "roms/pentagon/128p.rom"]
            case .timexTC2048:
                return ["roms/timex/tc2048.rom"]
            case .timexTS2068:
                return ["roms/timex/tc2068-0.rom"]
            }
        }()
        for root in romSearchRoots {
            for rel in candidates {
                let url = root.appendingPathComponent(rel)
                if FileManager.default.isReadableFile(atPath: url.path) {
                    loadRom(at: url)
                    return
                }
            }
        }
        status = "Missing ROM — run ./scripts/fetch_roms.sh"
    }

}
