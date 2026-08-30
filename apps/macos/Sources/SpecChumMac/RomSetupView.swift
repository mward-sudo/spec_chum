import AppKit
import SwiftUI
import UniformTypeIdentifiers
import CSpecChumHost

/// One required ROM slot from `sc_model_rom_setup_json`.
struct RomSetupSlot: Identifiable, Equatable {
    let id: String
    let label: String
    let installPath: String
    let alternatePaths: [String]
    let expectedBytes: Int
    let userProvided: Bool
    let status: String
    let resolvedPath: String?
    let hint: String

    var statusLabel: String {
        switch status {
        case "found": "Found"
        case "wrong_size": "Wrong size"
        default: "Missing"
        }
    }

    var statusColor: Color {
        switch status {
        case "found": .green
        case "wrong_size": .orange
        default: .red
        }
    }

    var sizeHint: String {
        let kb = expectedBytes / 1024
        return kb > 0 ? "\(kb) KiB" : "\(expectedBytes) bytes"
    }
}

/// Parsed `sc_model_rom_setup_json` payload.
struct RomSetupPayload: Equatable {
    let modelTitle: String
    let complete: Bool
    let fetchable: Bool
    let slots: [RomSetupSlot]
}

enum RomSetupCodec {
    private struct JsonSlot: Decodable {
        let id: String
        let label: String
        let install_path: String
        let alternate_paths: [String]
        let expected_bytes: Int
        let user_provided: Bool
        let status: String
        let resolved_path: String?
        let hint: String
    }

    private struct JsonRoot: Decodable {
        let model_title: String
        let complete: Bool
        let fetchable: Bool
        let slots: [JsonSlot]
    }

    static func decode(_ json: String) -> RomSetupPayload? {
        guard let data = json.data(using: .utf8),
              let root = try? JSONDecoder().decode(JsonRoot.self, from: data)
        else { return nil }
        return RomSetupPayload(
            modelTitle: root.model_title,
            complete: root.complete,
            fetchable: root.fetchable,
            slots: root.slots.map {
                RomSetupSlot(
                    id: $0.id,
                    label: $0.label,
                    installPath: $0.install_path,
                    alternatePaths: $0.alternate_paths,
                    expectedBytes: $0.expected_bytes,
                    userProvided: $0.user_provided,
                    status: $0.status,
                    resolvedPath: $0.resolved_path,
                    hint: $0.hint
                )
            }
        )
    }

    static func fetch(model: HostBridge.Model) -> RomSetupPayload? {
        guard let cstr = sc_model_rom_setup_json(model.rawValue) else { return nil }
        defer { sc_string_free(cstr) }
        return decode(String(cString: cstr))
    }
}

/// Dialog listing required ROM slots with per-slot file pickers (#188).
struct RomSetupView: View {
    @ObservedObject var host: HostBridge
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Required ROMs")
                        .font(.headline)
                    Text(host.romSetupPayload?.modelTitle ?? host.model.title)
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }
                Spacer()
                Button("Done") {
                    host.showRomSetup = false
                }
                .keyboardShortcut(.cancelAction)
            }

            if let payload = host.romSetupPayload {
                if payload.fetchable {
                    Text(
                        "System ROMs are not shipped with Spec Chum. Fetch official images with ./scripts/fetch_roms.sh, or choose files below — paths are remembered across restarts."
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                } else {
                    Text(
                        "This model needs user-provided ROM dumps (never fetched automatically). Choose each file below — paths are remembered across restarts."
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                }

                ForEach(payload.slots) { slot in
                    RomSetupSlotRow(host: host, slot: slot)
                }

                if payload.complete {
                    Label("All required ROMs are present.", systemImage: "checkmark.circle.fill")
                        .foregroundStyle(.green)
                        .font(.callout)
                } else if let err = host.romSetupError {
                    Text(err)
                        .font(.caption)
                        .foregroundStyle(.red)
                }
            } else {
                Text("Could not load ROM requirements.")
                    .foregroundStyle(.secondary)
            }

            HStack {
                Spacer()
                if host.romSetupPayload?.complete == true {
                    Button("Load machine") {
                        host.finishRomSetup(loadMachine: true)
                    }
                    .keyboardShortcut(.defaultAction)
                }
                Button("Close") {
                    host.showRomSetup = false
                }
            }
        }
        .padding()
        .frame(minWidth: 520, minHeight: 280)
        .onAppear {
            host.refreshRomSetup()
        }
    }
}

private struct RomSetupSlotRow: View {
    @ObservedObject var host: HostBridge
    let slot: RomSetupSlot

    var body: some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 8) {
                HStack(alignment: .firstTextBaseline) {
                    Text(slot.label)
                        .font(.body.bold())
                    Spacer()
                    Text(slot.statusLabel)
                        .font(.caption.bold())
                        .foregroundStyle(slot.statusColor)
                }
                Text("→ \(slot.installPath) (\(slot.sizeHint))")
                    .font(.system(.caption, design: .monospaced))
                    .foregroundStyle(.secondary)
                if let resolved = slot.resolvedPath {
                    Text(resolved)
                        .font(.caption2)
                        .foregroundStyle(slot.status == "found" ? Color.secondary : Color.orange)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
                Text(slot.hint)
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                Button("Choose file…") {
                    host.pickRomForSlot(slot.id)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}
