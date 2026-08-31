# Agent debug control plane (proposed)

> **Status:** proposed / **not implemented yet**. Track implementation in
> [#210](https://github.com/mward-sudo/spec_chum/issues/210) and
> [`.cursor/skills/spec-chum-debugging/SKILL.md`](../.cursor/skills/spec-chum-debugging/SKILL.md).

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

```
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
| **B — clients adapt** | `spec-chum-debug` talks HTTP by default; egui Debug + macOS inspector route through backend; integration tests hit HTTP |
| **C — dedupe** | Deprecate duplicate direct `host_api` debug/control paths where safe; document remaining C ABI as FFI-only for non-Rust shells |

Phase A is mergeable without breaking current workflows. Phases B/C are explicit
follow-ups in the tracker issue.

## Technology choice: REST on loopback

| Option | Verdict |
| --- | --- |
| **HTTP REST on `127.0.0.1`** | **Chosen.** Agents already speak HTTP; PNG bodies and JSON inspect fit naturally; easy `curl`/MCP fetch; optional OpenAPI; debuggable in a browser tab. |
| Unix domain socket + JSON-RPC | Lower overhead, but poorer agent ergonomics and no standard file-download story for framebuffers. |
| gRPC + protobuf | Heavy codegen/deps for a localhost-only tool; poor fit for “save this PNG”. |
| **WebSocket (optional)** | **Later** — push trace events, breakpoint notifications, tape progress; not required for MVP. |

### Security

- Bind **`127.0.0.1` only** (or `::1`); reject other interfaces.
- Optional bearer token via `SPEC_CHUM_AGENT_TOKEN` (or `--token`); default off on
  trusted dev machines.
- No TLS (localhost); document that the server must never be exposed publicly.

Default port: **`17384`** (`SPEC_CHUM_AGENT_PORT`, mnemonic *SPEC* on phone keypad)
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
| Machine | `POST /v1/model` — select built-in model; `POST /v1/config` — apply `#187` custom profile JSON; `POST /v1/reset`; `POST /v1/running` pause/run; `POST /v1/run` — advance *N* frames or until idle |
| Execution | `POST /v1/step` — one `step_once`; `POST /v1/step` body `{ "count": N }`; `POST /v1/continue`; `POST /v1/run-until` — PC / budget (maps `Debugger::run_until`) |
| Tape | `POST /v1/tape/open`, `/play`, `/pause`, `/rewind`, `/eject`; load options flash vs EAR vs experience + speed |
| Type-load | `POST /v1/tape/type-load` — scripted LOAD "" [CODE] (today's `type-load` subcommand) |
| Media | `POST /v1/snapshot`, `/rzx`, `/dsk`, `/trd`, … |
| Input | `POST /v1/keys` matrix press/release; `POST /v1/joystick`; Kempston mouse delta/buttons |
| Hardware | Multiface, DivMMC, Interface 1, Timex `.dck` dock insert/eject, Beta/TR-DOS ROM attach |
| Host prefs | `GET|PATCH /v1/prefs` — volume, mute, joystick mode, tape defaults, throttle, living-room toggle (host display only) |
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
| Trace | `GET|PUT /v1/trace/categories`; `POST /v1/trace/clear`; `GET /v1/trace` — ring text/JSON/ndjson |
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
7. `POST /v1/trace/categories`, `GET /v1/trace`

Remaining control/inspect/debug rows land in Phase A follow-ups before Phase B.

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

- **Standalone:** `cargo run -p agent_server` (headless agent daemon).
- **Embedded:** egui / SpecChumMac spawn server on startup when
  `SPEC_CHUM_AGENT=1` (default off until stable).

## Agent workflow (target)

```text
1. ./scripts/fetch_roms.sh
2. Start agent server (daemon or host with SPEC_CHUM_AGENT=1)
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

- **Extend `spec-chum-debug` only** — keeps one-shot process model; poor fit for
  framebuffer-after-N-frames, breakpoints, and GUI parity.
- **Stdin/stdout JSON lines** — simple but weak for binary PNG payloads and concurrent agents.
- **Expose raw `sc_*` over FFI from agents** — ties agents to in-process linking;
  HTTP keeps language-agnostic tooling.
