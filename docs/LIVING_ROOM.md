# Experimental Bevy living-room CRT host

## Spectrum Cabinet fixed-room comparison (#558)

The standalone room supports a fixed-camera composition experiment alongside the
existing 3D room. Set `SPEC_CHUM_ROOM_PRESENTATION=fixed-cabinet` before launch
to select it; unset the variable (or use `3d`) to keep the existing 3D room.
Switching presentations currently requires a restart. The fixed mode has no
intro dolly, mouse-look, or zoom stops; viewport changes only adjust camera
distance to keep the authored hero bounds in frame. The regular egui flat display
remains available as the close-view alternative.

The prototype uses flat, unlit Bevy rectangle meshes in the same 3D world and
camera. A matching unlit 8.0 × 9.0 m base wall extends behind the 4.2 × 2.45 m
authored wall so portrait and ultrawide viewports do not expose clear-color
margins; it scales with the camera frustum outside the tested aspect range,
while detail layers and cabinet retain their authored scale. The set has
an inset panel, paired curtains, low shelf, and a 0.62 × 0.44 m cyan
screen-spill panel with a radial alpha mask for soft edges. There are no
separate foreground art layers yet; the live TV geometry itself is the sole
foreground occluder. These vector-like layers are resolution-independent and
need no raster source asset. The live TV stand, cabinet, curved CRT phosphor,
and machine framebuffer remain 3D/runtime content. The back wall is behind the
TV and the panel is behind the CRT, so room artwork cannot cover the live image.
The simple rectangles are authored in the project; no generated or third-party
artwork is used. The stand and television models continue to use the existing
Poly Haven assets (CC0; see `crates/living_room/assets/CREDITS`).

The camera framing contract reserves 1.55 × 1.62 m hero bounds at 78% of the
viewport. The geometry-only resize cases are 320×900, 450×1000, 800×800,
1280×720, 2560×1080, and 320×240. Headless Bevy screenshots were captured at
each size; they show the complete CRT and active test pattern. Human visual
review against the Spectrum Cabinet concept remains open. On this Mac, the same
headless 1920×1080 probe measured fixed mode at 3.00 ms/tick average (p95
3.32 ms) and the existing 3D mode at 4.17 ms/tick average (p95 4.86 ms), each
over 100 ticks. This
is a rendering-cost diagnostic, not a display frame-pacing or input-latency
measurement, and does not predict reference hardware performance. Input
latency remains unmeasured. Keep the existing 3D room selectable until visual
and technical go/no-go review is recorded.

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
Launch from a normal Terminal with WindowServer — headless SSH sessions or
machines without a display often die in ~5s (monitor scale factor 0 / no display
attachment).

## Scene editing with Skein (opt-in, standalone only)

[Skein](https://bevyskein.dev/) is a **Bevy plugin + Blender extension** that stores
reflected Bevy components in glTF extras (`BEVY_skein`). With `--features skein`, a
configured export **is the living room** (procedural `room.rs` spawn is skipped).

**Assumes the Skein Blender add-on is already installed.** SpecChumMac stays
`--no-default-features` (no Skein / BRP). egui / Windows / Linux shells unchanged.

### Bevy / crate versions

| Piece | Version | Notes |
| --- | --- | --- |
| Spec Chum Bevy | **0.19** | Workspace pin. **No Bevy bump required** for Skein. |
| `bevy_skein` | **0.6.0** | Matches Bevy 0.19 (`--features skein`). |
| `bevy_skein` 0.7 | Bevy **0.20** | Deferred — [#438](https://github.com/mward-sudo/spec_chum/issues/438). |

### When is the Skein scene the room?

| Condition | Room source |
| --- | --- |
| Feature **off** (default SpecChumMac / CI without flag) | Procedural `room.rs` |
| `--features skein` + export **missing** | Procedural `room.rs` |
| `--features skein` + `SPEC_CHUM_ROOM_SKEIN_SCENE=off` | Procedural `room.rs` |
| `--features skein` + export **present** (default path or env) | **Skein glTF is the room** |

`SPEC_CHUM_ROOM_SKEIN_SCENE`:

| Value | Effect |
| --- | --- |
| unset / empty | Use `skein/living_room_edit.gltf#Scene0` if that file exists |
| `skein/foo.gltf` or `…#Scene0` | That asset (appends `#Scene0` if omitted) |
| `off` / `0` / `false` / `no` | Force procedural |

### Paths

| Path | Role |
| --- | --- |
| `assets/polyhaven/models/*.gltf` | Import into Blender as mesh sources |
| `assets/skein/` | Your full-room glTF / `.blend` (local; gitignored except README) |
| `assets/skein/living_room_edit.blend` | Editable starter scene (generate locally) |
| `assets/skein/living_room_edit.gltf` | Default room when present |
| `*.blend` / `crates/living_room/blender/` | Keep local (gitignored) |

### Generate a starter `.blend` (matches procedural layout)

Do **not** rebuild the room by hand. From a checkout with Poly Haven assets fetched:

```bash
./scripts/fetch_living_room_assets.sh   # once
./scripts/generate_living_room_blend.sh
```

Requires Blender on `PATH`, or macOS `/Applications/Blender.app`, or `BLENDER=/path/to/Blender`.

Writes (gitignored):

| Output | Role |
| --- | --- |
| `assets/skein/living_room_edit.blend` | Open this in Blender to edit |
| `assets/skein/living_room_edit.gltf` (+ `.bin` / textures) | Room load for `--features skein` |

The generator places Poly Haven props, room shell, and lights to match `room.rs` / `glow.rs`, then injects `BEVY_skein` tags (including **`TelevisionCabinet`** on the `television_02` empty) so the export is runnable without a first manual tag pass.

### What stays in Rust (even when Skein owns the room)

| Still code-driven | Why |
| --- | --- |
| CRT phosphor + glass (`crt.rs`) | Framebuffer upload / shader; attaches to Blender-tagged `TelevisionCabinet` |
| Camera / intro zoom | Host look-at phosphor after attach |
| Host / tape / audio / agent | Not scene content |
| `GlowDriven` tint sync | Updates Blender-tagged spill lights from the Spectrum FB |
| Soft `GlobalAmbientLight` | Fallback so a dark export is not pure black |

**Required on the TV empty:** Bevy component **`TelevisionCabinet`** (and preferably `LiveTv`). Without it, phosphor never attaches. The generator + tagger already set this on `television_02`; after Fetch Registry, confirm/re-insert via the Skein panel so tags survive a Blender re-export.

### Enable Rust side + BRP

```bash
./scripts/fetch_roms.sh
./scripts/fetch_living_room_assets.sh   # once — Poly Haven sources for Blender import
./scripts/generate_living_room_blend.sh # optional — starter .blend + tagged .gltf
cargo run -p living_room --release --features skein
```

BRP: **`http://127.0.0.1:15702`**. Leave the process running while Fetching Registry.

Registered markers:

| Component | Use |
| --- | --- |
| `TelevisionCabinet` | **Required** — CRT phosphor parent |
| `LiveTv` / `RoomStatic` | Hybrid / layer tagging |
| `CrtFillLight` / `GlowDriven` | Framebuffer-driven spill |
| `IncandescentLamp` | Warm fixture tag (reserved) |
| `DynamicRoomFillLight` | Temporary #149 A/B |

Presets: **CRT fill (soft)**, **Warm sconce** (`PointLight`).

### Step-by-step — open → edit → export → run

1. **Generate** (if needed): `./scripts/generate_living_room_blend.sh`
2. **Open** `crates/living_room/assets/skein/living_room_edit.blend` in Blender.
3. **Start** `cargo run -p living_room --release --features skein` (BRP on).
4. **Blender → Fetch Bevy registry** → `127.0.0.1:15702`.
5. **Edit** the scene (move furniture/lights; Skein panel on empties).  
   One-time after Fetch: select empty **`television_02`** → insert **`TelevisionCabinet`** (+ `LiveTv`) if the panel does not already show them. Optional: apply presets / `GlowDriven` on CRT spill lights (`crt_fill_light`, `crt_wall_bounce`).
6. **Export glTF** (enable extras / `BEVY_skein`) →  
   `crates/living_room/assets/skein/living_room_edit.gltf`  
   Or re-run `./scripts/generate_living_room_blend.sh` after large layout resets, then `python3 scripts/blender/tag_living_room_skein_gltf.py` if you only re-exported from Blender without the tagger.
7. **Run again** with `--features skein`. Log should say *loading … as the room*.  
   Procedural furniture / walls / glow fills are **not** spawned.
8. Force procedural anytime: `SPEC_CHUM_ROOM_SKEIN_SCENE=off`.

### SpecChumMac

Mac embed: `--no-default-features` → always procedural room (no Skein). Promote a
committed glTF load path later if the export becomes shipping art; until then,
standalone is the editor.

### Verify

```bash
./scripts/check_living_room.sh
# includes --features skein clippy/test
```

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
- Present path: Bevy renders offscreen → GPU blit into a shared **IOSurface** →
  `CALayer.contents` (no per-frame CPU `CGImage` readback). Default render size is **1920×1080**
  (`DEFAULT_ROOM_W/H`); Swift steps the long edge up to a **2560** backing-pixel cap,
  aspect-matched to the view.
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
- Swift HUD: host ms, **roomHz** / **specHz**, skip-busy, present WxH, Bevy last/avg/max tick time.
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

Poly Haven CC0 meshes/textures under `polyhaven/` are **gitignored**. Fresh clones / worktrees must run `./scripts/fetch_living_room_assets.sh` (or let `build_macos_app.sh` / `run_macos_app.sh` auto-fetch). Staging validates `polyhaven.manifest` and hard-fails on incomplete trees unless `SPEC_CHUM_ALLOW_EMPTY_LIVING_ROOM_ASSETS=1`. Enabling living-room mode with a missing tree surfaces an actionable status error instead of a black void (#368).

### Blender-baked static lighting (#149)

The `New` scene comparison uses a checked-in Blender lightmap package for static
room geometry. The TV, cabinet, phosphor and framebuffer-driven CRT spill remain
live. The package is generated from the room layout and fetched Poly Haven
assets; after changing either, rebake before building the Mac app:

```bash
SPEC_CHUM_LIGHTMAP_THREADS=2 ./scripts/bake_living_room_lightmaps.sh
```

Blender 4.2 or newer is required. The script writes
`crates/living_room/assets/lightmaps/room_static.gltf`, the UV1 lightmap atlas,
referenced textures, and a local `room_static_bake.blend` for inspection. Keep
the glTF, atlas, and textures together in version control. The `.blend` is
gitignored; the bake script and fetched Poly Haven source assets are the
reproducible inputs. The saved local `.blend` packs its images so it can be
opened independently for inspection.
The app loads the baked room only for the `New` variant when both glTF and atlas
are present. `Current` continues to use the procedural baseline, and a missing
bake leaves `New` on its comparison fallback. The procedural room stays visible
until every baked mesh has received the lightmap; failed or incomplete mesh
binding leaves the fallback room visible. The opening waits for scene readiness
for up to three seconds, then proceeds so a failed load cannot hold the room on
black.
Opening light levels animate the baked exposure and live lights together.

Run `./scripts/check_living_room.sh` after rebaking or changing the Bevy binding.
For visual review in SpecChumMac, enable Living Room and select **Scene: New**.

## Dual-clock embed plan

**Status:** **implemented** for SpecChumMac (Phases 0–3). Not research-only.

| Done | Work |
| --- | --- |
| **0** | Perf HUD / `sample` baseline (`tmp-perf-capture/sample-dualclock.txt`) |
| **1** | Decouple clocks: main publishes FB only; `CADisplayLink` paces `sc_room_tick` |
| **2** | Linear `CALayer` filters + aspect-matched present (2560 cap) |
| **3** | Display delta via `sc_room_set_frame_delta_seconds`; zoom/skip = cheap mutations |

Phase 4 (`CAMetalLayer` drawable) remains optional. Phase 5 (pipelined Bevy) was
evaluated and rejected for the current Mac embed; see the results below.
Refs [#146](https://github.com/mward-sudo/spec_chum/issues/146).

### Automated test coverage gap

`./scripts/check_living_room.sh` runs `cargo fmt/clippy/test -p living_room` in
**release** by default (Bevy debug is multi‑GB) and the **headless** `room_perf`
example. Set `SPEC_CHUM_ROOM_DEBUG=1` for debug clippy/test. It times the
**shipping present path** (`SimulatePresentPath` — no blocking CPU readback). It
does **not** exercise:

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
| Phosphor shader | Fixed soft-H mix, scan and grille at every zoom | Camera distance and curved-mesh mip filtering change apparent size without changing phosphor parameters |
| 3D room render target | **1920×1080** default; Swift caps long edge at **2560**, aspect-matched to view | Geometry needs pixels; CALayer linear-upscales when the window is larger |
| Room MSAA | Optional **MSAA on room camera only** | Smooths 3D mesh edges; does not change sampling inside the phosphor shader |
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
- Zoom / skip intro: `livingRoomThread.async` → `sc_room_nudge_zoom` / `sc_room_skip_intro` only — **no forced `sc_room_tick`**; DisplayLink presents the new pose.
- Coalesce: at most one room tick in flight; “latest wins” for FB publish; never unbounded tick queues.
- On Bevy overrun: drop/coalesce **room** frames; never skip or delay Spectrum `sc_run_frame`.

### Bevy 0.19 headless + pipelined rendering

**Current embed (`headless.rs`) after dual-clock:**

- `SubApps` + disable `WinitPlugin` + disable **`PipelinedRenderingPlugin`**
- `TimeUpdateStrategy::ManualDuration` advanced each tick from **display delta**
  (`sc_room_set_frame_delta_seconds`) — not Spectrum 1/50
- Present: GPU blit into IOSurface (`present.rs` / `present_metal.rs`); `PollType::Poll` when presenting

**Phase 5 evaluation:** Bevy moves the render schedule to a dedicated thread so
frame N render overlaps frame N+1 sim. A five-minute headless A/B soak completed
18,000 frames per mode, with changing Spectrum framebuffers and camera zoom.
The baseline averaged 9.15 ms per tick (p95 12.09 ms); pipelined rendering
averaged 8.96 ms (p95 12.40 ms). Both met the 60 Hz budget and missed 120 Hz,
with no meaningful improvement.

The first native pipeline run rendered black because a serial DispatchQueue did
not keep one OS thread across jobs. Bevy's `MainThreadExecutor` is thread-local;
the render worker waits for main-world work when `SubApps::update` runs from a
different OS thread than the one that created the room. A dedicated
`LivingRoomThread` fixes that affinity, and a sustained native run rendered the
Spectrum boot screen with `roomHz=137`, `specHz=51`, and `present=1`. Keep
pipelined rendering disabled by default pending a separate performance case;
revisit it only with measured benefit and explicit render-completion ownership.

Other lifecycle risks to test before enabling it:

- Manual `SubApps::update` + extract channels assume Bevy’s runner lifecycle; headless + force-load staticlib may deadlock or double-own the render thread.
- Present blit must stay inside Bevy’s ordered submit (already true); out-of-band Metal present races.

The pinned thread is now used for all room FFI calls, even while pipelining is
disabled, so the same host remains safe if the experiment is repeated.

### Phases

| Phase | Status | Work | Risks |
| --- | --- | --- | --- |
| **0 — Measure** | **Done** | Baseline with `SPEC_CHUM_ROOM_PERF=1`; `sample-dualclock.txt` — main idle of `sc_room_tick`, room queue holds Bevy. | Misreading pre–dual-clock samples |
| **1 — Decouple clocks** | **Done** | `runFrame` publishes FB only. `CADisplayLink` → room queue tick. Latest-slot upload at tick start. | Stale FB if publish races (gen slot handles) |
| **2 — Present filter + res** | **Done** | CALayer **linear** filters; aspect-matched present up to **2560** long-edge (default **1920×1080**). | Resize recreate cost on debounced window changes |
| **3 — CRT look / Δt** | **Done** | Display delta for Bevy `Time`; zoom/skip without forced tick; phosphor nearest / present linear. | Over-softening if phosphor sampler flipped to linear |
| **4 — Drawable path (optional)** | Open | `CAMetalLayer` + DisplayLink; blit or texture into drawable; retire IOSurface if redundant. | wgpu-hal / MTLDevice identity; drawable timeout under SwiftUI load |
| **5 — Pipelined spike** | Deferred | No material headless performance gain. The initial black screen was Bevy main-executor thread affinity and is fixed by the pinned host thread; native rendering now succeeds. Keep disabled pending measured benefit and explicit frame ownership. | Async renderer and IOSurface presentation still need lifecycle and completion validation |

### What NOT to do

- Drive Spectrum / `sc_run_frame` at 120 Hz (or display rate) — wrong for ULA timing and audio.
- Run Bevy `sc_room_tick` on AppKit main (regresses input hitch; prior sample evidence).
- Nearest-upscale the **full window** / full 3D present.
- Bind Bevy render size 1:1 to Retina backing without a budget (create/resize stalls).
- Resurrect living_room `cdylib` next to `host_api`.
- Use `@Published displayTick` spam for room present.
- Block main on `livingRoomThread.sync` except create/destroy/bind.
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

For a deterministic image comparison without opening the app, dispatch the
**CRT visual comparison** workflow with a baseline ref (default `v0.4.0`) and
candidate ref (default `main`). It uploads baseline, candidate and side-by-side
PPM captures of the same color-bar, fine-line and checker pattern. Both renders
use `HeadlessRoom` and create no window.

## Quality knobs

Runtime A/B via `SPEC_CHUM_ROOM_*` (implemented in `crates/living_room/src/quality.rs`):

| Variable | Default | Effect |
| --- | --- | --- |
| `SPEC_CHUM_ROOM_HALATION` | `bloom` | `bloom` keeps the established camera Bloom baseline; `material` disables global Bloom and uses local phosphor scatter. |
| `SPEC_CHUM_ROOM_BLOOM` | on | Enables/disables Bevy Bloom in `halation=bloom` mode. Material mode keeps global Bloom off. |
| `SPEC_CHUM_ROOM_BLOOM_MIPS` | **512** | Bloom mip cap (64–1024); applies when camera Bloom is enabled. |
| `SPEC_CHUM_ROOM_MSAA` | **4** | Room camera MSAA (`0`, `2`, `4`) smooths 3D mesh edges. It does not change sampling inside the phosphor shader. **8× rejected** — Metal goes black. |
| `SPEC_CHUM_ROOM_LIGHTS` | `full` | `full` or `min` (fewer sconces). |
| `SPEC_CHUM_ROOM_SCENE` | `current` | Temporary #149 A/B: `current` (procedural baseline) or `new` (baked static room when the package is present). Prefer SpecChumMac toolbar toggle. **Remove when #149 is complete.** |
| `SPEC_CHUM_ROOM_SKEIN_SCENE` | auto | Standalone `--features skein` only: Bevy asset path for the **room** glTF (default `skein/living_room_edit.gltf#Scene0` if file exists → replaces procedural `room.rs`). `off` forces procedural. |
| `SPEC_CHUM_ROOM_PERF` | off | Rolling tick µs to stderr + Swift HUD fields. |
| `SPEC_CHUM_ROOM_PERF_SOFT` | off | `room_perf`: warn instead of fail on budget exceed. |

Dial down for matrix runs, e.g.:

```bash
SPEC_CHUM_ROOM_BLOOM=0 SPEC_CHUM_ROOM_MSAA=0 SPEC_CHUM_ROOM_LIGHTS=min \
  cargo run -p living_room --example room_perf --release
```

Room camera uses `ClusterConfig::Single` (few lights — skip tiled cluster allocation).

For the #458 visual A/B, launch twice with `SPEC_CHUM_ROOM_SCENE=current`, once
with `SPEC_CHUM_ROOM_HALATION=bloom` and once with `=material`. Keep the window
size, camera preset, and exposure unchanged. The Bloom mode retains the legacy
camera-wide glow; material mode replaces that role with phosphor-local scatter.

## Hybrid plates (retired)

The old `SPEC_CHUM_ROOM_HYBRID` knob no longer enables camera-parented **bake
plates**. The path is hard-disabled because static room geometry rendered to an
unlit quad did not parallax correctly during zooms and could blank the
background during preset transitions.

Full 3D at default quality already meets the 60 Hz budget on M4-class hardware.
The current static-room path uses Blender lightmaps and `EnvironmentMapLight`.

## Temporary #149 scene comparison (SpecChumMac only)

SpecChumMac living-room mode shows a **Scene: Current / Scene: New** toolbar
toggle (next to Living Room). This is a **Mac-only verification harness** — not
on egui, Windows, or Linux shells.

| Variant | Look |
| --- | --- |
| **Current** | Pre-#149 baseline (warm ambient + dynamic sconces / wall bounce). No `EnvironmentMapLight`. |
| **New** | Blender-baked static room with UV1 lightmap, moodier cream-warm ambient, lit sconces, and a procedural hemispherical `EnvironmentMapLight`. Live TV and CRT spill remain dynamic; a dim cyan strip appears only when the bake package is absent. |

Toggle switches the live Bevy scene (not a label-only flag). Select the `New`
variant with `SPEC_CHUM_ROOM_SCENE=new`. The toolbar control, scene-variant
FFI, fallback lighting and cyan cue are temporary and should be removed when
#149's remaining visual and workflow acceptance criteria are complete.

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
| Scanline floor 0.58 in `crt_tube.wgsl` | **Done** |
| Hybrid plates default **off** | **Done** |

### Tier 2 — structural perf / lighting

| Item | Notes |
| --- | --- |
| **MetalFX spatial upscaling (#462)** | Evaluated and not retained: on an Apple M4, the same 48K boot screen and 2560×2244 present target rendered at 1716×1504 (0.67) or 1280×1122 (0.5); the captured splash remained intact at both scales, but neither reduced scale showed a repeatable tick-time or frame-rate benefit over full resolution in equal-duration samples. The optional `CAMetalLayer`/MetalFX path was removed; the stable IOSurface path remains. Keep the 16-byte IOSurface row-stride validation, which independently prevents Metal texture-import aborts at unaligned widths. |
| **Pipelined rendering spike** | Deferred: thread affinity fixed the native black screen, but headless A/B showed no material perf gain. Keep disabled pending a measured benefit and completion/lifecycle validation. |
| **Blender lightmaps** | Replace dynamic PBR fill with baked `Lightmap` + `EnvironmentMapLight`; drop hybrid plates. SpecChumMac temporary Current/New A/B toggle documents verification until this lands. Opt-in **Skein** (`--features skein`) helps tag Bevy markers / lights from Blender while lightmaps land — see [Scene editing with Skein](#scene-editing-with-skein-opt-in-standalone-only). |
| **Halation in CRT material** | Move main glow from separate bloom pass into phosphor shader (tier-2 structural). |

Solari / TAA / DLSS are **not viable** on this stack.

### Tier 3 — CRT fidelity refactor

Separate task tracked by #148. The original fidelity gaps—window-space phosphor
processing, a non-energy-normalized beam, and a raster-locked grille—are resolved.
The colour path uses an sRGB source texture, an HDR room-camera intermediate, and one
Bevy tonemapping/output conversion. **Completed and remaining work:**

1. **Tube-space RT**: the 352×296 sRGB framebuffer is reconstructed by an isolated
   2D camera into a fixed 1280×960 linear HDR canvas. Its mip 0 is copied to a
   separate 11-level tube image, then Bevy generates the remaining mips before
   the room camera samples the curved CRT mesh. The bezel and room-sized present
   target remain separate.
2. Energy-normalized multi-line beam reconstruction, analytic aperture grille
   (~280 triads), and linear-light horizontal filtering with a normalized 3-tap
   kernel. Tube-space reconstruction and optional material halation are implemented
   in `crt_tube.wgsl`; curved-mesh sampling remains in `crt_phosphor.wgsl`.
3. Keep scan/grille/brightness fixed across zoom. Room-camera FXAA is removed because
   it post-processes the tube too; MSAA only smooths room geometry, not phosphor sampling.

Tier-2 prerequisites (#149 and #150) are complete. The remaining #148 work is
the offline visual comparison against the v0.4.0 baseline.

## Quality gate

Bevy is **excluded** from the default `./scripts/check.sh` / Linux CI `check` job
(compile cost). CI runs `./scripts/check_living_room.sh` on **macOS** (`living-room` job)
with **release** clippy/test. Opt in locally:

```bash
./scripts/check_living_room.sh
# or: SPEC_CHUM_CHECK_LIVING_ROOM=1 ./scripts/check.sh
# SPEC_CHUM_ROOM_DEBUG=1 ./scripts/check_living_room.sh   # disk-heavy Bevy debug
```

Historical note ([#171](https://github.com/mward-sudo/spec_chum/issues/171)): Apple ld
`__eh_frame section too large` notes on large Bevy examples were previously seen in CI;
they have not appeared on recent `macos-latest` runs and do not fail `-Dwarnings` if they return.

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
- Tube reconstruction WGSL (`crt_tube.wgsl`): energy-normalized luminance-adaptive
  beams, analytic 280-triad aperture grille, fixed soft-H / sharp-V sampling,
  sRGB texture input and linear HDR shader output, black lift,
  vignette, PAL flicker ≤1%; tiny
  in-shader halation/diffusion only. `crt_phosphor.wgsl` samples the completed
  tube image with mip filtering on the physically curved mesh.
- Bevy `Bloom` is the main room halation at pull-back zoom in the `bloom` baseline
  (intensity ramps with `CrtLookBlend` in `camera.rs`). The selectable `material`
  halation mode disables camera Bloom and moves the primary CRT halo into the
  phosphor shader; Bloom remains available as the baseline / comparison path.
- Living-room camera uses **`Exposure` ev100 8.2** (between Bevy `INDOOR` 7.0
  and default `BLENDER` 9.7). Pure `BLENDER` crushed furniture; full `INDOOR` plus
  high spill/bloom washed the CRT when zoomed out ([#233](https://github.com/mward-sudo/spec_chum/issues/233)).
- Soft CRT spill (~1.1k–4.0k lm) tints walls from the framebuffer; TV-wall
  sconces ~14.4k lm tungsten; warm `GlobalAmbientLight` keeps sofa / wallpaper
  readable. Pull-back bloom stays mild so the tube face stays legible.
- **Fidelity gaps** (tier 3): final baseline comparison — see **Roadmap → Tier 3**.

## Out of scope (v1)

Walkable room, mouse-look, diegetic remote, notarised release packaging of the living-room
binary / embed (local `.app` asset bundling is supported for development).
