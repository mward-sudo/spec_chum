# Spec Chum

[![CI](https://github.com/mward-sudo/spec_chum/actions/workflows/ci.yml/badge.svg)](https://github.com/mward-sudo/spec_chum/actions/workflows/ci.yml)

A from-scratch, hardware-accurate ZX Spectrum emulator written in Rust with an egui frontend (plus an optional native macOS SwiftUI shell).

UI stack rationale (egui vs iced / Slint / Tauri / native shells / optional libretro): [docs/UI_ARCHITECTURE.md](docs/UI_ARCHITECTURE.md).

Native macOS (liquid glass): [docs/MACOS_NATIVE.md](docs/MACOS_NATIVE.md) — `./scripts/run_macos_app.sh`.

Experimental Bevy 3D living-room CRT host: [docs/LIVING_ROOM.md](docs/LIVING_ROOM.md) — `cargo run -p living_room --release` ([#146](https://github.com/mward-sudo/spec_chum/issues/146)).

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
./scripts/run_system_tests.sh   # optional day-to-day: third-party ULA/ROM TAP suite (slow)
./scripts/run_slow_tests.sh     # required before vX.Y.Z: z80doc + system-tests + z80full
```

## Known limitations / follow-ups

- **z80test** — `z80doc` and `z80full` run under `--features slow-tests` ([#17](https://github.com/mward-sudo/spec_chum/issues/17) closed; [#122](https://github.com/mward-sudo/spec_chum/issues/122)). CI selects `z80doc` by name; run `z80full_all_tests_passed` for the full suite. **Releases require** `./scripts/run_slow_tests.sh`.
- **System tests** — third-party ULA/ROM TAP suite ([#108](https://github.com/mward-sudo/spec_chum/issues/108)): `./scripts/run_system_tests.sh`. Not part of default CI; **required before release**.
- **M5 peripherals (in progress)** — joystick modes (Kempston / Sinclair / Cursor), Kempston mouse, Multiface 1 paging ([MULTIFACE.md](docs/MULTIFACE.md)), DivMMC / Interface 1 / Beta Disk stubs on Bus48/Bus128, +3 FDC READ DATA command stream on ports `3FFD`/`2FFD`, AY stereo ACB/ABC/Mono UI. Still incomplete vs real hardware: ESXDOS boot, IF1 Microdrive protocol, TR-DOS ROM paging edge cases ([#138](https://github.com/mward-sudo/spec_chum/issues/138)–[#140](https://github.com/mward-sudo/spec_chum/issues/140)).
- **+3DOS** — DSK sector read + READ DATA command bytes work; full +3DOS boot needs a deeper µPD765 command set ([#141](https://github.com/mward-sudo/spec_chum/issues/141)).
- **macOS native shell** — audio, keymap, GCController, Open Snapshot/RZX/DSK, Type LOAD, and optional `macos-shell` CI job shipped on this branch ([#67](https://github.com/mward-sudo/spec_chum/issues/67), [#68](https://github.com/mward-sudo/spec_chum/issues/68)); App bundle / signing polish can follow.
- **Tape** — flash-load and turbo paths work; egui interim “Experience (~20s EAR)” uses 16× EAR only — true abbreviated tones still open ([#82](https://github.com/mward-sudo/spec_chum/issues/82)).


## License

MIT — see [LICENSE](LICENSE). Sinclair ROM binaries remain subject to their own terms and are not redistributed by this project.
