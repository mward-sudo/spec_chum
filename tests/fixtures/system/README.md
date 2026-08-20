# Third-party system-test fixtures

Independent ZX Spectrum programs used by `--features system-tests`.
Spec Chum did not author these; they print measurements that can be
checked against published 48K/128K/+3 timing.

**TAP files are not in git.** `./scripts/fetch_system_tests.sh` caches them
under `.rom-cache/system-tests/` (gitignored). Re-run with
`FORCE_SYSTEM_TESTS=1` to refresh. Downloads are written atomically and
verified against the SHA-256 digests below before the cache accepts them.

Do not add commercial game TAPs.

| Cached file | Program | Licence | Source | SHA-256 |
| --- | --- | --- | --- | --- |
| `minfo.tap` | Minfo (frame / INT / first contended / line T) | GPL (author notice on tape) | [Jan Bobrowski](https://torinak.com/~jb/zx/minfo.tap) (2025-12-13) | `c1ff004f9a5cb66d99afadff618c59e255e19ad33bd87908e63977c864d4979d` |
| `ulatest3.tap` | ULA test 3 (floating bus + contention grid) | GPL | [Jan Bobrowski](https://torinak.com/~jb/zx/ulatest3.tap) | `9445d3bd1661c2d5a62e2b3762ebd1ab00af9b319435ae7397bf6eb51462c6c9` |
| `timingtest.tap` | Timing Test v0.3 (frame duration + I/O/contention cases) | GPL | [Patrik Rak](http://zxds.raxoft.cz/taps/misc/timingtest.zip) (`timingtest-0.3/timing.tap`) | `da62ba6438af1398b3e1d1bbd3627985373b8cc81a72628421285329f4903962` |

ZIP intermediate (also verified): `timingtest-0.3.zip` SHA-256
`bacff01453a01c14754c167b5b02695ec29a0a5960c2f202b52b26494c2e8dff`.
Bobrowski fixtures use HTTPS; the raxoft ZIP host is HTTP-only, so integrity
rests on the committed digests.

## Published 48K PAL numbers these programs display

- Frame length: **69888** T
- First contended memory cycle: **14335** T (Minfo “First contended”)
- INT pulse: **32** T (Minfo “INT time”, ULA 48K)

128K / grey +2 / +2A / +3 PAL frame length: **70908** T.

## Running

```bash
./scripts/run_system_tests.sh
```
