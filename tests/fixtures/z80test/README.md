# z80test fixtures

Patrik Rak’s [z80test](https://github.com/raxoft/z80test) (MIT).

| File | Source | Suite |
| --- | --- | --- |
| `z80doc.tap` | [v1.2a release](https://github.com/raxoft/z80test/releases/tag/v1.2a) (checked in) | Documented flags + registers |
| `z80full.tap` | same (checked in) | All flags + registers |
| `z80flags.tap` | same (checked in) | All flags, ignore GPRs |
| `z80docflags.tap` | same (checked in) | Documented flags only |
| `z80ccf.tap` | same (checked in) | SCF/CCF after every insn (Q) |
| `z80memptr.tap` | same (checked in) | MEMPTR via `BIT n,(HL)` |
| `z80ccfscr.tap` | via `./scripts/fetch_z80test.sh` | Visual SCF/CCF pattern (optional; not harnessed) |

Refresh / recover missing TAPs:

```bash
./scripts/fetch_z80test.sh
```

## Running

```bash
./scripts/fetch_roms.sh
cargo test -p machine --features slow-tests --release z80doc_all_tests_passed -- --nocapture
cargo test -p machine --features slow-tests --release z80full_all_tests_passed -- --nocapture
cargo test -p machine --features slow-tests --release z80flags_all_tests_passed -- --nocapture
cargo test -p machine --features slow-tests --release z80docflags_all_tests_passed -- --nocapture
cargo test -p machine --features slow-tests --release z80ccf_all_tests_passed -- --nocapture
cargo test -p machine --features slow-tests --release z80memptr_all_tests_passed -- --nocapture
```

PR CI runs `z80doc` only. Before a `vX.Y.Z` release, run the full slow suite (includes `z80doc` + `z80full`):

```bash
./scripts/run_slow_tests.sh
```
