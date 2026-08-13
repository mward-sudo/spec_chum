# Tape fixtures

Freely redistributable TAP images for automated tests. Rebuild with:

```bash
python3 scripts/build_tape_fixtures.py
```

| File | Purpose |
| --- | --- |
| `minimal_code.tap` | CODE header + 6-byte routine at `0x8000`: `LD HL,4000 / LD (HL),42 / RET` |
| `attr_mark.tap` | CODE at `0x8000`: writes `0xD7` to attribute `0x5800` then `RET` (visible load mark) |
| `print_ok.tap` | 1-line BASIC `10 PRINT "OK"` (header name `printok`) |
| `minimal.tzx` | Minimal standard-speed TZX wrapper used by TZX unit tests |

Do not add commercial game TAPs.
