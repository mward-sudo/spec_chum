# Spec Chum

[![CI](https://github.com/mward-sudo/spec_chum/actions/workflows/ci.yml/badge.svg)](https://github.com/mward-sudo/spec_chum/actions/workflows/ci.yml)

A from-scratch, hardware-accurate ZX Spectrum emulator written in Rust with an egui frontend (plus an optional native macOS SwiftUI shell).

UI stack rationale (egui vs iced / Slint / Tauri / native shells / optional libretro): [docs/UI_ARCHITECTURE.md](docs/UI_ARCHITECTURE.md).

Native macOS (liquid glass): [docs/MACOS_NATIVE.md](docs/MACOS_NATIVE.md) — `./scripts/run_macos_app.sh`.

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

### Native macOS shell (optional)

Requires full Xcode. Builds Rust `host_api` + SwiftUI:

```bash
./scripts/fetch_roms.sh
./scripts/run_macos_app.sh
```

See [docs/MACOS_NATIVE.md](docs/MACOS_NATIVE.md).

## Releases

Push a `vX.Y.Z` tag to build release archives: macOS `.zip` with `Spec Chum.app`,
Windows `.zip` with `.exe`s, Linux `.tar.gz` with binaries (optional
Apple/Windows/GPG signing). See [docs/RELEASE.md](docs/RELEASE.md).

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
./scripts/run_system_tests.sh   # optional: third-party ULA/ROM TAP suite (slow)
```

## Known limitations / follow-ups

- **z80test** — `z80doc` is integrated ([#17](https://github.com/mward-sudo/spec_chum/issues/17) closed); keep slow-tests green. `z80full` is optional/`#[ignore]` via `./scripts/fetch_z80test.sh`.
- **System tests** — optional third-party ULA/ROM TAP suite ([#108](https://github.com/mward-sudo/spec_chum/issues/108)): `./scripts/run_system_tests.sh`. Not part of default CI.
- **AY** — mono PSG + beeper mix shipped ([#33](https://github.com/mward-sudo/spec_chum/issues/33)); stereo ACB/ABC pan is a possible follow-up.
- **Disk** — +3 DSK sector read path is minimal ([#25](https://github.com/mward-sudo/spec_chum/issues/25)); full uPD765 command set / write support can deepen later.

## License

MIT — see [LICENSE](LICENSE). Sinclair ROM binaries remain subject to their own terms and are not redistributed by this project.
