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

## License

MIT — see [LICENSE](LICENSE). Sinclair ROM binaries remain subject to their own terms and are not redistributed by this project.
