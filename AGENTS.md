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
| `debug_cli` | Headless agent debugger binary `spec-chum-debug` |
| `host_api` | C ABI host surface for native shells / future cores |
| `control_plane` / `agent_server` | Localhost agent debug HTTP API ([#210](https://github.com/mward-sudo/spec_chum/issues/210)) — `spec-chum-agent` / `spec-chum-debug --serve`; see [docs/AGENT_DEBUG_API.md](docs/AGENT_DEBUG_API.md) |
| `app` | egui / eframe frontend binary (see `docs/UI_ARCHITECTURE.md`) |
| `living_room` | Experimental Bevy 3D CRT host (`spec-chum-room`); excluded from default `./scripts/check.sh` unless `SPEC_CHUM_CHECK_LIVING_ROOM=1` — see `docs/LIVING_ROOM.md` / #146. SpecChumMac **always links** the living_room staticlib for `host_api`; living-room is opt-in only as a *display mode*. |

Optional native macOS SwiftUI shell: `apps/macos/` — build with `./scripts/run_macos_app.sh` (see `docs/MACOS_NATIVE.md`). Models include distinct **+2A** (tape Loader) and **+3** (disk Loader).

## Hard constraints

- Do **not** edit plan files under `.cursor/plans/` (or similar).
- Do **not** commit ROM binaries (`roms/`, `*.rom`).
- Do **not** change macOS **system** speaker volume (`osascript` `set volume` / `output volume`, CoreAudio device gain, etc.) or force-unmute the Mac. App-internal mute/volume (`specChum.outputVolume`) is the user’s preference — leave it alone unless the user explicitly asks.
- Library crates: `thiserror` for public errors; no bare `unwrap` in non-test code.
- Binary (`app`): `anyhow` is fine for top-level error context.
- `unsafe` is denied workspace-wide; only introduce it with a documented `SAFETY` rationale and a narrowly scoped `#[allow(unsafe_code)]`.

GitHub Release archives (egui `spec_chum` + `spec-chum-debug`) are built by
`.github/workflows/release.yml` on `vX.Y.Z` tags. See [docs/RELEASE.md](docs/RELEASE.md).
Do not attach ROMs. macOS ships an egui-wrapped `Spec Chum.app` in a `.zip`;
Windows a `.zip` of `.exe`s; Linux a `.tar.gz`. Native SwiftUI `.app` / DMG /
notarisation remain [#68](https://github.com/mward-sudo/spec_chum/issues/68).
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

## Testing expectations

- Z80: Fuse `tests.in` / `tests.expected` before merging opcode groups.
- Contention / floating bus: table-driven unit tests.
- ROM-dependent integration tests must skip cleanly when `roms/` is missing.
- z80test: `cargo test -p machine --features slow-tests --release z80doc_all_tests_passed` and `… z80full_all_tests_passed` (fixtures in `tests/fixtures/z80test/`). [#17](https://github.com/mward-sudo/spec_chum/issues/17) / [#122](https://github.com/mward-sudo/spec_chum/issues/122) — see `.cursor/rules/z80test-issue-17.mdc`. **Releases require** `./scripts/run_slow_tests.sh` (includes z80full).
- System tests (not default CI): `./scripts/run_system_tests.sh` — third-party ULA/ROM TAPs cached in `.rom-cache/system-tests/` (not git; [#108](https://github.com/mward-sudo/spec_chum/issues/108)). Optional for routine PR work; **required before release** (included in `./scripts/run_slow_tests.sh`).

## PR / stack

Track work in GitHub Issues (milestones M0–M4). Prefer small PRs (one concern). See `CONTRIBUTING.md` for `gh stack` commands.

### Stay in sync with issues

Before implementing: `gh issue list` / `gh issue view N` for related work. Prefer extending existing issues over duplicates. Link PRs with `Closes #N` / `Refs #N`. Do not close until acceptance criteria are truly met. When work discovers gaps, update or reopen the issue rather than silently diverging. Cursor rule: `.cursor/rules/github-issues.mdc`.

### CodeRabbit — on-demand + merge gate

For in-editor / local review before push, prefer the **CodeRabbit Cursor plugin** or `coderabbit review --agent` (`CONTRIBUTING.md` → “CodeRabbit — in-editor review”) — required habit before merge-candidate when CLI available. `.coderabbit.yaml` disables automatic reviews. Iterate on **draft** PRs (GitHub CR completeness not required). When merge-candidate: mark ready → `@coderabbitai full review` (or label) **if not rate-limited** → disposition → `./scripts/check_pr_reviews.sh`. After fix commits: **`@coderabbitai review`** (incremental). Prefer a **human** trigger — CodeRabbit may ignore other bots. Disposition outside-diff / summary nits too (gate only sees GraphQL threads). See `CONTRIBUTING.md` → “CodeRabbit — review when ready”.

### Before merge — CodeRabbit + bot review threads (hard gate)

Any task to **finish, land, or merge a PR** must include this gate. A **ready** PR is **not merge-ready** while **either**:

- CodeRabbit on HEAD is **pending / in progress / missing / failed** — hold for a completed GitHub review when possible; or
- CodeRabbit (or similar bots) have unresolved **actionable** review threads,

unless the user **explicitly** waives. **Rate-limited / skipped** (including green `Review rate limited`) is **not** “Review completed” and is **not** a hard merge hold: gate 1 soft-passes with a warning; agents must have run local CR and dispositioned threads; optional revisit issue (e.g. [#181](https://github.com/mward-sudo/spec_chum/issues/181)). **Drafts** may skip CR completeness in the script but still fail on unresolved bot threads; do not merge drafts.

1. Run `./scripts/check_pr_reviews.sh` (current PR) or `./scripts/check_pr_reviews.sh <n>` — ready PRs: hard-fail pending/missing/error; soft-pass rate-limited/skipped; hard-fail unresolved bot threads.
2. If missing/pending: first pass → `@coderabbitai full review` (or label); after prior full review + fixes → `@coderabbitai review`. If rate-limited: do not burn quota; rely on local CR + soft-pass; optional revisit issue.
3. Disposition each finding (threads **and** actionable outside-diff / summary nits): fix, wontfix+resolve, or open a follow-up issue then resolve — never leave actionable comments hanging; re-run the script (and the **Bot review threads** CI check if red).
4. Waiver only with user instruction: `--waive`, `SPEC_CHUM_REVIEW_WAIVER`, or label `waive-bot-reviews` — document on the PR. (Rate-limit soft-pass does not require this waiver.)

CI: `.github/workflows/pr-bot-reviews.yml` (default `GITHUB_TOKEN`). Local/script remains mandatory for agents. See `.cursor/rules/pr-review-merge.mdc` (lesson from [#83](https://github.com/mward-sudo/spec_chum/pull/83)).

Recently closed accuracy/feature issues: [#33](https://github.com/mward-sudo/spec_chum/issues/33) AY, [#34](https://github.com/mward-sudo/spec_chum/issues/34) border/beam, [#24](https://github.com/mward-sudo/spec_chum/issues/24) +2A/+3, [#25](https://github.com/mward-sudo/spec_chum/issues/25) TZX/RZX/Kempston/disk.

## Cursor Cloud specific instructions

The startup update script already runs `cargo fetch` and `./scripts/fetch_roms.sh`, and the base image already has the audio/GUI system libraries (`libasound2-dev`, `libgtk-3-dev`, `libglib2.0-dev`, `libudev-dev`, plus Mesa/X11 runtime libs). So on a fresh cloud agent you can go straight to building, testing, and running.

Standard commands are in `README.md` and the "Agent workflow" / "Testing expectations" sections above (`./scripts/check.sh`, `cargo test --workspace`, `cargo run -p app --release`). Non-obvious cloud caveats only:

- **ROMs are required for `app` / `spec-chum-debug` execution and ROM-dependent tests, and are not in git.** They live under `roms/` (gitignored) and are fetched by `./scripts/fetch_roms.sh` (already run by the update script). Without them those binaries error at ROM load, and ROM-dependent integration tests skip. Re-run the script if `roms/` is missing.
- **Running the egui app (`spec_chum`) needs an X display.** In the cloud VM the desktop is on `DISPLAY=:1` with software GL (llvmpipe), which works fine. Launch with `DISPLAY=:1 ./target/release/spec_chum` (build release first for smooth interaction). Keep it in a tmux session so it survives.
- **ALSA "cannot find card '0'" / "Unknown PCM default" warnings are harmless.** The VM has no sound card; the app (and `cpal`) degrade gracefully and keep running — do not treat these as failures.
- **Headless emulator driving:** prefer `spec-chum-debug` locally or the **Agent Debug HTTP API** (`spec-chum-agent` / `spec-chum-debug --serve` on `127.0.0.1:17384`) for long-lived sessions and 1:1 framebuffer PNG export. See `.cursor/skills/spec-chum-debugging/SKILL.md` and [docs/AGENT_DEBUG_API.md](docs/AGENT_DEBUG_API.md).
- **Driving the GUI keyboard (computer-use):** the Spectrum uses single-key keyword entry — pressing `p` at the `K` cursor inserts the whole `PRINT` keyword, including its trailing space (do not type the word letter-by-letter). Symbol-layer chars via computer-use are flaky (`"` is Shift+apostrophe and often mis-types stray apostrophes); prefer simple numeric expressions like `PRINT 2+2` for reliable smoke tests. Host→matrix mapping lives in `crates/app/src/keymap.rs`.

