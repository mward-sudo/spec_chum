# Agent debug control plane (proposed)

> **Status:** Phases A–H partially landed — loopback HTTP API on `127.0.0.1:17384`
> (default). Track remaining work in
> [#210](https://github.com/mward-sudo/spec_chum/issues/210) and
> [`.cursor/skills/spec-chum-debugging/SKILL.md`](../.cursor/skills/spec-chum-debugging/SKILL.md).
>
> **Phase G ([#219](https://github.com/mward-sudo/spec_chum/issues/219)):**
> `GET`/`PATCH /v1/prefs` (session-scoped), `POST /v1/mouse`, `POST /v1/tape/eject`,
> `POST /v1/continue`. Living-room display toggle deferred (not in `UiPreferences`).
> Prefs are **not** written to `ui-prefs.json` from the agent server.
>
> **Phase H ([#220](https://github.com/mward-sudo/spec_chum/issues/220)):**
> `/v1/hardware/*` Multiface / DivMMC / IF1 / MDR / TR-DOS ROM attach over HTTP
> (wraps existing `HostSession` APIs). Accuracy follow-ups remain on #137–#140.

## Motivation

Cursor agents debugging Spec Chum today hit friction that has nothing to do with
emulator accuracy:

- **GUI automation is flaky** — file pickers, ROM dialogs, multi-monitor layouts,
  and host window `screencapture` / `osascript` are unreliable for scripted QA.
- **Parallel surfaces diverge** — `spec-chum-debug` (fresh process per invocation),
  egui **Debug**, SpecChumMac inspector, and raw `host_api` `sc_*` calls duplicate
  semantics with different lifetimes and capabilities.
- **Visual QA needs guest pixels** — Timex hi-res (Techdraw, Death Chase), border
  colour, and SCLD mode checks need the **emulator framebuffer at 1:1**, not a
  scaled CRT or living-room render.

**End-state goal:** agents (and humans via CLI/GUI) **control, inspect, and debug
all emulator aspects** through one localhost API backed by a single Rust service
layer — **no GUI automation required**.

## Architectural principle: single source of truth

All debugging / control / inspect paths **converge** on one backend:

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
    Cursor agents            egui / macOS              spec-chum-debug
    curl / fetch             (Phase B)                 (Phase B)
```

- The **HTTP server is a thin transport** over the shared crate (or an embedded
  call into the same types when the host runs in-process).
- **`spec-chum-debug` becomes a client** of that API (or a thin wrapper over the
  same service crate) — not a parallel code path forever.
- **egui Debug** and **SpecChumMac Debug** eventually call the same backend
  (in-process handle or loopback HTTP to the embedded server).
- **Agents prefer HTTP**; humans use GUI/CLI with **identical semantics**.

### Migration phases

| Phase | Deliverable |
| --- | --- |
| **A — API + parallel surfaces** | Shared `control_plane` crate; localhost HTTP server (`spec-chum-agentd` or embedded in a long-lived host); MVP+ endpoints; existing CLI/GUI unchanged |
| **B — clients adapt** | `spec-chum-debug` talks HTTP when `SPEC_CHUM_AGENT_URL` set; egui Debug on shared `HostSession`; `SPEC_CHUM_AGENT=1` embeds HTTP on that same plane; integration tests hit HTTP |
| **C — dedupe** | Deprecate duplicate direct `host_api` debug/control paths where safe; document remaining C ABI as FFI-only for non-Rust shells |

**Phase B (partial — [#221](https://github.com/mward-sudo/spec_chum/issues/221) open):**

| Piece | Status |
| --- | --- |
| `spec-chum-debug` HTTP client (`SPEC_CHUM_AGENT_URL` / `--agent-url`) | Done ([#213](https://github.com/mward-sudo/spec_chum/pull/213) / [#214](https://github.com/mward-sudo/spec_chum/pull/214)) |
| Mem watches over HTTP | Done |
| SpecChumMac agent workflow docs | Done ([#225](https://github.com/mward-sudo/spec_chum/pull/225) / `MACOS_NATIVE.md`) |
| egui `SPEC_CHUM_AGENT=1` embed | **Done** — thin transport over the GUI `Arc<ControlPlane>` / shared `HostSession` (same live machine as Debug) |
| egui Debug panel live session | **Done** — Debug routes through shared [`HostSession`](../crates/host_api/src/session.rs) behind `EmulatorSession` / `ControlPlane` |
| SpecChumMac in-process embed | **Deferred** — keep FFI + standalone agent; in-process only if cycle-safe (no `host_api` ↔ `control_plane`) |

**Phase C (docs complete — [#222](https://github.com/mward-sudo/spec_chum/issues/222)):**
`spec-chum-debug` local commands route through
[`HostSession`](../crates/host_api/src/session.rs) — the same type wrapped by
`control_plane::ControlPlane` and the agent HTTP server. The one-shot CLI no longer
constructs a parallel `machine::Machine` path.

**Primary surfaces (prefer these):**

| Consumer | Surface |
| --- | --- |
| Agents / automation | Loopback HTTP (`control_plane` + `agent_server`) |
| Rust CLI | `spec-chum-debug` → `HostSession` locally, or HTTP client when `SPEC_CHUM_AGENT_URL` set |
| Rust library hosts | `control_plane::ControlPlane` / `HostSession` (no C ABI) |

**C ABI = FFI-only:** `sc_debug_*`, `sc_inspect_json`, `sc_peek` / `sc_poke`,
`sc_step`, `sc_add_breakpoint`, `sc_run_until_break`, and related entry points in
[`spec_chum_host.h`](../crates/host_api/include/spec_chum_host.h) remain **thin
wrappers** over `HostSession` + the global `trace` ring for **non-Rust shells**
(SpecChumMac Swift, future foreign-language hosts). They are **not** the agent
primary API and must **not** gain a `host_api` → `control_plane` dependency
(avoids a crate cycle).

#### Remaining parallel-path inventory

| Path | Disposition |
| --- | --- |
| C ABI `sc_debug_*` / inspect / step / breakpoints | **Keep as FFI-only** — SpecChumMac / non-Rust |
| egui `EmulatorSession` Debug panel | **Uses shared `HostSession`** via `Arc` + `ControlPlane` (#221) |
| egui `SPEC_CHUM_AGENT=1` embedded server | **Thin transport** over the GUI plane (same live session as Debug) |
| SpecChumMac inspector / `sc_*` | **Keep as FFI** — document agent workflow via standalone `spec-chum-agent` ([#221](https://github.com/mward-sudo/spec_chum/issues/221) docs); in-process `ControlPlane` only if cycle-safe |
| `spec-chum-debug` local (no agent URL) | **Keep** — already on `HostSession` (same type as `control_plane`) |
| Direct `machine::Machine` in debug CLI | **Removed** (Phase C partial / [#215](https://github.com/mward-sudo/spec_chum/pull/215)) |

Phase A–H HTTP rows continue on [#210](https://github.com/mward-sudo/spec_chum/issues/210).
egui in-process HTTP↔GUI share landed for #221; SpecChumMac in-process integration remains deferred.
Field privatize follow-up: [#227](https://github.com/mward-sudo/spec_chum/issues/227).

Phase A is mergeable without breaking current workflows.

## Technology choice: REST on loopback

| Option | Verdict |
| --- | --- |
| **HTTP REST on `127.0.0.1`** | **Chosen.** Agents already speak HTTP; PNG bodies and JSON inspect fit naturally; easy `curl`/MCP fetch; optional OpenAPI; debuggable in a browser tab. |
| Unix domain socket + JSON-RPC | Lower overhead, but poorer agent ergonomics and no standard file-download story for framebuffers. |
| gRPC + protobuf | Heavy codegen/deps for a localhost-only tool; poor fit for “save this PNG”. |
| **WebSocket (optional)** | **Later** — push trace events, breakpoint notifications, tape progress; not required for MVP. |

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

> **“Screenshots” means guest framebuffer export — not OS window capture.**

Agents must **not** use `screencapture`, multi-monitor grabs, or living-room CRT
photos for emulator visual QA. The API exports the same RGBA buffer hosts already
expose via `sc_framebuffer_ptr` / `HostSession::framebuffer()`:

| Query | Meaning |
| --- | --- |
| `border=false` | **Paper only** — active display file at native resolution |
| `border=true` | Paper + ULA border (Spec Chum layout; bottom border taller) |
| `format=png` | `image/png` body (default for agents) |
| `format=rgba` | Raw RGBA8 row-major (`width × height × 4` bytes) |

**1:1 native dimensions** (from `ula::framebuffer_dims`, no host scaling, no CRT
filter, no living-room post-process):

| Mode | Paper (`border=false`) | With border (`border=true`) |
| --- | --- | --- |
| Sinclair lo-res (48K/128K/+3/TC2048 TS2068 lo modes) | **256×192** | **352×296** |
| Timex SCLD hi-res (modes 4–7) | **512×192** | **640×296** |

Response headers / JSON metadata include `width`, `height`, `border`, `hires`,
`scld_mode` (when Timex), and `model` so agents validate size before visual diff.

**Not this endpoint:**

- Living-room Bevy CRT (experimental display mode) — scaled, shaded, halation.
- egui/macOS window bitmap — host DPI, chrome, optional zoom.
- Audio waveform or border-event trace (use `/v1/inspect` + trace instead).

Example (once implemented):

```bash
curl -sS -H "Authorization: Bearer $SPEC_CHUM_AGENT_TOKEN" \
  'http://127.0.0.1:17384/v1/framebuffer?border=false&format=png' \
  -o /tmp/spec_paper.png
# Agent: Read /tmp/spec_paper.png for Techdraw hi-res QA
```

## API surface (full end-state)

Phased delivery below; **acceptance** requires every row before the issue closes.

### Control

| Area | Operations |
| --- | --- |
| Machine | `POST /v1/model` — select built-in model; `POST /v1/config` — apply `#187` custom profile JSON; `POST /v1/reset`; `POST /v1/running` pause/run; `POST /v1/run` — advance within a **finite budget** (see below) |
| Execution | `POST /v1/step` — one `step_once`; `POST /v1/step` body `{ "count": N }`; `POST /v1/continue` — resume after debugger stop (`continue_from_pc`); `POST /v1/run-until` — PC / budget (maps `Debugger::run_until`) |
| Tape | `POST /v1/tape/open`, `/play`, `/pause`, `/rewind`, `/eject`; load options flash vs EAR vs experience + speed |
| Type-load | `POST /v1/type-load` — scripted LOAD "" [CODE] (today's `type-load` subcommand) |
| ROM | `POST /v1/rom` — load ROM image from host filesystem path |
| Media | `POST /v1/snapshot`, `/rzx`, `/dsk`, `/trd`, … |
| Input | `POST /v1/keys` matrix press/release; `POST /v1/joystick`; `POST /v1/mouse` Kempston delta/buttons |
| Hardware | `GET /v1/hardware` — attach flags; `POST /v1/hardware/multiface` (+ `/nmi`); `POST /v1/hardware/interface1` (+ `/rom`, `/v1/hardware/mdr`); `POST /v1/hardware/divmmc` (+ `/sd`, `/eeprom`); `POST /v1/hardware/trdos/rom`; Timex `.dck` via `/v1/timex/dock` |
| Host prefs | `GET` / `PATCH /v1/prefs` — volume, mute, joystick mode, tape defaults, throttle, Kempston mouse enable (session-scoped in agent server; living-room display toggle deferred) |
| Border | `POST /v1/border` — `with_border` flag (changes framebuffer dims) |

### Inspect

| Area | Operations |
| --- | --- |
| Core | `GET /v1/inspect` — full `Inspect` JSON (CPU, raster, paging, tape, AY, Timex fields) |
| Video | `GET /v1/framebuffer` — PNG or RGBA (see above); `GET /v1/video` — dims + SCLD mode without pixels |
| Memory | `GET /v1/peek?addr=&len=`; `POST /v1/poke`; optional `GET /v1/memory/regions` for paged views |
| Disasm | `GET /v1/disasm?addr=&count=` |
| Debugger state | `GET /v1/debug/breakpoints`, `/watches`, `/last-break` |
| ROM | `GET /v1/rom/setup` — slots + availability (`sc_model_rom_setup_json` parity) |
| Status | `GET /v1/status`, `/v1/health`; `GET /v1/errors/last` |
| Prefs | `GET /v1/prefs` snapshot |

### Debug

| Area | Operations |
| --- | --- |
| Breakpoints | `POST /v1/debug/breakpoints/pc`; `DELETE`…; mem/port watches |
| Trace | `GET /v1/trace/categories` — list enabled categories; `PUT /v1/trace/categories` — enable/disable; `POST /v1/trace/clear`; `GET /v1/trace` — ring text/JSON/ndjson |
| Run control | `POST /v1/run-until` — PC, mem write, port, halt, insn budget |
| Step semantics | `step` = one instruction; `step-over` deferred until call-stack support exists — document as optional |

### MVP slice (Phase A first merge)

Smallest useful agent surface:

1. `GET /v1/health`
2. `GET /v1/inspect`
3. `GET /v1/framebuffer` (PNG + rgba; `border` query)
4. `POST /v1/model`, `/reset`, `/run`
5. `POST /v1/tape/open`, `/play`, `/pause`, load-options, `/type-load`
6. `GET /v1/peek`, `GET /v1/disasm`
7. `GET /v1/trace/categories`, `PUT /v1/trace/categories`, `GET /v1/trace`

Remaining control/inspect/debug rows land in later phases on [#210](https://github.com/mward-sudo/spec_chum/issues/210).

### Phase G — prefs / mouse / eject / continue

| Route | Notes |
| --- | --- |
| `GET /v1/prefs` | Session snapshot: `volume`, `muted`, `throttle`, `joystick_mode`, `kempston_mouse`, `tape_experience`, `tape_ear_speed` |
| `PATCH /v1/prefs` | Partial update; applies joystick mode + tape load options to the loaded machine when present. Does **not** persist `ui-prefs.json`. |
| `POST /v1/mouse` | Requires `kempston_mouse: true` in prefs. Body `{ "dx", "dy", "left", "right", "middle" }` and/or `{ "clear": true }` (full axis+button reset) — Kempston mouse via `HostSession` ([#136](https://github.com/mward-sudo/spec_chum/issues/136)) |
| `POST /v1/tape/eject` | Clears the inserted TAP/TZX deck |
| `POST /v1/continue` | `continue_from_pc` after a debugger stop; JSON `{ "reason", "paused" }` |

Living-room display toggle is **deferred** (not modeled in shared prefs yet).

### Phase H — peripherals HTTP attach

| Route | Notes |
| --- | --- |
| `GET /v1/hardware` | `{ has_multiface, has_interface1, has_divmmc, has_timex_dock }` |
| `POST /v1/hardware/multiface` | Body `{ "path" }` — 8 KiB Multiface 1 ROM (48K only); Refs [#137](https://github.com/mward-sudo/spec_chum/issues/137) |
| `POST /v1/hardware/multiface/nmi` | Red-button NMI when attached |
| `POST /v1/hardware/interface1` | Attach IF1 (loads `roms/if1*.rom` if present); Refs [#139](https://github.com/mward-sudo/spec_chum/issues/139) |
| `POST /v1/hardware/interface1/rom` | Body `{ "path" }` — explicit IF1 ROM |
| `POST /v1/hardware/mdr` | Body `{ "path" }` — Microdrive cartridge (attaches IF1) |
| `POST /v1/hardware/divmmc` | Attach DivMMC (no media); Refs [#138](https://github.com/mward-sudo/spec_chum/issues/138) |
| `POST /v1/hardware/divmmc/sd` | Body `{ "path" }` — flat SD image |
| `POST /v1/hardware/divmmc/eeprom` | Body `{ "path" }` — ESXDOS EEPROM |
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

## Implementation sketch

Suggested crates (names tentative):

| Crate | Role |
| --- | --- |
| `control_plane` | `ControlService` trait + `HostSession` wiring; all ops return `Result` + structured errors |
| `agent_server` | `axum` (or `tiny_http`) loopback server; maps routes → `ControlService` |
| `debug_cli` | HTTP client + human-readable output; `--local` escape hatch for offline tests only until Phase C |

Long-lived session (contrast with today’s one-shot CLI) unlocks tape mid-load,
breakpoint debugging, and framebuffer grab **after** N frames without respawn.

Hosts:

- **Standalone:** `cargo run -p agent_server -- --model 48k` (`spec-chum-agent` binary).
- **Embedded CLI:** `spec-chum-debug --serve --model 48k` (same HTTP surface).
- **HTTP client (Phase B):** `SPEC_CHUM_AGENT_URL=http://127.0.0.1:17384 spec-chum-debug …`
  or `--agent-url …` on supported subcommands.
- **Embedded GUI (Phase B):** egui Debug and optional `SPEC_CHUM_AGENT=1` HTTP share
  one `Arc<ControlPlane>` / `HostSession` (same live PC). Requires
  `SPEC_CHUM_AGENT_TOKEN` or `SPEC_CHUM_AGENT_INSECURE=1`. SpecChumMac: standalone
  `spec-chum-agent` / `spec-chum-debug --serve` until in-process is cycle-safe.

### Quick test (curl)

```bash
./scripts/fetch_roms.sh
cargo build -p agent_server --release
./target/release/spec-chum-agent --model 48k &
AGENT=http://127.0.0.1:17384

curl -sS "$AGENT/v1/health" | jq .
curl -sS -X POST "$AGENT/v1/run" -H 'Content-Type: application/json' -d '{"frames":1}'
curl -sS "$AGENT/v1/inspect" | jq '.regs.pc'
curl -sS "$AGENT/v1/framebuffer?border=false&format=png" -o /tmp/spec_paper.png
file /tmp/spec_paper.png   # PNG image data, 256 x 192
```

Optional bearer token: set `SPEC_CHUM_AGENT_TOKEN` on server and pass
`-H "Authorization: Bearer $SPEC_CHUM_AGENT_TOKEN"` on requests.

## Agent workflow (target)

```text
1. ./scripts/fetch_roms.sh
2. Start agent server (`spec-chum-agent` / `spec-chum-debug --serve`)
3. POST /v1/model { "model": "timex_ts2068" }
4. POST /v1/tape/open + /type-load OR POST /v1/run { "frames": 100 }
5. GET /v1/inspect — assert PC, tape, SCLD fields
6. GET /v1/framebuffer?border=false&format=png — Read PNG for visual QA
7. GET /v1/trace — tape.flash.* events on failure
```

Prefer this over `screencapture`, osascript, or computer-use GUI driving.

## Related docs & code

- [DEBUGGING.md](DEBUGGING.md) — trace categories, `spec-chum-debug` today, Inspect fields
- [TIMEX.md](TIMEX.md) — SCLD modes, hi-res 512×192, dock cartridges (#192)
- `crates/host_api/include/spec_chum_host.h` — today's C ABI (Phase C dedupe target)
- `crates/machine/src/inspect.rs`, `debugger.rs`
- `crates/ula/src/lib.rs` — `framebuffer_dims`
- Closed epic [#90](https://github.com/mward-sudo/spec_chum/issues/90) — debugger foundations

## Alternatives considered

### Internal transports (rejected as primary)

- **Extend `spec-chum-debug` only** — keeps one-shot process model; poor fit for
  framebuffer-after-N-frames, breakpoints, and GUI parity.
- **Stdin/stdout JSON lines** — simple but weak for binary PNG payloads and concurrent agents.
- **Expose raw `sc_*` over FFI from agents** — ties agents to in-process linking;
  HTTP keeps language-agnostic tooling.
- **Unix domain socket + JSON-RPC** — lower overhead, but poorer agent ergonomics and
  no standard file-download story for framebuffers.
- **gRPC + protobuf** — heavy codegen/deps for localhost-only tooling.

### External emulator protocols (surveyed — not adopted wholesale)

| Protocol | Transport | Fit for Spec Chum agent QA |
| --- | --- | --- |
| **Fuse remote** | None shipped; [feature #100](https://sourceforge.net/p/fuse-emulator/feature-requests/100/) telnet mock-up stalled. GDB only via Spectranet *guest* stub + fork, not an emulator API. | Poor — no stable remote surface; nothing to wrap. |
| **ZEsarUX ZRCP** | Telnet-like TCP (default port 10000); huge text command set (`cpu-step`, `disassemble`, snapshots, memory breakpoints). Used by [DeZog](https://github.com/maziac/DeZog) / VS Code plugins. | Partial for step/peek/disasm; **no** 1:1 PNG framebuffer, **no** `Inspect`-shaped JSON, **no** tape/type-load / Timex dock / SCLD metadata; text parsing is brittle for agents. |
| **CSpect DZRP** | Binary request/response over socket (DeZogPlugin, port 11000). Toolkit protocol — remotes implement subsets; Next/TBBLUE/sprite oriented. | Good for IDE source-debug with DeZog; **no** framebuffer export, **no** tape automation, Timex/SCLD not covered; requires external plugin DLL. |
| **MAME** | GDB Remote Serial Protocol (`debuggdbstub`, plugin `gdbstub`); Lua `-autoboot_script` for one-shot automation. [mame-mcp](https://github.com/astrobleem/mame-mcp) wraps live sessions in MCP JSON — external bridge, not MAME core. | GDB is CPU/step centric; no Spectrum-specific inspect, tape paths, or guest framebuffer with border/hi-res modes. |
| **RetroArch NCI** | UDP commands (port 55355): `READ_CORE_MEMORY`, `FRAMEADVANCE`, `SCREENSHOT` (writes host screenshot dir). | `SCREENSHOT` is RetroArch-processed output, not guest 1:1 paper/border buffer; UDP hotkeys are flaky under load; core-dependent memory map. |
| **GDB / Z80 RSP** | Serial/TCP GDB stub (`gdb/stubs/z80-stub.c`, [mini-gdbstub](https://github.com/RinHizakura/mini-gdbstub)). | Source-level debug for compiled Z80 targets; no model select, tape load, type-load, trace ring, or framebuffer QA. |
| **Rust emulator patterns** | Ad hoc: JSON-RPC over stdio (plugin hosts), custom HTTP per project (e.g. wasm debugger services). No shared Spectrum/emulator standard. | Patterns confirm **custom localhost API** is normal; nothing to reuse. |

**Conclusion:** existing protocols optimise for **human IDE debugging** (DeZog ↔ ZEsarUX/CSpect) or **generic CPU GDB**, not **agent automation** (long-lived session, rich `Inspect` JSON, tape/type-load, Timex hi-res framebuffer at native dims). None is a drop-in substitute for the planned REST surface.

### Hybrid / compatibility (optional later, not MVP)

- **REST facade over `control_plane`** remains the architecture — HTTP is transport only.
- **ZRCP or DZRP adapter** on the same backend could help DeZog users, but doubles protocol maintenance; defer unless a concrete consumer appears.
- **Fuse-compatible subset** — no published Fuse remote API to emulate; not worth inventing a faux-Fuse dialect.
- **WebSocket push** (trace, breakpoints, tape progress) — complementary to REST; listed as Phase A+ optional in [#210](https://github.com/mward-sudo/spec_chum/issues/210).
