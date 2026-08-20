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
| `custom_loader.tap` | CODE @`8000` calls ROM `LD-BYTES` for a following flag `0xC8` byte at `0x9000` (Boggit-style custom flag). Load with `LOAD "" CODE`, then `RANDOMIZE USR 32768`. |
| `minimal.tzx` | Minimal standard-speed TZX wrapper used by TZX unit tests (not rebuilt by the script above) |

Do not add commercial game TAPs.

## Load matrix (CI)

`cargo test -p machine --lib matrix` covers:

| Fixture | Models | Instant | EAR speeds |
| --- | --- | --- | --- |
| `attr_mark.tap` | 48K, 128K, +3 | yes | 1/2/5/10/20 (48K); 2/5/10/20 (128K/+3; 1× skipped as slow) |
| `custom_loader.tap` | 48K, 128K, +3 | yes | 10, 20 |

## Local repro (not in git)

Optional commercial tape for optional local tests (never commit):

```bash
export SPEC_CHUM_BOGGIT_TZX="$HOME/Downloads/BoggitThe/The Boggit - Side 1.tzx"
cargo test -p machine --lib boggit -- --nocapture
```

### How to load The Boggit on 128K at 1×

1. Model **128K**, insert Side 1 TZX (converted to TAP automatically).
2. Turn **Instant flash-load** **off** for a pure EAR experience, **or** leave it **on** (ROM/48 BASIC blocks flash; custom `0xC8` blocks still need the RAM loader + EAR/Instant trap).
3. Prefer **Tape Loader** from the 128 menu (Enter), **or** 48 BASIC then `LOAD ""` (**not** `LOAD "" CODE` — Side 1 starts with a **PROGRAM**).
4. Press **Play** when the border goes red/cyan / the loader waits.
5. EAR speed **1×** is realtime; higher speeds shorten leader/pause only.

Debug / observability: see [`docs/DEBUGGING.md`](../../../docs/DEBUGGING.md).

