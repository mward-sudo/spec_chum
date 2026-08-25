# Debugging & observability

Structured emulator tracing lives in the `trace` crate: a category-gated ring
buffer with dump APIs for tests, the egui app, and the native macOS shell.

When tracing is **off**, each emit site is a single `AtomicU64` load (`Relaxed`)
and an early return — no allocation and no lock on the hot path.

The `z80` crate does **not** emit traces. CPU events (`CpuStep` / `CpuIrq` /
`CpuHalt` / `UlaInt`) are produced from `Machine::step_once` and `run_frame`.

## Enable

| Mechanism | Effect |
| --- | --- |
| `SPEC_CHUM_DEBUG=1` | Enable default categories: `bus,tape,ula,machine` (not full CPU stream) |
| `SPEC_CHUM_TRACE=tape,cpu` | Enable an explicit comma-separated list |
| `SPEC_CHUM_TRACE=all` | Enable every category including CPU |
| `SPEC_CHUM_TRACE=default` | Same as `SPEC_CHUM_DEBUG=1` |
| `SPEC_CHUM_TRACE_CAPACITY=N` | Ring size (default **8192**) |
| `SPEC_CHUM_TRACE_CPU_EVERY=N` | Sample every Nth CPU step when `cpu` is on (default 1) |
| `SPEC_CHUM_TRACE_FILE=/path/to/dump.txt` | Optional path used by `dump_to_env_file()` / harness failures |
| `SPEC_CHUM_TRACE_APPEND=1` | With `SPEC_CHUM_TRACE_FILE`, append each event as it is emitted (writer is flushed so short CLI runs are not empty) |

Hosts call `trace::init_from_env()` (egui `main`, `HostSession::new`,
`sc_debug_init_from_env`) so env vars apply once at startup. `spec-chum-debug`
also applies `--trace` **before** tape insert / `type-load` so those events are
recorded. `SPEC_CHUM_TRACE_APPEND=1` writes each event (and flushes) to
`SPEC_CHUM_TRACE_FILE`; do not rely on `dump-trace` in a second process.

### Categories (bitmask)

| Name | Bit | Contents |
| --- | --- | --- |
| `cpu` | 1 | Instruction / IRQ / HALT samples (`Machine::step_once` / `run_frame`) |
| `bus` | 2 | Port `FE` / `7FFD` / `1FFD`, EAR edges, contention, floating bus |
| `tape` | 4 | Play/pause/rewind, block consume, flash-load enter/exit/skip, sampled EAR edge counts |
| `ula` | 8 | Frame count, INT, border changes (sampled) |
| `machine` | 16 | Model, load mode, LD-BYTES hold, snapshot apply |
| `ay` | 32 | AY register select / write |
| `disk` | 64 | +3 FDC port traffic |
| `mem` | 128 | Debugger memory-watch hits |

C ABI: `sc_debug_set_categories(cats)` with the same bits; `0` disables.

## Dump API

Rust (`trace` crate):

- `snapshot()` / `len()` / `clear()`
- `dump_string()` / `dump_to_writer` / `dump_to_file` / `dump_to_stderr`
- `dump_json()` / `dump_ndjson()` — hand-rolled JSON (no `serde`)
- `dump_to_env_file()` — writes when `SPEC_CHUM_TRACE_FILE` is set

C ABI (`host_api`):

- `sc_debug_dump()` — heap string; free with `sc_string_free`
- `sc_debug_dump_to_file(path)`
- `sc_debug_event_count()` / `sc_debug_clear()`

## Headless CLI (`spec-chum-debug`)

Agent / script entry. Crate `debug_cli`, binary `spec-chum-debug`.

Each invocation is a **new process** (fresh machine and empty trace ring).
Put `--trace`, `--snapshot`, `--tap`, and the subcommand on the **same** command.

```bash
# Inspect after N frames (run already prints Inspect)
cargo run -p debug_cli -- --model 48k run --frames 1
cargo run -p debug_cli -- --model 48k --json dump-state

# Run until PC (hex), mem write watch, PC break over frames
cargo run -p debug_cli -- until-pc 056C --max 10000000
cargo run -p debug_cli -- watch-write 4000 --max 10000000
cargo run -p debug_cli -- break-pc 8000 --frames 200

# Peek / disassemble
cargo run -p debug_cli -- peek 4000 --len 64
cargo run -p debug_cli -- disasm --count 16
cargo run -p debug_cli -- disasm --addr 056C --count 8

# Tape + trace (enable categories on the command that generates events)
cargo run -p debug_cli -- --tap tests/fixtures/tape/attr_mark.tap type-load --code
# 128K / +3: 48 BASIC then LOAD "" CODE (not Tape Loader)
cargo run -p debug_cli -- --model 128k --tap tests/fixtures/tape/attr_mark.tap type-load --code
# EAR bitstream (ROM LD-BYTES; pause until typed). Speed shortens leader/pause only.
# Models: 48k / 128k / plus2a / plus3. Instant flash is default; `--ear-load` disables it.
cargo run -p debug_cli -- --model 48k --tap tests/fixtures/tape/attr_mark.tap \
  --ear-load --speed 10 type-load --code --max 2000
# Custom flag after CODE (Boggit-style): LOAD "" CODE then USR — see custom_loader.tap
cargo run -p debug_cli -- --model 128k --tap tests/fixtures/tape/custom_loader.tap \
  type-load --code

# Same process: --trace plus append (file is flushed; dump-trace cannot see another PID)
SPEC_CHUM_TRACE_FILE=/tmp/sc_append.txt SPEC_CHUM_TRACE_APPEND=1 \
  cargo run -p debug_cli -- --trace tape --tap tests/fixtures/tape/attr_mark.tap type-load --code
cargo run -p debug_cli -- --trace tape,cpu --json dump-trace

# After `cargo build -p debug_cli`:
./target/debug/spec-chum-debug dump-state
```

`--trace` takes the same category list as `SPEC_CHUM_TRACE`. `--json` prints
`Inspect::to_json()` or `trace::dump_json()`. `--snapshot` loads SNA/Z80;
`--rom` / `--model 128k|plus2a|plus3` select machine.

`Debugger` on `Machine`: `paused`, PC breaks, mem/port watches,
`run_until_break`. Continue from a PC hit uses `continue_from_pc` so the
breakpoint is not re-taken immediately.

## Mac vs egui

### egui (`cargo run -p app`)

- Env vars above work at process start.
- Menu **Debug**: open **Debugger window**, toggle categories (including AY),
  clear ring, dump to stderr / file.
- **Debugger window**: Pause / Step / Run, `Inspect` text, disassembly at PC,
  hexdump, PC breakpoints, last ring events.

### macOS SwiftUI (`./scripts/run_macos_app.sh`)

- Same env vars (export before launching the wrapper).
- Menu **Debug**: Enable default trace, dump to file / Desktop, clear ring.
- No full Pause/Step inspector yet — use `spec-chum-debug` or the egui window.
- C hooks: `sc_debug_*` in `spec_chum_host.h`.

## Recipes

### Fuse mismatch

```bash
cargo test -p z80 fuse_all_vectors --offline -- --nocapture
```

A failed vector prints expected vs actual registers/memory/T-states plus a
disassembly window of the **start** PC from the test’s `FlatMem` (no global
`trace` crate). Example:

```text
00: PC: got 0001 want 0002
disasm @0000:
0000  00          NOP
…
```

### Tape load

`SPEC_CHUM_TRACE=tape` (or `default`). Inspect `tape playing=` / `flash=` /
`block=` on `Machine::inspect()`, or `Inspect::to_json()`. See flash-load
events below. CLI: `--tap … type-load [--code]`.

### Contention / beam

`Machine::inspect()` prints `frame_t`, `line`, `x`, `INT`, and `contend_pc`
(wait states if the next opcode fetch is contended). Enable `bus` for
`bus.contend` / `bus.floating` ring events.

### Paging (128K / +3)

Inspect `7FFD` / `1FFD` / `ROM` / `C000`. Trace `bus,machine` for port writes
and `machine.snapshot` after SNA/Z80 apply.

### AY

128K / +3. `SPEC_CHUM_TRACE=ay` logs `ay.select` / `ay.write`. Registers are
on `Inspect` (`ay_regs` / `to_json()`).

## Interpreting flash-load events

Typical successful instant load:

```text
tape.play block=0/2
tape.flash.enter block=0 flag=00 load=1 dest=5C00 len=17 PC=056C … AF'=00C1 …
tape.block idx=0 flag=00 len=17
tape.flash.exit ok=1 bytes=17 block_after=1 …
tape.flash.enter block=1 flag=FF load=1 dest=8000 len=…
tape.flash.exit ok=1 bytes=… block_after=2 …
```

Failure / skip clues:

| Event | Meaning |
| --- | --- |
| `tape.flash.skip reason=paused` | Trap hit while deck paused (Play not pressed) |
| `tape.flash.skip reason=wrong_flag` | Block flag ≠ expected (A′); block skipped, search continues |
| `tape.flash.skip reason=length_mismatch` | TAP length ≠ DE+2 |
| `tape.flash.skip reason=checksum_fail` | XOR checksum mismatch |
| `tape.flash.skip reason=no_block` | No more TAP blocks |
| `tape.flash.exit ok=0` | Trap returned Failure (carry clear / RET) |
| `machine.ld_bytes_hold holding=1` | PC held at `0x056C` waiting for Play |

**Register note:** at ROM `LD-BYTES` trap `0x056C`, flag and load/verify carry are
in **A′ / F′** (after `EX AF,AF'`). Trace `FlashLoadEnter` prints both AF and AF′.

## Failing-load harness

```bash
# attr_mark is a CODE block — harness types LOAD "" CODE; asserts flash enter/exit + bytes:
cargo test -p machine attr_mark_load_path_dumps_trace_on_failure -- --nocapture

# Same path, hard gate (also runs in default `cargo test`):
cargo test -p machine attr_mark_load_path_must_succeed -- --nocapture

# PROGRAM fixture (plain LOAD ""):
cargo test -p machine print_ok_load_quotes_succeeds -- --nocapture

# Optional file dump:
SPEC_CHUM_TRACE_FILE=/tmp/spec_chum_trace.txt \
  cargo test -p machine attr_mark_load_path_dumps_trace_on_failure -- --nocapture
```

Fixture: `tests/fixtures/tape/attr_mark.tap` (CODE at `0x8000` marking attr
`0x5800`). Use **Tape → Type LOAD "" CODE** (or type `LOAD "" CODE` by hand).
Plain `LOAD ""` only accepts PROGRAM headers (see `print_ok.tap`).

z80test fail/timeout (`--features slow-tests`) panics with RST10 capture plus
`Machine::inspect()` Display.

Local commercial tape for optional repro (**do not commit**):

`<path-to-local-commercial-tape>/The Boggit - Side 1.tzx`

See also `tests/fixtures/tape/README.md`.

## Hot-path cost

- Disabled: one atomic load per emit site; no `Mutex`, no alloc.
- Enabled: lock ring, push `Copy` event (no per-instruction heap).
- Prefer `SPEC_CHUM_TRACE=tape` (or `default`) while debugging loads; only add
  `cpu` for short windows (`SPEC_CHUM_TRACE_CPU_EVERY` helps).
  `SPEC_CHUM_TRACE_APPEND=1` is for long agent runs, not the default hot path.
