import SwiftUI

struct MediaLibraryEntry: Decodable, Identifiable {
    enum CodingKeys: String, CodingKey {
        case path, name, format, category, available, compatibility, title
        case titleSource = "title_source"
    }

    let path: String
    let name: String
    let format: String
    let category: String
    let available: Bool
    let compatibility: String
    let title: String?
    let titleSource: String?

    var id: String { path }
    var canOpen: Bool {
        compatibility == "ready"
            || compatibility == "may_require_machine"
            || compatibility == "may_select_machine"
    }

    var symbolName: String {
        switch category {
        case "tape": "opticaldiscdrive"
        case "snapshot": "doc"
        case "recording": "record.circle"
        case "disk": "externaldrive"
        default: "doc"
        }
    }

    var compatibilityDescription: String {
        switch compatibility {
        case "ready": "Compatible with the current machine"
        case "may_require_machine": "May need a machine"
        case "may_select_machine": "Snapshot selects model; ROM may be required"
        case "requires_machine": "Select a machine before opening"
        case "requires_plus3": "Requires a +3 or +3e"
        case "requires_beta_model": "Requires a Beta-compatible model"
        case "requires_beta": "Requires Beta Disk hardware"
        case "requires_trdos_rom": "Requires a TR-DOS ROM"
        case "unsupported_on_next": "Not supported on Spectrum Next"
        default: "Compatibility unknown"
        }
    }

    var formatDescription: String { format.uppercased() }
}

private enum LibraryCategory: String, CaseIterable, Identifiable {
    case all, tape, snapshot, recording, disk

    var id: String { rawValue }
    var title: String { rawValue.capitalized }
}

struct MediaLibraryView: View {
    @ObservedObject var host: HostBridge
    @Environment(\.dismiss) private var dismiss
    @State private var search = ""
    @State private var category: LibraryCategory = .all
    @State private var selectedPath: String?

    private var entries: [MediaLibraryEntry] {
        host.mediaLibraryEntries(search: search, category: category.rawValue)
    }

    private var selectedEntry: MediaLibraryEntry? {
        entries.first { $0.path == selectedPath } ?? entries.first
    }

    var body: some View {
        NavigationSplitView {
            sidebar
        } detail: {
            detail
        }
        .navigationTitle("Library")
        .toolbar {
            ToolbarItem(placement: .confirmationAction) {
                Button("Done") { dismiss() }
            }
        }
    }

    private var sidebar: some View {
        List(selection: $selectedPath) {
            ForEach(entries) { entry in
                MediaLibraryRow(entry: entry).tag(entry.path)
            }
        }
        .overlay {
            if entries.isEmpty {
                ContentUnavailableView(
                    search.isEmpty ? "No Recent Media" : "No Matches",
                    systemImage: "opticaldisc",
                    description: Text(emptyDescription)
                )
            }
        }
        .searchable(text: $search, prompt: "Name or path")
        .toolbar {
            ToolbarItem(placement: .principal) {
                Picker("Type", selection: $category) {
                    ForEach(LibraryCategory.allCases) { item in
                        Text(item.title).tag(item)
                    }
                }
                .pickerStyle(.segmented)
                .frame(maxWidth: 340)
            }
        }
    }

    private var detail: some View {
        Group {
            if let selectedEntry {
                MediaLibraryDetails(entry: selectedEntry, host: host)
            } else {
                ContentUnavailableView("Select Media", systemImage: "opticaldisc")
            }
        }
    }

    private var emptyDescription: String {
        search.isEmpty
            ? "Open supported media to add it to your Library."
            : "Try a different name or type."
    }
}

private struct MediaLibraryRow: View {
    let entry: MediaLibraryEntry

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Image(systemName: entry.symbolName)
                    .foregroundStyle(.tint)
                Text(entry.title ?? entry.name).lineLimit(1)
                Spacer(minLength: 8)
                Text(entry.formatDescription)
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
            }
            Label(
                entry.available ? entry.compatibilityDescription : "File is unavailable",
                systemImage: entry.available ? "checkmark.circle" : "exclamationmark.triangle"
            )
            .font(.caption)
            .foregroundStyle(entry.available ? Color.secondary : Color.orange)
            .lineLimit(1)
        }
        .padding(.vertical, 3)
    }
}

private struct MediaLibraryDetails: View {
    let entry: MediaLibraryEntry
    @ObservedObject var host: HostBridge
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(entry.name)
                .font(.title2.weight(.semibold))
                .textSelection(.enabled)
            if let title = entry.title {
                LabeledContent("Known title", value: title)
                if let titleSource = entry.titleSource {
                    LabeledContent("Title source", value: titleSource)
                }
            }
            LabeledContent("Format", value: entry.formatDescription)
            LabeledContent("Type", value: LibraryCategory(rawValue: entry.category)?.title ?? entry.category)
            LabeledContent("Availability", value: entry.available ? "Available" : "Unavailable")
            LabeledContent("Compatibility", value: entry.compatibilityDescription)
            VStack(alignment: .leading, spacing: 5) {
                Text("Original path").font(.caption).foregroundStyle(.secondary)
                Text(entry.path).font(.callout.monospaced()).textSelection(.enabled)
            }
            Spacer()
            HStack {
                Button("Remove from Library", role: .destructive) {
                    host.removeRecentFile(URL(fileURLWithPath: entry.path))
                }
                Spacer()
                Button("Open") {
                    if host.openLibraryEntry(entry) { dismiss() }
                }
                .keyboardShortcut(.defaultAction)
                .disabled(!entry.available || !entry.canOpen)
            }
        }
        .padding(24)
    }
}
