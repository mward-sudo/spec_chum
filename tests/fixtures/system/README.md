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
| `floatspy.tap` | Floating Spy v0.33 (Ramsoft floating-bus self-test) | freeware (Ramsoft notice) | [zxe.io depot](https://zxe.io/depot/software/ZX%20Spectrum/Floating%20Spy%20v0.33%20%282002-04%29%28Ramsoft%29%5B%21%5D.zip) | `dc4a3ba0b0b74396919e0a67f0984aaa5762a3bdd3e0afb4bc38ac72fa7bef34` |
| `ula48_simple.tap` | ULA 48 Simple Test (azesmbog) | freeware | [zxe.io](https://zxe.io/depot/software/ZX%20Spectrum/ULA%2048%20Simple%20Test%20%282012-10-06%29%28azesmbog%29%5B%21%5D.tap) | `1bcd04d0dda815eb8ae49014b828752288551f596a82c7ffd46662d6f82f2c4e` |
| `ula128_timing.tap` | ULA 128 Timing Test (azesmbog) | freeware | [zxe.io](https://zxe.io/depot/software/ZX%20Spectrum/ULA%20128%20Timing%20Test%20%282012-10-06%29%28azesmbog%29%5B%21%5D.tap) | `59578bae6352d6a92b1887b392ee786aa9f162624a1ddef9541037b517b0c90f` |
| `ula128e_plus3.tap` | ULA 128E +3 Test (azesmbog) | freeware | [zxe.io](https://zxe.io/depot/software/ZX%20Spectrum/ULA%20128E%20%2B3%20Test%20%282012-10-10%29%28azesmbog%29%5B%21%5D.tap) | `60b3aaeca5b9d45c712d874fafa136c97d373c1569964cb87703ddf53911a8d5` |
| `snow.tap` | Weiv snow effect (48K ULA bug, I=$40–$7F) | GFDL (Weiv bundle) | [zxe.io Snow Tests zip](https://zxe.io/depot/software/ZX%20Spectrum/Snow%20Tests%20%282022-10-19%29%28Weiv%29%5B%21%5D.zip) (`SnowTests/snow.tap`) | `d930335f0455604c0e0082f20105f82cc0d85f1e2a4daa30f124132cd041e74a` |

ZIP intermediates (also verified): `timingtest-0.3.zip` SHA-256
`bacff01453a01c14754c167b5b02695ec29a0a5960c2f202b52b26494c2e8dff`;
`floating-spy-0.33.zip` SHA-256
`3663cfc76b0733491c69faf088dfacda9294aa76e828600070380086138747ed`;
`weiv-snow-tests.zip` SHA-256
`907ee0c8d40203de7e058c70a4100a9d414cf5a7e0936ffb31861131c6233be7`.
Bobrowski fixtures use HTTPS; the raxoft ZIP host is HTTP-only, so integrity
rests on the committed digests.

## Published 48K PAL numbers these programs display

- Frame length: **69888** T
- First contended memory cycle: FAQ / ULA table **14335** T (INT low = T0,
  early timing). Minfo and Timing Test print **14336** under the common
  INT-low-as-T1 numbering (same physical edge).
- INT pulse: **32** T (Minfo “INT time”, ULA 48K)
- Contended-NOP totals (4T + delay): **10 9 8 7 6 5 4 4** (Timing Test)
- Timing Test IN menus report `duration − pre(14) − RET(10)` ⇒ baseline **4**
  (+ I/O wait). First paper rows (early timing): FE/FFFE `9 8 7 6 5 4 4 10`,
  7FFE `10 9 8 7 6 5 4 10`, 7FFF `16 15 14 13 12 11 10 16`, FF/FFFF all `4`.
- Floating Spy self-test (`T`): **Floating bus OK**

128K / grey +2 / +2A / +3 PAL frame length: **70908** T.

## Running

```bash
./scripts/run_system_tests.sh
```

Before a `vX.Y.Z` release, the full slow suite is **required** (system-tests +
z80doc + z80full) — not optional:

```bash
./scripts/run_slow_tests.sh
```

See `docs/RELEASE.md` for cache vs fixture paths (`.rom-cache/` archives vs
`tests/fixtures/z80test/` extracted TAPs).