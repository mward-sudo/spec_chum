# Native Windows shell (Win32 + Rust host_api)

Optional product UI for Spec Chum on Windows — classic **Win32** window and menus
driven by the shared Rust [`host_api`](../crates/host_api) session (same thin-host
pattern as SpecChumMac). Cross-platform **egui** (`cargo run -p app`) remains the
shipped Windows product UI in GitHub Releases until this shell is promoted.

Track: [#351](https://github.com/mward-sudo/spec_chum/issues/351). Strategy:
[UI_ARCHITECTURE.md — Native shells](UI_ARCHITECTURE.md#native-shells-351).
Parity rule: [`.cursor/rules/gui-app-parity.mdc`](../.cursor/rules/gui-app-parity.mdc).

## Feature parity vs platform HIG

| Layer | Rule |
| --- | --- |
| **Product features** | Match egui / SpecChumMac (media types, tape transport, agent HTTP, …) unless a capability is genuinely unavailable on Windows |
| **Chrome / UX** | Follow Windows conventions (Win32 menus, Ctrl shortcuts, Fluent/WinUI later) — do not mimic macOS chrome |
| **Shared core** | Behaviour in `host_api` / `control_plane`; this crate stays a thin adapter |

Example: **Open Tape** is ⌘O on SpecChumMac and **Ctrl+O** here — same action, platform modifier.

## Vertical slice (this tree)

| Capability | Status |
| --- | --- |
| Native Win32 window + File/Tape menus | yes |
| Framebuffer present (`StretchDIBits`) | yes |
| Open tape / snapshot / RZX / DSK / TRD | yes (`rfd` native dialogs; same formats as egui/macOS) |
| Tape Play / Pause / Rewind | yes |
| Keyboard → Spectrum matrix | yes (VK map in `windows_shell::keymap`) |
| Audio (cpal from `HostSession` PCM) | yes |
| Agent Debug HTTP (`SPEC_CHUM_AGENT=1`) | yes (same embed as egui) |
| Hardware attach panels / prefs UI / Debug window | **deferred** — deepen under GUI-parity before promoting off egui as release primary |
| Living-room / WinUI 3 | **out of scope** for this slice |

## Requirements

- Windows 10/11 with a Rust MSVC toolchain (`x86_64-pc-windows-msvc`)
- Fetched ROMs: `./scripts/fetch_roms.sh` (or copy `roms/` next to the binary)

## Build & run

From the repository root **on Windows**:

```powershell
./scripts/build_windows_app.ps1
./scripts/run_windows_app.ps1
```

Or:

```powershell
cargo run -p windows_shell --release
```

On macOS/Linux the `spec_chum_windows` binary is a stub that exits with a pointer
to egui — the Win32 UI is `cfg(windows)` only. Keymap unit tests still run everywhere:

```bash
cargo test -p windows_shell
```

### Agent Debug HTTP

Same as egui / SpecChumMac:

```powershell
$env:SPEC_CHUM_AGENT = "1"
$env:SPEC_CHUM_AGENT_INSECURE = "1"   # or set SPEC_CHUM_AGENT_TOKEN
cargo run -p windows_shell --release
```

## CI

Opt-in job `windows-shell` in `.github/workflows/ci.yml` builds the crate on
`windows-latest` and does not block the Linux fmt/clippy/test gate.

## Non-goals (for now)

- Replacing egui in Windows release packages ([#231](https://github.com/mward-sudo/spec_chum/issues/231))
- WinUI 3 XAML chrome (provisional future; this slice is classic Win32)
- Full Hardware / Settings / Debug panel parity with SpecChumMac — required before the shell becomes the Windows release primary, tracked under [#351](https://github.com/mward-sudo/spec_chum/issues/351)
