# Spec Chum

[![CI](https://github.com/mward-sudo/spec_chum/actions/workflows/ci.yml/badge.svg)](https://github.com/mward-sudo/spec_chum/actions/workflows/ci.yml)

A from-scratch, hardware-accurate ZX Spectrum emulator written in Rust with an egui frontend.

## Goals

- Cycle-accurate Z80 (own implementation)
- Accurate ULA timing (contention, floating bus, border)
- 48K first, then 128K / grey +2
- TDD against Fuse vectors and z80test
- System ROMs fetched separately (not redistributed)

## Build

```bash
cargo build --release
./scripts/fetch_roms.sh
cargo run -p app --release
```

## ROMs

System ROMs are **not** included in this repository. Fetch them with:

```bash
./scripts/fetch_roms.sh
```

Source: [spectrumforeveryone/zx-roms](https://github.com/spectrumforeveryone/zx-roms) (pinned commit in the script).

## Development

See [CONTRIBUTING.md](CONTRIBUTING.md) for Rust practices, TDD expectations, and the `gh stack` workflow. Agent-oriented notes live in [AGENTS.md](AGENTS.md).

```bash
./scripts/check.sh   # fmt + clippy -D warnings + tests
```

## Known limitations

Honest gaps after the M0–M4 scaffolding (tracking issues):

- **z80test** — `z80doc` runs under `--features slow-tests` (see `tests/fixtures/z80test`). `z80full` and related TAPs: `./scripts/fetch_z80test.sh` then the same feature ([#17](https://github.com/mward-sudo/spec_chum/issues/17)).
- **AY audio** — 128K has AY register I/O only; no PSG synthesis or mix with the beeper ([#33](https://github.com/mward-sudo/spec_chum/issues/33)).
- **128K/+2 timing** — still uses 48K-oriented contention windows; floating bus on 128 returns `0xff`; border/beam updates are coarse ([#34](https://github.com/mward-sudo/spec_chum/issues/34)).

Later scope (unchanged): +2A/+3 ([#24](https://github.com/mward-sudo/spec_chum/issues/24)), TZX/RZX/Kempston/disk ([#25](https://github.com/mward-sudo/spec_chum/issues/25)).

## License

MIT — see [LICENSE](LICENSE). Sinclair ROM binaries remain subject to their own terms and are not redistributed by this project.
