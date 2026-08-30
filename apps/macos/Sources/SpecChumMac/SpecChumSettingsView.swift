import SwiftUI

/// Preferences (⌘,) — model, EAR speed, host volume, joystick, Kempston mouse.
/// Frequent actions (Open / Instant / Play / Rewind) stay on the toolbar.
struct SpecChumSettingsView: View {
    @ObservedObject var host: HostBridge

    var body: some View {
        Form {
            Section("Machine") {
                Text("Built-in models and custom configurations are in the Machine menu and toolbar picker.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Text("Built-ins use default ROMs from ./scripts/fetch_roms.sh. Custom ROM overrides belong in a saved configuration only.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Text("Timex TC2048 (Phase 1): standard 256×192 only — 512×192 hi-res and extended SCLD modes are not drawn yet. See docs/TIMEX.md.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Section("Tape") {
                Picker("EAR speed", selection: $host.tapeSpeed) {
                    Text("1x").tag(UInt32(1))
                    Text("2x").tag(UInt32(2))
                    Text("5x").tag(UInt32(5))
                    Text("10x").tag(UInt32(10))
                    Text("20x").tag(UInt32(20))
                }
                .help("While Play: N Spectrum frames per tick (wall-clock ≈ realtime/N). Instant flash ignores this.")
                Text("Toolbar Instant always opens a TAP/TZX panel, then flash-loads. Play alone stays on the EAR path at this speed. Disk images use Open Tape / Disk — Instant does not Type LOAD for DSK.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Section("Audio") {
                Toggle("Mute", isOn: $host.outputMuted)
                    .help("Silence host PCM output only (not EAR fidelity)")
                Slider(value: $host.outputVolume, in: 0 ... 1) {
                    Text("Volume")
                }
                .disabled(host.outputMuted)
                Text("Host mixer gain for beeper / EAR mix / AY. Does not change tape bitstream timing.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Section("Joystick") {
                Picker("Mode", selection: $host.joystickMode) {
                    ForEach(HostBridge.JoystickMode.allCases) { mode in
                        Text(mode.title).tag(mode)
                    }
                }
                .help("Host joystick presentation (GCController + keyboard mirror)")
                Text("Arrows + Tab fire still inject cursor matrix chords alongside the stick mask.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Section("Mouse") {
                Toggle("Kempston mouse", isOn: $host.kempstonMouse)
                    .help("Map Spectrum / living-room pointer delta and buttons to Kempston ports (egui parity)")
                Text("When enabled, motion over the Spectrum or living-room view updates ports FBDF/FFDF/FADF.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Section("ROMs / copyright") {
                Text(
                    "Amstrad have kindly given their permission for the redistribution of their copyrighted material but retain that copyright."
                )
                .font(.caption)
                Text(
                    "System ROMs are not shipped with Spec Chum. Fetch official images with ./scripts/fetch_roms.sh — see docs/ROMS.md."
                )
                .font(.caption)
                .foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
        .padding()
        .frame(width: 440, height: 500)
    }
}
