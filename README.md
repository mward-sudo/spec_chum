# Spec Chum

[![CI](https://github.com/mward-sudo/spec_chum/actions/workflows/ci.yml/badge.svg)](https://github.com/mward-sudo/spec_chum/actions/workflows/ci.yml)

A from-scratch, hardware-accurate ZX Spectrum emulator written in Rust.

**Docs index (players vs developers):** [docs/README.md](docs/README.md).

**Public site (GitHub Pages):** [docs/www/](docs/www/) — simple static landing; updates when a `v*` release is cut (see [docs/www/README.md](docs/www/README.md)).

## Quick start

### Play a release build

Download a package from [GitHub Releases](https://github.com/mward-sudo/spec_chum/releases):

- **macOS** — `.dmg.zip` → unzip → open the `.dmg` → drag **Spec Chum.app** to Applications
- **Windows** — `*-setup.exe` (installer) or portable `.zip`
- **Linux** — `.deb`, AppImage, or `.tar.gz`

Redistributable Spectrum ROMs are bundled in release packages. See [docs/ROMS.md](docs/ROMS.md).

### Build from source

```bash
cargo build --release
./scripts/fetch_roms.sh
cargo run -p app --release
```

Native macOS shell (full Xcode):

```bash
./scripts/fetch_roms.sh
./scripts/run_macos_app.sh
```

See [docs/MACOS_NATIVE.md](docs/MACOS_NATIVE.md).

## What it aims for

- Cycle-accurate Z80 (own implementation)
- Accurate ULA timing (contention, floating bus, border)
- 48K, 128K, grey +2, +2A, +3 / +3e, Timex TC2048 / TS2068
- TDD against Fuse vectors and z80test
- System ROMs fetched separately (not committed to git)

## Hosts & platforms

| Host | Role |
| --- | --- |
| **egui** (`crates/app`) | Cross-platform UI; CI / headless fallback |
| **SpecChumMac** (`apps/macos`) | Native macOS SwiftUI product shell (release `.dmg`) |
| **windows_shell** | Native Win32 shell — Windows release primary — [docs/WINDOWS_NATIVE.md](docs/WINDOWS_NATIVE.md) (#351) |
| **linux_shell** | Native GTK4 shell — Linux release primary — [docs/LINUX_NATIVE.md](docs/LINUX_NATIVE.md) (#351) |
| **living_room** | Experimental Bevy 3D CRT — [docs/LIVING_ROOM.md](docs/LIVING_ROOM.md) (#146) |

UI stack rationale and native-shell strategy: [docs/UI_ARCHITECTURE.md](docs/UI_ARCHITECTURE.md).

## Releases & ROMs

Push a `vX.Y.Z` tag to build release archives (see [docs/RELEASE.md](docs/RELEASE.md)).
ROM **bytes are not** in git. Source checkouts fetch the managed redistributable
set with `./scripts/fetch_roms.sh`; release packages embed that set. Multiface,
TR-DOS, IF1, and similar firmware stay user-provided only — details in
[docs/ROMS.md](docs/ROMS.md).

**Amstrad / Sinclair:** Amstrad have kindly given their permission for the
redistribution of their copyrighted material but retain that copyright. Do not
alter the copyright messages inside ROM images.

## Current capabilities (summary)

Present-tense status — not a development diary. Detail and open gaps live in the linked docs / issues.

- **CPU / ULA** — Fuse vectors; z80test `z80doc` / `z80full` under `--features slow-tests` ([#17](https://github.com/mward-sudo/spec_chum/issues/17), [#122](https://github.com/mward-sudo/spec_chum/issues/122)). System TAP suite: [docs/TESTING.md](docs/TESTING.md) / [#108](https://github.com/mward-sudo/spec_chum/issues/108). Releases require `./scripts/run_slow_tests.sh`.
- **Tape** — flash-load, turbo EAR speeds, Experience abbreviated-pause load; TAP/TZX paths in egui and SpecChumMac ([#82](https://github.com/mward-sudo/spec_chum/issues/82)).
- **Peripherals** — Kempston / Sinclair / Cursor joysticks, Kempston mouse, Multiface 1 ([docs/MULTIFACE.md](docs/MULTIFACE.md)), DivMMC SPI + automap, Interface 1 Microdrive hooks, Beta Disk / TR-DOS (optional `roms/trdos.rom`), +3 µPD765 SEEK/READ/WRITE and Loader smokes. Still incomplete vs real hardware for full ESXDOS boot, full IF1 BASIC accuracy, TR-DOS `RUN` with a real TR-DOS ROM, and some VG93 format paths ([#138](https://github.com/mward-sudo/spec_chum/issues/138)–[#140](https://github.com/mward-sudo/spec_chum/issues/140)).
- **+3DOS** — command/result path and synthetic Loader / `LOAD "DISK"` smokes ([tests/fixtures/plus3/README.md](tests/fixtures/plus3/README.md)); copy-protected / weird DSK geometry and CP/M SYSTEM boot remain out of scope.
- **Timex** — TC2048 / TS2068 (MMU, AY, Warajevo `.dck`, SCLD hi-colour / hi-res) — [docs/TIMEX.md](docs/TIMEX.md) ([#192](https://github.com/mward-sudo/spec_chum/issues/192)).
- **Debug / automation** — localhost Agent Debug HTTP API (`spec_chum --serve` / `SPEC_CHUM_AGENT=1`) for scripted control and 1:1 framebuffer PNG — [docs/AGENT_DEBUG_API.md](docs/AGENT_DEBUG_API.md).

## Development

Contributor workflow: [CONTRIBUTING.md](CONTRIBUTING.md).
Full docs map: [docs/README.md](docs/README.md).

```bash
./scripts/check.sh              # fmt + clippy -D warnings + tests
./scripts/run_system_tests.sh   # optional: third-party ULA/ROM TAP suite (slow)
./scripts/run_slow_tests.sh     # required before vX.Y.Z
```

Engineering quality backlog: [#171](https://github.com/mward-sudo/spec_chum/issues/171).

LLM-assisted coding notes (crate map, hard constraints): [AGENTS.md](AGENTS.md) — not required for human contributors.

## License

MIT — see [LICENSE](LICENSE) (Spec Chum source only).

Amstrad have kindly given their permission for the redistribution of their
copyrighted Spectrum ROM material but retain that copyright. This project does
not commit ROM binaries; fetch them yourself (see [docs/ROMS.md](docs/ROMS.md)).
