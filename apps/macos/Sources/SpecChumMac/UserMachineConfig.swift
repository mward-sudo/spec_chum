import Foundation

/// JSON shape matches `host_api::UserMachineConfig` / `UiPreferences` (#187).
struct UserMachineConfig: Codable, Identifiable, Equatable {
    var id: String
    var name: String
    var base: PrefModelSlug
    var customRomPath: String?
    var joystickMode: PrefJoystickSlug
    var kempstonMouse: Bool
    var ayStereo: PrefAyStereoSlug
    var attachMultiface: Bool
    var multifaceRomPath: String?
    var attachDivmmc: Bool
    var divmmcEepromPath: String?
    var attachInterface1: Bool
    var interface1RomPath: String?
    var attachBeta: Bool
    var trdosRomPath: String?

    enum CodingKeys: String, CodingKey {
        case id, name, base
        case customRomPath = "custom_rom_path"
        case joystickMode = "joystick_mode"
        case kempstonMouse = "kempston_mouse"
        case ayStereo = "ay_stereo"
        case attachMultiface = "attach_multiface"
        case multifaceRomPath = "multiface_rom_path"
        case attachDivmmc = "attach_divmmc"
        case divmmcEepromPath = "divmmc_eeprom_path"
        case attachInterface1 = "attach_interface1"
        case interface1RomPath = "interface1_rom_path"
        case attachBeta = "attach_beta"
        case trdosRomPath = "trdos_rom_path"
    }

    static func newNamed(_ name: String, base: HostBridge.Model) -> UserMachineConfig {
        UserMachineConfig(
            id: "cfg-\(UInt64(Date().timeIntervalSince1970 * 1_000_000_000))",
            name: name,
            base: PrefModelSlug.from(base),
            customRomPath: nil,
            joystickMode: .kempston,
            kempstonMouse: false,
            ayStereo: .mono,
            attachMultiface: false,
            multifaceRomPath: nil,
            attachDivmmc: false,
            divmmcEepromPath: nil,
            attachInterface1: false,
            interface1RomPath: nil,
            attachBeta: false,
            trdosRomPath: nil
        )
    }
}

enum PrefModelSlug: String, Codable, CaseIterable {
    case spectrum16K = "spectrum_16k"
    case spectrum48 = "spectrum_48"
    case spectrum128 = "spectrum_128"
    case spectrumPlus2 = "spectrum_plus2"
    case spectrumPlus2A = "spectrum_plus2a"
    case spectrumPlus3 = "spectrum_plus3"
    case pentagon128 = "pentagon128"

    /// Canonical UI order (matches `machine::ALL_MODELS` / `HostBridge.Model.pickerOrder`).
    static let pickerOrder: [PrefModelSlug] = HostBridge.Model.pickerOrder.map { from($0) }

    var hostModel: HostBridge.Model {
        switch self {
        case .spectrum48: .spectrum48
        case .spectrum16K: .spectrum16K
        case .spectrum128: .spectrum128
        case .spectrumPlus2: .spectrumPlus2
        case .spectrumPlus2A: .spectrumPlus2A
        case .spectrumPlus3: .spectrumPlus3
        case .pentagon128: .pentagon128
        }
    }

    var title: String { hostModel.title }

    static func from(_ model: HostBridge.Model) -> PrefModelSlug {
        switch model {
        case .spectrum48: .spectrum48
        case .spectrum16K: .spectrum16K
        case .spectrum128: .spectrum128
        case .spectrumPlus2: .spectrumPlus2
        case .spectrumPlus2A: .spectrumPlus2A
        case .spectrumPlus3: .spectrumPlus3
        case .pentagon128: .pentagon128
        }
    }
}

enum PrefJoystickSlug: String, Codable, CaseIterable {
    case kempston
    case sinclairLeft = "sinclair_left"
    case sinclairRight = "sinclair_right"
    case cursor

    var hostMode: HostBridge.JoystickMode {
        switch self {
        case .kempston: .kempston
        case .sinclairLeft: .sinclairLeft
        case .sinclairRight: .sinclairRight
        case .cursor: .cursor
        }
    }

    static func from(_ mode: HostBridge.JoystickMode) -> PrefJoystickSlug {
        switch mode {
        case .kempston: .kempston
        case .sinclairLeft: .sinclairLeft
        case .sinclairRight: .sinclairRight
        case .cursor: .cursor
        }
    }
}

enum PrefAyStereoSlug: String, Codable, CaseIterable {
    case mono
    case acb
    case abc
}

struct HardwareCompatFlags {
    let multiface: Bool
    let divmmc: Bool
    let interface1: Bool
    let beta: Bool
    let ayStereo: Bool

    static func forBase(_ base: PrefModelSlug) -> HardwareCompatFlags {
        switch base.hostModel {
        case .spectrum16K, .spectrum48:
            return HardwareCompatFlags(
                multiface: true, divmmc: true, interface1: true, beta: true, ayStereo: false
            )
        case .spectrum128, .spectrumPlus2, .pentagon128:
            return HardwareCompatFlags(
                multiface: false, divmmc: true, interface1: true, beta: true, ayStereo: true
            )
        case .spectrumPlus2A, .spectrumPlus3:
            return HardwareCompatFlags(
                multiface: false, divmmc: false, interface1: false, beta: false, ayStereo: true
            )
        }
    }
}
