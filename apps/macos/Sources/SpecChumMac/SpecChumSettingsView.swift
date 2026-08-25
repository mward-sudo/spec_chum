import SwiftUI

/// Preferences window (⌘,) — EAR speed / Joystick / model (HIG Settings scene).
/// Instant flash-load is a toolbar / Tape menu **action**, not a sticky checkbox.
struct SpecChumSettingsView: View {
    @ObservedObject var host: HostBridge

    var body: some View {
        TabView {
            Form {
                Section("Machine") {
                    Picker("Model", selection: $host.model) {
                        ForEach(HostBridge.Model.allCases) { model in
                            Text(model.title).tag(model)
                        }
                    }
                }
                Section("Tape") {
                    Picker("EAR speed", selection: $host.tapeSpeed) {
                        Text("1x").tag(UInt32(1))
                        Text("2x").tag(UInt32(2))
                        Text("5x").tag(UInt32(5))
                        Text("10x").tag(UInt32(10))
                        Text("20x").tag(UInt32(20))
                    }
                    .help("EAR bitstream speed (also speeds pulse TZX); Instant toolbar enables flash-load for a load")
                    Text("Use the Instant toolbar button (or Tape → Instant) to flash-load: Type LOAD \"\" then Play.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            .formStyle(.grouped)
            .padding()
            .tabItem {
                Label("General", systemImage: "gearshape")
            }

            Form {
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
            }
            .formStyle(.grouped)
            .padding()
            .tabItem {
                Label("Input", systemImage: "gamecontroller")
            }
        }
        .frame(width: 420, height: 280)
    }
}
