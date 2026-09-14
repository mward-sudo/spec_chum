# Native Linux shell (GTK4 + Rust host_api)

Optional product UI for Spec Chum on Linux — **GTK4** (gtk-rs) window and menus
driven by the shared Rust [`host_api`](../crates/host_api) session (same thin-host
pattern as SpecChumMac / `windows_shell`). Cross-platform **egui** (`cargo run -p app`)
remains the shipped Linux product UI in GitHub Releases until this shell is promoted.

Track: [#351](https://github.com/mward-sudo/spec_chum/issues/351). Strategy:
[UI_ARCHITECTURE.md — Native shells](UI_ARCHITECTURE.md#native-shells-351).
Parity rule: [`.cursor/rules/gui-app-parity.mdc`](../.cursor/rules/gui-app-parity.mdc).

## Toolkit choice

| Option | Notes |
| --- | --- |
| **GTK4 via gtk-rs (chosen)** | Thin Rust bindings, HIG-friendly on GNOME/Cosmic/etc., mature file dialogs / menus, no webview |
| Qt (cxx-qt / qml) | Strong KDE fit; heavier binding/tooling tax for a small team |
| iced / egui-as-“native” | Not the product preference — egui already ships as interim primary |

## Feature parity vs platform HIG

| Layer | Rule |
| --- | --- |
| **Product features** | Match egui / SpecChumMac / `windows_shell` (media types, tape transport, agent HTTP, …) unless a capability is genuinely unavailable on Linux |
| **Chrome / UX** | Follow GTK / desktop conventions (menubar, Ctrl shortcuts) — do not mimic macOS chrome |
| **Shared core** | Behaviour in `host_api` / `control_plane`; this crate stays a thin adapter |

Example: **Open Tape** is ⌘O on SpecChumMac and **Ctrl+O** here — same action, platform modifier.

## Capability matrix (vertical slice)

| Capability | Status |
| --- | --- |
| Native GTK4 window + File/Tape menus | yes |
| Framebuffer present (`gdk::MemoryTexture` → `gtk::Picture`) | yes |
| Open tape / snapshot / RZX / DSK / TRD | yes (`rfd` native dialogs; same formats as egui/macOS/Windows) |
| Tape Play / Pause / Rewind | yes |
| Keyboard → Spectrum matrix | yes (GDK keyval map in `linux_shell::keymap`) |
| Audio (cpal from `HostSession` PCM) | yes |
| Agent Debug HTTP (`SPEC_CHUM_AGENT=1`) | yes (same embed as egui / Windows) |
| Machine model select + Reset | **deferred** (next deepen) |
| Hardware attach (Multiface / DivMMC / IF1 / Beta / Timex dock) | **deferred** |
| Settings / prefs UI (`UiPreferences` load/save beyond startup) | **partial** — prefs load at startup; Settings menu deferred |
| Debug / inspect panel | **deferred** |
| Living-room display | **out of scope** for this shell |
| Release packaging promote over egui | **still open** under [#351](https://github.com/mward-sudo/spec_chum/issues/351) / [#231](https://github.com/mward-sudo/spec_chum/issues/231) |

## Requirements

- Linux with GTK 4 development/runtime libraries (`libgtk-4-dev` / `libgtk-4-1`, plus the existing ALSA/udev deps used by egui)
- Fetched ROMs: `./scripts/fetch_roms.sh` (or copy `roms/` next to the binary)

`SPEC_CHUM_ROOT` (and optional `SPEC_CHUM_ROM_ROOT`) must point at the **repository root**
(parent of `roms/`), not at `roms/` itself — same layout as
[WINDOWS_NATIVE.md](WINDOWS_NATIVE.md#rom-search-roots).

## Build & run

From the repository root **on Linux**:

```bash
./scripts/run_linux_shell.sh
```

Or:

```bash
cargo run -p linux_shell --release
```

On macOS/Windows the `spec_chum_linux` binary is a stub that exits with a pointer
to egui — the GTK4 UI is `cfg(target_os = "linux")` only. Keymap unit tests still
run everywhere:

```bash
cargo test -p linux_shell
```

### Agent Debug HTTP

Same as egui / SpecChumMac / Windows:

```bash
SPEC_CHUM_AGENT=1 SPEC_CHUM_AGENT_INSECURE=1 cargo run -p linux_shell --release
```

Prefs file: same `ui-prefs.json` as egui (`SPEC_CHUM_PREFS_PATH` override supported).

## CI

Opt-in job `linux-shell` in `.github/workflows/ci.yml` builds the crate on
`ubuntu-latest` with `libgtk-4-dev`. Default `fmt + clippy + test` also installs
GTK4 so workspace clippy stays green. Does not change Linux release packaging.

## Non-goals (for now)

- Replacing egui in Linux release packages ([#231](https://github.com/mward-sudo/spec_chum/issues/231))
- Closing epic [#351](https://github.com/mward-sudo/spec_chum/issues/351) — Windows release-primary promotion and Linux deepen remain open
- Qt / webview / Electron hosts
