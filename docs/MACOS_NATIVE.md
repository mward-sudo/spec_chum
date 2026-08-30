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

1. `cargo build -p living_room --release --no-default-features` (staticlib embeds
   `host_api` + headless Bevy; omits standalone Bevy chrome / cpal / `rfd`)
2. Sync `spec_chum_host.h` + `spec_chum_room.h` into the Swift package
3. `swift build -c release` for `apps/macos` (sets `DEVELOPER_DIR` to Xcode if needed;
   `force_load`s `libspec_chum_room.a` — a living_room cdylib next to host panics/blacks)
4. Copy `crates/living_room/assets` → staged app `Contents/Resources/living_room_assets`

Environment:

- `SPEC_CHUM_ROOT` — repo root used to find `roms/` and (fallback) living-room assets
- `SPEC_CHUM_LIVING_ROOM_ASSETS` — optional override for Bevy asset root (set by the staged launcher)
- `SPEC_CHUM_LIVING_ROOM=1` — opt-in: start in living-room mode (`run_macos_app.sh` bakes into the
  staged wrapper only when set; default launch stays flat Spectrum)
- `SPEC_CHUM_ROOM_PERF=1` — opt-in: room/host Hz HUD + stderr (same wrapper bake rule)
- `SPEC_CHUM_AUDIO_DEBUG=1` — AudioQueue init / enqueue / callback stats → stderr, NSLog, and `/tmp/spec-chum-audio.log` (same wrapper bake rule)
- `SPEC_CHUM_AUDIO_CAPTURE=1` — write scheduled PCM to `/tmp/spec-chum-capture.wav` (direct binary launch; not baked by `run_macos_app.sh`)
- `SPEC_CHUM_OPEN_TAPE=/path/to.tap` — on launch, open that tape (+ optional `SPEC_CHUM_AUTO_PLAY_TAPE=1` to Play). **Never** raise macOS system speaker volume or force `specChum.outputVolume` / unmute via `defaults` / `osascript` while testing — leave the user’s mixer alone
- `SPEC_CHUM_INPUT_LATENCY=1` — key→present probe → stderr + `/tmp/spec-input-latency.log` (same wrapper bake rule)
- `DEVELOPER_DIR` — override Xcode path (defaults to `Xcode.app` or `Xcode-beta.app`)
- `SPEC_CHUM_DEBUG=1` / `SPEC_CHUM_TRACE=tape,cpu` — structured tracing (see [`DEBUGGING.md`](DEBUGGING.md)); Mac **Debug** menu dumps the ring

`run_macos_app.sh` launches via **`open`**, so the parent shell environment is **not** forwarded — only vars baked into the staged wrapper at stage time (above) plus `SPEC_CHUM_ROOT` / `SPEC_CHUM_LIVING_ROOM_ASSETS`. For live stderr while debugging audio, run the binary directly:

```bash
SPEC_CHUM_AUDIO_DEBUG=1 apps/macos/.build/release/SpecChumMac
```

(ROMs: run from repo root or set `SPEC_CHUM_ROOT` to the checkout.)

## What this vertical slice includes

- Native `MenuBar` / `Commands` plus **Settings** (⌘,) — HIG split: **toolbar** = frequent (Open / Instant / Play / Rewind / volume / machine picker); **Settings** = EAR speed, volume, joystick, Kempston mouse; **menus** = File Open… counterparts, Tape Type LOAD, **Machine** (built-in models + custom configurations), Hardware Multiface, Debug (no Instant/Play/joystick clones)
- System **`.toolbar`** (SF Symbols: open / Instant; Play–Rewind–EAR speed **only when a tape is inserted**; machine picker; mute + volume slider; reset) + caption status footer; liquid-glass **`glassEffect`** on macOS 26+ for footer/wash, else **`.ultraThinMaterial`**
- **Machine (#187):** toolbar picker and **Machine** menu — built-in models (default ROMs; hardware via **Hardware** menu) and **My configurations** (New / Edit / Delete / Apply). Custom ROM override only in saved profiles (`sc_apply_user_config_json`). No toolbar **Load ROM**.
- Window title reflects media + machine (`attr_mark.tap — 48K`); min size 640×520
- ~50 Hz framebuffer blit (RGBA from Rust) with nearest-neighbor aspect-fit — **default**
- Optional experimental **living room** display (Bevy headless + IOSurface present); toggle in the UI.
  SpecChumMac always links the living_room staticlib; the *mode* defaults **off**.
  See [LIVING_ROOM.md](LIVING_ROOM.md).
- **Open Tape** toolbar / File (⌘O): TAP/TZX via `NSOpenPanel`; on **+3** the control is **Open Tape / Disk** and also accepts `.dsk`. Snapshots / RZX stay separate File items
- **Snapshots / RZX / DSK:** File → **Open Snapshot…** (`.sna` / `.z80`), **Open RZX…**, **Open Disk…** (`.dsk`, **+3 only**) via `sc_load_snapshot` / `sc_load_rzx` / `sc_load_dsk`
- **Instant** toolbar **only** (not duplicated in Tape menu): always opens a TAP/TZX panel, inserts, flash-loads, types `LOAD ""`, then Play. Flash-load restores **off** when the deck stops (or on Pause / Rewind / Play). Instant does **not** offer `.dsk`
- **Type LOAD ""** / **Type LOAD "" CODE** (**Tape** menu only): keyword script via `sc_set_key` (egui `KeyScript` parity); 128K/+3 navigates to **48 BASIC** first (+3 menu **Loader** is disk-only); **+2A** selects tape **Loader** for PROGRAM
- **Hardware:** Multiface 1 attach + NMI (`sc_attach_multiface` / `sc_multiface_nmi`, 48K). Supply your own **8 KiB** Multiface ROM (not shipped — see [MULTIFACE.md](MULTIFACE.md)). Interface 1 attach + MDR (`sc_attach_interface1` / `sc_insert_mdr`); optional IF1 ROM via `sc_load_interface1_rom` or `roms/if1.rom`. DivMMC attach + SD/EEPROM (`sc_attach_divmmc` / `sc_load_divmmc_sd` / `sc_load_divmmc_eeprom`, 48K/128K). Joystick mode is **Settings → Input**. File → **Open TRD…** and Hardware → **Load TR-DOS ROM…** (`sc_load_trd` / `sc_load_trdos_rom`) on 48K/128K. Beta stubs remain egui-first.
- **Audio:** mono PCM from `sc_audio_*` each frame via **AudioQueue** (beeper + EAR mix + AY). Toolbar **mute** + **volume** (0…1) are **host mixer gain only** — they do not change EAR bit fidelity or flash-load. Persisted in `UserDefaults` (`specChum.outputVolume` / `specChum.outputMuted`)
- **Session prefs (#186):** model, EAR speed / Experience, joystick mode, Kempston mouse, recent files, and **custom machine configs** (`specChum.customConfigs`, `specChum.activeConfigId`) persist in `UserDefaults`. Window frame uses `NSWindow` autosave (`SpecChumMainWindow`). Instant / flash-load is never sticky. egui / living_room share a JSON file (`ui-prefs.json`; override with `SPEC_CHUM_PREFS_PATH`)
- **Tape progress:** `ProgressView` from `sc_tape_progress` (shown only when a tape is present)
- Keyboard: app activation + Spectrum `NSView` first responder + `sc_set_key` (see below)
- **Joystick:** `GCController` (USB/Bluetooth) + keyboard Kempston mirror via `sc_set_joystick` (mode in Settings)
- **Kempston mouse:** Settings toggle; Spectrum / living-room pointer delta + buttons via `sc_set_mouse_delta` / `sc_set_mouse_buttons` (egui Machine-menu parity)

## Tape loading (Play / LD-BYTES)

Insert starts **paused** with **flash-load off**. **Play** / **Rewind** / EAR **speed** / progress appear on the toolbar **only while a tape is inserted** (Open and Instant stay available). **Play** always uses the **EAR** path at the toolbar / Settings **EAR speed** (`1x`…`20x`). While Play is active, speed runs that many Spectrum frames per host tick (CPU+ULA+tape stay in lockstep, ROM-accurate pulse widths), so wall-clock ≈ realtime / speed — e.g. Boggit Side 1 pt1 ~333s @1× → ~17s @20× (full Side 1 ~657s → ~33s). Mid-play speed changes apply on the next tick. Host PCM keeps the last inner frame only (no S-second audio burst). **Machine Reset** keeps the inserted tape and +3 disk (tape pauses at its position). **Instant** always re-prompts with a TAP/TZX panel; flash-load restores **off** when the deck stops. On +3 do **not** use menu **Loader** for tape — that is +3DOS disk. On +2A, menu **Loader** *is* tape.

The core **holds** at ROM `LD-BYTES` (`0x056C`) while paused so Play can still arm EAR (or Instant flash-load). Pressing Play after the ROM has already run past that trap used to show a brief border flash (pilot) then stall — that race is fixed.

Standard-speed TZX is converted to TAP for flash-load. **Instant** is a toolbar action: file panel → insert → flash on → Type LOAD `""` → Play. CODE blocks still need **Tape → Type LOAD "" CODE** then **Play** (EAR), or Instant after you are already at LD-BYTES with a CODE loader. ABI: `sc_tape_set_load_options_ex(handle, flash, speed, experience)`.

### Disk UI (minimal — enough for now)

All Spectrum models have EAR/tape hardware; “no tape chrome” means **no tape inserted**, not a disk-only machine. For **+3** disks:

- **Open Tape / Disk** (toolbar) and **File → Open Disk…** insert a `.dsk` via `sc_load_dsk` — no separate Play (the FDC/`+3DOS` Loader owns the transfer).
- **Instant** is tape-oriented (TAP/TZX only). Do not fake Type LOAD flash for disks.
- Optional later: eject / “disk inserted” status. **Do not** build a full +3DOS browser unless it becomes trivial.

### Verify on Mac 48K

1. `./scripts/fetch_roms.sh`
2. `./scripts/build_macos_app.sh` (or let `run_macos_app.sh` build for you)
3. `./scripts/run_macos_app.sh`
4. Model **48K** (Settings). Press toolbar **Instant** — pick `tests/fixtures/tape/print_ok.tap`. Expect Type LOAD then a quick flash-load; flash-load should not stick for a later **Play**.
5. Open a tape via **Open Tape**, Type LOAD `""` (Tape menu), set EAR speed (e.g. 10x), press **Play** — expect border/tones (EAR), not an instant skip.
6. For CODE blocks (`attr_mark.tap` / `minimal_code.tap`): **Type LOAD "" CODE**, then **Play**. `RANDOMIZE USR 32768` should paint the top-left attribute on `attr_mark`.
7. Optional: local Boggit TZX (not in git) — Instant for the PROGRAM header (`BOGGIT pt1`). Later custom-loader blocks (`flag 0xC8`) need EAR (Play / Experience).
8. With no tape inserted: Play / Rewind / speed / progress are hidden; Open + Instant remain.

### Snapshots / RZX / disk

1. **File → Open Snapshot…** — `.sna` / `.z80` (48K or banked 128K/+3; host may switch model and autoload ROM).
2. **File → Open RZX…** — requires a machine/ROM already loaded.
3. **Open Tape / Disk** (toolbar, +3) or **File → Open Disk…** — `.dsk` on **Spectrum +3** only (`sc_load_dsk`; +2A has no floppy). Boot **Loader** / +3DOS — not Instant.
4. **File → Open TRD…** — `.trd` on **48K/128K** (`sc_load_trd`). **Hardware → Load TR-DOS ROM…** for the 16 KiB ROM (`sc_load_trdos_rom`, not fetched by `fetch_roms.sh`). Enter TR-DOS with `RANDOMIZE USR 15616`.

### Experience ~20s load

Toolbar / Settings **Experience** (~20s): abbreviated inter-block pauses on the EAR path at **16×** ([#82](https://github.com/mward-sudo/spec_chum/issues/82)). Same Play / Pause / LD-BYTES hold behaviour as normal EAR; ABI via `sc_tape_set_load_options_ex(..., experience)`.

## Keyboard (Mac native shell)

Mapping matches egui (`crates/app/src/keymap.rs`) and is **tested in Rust**
against ANSI key codes in [`host_api::keymap`](../crates/host_api/src/keymap.rs)
(`SpectrumKeymap.swift` mirrors that table):

| Host | Spectrum |
| --- | --- |
| Letters / digits | Direct matrix |
| Shift | Caps Shift |
| Option / Ctrl | Symbol Shift |
| Arrows | Host joystick only (Kempston port and/or Sinclair/Cursor matrix via `sc_set_joystick`) — **not** duplicate Caps+5/6/7/8 matrix chords |
| Tab | Kempston fire (joystick bit 4) |
| Backspace | Caps + 0 (DELETE) |
| `'` / `"` | Symbol + 7 / Symbol + P |
| `; , . / - = [ ] \\ \`` (+ Shift variants) | Same Symbol layer as egui |

**Keyword entry:** one short press inserts the whole keyword (`j` → `LOAD` with trailing space).
OS autorepeat is ignored; keys stay down until `keyUp` (see below).

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

### Manual verify (48K BASIC)

After `./scripts/run_macos_app.sh` (click the SpecChum window — keys must **not** echo in Terminal):

| Action | Expected |
| --- | --- |
| `p` at `K` cursor | Inserts `PRINT` (keyword + space) |
| Short `j` | One `LOAD` keyword, not repeated |
| Shift+`2` (Quote key) | `"` via Symbol+P, not Caps+quote |
| `'` (Quote alone) | Symbol+7 → `'` |
| Option+`j` | `-` (Symbol layer) |
| Arrow keys (Kempston mode) | Cursor in BASIC **and** Kempston left/right/up/down in games; matrix must not also show Caps+5/6/7/8 |
| Tab (Kempston) | Fire bit on port `0x1F` |
| Backspace | DELETE (Caps+0) |
| Settings → Cursor + arrows | Caps+5/6/7/8 via joystick routing only |
| Tape → Type LOAD `""` | Keyword script; 128K/+3 drops to 48 BASIC first |

Rust regression tests: `cargo test -p host_api keymap` and session joystick/matrix tests in `crates/host_api/src/session.rs`.

## Joystick (Mac native shell)

Wired to `host_api` (`sc_set_joystick_mode` / `sc_set_joystick` / `sc_clear_joystick`).
Default mode is **Kempston** (0). **Settings → Input** exposes Kempston, Sinclair left
(1–5), Sinclair right (6–0), and Cursor — matching the ABI mode values.

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

## Frame pacing (~50 Hz core / display-paced room)

The shell advances the core with `sc_run_frame` at **~50 Hz wall clock**, matching
egui’s `request_repaint_after(20ms)` throttle:

- `HostBridge` owns a **`DispatchSourceTimer`** on the main queue (~20 ms). SwiftUI
  observes `displayTick` to refresh the flat Spectrum view.
- Living-room mode: main only **publishes** Spectrum RGBA; **`CADisplayLink`** paces
  Bevy `sc_room_tick` on `dev.specchum.living-room` at monitor refresh (IOSurface →
  linear `CALayer`; coalesce if a tick is in flight). See [LIVING_ROOM.md](LIVING_ROOM.md).
- `HostBridge.runFrame()` also gates on `ProcessInfo.systemUptime` (~20 ms period,
  small catch-up after hitches) so timer jitter cannot run unbounded frames.
- `@Published` tape flags are updated only when values change (writing them every
  frame re-entered SwiftUI and worsened turbo loops historically).

**Verify:** 48K BASIC cursor blink should look like real hardware / egui (~50 Hz),
not a rapid flicker. With `SPEC_CHUM_ROOM_PERF=1`, HUD `specHz` ≈ 50 and `roomHz`
tracks the display when living room is on.

## UI conventions (HIG-oriented)

- **Menus:** File Open… (ellipsis + separators); app menus Tape / Machine / Hardware / Debug; View → Show Inspector; Help → Spec Chum Help. ⌘-modified shortcuts only — they clear the matrix so Spectrum typing is not stolen when the display is focused.
- **Toolbar:** semantic placements + SF Symbols (Open Tape[/Disk], Instant, Play/Pause, Rewind, EAR speed, Reset); accessibility labels on icon-only controls; after chrome actions, focus returns to the Spectrum `NSView`.
- **Settings:** EAR speed, model, joystick mode live in the Settings scene (Instant is a toolbar/Tape **action** that always opens a file panel, not a sticky checkbox).
- **Focus:** do not add decorative SwiftUI `.focusable()` rings; keep `NSApp.activate` + first responder (see Keyboard section).

## Liquid glass APIs used

| API | Where | Fallback |
| --- | --- | --- |
| `View.glassEffect(_:in:)` | Status footer, window wash (`GlassChrome.swift`, `ContentView.swift`) | `.ultraThinMaterial` / gradient |
| `Glass.regular` / `.clear` | Same | n/a |
| `#available(macOS 26, *)` | Gates liquid glass | Older macOS materials |

## Tape load modes (egui + macOS)

| Mode | Behaviour |
| --- | --- |
| **Play** (default) | Realtime EAR bitstream; optional **1×–20×** speed multiplier |
| **Experience (~20s)** | Abbreviated inter-block pauses on the EAR path at **16×** ([#82](https://github.com/mward-sudo/spec_chum/issues/82)) |
| **Instant** | Flash-load at LD-BYTES (Type LOAD + Play); no EAR timing |

Pause / Play hold at LD-BYTES works in Experience mode.

## Not in this slice (still egui-only or follow-ups)

- Signed / notarized SwiftUI distribution bundle (dev launch uses a staged `.app` via `open`)
- Beta UI on macOS (egui Hardware stubs first); IF1 + MDR and DivMMC SD/EEPROM attach are in the native Hardware menu

## CI (`macos-shell` / `living-room` — [#68](https://github.com/mward-sudo/spec_chum/issues/68) / [#146](https://github.com/mward-sudo/spec_chum/issues/146))

Workflow [`.github/workflows/ci.yml`](../.github/workflows/ci.yml):

- **`macos-shell`** on `macos-latest`: `./scripts/build_macos_app.sh` (living_room
  staticlib + SwiftPM). Independent of Linux `check`.
- **`living-room`** on `macos-latest`: `./scripts/check_living_room.sh` (fmt/clippy/test/perf).

GitHub Releases currently ship an **egui**-wrapped `Spec Chum.app` (see
[RELEASE.md](RELEASE.md)). This SwiftUI shell is not yet a release artifact;
DMG/notarisation remain follow-ups under [#68](https://github.com/mward-sudo/spec_chum/issues/68).

## Layout

```
crates/host_api/          # Rust safe session + C ABI (sc_*)
crates/living_room/       # Headless Bevy embed + standalone spec-chum-room (#146)
apps/macos/               # SwiftPM executable SpecChumMac (+ staged .app at run)
scripts/build_macos_app.sh
scripts/run_macos_app.sh
```
