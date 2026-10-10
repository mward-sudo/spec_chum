import AppKit
import SwiftUI

/// Catalog-backed machine row shared by the toolbar and Machine menu.
struct MachineChoiceLabel: View {
    let model: HostBridge.Model
    var selected = false

    var body: some View {
        HStack(spacing: 8) {
            if let photo = HostBridge.Model.catalogImage(for: model) {
                Image(nsImage: photo)
                    .resizable()
                    .scaledToFit()
                    .frame(width: 54, height: 30)
                    .accessibilityHidden(true)
            } else {
                ZStack {
                    RoundedRectangle(cornerRadius: 4)
                        .fill(.quaternary)
                    Image(systemName: "desktopcomputer")
                        .foregroundStyle(.secondary)
                }
                .frame(width: 54, height: 30)
                .accessibilityLabel("Hardware photo unavailable for \(model.title)")
            }

            VStack(alignment: .leading, spacing: 2) {
                Text(model.title)
                Text(model.pickerSummary)
                    .font(.caption2)
                    .foregroundStyle(.secondary)
            }
            if !model.romAvailable {
                Image(systemName: "exclamationmark.circle")
                    .foregroundStyle(.secondary)
                    .accessibilityLabel("ROM setup required")
            }
            if selected {
                Image(systemName: "checkmark")
                    .accessibilityLabel("Selected")
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityValue(model.romAvailable ? "ROMs ready" : "ROM setup required")
    }
}
