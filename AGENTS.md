# Spec Chum — agent notes

Cross-tool project facts for LLM-assisted work. Cursor-specific rules live in `.cursor/rules/`.

## What this is

From-scratch ZX Spectrum emulator in Rust + egui. Cycle-accurate Z80 and ULA timing are first-class goals. System ROMs are **not** in git — use `./scripts/fetch_roms.sh`.

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
| `app` | egui / eframe frontend binary (see `docs/UI_ARCHITECTURE.md`) |

Optional native macOS SwiftUI shell: `apps/macos/` — build with `./scripts/run_macos_app.sh` (see `docs/MACOS_NATIVE.md`).

## Hard constraints

- Do **not** edit plan files under `.cursor/plans/` (or similar).
- Do **not** commit ROM binaries (`roms/`, `*.rom`).
- Library crates: `thiserror` for public errors; no bare `unwrap` in non-test code.
- Binary (`app`): `anyhow` is fine for top-level error context.
- `unsafe` is denied workspace-wide; only introduce it with a documented `SAFETY` rationale and a narrowly scoped `#[allow(unsafe_code)]`.

GitHub Release binaries (egui `spec_chum` + `spec-chum-debug`) are built by
`.github/workflows/release.yml` on `vX.Y.Z` tags. See [docs/RELEASE.md](docs/RELEASE.md).
Do not attach ROMs. Native SwiftUI `.app` / DMG packaging, notarisation, and related CI are tracked in [#68](https://github.com/mward-sudo/spec_chum/issues/68) (out of scope for the CLI release workflow).

## Agent workflow (clippy-first)

Before claiming a task done:

```bash
./scripts/check.sh
```

Or equivalently: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.

## Testing expectations

- Z80: Fuse `tests.in` / `tests.expected` before merging opcode groups.
- Contention / floating bus: table-driven unit tests.
- ROM-dependent integration tests must skip cleanly when `roms/` is missing.
- z80test: `cargo test -p machine --features slow-tests --release z80doc_all_tests_passed` (fixture in `tests/fixtures/z80test/`). [#17](https://github.com/mward-sudo/spec_chum/issues/17) is **done** (`z80doc`); `z80full` remains opt-in/`#[ignore]` — see `.cursor/rules/z80test-issue-17.mdc`.
- System tests (optional, not default CI): `./scripts/run_system_tests.sh` — third-party ULA/ROM TAPs cached in `.rom-cache/system-tests/` (not git; [#108](https://github.com/mward-sudo/spec_chum/issues/108)).

## PR / stack

Track work in GitHub Issues (milestones M0–M4). Prefer small PRs (one concern). See `CONTRIBUTING.md` for `gh stack` commands.

### Stay in sync with issues

Before implementing: `gh issue list` / `gh issue view N` for related work. Prefer extending existing issues over duplicates. Link PRs with `Closes #N` / `Refs #N`. Do not close until acceptance criteria are truly met. When work discovers gaps, update or reopen the issue rather than silently diverging. Cursor rule: `.cursor/rules/github-issues.mdc`.

### CodeRabbit — on-demand + merge gate

`.coderabbit.yaml` disables automatic reviews. Iterate on **draft** PRs (CR completeness not required). When merge-candidate: mark ready → `@coderabbitai full review` (or label `coderabbit-review`) → disposition → `./scripts/check_pr_reviews.sh`. After follow-up commits, request **full review** again (not `@coderabbitai review` — skipped while `auto_incremental_review: false`). See `CONTRIBUTING.md` → “CodeRabbit — review when ready”.

### Before merge — CodeRabbit clean + bot review threads (hard gate)

Any task to **finish, land, or merge a PR** must include this gate. A **ready** PR is **not merge-ready** while **either**:

- CodeRabbit on the latest HEAD is **pending / in progress / rate-limited / skipped** (or missing / failed) — a green `Review rate limited` or `Review skipped` status is **not** a completed review; open a revisit issue and hold; or
- CodeRabbit (or similar bots) have unresolved **actionable** review threads,

unless the user **explicitly** waives. **Drafts** may skip CR completeness in the script but still fail on unresolved bot threads; do not merge drafts.

1. Run `./scripts/check_pr_reviews.sh` (current PR) or `./scripts/check_pr_reviews.sh <n>` — on ready PRs fails on rate-limited/pending/missing CodeRabbit **and** on unresolved bot threads.
2. If missing/pending/skipped: request `@coderabbitai full review` (or label `coderabbit-review`) if on-demand — not `@coderabbitai review` while incremental is disabled; if rate-limited: hold; wait for a completed CR pass on HEAD; do not merge.
3. Fix or reply wontfix, then resolve each thread; re-run the script (and the **Bot review threads** CI check if red).
4. Waiver only with user instruction: `--waive`, `SPEC_CHUM_REVIEW_WAIVER`, or label `waive-bot-reviews` — document on the PR.

CI: `.github/workflows/pr-bot-reviews.yml` (default `GITHUB_TOKEN`). Local/script remains mandatory for agents. See `.cursor/rules/pr-review-merge.mdc` (lesson from [#83](https://github.com/mward-sudo/spec_chum/pull/83)).

Recently closed accuracy/feature issues: [#33](https://github.com/mward-sudo/spec_chum/issues/33) AY, [#34](https://github.com/mward-sudo/spec_chum/issues/34) border/beam, [#24](https://github.com/mward-sudo/spec_chum/issues/24) +2A/+3, [#25](https://github.com/mward-sudo/spec_chum/issues/25) TZX/RZX/Kempston/disk.

