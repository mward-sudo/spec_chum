import AppKit
import SwiftUI

struct MachinePickerView: View {
    @ObservedObject var host: HostBridge
    @Environment(\.dismiss) private var dismiss

    private let columns = [GridItem(.adaptive(minimum: 330), spacing: 14)]

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Choose a machine")
                .font(.title2.weight(.semibold))
            Text("Select a built-in model. Saved configurations remain available from the Machine menu.")
                .foregroundStyle(.secondary)
            ScrollView {
                LazyVGrid(columns: columns, spacing: 14) {
                    ForEach(HostBridge.Model.pickerOrder) { model in
                        machineCard(model)
                    }
                }
            }
            HStack {
                Spacer()
                Button("Close") { dismiss() }
                    .keyboardShortcut(.cancelAction)
            }
        }
        .padding(20)
        .accessibilityIdentifier("machine-picker")
    }

    private func machineCard(_ model: HostBridge.Model) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Group {
                if let image = HostBridge.Model.catalogImage(for: model) {
                    Image(nsImage: image)
                        .resizable()
                        .scaledToFit()
                        .accessibilityLabel(model.imageDescription ?? "Hardware image for \(model.title)")
                } else {
                    Label("Image unavailable", systemImage: "desktopcomputer")
                        .frame(maxWidth: .infinity, minHeight: 110)
                        .foregroundStyle(.secondary)
                        .accessibilityLabel("Hardware image unavailable for \(model.title)")
                }
            }
            .frame(height: 132)

            Text(model.title).font(.headline)
            Text(model.pickerSummary)
                .font(.subheadline)
                .foregroundStyle(.secondary)
            Text("Compatible peripherals: \(compatiblePeripherals(model))")
                .font(.caption)
                .foregroundStyle(.secondary)
            Label(
                model.romAvailable ? "ROMs ready" : "ROM setup required",
                systemImage: model.romAvailable ? "checkmark.circle" : "exclamationmark.circle"
            )
            .font(.caption)
            Button(host.activeConfigId == nil && host.model == model ? "Selected" : "Select") {
                host.selectBuiltinModel(model)
                dismiss()
            }
            .disabled(host.activeConfigId == nil && host.model == model)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(12)
        .background(.quaternary, in: RoundedRectangle(cornerRadius: 10))
        .accessibilityElement(children: .contain)
    }

    private func compatiblePeripherals(_ model: HostBridge.Model) -> String {
        let compat = model.hardwareCompat
        var names: [String] = []
        if compat.multiface { names.append("Multiface") }
        if compat.divmmc { names.append("DivMMC") }
        if compat.interface1 { names.append("Interface 1") }
        if compat.beta { names.append("Beta Disk") }
        if compat.timexDock { names.append("Timex Dock") }
        return names.isEmpty ? "none listed" : names.joined(separator: " · ")
    }
}
