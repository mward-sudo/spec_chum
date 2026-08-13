# Tape fixtures

Freely redistributable TAP/TZX images for automated tests.

Rebuild **TAP** fixtures only with:

```bash
python3 scripts/build_tape_fixtures.py
```

That script writes `minimal_code.tap`, `attr_mark.tap`, and `print_ok.tap`. It does not rebuild `minimal.tzx` (hand-maintained for TZX unit tests).

| File | Purpose |
| --- | --- |
| `minimal_code.tap` | CODE header + 6-byte routine at `0x8000`: `LD HL,4000 / LD (HL),42 / RET` |
| `attr_mark.tap` | CODE at `0x8000`: writes `0xD7` to attribute `0x5800` then `RET` (visible load mark). Load with `LOAD "" CODE`. |
| `print_ok.tap` | 1-line BASIC `10 PRINT "OK"` (header name `printok`). Load with `LOAD ""`. |
| `minimal.tzx` | Minimal standard-speed TZX wrapper used by TZX unit tests (not rebuilt by the script above) |

Do not add commercial game TAPs.

## Local repro (not in git)

Optional commercial tape for optional local tests (never commit):

`<path-to-local-commercial-tape>/The Boggit - Side 1.tzx`

Debug / observability for failed loads: see [`docs/DEBUGGING.md`](../../../docs/DEBUGGING.md).
Harness: `cargo test -p machine attr_mark_load_path_must_succeed -- --nocapture` (types `LOAD "" CODE`).
`print_ok.tap`: `cargo test -p machine print_ok_load_quotes_succeeds`.

