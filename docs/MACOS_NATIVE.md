# Native macOS shell (SwiftUI + Rust host_api)

Optional product UI for Spec Chum on Apple silicon / Intel Macs. The accuracy core
stays in Rust; this app is a thin SwiftUI host with native menus and liquid-glass
materials when the OS/SDK supports them.

Cross-platform primary host remains the egui app (`cargo run -p app`).

## Requirements

- Full **Xcode** (or Xcode beta), not Command Line Tools alone — SwiftUI macros need it
- Rust toolchain (`rustup`)
- Fetched ROMs: `./scripts/fetch_roms.sh`

## Build & run

From the repository root:

```bash
./scripts/build_macos_app.sh
./scripts/run_macos_app.sh
```

`run_macos_app.sh` stages a minimal `SpecChumMac.app` and launches it with **`open`** (Launch Services) so SpecChum becomes the **key application**. Do **not** type into the Terminal that ran the script — click the Spec Chum window if needed; keys should drive 48K BASIC and must **not** echo in Terminal.

`build_macos_app.sh` will:

1. `cargo build -p host_api --release` (staticlib + cdylib C ABI)
2. Sync `spec_chum_host.h` into the Swift package
3. `swift build -c release` for `apps/macos` (sets `DEVELOPER_DIR` to Xcode if needed)
4. Normalize the dylib install name to `@rpath/libspec_chum_host.dylib`

Environment:

- `SPEC_CHUM_ROOT` — repo root used to find `roms/` (set by the app wrapper)
- `DEVELOPER_DIR` — override Xcode path (defaults to `Xcode.app` or `Xcode-beta.app`)
- `DYLD_LIBRARY_PATH` — set by the staged `.app` launcher to `target/release`

## What this vertical slice includes

- Native `MenuBar` / `Commands` (File → Open Tape / Open ROM, Tape Play/Pause/Rewind, Machine Reset + model)
- Toolbar / status chrome via SwiftUI **`glassEffect`** on macOS 26+; fallback **`.ultraThinMaterial`**
- ~50 Hz framebuffer blit (RGBA from Rust) with nearest-neighbor aspect-fit
- TAP/TZX open via `NSOpenPanel`, Play/Pause wired to `host_api`
- Keyboard: app activation + Spectrum `NSView` first responder + `sc_set_key` (see below)

## Keyboard (Mac native shell)

Mapping matches egui (`crates/app/src/keymap.rs`) using **ANSI key codes** in
`SpectrumKeymap.swift`:

| Host | Spectrum |
| --- | --- |
| Letters / digits | Direct matrix |
| Shift | Caps Shift |
| Option / Ctrl | Symbol Shift |
| Arrows | Caps + 5/6/7/8 (cursor) |
| Backspace | Caps + 0 (DELETE) |
| `'` / `"` | Symbol + 7 / Symbol + P |
| `; , . / - = [ ] \\ \`` (+ Shift variants) | Same Symbol layer as egui |

**Key application vs focus ring:** A SwiftUI focus ring (or `.focusable()`) does **not**
make SpecChum the key app. Keystrokes only reach the Spectrum matrix when SpecChum
is key. On launch / appear / click the shell calls `NSApp.activate`,
`makeKeyAndOrderFront`, and makes the Spectrum `NSView` first responder
(`acceptsFirstResponder`, no decorative focus ring). Prefer the view’s
`keyDown` / `keyUp` / `flagsChanged`; a local `NSEvent` monitor is only a backup
when a SwiftUI host steals first responder while the window stays key.
⌘-modified keys clear the matrix and are left alone for menu shortcuts.

**Verify:** after `./scripts/run_macos_app.sh`, typing must **not** appear in Terminal;
after 48K BASIC boots, letters should appear in BASIC.

Not yet: Kempston mirroring (egui still maps Tab/arrows to joystick).

## Liquid glass APIs used

| API | Where | Fallback |
| --- | --- | --- |
| `View.glassEffect(_:in:)` | Toolbar bar, chips, status strip, window wash (`GlassChrome.swift`, `ContentView.swift`) | `.ultraThinMaterial` / gradient |
| `Glass.regular` / `.regular.interactive()` / `.clear` | Same | n/a |
| `#available(macOS 26, *)` | Gates liquid glass | Older macOS materials |

## Not in this slice (still egui-only or follow-ups)

- Audio (beeper / AY via `cpal` in egui)
- Kempston joystick mirroring in the native shell
- Snapshots (SNA/Z80), RZX, DSK
- Signed / notarized distribution bundle (dev launch uses a staged `.app` via `open`)
- CI job that compiles the Swift shell (Linux CI stays Rust-only)

## Layout

```
crates/host_api/          # Rust safe session + C ABI (sc_*)
apps/macos/               # SwiftPM executable SpecChumMac (+ staged .app at run)
scripts/build_macos_app.sh
scripts/run_macos_app.sh
```
