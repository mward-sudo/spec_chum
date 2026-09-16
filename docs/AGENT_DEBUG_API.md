# Agent Debug HTTP API

> **Audience:** developers and automation (scripted control / inspect).
> “Agent” here means this HTTP surface and `SPEC_CHUM_AGENT*` env vars — not LLM coding assistants.
>
> **Status:** **Implemented** on `main` — loopback HTTP on `127.0.0.1:17384` (default),
> SpecChumMac/egui live embed, port watches, prefs/hardware routes, and
> `GET /v1/memory/regions`. Optional later: WebSocket push / OpenAPI schema
> ([#236](https://github.com/mward-sudo/spec_chum/issues/236)).

## Purpose

Scripted debugging without driving the GUI:

- **Stable control / inspect** — one localhost API over a shared Rust service layer
  (`control_plane`), instead of divergent one-shot CLI vs egui Debug vs FFI paths.
- **Guest pixels at 1:1** — Timex hi-res, border colour, and SCLD checks need the
  **emulator framebuffer**, not a scaled CRT or OS window grab.
- **Same semantics for humans and scripts** — GUI Debug, `spec-chum-debug`, and HTTP
  clients share the same backend behaviour.

## Architecture

All debugging / control / inspect paths converge on one backend:

```text
                    ┌─────────────────────────────────────┐
                    │  control_plane (shared Rust crate)  │
                    │  HostSession + Machine + Debugger   │
                    │  + trace ring + framebuffer export  │
                    └──────────────┬──────────────────────┘
                                   │
         ┌─────────────────────────┼─────────────────────────┐
         │                         │                         │
   HTTP server              in-process              HTTP client
   (loopback)               direct call             (debug_cli)
         │                         │                         │
    curl / scripts           egui / SpecChumMac      spec-chum-debug
    automation clients       Debug UI                (local or --agent-url)
```

- The **HTTP server is a thin transport** over the shared crate (or an embedded
  call into the same types when the host runs in-process).
- **`spec-chum-debug`** uses `HostSession` locally, or HTTP when `SPEC_CHUM_AGENT_URL`
  / `--agent-url` is set.
- **egui Debug** and **SpecChumMac Debug** use the same live session as an optional
  embedded HTTP server (`SPEC_CHUM_AGENT=1`).

### What ships today

| Piece | Notes |
| --- | --- |
| Standalone server | `spec_chum --serve` / `spec-chum-agent` / `spec-chum-debug --serve` |
| `spec-chum-debug` HTTP client | `SPEC_CHUM_AGENT_URL` / `--agent-url` |
| egui embed | `SPEC_CHUM_AGENT=1` over the GUI `ControlPlane` / `HostSession` |
| SpecChumMac embed | `SPEC_CHUM_AGENT=1` + `sc_agent_embed_start` on the live `sc_*` session |
| Prefs / mouse / tape eject / continue | `GET`/`PATCH /v1/prefs`, `POST /v1/mouse`, `POST /v1/tape/eject`, `POST /v1/continue` |
| Hardware attach | `/v1/hardware/*` (Multiface / DivMMC / IF1 / MDR / TR-DOS ROM) |

`spec-chum-debug` local commands route through
[`HostSession`](../crates/host_api/src/session.rs) — the same type wrapped by
`control_plane::ControlPlane` and the HTTP server.

**Primary surfaces (prefer these):**

| Consumer | Surface |
| --- | --- |
| Automation / scripts | Loopback HTTP (`control_plane` + `agent_server`) |
| Rust CLI | `spec-chum-debug` → `HostSession` locally, or HTTP client when `SPEC_CHUM_AGENT_URL` set |
| Rust library hosts | `control_plane::ControlPlane` / `HostSession` (no C ABI) |

**C ABI = FFI-only:** `sc_debug_*`, `sc_inspect_json`, `sc_peek` / `sc_poke`,
`sc_step`, `sc_add_breakpoint`, `sc_run_until_break`, and related entry points in
[`spec_chum_host.h`](../crates/host_api/include/spec_chum_host.h) remain **thin
wrappers** over `HostSession` + the global `trace` ring for **non-Rust shells**
(SpecChumMac Swift, future foreign-language hosts). They are **not** the primary automation API and must **not** gain a `host_api` → `control_plane` dependency
(avoids a crate cycle).

#### Surface inventory

| Path | Role today |
| --- | --- |
| C ABI `sc_debug_*` / inspect / step / breakpoints | **FFI-only** for SpecChumMac / non-Rust shells |
| egui Debug panel | Shared `HostSession` via `ControlPlane` |
| egui / SpecChumMac `SPEC_CHUM_AGENT=1` | Embedded HTTP on the live GUI session |
| `spec-chum-debug` (no agent URL) | Local `HostSession` (same type as `control_plane`) |

## Technology choice: REST on loopback

| Option | Verdict |
| --- | --- |
| **HTTP REST on `127.0.0.1`** | **Chosen.** PNG bodies and JSON inspect fit naturally; easy `curl` / scripts; optional OpenAPI; debuggable in a browser tab. |
| Unix domain socket + JSON-RPC | Lower overhead, but weaker tooling ergonomics and no standard file-download story for framebuffers. |
| gRPC + protobuf | Heavy codegen/deps for a localhost-only tool; poor fit for “save this PNG”. |
| **WebSocket (optional)** | **Later** — push trace events, breakpoint notifications, tape progress ([#236](https://github.com/mward-sudo/spec_chum/issues/236)). |

### Security

- Bind **`127.0.0.1` only** (or `::1`); reject other interfaces.
- **Mutating routes** (`POST`, `PUT`, `PATCH`, `DELETE`) require a bearer token via
  `SPEC_CHUM_AGENT_TOKEN` (or `--token`) **by default** — even on loopback. Set
  `SPEC_CHUM_AGENT_INSECURE=1` only on trusted dev machines to allow unauthenticated
  mutations (not recommended when a browser tab can reach the server).
- **Startup without a token:** the server **refuses to start** unless
  `SPEC_CHUM_AGENT_INSECURE=1` is set (or `--insecure` is passed). Mutating requests
  without a valid bearer token return **`401 Unauthorized`**.
- `GET` routes remain unauthenticated on loopback unless a token is configured (then
  all routes require it).
- No TLS (localhost); document that the server must never be exposed publicly.

Default port: **`17384`** (`SPEC_CHUM_AGENT_PORT`; `1` + phone-keypad *SPEC* `7384`)
— configurable; single-instance lock file to avoid port clashes.

## Framebuffer export (visual QA)

> Guest **1:1** pixels live at `GET /v1/framebuffer`. Host presentation / OS window
> shots are separate — see [Host view screenshots](#host-view-screenshots-239) below.

Prefer these API endpoints over unconstrained `screencapture` / multi-monitor grabs
for emulator visual QA (guest buffer or carefully scoped own-window capture).

The guest export is the same RGBA buffer hosts already expose via
`sc_framebuffer_ptr` / `HostSession::framebuffer()`:

| Query | Meaning |
| --- | --- |
| `border=false` | **Paper only** — active display file at native resolution |
| `border=true` | Paper + ULA border (Spec Chum layout; bottom border taller) |
| `format=png` | `image/png` body (default for automation clients) |
| `format=rgba` | Raw RGBA8 row-major (`width × height × 4` bytes) |

**1:1 native dimensions** (from `ula::framebuffer_dims`, no host scaling, no CRT
filter, no living-room post-process):

| Mode | Paper (`border=false`) | With border (`border=true`) |
| --- | --- | --- |
| Sinclair lo-res (48K/128K/+3/TC2048 TS2068 lo modes) | **256×192** | **352×296** |
| Timex SCLD hi-res (modes 4–7) | **512×192** | **640×296** |

Response headers / JSON metadata include `width`, `height`, `border`, `hires`,
`scld_mode` (when Timex), and `model` so clients can validate size before visual diff.

**Not `/v1/framebuffer`:**

- Living-room Bevy CRT (experimental display mode) — scaled, shaded, halation.
- egui chrome / letterboxed display — use `/v1/host/*` below.
- Audio waveform or border-event trace (use `/v1/inspect` + trace instead).

Example:

```bash
curl -sS -H "Authorization: Bearer $SPEC_CHUM_AGENT_TOKEN" \
  'http://127.0.0.1:17384/v1/framebuffer?border=false&format=png' \
  -o /tmp/spec_paper.png
# Inspect /tmp/spec_paper.png for Techdraw hi-res QA
```

## Host view screenshots (#239)

| Endpoint | Source | Notes |
| --- | --- | --- |
| `GET /v1/host/display` | **In-process** software NEAREST + letterbox of the guest RGBA (matches egui `TextureOptions::NEAREST` + `fit_size`) | Query: `scale=1..16` **or** `width`+`height`; default 2× when no live panel. **egui and SpecChumMac** (when agent embedded) publish live panel size (`X-SpecChum-Panel: live`). |
| `GET /v1/host/window` | **OS capture of this process’s own window only** | Shared `OwnWindowCapturer` in `control_plane`. macOS: `CGWindowListCreateImage` + registered `CGWindowID`, PID-checked. **Does not** activate, focus, or change z-order. SpecChumMac publishes via `sc_agent_set_host_window_id`; egui via eframe `NSView`. Standalone / unset id → `503`. |

**Hard rules for `/v1/host/window`:** never frontmost/desktop/focused-window APIs; never bring the window forward to capture; fail closed on missing/stale/wrong-PID id. egui and SpecChumMac expose the **same** `/v1/host/*` surface (not platform-divergent feature sets).

```bash
# Presented display (works on standalone server too)
curl -sS -H "Authorization: Bearer $SPEC_CHUM_AGENT_TOKEN" \
  'http://127.0.0.1:17384/v1/host/display?scale=2&format=png' \
  -o /tmp/spec_display.png

# Full host window (requires embedded GUI: egui or SpecChumMac + SPEC_CHUM_AGENT=1)
curl -sS -H "Authorization: Bearer $SPEC_CHUM_AGENT_TOKEN" \
  'http://127.0.0.1:17384/v1/host/window' \
  -o /tmp/spec_window.png
```

## API surface

Routes available on the loopback server today (plus notes where behaviour is deferred).

### Control

| Area | Operations |
| --- | --- |
| Machine | `POST /v1/model` — select built-in model; `POST /v1/config` — apply `#187` custom profile JSON; `POST /v1/reset`; `POST /v1/running` pause/run; `POST /v1/run` — advance within a **finite budget** (see below) |
| Execution | `POST /v1/step` — one `step_once`; `POST /v1/step` body `{ "count": N }`; `POST /v1/continue` — resume after debugger stop (`continue_from_pc`); `POST /v1/run-until` — PC / budget (maps `Debugger::run_until`); `POST /v1/regs` — patch `pc` / `sp` / `af` (hex strings; #261 TR-DOS entry) |
| Tape | `POST /v1/tape/open`, `/play`, `/pause`, `/rewind`, `/eject`; load options flash vs EAR vs experience + speed. EAR `speed` is a *loading* multiplier only — `GET /v1/inspect` reports `tape.effective_speed` (1 once the deck finishes) as the live rate. `flash_load` on a pulse-only TZX has no LD-BYTES trap to poke, so it loads off EAR at 64× rather than at `speed` (#390) |
| Type-load | `POST /v1/type-load` — scripted LOAD "" [CODE] (today's `type-load` subcommand) |
| ROM | `POST /v1/rom` — load ROM image from host filesystem path |
| Media | `POST /v1/snapshot`, `/rzx`, `/dsk`, `/trd`, … |
| Input | `POST /v1/keys` matrix press/release; `POST /v1/joystick`; `POST /v1/mouse` Kempston delta/buttons |
| Hardware | `GET /v1/hardware` — attach flags; `POST /v1/hardware/multiface` (+ `/nmi`); `POST /v1/hardware/interface1` (+ `/rom`, `/v1/hardware/mdr`); `POST /v1/hardware/divmmc` (+ `/sd`, `/eeprom`); `POST /v1/hardware/beta`; `POST /v1/hardware/trdos/rom`; Timex `.dck` via `/v1/timex/dock` |
| Host prefs | `GET` / `PATCH /v1/prefs` — volume, mute, joystick mode, tape defaults, throttle, Kempston mouse enable (session-scoped in agent server; living-room display toggle deferred) |
| Border | `POST /v1/border` — `with_border` flag (changes framebuffer dims) |

### Inspect

| Area | Operations |
| --- | --- |
| Core | `GET /v1/inspect` — full `Inspect` JSON (CPU, raster, paging, tape, AY, Timex fields) |
| Video | `GET /v1/framebuffer` — PNG or RGBA (see above); `GET /v1/video` — dims + SCLD mode without pixels |
| Memory | `GET /v1/peek?addr=&len=`; `POST /v1/poke`; `GET /v1/memory/regions` (CPU map + paging) |
| Disasm | `GET /v1/disasm?addr=&count=` |
| Debugger state | `GET /v1/debug/breakpoints`, `/watches`, `/port-watches`, `/last-break` |
| ROM | `GET /v1/rom/setup` — slots + availability (`sc_model_rom_setup_json` parity) |
| Status | `GET /v1/status`, `/v1/health`; `GET /v1/errors/last` — status may include optional `media_title` / `media_sha512` when a tape is inserted ([#366](https://github.com/mward-sudo/spec_chum/issues/366) / [#373](https://github.com/mward-sudo/spec_chum/issues/373), [`TAPE_IDENTITY.md`](TAPE_IDENTITY.md)) |
| Prefs | `GET /v1/prefs` snapshot |

### Debug

| Area | Operations |
| --- | --- |
| Breakpoints | `GET`/`POST /v1/debug/breakpoints` (body `{ "pc" }`); `DELETE /v1/debug/breakpoints/{pc}`; mem watches at `/v1/debug/watches`; port watches at `/v1/debug/port-watches` |
| Trace | `GET /v1/trace/categories` — list enabled categories; `PUT /v1/trace/categories` — enable/disable; `POST /v1/trace/clear`; `GET /v1/trace` — ring text/JSON/ndjson |
| Run control | `POST /v1/run-until` — PC, mem write, port, halt, insn budget |
| Step semantics | `step` = one instruction; `step-over` deferred until call-stack support exists — document as optional |

### Core routes (minimum useful surface)

Smallest useful automation surface:

1. `GET /v1/health`
2. `GET /v1/inspect`
3. `GET /v1/framebuffer` (PNG + rgba; `border` query)
4. `POST /v1/model`, `/reset`, `/run`
5. `POST /v1/tape/open`, `/play`, `/pause`, load-options, `/type-load`
6. `GET /v1/peek`, `GET /v1/disasm`
7. `GET /v1/trace/categories`, `PUT /v1/trace/categories`, `GET /v1/trace`


### Phase G — prefs / mouse / eject / continue

| Route | Notes |
| --- | --- |
| `GET /v1/prefs` | Session snapshot: `volume`, `muted`, `throttle`, `joystick_mode`, `kempston_mouse`, `tape_experience`, `tape_ear_speed` |
| `PATCH /v1/prefs` | Partial update; applies joystick mode + tape load options to the loaded machine when present. Does **not** persist `ui-prefs.json`. |
| `POST /v1/mouse` | Requires `kempston_mouse: true` in prefs. Body `{ "dx", "dy", "left", "right", "middle" }` and/or `{ "clear": true }` (full axis+button reset) — Kempston mouse via `HostSession` ([#136](https://github.com/mward-sudo/spec_chum/issues/136)) |
| `POST /v1/tape/eject` | Clears the inserted TAP/TZX deck |
| `POST /v1/continue` | `continue_from_pc` after a debugger stop; JSON `{ "reason", "paused" }` |
| `POST /v1/regs` | Patch `pc` / `sp` / `af` (optional hex strings, at least one required); JSON `HostRegs` (#261) |

### Port watches

| Route | Notes |
| --- | --- |
| `GET /v1/debug/port-watches` | List I/O port watches (`addr`, `read`, `write`, `mask`) |
| `POST /v1/debug/port-watches` | Body `{ "addr", "read"?, "write"?, "mask"? }` — at least one of read/write; optional hex `mask` (default `0xFFFF` = exact). Keyboard row polls: `{ "addr": "fe", "read": true, "mask": "ff" }` matches `$xxFE` ([#387](https://github.com/mward-sudo/spec_chum/issues/387)) |
| `DELETE /v1/debug/port-watches/{addr}` | Remove port watch by configured `addr` (idempotent) |

Mem watches remain on `/v1/debug/watches` (`GET` lists both mem and port; `POST` adds mem with the same optional `mask`;
`DELETE /v1/debug/watches/{addr}` removes mem). `spec-chum-debug watch-write --port` uses the
port-watch HTTP path when `--agent-url` is set.

Living-room display toggle is **deferred** (not modeled in shared prefs yet).

### Phase H — peripherals HTTP attach

| Route | Notes |
| --- | --- |
| `GET /v1/hardware` | `{ has_multiface, has_interface1, has_divmmc, has_timex_dock, has_beta }` |
| `POST /v1/hardware/multiface` | Body `{ "path" }` — 8 KiB Multiface ROM (MF1 on 48K-class, MF128 on 128K/+2; +2A/+3 reject); Refs [#168](https://github.com/mward-sudo/spec_chum/issues/168) |
| `POST /v1/hardware/multiface/nmi` | Red-button NMI when attached |
| `POST /v1/hardware/interface1` | Attach IF1 (loads `roms/if1*.rom` if present); Refs [#139](https://github.com/mward-sudo/spec_chum/issues/139) |
| `POST /v1/hardware/interface1/rom` | Body `{ "path" }` — explicit IF1 ROM |
| `POST /v1/hardware/mdr` | Body `{ "path" }` — Microdrive cartridge (attaches IF1) |
| `POST /v1/hardware/divmmc` | Attach DivMMC (no media); Refs [#138](https://github.com/mward-sudo/spec_chum/issues/138) |
| `POST /v1/hardware/divmmc/sd` | Body `{ "path", "slot"? }` — flat SD image; `slot` defaults to `0` (CS bit0); use `1` for the second card (CS bit1) |
| `POST /v1/hardware/divmmc/eeprom` | Body `{ "path" }` — ESXDOS EEPROM |
| `POST /v1/hardware/beta` | Attach Beta Disk (no media); Refs [#140](https://github.com/mward-sudo/spec_chum/issues/140) |
| `POST /v1/hardware/trdos/rom` | Body `{ "path" }` — 16 KiB TR-DOS ROM / Beta attach; Refs [#140](https://github.com/mward-sudo/spec_chum/issues/140). Disk images remain `POST /v1/trd` |
| `POST`/`DELETE /v1/timex/dock` | Unchanged Timex `.dck` insert/eject |

HTTP wraps real `HostSession` attach APIs — model/ROM errors surface as structured API errors (no fake success).

### `POST /v1/run` budget semantics

`POST /v1/run` advances emulation within a **finite budget** until one of:

1. The budget is exhausted (`stopped_reason: "budget"`),
2. `until_idle` is `true` and the machine becomes idle (`stopped_reason: "idle"`),
3. A debugger stop fires — breakpoint, watch, or `run-until` predicate
   (`stopped_reason: "debug"`).

**Request body** — optional; omitted body is treated as `{ "frames": 100 }`. When a
body is present it must include at least one of `frames` or `instructions` (both
finite, positive); otherwise the server returns **`400`**.

```json
{ "frames": 300, "until_idle": false }
```

`until_idle` defaults to **`false`** — idle is **not** an implicit stop unless the
client opts in. When the budget is hit first, the response includes `frames_run` /
`instructions_run` and the current `inspect` snapshot.

## Crates


| Crate | Role |
| --- | --- |
| `control_plane` | `ControlService` trait + `HostSession` wiring; all ops return `Result` + structured errors |
| `agent_server` | `axum` (or `tiny_http`) loopback server; maps routes → `ControlService` |
| `debug_cli` | HTTP client + human-readable output; local `HostSession` when no agent URL |

A long-lived session unlocks tape mid-load, breakpoint debugging, and framebuffer
grab **after** N frames without respawning the process.

Hosts:

- **Standalone:** `cargo run -p agent_server -- --model 48k` (`spec-chum-agent` binary).
- **Embedded CLI:** `spec_chum --serve --model 48k` (preferred; same HTTP surface).
  Source-build aliases: `spec-chum-debug --serve` / `spec-chum-agent`.
- **HTTP client:** `SPEC_CHUM_AGENT_URL=http://127.0.0.1:17384 spec-chum-debug …`
  or `--agent-url …` on supported subcommands. One-shot media flags include
  `--tap`/`--tzx`, `--snapshot`, and `--trd` / `--trdos-rom` ([#262](https://github.com/mward-sudo/spec_chum/issues/262))
  (maps to `POST /v1/trd` and `POST /v1/hardware/trdos/rom`).
- **Embedded GUI:** egui Debug and optional `SPEC_CHUM_AGENT=1` HTTP share
  one `Arc<ControlPlane>` / `HostSession` (same live PC). Requires
  `SPEC_CHUM_AGENT_TOKEN` or `SPEC_CHUM_AGENT_INSECURE=1`. SpecChumMac: same via
  `SPEC_CHUM_AGENT=1` + embedded server on the live `sc_*` session (see `MACOS_NATIVE.md`).

### Quick test (curl)

```bash
./scripts/fetch_roms.sh
cargo build -p agent_server --release
./target/release/spec-chum-agent --model 48k &
AGENT=http://127.0.0.1:17384

curl -sS "$AGENT/v1/health" | jq .
curl -sS -X POST "$AGENT/v1/run" -H 'Content-Type: application/json' -d '{"frames":1}'
curl -sS "$AGENT/v1/inspect" | jq '.pc'
curl -sS -X POST "$AGENT/v1/regs" -H 'Content-Type: application/json' \
  -d '{"pc":"0x3D00"}' | jq .
curl -sS "$AGENT/v1/framebuffer?border=false&format=png" -o /tmp/spec_paper.png
file /tmp/spec_paper.png   # PNG image data, 256 x 192
```

Optional bearer token: set `SPEC_CHUM_AGENT_TOKEN` on server and pass
`-H "Authorization: Bearer $SPEC_CHUM_AGENT_TOKEN"` on requests.

## Typical automation workflow

```text
1. ./scripts/fetch_roms.sh
2. Start agent server (`spec_chum --serve` / `spec-chum-agent` / `spec-chum-debug --serve`)
3. POST /v1/model { "model": "timex_ts2068" }
4. POST /v1/tape/open + /type-load OR POST /v1/run { "frames": 100 }
5. GET /v1/inspect — assert PC, tape, SCLD fields
6. GET /v1/framebuffer?border=false&format=png — Read PNG for visual QA
7. GET /v1/trace — tape.flash.* events on failure
```

Prefer this over unconstrained OS screenshots or GUI automation.

## Related docs & code

- [DEBUGGING.md](DEBUGGING.md) — trace categories, `spec-chum-debug` today, Inspect fields
- [TIMEX.md](TIMEX.md) — SCLD modes, hi-res 512×192, dock cartridges (#192)
- `crates/host_api/include/spec_chum_host.h` — C ABI for native shells (FFI-only debug entry points)
- `crates/machine/src/inspect.rs`, `debugger.rs`
- `crates/ula/src/lib.rs` — `framebuffer_dims`
- Closed epic [#90](https://github.com/mward-sudo/spec_chum/issues/90) — debugger foundations
- LLM debugging skill (assistants only): [`.cursor/skills/spec-chum-debugging/SKILL.md`](../.cursor/skills/spec-chum-debugging/SKILL.md)

## Alternatives considered

### Internal transports (rejected as primary)

- **Extend `spec-chum-debug` only** — keeps one-shot process model; poor fit for
  framebuffer-after-N-frames, breakpoints, and GUI parity.
- **Stdin/stdout JSON lines** — simple but weak for binary PNG payloads and concurrent clients.
- **Expose raw `sc_*` over FFI to remote tooling** — ties clients to in-process linking;
  HTTP keeps language-agnostic tooling.
- **Unix domain socket + JSON-RPC** — lower overhead, but weaker tooling ergonomics and
  no standard file-download story for framebuffers.
- **gRPC + protobuf** — heavy codegen/deps for localhost-only tooling.

### External emulator protocols (surveyed — not adopted wholesale)

| Protocol | Transport | Fit for Spec Chum automation QA |
| --- | --- | --- |
| **Fuse remote** | None shipped; [feature #100](https://sourceforge.net/p/fuse-emulator/feature-requests/100/) telnet mock-up stalled. GDB only via Spectranet *guest* stub + fork, not an emulator API. | Poor — no stable remote surface; nothing to wrap. |
| **ZEsarUX ZRCP** | Telnet-like TCP (default port 10000); huge text command set (`cpu-step`, `disassemble`, snapshots, memory breakpoints). Used by [DeZog](https://github.com/maziac/DeZog) / VS Code plugins. | Partial for step/peek/disasm; **no** 1:1 PNG framebuffer, **no** `Inspect`-shaped JSON, **no** tape/type-load / Timex dock / SCLD metadata; text parsing is brittle for scripts. |
| **CSpect DZRP** | Binary request/response over socket (DeZogPlugin, port 11000). Toolkit protocol — remotes implement subsets; Next/TBBLUE/sprite oriented. | Good for IDE source-debug with DeZog; **no** framebuffer export, **no** tape automation, Timex/SCLD not covered; requires external plugin DLL. |
| **MAME** | GDB Remote Serial Protocol (`debuggdbstub`, plugin `gdbstub`); Lua `-autoboot_script` for one-shot automation. [mame-mcp](https://github.com/astrobleem/mame-mcp) wraps live sessions in MCP JSON — external bridge, not MAME core. | GDB is CPU/step centric; no Spectrum-specific inspect, tape paths, or guest framebuffer with border/hi-res modes. |
| **RetroArch NCI** | UDP commands (port 55355): `READ_CORE_MEMORY`, `FRAMEADVANCE`, `SCREENSHOT` (writes host screenshot dir). | `SCREENSHOT` is RetroArch-processed output, not guest 1:1 paper/border buffer; UDP hotkeys are flaky under load; core-dependent memory map. |
| **GDB / Z80 RSP** | Serial/TCP GDB stub (`gdb/stubs/z80-stub.c`, [mini-gdbstub](https://github.com/RinHizakura/mini-gdbstub)). | Source-level debug for compiled Z80 targets; no model select, tape load, type-load, trace ring, or framebuffer QA. |
| **Rust emulator patterns** | Ad hoc: JSON-RPC over stdio (plugin hosts), custom HTTP per project (e.g. wasm debugger services). No shared Spectrum/emulator standard. | Patterns confirm **custom localhost API** is normal; nothing to reuse. |

**Conclusion:** existing protocols optimise for **human IDE debugging** (DeZog ↔ ZEsarUX/CSpect) or **generic CPU GDB**, not **Spectrum automation** (long-lived session, rich `Inspect` JSON, tape/type-load, Timex hi-res framebuffer at native dims). None replaces this localhost REST surface.

### Hybrid / compatibility (optional later)

- **REST facade over `control_plane`** remains the architecture — HTTP is transport only.
- **ZRCP or DZRP adapter** on the same backend could help DeZog users, but doubles protocol maintenance; defer unless a concrete consumer appears.
- **Fuse-compatible subset** — no published Fuse remote API to emulate; not worth inventing a faux-Fuse dialect.
- **WebSocket push** (trace, breakpoints, tape progress) — complementary to REST; tracked in [#236](https://github.com/mward-sudo/spec_chum/issues/236) (optional).
- **OpenAPI schema** — machine-readable route catalog; same [#236](https://github.com/mward-sudo/spec_chum/issues/236).
