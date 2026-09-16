# Spec Chum — agent notes

Cross-tool project facts for LLM-assisted work. Cursor-specific rules live in `.cursor/rules/`.

## What this is

From-scratch ZX Spectrum emulator in Rust + egui. **Hardware-faithful** cycle-accurate Z80 and ULA timing are first-class goals — prefer real accuracy fixes over weakening tests or leaving suites ignored. System ROMs are **not** in git — use `./scripts/fetch_roms.sh`.

**Convenience exceptions** (flash-load / turbo tape speed, UI helpers) intentionally diverge from real EAR timing but must still **load correctly**. Do not “fix” those paths to match hardware T-states; do not weaken hardware-path assertions to accommodate them. Accuracy tests stay on the real-timing path.

## Crate map

| Crate | Role |
| --- | --- |
| `z80` | CPU core (own implementation; `disasm_one` for dumps/UI) |
| `bus` | Memory / I/O interconnect |
| `ula` | Video, contention, floating bus, border |
| `tape` | Tape loading |
| `formats` | Snapshot / tape file formats |
| `machine` | Wired Spectrum models (`Inspect`, `Debugger`) |
| `trace` | Structured debug ring buffer (env/host gated) |
| `debug_cli` | Headless agent debugger library + `spec-chum-debug` alias; preferred entry is `spec_chum --serve` / `spec_chum debug …` |
| `host_api` | C ABI host surface for native shells / future cores |
| `control_plane` / `agent_server` | Localhost agent debug HTTP API ([#210](https://github.com/mward-sudo/spec_chum/issues/210)) — `spec_chum --serve` / `spec-chum-agent` / `spec-chum-debug --serve`; see [docs/AGENT_DEBUG_API.md](docs/AGENT_DEBUG_API.md) |
| `app` | egui / eframe frontend binary (see `docs/UI_ARCHITECTURE.md`) |
| `windows_shell` | Optional Win32 native shell (`spec_chum_windows`) over `host_api` — [docs/WINDOWS_NATIVE.md](docs/WINDOWS_NATIVE.md) / [#351](https://github.com/mward-sudo/spec_chum/issues/351) |
| `linux_shell` | Optional GTK4 native shell (`spec_chum_linux`) over `host_api` — [docs/LINUX_NATIVE.md](docs/LINUX_NATIVE.md) / [#351](https://github.com/mward-sudo/spec_chum/issues/351); egui remains Linux release primary |
| `living_room` | Experimental Bevy 3D CRT host (`spec-chum-room`); excluded from default `./scripts/check.sh` unless `SPEC_CHUM_CHECK_LIVING_ROOM=1` — see `docs/LIVING_ROOM.md` / #146. SpecChumMac **always links** the living_room staticlib for `host_api`; living-room is opt-in only as a *display mode*. |

Optional native macOS SwiftUI shell: `apps/macos/` — build with `./scripts/run_macos_app.sh` (see `docs/MACOS_NATIVE.md`). Models include distinct **+2A** (tape Loader) and **+3** (disk Loader).

**GUI parity:** egui (`crates/app`), SpecChumMac, the optional Windows Win32 shell (`crates/windows_shell`), and the optional Linux GTK4 shell (`crates/linux_shell`) must stay **feature-aligned** unless a capability is genuinely platform-specific. **Chrome may follow platform HIG** (⌘ vs Ctrl, native menus/materials). Shared logic in `control_plane` / `host_api`; hosts get the same agent HTTP surface. **Never leave another host as a silent follow-up** (vertical-slice deferrals must be explicit in docs). See `.cursor/rules/gui-app-parity.mdc`.

## Hard constraints

- Do **not** edit plan files under `.cursor/plans/` (or similar).
- Do **not** commit ROM binaries (`roms/`, `*.rom`).
- Do **not** change macOS **system** speaker volume (`osascript` `set volume` / `output volume`, CoreAudio device gain, etc.) or force-unmute the Mac. App-internal mute/volume (`specChum.outputVolume`) is the user’s preference — leave it alone unless the user explicitly asks.
- Library crates: `thiserror` for public errors; no bare `unwrap` in non-test code.
- Binary (`app`): `anyhow` is fine for top-level error context.
- `unsafe` is denied workspace-wide; only introduce it with a documented `SAFETY` rationale and a narrowly scoped `#[allow(unsafe_code)]`.

GitHub Release archives (single primary app per platform) are built by
`.github/workflows/release.yml` on `vX.Y.Z` tags. See [docs/RELEASE.md](docs/RELEASE.md).
Do not commit ROM binaries. Release CI fetches redistributable ROMs and embeds
them inside packages (macOS SpecChumMac `.app` / Windows / Linux — see
[docs/ROMS.md](docs/ROMS.md)). macOS ships SpecChumMac as a **`.dmg.zip`**
(unzip → `.dmg` with Applications shortcut; notarised/stapled when Apple
secrets are set;
[#403](https://github.com/mward-sudo/spec_chum/issues/403),
[#363](https://github.com/mward-sudo/spec_chum/issues/363),
[#361](https://github.com/mward-sudo/spec_chum/issues/361),
[#354](https://github.com/mward-sudo/spec_chum/issues/354));
Windows a portable `.zip` **and** Inno Setup `*-setup.exe`; Linux a `.tar.gz`,
**AppImage**, and **`.deb`**. Shared Spectrum app icon (macOS `.icns` / Windows `.ico` /
Linux PNG / egui window) lives under `packaging/` — regenerate with
`python3 scripts/generate_app_icons.py`
([#231](https://github.com/mward-sudo/spec_chum/issues/231)).
Native UI shells (non-macOS): [#351](https://github.com/mward-sudo/spec_chum/issues/351) — strategy in [docs/UI_ARCHITECTURE.md](docs/UI_ARCHITECTURE.md#native-shells-351) (Linux: egui release + optional GTK4 `linux_shell`, see [docs/LINUX_NATIVE.md](docs/LINUX_NATIVE.md); Windows egui release + optional Win32 `windows_shell`, see [docs/WINDOWS_NATIVE.md](docs/WINDOWS_NATIVE.md)).
**Before tagging `vX.Y.Z`:** the full slow suite must pass — `./scripts/run_slow_tests.sh`
(z80doc + system-tests + z80full). Default CI / `./scripts/check.sh` alone is not enough.

## Agent workflow (clippy-first)

**While iterating** — debug-build only crates relevant to the task:

```bash
./scripts/check_crates.sh                 # infer from git diff vs origin/main
./scripts/check_crates.sh control_plane agent_server host_api
```

**Before claiming a task done / merge** — full workspace gate (debug, excludes `living_room`):

```bash
./scripts/check.sh
```

Or equivalently: `cargo fmt --all`, `cargo clippy --workspace --all-targets --exclude living_room -- -D warnings`, `cargo test --workspace --exclude living_room`.

**Living room / SpecChumMac** — Bevy is **release** by default (debug Bevy is multi‑GB):

```bash
./scripts/check_living_room.sh            # clippy+test --release + room_perf
# SPEC_CHUM_ROOM_DEBUG=1 ./scripts/check_living_room.sh   # opt-in disk-heavy debug
./scripts/build_macos_app.sh              # always release staticlib
```

Do not run `cargo check -p living_room` (debug) unless you intentionally need Bevy debug symbols — prefer `--release` or `check_living_room.sh`. Set `SPEC_CHUM_CHECK_LIVING_ROOM=1` only when `./scripts/check.sh` should also run the living-room release gate.

## graphify knowledge graph

Checked-in outputs live under `graphify-out/` (`graph.json`, `graph.html`, `GRAPH_REPORT.md`, `manifest.json`, AST `cache/`). Cursor agents must query the graph before exploring — see `.cursor/rules/graphify.mdc`. CodeRabbit skips `graphify-out/**` via `.coderabbit.yaml` `path_filters` (review-only; keep the tree committed). Local CLI: rely on that YAML, or scope with `coderabbit review --agent --dir crates` / `--dir apps`.

| Task | Command |
| --- | --- |
| Query architecture | `graphify query "…"` / `graphify path A B` / `graphify explain concept` |
| Refresh after **code** edits | `./scripts/graphify_update.sh` (AST-only, no API cost) |
| Refresh after **doc** edits | `./scripts/graphify_update.sh --full` (LLM; needs API key) |
| Auto-refresh on commit | `./scripts/graphify_install_hooks.sh` (once per clone) |
| After task done | Skim `GRAPH_REPORT.md` Suggested Questions → **do now** (same task/PR if tiny) / **defer** (must file or link a GitHub issue) / **ignore** (short rationale) — see `.cursor/rules/graphify.mdc` |

Broad overview: `graphify-out/GRAPH_REPORT.md`. Wiki index (when present): `graphify-out/wiki/index.md`.

## Testing expectations

Tier matrix and gate inventory: [docs/TESTING.md](docs/TESTING.md) ([#171](https://github.com/mward-sudo/spec_chum/issues/171)).

- Z80: Fuse `tests.in` / `tests.expected` before merging opcode groups.
- Contention / floating bus: table-driven unit tests.
- ROM-dependent integration tests must skip cleanly when `roms/` is missing.
- z80test ([#17](https://github.com/mward-sudo/spec_chum/issues/17) done; [#122](https://github.com/mward-sudo/spec_chum/issues/122)): keep `z80doc` / `z80full` green under `--features slow-tests --release` (fixtures in `tests/fixtures/z80test/`; do not reopen #17 / treat as stub). CI filters to `z80doc` by name; **releases require** `./scripts/run_slow_tests.sh` (includes z80full).
- System tests (not default CI): `./scripts/run_system_tests.sh` — third-party ULA/ROM TAPs cached in `.rom-cache/system-tests/` (not git; [#108](https://github.com/mward-sudo/spec_chum/issues/108)). Optional for routine PR work; **required before release** (included in `./scripts/run_slow_tests.sh`).

## PR / stack / merge gate

Track work in GitHub Issues (milestones M0–M4). Prefer small PRs (one concern). See `CONTRIBUTING.md` for `gh stack` and the full merge-gate SSOT.

- Issues: `.cursor/rules/github-issues.mdc` (`gh issue list` before coding; `Closes` vs `Refs`).
- Merge / CodeRabbit: **agent checklist** `.cursor/rules/pr-review-merge.mdc` — **CI soft-pass ≠ merge-ready**; either-completed or dual >10m; disposition every actionable bot finding.
- Detail + human workflow: `CONTRIBUTING.md` → “CodeRabbit — review when ready” / “Review check before merge”.
- Disposition habits: `.cursor/rules/coderabbit-lessons.mdc`.

Closed accuracy/feature baselines (historical): [#33](https://github.com/mward-sudo/spec_chum/issues/33) AY, [#34](https://github.com/mward-sudo/spec_chum/issues/34) border/beam, [#24](https://github.com/mward-sudo/spec_chum/issues/24) +2A/+3, [#25](https://github.com/mward-sudo/spec_chum/issues/25) TZX/RZX/Kempston/disk.

## Cursor Cloud specific instructions

The startup update script already runs `cargo fetch` and `./scripts/fetch_roms.sh`, and the base image already has the audio/GUI system libraries (`libasound2-dev`, `libgtk-3-dev`, `libglib2.0-dev`, `libudev-dev`, plus Mesa/X11 runtime libs). So on a fresh cloud agent you can go straight to building, testing, and running.

Standard commands are in `README.md` and the "Agent workflow" / "Testing expectations" sections above (`./scripts/check.sh`, `cargo test --workspace`, `cargo run -p app --release`). Non-obvious cloud caveats only:

- **ROMs are required for `app` / headless debug execution and ROM-dependent tests, and are not in git.** They live under `roms/` (gitignored) and are fetched by `./scripts/fetch_roms.sh` (already run by the update script). Without them those binaries error at ROM load, and ROM-dependent integration tests skip. Re-run the script if `roms/` is missing.
- **Running the egui app (`spec_chum`) needs an X display.** In the cloud VM the desktop is on `DISPLAY=:1` with software GL (llvmpipe), which works fine. Launch with `DISPLAY=:1 ./target/release/spec_chum` (build release first for smooth interaction). Keep it in a tmux session so it survives.
- **ALSA "cannot find card '0'" / "Unknown PCM default" warnings are harmless.** The VM has no sound card; the app (and `cpal`) degrade gracefully and keep running — do not treat these as failures.
- **Headless emulator driving:** prefer `spec_chum --serve` / `spec_chum debug …`
  (or the source-build alias `spec-chum-debug`) / the **Agent Debug HTTP API**
  (`spec-chum-agent` on `127.0.0.1:17384`). See `.cursor/skills/spec-chum-debugging/SKILL.md`
  and [docs/AGENT_DEBUG_API.md](docs/AGENT_DEBUG_API.md).
- **Driving the GUI keyboard (computer-use):** the Spectrum uses single-key keyword entry — pressing `p` at the `K` cursor inserts the whole `PRINT` keyword, including its trailing space (do not type the word letter-by-letter). Symbol-layer chars via computer-use are flaky (`"` is Shift+apostrophe and often mis-types stray apostrophes); prefer simple numeric expressions like `PRINT 2+2` for reliable smoke tests. Host→matrix mapping lives in `crates/app/src/keymap.rs`.

