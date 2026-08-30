import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// New / edit custom machine profile (#187) — ROM override and hardware toggles.
/// Built-in models are select-only; this sheet is never used for them.
struct MachineConfigEditorView: View {
    @ObservedObject var host: HostBridge
    @Environment(\.dismiss) private var dismiss

    @State private var draft: UserMachineConfig
    @State private var errorText: String?
    private let isNew: Bool

    init(host: HostBridge, draft: UserMachineConfig, isNew: Bool) {
        self.host = host
        _draft = State(initialValue: draft)
        self.isNew = isNew
    }

    private var hardwareCompat: HardwareCompatFlags {
        HardwareCompatFlags.forBase(draft.base)
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                Text(
                    "Saved profile: pick a base model, optional main ROM override, and hardware to attach on load."
                )
                .font(.caption)
                .foregroundStyle(.secondary)

                TextField("Name", text: $draft.name)
                    .textFieldStyle(.roundedBorder)

                GroupBox("Base model") {
                    Picker("Base", selection: $draft.base) {
                        ForEach(PrefModelSlug.allCases, id: \.self) { slug in
                            Text(slug.title).tag(slug)
                        }
                    }
                    .pickerStyle(.radioGroup)
                    .labelsHidden()
                    .onChange(of: draft.base) { _, newBase in
                        draft = Self.sanitized(draft, for: newBase)
                    }
                }

                GroupBox("Main ROM (optional)") {
                    pathRow("Main ROM", path: $draft.customRomPath, placeholder: "(default for base model)")
                    Text("Leave empty to use the fetched default ROM for the base model.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }

                GroupBox("Input") {
                    Picker("Joystick", selection: $draft.joystickMode) {
                        Text("Kempston").tag(PrefJoystickSlug.kempston)
                        Text("Sinclair left").tag(PrefJoystickSlug.sinclairLeft)
                        Text("Sinclair right").tag(PrefJoystickSlug.sinclairRight)
                        Text("Cursor").tag(PrefJoystickSlug.cursor)
                    }
                    Toggle("Kempston mouse", isOn: $draft.kempstonMouse)
                    if hardwareCompat.ayStereo {
                        Picker("AY stereo", selection: $draft.ayStereo) {
                            Text("Mono").tag(PrefAyStereoSlug.mono)
                            Text("ACB").tag(PrefAyStereoSlug.acb)
                            Text("ABC").tag(PrefAyStereoSlug.abc)
                        }
                    }
                }

                GroupBox("Attach peripherals (saved with profile)") {
                    if hardwareCompat.multiface || hardwareCompat.divmmc || hardwareCompat.interface1
                        || hardwareCompat.beta
                    {
                        if hardwareCompat.multiface {
                            Toggle("Multiface 1", isOn: $draft.attachMultiface)
                                .onChange(of: draft.attachMultiface) { _, on in
                                    if !on { draft.multifaceRomPath = nil }
                                }
                            if draft.attachMultiface {
                                pathRow("Multiface ROM", path: $draft.multifaceRomPath)
                            }
                        }
                        if hardwareCompat.divmmc {
                            Toggle("DivMMC", isOn: $draft.attachDivmmc)
                                .onChange(of: draft.attachDivmmc) { _, on in
                                    if !on { draft.divmmcEepromPath = nil }
                                }
                            if draft.attachDivmmc {
                                pathRow("ESXDOS EEPROM", path: $draft.divmmcEepromPath)
                            }
                        }
                        if hardwareCompat.interface1 {
                            Toggle("Interface 1 (stub)", isOn: $draft.attachInterface1)
                                .onChange(of: draft.attachInterface1) { _, on in
                                    if !on { draft.interface1RomPath = nil }
                                }
                            if draft.attachInterface1 {
                                pathRow("IF1 ROM", path: $draft.interface1RomPath)
                            }
                        }
                        if hardwareCompat.beta {
                            Toggle("Beta Disk", isOn: $draft.attachBeta)
                                .onChange(of: draft.attachBeta) { _, on in
                                    if !on { draft.trdosRomPath = nil }
                                }
                            if draft.attachBeta {
                                pathRow("TR-DOS ROM", path: $draft.trdosRomPath)
                            }
                        }
                    } else {
                        Text("No optional peripheral hardware on this base model.")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }

                if let errorText {
                    Text(errorText)
                        .font(.caption)
                        .foregroundStyle(.red)
                }

                HStack {
                    Spacer()
                    Button("Cancel") { dismiss() }
                    Button(isNew ? "Create" : "Save") {
                        var toSave = draft
                        let trimmed = toSave.name.trimmingCharacters(in: .whitespacesAndNewlines)
                        guard !trimmed.isEmpty else {
                            errorText = "Configuration name is required"
                            return
                        }
                        toSave.name = trimmed
                        toSave = Self.sanitized(toSave, for: toSave.base)
                        if host.saveCustomConfiguration(toSave, isNew: isNew) {
                            dismiss()
                        } else {
                            errorText = host.status
                        }
                    }
                    .keyboardShortcut(.defaultAction)
                }
            }
            .padding(20)
        }
        .frame(width: 480)
        .frame(minHeight: 520, maxHeight: 720)
    }

    @ViewBuilder
    private func pathRow(
        _ label: String,
        path: Binding<String?>,
        placeholder: String = "(none)"
    ) -> some View {
        HStack {
            Text(label)
                .frame(width: 110, alignment: .leading)
            Text(path.wrappedValue ?? placeholder)
                .font(.caption)
                .lineLimit(1)
                .truncationMode(.middle)
            Button("Browse…") { browseRom(path: path) }
            if path.wrappedValue != nil {
                Button("Clear") { path.wrappedValue = nil }
            }
        }
    }

    private func browseRom(path: Binding<String?>) {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.allowedContentTypes = [
            UTType(filenameExtension: "rom") ?? .data,
            UTType(filenameExtension: "bin") ?? .data,
            UTType(filenameExtension: "eeprom") ?? .data,
        ]
        if panel.runModal() == .OK, let url = panel.url {
            path.wrappedValue = url.path
        }
    }

    private static func sanitized(_ draft: UserMachineConfig, for base: PrefModelSlug) -> UserMachineConfig {
        var draft = draft
        let compat = HardwareCompatFlags.forBase(base)
        if !compat.multiface {
            draft.attachMultiface = false
            draft.multifaceRomPath = nil
        }
        if !compat.divmmc {
            draft.attachDivmmc = false
            draft.divmmcEepromPath = nil
        }
        if !compat.interface1 {
            draft.attachInterface1 = false
            draft.interface1RomPath = nil
        }
        if !compat.beta {
            draft.attachBeta = false
            draft.trdosRomPath = nil
        }
        if !compat.ayStereo {
            draft.ayStereo = .mono
        }
        if !draft.attachMultiface {
            draft.multifaceRomPath = nil
        }
        if !draft.attachDivmmc {
            draft.divmmcEepromPath = nil
        }
        if !draft.attachInterface1 {
            draft.interface1RomPath = nil
        }
        if !draft.attachBeta {
            draft.trdosRomPath = nil
        }
        return draft
    }
}
