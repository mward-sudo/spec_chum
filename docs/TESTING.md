# Testing and quality gates

Provable-correctness tiers and where each check runs. Tracked under
[#171](https://github.com/mward-sudo/spec_chum/issues/171). Release tagging
requirements remain in [RELEASE.md](RELEASE.md).

## Test tier matrix

| Tier | When | Commands / CI | What it proves |
| --- | --- | --- | --- |
| **Fast (PR / agent default)** | Every PR; before claiming done | `./scripts/check_crates.sh` while iterating; `./scripts/check.sh` before merge. CI: **`fmt + clippy + test`** (+ **`z80doc`** by name) | Workspace builds with `-D warnings` (excl. `living_room`); Fuse / unit / table tests; ROM-dependent tests skip cleanly when `roms/` is missing; GUI smoke without xvfb |
| **Living room / SpecChumMac** | When touching Bevy host, `host_api` FFI used by macOS, or living-room display | `./scripts/check_living_room.sh` (**release** by default). CI: **`living_room`**, **`macos-shell`** | Bevy room + staticlib / Swift shell compile; set `SPEC_CHUM_CHECK_LIVING_ROOM=1` to fold living-room into `./scripts/check.sh` |
| **Slow (pre-release)** | Before tagging `vX.Y.Z` | `./scripts/run_slow_tests.sh` | z80doc + z80ccf + z80memptr + system-tests + **z80full** — real CPU/ULA accuracy; no stubs ([#17](https://github.com/mward-sudo/spec_chum/issues/17), [#122](https://github.com/mward-sudo/spec_chum/issues/122)) |
| **Opt-in day-to-day** | Accuracy investigations | `./scripts/run_system_tests.sh`; individual `cargo test -p machine --features slow-tests --release …` | Third-party ULA/ROM TAPs ([#108](https://github.com/mward-sudo/spec_chum/issues/108)); full CPU suites outside release |

Default PR CI is **not** enough for a release. See [RELEASE.md](RELEASE.md).

### What “provable” means

- **Unit / Fuse:** opcode and contention tables match reference vectors before merging related groups.
- **z80doc / z80full / z80ccf / z80memptr:** Patrik Rak suites under `--features slow-tests --release` (fixtures in `tests/fixtures/z80test/`).
- **system-tests:** third-party ULA/ROM TAPs in `.rom-cache/system-tests/` (not git); failures are accuracy bugs — do not stub or weaken.
- **Integration / peripheral smokes:** assert observable hardware behaviour (FDC, paging, loader reachability), ROM/fixture-gated; never `assert!(… \|\| true)` placeholders.

### Hardware-faithful vs convenience

Flash-load, turbo tape, and similar UI helpers may diverge from real EAR timing but must still **load correctly**. Do not weaken hardware-path assertions to accommodate them; keep convenience-path tests clearly labelled ([AGENTS.md](../AGENTS.md)).

### ROM and fixture skip policy

- System ROMs: `roms/` via `./scripts/fetch_roms.sh` (not committed).
- ROM-dependent tests must **skip cleanly** when ROMs are missing (no hard fail in default CI).
- System-test TAPs: `.rom-cache/system-tests/` (fetched by `./scripts/run_system_tests.sh`).
- z80test fixtures: `tests/fixtures/z80test/` (`z80doc.tap` / `z80full.tap` in git; optional `./scripts/fetch_z80test.sh`).

## Lint and check inventory

| Check | Where it runs | Notes |
| --- | --- | --- |
| `cargo fmt --all -- --check` | `./scripts/check.sh`, CI `fmt + clippy + test` | `rustfmt.toml` |
| `cargo clippy --workspace --all-targets --exclude living_room -- -D warnings` | `./scripts/check.sh`, CI | Workspace lints in root `Cargo.toml`; `clippy.toml` |
| `cargo test --workspace --exclude living_room` | `./scripts/check.sh`, CI | Debug by default in the script |
| `./scripts/check_crates.sh` | Local / agents only | Debug clippy+test for crates touched vs `origin/main` |
| `./scripts/check_living_room.sh` | Opt-in / when living-room touched; CI `living_room` | **Release** Bevy by default |
| `./scripts/build_macos_app.sh` | CI `macos-shell` | SpecChumMac + living_room staticlib |
| z80doc (named filter) | CI slow-tests job | Bounded; full `z80full` is release-only |
| `./scripts/run_system_tests.sh` | Opt-in; inside `run_slow_tests.sh` | Needs network once for TAP cache |
| `./scripts/run_slow_tests.sh` | **Required before `vX.Y.Z`** | See [RELEASE.md](RELEASE.md) |
| `./scripts/check_pr_reviews.sh` | Local agents; CI **Bot review threads** | CodeRabbit HEAD + unresolved bot threads |
| `./scripts/check_deny.sh` / `cargo deny check` | Opt-in local; CI **cargo-deny** | `deny.toml` — licenses / advisories / bans / sources. Egui 0.31 transitive RustSec IDs ignored-with-reason (#171); revisit on `eframe`/`egui` bump. Not part of `./scripts/check.sh` |

### Workspace lint posture

- `unsafe_code = deny` workspace-wide; narrow `#[allow(unsafe_code)]` only with a `SAFETY` rationale.
- `unwrap_used = warn`; library crates use `thiserror` (no bare unwrap on non-test hot paths). Typed errors cover ROM load/build (`bus::RomLoadError`, `MachineBuildError`, `RomReadError`), peripheral attach (`MultifaceError` / `DivMmcError` / `BetaDiskError` / `Interface1Error` / `InsertDiskError`), Fuse/z80test harnesses, living_room present, egui `BuildMachineError`, and agent `ReadyError` ([#171](https://github.com/mward-sudo/spec_chum/issues/171) Pillar B). Remaining `Result<…, _>` → `String` **Ok** payloads are intentional text APIs (inspect / peek / disasm / trace dump), not error typing gaps.
- Clippy `pedantic` stays **allow** workspace-wide ([#171](https://github.com/mward-sudo/spec_chum/issues/171) Pillar B evaluation): full-group warn is dominated by `cast_*` / `many_single_char_names` in cores (~160 hits in `z80` alone). **`z80` / `ula` / `bus` / `tape` / `formats` / `machine` / `trace` / `control_plane` / `host_api` / `agent_server` / `debug_cli` / `app` / `living_room` enable `pedantic` crate-locally** with those endemic lints allowed; `match_same_arms` allowed in `z80` / `tape` / `formats` / `machine` / `app`; `machine` also allows inspect/joystick bool packs, progress `cast_precision_loss`, audio `float_cmp`, test `items_after_statements` / `ignore_without_reason`; `control_plane` also allows present `cast_precision_loss` and session/hardware `struct_excessive_bools`; `host_api` also allows prefs/status bool packs, audio `float_cmp`, progress `cast_precision_loss`, and test `items_after_statements`. `agent_server` / `debug_cli` use the shared endemic cast/name allows (easy pedantic fixes only). `app` also allows UI prefs/audio floats, egui bool packs, `items_after_statements`, and `match_same_arms` (easy pedantic fixes only). `living_room` also allows Bevy system `needless_pass_by_value`, Metal/IOSurface `doc_markdown`, camera/present floats, and hybrid bool packs (easy pedantic fixes only). Crate-root `#![allow(clippy::pedantic)]` removed on those crates so Cargo.toml levels apply. Nursery stays off (`missing_const_for_fn` noise). Prefer refactors over new `#[allow]`.
- **`#[allow]` policy:** every new allow needs a one-line rationale (and an issue link when temporary). Prefer module-level shared reasons over duplicated per-item noise. Inventory snapshot (2026-09-07): **22** sites — Bevy `too_many_arguments` / `type_complexity`, Z80 opcode maps, host FFI / session-handle `large_enum_variant`, intentional IF1 latch, living_room link-force / glow marker, and `unsafe_code` C ABI exports (all rationale-annotated).

### Known non-blocking noise

Documented exceptions (do not treat as green-gate failures unless they regress):

- Occasional Actions runner / composite-wrapper log notices (default CI Actions SHA-pinned: `actions/checkout@v6` / `actions/cache@v6`; see #171 CI hygiene).
- Historical (resolved / no longer seen on recent `macos-latest` CI): living-room `__eh_frame section too large` linker notes on `room_probe` / `room_perf`; if they return they are Apple ld notes and do not fail `-Dwarnings`.
- SpecChumMac builds set `MACOSX_DEPLOYMENT_TARGET=14.0` (+ matching `CFLAGS`/`CXXFLAGS`) in `./scripts/build_macos_app.sh` so clang deps (e.g. blake3 NEON) match Swift `.macOS(.v14)` — avoids `ld: built for newer macOS than being linked (14.0)`. If a stale archive still warns locally: `cargo clean -p blake3` then rebuild.
- **`cargo deny check` advisories:** licenses/bans/sources pass; remaining RustSec IDs (`paste`, `ttf-parser`, `quick-xml` 0.30) are ignored-with-reason in `deny.toml` as egui 0.31 / accesskit transitive deps (no untrusted XML parse in Spec Chum). Remove ignores when upgrading `eframe`/`egui` clears the crates (#171).

## Peripheral / M5 smoke inventory

Deepen PRs must add or extend ROM/fixture-gated smokes (never weaken hardware paths for convenience). Current pointers:

| Area | Issue | Fixtures / notes |
| --- | --- | --- |
| +3 FDC / DSK | [#141](https://github.com/mward-sudo/spec_chum/issues/141), [#166](https://github.com/mward-sudo/spec_chum/issues/166), [#164](https://github.com/mward-sudo/spec_chum/issues/164) | `tests/fixtures/plus3/`; machine `insert_disk` typed rejects |
| TR-DOS / Beta Disk | [#140](https://github.com/mward-sudo/spec_chum/issues/140) | `tests/fixtures/trdos/` |
| Interface 1 / Microdrive | [#139](https://github.com/mward-sudo/spec_chum/issues/139) | Attach typed; deepen smokes land with feature work |
| Multiface / DivMMC | [#138](https://github.com/mward-sudo/spec_chum/issues/138), [#168](https://github.com/mward-sudo/spec_chum/issues/168) | Attach typed; ROM-gated behaviour with deepen PRs |
| Timex TC2048 / TS2068 | [#192](https://github.com/mward-sudo/spec_chum/issues/192) | Boot smokes in `machine`; further accuracy on Timex issues |

## Opt-in tooling (evaluated under #171)

| Tool | Status |
| --- | --- |
| `cargo-deny` | **Landed** — `./scripts/check_deny.sh` + blocking CI; egui-transitive RustSec IDs ignored-with-reason |
| `cargo-audit` / Dependabot | Not required for #171 close; revisit with lockfile / advisory cadence |
| `cargo-machete` | Optional unused-deps pass — follow-up if dep drift appears |
| `cargo-llvm-cov` | Optional baseline for `z80` / `ula` / `machine` — not gating |
| `cargo-nextest` | Optional CI runner speedup — not required |
| `miri` | Limited value for cycle-accurate emulator cores — skip unless a pure-safe unit needs it |
| Mutation testing | **Out of scope** unless explicitly requested |

## Related docs

- [AGENTS.md](../AGENTS.md) — crate map, clippy-first workflow, accuracy vs convenience.
- [CONTRIBUTING.md](../CONTRIBUTING.md) — PR / CodeRabbit / TDD expectations.
- [RELEASE.md](RELEASE.md) — tag checklist and slow suite.
- `tests/fixtures/z80test/README.md`, `tests/fixtures/system/README.md` — fixture details.
