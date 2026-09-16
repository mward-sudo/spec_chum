# Native Windows shell (Win32 + Rust host_api)

**Release-primary** product UI for Spec Chum on Windows — classic **Win32** window
and menus driven by the shared Rust [`host_api`](../crates/host_api) session (same
thin-host pattern as SpecChumMac). GitHub Releases ship this shell as
`spec_chum.exe` (portable `.zip` + Inno Setup). Cross-platform **egui**
(`cargo run -p app`) remains the CI baseline and the headless `--serve` /
`debug …` host on Windows (source builds).

Track: [#351](https://github.com/mward-sudo/spec_chum/issues/351). Strategy:
[UI_ARCHITECTURE.md — Native shells](UI_ARCHITECTURE.md#native-shells-351).
Parity: product features match egui / SpecChumMac / `linux_shell` unless
genuinely unavailable on Windows; chrome may follow Win32 / Fluent HIG. See
[UI_ARCHITECTURE.md — Native shells](UI_ARCHITECTURE.md#native-shells-351).

## Feature parity vs platform HIG

| Layer | Rule |
| --- | --- |
| **Product features** | Match egui / SpecChumMac / `linux_shell` (media types, tape transport, Agent Debug HTTP, …) unless a capability is genuinely unavailable on Windows |
| **Chrome / UX** | Follow Windows conventions (Win32 menus, Ctrl shortcuts, Fluent/WinUI later) — do not mimic macOS chrome |
| **Shared core** | Behaviour in `host_api` / `control_plane`; this crate stays a thin adapter |

Example: **Open Tape** is ⌘O on SpecChumMac and **Ctrl+O** here — same action, platform modifier.

## Capability matrix

| Capability | Status |
| --- | --- |
| Native Win32 window + File/Tape menus | yes |
| Framebuffer present (`StretchDIBits`) | yes |
| Open tape / snapshot / RZX / DSK / TRD | yes (`rfd` native dialogs; same formats as egui/macOS) |
| Tape Play / Pause / Rewind | yes |
| Keyboard → Spectrum matrix | yes (VK map in `windows_shell::keymap`) |
| Audio (cpal from `HostSession` PCM) | yes |
| Agent Debug HTTP (`SPEC_CHUM_AGENT=1`) | yes (same embed as egui / SpecChumMac) |
| Machine model select + Reset | yes (all built-in models) |
| Hardware attach (Multiface / DivMMC / IF1 / Beta / Timex dock) | yes (Win32 **Hardware** menu → `HostSession`) |
| Settings / prefs (`UiPreferences` load/save) | yes (joystick, tape EAR/Experience/Instant, AY, mute, throttle, online titles) |
| Debug / inspect (pause / step / continue / breakpoints + inspector window) | yes (Win32 **Debug** menu + multiline inspect HWND) |
| Living-room / WinUI 3 | **out of scope** |
| Release packaging primary over egui | **yes** (staged as `spec_chum.exe`; [#351](https://github.com/mward-sudo/spec_chum/issues/351)) |

## Requirements

- Windows 10/11 with a Rust MSVC toolchain (`x86_64-pc-windows-msvc`)
- Fetched ROMs: `./scripts/fetch_roms.sh` (or copy `roms/` next to the binary)

## ROM search roots

`HostSession` uses `machine::search_roots()` for `roms/spec48.rom` (and other
model images), in order:

1. Process current directory
2. `SPEC_CHUM_ROOT` — **repository root** (parent of `roms/`), not the `roms` folder itself
3. `SPEC_CHUM_ROM_ROOT` — alternate root with the same `roms/…` layout
4. Directory containing the executable (so a packaged `roms/` next to `spec_chum.exe` works)
5. Executable parent’s `share/spec-chum` (Linux FHS / AppImage layout; usually unused on Windows)
6. Compile-time workspace root (dev builds)

macOS `.app` `Contents/Resources` is also searched when the exe lives under
`Contents/MacOS` (SpecChumMac); the Win32 shell does not use that layout.

Trailing slash on drive roots is fine (`S:\`). Paths are case-insensitive on Windows.

Parallels shared-folder example (repo mapped as `S:`):

```powershell
$env:SPEC_CHUM_ROOT = 'S:\'
Set-Location S:\
cargo run -p windows_shell --release
```

## Build & run

From the repository root **on Windows**:

```powershell
./scripts/build_windows_app.ps1
./scripts/run_windows_app.ps1
```

Or:

```powershell
$env:SPEC_CHUM_ROOT = (Get-Location).Path   # if cwd is already the repo root
cargo run -p windows_shell --release
```

Release CI builds `spec_chum_windows.exe` and stages it as **`spec_chum.exe`** in
the portable zip / Inno installer (same install name as before the promote).

On macOS/Linux the `spec_chum_windows` binary is a stub that exits with a pointer
to egui — the Win32 UI is `cfg(windows)` only. Keymap + menu-id unit tests still
run everywhere:

```bash
cargo test -p windows_shell
```

### Agent Debug HTTP

Same as SpecChumMac / egui embed:

```powershell
$env:SPEC_CHUM_AGENT = "1"
$env:SPEC_CHUM_AGENT_INSECURE = "1"   # or set SPEC_CHUM_AGENT_TOKEN
cargo run -p windows_shell --release
```

Headless `--serve` / `debug …` CLI remain on the egui binary (`cargo run -p app`),
not on the Win32 release exe (mirror of SpecChumMac vs egui on macOS).

Prefs file: same `ui-prefs.json` as egui (`SPEC_CHUM_PREFS_PATH` override supported).

## CI

Opt-in job `windows-shell` in `.github/workflows/ci.yml` builds the crate on
`windows-latest` and does not block the Linux fmt/clippy/test gate. Release
workflow builds this crate for Windows archives ([RELEASE.md](RELEASE.md)).

## Non-goals (for now)

- WinUI 3 XAML chrome (provisional future; this shell is classic Win32)
- Replacing egui as the **CI** / cross-platform fallback host
