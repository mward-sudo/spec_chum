# Experimental Bevy living-room CRT host

**Status:** experimental / not the default product UI. Tracked in
[#146](https://github.com/mward-sudo/spec_chum/issues/146).

A third host surface (alongside egui and the SwiftUI macOS shell): a dark, small
UK 1980s living room whose CRT is the only light. The Spectrum framebuffer is
uploaded onto a bulging phosphor mesh. CRT filters use open crt-aperture /
crt-easymode techniques; Retro Virtual Machine’s UK-TV look is a **visual
reference** only (not a copied pipeline). egui remains the primary shell — see
[UI_ARCHITECTURE.md](UI_ARCHITECTURE.md).

There are **two delivery paths**:

| Path | Role |
| --- | --- |
| Standalone `spec-chum-room` | Dev harness (Bevy chrome / cpal / `rfd`). Linux/Windows iteration. |
| SpecChumMac embed | Canonical Mac chrome. Headless Bevy display-only; `host_api` owns the Spectrum. **Default off** (Settings / toolbar toggle). |

## Requirements

- Rust toolchain + GPU with wgpu support
- Fetched system ROMs: `./scripts/fetch_roms.sh`
- Poly Haven CC0 assets (optional if already vendored):

```bash
./scripts/fetch_living_room_assets.sh
```

## Run (standalone)

```bash
./scripts/fetch_roms.sh
./scripts/fetch_living_room_assets.sh   # once
cargo run -p living_room --release
```

Binary name: `spec-chum-room` (requires the default `standalone` feature).
Launch from a normal Terminal with WindowServer — headless agent shells often die in ~5s
(monitor scale factor 0 / no display attachment).

## macOS SwiftUI embed

Build/run the native shell ([MACOS_NATIVE.md](MACOS_NATIVE.md)):

```bash
./scripts/build_macos_app.sh
./scripts/run_macos_app.sh
```

- SpecChumMac builds `living_room` as a **staticlib** with `--no-default-features`
  (no Bevy UI / cpal / `rfd`) and `force_load`s `libspec_chum_room.a` (embeds `host_api`).
  The macOS shell **always links** living_room for the host ABI; living-room is opt-in only
  as a *display mode*. A separate `cdylib` next to `host_api` panics / paints black — do not resurrect it.
- Living room display is **opt-in** (`livingRoomMode` defaults false). Flat Spectrum blit remains the default.
- Present path: Bevy renders offscreen → GPU blit into a shared **IOSurface** → `CALayer.contents`
  (no per-frame CPU `CGImage` readback). Default render size is **1920×1080** (`DEFAULT_ROOM_W/H`);
  Swift steps the long edge up to a **2560** backing-pixel cap (aspect-matched to the view).
  Layer filters are **linear** (phosphor texels stay nearest).
- **Dual clocks:** Spectrum `DispatchSourceTimer` ~50 Hz on AppKit main publishes RGBA;
  **`CADisplayLink`** paces `sc_room_tick` on `dev.specchum.living-room` at monitor refresh
  (coalesce if a tick is in flight). Zoom/skip are cheap mutations (no forced Bevy frame).
- Bevy `Time` uses display delta (`sc_room_set_frame_delta_seconds`), not fixed 1/50.
- Assets resolve at runtime (see below) and are copied into
  `SpecChumMac.app/Contents/Resources/living_room_assets`.

### Perf telemetry

```bash
SPEC_CHUM_ROOM_PERF=1 SPEC_CHUM_LIVING_ROOM=1 ./scripts/run_macos_app.sh
```

- Rust: rolling Bevy tick µs (`sc_room_perf_snapshot`); stderr ~1 Hz when env set.
- Swift HUD: host ms, **roomHz** / **specHz**, skip-busy, present WxH, bevy last/avg/max.
  `thread_hint` 1=AppKit main, 2=room queue. Expect roomHz ≫ 50 on ProMotion when healthy.

### Embed architecture (notes)

Apple’s recommended SwiftUI path for live 3D is **`NSViewRepresentable` → `MTKView` / `CAMetalLayer`**, with the drawable obtained late and presented via the Metal command buffer (WWDC / MetalKit). Apple’s custom Metal view sample also shows **display-paced render on a background thread** so UI stays responsive. Putting Metal work on the AppKit main thread tends to hitch input when SwiftUI also runs there.

Bevy’s supported embed pattern is **headless `SubApps`**: disable `WinitPlugin`, `RenderTarget::Image`, never `App::run()` — pump `update()` yourself (Bevy “externally driven headless” examples). Presenting into SwiftUI is **not** a Bevy feature: peers use **IOSurface → Metal → wgpu-hal** interop, with blits inside Bevy’s render graph (out-of-band `queue.submit` races ordered submit).

**SpecChumMac (Phases 0–3):** headless Bevy + Image target; Spectrum 50 Hz timer only publishes FB; `CADisplayLink` paces `sc_room_tick` on `dev.specchum.living-room`; IOSurface → linear `CALayer` present. Phase 4 may replace IOSurface with a `CAMetalLayer` drawable. Spectrum + keys stay on main.

### Asset root resolution

1. `SPEC_CHUM_LIVING_ROOM_ASSETS`
2. `SPEC_CHUM_ROOT/crates/living_room/assets`
3. `Contents/Resources/living_room_assets` next to the executable
4. `CARGO_MANIFEST_DIR/assets` (dev `cargo run`)

## Dual-clock embed plan

**Status:** **implemented** for SpecChumMac (Phases 0–3). Not research-only.

| Done | Work |
| --- | --- |
| **0** | Perf HUD / `sample` baseline (`tmp-perf-capture/sample-dualclock.txt`) |
| **1** | Decouple clocks: main publishes FB only; `CADisplayLink` paces `sc_room_tick` |
| **2** | Linear `CALayer` filters + aspect-matched present (2560 cap) |
| **3** | Display delta via `sc_room_set_frame_delta_seconds`; zoom/skip = cheap mutations |

Phase 4 (`CAMetalLayer` drawable) and Phase 5 (pipelined Bevy) remain optional follow-ups.
Refs [#146](https://github.com/mward-sudo/spec_chum/issues/146).

### Automated test coverage gap

`./scripts/check_living_room.sh` runs `cargo fmt/clippy/test -p living_room` and the
**headless** `room_perf` example. It times the **shipping present path** (`SimulatePresentPath` —
no blocking CPU readback). It does **not** exercise:

- SpecChumMac Swift embed (`LivingRoomDisplayView`, `CADisplayLink`, `CALayer.contents`)
- IOSurface GPU blit → Core Animation present (the path that regressed with stale frames)
- Keyboard/scroll input latency through AppKit

Manual smoke: build with `./scripts/build_macos_app.sh`, enable living room, type/scroll
**without** switching apps; optional `SPEC_CHUM_ROOM_PERF=1 SPEC_CHUM_INPUT_LATENCY=1`.

**Problem (historical, pre dual-clock):** Off-main `sc_room_tick` fixed AppKit blocking, but
room ticks were still slave to the Spectrum ~50 Hz timer (`runFrame` → enqueue tick), and
present used nearest full-frame upscale of a fixed 960×540 IOSurface.

### Why coalesce-from-50 Hz could not hit refresh rate (fixed)

Previous path (`HostBridge.runFrame` → enqueue `sc_room_set_framebuffer` + `sc_room_tick`):

1. `DispatchSourceTimer` on **main** fired ~50 Hz → `sc_run_frame` + copy RGBA.
2. Enqueued set_fb + tick on `dev.specchum.living-room` with **coalesce** (`roomTickInFlight`).
3. After tick, main set `CALayer.contents` to the same IOSurface.

Consequences (all addressed by Phases 1–2):

- Bevy updated **at most ~50 Hz**, even on ProMotion 120 Hz.
- If a Bevy tick exceeded ~20 ms, coalesce dropped room frames; zoom waited on the same queue.
- Present locked at **960×540** with **nearest** magnification → crunchy Retina upscale of the 3D frame.

Spectrum ULA frames and display refresh are **different clocks**. Coupling them under-samples
the display and over-couples camera to emulator cadence — hence dual-clock.

### Recommended architecture (dual clock)

```text
┌─────────────────────────────────────────────────────────────────────────┐
│ AppKit main                                                             │
│  • DispatchSourceTimer ~50 Hz (ULA / HostSession)                       │
│  • sc_run_frame + audio + keys / Kempston                               │
│  • Publish latest Spectrum RGBA → shared slot (triple-buffer / swap)    │
│  • Never call sc_room_tick here                                         │
└───────────────────────────────┬─────────────────────────────────────────┘
                                │ latest FB (lock-free or mutex slot)
                                ▼
┌─────────────────────────────────────────────────────────────────────────┐
│ Display pace (preferred)                                                │
│  CAMetalDisplayLink on CAMetalLayer  OR  MTKView.preferredFramesPerSecond│
│  • Callback / draw on room serial queue (or dedicated render run loop)  │
│  • sc_room_tick / SubApps::update @ display rate (60 / 120 / ProMotion) │
│  • Sample latest Spectrum FB (may be same FB for 2–3 display frames)    │
│  • Present: IOSurface blit  OR  late nextDrawable → Metal present       │
└─────────────────────────────────────────────────────────────────────────┘

Shared present target: IOSurface → CALayer (interim)  OR  CAMetalLayer drawable
Spectrum clock ≠ Bevy Time: use wall / display delta for camera; keep phosphor
upload event-driven from the 50 Hz publish (not from display rate).
```

**Clock mapping (game-engine dual tick):**

| Domain | Rate | Owner | Bevy mapping |
| --- | --- | --- | --- |
| Emulator / “simulation” | ~50 Hz ULA | Main `HostSession` | `ExternalFramebuffer` publish only — **not** `TimeUpdateStrategy` |
| Room render | Display refresh | DisplayLink / MTKView | `SubApps::update` + render extract/blit |
| Camera / CRT look blend | Display Δt | Room tick | `sc_room_set_frame_delta_seconds` + zoom snap; no longer ManualDuration 1/50 |

### CRT anti-alias / resize policy

Spectrum framebuffer is **`SCREEN_W`×`SCREEN_H` = 352×296** (`crates/living_room/src/crt.rs`). Policy:

| Stage | Filter / size | Rationale |
| --- | --- | --- |
| Phosphor upload texture | **Nearest** (already) | Preserve 8×8 Spectrum glyphs; shader snaps with `textureLoad` |
| Phosphor shader | Soft-H mix + sharp V; scan/grille via `CrtLookBlend` | CRT look at sofa distance; near-zero scan at CRT-fill zoom |
| 3D room render target | **1920×1080** default; Swift caps long edge at **2560**, aspect-matched to view | Geometry needs pixels; CALayer linear-upscales when the window is larger |
| Room MSAA | Optional **MSAA on room camera only** | Smooth bezel/furniture edges; phosphor mesh still samples nearest texels |
| Phosphor mipmaps | **Off** for FB texture | Mips blur glyphs; CRT mesh is close to screen-facing |
| Final present to window | **Linear / bilinear** CALayer (done) or Metal sampler | Upscale of the **composited 3D frame** must not be nearest |
| Flat Spectrum mode (non-room) | Unchanged product path | Out of scope for this plan |

**Fixed in Phase 2:** nearest-upscale of the entire room IOSurface was wrong — that AA policy
belongs on the **CRT texel**, not the **3D present**. Layer filters are now **linear**; phosphor
sampling stays nearest in Bevy.

### Apple present / pacing (macOS 14+; SpecChumMac already `.macOS(.v14)`)

**Shipped interim (Phases 1–2):** IOSurface → linear `CALayer.contents`, paced by
`CADisplayLink` → room queue `sc_room_tick`.

Preferred order for a later drawable migration (Phase 4):

1. **`CAMetalLayer` + `CAMetalDisplayLink`** — Metal-native, `preferredFrameRateRange` / latency; Apple’s path for variable refresh. Drive Bevy tick from the delegate on the **room queue**, not AppKit main. Complete GPU work before drawable `present()`.
2. **`MTKView` + `preferredFramesPerSecond`** — faster to embed via `NSViewRepresentable`; watch SwiftUI stutter when `currentDrawable` blocks on main — keep draw off-main or set `presentsWithTransaction` thoughtfully.
3. **`CVDisplayLink`** — legacy; avoid for new work on macOS 14+ (deprecated / less expressive).

Do **not** set `presentsWithTransaction` without measuring — it helps some SwiftUI+Metal hitch cases and hurts others (transaction sync with Core Animation).

### Input latency vs coalesce

**Implemented:**

- Keys: `LivingRoomNSView` → `host_api` on **main** (not gated on Bevy).
- Zoom / skip intro: `livingRoomQueue.async` → `sc_room_nudge_zoom` / `sc_room_skip_intro` only — **no forced `sc_room_tick`**; DisplayLink presents the new pose.
- Coalesce: at most one room tick in flight; “latest wins” for FB publish; never unbounded tick queues.
- On Bevy overrun: drop/coalesce **room** frames; never skip or delay Spectrum `sc_run_frame`.

### Bevy 0.19 headless + pipelined rendering

**Current embed (`headless.rs`) after dual-clock:**

- `SubApps` + disable `WinitPlugin` + disable **`PipelinedRenderingPlugin`**
- `TimeUpdateStrategy::ManualDuration` advanced each tick from **display delta**
  (`sc_room_set_frame_delta_seconds`) — not Spectrum 1/50
- Present: GPU blit into IOSurface (`present.rs` / `present_metal.rs`); `PollType::Poll` when presenting

**Pipelined rendering (Phase 5, optional):** Bevy moves the render schedule to a dedicated
thread so frame N render overlaps frame N+1 sim. Attractive now that DisplayLink owns pacing,
but risks:

- Manual `SubApps::update` + extract channels assume Bevy’s runner lifecycle; headless + force-load staticlib may deadlock or double-own the render thread.
- Present blit must stay inside Bevy’s ordered submit (already true); out-of-band Metal present races.

**Recommendation:** keep pipelined **disabled**. A future spike may add
`SPEC_CHUM_ROOM_PIPELINE=1` (not implemented yet). Display-rate pumping of
non-pipelined `update()` already unlocks 60/120 Hz present.

### Phases

| Phase | Status | Work | Risks |
| --- | --- | --- | --- |
| **0 — Measure** | **Done** | Baseline with `SPEC_CHUM_ROOM_PERF=1`; `sample-dualclock.txt` — main idle of `sc_room_tick`, room queue holds Bevy. | Misreading pre–dual-clock samples |
| **1 — Decouple clocks** | **Done** | `runFrame` publishes FB only. `CADisplayLink` → room queue tick. Latest-slot upload at tick start. | Stale FB if publish races (gen slot handles) |
| **2 — Present filter + res** | **Done** | CALayer **linear** filters; aspect-matched present up to **2560** long-edge (default **1920×1080**). | Resize recreate cost on debounced window changes |
| **3 — CRT look / Δt** | **Done** | Display delta for Bevy `Time`; zoom/skip without forced tick; phosphor nearest / present linear. | Over-softening if phosphor sampler flipped to linear |
| **4 — Drawable path (optional)** | Open | `CAMetalLayer` + DisplayLink; blit or texture into drawable; retire IOSurface if redundant. | wgpu-hal / MTLDevice identity; drawable timeout under SwiftUI load |
| **5 — Pipelined spike (optional)** | Open | Re-enable `PipelinedRenderingPlugin` behind env; soak test zoom + tape load. | Deadlock / frame pacing regression — ship only if green |

### What NOT to do

- Drive Spectrum / `sc_run_frame` at 120 Hz (or display rate) — wrong for ULA timing and audio.
- Run Bevy `sc_room_tick` on AppKit main (regresses input hitch; prior sample evidence).
- Nearest-upscale the **full window** / full 3D present.
- Bind Bevy render size 1:1 to Retina backing without a budget (create/resize stalls).
- Resurrect living_room `cdylib` next to `host_api`.
- Use `@Published displayTick` spam for room present.
- Block main on `livingRoomQueue.sync` except create/destroy/bind.
- Tie Bevy animation time permanently to ManualDuration 1/50 once display-paced.

### Acceptance criteria (user goals)

1. **Responsive chrome / zoom / keys:** Zoom and skip-intro feel immediate; keys never wait on Bevy. Main thread does not block on room render.
2. **CRT scales correctly:** Close zoom — sharp Spectrum text; pulled-back — intentional CRT filter, not pixelated 3D upscale. Window resize / Retina: bilinear present (or native-res render), not nearest full-frame scale.
3. **3D at display pace:** Room present Hz tracks monitor refresh (within OS limits; ProMotion may choose 80/120 via `preferredFrameRateRange`). Telemetry shows room ticks ≫ 50 when display > 50.
4. **Emulator at ULA pace:** Spectrum remains ~50 Hz wall-clock; not locked to display; not starved when Bevy is slow (drop room frames, never skip `sc_run_frame` for Bevy).
5. **Perf gate:** With `SPEC_CHUM_ROOM_PERF=1`, host frame stays low-ms; room skip-busy only under GPU overload; document target budgets in HUD.

### Files (Phases 0–3 landed; Phase 4+ optional)

| Area | Files | Notes |
| --- | --- | --- |
| Dual clock / DisplayLink | `HostBridge.swift`, `LivingRoomDisplayView.swift` | Done — DisplayLink + FB publish slot |
| Layer filter / size | `LivingRoomDisplayView.swift`; `sc_room_resize` | Done — linear + stepped sizes |
| Bevy time / tick API | `headless.rs`, `ffi.rs`, both `spec_chum_room.h` | Done — `sc_room_set_frame_delta_seconds` |
| Present | `present.rs`, `present_metal.rs` | IOSurface blit shipped; Phase 4 = drawable |
| CRT sampling | `crt.rs`, `crt_phosphor.wgsl`, `camera.rs` | Policy mostly present-side (done) |
| Docs / scripts | this file; `MACOS_NATIVE.md`; `run_macos_app.sh` | Opt-in `SPEC_CHUM_*` wrapper bake |

### Remaining responsiveness risks (after dual-clock)

Dual-clock fixes **present cadence** and **Bevy-vs-ULA starvation**. It does **not** by itself fix:

- **SwiftUI chrome** overdraw / glass toolbar layout passes.
- **Asset streaming** / first-frame shader compile (async compile already on; cold toggle can still hitch).
- **Bloom cost at 120 Hz** on very large windows (dynamic quality knobs exist; see below).
- **Serial-queue saturation** if a single tick > frame interval (still need frame skip).
- **Trackpad zoom discrete presets** (feel “sticky” even when render is fast). Preset
  transitions use **ease-out cubic** over **0.20 s** (`ZOOM_ANIM_SECS`); one step per
  ~48 px scroll (`SCROLL_PIXELS_PER_STEP` in standalone Bevy).

## Controls (standalone)

Host chrome (opaque top/bottom bars — not diegetic HUD in the room):

| Key | Action |
| --- | --- |
| (intro) Click / Space | Skip camera dolly |
| After lock: keyboard | Spectrum matrix (same chords as egui) |
| Toolbar **Open** / ⌘O | Open TAP / TZX / SNA / Z80 (`rfd`) |
| Toolbar **Play** | Play tape (EAR / realtime) |
| Toolbar **Instant** | Instant flash-load + play |
| Toolbar **Mute** | Mute host audio |
| Toolbar **Pause** / ⌥⌘P / Esc | Pause overlay |
| Toolbar **Reset** / ⌘R | Reset (re-select current model) |
| Toolbar **48K** / **128K** / **+3** · `F1`/`F2`/`F3` | Select model |

### Model selection (no desync)

Every model change goes through `EmulatorHost::select_model` →
`HostSession::select_model` (`set_model` + `try_autoload_rom`). Status text is
prefixed with the selected label (`48K` / `128K` / `+2A/+3`), which always
matches `session.model()`. Loading a 48K SNA/Z80 via `O` forces Spectrum 48K
and reloads the matching ROM inside `host_api` (same as egui’s snapshot path
intent). Digits `1`–`3` are host chrome only when paused / pre-lock so BASIC
number entry cannot wipe the machine; use `F1`–`F3` anytime.

## Performance

### Benchmark artifact (fixed)

Early perf work timed `HeadlessRoom::tick()` **with** a blocking CPU readback
(`copy_frame_rgba` → `map_async` + multi-megabyte BGRA copy every frame). That
dominated the sample and made bloom mips, MSAA, and resolution look ruinously
expensive — the numbers drove incorrect quality cuts (256 bloom mips, 960/1280 caps).

The **`room_perf`** example now warms with one readback pass, then switches to
[`SimulatePresentPath`](../crates/living_room/src/present.rs) — the same
non-blocking poll path as the IOSurface embed (no CPU map). See
`crates/living_room/examples/room_perf.rs`.

### Real numbers (M4 Air, present path)

With shipping defaults (full 3D, bloom mips **512**, MSAA **×4**, hybrid **off**):

| Present size | Tick-only (present path) |
| --- | --- |
| 1920×1080 | ~4–7 ms |
| 2560×1440 | ~4–7 ms |

**60 Hz is achievable** at default quality without hybrid plates or aggressive cuts.
ProMotion 120 Hz needs ≤8 ms average — tune with `SPEC_CHUM_ROOM_*` if needed.

Measure locally:

```bash
cargo run -p living_room --example room_perf --release
cargo run -p living_room --example room_perf --release -- 2560 1440
```

Bloom cost is mostly **per-pass CPU overhead**, not raw GPU fill — lowering mips
helps less than the old readback benchmark suggested.

### Headless probes vs live app

| Tool | Path | Caveat |
| --- | --- | --- |
| `room_perf` | Present-path tick timing (no readback in timed phase) | CI gate; not IOSurface |
| `room_probe` | CPU readback → PPM | Frames can look **darker** than SpecChumMac — readback artifact, **not a live bug** |
| SpecChumMac | IOSurface GPU blit → linear `CALayer` | Canonical look |

The live embed does **not** have the “black room” problem reported from headless
readback probes.

## Quality knobs

Runtime A/B via `SPEC_CHUM_ROOM_*` (implemented in `crates/living_room/src/quality.rs`):

| Variable | Default | Effect |
| --- | --- | --- |
| `SPEC_CHUM_ROOM_BLOOM` | on | Bevy bloom (CRT halation). `=0` disables. |
| `SPEC_CHUM_ROOM_BLOOM_MIPS` | **512** | Bloom mip cap (64–1024). |
| `SPEC_CHUM_ROOM_MSAA` | **4** | Room camera MSAA (`0`, `2`, `4`). **8× rejected** — Metal goes black. |
| `SPEC_CHUM_ROOM_FXAA` | on | Post FXAA on bezel edges. `=0` disables. |
| `SPEC_CHUM_ROOM_LIGHTS` | `full` | `full` or `min` (fewer sconces). |
| `SPEC_CHUM_ROOM_HYBRID` | **off** | Camera-space bake plates + live TV (experimental). |
| `SPEC_CHUM_ROOM_PERF` | off | Rolling tick µs to stderr + Swift HUD fields. |
| `SPEC_CHUM_ROOM_PERF_SOFT` | off | `room_perf`: warn instead of fail on budget exceed. |
| `SPEC_CHUM_ROOM_PIPELINE` | n/a | **Not implemented** — planned spike to re-enable `PipelinedRenderingPlugin` (always disabled today). |

Dial down for matrix runs, e.g.:

```bash
SPEC_CHUM_ROOM_BLOOM=0 SPEC_CHUM_ROOM_MSAA=0 SPEC_CHUM_ROOM_LIGHTS=min \
  cargo run -p living_room --example room_perf --release
```

Room camera uses `ClusterConfig::Single` (few lights — skip tiled cluster allocation).

## Hybrid plates (experimental, default off)

`SPEC_CHUM_ROOM_HYBRID=1` enables camera-parented **bake plates**: static room
geometry is rendered once per zoom stop to an unlit quad; only the TV/cabinet/CRT
updates every frame. Goal was ~60 Hz when full-room PBR looked costly on the
**old readback benchmark**.

**Default off** because:

- The plate is **camera-parented** — walls/sofa do not parallax correctly while zooming.
- Bake frames can **blank the background** during preset transitions.
- Full 3D at default quality already meets the 60 Hz budget on M4-class hardware.

Prefer **Blender lightmaps + `EnvironmentMapLight`** (tier 2) over extending hybrid plates.

## Framework choice

**Stay on Bevy.** Prior perf pain was the measurement harness, not fighting the
engine. Bevy gives headless `SubApps`, post-process bloom, glTF/PBR, and wgpu
Metal on Apple Silicon with one Rust codebase for standalone + embed.

Alternatives considered (RealityKit, Godot, Unity, raw wgpu): rejected for embed
complexity, licensing, or duplicating work already landed. Revisit raw wgpu only
if a GPU trace shows Bevy overhead **after** lightmaps and tier-2 wins land.

## Roadmap

### Tier 1 — done / quick wins

| Item | Status |
| --- | --- |
| Dual-clock embed (Spectrum 50 Hz ≠ display refresh) | **Done** |
| Present-path perf harness (`SimulatePresentPath`) | **Done** |
| Quality defaults restored (bloom 512, MSAA 4, 1920×1080) | **Done** |
| `ClusterConfig::Single` on room camera | **Done** |
| Scanline floor 0.58 in `crt_phosphor.wgsl` | **Done** |
| Hybrid plates default **off** | **Done** |

### Tier 2 — structural perf / lighting

| Item | Notes |
| --- | --- |
| **MetalFX spatial upscaling** | Render below backing scale, upscale in `present_metal.rs` — future win on Retina. |
| **Pipelined rendering spike** | Planned only — no `SPEC_CHUM_ROOM_PIPELINE` reader yet; soak before ship. |
| **Blender lightmaps** | Replace dynamic PBR fill with baked `Lightmap` + `EnvironmentMapLight`; drop hybrid plates. |
| **Halation in CRT material** | Move main glow from separate bloom pass into phosphor shader (tier-2 structural). |

Solari / TAA / DLSS are **not viable** on this stack.

### Tier 3 — CRT fidelity refactor

Separate large task (see CRT research). Root causes: mask/scanlines past Nyquist,
non-energy-conserving beam reconstruction, double gamma. **Suggested order:**

1. **Tube-space RT** at 1280×960 + mip chain; energy-conserving beam reconstruction;
   colour space fix (`hdr: true`, linear output, single gamma).
2. Analytic aperture grille (~280 triads); tube-space halation; horizontal filter fix.
3. Delete zoom ramps for scan/grille/bright (wrong direction); remove FXAA on tube;
   MSAA does not help inside the phosphor shader.

Do **not** start tier 3 until tier-2 lighting/perf baseline is stable.

## Quality gate

Bevy is **excluded** from the default `./scripts/check.sh` / Linux CI `check` job
(compile cost). CI runs `./scripts/check_living_room.sh` on **macOS** (`living-room` job).
Opt in locally:

```bash
./scripts/check_living_room.sh
# or: SPEC_CHUM_CHECK_LIVING_ROOM=1 ./scripts/check.sh
```

`SPEC_CHUM_LIVING_ROOM=1` is **app boot** (start SpecChumMac in living-room display mode) —
not the check.sh include-crate gate.

## Assets

See [crates/living_room/assets/CREDITS](../crates/living_room/assets/CREDITS).
Models and PBR textures are **Poly Haven CC0** (1k). Shaders are Spec Chum MIT.

## CRT notes

- **RVM** (Retro Virtual Machine) is a **visual reference** only — proprietary; we do
  not copy its pipeline. Look is approximated with open **crt-aperture** /
  **crt-easymode** techniques on the mesh.
- Curvature is **mesh geometry**, not a 2D barrel warp.
- Phosphor WGSL (`crt_phosphor.wgsl`): luminance-adaptive beams (scan floor **0.58**),
  aperture-grille triad (~0.30), soft-H / sharp-V sampling (`soft_mix` ≈ 0.40),
  gamma 2.2/2.2, brightness ≈ 2.4, black lift, vignette, PAL flicker ≤1%; tiny
  in-shader halation/diffusion only.
- Bevy `Bloom` is the main room halation at pull-back zoom (intensity ramps with
  `CrtLookBlend` in `camera.rs`).
- Room fill light follows framebuffer dominant colour (border LOAD glow).
- Constant **incandescent** tungsten lamp (left of TV) + warm ambient so sofa /
  wallpaper stay readable; CRT spill still tints on top.
- **Fidelity gaps** (tier 3): Nyquist-limited mask/scanlines, energy-conserving
  beam rebuild, single gamma path — see **Roadmap → Tier 3**.

## Out of scope (v1)

Walkable room, mouse-look, diegetic remote, notarised release packaging of the living-room
binary / embed (local `.app` asset bundling is supported for development).
