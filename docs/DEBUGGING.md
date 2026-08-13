# Debugging & observability

Structured emulator tracing lives in the `trace` crate: a category-gated ring
buffer with dump APIs for tests, the egui app, and the native macOS shell.

When tracing is **off**, each emit site is a single `AtomicU64` load (`Relaxed`)
and an early return — no allocation and no lock on the hot path.

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

Hosts call `trace::init_from_env()` (egui `main`, `HostSession::new`,
`sc_debug_init_from_env`) so env vars apply once at startup.

### Categories (bitmask)

| Name | Bit | Contents |
| --- | --- | --- |
| `cpu` | 1 | Instruction / IRQ / HALT samples |
| `bus` | 2 | Port `FE` / `7FFD` / `1FFD`, EAR edges |
| `tape` | 4 | Play/pause/rewind, block consume, flash-load enter/exit/skip, EAR rate |
| `ula` | 8 | Frame count, border changes (sampled) |
| `machine` | 16 | Model, load mode, LD-BYTES hold |

C ABI: `sc_debug_set_categories(cats)` with the same bits; `0` disables.

## Dump API

Rust (`trace` crate):

- `snapshot()` / `len()` / `clear()`
- `dump_string()` / `dump_to_writer` / `dump_to_file` / `dump_to_stderr`
- `dump_to_env_file()` — writes when `SPEC_CHUM_TRACE_FILE` is set

C ABI (`host_api`):

- `sc_debug_dump()` — heap string; free with `sc_string_free`
- `sc_debug_dump_to_file(path)`
- `sc_debug_event_count()` / `sc_debug_clear()`

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

## Mac vs egui

### egui (`cargo run -p app`)

- Env vars above work at process start.
- Menu **Debug**: toggle categories, clear ring, dump to stderr / file, show PC
  and last ring events.

### macOS SwiftUI (`./scripts/run_macos_app.sh`)

- Same env vars (export before launching the wrapper).
- Menu **Debug**: Enable default trace, dump to file / Desktop, clear ring.
- C hooks: `sc_debug_*` in `spec_chum_host.h`.

## Failing-load harness

```bash
# Runs LOAD "" with tracing; dumps ring to stderr if CODE is missing (CI still
# asserts that tape/machine events were recorded):
cargo test -p machine attr_mark_load_path_dumps_trace_on_failure -- --nocapture

# Hard success gate (ignored until #85):
cargo test -p machine attr_mark_load_path_must_succeed -- --ignored --nocapture

# Optional file dump:
SPEC_CHUM_TRACE_FILE=/tmp/spec_chum_trace.txt \
  cargo test -p machine attr_mark_load_path_dumps_trace_on_failure -- --nocapture
```

Fixture: `tests/fixtures/tape/attr_mark.tap` (CODE at `0x8000` marking attr
`0x5800`).

Local commercial tape for optional repro (**do not commit**):

`/Users/michael/Downloads/BoggitThe/The Boggit - Side 1.tzx`

See also `tests/fixtures/tape/README.md` and open issue for tape load still
failing in practice (blocked on this infra until traces identify the cause).

## Hot-path cost

- Disabled: one atomic load per emit site; no `Mutex`, no alloc.
- Enabled: lock ring, push `Copy` event (no per-instruction heap).
- Prefer `SPEC_CHUM_TRACE=tape` (or `default`) while debugging loads; only add
  `cpu` for short windows (`SPEC_CHUM_TRACE_CPU_EVERY` helps).
