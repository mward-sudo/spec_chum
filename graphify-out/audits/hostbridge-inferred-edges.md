# HostBridge INFERRED-edge audit (#345)

Audited 2026-09-11 against SpecChumMac sources. All six graphify INFERRED
edges involving `HostBridge` were **correct** (real ownership / `host.` use).
Promoted to `EXTRACTED` in `graphify-out/graph.json` with `_audit:
verified_correct_#345`.

| Edge | Why correct |
| --- | --- |
| `.livingRoomToolbar` → `HostBridge` | Toolbar actions call `host.presentOpenMediaPanel`, Instant, ROM setup, living-room toggle, mute (`ContentView.swift`) |
| `.statusFooterMessageIsError` → `HostBridge` | Reads `host.status` for error heuristics |
| `.statusFooterShowsHostStatus` → `HostBridge` | Reads `host.status` / tape flags |
| `SpecChumMacApp.body` → `HostBridge` | Scene binds `ContentView(host:)`, sheets, menus to `@StateObject` host |
| `SpecChumMacApp` → `HostBridge` | `@StateObject private var host = HostBridge()` |
| `HostBridge` → `TapeAudioPlayer` | `let audio = TapeAudioPlayer()` owned by the bridge |

Swift AST extraction often leaves property/`@StateObject` edges as INFERRED;
promotion here is deliberate verification, not a code change.

#345 checklist complete after Plus3Fdc split (`crates/formats/src/fdc/` — commands / sector / result); host_api FFI split landed in #407.
are separate follow-ups — not part of this slice.
