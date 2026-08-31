---
name: spec-chum-debugging
description: >-
  Debug Spec Chum ZX Spectrum emulator via spec-chum-debug, Inspect JSON,
  breakpoints, watches, disasm, tape load (flash vs EAR), and SPEC_CHUM_*
  trace categories. Use when debugging the emulator, tape load failures,
  LD-BYTES, EAR polarity, flash-load, type-load, inspect dumps, Fuse/z80test
  mismatches, host_api sc_debug_*, egui Debug window, or the Agent Debug HTTP
  API (localhost control plane, Phase A implemented).
---

# Spec Chum debugging

Project debugger workflow for agents. Full detail: [`docs/DEBUGGING.md`](../../../docs/DEBUGGING.md).

## Choose a surface

| Goal | Use |
| --- | --- |
| **Long-lived control + 1:1 framebuffer PNG** | **Agent Debug HTTP API** — `spec-chum-agent` or `spec-chum-debug --serve` |
| Scripted repro via API client | `SPEC_CHUM_AGENT_URL=… spec-chum-debug …` (Phase B subset) |
| Scripted repro, Inspect JSON, breakpoints, tape `type-load` | Headless **`spec-chum-debug`** (local one-shot process) |
| Regression / harness / matrix | **`cargo test -p machine`** (and `tape` / `trace` / `z80`) |
| Interactive Pause / Step / disasm UI | **egui** `cargo run -p app` → Menu **Debug** |
| Native SwiftUI / C host | **`host_api`** `sc_debug_*` / `sc_inspect_json` |
| macOS shell (limited inspector) | `./scripts/run_macos_app.sh` + same env; prefer CLI for deep steps |

Each `spec-chum-debug` invocation is a **new process** (fresh machine + empty trace ring). Put `--trace`, `--tap`/`--tzx`, `--snapshot`, and the subcommand on the **same** command.

## Agent Debug API (unified control plane)

> **Implemented (Phase A):** loopback HTTP on `127.0.0.1:17384` (default).
> Design: [`docs/AGENT_DEBUG_API.md`](../../../docs/AGENT_DEBUG_API.md).
> Tracker: [#210](https://github.com/mward-sudo/spec_chum/issues/210).

**Unification goal:** one shared Rust backend (`control_plane`) serves HTTP;
`spec-chum-debug --serve` / `spec-chum-agent` ship today; egui Debug and
SpecChumMac converge in Phase B/C.

### Start server

```bash
./scripts/fetch_roms.sh
cargo build -p agent_server --release
./target/release/spec-chum-agent --model 48k
# or: spec-chum-debug --serve --model 48k --tap path.tap
```

### When to prefer the API

- Visual QA (Techdraw hi-res, Timex SCLD, border colour) — **export guest framebuffer
  at 1:1**, not OS `screencapture` or living-room CRT.
- Long sessions: tape load mid-run, breakpoints, grab PNG after N frames.
- Avoid GUI automation (file pickers, ROM dialogs, multi-monitor).

### Embedded agent in egui (Phase B)

```bash
SPEC_CHUM_AGENT=1 cargo run -p app --release
# Optional: SPEC_CHUM_AGENT_MODEL=timex_ts2068 SPEC_CHUM_AGENT_PORT=17384
curl -sS http://127.0.0.1:17384/v1/health | jq .
```

This starts a **parallel** debug session (not yet the GUI's live machine). For shared
host state, use standalone `spec-chum-agent` or macOS with a separate server process.

### HTTP client (Phase B)

```bash
export SPEC_CHUM_AGENT_URL=http://127.0.0.1:17384
spec-chum-debug --model 48k run --frames 1
spec-chum-debug --tap tests/fixtures/tape/attr_mark.tap type-load --code
```

Remote mode supports: `run`, `dump-state`, `dump-trace`, `peek`, `disasm`, `type-load`,
`until-pc`, `break-pc`, `watch-write`. Requires a running agent (`spec-chum-agent`,
`spec-chum-debug --serve`, or egui with `SPEC_CHUM_AGENT=1`). `--rom` / `--snapshot`
remain local-only (no HTTP upload yet — #210).

### Framebuffer export (not window capture)

`GET /v1/framebuffer?border=false|true&format=png|rgba` returns the emulator RGBA
buffer (`sc_framebuffer_*` / `ula::framebuffer_dims`) — **exact guest pixels**:

| Mode | Paper | With border |
| --- | --- | --- |
| Lo-res | 256×192 | 352×296 |
| Timex hi-res | 512×192 | 640×296 |

- **Do** `Read` the saved PNG for visual assertions.
- **Do not** capture the macOS/egui window or living-room Bevy view (scaled CRT).

### Migration (summary)

| Phase | What |
| --- | --- |
| A | Shared backend + HTTP server; parallel CLI/GUI |
| B | `spec-chum-debug` + Debug UIs become API clients |
| C | Dedupe redundant `host_api` debug paths where safe |

Until Phase B ships, use `spec-chum-debug` + Inspect JSON below.

## Quick start

```text
- [ ] ROMs present (`roms/…`); else `./scripts/fetch_roms.sh`
- [ ] `cargo build -p debug_cli`
- [ ] Prefer Read/Grep/Glob for source; Shell for cargo / binary / `./scripts/check.sh` / `gh`
- [ ] Enable trace on the command that generates events (`--trace` or SPEC_CHUM_*)
- [ ] For tape: decide flash (default) vs `--ear-load`; pick model; verify `load_ok` / peek
```

## Build & common recipes

```bash
./scripts/fetch_roms.sh          # if roms/ missing
cargo build -p debug_cli
BIN=./target/debug/spec-chum-debug

# Inspect
$BIN --model 48k run --frames 1
$BIN --model 48k --json dump-state

# PC / mem
$BIN until-pc 056C --max 10000000
$BIN break-pc 8000 --frames 200
$BIN watch-write 4000 --max 10000000
$BIN peek 4000 --len 64
$BIN disasm --count 16
$BIN disasm --addr 056C --count 8

# Snapshot / model
$BIN --snapshot path.z80 --json dump-state
$BIN --model 128k --json dump-state
$BIN --model plus3 run --frames 1
```

Addresses: hex (`056C`, `0x056C`, `$5C00`) or decimal.

Global flags (before subcommand): `--model 48k|128k|plus3`, `--rom PATH`, `--tap` / `--tzx`, `--snapshot`, `--trace LIST`, `--json`, `--ear-load`, `--speed N` (1..=64, EAR only).

### Subcommands

| Cmd | Purpose |
| --- | --- |
| `run --frames N` | Run frames; print Inspect |
| `dump-state` | Inspect only |
| `dump-trace [--last N]` | Ring dump (`--json` → JSON) |
| `peek ADDR [--len N]` | Hexdump |
| `disasm [--addr A] [--count N]` | Disasm at PC or addr |
| `until-pc PC [--max N]` | Run until PC break |
| `break-pc PC [--frames N]` | PC break over frames |
| `watch-write ADDR [--max N]` | Break on mem write |
| `type-load [--code] [--warmup N] [--max N]` | Boot, type LOAD, wait for load |

## Tape debugging

Default load mode is **instant flash-load** at ROM `LD-BYTES` (`0x056C`). `--ear-load` disables the trap and uses the EAR bitstream; `--speed N` runs N Spectrum frames per `run_frame` while the deck is playing (wall-clock ≈ realtime/N; pulse widths stay ROM-accurate).

### Recipes

```bash
# Flash CODE (attr_mark → bytes at 0x8000, attr mark 0xD7 @ 0x5800)
$BIN --model 48k --tap tests/fixtures/tape/attr_mark.tap type-load --code
$BIN --model 128k --tap tests/fixtures/tape/attr_mark.tap type-load --code
$BIN --model plus3 --tap tests/fixtures/tape/attr_mark.tap type-load --code

# PROGRAM (print_ok)
$BIN --tap tests/fixtures/tape/print_ok.tap type-load

# EAR (raise --max; default EAR max is 200000 frames)
$BIN --model 48k --tap tests/fixtures/tape/attr_mark.tap \
  --ear-load --speed 10 type-load --code --max 200000

# Custom flag after CODE (Boggit-style) — flash for CODE, then USR path
$BIN --model 128k --tap tests/fixtures/tape/custom_loader.tap type-load --code

# Same-process tape + append (do NOT dump-trace in a second process)
SPEC_CHUM_TRACE_FILE=/tmp/sc_append.txt SPEC_CHUM_TRACE_APPEND=1 \
  $BIN --trace tape --tap tests/fixtures/tape/attr_mark.tap type-load --code
```

Success signals from `type-load`:

- Text: `load_ok=true` (exit 0); JSON includes `"load_ok":true`
- CODE/`attr_mark`: peek `0x8000` starts `21 00 58 36 D7 C9`; attr `0x5800 == 0xD7`
- Inspect tape fields: `playing`, `flash_load`, `speed`, `block_index` / `block_count`

### Model notes (lessons from #99–#101 / PR #102)

- **128K / +3 `type-load`**: enters **48 BASIC** (cursor menu), then keyword `LOAD ""` [CODE] — not Tape Loader alone. Prefer this path for CODE fixtures.
- **EAR**: sync pulses must keep alternating after an odd-length leader; turbo speed must not shrink data pulse widths. If `load_ok=false` but blocks advance, suspect EAR/sync — compare flash control run.
- **Tape paused until after typing**: CLI starts deck paused, types LOAD, then Play — matches ROM wait-for-Play.
- **Fixtures**: see `tests/fixtures/tape/README.md` (matrix, `custom_loader.tap`, optional Boggit local path).

### Flash-load trace clues

Enable `--trace tape` or `SPEC_CHUM_TRACE=tape`. Typical success: `tape.flash.enter` → `tape.flash.exit ok=1`. Skips: `wrong_flag`, `length_mismatch`, `checksum_fail`, `paused`, `no_block`. At `0x056C`, flag/load-verify live in **A′/F′** (after `EX AF,AF'`).

Machine harnesses:

```bash
cargo test -p machine attr_mark_load_path_must_succeed -- --nocapture
cargo test -p machine --lib matrix   # flash + EAR matrix (ROMs required)
```

## Trace env & flush

| Var / flag | Effect |
| --- | --- |
| `SPEC_CHUM_DEBUG=1` | Default cats: `bus,tape,ula,machine` (not full CPU) |
| `SPEC_CHUM_TRACE=tape,cpu` / `all` / `default` | Explicit category list |
| `SPEC_CHUM_TRACE_CAPACITY=N` | Ring size (default 8192) |
| `SPEC_CHUM_TRACE_CPU_EVERY=N` | Sample every Nth CPU step |
| `SPEC_CHUM_TRACE_FILE=path` | Path for append / `dump_to_env_file` |
| `SPEC_CHUM_TRACE_APPEND=1` | Append+flush each event to that file |
| CLI `--trace LIST` | Same categories; enabled **before** tape insert / `type-load` |

Categories: `cpu`, `bus`, `tape`, `ula`, `machine`, `ay`, `disk`, `mem`.

**Caveats:**

- Ring is **per-process**. `dump-trace` cannot see another PID’s events.
- Prefer `SPEC_CHUM_TRACE_APPEND=1` + `--trace` on the **same** run for agent-visible files (PR #101 flush fix).
- Prefer `tape` / `default` while debugging loads; add `cpu` only for short windows.

C ABI: `sc_debug_init_from_env`, `sc_debug_set_categories`, `sc_debug_dump` / `_json` / `_to_file`, `sc_debug_clear`, `sc_debug_event_count` (free strings with `sc_string_free`).

## Interpreting Inspect / failures

**Inspect** (`dump-state`, `run`, `--json`): regs, `cpu_t`, `frame_t` / raster, `INT`, `contend_at_pc`, border/EAR/MIC, paging (`7FFD`/`1FFD`/ROM/`C000`), optional `tape` + `ay_regs`.

JSON: `--json dump-state` or `type-load --json` wraps `inspect` + `load_ok`.

**Fuse** (`cargo test -p z80 fuse_all_vectors --offline -- --nocapture`): failed vector prints got/want regs/mem/T-states and a **start-PC** disasm window (no global `trace` crate).

**z80test** (`--features slow-tests`): fail/timeout panics with RST10 capture + `Machine::inspect()` Display. Keep `z80doc` and `z80full` green under the feature (CI runs `z80doc` by name).

**Contention / beam**: Inspect `frame_t`, line/`x`, `INT`, `contend_pc`; enable `bus` for contend/floating events.

**Paging**: Inspect `7FFD`/`1FFD`; trace `bus,machine`.

## egui Debug window

`cargo run -p app` with env at process start. Menu **Debug**: toggle categories, clear ring, dump stderr/file. Debugger window: Pause / Step / Run, Inspect text, disasm at PC, hexdump, PC breakpoints, last ring events.

## Agent tool preference

- **Read / Grep / Glob / StrReplace / Write** for files (not `cat`/`sed`/`rg`/`find` in Shell).
- **Shell** for `cargo`, `./scripts/check.sh`, `./scripts/fetch_roms.sh`, running `spec-chum-debug`, and `gh`.
- **Visual QA:** prefer API framebuffer PNG (`GET /v1/framebuffer`) when available; never `screencapture` / osascript for emulator pixels.
- Before claiming Rust done: `./scripts/check.sh`.

## Deeper reference

- [`docs/AGENT_DEBUG_API.md`](../../../docs/AGENT_DEBUG_API.md) — Phase A localhost control plane (implemented), framebuffer export, unification roadmap
- [`docs/DEBUGGING.md`](../../../docs/DEBUGGING.md) — categories, flash-load event table, harness recipes, hot-path cost
- [`tests/fixtures/tape/README.md`](../../../tests/fixtures/tape/README.md) — fixtures + load matrix
- `crates/debug_cli/src/main.rs` — authoritative CLI flags
- `crates/machine/src/inspect.rs` — Inspect fields
- `crates/host_api/src/ffi.rs` — `sc_debug_*` / `sc_inspect_json`
