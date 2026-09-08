# Tape fixtures

Freely redistributable TAP/TZX images for automated tests.

Rebuild **TAP** fixtures only with:

```bash
python3 scripts/build_tape_fixtures.py
```

That script writes `minimal_code.tap`, `attr_mark.tap`, `print_ok.tap`, and
`custom_loader.tap`. It does not rebuild `minimal.tzx` (hand-maintained for TZX unit tests).

| File | Purpose |
| --- | --- |
| `minimal_code.tap` | CODE header + 6-byte routine at `0x8000`: `LD HL,4000 / LD (HL),42 / RET` |
| `attr_mark.tap` | CODE at `0x8000`: writes `0xD7` to attribute `0x5800` then `RET` (visible load mark). Load with `LOAD "" CODE`. |
| `print_ok.tap` | 1-line BASIC `10 PRINT "OK"` (header name `printok`). Load with `LOAD ""`. |
| `custom_loader.tap` | CODE @`8000` calls ROM `LD-BYTES` for a following `0xC8` flag block and loads byte `0xA5` at `0x9000` (Boggit-style custom flag). Load with `LOAD "" CODE`, then `RANDOMIZE USR 32768`. |
| `minimal.tzx` | Minimal standard-speed TZX wrapper used by TZX unit tests (not rebuilt by the script above) |

Do not add commercial game TAPs.

## TZX Loop Start/End regression (#372)

CI always covers synthetic Loop Start (`0x24`) / Loop End (`0x25`) via:

- `cargo test -p tape loop_start_end_expands_pure_tone` — pulse schedule expands N×
- `cargo test -p host_api open_tape_expands_tzx_loop_blocks` — SpecChumMac `open_tape` path

Prior fixtures (`minimal.tzx`, Boggit-style standard-only) never exercised loop blocks, so
commercial Speedlock TZXs like Arkanoid failed open with no CI signal.

## Content identity (#366)

Fixture digests are registered in the offline catalogue
(`formats::media_identity`) so hosts can show human titles (e.g. `PRINT "OK"`)
instead of filenames. See [`docs/TAPE_IDENTITY.md`](../../../docs/TAPE_IDENTITY.md).

## Load matrix (CI)

`cargo test -p machine --lib matrix` covers:

| Fixture | Models | Instant | EAR speeds |
| --- | --- | --- | --- |
| `attr_mark.tap` | 48K, 128K, +3 | yes | 1/2/5/10/20 (48K); 2/5/10/20 (128K/+3) |
| `custom_loader.tap` | 48K, 128K, +3 | yes | 1 (48K) + 2/5/10/20 (all) |

Set `SPEC_CHUM_FULL_TAPE_MATRIX=1` to also run EAR@1 on 128K/+3 (slow).

## Local repro (not in git)

Optional commercial tape for optional local tests (never commit):

```bash
export SPEC_CHUM_BOGGIT_TZX="$HOME/Downloads/BoggitThe/The Boggit - Side 1.tzx"
# Instant + EAR@2/5/10/20 on 48K/128K/+3; add FULL for EAR@1
SPEC_CHUM_FULL_TAPE_MATRIX=1 cargo test -p machine --lib boggit -- --nocapture
```

Speedlock / Loop Start (`0x24`) example (optional local only):

```bash
# ~/Downloads/Arkanoid.tzx — HostSession open + TzxPlayer parse
cargo test -p tape arkanoid_downloads -- --nocapture
cargo test -p tape arkanoid_pause_polarity -- --nocapture
cargo test -p host_api open_local_arkanoid -- --nocapture
# EAR Play past Speedlock sampler + post-tape DI turbo (#379 / #380)
cargo test -p host_api --release --lib arkanoid_ear_leaves_sampler -- --ignored --nocapture
# First DI delay stage RET (~5 min @64×; multi-stage → game is longer)
cargo test -p host_api --release --lib arkanoid_speedlock_first_delay_ret -- --ignored --nocapture
# Bounded post-tape wait (screen + delay stub; not full game entry)
cargo test -p host_api --release --lib arkanoid_ear_load_leaves_speedlock -- --ignored --nocapture
```

TZX pause polarity follows Fuse `force_low` / `LEVEL_LOW` (absolute low on the
first edge after a non-zero pause). A bad `LEVEL_LOW` that emitted an *extra*
low at tape start inverted the whole EAR schedule and stuck Arkanoid at `$FD2A`.
Play turbo also continues after the deck finishes while `IFF1=0` and `PC≥$8000`
so Speedlock border-delays do not appear hung at 1× after a turbo load.

After EAR exhaust, Arkanoid sits in a **multi-stage** Speedlock DI delay at
`$F448` (`LD E,IXH` at `$F44E`, not a CALL). The first stage RETs at `$F476`
after ~12.7k host frames at EAR 64× (~4.5 hours of Spectrum time / ~4–5 minutes
wall-clock). Further nested stages continue before `IFF1`/game entry — this is
ROM-accurate protection delay, not an EAR desync. Prefer EAR **64×** (UI option)
while waiting; chrome shows “Speedlock delay…” once the deck is finished.
### How to load The Boggit on 128K at 1×

1. Model **128K**, insert Side 1 TZX (converted to TAP automatically).
2. Turn **Instant flash-load** **off** for a pure EAR experience, **or** leave it **on** (ROM/48 BASIC blocks flash; custom `0xC8` blocks still need the RAM loader + EAR/Instant trap).
3. Prefer **Tape Loader** from the 128 menu (Enter), **or** 48 BASIC then `LOAD ""` (**not** `LOAD "" CODE` — Side 1 starts with a **PROGRAM**).
4. Press **Play** when the border goes red/cyan / the loader waits.
5. EAR speed **1×** is realtime; higher speeds run N Spectrum frames per host tick while Play is active (wall-clock ≈ realtime / N; pulse widths stay ROM-accurate).

Debug / observability: see [`docs/DEBUGGING.md`](../../../docs/DEBUGGING.md).

