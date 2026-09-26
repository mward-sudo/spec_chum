# Spec Chum — agent notes

Cross-tool project facts for LLM-assisted work. Cursor-specific rules live in `.cursor/rules/`.

## Jev-guided workflow

For software-engineering tasks, use configured Jev MCP `jev_decide` once at intake with `decision_kind="route"` and a short, redacted summary. Keep the call lightweight; if unavailable, continue with task evidence and ordinary judgment. Jev advises; deterministic checks establish facts. Do not send secrets, full transcripts, or large source excerpts.

At a genuine loop boundary—after a coherent implementation and its checks, a failed attempt, or when next action/completion is uncertain—use Jev for a `loop` decision. Do not call it after each edit or check. If unavailable, continue on evidence. Treat `review_recommended` as low confidence, not code review or proof; it cannot override evidence, user direction, or project gates.

For related Jev decisions in one task, reuse the same short, non-sensitive `task_id`. Jev records a provider session ID and a repository label automatically. After acting on a recommendation and learning the factual result, call `jev_log_update` with the returned `decision_id`, whether the recommendation was followed, overridden, or not applicable, and a concise outcome note. Do not guess outcomes or include secrets, full transcripts, or sensitive source details. Historical log entries are not to be backfilled from assumptions.

Treat Jev's low-confidence signal as a reason for conservative main-agent review, but make independent review requirements from project policy and task risk. That flag alone does not justify spending CodeRabbit's limited PR or CLI review allowance. Required review still applies when project policy, the PR merge gate, broad impact, security sensitivity, or unresolved uncertainty calls for it. Every ready PR follows `.cursor/rules/pr-review-merge.mdc` and the required CodeRabbit merge gate; Jev cannot waive or replace that process. Never use Jev in place of compilation, tests, lint, diff inspection, CodeRabbit checks, or a human review required by project policy. Read its full probability distribution and `review_recommended`; if the top probability is below 0.70 or the top-two margin is below 0.20, have the main agent choose conservatively and review the result. These thresholds are operational heuristics, not calibrated guarantees.

Use Jev's route to select the lowest sufficient reasoning lane (SMALL main session, MEDIUM Luna medium, HIGH Luna high, ESCALATE Sol high for security-sensitive work, architecture, migrations, broad impact, or repeated failures). Delegate one bounded implementation/diagnosis task at a time, with ownership and expected output; skip delegation when direct work is faster. The coordinating agent owns scope, acceptance criteria, integration, verification, and final review.

## Agentic coding loop

Use this loop for implementation work; scale the ceremony to the task. A one-file, obvious fix needs a short scope and a relevant check, not a project plan. A multi-step or cross-cutting task needs an explicit working record.

1. **Frame the outcome.** Identify what “done” means, relevant constraints, and acceptance checks. Find the related issue when useful or required by task policy; do not create tracking ceremony for a tiny, self-contained fix. Ask only when missing intent/product judgment materially changes the answer. Continue independent work while awaiting a response.
2. **Inspect the workspace.** Check branch/status and preserve unrelated edits. Read relevant instructions and source context; use graphify and issue lookup as the project policies require. Select tools or subagents only when they add value. Give delegated work a bounded question or ownership area and integrate its findings yourself.
3. **Implement a useful slice.** For user-visible features, prefer vertical slices that deliver one observable behavior through every required layer (shared/core behavior, host/UI surface, and an end-to-end or integration check). Each slice should be independently usable or provide a clear, testable increment. Avoid broad horizontal scaffolding across layers that leaves no working path. Keep the slice small enough to review; split larger outcomes into ordered, behavior-complete increments and keep the acceptance criteria explicit. Use horizontal implementation when the problem is inherently layer-wide or accuracy-focused—such as a CPU instruction/timing correction, shared API/schema migration, mechanical refactor, or a single invariant that must change consistently across layers. In either shape, run the narrowest check that can falsify correctness, inspect failures, and revise the hypothesis before retrying. Add/update tests that capture intended behavior and preserve hardware-accuracy assertions.
4. **Verify the slice and close.** Check the behavior across each layer included in the slice, including the user-facing path when practical; do not rely only on isolated unit tests when an integration test can establish the end-to-end outcome. Run task-appropriate required checks (crate iteration, workspace, or slow/release gates). Review the final diff for scope, accidental/generated files, and unresolved findings. Update graph artifacts after code edits when required. Meet the independent PR review/merge policy when applicable; a green build does not satisfy it.
5. **Leave useful state.** Report the outcome, paths, checks and exact results, limitations, and deferred work. A same-thread continuation can use the conversation. For cross-session or cross-agent handoff, leave a durable concise note in the active issue/PR (or equivalent task record) with goal, branch/HEAD, changed paths, decisions, checks and exact outcomes, blockers, and next action. Avoid a separate progress file unless no existing record fits.

### Working record and continuation

Use the lightest record that makes the work resumable. Issues capture agreed intent and acceptance criteria, including intended behavior slices for substantive features; PRs capture the delivered slice, checks, review, and merge state; durable handoff notes capture cross-session execution state. Update the existing source of truth when scope, decisions, status, or follow-up changes. Do not mirror details or paste chat transcripts. Never mark work complete while required criteria/checks remain unmet. `continue` resumes active scope; shorthand ordering is in CONTRIBUTING.md.

### Human decision points

Default to autonomous progress on reversible local work. Human input is most valuable for product intent, ambiguous tradeoffs, acceptance of a known limitation, prioritization, and actions with external or hard-to-reverse effects (for example spend, publish, deploy, merge, delete, or change user preferences) when not already authorized. Honor authorization already given; do not ask again. When input is necessary, finish independent work first and ask one concrete, decision-shaped question with a recommended option and consequence. Do not stop for hypothetical risk or routine implementation choices.

Use judgment about review depth: main-agent diff review is always expected; independent agent or tool review is most useful for security, architecture, broad changes, public API/format changes, concurrency/timing accuracy, and unresolved uncertainty. Do not spend scarce external review quota merely because a routing signal is uncertain. Project PR policy still defines mandatory review gates.

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
| `windows_shell` | Native Win32 shell (`spec_chum_windows` → release `spec_chum.exe`) over `host_api` — [docs/WINDOWS_NATIVE.md](docs/WINDOWS_NATIVE.md) / [#351](https://github.com/mward-sudo/spec_chum/issues/351) |
| `linux_shell` | Native GTK4 shell (`spec_chum_linux` → release `spec_chum`) over `host_api` — [docs/LINUX_NATIVE.md](docs/LINUX_NATIVE.md) / [#351](https://github.com/mward-sudo/spec_chum/issues/351); egui remains Linux CI / headless fallback |
| `living_room` | Experimental Bevy 3D CRT host (`spec-chum-room`); excluded from default `./scripts/check.sh` unless `SPEC_CHUM_CHECK_LIVING_ROOM=1` — see `docs/LIVING_ROOM.md` / #146. SpecChumMac **always links** the living_room staticlib for `host_api`; living-room is opt-in only as a *display mode*. |

Optional native macOS SwiftUI shell: `apps/macos/` — build with `./scripts/run_macos_app.sh` (see `docs/MACOS_NATIVE.md`). Models include distinct **+2A** (tape Loader) and **+3** (disk Loader).

**GUI parity:** egui (`crates/app`), SpecChumMac, the Windows Win32 shell (`crates/windows_shell`; release primary), and the Linux GTK4 shell (`crates/linux_shell`; release primary) must stay **feature-aligned** unless a capability is genuinely platform-specific. **Chrome may follow platform HIG** (⌘ vs Ctrl, native menus/materials). Shared logic in `control_plane` / `host_api`; hosts get the same agent HTTP surface. **Never leave another host as a silent follow-up** (vertical-slice deferrals must be explicit in docs). See `.cursor/rules/gui-app-parity.mdc`.

## Hard constraints

- Do **not** edit plan files under `.cursor/plans/` (or similar).
- Do **not** commit ROM binaries (`roms/`, `*.rom`).
- Do **not** change macOS **system** speaker volume (`osascript` `set volume` / `output volume`, CoreAudio device gain, etc.) or force-unmute the Mac. App-internal mute/volume (`specChum.outputVolume`) is the user’s preference — leave it alone unless the user explicitly asks.
- Non-test code must not use bare `unwrap`; the workspace Clippy `unwrap_used = warn` lint is promoted to an error by CI/check scripts (`-D warnings`). Prefer `?` or return a typed error; use `expect` only for a documented invariant that cannot fail at that point, with a message stating the invariant. `expect_used` is allowed by Clippy, so this is a project rule rather than an enforced lint.
- Library crates: `thiserror` for public errors.
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
Native UI shells: [#351](https://github.com/mward-sudo/spec_chum/issues/351) — strategy in [docs/UI_ARCHITECTURE.md](docs/UI_ARCHITECTURE.md#native-shells-351) (Windows: Win32 `windows_shell` release primary, see [docs/WINDOWS_NATIVE.md](docs/WINDOWS_NATIVE.md); Linux: GTK4 `linux_shell` release primary, see [docs/LINUX_NATIVE.md](docs/LINUX_NATIVE.md)).
**Before tagging `vX.Y.Z`:** the full slow suite must pass — `./scripts/run_slow_tests.sh`
(z80doc + system-tests + z80full). Default CI / `./scripts/check.sh` alone is not enough.

## Verification workflow (clippy-first)

### Continue shorthand

When a task is active, `continue` resumes that work with its existing scope; it does not add a request for review or merge unless those were already requested. With no active task, `continue` means pick an appropriate open issue and implement it. The shorthand encodes order: `CRM` means continue implementation, then review, then merge; `RMC` means review and merge the current work, then continue; `RM` means review and merge the current work. Follow the repository's review and merge gates before merging.

**While iterating** — debug-build only crates relevant to the task:

```bash
./scripts/check_crates.sh                 # infer from git diff vs origin/main
./scripts/check_crates.sh control_plane agent_server host_api
```

**For full-workspace / merge readiness** — full workspace gate (debug, excludes `living_room`):

```bash
./scripts/check.sh
```

Or equivalently: `cargo fmt --all`, `cargo clippy --workspace --all-targets --exclude living_room -- -D warnings`, `cargo test --workspace --exclude living_room`.

**Living room / SpecChumMac** — Bevy is **release** by default (debug Bevy is multi‑GB):

```bash
./scripts/check_living_room.sh            # clippy+test --release + room_perf
# SPEC_CHUM_ROOM_DEBUG=1 ./scripts/check_living_room.sh   # opt-in disk-heavy debug
./scripts/build_macos_app.sh              # always release staticlib
# Optional: offload Cargo/Swift caches to External SSD when mounted
source scripts/dev_env.sh
```

Do not run `cargo check -p living_room` (debug) unless you intentionally need Bevy debug symbols — prefer `--release` or `check_living_room.sh`. Set `SPEC_CHUM_CHECK_LIVING_ROOM=1` only when `./scripts/check.sh` should also run the living-room release gate.

## graphify knowledge graph

Checked-in graph outputs live under `graphify-out/` (`graph.json`, `graph.html`, `GRAPH_REPORT.md`, `manifest.json`, AST `cache/`). CodeRabbit skips `graphify-out/**` via `.coderabbit.yaml` `path_filters` (review-only; keep the tree committed). Local CLI: rely on that YAML, or scope with `coderabbit review --agent --dir crates` / `--dir apps`.

### Instruction precedence

User instructions take precedence, followed by this cross-tool `AGENTS.md`, then tool-specific instructions (such as `.cursor/rules/` or another agent's local rules). Tool-specific rules may add workflow details but must not override user instructions or project-wide constraints here. `CONTRIBUTING.md` and `docs/` provide contributor and topic detail; they do not override these instructions. Keep shared constraints and facts in this file, and tool-only workflow in the relevant tool configuration.

### CLI setup and fallback

Install the current CLI with `uv tool install graphifyy` (or `pipx install graphifyy`), then ensure the tool's bin directory is on `PATH`. Confirm setup with `command -v graphify` and `graphify --version`. If graphify is unavailable, use the checked-in `graphify-out/` artifacts when useful, then continue with normal source exploration; record that the graph query was unavailable. Graph queries remain the preferred architecture entry point when the CLI is available.

| Task | Command |
| --- | --- |
| Query architecture | `graphify query "…"` / `graphify path A B` / `graphify explain concept` |
| Refresh after **code** edits | `./scripts/graphify_update.sh` (AST-only, no API cost) |
| Refresh after **doc** edits | `./scripts/graphify_update.sh --full` (LLM; needs API key) |
| Auto-refresh on commit | `./scripts/graphify_install_hooks.sh` (once per clone) |
| After substantive code/architecture work | Optionally skim `GRAPH_REPORT.md` Suggested Questions; track a relevant follow-up only if it is actionable and worth doing — see `.cursor/rules/graphify.mdc` |

Broad overview: `graphify-out/GRAPH_REPORT.md`. Wiki index (when present): `graphify-out/wiki/index.md`.

## Testing expectations

Tier matrix and gate inventory: [docs/TESTING.md](docs/TESTING.md) ([#171](https://github.com/mward-sudo/spec_chum/issues/171)).

- Z80: Fuse `tests.in` / `tests.expected` before merging opcode groups.
- Contention / floating bus: table-driven unit tests.
- ROM-dependent integration tests must skip cleanly when `roms/` is missing.
- z80test ([#17](https://github.com/mward-sudo/spec_chum/issues/17) done; [#122](https://github.com/mward-sudo/spec_chum/issues/122)): keep `z80doc` / `z80full` green under `--features slow-tests --release` (fixtures in `tests/fixtures/z80test/`; do not reopen #17 / treat as stub). CI filters to `z80doc` by name; **releases require** `./scripts/run_slow_tests.sh` (includes z80full).
- System tests (not default CI): `./scripts/run_system_tests.sh` — third-party ULA/ROM TAPs cached in `.rom-cache/system-tests/` (not git; [#108](https://github.com/mward-sudo/spec_chum/issues/108)). Optional for routine PR work; **required before release** (included in `./scripts/run_slow_tests.sh`).

## PR / stack / merge gate

Track substantive planned work in GitHub Issues (milestones M0–M4); tiny self-contained fixes can remain untracked. Prefer focused PRs. See `CONTRIBUTING.md` for `gh stack` and the full merge-gate SSOT.

- Issues: `.cursor/rules/github-issues.mdc` (check related work for substantive implementation; `Closes` vs `Refs`).
- Merge / CodeRabbit: **agent checklist** `.cursor/rules/pr-review-merge.mdc` — **CI soft-pass ≠ merge-ready**; GitHub review completed or dual >10m; disposition every actionable bot finding.
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
