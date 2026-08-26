# UI architecture

How Spec Chum should present the emulator to users, and why we keep the current stack for now.

## Recommendation (default)

**Keep Rust core + egui/eframe as the primary cross-platform host.** Polish carefully, and avoid fake “liquid glass” / fullsize content under the macOS titlebar.

Spec Chum’s scarce resource is **hardware accuracy** (Z80, ULA, tape), not UI novelty. egui already ships framebuffer blit, menus, file dialogs (`rfd`), and headless `Context::run` tests.

**Optional on macOS:** a native SwiftUI shell with real liquid glass / materials, driven by the `host_api` C ABI — see [MACOS_NATIVE.md](MACOS_NATIVE.md) and [#66](https://github.com/mward-sudo/spec_chum/issues/66). This does **not** replace egui on Linux/Windows/CI.

**Optional later:** a thin **libretro / RetroArch** core once the machine host API is stable (`run_frame`, framebuffer, audio batch, input inject, optional serialize). Tracked in [#64](https://github.com/mward-sudo/spec_chum/issues/64). The same `crates/host_api` surface is a useful stepping stone.

**Optional experimental:** a Bevy 3D living-room CRT host (`crates/living_room`, binary `spec-chum-room`) — dark UK 1980s room, framebuffer on a physical tube. Not the default product UI; excluded from the default Linux CI/check gate unless `SPEC_CHUM_CHECK_LIVING_ROOM=1` (macOS `living-room` job runs `check_living_room.sh`). SpecChumMac **always links** the living_room staticlib (embeds `host_api`); the living-room *display* defaults **off**. See [LIVING_ROOM.md](LIVING_ROOM.md) and [#146](https://github.com/mward-sudo/spec_chum/issues/146).

## Comparison (emulator frontends)

| Option | Framebuffer | Audio | Input latency | Menus / dialogs | macOS polish | Fit for Spec Chum |
| --- | --- | --- | --- | --- | --- | --- |
| **egui / eframe** (current) | Texture upload / `ColorImage` — good | Via `cpal` beside egui — fine | Immediate-mode; low if we avoid heavy UI work per frame | Custom menus + `rfd`; not native NSMenu | “OK” if we stay below the titlebar; fake glass under traffic lights was a mistake | **Best default** |
| **iced** | Solid wgpu path | Same `cpal` pattern | Elm-style; good if messages stay thin | Custom widgets; dialogs via crates | Cleaner declarative layout; still not AppKit | Strong alt if we outgrow immediate mode; migration cost high |
| **dioxus-desktop** | WebView or custom; blit often via canvas/WebGL | WebAudio or native bridge | Extra hop through web stack | HTML/CSS familiar; file dialogs possible | Can look native-ish with CSS; not true Cocoa | Weak for cycle-accurate emu (latency, GPU path complexity) |
| **Slint** | Designed for embedded UIs; can host images | External | Good | Declarative; tooling strong | Polished widgets; still not NSMenu | Possible later; DSL + tooling tax for a small team |
| **Tauri + web** | Canvas/WebGL blit | WebAudio or plugin | IPC + browser — usually worse | Excellent HTML/CSS menus; native dialogs | Can look great with CSS | Overkill; process split hurts hot-path simplicity |
| **gpui** (Zed) | GPU-first, powerful | Possible but immature ecosystem for apps like ours | Excellent in editor context | Custom | Very Mac-capable in Zed’s hands | High churn / API risk; not aimed at emulators |
| **Rust core + native shells** (SwiftUI / WinUI / GTK) | Best-in-class per OS if we invest | Best-in-class per OS | Best if wired carefully | True native menus | Best polish | Ideal “product” endgame; **3× UI maintenance** for a small team |
| **libretro / RetroArch core** | Software blit via `retro_video_refresh` (or HW FBO) | `retro_audio_sample` / batch | Frontend polls; core maps joypad/keyboard each `retro_run` | RetroArch menus / overlays — not Spec Chum UX | Whatever RA provides per platform | **Optional backend later**; accuracy stays in Spec Chum |

### Tradeoffs that matter here

1. **Framebuffer blit** — We need ~50 Hz 352×296 (or similar) RGBA uploads. egui textures and iced/wgpu are both fine. WebView paths add copies and vsync quirks. Libretro expects one video callback per `retro_run` (typically one emulated frame).
2. **Audio** — Beeper/AY already run through `cpal` in the egui app. UI crate choice should not own the audio thread; keep that separation. A libretro core would push PCM via the frontend’s audio callback instead of `cpal`.
3. **Input latency** — Matrix keyboard + Kempston want raw key edges each frame. Immediate-mode egui is fine; avoid web IPC for keys. Libretro uses poll + state queries (and optional keyboard callbacks); mapping Spectrum keys/joypad is core work.
4. **Menus / file dialogs** — Native NSMenu would fix “feel,” but egui menus work if hit zones are real (normal titlebar, opaque panels). `rfd` covers Open dialogs. Libretro defers almost all chrome to RetroArch (content load, shaders, overlays).
5. **macOS polish** — Fullsize content view + translucent panels looked clever and broke clicking (#60). Prefer boring, opaque chrome under a normal titlebar.

## libretro / RetroArch (optional backend)

A libretro core is a **dynamic library** (`cdylib`) that implements the C ABI in `libretro.h`. RetroArch (or another frontend) loads it and owns windowing, audio devices, controllers, shaders, and most UX.

### API surface (what a Spectrum core must do)

| Area | Libretro hooks | Spec Chum mapping (sketch) |
| --- | --- | --- |
| Lifecycle | `retro_init` / `retro_deinit`, `retro_load_game` / `unload` | Construct `machine::Machine`, load ROM + optional TAP/SNA/etc. |
| Run | `retro_run` — one video frame of work; poll input ≥1×; video callback exactly once | `Machine::run_frame` then `render_rgba` + audio batch |
| Video | `retro_video_refresh` (software) or HW render FBO | Export ULA framebuffer (agreed pixel format / geometry) |
| Audio | sample or batch (`int16` stereo) | Map `FrameAudio` / beeper+AY to PCM at negotiated rate |
| Input | `retro_input_poll` + `retro_input_state` (joypad, keyboard, …) | Drive `keyboard_mut` / Kempston |
| Save / netplay | `retro_serialize_size` / `serialize` / `unserialize` (optional but valuable) | Snapshot of machine state (not yet a single stable blob API) |
| Options | `RETRO_ENVIRONMENT_SET_CORE_OPTIONS` | Model, border, flash-load, etc. |

Rust can wrap the ABI with crates such as [`libretro-core`](https://docs.rs/libretro-core/) when we implement; the accuracy core remains `z80` / `ula` / `machine`.

### Pros

- RetroArch frontend polish: shaders, overlays, multi-platform packaging users already know
- Controller / input ecosystem and (with serialize) rewind / netplay hooks
- Forces a clean **host API** boundary: frame step, pixels, PCM, input inject

### Cons

- Hard dependency on RetroArch (or another libretro frontend) for that distribution path
- Limited custom Spec Chum UX (tape Play semantics, model UI, etc. become core options / RA menus)
- Packaging as a platform-specific dynamic library; ABI / env callback constraints
- Harder interactive debugging than a standalone egui binary
- Does not replace ownership of accuracy work inside Spec Chum

### Recommendation for Spec Chum

1. **Primary host:** keep egui/eframe.
2. **Backlog:** optional libretro core after the machine host API is stable — see [#64](https://github.com/mward-sudo/spec_chum/issues/64).
3. **Do not** treat RetroArch as the default product shell; treat it as an alternate distribution channel once `run_frame` + framebuffer + audio + input (and ideally serialize) are intentional public host surfaces.

## Decision record

| Choice | Rationale |
| --- | --- |
| Stay on **egui/eframe** as default | Already integrated; testable headless; matches small-team bandwidth |
| Optional **SwiftUI macOS shell** | Native menus + liquid glass via `host_api`; see [MACOS_NATIVE.md](MACOS_NATIVE.md) / #66 — does not replace egui |
| No fullsize content under titlebar (egui) | Hit-testing and traffic lights must stay out of the menu strip |
| Opaque egui panels | Predictable contrast and clicks; drop fake glass in egui |
| Native multi-shell beyond macOS later | Only if packaging/a11y/menus become product-critical |
| Avoid Tauri/Dioxus-web for the machine loop | Extra process/IPC is the wrong complexity for a Spectrum core |
| **libretro later, not now** | RA ecosystem is attractive; defer until host API is stable; track in #64 |
| Optional **Bevy living-room** | Experimental immersion host; SpecChumMac links staticlib, display opt-in; keep out of default CI; #146 |

## Practical chrome rules (current shell)

- Normal window titlebar (do not draw the menu under traffic lights).
- Opaque top menu strip with a minimum clickable height.
- Status text for tape play/pause; Play must arm EAR + flash-load (`playing == true`).
- Prefer `rfd` for file picks; keep ROM/tape/disk logic in `machine` / `tape` / `formats`.
