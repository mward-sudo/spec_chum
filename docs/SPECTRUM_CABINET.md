# Spectrum Cabinet — cross-platform product specification

**Status:** approved design direction; fixed-room prototype implemented; visual and technical go/no-go pending.
**Product hosts:** SpecChumMac (SwiftUI), Windows shell (Win32), Linux shell (GTK4), and egui fallback.
**Parent issue:** [#557 — Spectrum Cabinet: cross-platform collection-first emulator experience](https://github.com/mward-sudo/spec_chum/issues/557).

## Tracking issues

- [#558 — Prototype a fixed 2D room with a live 3D CRT](https://github.com/mward-sudo/spec_chum/issues/558)
- [#559 — Add shared navigation and System/Light/Dark appearance](https://github.com/mward-sudo/spec_chum/issues/559)
- [#560 — Build a local-first media library browser](https://github.com/mward-sudo/spec_chum/issues/560)
- [#561 — Add accurate illustrated machine and peripheral selection](https://github.com/mward-sudo/spec_chum/issues/561)
- [#562 — Unify the debugger workspace across GUI hosts](https://github.com/mward-sudo/spec_chum/issues/562)
- [#563 — Integrate the approved fixed-camera room on all GUI hosts](https://github.com/mward-sudo/spec_chum/issues/563), gated on #558.

## Product direction

Spectrum Cabinet is a collection-first emulator experience with an optional immersive play room. The library, accurate model imagery, and focused debugger make it easy to find and understand content; the living-room scene gives active play a distinctive identity.

The dark visual direction is the primary art reference. Light appearance is a full peer theme for application chrome, navigation, library, model selection, debugger, and settings. The 3D room artwork may retain its own lighting and palette in either theme; system appearance must not reduce CRT legibility.

### Design principles

- Preserve the ZX Spectrum’s hardware and timing behavior. UI improvements remain host-side.
- Share product behavior and data through `host_api`, `control_plane`, and the machine catalog where appropriate. Keep each host a thin adapter.
- Follow the host OS for menus, title bars, file panels, settings, keyboard modifiers, accessibility, and window behavior. Do not copy one platform’s chrome to the others.
- Make the library local-first and non-destructive. It references user media; it does not copy, rename, or modify ROM, tape, snapshot, disk, or RZX files.
- Use accurate, rights-cleared hardware illustrations. Use project-created or appropriately licensed artwork for any library covers. Do not bundle commercial game covers, game media, or ROMs as UI assets.
- Keep the existing flat Spectrum display available on every host. The room is an optional presentation mode and must not block emulation when unavailable.
- Do not invent controls for unsupported machine behavior. Model/peripheral choices must follow the existing catalog and compatibility metadata.

## Information architecture

| Surface | Purpose | Main content and actions |
| --- | --- | --- |
| **Library** | Find and open media | Recent items, manually added items/locations if implemented, format filters, search, selected-item details, Open/Remove-from-library. Initial implementation should build on the existing recent-path list; no recursive disk scan or cloud service is implied. |
| **Play** | Use the active Spectrum | Flat display or Living Room view, model/status, keyboard/game-controller input, reset, tape transport when a tape is inserted, and a route back to Library. |
| **Machines** | Select a built-in model or saved configuration | Hardware images, model identity, memory/sound summary, ROM readiness, and only compatible attached peripherals. Saved custom configurations remain distinct from built-in models. |
| **Debugger** | Inspect and control execution | Pause/continue, single-step, Z80 registers/flags, PC breakpoints, memory inspection supported by the host API, and structured trace where available. |
| **Settings** | Configure persistent preferences | Appearance, display mode, tape, input, audio, online title lookup, and ROM setup entry points. |

### Media support and existing semantics

- Tape: TAP/TZX. Play remains the EAR path with the selected speed; Experience remains the abbreviated loading experience; Instant remains an action that opens a tape and uses the ROM load path where applicable.
- Snapshot: SNA/Z80.
- RZX: existing RZX playback/open flow and its machine prerequisite.
- +3 disk: DSK on compatible +3 models; it is not a tape and has no Instant action.
- TR-DOS disk: TRD when Beta Disk is attached and the required TR-DOS ROM is available.
- The library classifies by supported format and the active model/hardware constraints. It must not imply that every format is usable on every model.
- Optional online tape-title lookup stays off by default, continues to send only the documented content hash, and is visibly explained. The library must work offline.

## Cross-platform presentation

All hosts expose the same Library, Play, Machines, Debugger, and Settings capabilities and the same media/model semantics. A bounded release may defer an unavailable presentation capability only when the affected host, fallback, and follow-up issue are explicit. The flat Spectrum display is the universal fallback.

| Host | Product surface | Platform-specific presentation |
| --- | --- | --- |
| **macOS** | SpecChumMac, SwiftUI | Standard title bar and app menus; unified toolbar for frequent actions; `NavigationSplitView`-style library navigation; standard Settings scene for preferences; native open/save panels; ⌘ shortcuts. Debugger may use a dedicated resizable window. Preserve the current first-responder behavior so guest typing returns to the Spectrum view after menu/window actions. |
| **Windows** | `windows_shell`, Win32 | Native Win32 menu and common file dialogs; Ctrl shortcuts; native windowing and accessibility. Use a conventional navigation rail/list and resizable client panes rather than imitating macOS materials. |
| **Linux** | `linux_shell`, GTK4 | GTK/desktop conventions, header bar and menus, native file chooser, Ctrl shortcuts, GTK accessibility and system theme. |
| **egui** | `crates/app` | Cross-platform fallback and CI baseline; use the same product navigation, media filters, model information, debugger capabilities, and appearance choice with egui-native panels and opaque chrome. Preserve the normal titlebar and existing hit-test boundaries. |

## Appearance and accessibility

- Provide **System**, **Light**, and **Dark** appearance choices. System follows the OS. Persist explicit Light/Dark choices using shared preference semantics.
- Keep the two themes structurally identical. Theme changes must not alter emulator state, tape behavior, media selection, or room camera state.
- Use each platform’s native control states and focus indicators. Keep keyboard focus visible in library, machine selection, debugger, and settings; guest keyboard input must remain distinct from host shortcuts.
- Respect text scaling, high contrast/accessibility settings, reduced motion, and platform accessibility APIs. Do not rely on color alone for ROM, media, or debugger status.
- Use descriptive accessible names for model and peripheral imagery; include a text/silhouette fallback when an image is missing.

## Hardware imagery and model selection

- Present actual supported models as a visual comparison, using verified project-created renders or rights-cleared images. The labels and hardware facts come from the model catalog rather than artwork metadata.
- Show ROM readiness before switching models when required. Missing or invalid ROMs must keep the current working model active until setup succeeds, consistent with existing host behavior.
- Show only peripherals compatible with the selected base model. Reuse the hardware-compatibility rules and existing saved-configuration behavior; do not create host-only hardware semantics.
- Use image assets as explanatory UI. They do not imply that ROMs, firmware, or physical-device support ship with the app.

## Living Room: 2D art direction with a 3D CRT

The user-selected target is a mostly 2D authored room composition with the live television/CRT kept in 3D. The first implementation step is a comparison prototype, not an assumption that a 2D plate will improve fidelity or performance.

- Keep the existing Bevy renderer and live Spectrum framebuffer path. Preserve the curved phosphor surface, glass, shader, input, and framebuffer-driven CRT effects.
- Use one fixed hero camera position with layered room artwork behind and selectively in front of the 3D television. The first version has no moving intro, user camera, or zoom presets. Add authored light/spill masks around the CRT as needed because static art does not receive the current dynamic room lighting.
- Treat foreground occlusion and screen-to-room color spill as part of the composition. Keep the complete CRT visible at supported aspect ratios.
- The existing multi-preset zoom and moving intro cannot simply be combined with flat camera-mounted plates: the prior approach was disabled after parallax and blank-background problems. Do not add another viewpoint in the first version; the flat display remains available when users want a close, unobstructed picture.
- Keep the current 3D room available for A/B comparison until the new composition passes art, CRT-legibility, resizing, frame-pacing, and input-latency review on supported hosts.
- Preserve a direct route to the flat display if room assets, GPU support, or room initialization are unavailable.
- Scene art, room textures, and hardware renders must have documented rights and sufficient resolution for the intended window sizes.

### Prototype status (#558)

The opt-in Bevy prototype is implemented in `crates/living_room/src/cabinet_room.rs` and documented in [`docs/LIVING_ROOM.md`](LIVING_ROOM.md). `SPEC_CHUM_ROOM_PRESENTATION=fixed-cabinet` selects it in the standalone room; the existing 3D room remains the default. This is not yet a GUI-host presentation mode. Cross-host rollout remains in #563 and is blocked on the review below.

- The prototype uses project-authored flat Bevy rectangles for its wall, trim, and curtains, with the existing Poly Haven CC0 television assets and live curved CRT. No new third-party art is bundled.
- Headless captures cover 320×900, 450×1000, 800×800, 1280×720, 2560×1080, and 320×240. They show the full CRT and test pattern, but are not a substitute for review against the approved concept and current 3D room.
- On this Mac, the same headless 1920×1080, 100-tick diagnostic measured 3.00 ms/tick average (p95 3.32 ms) for fixed mode and 4.17 ms/tick average (p95 4.86 ms) for the existing 3D mode. This measures renderer tick cost only; frame pacing and input latency remain unmeasured.
- A human visual review and explicit technical go/no-go remain required before promoting the room or adding it to GUI hosts. Keep the 3D room selectable and the flat display available until that decision is recorded.

## Delivery sequence

1. **Room feasibility:** define art composition, asset rights, and the single fixed viewpoint; prototype a layered 2D room plus 3D television/CRT. Decide whether the production path should proceed based on actual screenshots and measurements.
2. **Shared product foundation:** navigation, System/Light/Dark appearance, and shared model/media presentation contracts across all four GUI hosts.
3. **Library and machines:** deliver useful recent-media browsing and illustrated model/peripheral selection using real shared data and format/model compatibility.
4. **Debugger and settings:** make the inspected execution workflow discoverable and consistent while retaining native window/settings patterns.
5. **Room production:** only after the feasibility gate, build and integrate final assets, fallback behavior, aspect-ratio handling, and cross-host checks.

## Completion criteria

- The same supported media, model/configuration, input/audio preferences, debugger actions, and flat display are available across all four GUI hosts, subject only to documented platform-specific presentation and explicit bounded deferrals.
- Users can browse/reopen recent supported media locally, see accurate model/peripheral choices, and resolve ROM requirements without losing the active working machine.
- System/Light/Dark appearance works across every host and remains accessible.
- The room prototype keeps the live 3D CRT and Spectrum framebuffer readable at the fixed camera position, survives representative window sizes, and avoids unintended cropping or distortion; final promotion depends on measured frame pacing/input responsiveness and visual review against the approved direction.
- Tests, builds, formatting/lint, graph refresh for code changes, and cross-host review follow the touched crates and project gates. This document is a product spec, not evidence that implementation is complete.
