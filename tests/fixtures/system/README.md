# Third-party system-test fixtures

Independent ZX Spectrum programs used by `--features system-tests`.
Spec Chum did not author these; they print measurements that can be
checked against published 48K/128K/+3 timing.

**TAP files are not in git.** `./scripts/fetch_system_tests.sh` caches them
under `.rom-cache/system-tests/` (gitignored). Re-run with
`FORCE_SYSTEM_TESTS=1` to refresh.

Do not add commercial game TAPs.

| Cached file | Program | License | Source |
| --- | --- | --- | --- |
| `minfo.tap` | Minfo (frame / INT / first contended / line T) | GPL (author notice on tape) | [Jan Bobrowski](http://torinak.com/~jb/zx/minfo.tap) (2025-12-13) |
| `ulatest3.tap` | ULA test 3 (floating bus + contention grid) | GPL | [Jan Bobrowski](http://torinak.com/~jb/zx/ulatest3.tap) |
| `timingtest.tap` | Timing Test v0.3 (frame duration + I/O/contention cases) | GPL | [Patrik Rak](http://zxds.raxoft.cz/taps/misc/timingtest.zip) (`timingtest-0.3/timing.tap`) |

## Published 48K PAL numbers these programs display

- Frame length: **69888** T
- First contended memory cycle: **14335** T (Minfo “First contended”)
- INT pulse: **32** T (Minfo “INT time”, ULA 48K)

128K / grey +2 / +2A / +3 PAL frame length: **70908** T.

## Running

```bash
./scripts/run_system_tests.sh
```
