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
- `SPEC_CHUM_DEBUG=1` / `SPEC_CHUM_TRACE=tape,cpu` — structured tracing (see [`DEBUGGING.md`](DEBUGGING.md)); Mac **Debug** menu dumps the ring

## What this vertical slice includes

- Native `MenuBar` / `Commands` (File → Open Tape / Open ROM, Tape Play/Pause/Rewind, Machine Reset + model, Debug trace dump)
- Toolbar / status chrome via SwiftUI **`glassEffect`** on macOS 26+; fallback **`.ultraThinMaterial`**
- ~50 Hz framebuffer blit (RGBA from Rust) with nearest-neighbor aspect-fit
- TAP/TZX open via `NSOpenPanel`, Play/Pause wired to `host_api`
- **Audio:** mono PCM from `sc_audio_*` each frame via `AVAudioEngine` (beeper + EAR mix + AY)
- **Tape progress:** `ProgressView` from `sc_tape_progress` (block / pulse position)
- Keyboard: app activation + Spectrum `NSView` first responder + `sc_set_key` (see below)
- **Joystick:** `GCController` (USB/Bluetooth) + keyboard Kempston mirror via `sc_set_joystick` (see below)

## Tape loading (Play / LD-BYTES)

Insert starts **paused**. Enter the loader first (48K: `LOAD ""` / Type LOAD; 128K: Tape Loader), then **Tape → Play**.

The core **holds** at ROM `LD-BYTES` (`0x056C`) while paused so Play can still arm flash-load / EAR. Pressing Play after the ROM has already run past that trap used to show a brief border flash (pilot) then stall — that race is fixed.

Standard-speed TZX is converted to TAP for flash-load. **Instant** flash-load (default) is near-instant (little border activity). Turn Instant off for authentic EAR border stripes / load tones; use the **Speed** control (`1x`…`20x`) to turbo the EAR bitstream. Options: Mac toolbar Instant checkbox + Speed menu; egui **Tape** menu; `sc_tape_set_load_options`.

### Verify on Mac 48K

1. `./scripts/fetch_roms.sh` then `./scripts/run_macos_app.sh`
2. Model **48K**. Open `tests/fixtures/tape/attr_mark.tap` (or `print_ok.tap` / `minimal_code.tap`).
3. Leave **Instant** on. For PROGRAM loaders type `LOAD ""` + Enter (egui **Type LOAD ""**). For CODE blocks type `LOAD "" CODE` (egui **Type LOAD "" CODE**). Press **Play**.
4. Expect program name in the border/ROM print path, then a quick data load. For `attr_mark.tap`, `RANDOMIZE USR 32768` should paint the top-left attribute.
5. Optional: local Boggit TZX (not in git) — same flow; first header should show `BOGGIT pt1`. Later custom-loader blocks (`flag 0xC8`) need Instant off / EAR (or game’s own loader).

### Experience ~20s load

Abbreviated “feel of loading” that always finishes in about 20 seconds is tracked separately (follow-up to #82); ship Instant + Speed first.

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
when a SwiftUI host steals first responder while the window stays key (it does
**not** inject while the Spectrum view is first responder).
⌘-modified keys clear the matrix and are left alone for menu shortcuts.

**Hold until keyUp:** OS autorepeat (`NSEvent.isARepeat`) is ignored. Spectrum
keeps a matrix key held until `keyUp`; treating repeats as new presses spams 48K
keyword mode (e.g. one short `j` → many `LOAD`s). Duplicate press events while a
key is already held are also ignored so the matrix does not flicker.

**Verify:** after `./scripts/run_macos_app.sh`, typing must **not** appear in Terminal;
after 48K BASIC boots, letters should appear in BASIC; one short `j` → one `LOAD`.

## Joystick (Mac native shell)

Wired to `host_api` (`sc_set_joystick_mode` / `sc_set_joystick` / `sc_clear_joystick`).
Default mode is **Kempston** (0); Sinclair L/R and Cursor modes exist in the ABI for a
later menu.

**GameController (USB / Bluetooth):** each `runFrame` polls `GCController.controllers()`.
For `extendedGamepad`: d-pad and left thumbstick (digital threshold ≈ 0.5) map to
directions; **button A** is fire. Bits match Kempston / `sc_set_joystick`
(0=right, 1=left, 2=down, 3=up, 4=fire). Wireless discovery starts at launch
(`GCController.startWirelessControllerDiscovery`). Pair Bluetooth pads in System
Settings; the staged `.app` Info.plist includes a Bluetooth usage string for
discovery prompts.

**Keyboard Kempston (egui parity):** arrows and **Tab** (fire) OR into the same
joystick mask while still injecting cursor matrix chords (Caps+5/6/7/8).

**Verify:** with a USB/BT pad or arrows+Tab, Kempston-aware software should see
directions/fire on port `0x1F`.

## Frame pacing (~50 Hz)

The shell advances the core with `sc_run_frame` at **~50 Hz wall clock**, matching
egui’s `request_repaint_after(20ms)` throttle:

- `TimelineView` uses a **stable** `PeriodicTimelineSchedule` (not `from: .now` on
  every parent re-render — that reset the schedule and turbo’d the machine).
- `HostBridge.runFrame()` also gates on `ProcessInfo.systemUptime` (~20 ms period,
  small catch-up after hitches) so SwiftUI over-scheduling cannot run unbounded frames.
- `@Published` tape flags are updated only when values change (writing them every
  frame re-entered SwiftUI and worsened the turbo loop).

**Verify:** 48K BASIC cursor blink should look like real hardware / egui (~50 Hz),
not a rapid flicker.

## Liquid glass APIs used

| API | Where | Fallback |
| --- | --- | --- |
| `View.glassEffect(_:in:)` | Toolbar bar, chips, status strip, window wash (`GlassChrome.swift`, `ContentView.swift`) | `.ultraThinMaterial` / gradient |
| `Glass.regular` / `.regular.interactive()` / `.clear` | Same | n/a |
| `#available(macOS 26, *)` | Gates liquid glass | Older macOS materials |

## Not in this slice (still egui-only or follow-ups)

- Joystick mode picker UI (ABI supports Sinclair L/R + Cursor; shell defaults to Kempston)
- Snapshots (SNA/Z80), RZX, DSK
- Signed / notarized SwiftUI distribution bundle (dev launch uses a staged `.app` via `open`)
- CI job that compiles the Swift shell (Linux CI stays Rust-only)

GitHub Releases currently ship an **egui**-wrapped `Spec Chum.app` (see
[RELEASE.md](RELEASE.md)). This SwiftUI shell is not yet a release artifact;
optional CI build + DMG/notarisation: [#68](https://github.com/mward-sudo/spec_chum/issues/68).

## Layout

```
crates/host_api/          # Rust safe session + C ABI (sc_*)
apps/macos/               # SwiftPM executable SpecChumMac (+ staged .app at run)
scripts/build_macos_app.sh
scripts/run_macos_app.sh
```
