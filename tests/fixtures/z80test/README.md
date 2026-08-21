# z80test fixtures

Patrik Rak’s [z80test](https://github.com/raxoft/z80test) (MIT).

| File | Source |
| --- | --- |
| `z80doc.tap` | [v1.2a release](https://github.com/raxoft/z80test/releases/tag/v1.2a) (checked in) |

Other TAPs (`z80full.tap`, …): `./scripts/fetch_z80test.sh`.

## Running

```bash
./scripts/fetch_roms.sh
cargo test -p machine --features slow-tests --release z80doc_all_tests_passed -- --nocapture
```

Before a `vX.Y.Z` release, run the full slow suite (includes `z80doc` + `z80full`):

```bash
./scripts/run_slow_tests.sh
```
