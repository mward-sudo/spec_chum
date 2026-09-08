import AppKit
import SwiftUI

struct ContentView: View {
    @ObservedObject var host: HostBridge

    var body: some View {
        Group {
            if host.livingRoomMode {
                livingRoomChrome
            } else {
                flatSpectrumChrome
            }
        }
        .background {
            if host.livingRoomMode {
                Color.black.ignoresSafeArea()
            } else {
                WindowBackground()
            }
        }
        .background(WindowTitleBinder(title: host.windowTitle))
        .toolbar {
            livingRoomToolbar
        }
        .toolbarRole(.editor)
        // HIG: system toolbar material over content; room fills the window behind it.
        .toolbarBackground(.ultraThinMaterial, for: .windowToolbar)
        .toolbarBackground(.visible, for: .windowToolbar)
        .onAppear {
            activateSpecChum()
            host.ensureAudioOutput()
            FocusSpectrumView.postDelayed()
        }
        .sheet(isPresented: $host.showInspector) {
            DebugInspectorView(host: host)
        }
        .sheet(isPresented: $host.showMachineConfigEditor) {
            if let draft = host.machineConfigEditorDraft {
                MachineConfigEditorView(
                    host: host,
                    draft: draft,
                    isNew: host.machineConfigEditorIsNew
                )
            }
        }
        .onChange(of: host.showInspector) { _, showing in
            if !showing {
                FocusSpectrumView.post()
            }
        }
        .onChange(of: host.model) { _, _ in
            // Toolbar Machine menus often keep an NSControl as first responder after pick.
            FocusSpectrumView.postDelayed()
        }
        .onChange(of: host.showRomSetup) { _, showing in
            if !showing {
                FocusSpectrumView.postDelayed()
            }
        }
        .onChange(of: host.showMachineConfigEditor) { _, showing in
            if !showing {
                FocusSpectrumView.postDelayed()
            }
        }
        .onChange(of: host.livingRoomMode) { _, _ in
            FocusSpectrumView.postDelayed()
        }
    }

    /// Full-bleed 3D room. Toolbar + glass footer sit in chrome bands; CRT is framed
    /// in the clear centre (preset fill ≤ ~58%) so UI never covers the tube.
    @ViewBuilder
    private var livingRoomChrome: some View {
        ZStack(alignment: .bottom) {
            LivingRoomDisplayView(
                host: host
            )
            .ignoresSafeArea()
            .onTapGesture {
                activateSpecChum()
                FocusSpectrumView.post()
            }

            // Footer stays in the lower chrome band (below the CRT safe frame).
            statusFooter
                .padding(.horizontal, 20)
                .padding(.bottom, 10)
                .allowsHitTesting(true)
        }
    }

    /// Classic inset Spectrum display + docked status (unchanged product layout).
    @ViewBuilder
    private var flatSpectrumChrome: some View {
        VStack(spacing: 0) {
            SpectrumDisplayView(host: host)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(Color.black)
            .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
            .padding(.horizontal, 12)
            .padding(.top, 8)
            .padding(.bottom, 8)
            .onTapGesture {
                activateSpecChum()
                FocusSpectrumView.post()
            }

            statusFooter
                .padding(.horizontal, 12)
                .padding(.bottom, 10)
        }
    }

    @ToolbarContentBuilder
    private var livingRoomToolbar: some ToolbarContent {
        ToolbarItemGroup(placement: .navigation) {
            Button {
                chromeAction { host.presentOpenMediaPanel() }
            } label: {
                Label(host.openMediaTitle, systemImage: "opticaldiscdrive")
            }
            .help(host.openMediaMenuTitle)
            .accessibilityLabel(host.openMediaTitle)

            Button {
                chromeAction { host.instantLoadTape() }
            } label: {
                Label("Instant", systemImage: "bolt.fill")
            }
            .help("Always asks for a tape image, then flash-loads (Type LOAD \"\" + Play)")
            .accessibilityLabel("Instant load")
        }

        // Tape Play / Rewind / progress / load-mode live in the status footer (not toolbar).

        // Trailing cluster — separate placements so `.unified` does not cram/wrap one group.
        ToolbarItem(placement: .status) {
            machineModelMenu
        }

        // Separate ToolbarItem — conditional children inside ToolbarItemGroup often fail to
        // appear or refresh on macOS unified toolbars (#188 Pentagon ROMs affordance).
        if host.showRomsToolbarButton {
            ToolbarItem(placement: .status) {
                Button {
                    chromeAction { host.presentRomSetup() }
                } label: {
                    Label("ROMs", systemImage: "memorychip")
                }
                .help("Choose ROM files required for the current model")
                .accessibilityLabel("ROMs")
            }
        }

        ToolbarItem(placement: .status) {
            Toggle(isOn: $host.livingRoomMode) {
                Label("Living Room", systemImage: "sofa.fill")
            }
            .toggleStyle(.button)
            .help("Bevy 3D living-room CRT (experimental) vs flat Spectrum display")
            .accessibilityLabel("Living Room display")
        }

        ToolbarItem(placement: .status) {
            HStack(spacing: 6) {
                Button {
                    host.outputMuted.toggle()
                    FocusSpectrumView.post()
                } label: {
                    Image(
                        systemName: host.outputMuted || host.outputVolume <= 0.001
                            ? "speaker.slash.fill"
                            : "speaker.wave.2.fill"
                    )
                    .imageScale(.medium)
                    .frame(width: 28, height: 22)
                    .contentShape(Rectangle())
                }
                .help("Mute host audio output (does not change EAR / flash-load)")
                .accessibilityLabel(host.outputMuted ? "Unmute" : "Mute")

                Slider(value: $host.outputVolume, in: 0 ... 1)
                    .frame(width: 88)
                    .disabled(host.outputMuted)
                    .help("Host output volume (PCM gain only)")
                    .accessibilityLabel("Volume")
                    .onChange(of: host.outputVolume) { _, _ in
                        FocusSpectrumView.post()
                    }
            }
        }

        ToolbarItem(placement: .confirmationAction) {
            Button {
                chromeAction { host.reset() }
            } label: {
                Label("Reset", systemImage: "arrow.counterclockwise")
            }
            .help("Reset machine")
            .accessibilityLabel("Reset machine")
        }
    }

    /// Machine model Menu — hug content; padded label for readable inset inside glass.
    private var machineModelMenu: some View {
        Menu {
            Section("Built-in models") {
                Text("Select only — default ROMs. Session hardware via Hardware menu.")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                ForEach(HostBridge.Model.pickerOrder) { pick in
                    Button {
                        chromeAction { host.selectBuiltinModel(pick) }
                    } label: {
                        HStack {
                            Text(pick.title)
                            if !pick.romAvailable {
                                Image(systemName: "exclamationmark.circle")
                                    .foregroundStyle(.secondary)
                            }
                            if host.activeConfigId == nil && host.model == pick {
                                Image(systemName: "checkmark")
                            }
                        }
                    }
                }
            }
            Section("My configurations") {
                Button("+ New configuration…") {
                    chromeAction { host.beginNewConfiguration() }
                }
                if host.customConfigs.isEmpty {
                    Text("(none saved yet)")
                }
                ForEach(host.customConfigs) { cfg in
                    Button {
                        chromeAction { host.selectCustomConfiguration(id: cfg.id) }
                    } label: {
                        HStack {
                            Text(cfg.name)
                            if host.activeConfigId == cfg.id {
                                Image(systemName: "checkmark")
                            }
                        }
                    }
                }
                if !host.customConfigs.isEmpty {
                    Menu("Manage configuration…") {
                        ForEach(host.customConfigs) { cfg in
                            Button("Edit “\(cfg.name)”…") {
                                chromeAction { host.beginEditConfiguration(id: cfg.id) }
                            }
                            Button("Delete “\(cfg.name)”", role: .destructive) {
                                chromeAction { host.deleteConfiguration(id: cfg.id) }
                            }
                        }
                    }
                }
                if host.isCustomConfigActive {
                    Divider()
                    Button("Edit configuration…") {
                        chromeAction { host.beginEditActiveConfiguration() }
                    }
                    Button("Delete configuration", role: .destructive) {
                        chromeAction { host.deleteActiveConfiguration() }
                    }
                }
            }
        } label: {
            // No greedy min/maxWidth on the Menu — it expands into the proposal and the
            // Liquid Glass pill grows empty (flush-left short titles). Hug content instead;
            // truncate long custom names in `machineToolbarLabel` (#184 readable).
            // Em-space is part of the measured title (SwiftUI `.padding` is not wrapped by glass).
            HStack(spacing: 0) {
                Text(verbatim: "\u{2003}")
                    .accessibilityHidden(true)
                Text(host.machineToolbarLabel)
            }
        }
        .fixedSize(horizontal: true, vertical: false)
        .help("Built-ins: select only. Custom profiles: edit hardware & ROM.")
        .accessibilityLabel("Machine")
        .accessibilityValue(host.machineDisplayTitle)
    }

    /// Tape deck + identity in the footer chrome (replaces the old EAR status caption).
    /// Left: load affordance or truncated tape name. Centre/right: progress + deck controls when inserted.
    private var statusFooter: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 10) {
                statusFooterLeading
                    .layoutPriority(1)
                if host.hasTape || host.tapePlaying {
                    statusFooterProgress
                        .frame(maxWidth: .infinity)
                        .layoutPriority(0)
                    statusFooterDeckControls
                        .layoutPriority(1)
                } else {
                    Spacer(minLength: 0)
                }
            }
            // Host status under the deck chrome. Always-on "Inserted…" duplicated the
            // tape name row (#375 open-error surfacing). Keep errors + empty-deck
            // messages, plus Instant progress; hide routine Inserted/Play/Pause clutter.
            if statusFooterShowsHostStatus {
                Text(host.status)
                    .font(.caption2)
                    .foregroundStyle(statusFooterMessageIsError ? Color.orange : Color.secondary)
                    .lineLimit(2)
                    .truncationMode(.tail)
                    .textSelection(.enabled)
                    .help(host.status)
                    .accessibilityLabel("Host status")
                    .accessibilityValue(host.status)
            }
            if !host.roomPerfLine.isEmpty {
                Text(host.roomPerfLine)
                    .font(.system(.caption2, design: .monospaced))
                    .foregroundStyle(.secondary)
                    .lineLimit(3)
                    .textSelection(.enabled)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .glassBarBackground()
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Tape status")
    }

    /// Heuristic: open/play/attach failures vs routine "Inserted…" / Instant progress.
    private var statusFooterMessageIsError: Bool {
        let s = host.status.lowercased()
        return s.contains("fail")
            || s.contains("unsupported")
            || s.contains("error")
            || s.contains("truncated")
            || s.contains("missing")
    }

    /// Second footer line: errors always; Instant feedback; any status when no deck chrome.
    private var statusFooterShowsHostStatus: Bool {
        guard !host.status.isEmpty else { return false }
        if statusFooterMessageIsError { return true }
        if !(host.hasTape || host.tapePlaying) { return true }
        return host.status.hasPrefix("Instant")
    }

    @ViewBuilder
    private var statusFooterLeading: some View {
        if host.hasTape || host.tapePlaying {
            Text(statusFooterTapeName)
                .font(.caption.weight(.medium))
                .foregroundStyle(.primary)
                .lineLimit(1)
                .truncationMode(.middle)
                .frame(maxWidth: 180, alignment: .leading)
                .help(statusFooterTapeName)
                .accessibilityLabel("Tape")
                .accessibilityValue(statusFooterTapeName)
        } else {
            HStack(spacing: 8) {
                Text("No tape present")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                Button {
                    chromeAction { host.presentOpenMediaPanel() }
                } label: {
                    Label(host.openMediaTitle, systemImage: "opticaldiscdrive")
                        .labelStyle(.titleAndIcon)
                        .font(.caption)
                }
                .buttonStyle(.bordered)
                .controlSize(.small)
                .help(host.openMediaMenuTitle)
                .accessibilityLabel(host.openMediaTitle)
            }
            .fixedSize(horizontal: true, vertical: false)
        }
    }

    private var statusFooterTapeName: String {
        if let media = host.mediaTitle, !media.isEmpty {
            return media
        }
        return "Tape"
    }

    private var statusFooterProgress: some View {
        let frac = min(max(host.tapeFraction ?? 0, 0), 1)
        let label = host.tapeBlockLabel.isEmpty ? "…" : host.tapeBlockLabel
        return ZStack {
            // Track must stay visible on glass chrome (low-contrast Capsule washes out).
            Capsule()
                .fill(Color(nsColor: .tertiaryLabelColor).opacity(0.35))
                .overlay(
                    Capsule()
                        .strokeBorder(Color.primary.opacity(0.18), lineWidth: 0.5)
                )
            GeometryReader { geo in
                Capsule()
                    .fill(Color.accentColor)
                    .frame(width: max(frac > 0.001 ? 8 : 0, geo.size.width * frac))
            }
            .clipShape(Capsule())
            .padding(1)
            Text(label)
                .font(.caption2.weight(.semibold))
                .foregroundStyle(.primary)
                .lineLimit(1)
                .minimumScaleFactor(0.75)
                .padding(.horizontal, 8)
                .allowsHitTesting(false)
        }
        .frame(maxWidth: .infinity)
        .frame(height: 20)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Tape progress")
        .accessibilityValue("\(label), \(Int((frac * 100).rounded()))%")
    }

    private var statusFooterDeckControls: some View {
        HStack(spacing: 2) {
            Button {
                chromeAction { host.rewindTape() }
            } label: {
                Image(systemName: "backward.end.fill")
                    .imageScale(.small)
                    .frame(width: 22, height: 20)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.borderless)
            .help("Rewind tape")
            .accessibilityLabel("Rewind tape")

            Button {
                chromeAction {
                    if host.tapePlaying {
                        host.pauseTape()
                    } else {
                        host.playTape()
                    }
                }
            } label: {
                Image(systemName: host.tapePlaying ? "pause.fill" : "play.fill")
                    .imageScale(.small)
                    .frame(width: 22, height: 20)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.borderless)
            .help(host.tapePlaying ? "Pause tape" : "Play tape (EAR path; Instant is flash-load)")
            .accessibilityLabel(host.tapePlaying ? "Pause tape" : "Play tape")

            Picker("Load", selection: tapeLoadModeBinding) {
                Text("Experience").tag(UInt32(0))
                Text("1x").tag(UInt32(1))
                Text("2x").tag(UInt32(2))
                Text("5x").tag(UInt32(5))
                Text("10x").tag(UInt32(10))
                Text("20x").tag(UInt32(20))
                Text("64x").tag(UInt32(64))
            }
            .pickerStyle(.menu)
            .labelsHidden()
            .fixedSize()
            .controlSize(.mini)
            .help("Experience: ~20s abbreviated EAR load; otherwise N Spectrum frames/tick (64× helps Speedlock delays)")
            .accessibilityLabel("Tape load mode")
            .accessibilityValue(tapeLoadModeAccessibilityValue)
        }
        .fixedSize(horizontal: true, vertical: false)
    }

    private var tapeLoadModeBinding: Binding<UInt32> {
        Binding(
            get: { host.experienceLoad ? 0 : host.tapeSpeed },
            set: { val in
                if val == 0 {
                    // tapeSpeed.didSet clears experienceLoad — set speed first.
                    host.tapeSpeed = 16
                    host.experienceLoad = true
                } else {
                    host.experienceLoad = false
                    host.tapeSpeed = val
                }
                FocusSpectrumView.post()
            }
        )
    }

    private var tapeLoadModeAccessibilityValue: String {
        host.experienceLoad ? "Experience" : "\(host.tapeSpeed)x"
    }

    /// After chrome clicks, return key focus to the Spectrum / room view.
    private func chromeAction(_ work: () -> Void) {
        work()
        FocusSpectrumView.post()
    }
}

struct DebugInspectorView: View {
    @ObservedObject var host: HostBridge

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("Debug Inspector")
                    .font(.headline)
                Spacer()
                Button("Done") {
                    host.showInspector = false
                }
                .keyboardShortcut(.cancelAction)
            }
            HStack(spacing: 16) {
                Text(String(format: "PC %04X", host.debugPc))
                Text(String(format: "SP %04X", host.debugSp))
                Text(String(format: "AF %04X", host.debugAf))
            }
            .font(.system(.body, design: .monospaced))
            HStack {
                Button(host.paused ? "Continue" : "Pause") {
                    host.setPaused(!host.paused)
                }
                Button("Step") {
                    if !host.paused {
                        host.setPaused(true)
                    }
                    host.step()
                }
            }
            ScrollView {
                Text(host.inspectJsonPreview)
                    .font(.system(.caption, design: .monospaced))
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .padding()
        .frame(minWidth: 480, minHeight: 320)
        .onAppear {
            host.refreshInspector()
        }
    }
}

/// Ask the embedded Spectrum `NSView` to take first responder (SwiftUI focus alone is not enough).
enum FocusSpectrumView {
    static let name = Notification.Name("SpecChumFocusSpectrumView")
    private(set) static var menuTrackingDepth = 0

    static var isMenuTracking: Bool { menuTrackingDepth > 0 }

    /// Register once at launch — gates the key monitor while AppKit menus are open.
    static func installMenuTrackingObservers() {
        NotificationCenter.default.addObserver(
            forName: NSMenu.didBeginTrackingNotification,
            object: nil,
            queue: .main
        ) { _ in
            menuTrackingDepth += 1
        }
        NotificationCenter.default.addObserver(
            forName: NSMenu.didEndTrackingNotification,
            object: nil,
            queue: .main
        ) { _ in
            menuTrackingDepth = max(0, menuTrackingDepth - 1)
            postDelayed()
        }
    }

    static func post() {
        NotificationCenter.default.post(name: name, object: nil)
    }

    /// Reclaim focus after SwiftUI chrome (toolbar menus, sheets) closes.
    /// Retries across run loops — Machine picker menus often steal first responder briefly.
    static func postDelayed() {
        post()
        DispatchQueue.main.async { post() }
        for delay in [0.05, 0.15, 0.35] {
            DispatchQueue.main.asyncAfter(deadline: .now() + delay) {
                post()
            }
        }
    }
}

/// Keep the NSWindow title in sync with media + machine (document-style HIG).
private struct WindowTitleBinder: NSViewRepresentable {
    var title: String

    func makeNSView(context: Context) -> NSView {
        let view = NSView()
        DispatchQueue.main.async { apply(to: view) }
        return view
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        apply(to: nsView)
    }

    private func apply(to view: NSView) {
        guard let window = view.window else { return }
        if window.title != title {
            window.title = title
        }
        WindowChrome.applyTabBarPolicy(to: window)
    }
}

/// Shared NSWindow chrome: unified toolbar drag band, hide macOS document tabs.
enum WindowChrome {
    static func applyTabBarPolicy(to window: NSWindow) {
        window.tabbingMode = .disallowed
        if !window.styleMask.contains(.fullSizeContentView) {
            window.styleMask.insert(.fullSizeContentView)
        }
        window.titlebarAppearsTransparent = true
        window.toolbarStyle = .unified
        // Document tab strip (single "Spec Chum" tab + "+") sits below the toolbar — hide it.
        window.titleVisibility = .hidden
        window.titlebarSeparatorStyle = .none
        // Persist frame across relaunches (#186); clamp is handled by min window size.
        if window.frameAutosaveName != "SpecChumMainWindow" {
            window.setFrameAutosaveName("SpecChumMainWindow")
        }
    }
}

/// Flat-mode window wash (living room uses black behind the Metal blit).
private struct WindowBackground: View {
    var body: some View {
        if #available(macOS 26, *) {
            Color.clear
                .glassEffect(.clear, in: .rect)
                .ignoresSafeArea()
        } else {
            LinearGradient(
                colors: [
                    Color(nsColor: .windowBackgroundColor),
                    Color(nsColor: .underPageBackgroundColor),
                ],
                startPoint: .topLeading,
                endPoint: .bottomTrailing
            )
            .ignoresSafeArea()
        }
    }
}
