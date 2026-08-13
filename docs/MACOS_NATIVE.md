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

`build_macos_app.sh` will:

1. `cargo build -p host_api --release` (staticlib + cdylib C ABI)
2. Sync `spec_chum_host.h` into the Swift package
3. `swift build -c release` for `apps/macos` (sets `DEVELOPER_DIR` to Xcode if needed)
4. Normalize the dylib install name to `@rpath/libspec_chum_host.dylib`

Environment:

- `SPEC_CHUM_ROOT` — repo root used to find `roms/`
- `DEVELOPER_DIR` — override Xcode path (defaults to `Xcode.app` or `Xcode-beta.app`)
- `DYLD_LIBRARY_PATH` — set by `run_macos_app.sh` to `target/release`

## What this vertical slice includes

- Native `MenuBar` / `Commands` (File → Open Tape / Open ROM, Tape Play/Pause/Rewind, Machine Reset + model)
- Toolbar / status chrome via SwiftUI **`glassEffect`** on macOS 26+; fallback **`.ultraThinMaterial`**
- ~50 Hz framebuffer blit (RGBA from Rust) with nearest-neighbor aspect-fit
- TAP/TZX open via `NSOpenPanel`, Play/Pause wired to `host_api`
- Keyboard: Spectrum view focuses on appear/click; local `NSEvent` monitor + `sc_set_key` (see below)

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
| `; , . / - =` (+ Shift variants) | Same Symbol layer as egui |

**Focus:** SwiftUI often keeps first responder off `NSViewRepresentable` children.
The shell claims focus on appear/click and installs a **local key monitor** while
the window is key so BASIC typing works even when the hosting view steals focus.
⌘-modified keys are left alone for menu shortcuts.

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
- App bundle / signing / notarization
- CI job that compiles the Swift shell (Linux CI stays Rust-only)

## Layout

```
crates/host_api/          # Rust safe session + C ABI (sc_*)
apps/macos/               # SwiftPM executable SpecChumMac
scripts/build_macos_app.sh
scripts/run_macos_app.sh
```
