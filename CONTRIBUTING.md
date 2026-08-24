# Contributing

## Workflow

- Track work in GitHub Issues (milestones M0–M4).
- Before implementing, check related issues (`gh issue list` / `gh issue view N`). Prefer extending an existing issue over opening a duplicate.
- Use stacked PRs via `gh stack` (one concern per branch).
- Link PRs with `Closes #N` / `Refs #N`.
- Do not close an issue until its acceptance criteria are truly met (placeholder/stub PRs must use `Refs`, not `Closes`).
- When work discovers gaps, update or reopen the issue rather than silently diverging from the tracker.

## Rust practices

- **Edition**: 2021 (workspace). Toolchain: stable with `rustfmt` + `clippy` (`rust-toolchain.toml`).
- **Lints**: shared via `[workspace.lints]` in the root `Cargo.toml`; every crate sets `lints.workspace = true`.
- **`unsafe`**: denied workspace-wide. Only allow with a narrow `#[allow(unsafe_code)]` and a `SAFETY` comment.
- **Errors**: `thiserror` in library crates; `anyhow` is fine in `app`.
- **Formatting / Clippy**: `rustfmt.toml` and `clippy.toml` at the repo root. CI runs `cargo fmt --check` and `cargo clippy --workspace --all-targets -- -D warnings`.
- **API design**: prefer the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/) for public surfaces.

Local quality gate (same as CI intent):

```bash
./scripts/check.sh
```

GitHub Release binaries (macOS / Linux / Windows) are produced by tagging
`vX.Y.Z`. See [docs/RELEASE.md](docs/RELEASE.md).

## AI / agent-assisted work

- Read `AGENTS.md` for crate boundaries and hard constraints.
- Cursor project rules live in `.cursor/rules/` (always-on project policy + Rust globs), including `github-issues.mdc` for tracker sync and `pr-review-merge.mdc` for bot review gates.
- Before coding, agents should consult open issues, so implementations do not drift from tracked acceptance criteria.
- Agents should be **clippy-first**: run `./scripts/check.sh` before claiming done; do not “promise” clean code without running the gate.
- Keep PRs small and crate-scoped so parallel agents do not clobber each other.
- Do **not** edit plan files under `.cursor/plans/` (or similar).
- **Before merge / finish PR:** mark ready if still draft, request one CodeRabbit pass when merge-candidate, then run `./scripts/check_pr_reviews.sh`. **Hold until CodeRabbit is clean** on HEAD (not pending / missing / rate-limited) **and** unresolved **actionable** bot threads are cleared, unless the user explicitly waives. See below.

## CodeRabbit — review when ready (usage)

Repo config: [`.coderabbit.yaml`](.coderabbit.yaml) ([auto-review docs](https://docs.coderabbit.ai/configuration/auto-review), [review commands](https://docs.coderabbit.ai/reference/review-commands)). Automatic reviews are **off** so iteration pushes do not burn allowance. Workflow:

1. Keep the PR as a **draft** while iterating (bot-review check skips CR completeness on drafts).
2. When merge-candidate: mark **Ready for review**, then request a first pass with `@coderabbitai full review` or label `coderabbit-review`.
3. After fixing review findings (commits that move HEAD): request an **incremental** pass with `@coderabbitai review`. Do not burn another full review for post-fix HEAD unless the prior full review never completed or CodeRabbit asks for one.
4. Disposition actionable threads (including outside-diff / summary nits that are not GraphQL threads), then run the merge gate below.

**Human trigger preferred:** CodeRabbit may ignore review commands and thread replies from other GitHub bots (`Skipped: comment is from another GitHub bot`). Prefer a human `@coderabbitai …` comment when agents’ requests are ignored. Label `coderabbit-review` remains a backup.

`auto_incremental_review` in `.coderabbit.yaml` controls **automatic** re-review on later pushes when auto-review/labels apply; it is not a prerequisite for the manual `@coderabbitai review` command (per CodeRabbit docs). Keep it `true` so label-triggered follow-ups stay incremental-friendly.

If the YAML and CodeRabbit GitHub app UI disagree, keep **Automatic Reviews** off in the app so committed config wins. Undraft alone may not trigger a review — always comment or label.

## Review check before merge

Lesson from [#83](https://github.com/mward-sudo/spec_chum/pull/83): do not ignore CodeRabbit. For **ready** PRs, hold until a completed review on HEAD **and** unresolved actionable bot threads are dispositioned.

**Hold policy (ready / non-draft):** Do not merge while CodeRabbit’s commit status on the PR head is pending, in progress/queued, **rate-limited**, **skipped**, failed, or missing (including when on-demand review was never requested). CodeRabbit may report `Review rate limited` or `Review skipped: …` with a green/success state — that is still a hold. Prefer a follow-up issue titled like `Revisit CodeRabbit on PR #N (rate-limited)` rather than merging.

**Drafts:** `./scripts/check_pr_reviews.sh` skips CodeRabbit HEAD completeness but still fails on unresolved bot threads. Do not merge drafts.

**CI:** workflow **Bot review threads** (`.github/workflows/pr-bot-reviews.yml`) runs `./scripts/check_pr_reviews.sh` on PRs (and when reviews/comments arrive) using the default `GITHUB_TOKEN` (`pull-requests: read`). Treat a failing check as blocking for merge. After resolving threads or when CodeRabbit finishes, re-run that job if GitHub did not re-trigger it.

**Agents / local (mandatory before merge):**

```bash
./scripts/check_pr_reviews.sh          # current branch PR
./scripts/check_pr_reviews.sh 87       # explicit PR
# Explicit waiver only when the user asked for one:
./scripts/check_pr_reviews.sh 87 --waive "user waived nit: comma-only"
# Or add PR label: waive-bot-reviews
```

The script (1) on ready PRs checks CodeRabbit on HEAD and **hard-fails** on pending / rate-limited / skipped / missing / error (drafts skip this step), then (2) paginates GraphQL `reviewThreads`, prints unresolved bot comment URLs, and exits non-zero unless waived. Cursor rule: `.cursor/rules/pr-review-merge.mdc`.

## TDD

- Z80: Fuse `tests.in` / `tests.expected` before merging opcode groups.
- Contention / floating bus: table-driven unit tests required.
- Integration tests that need ROMs must skip cleanly when `roms/` is missing.
- **z80test** (Patrik Rak): `cargo test -p machine --features slow-tests --release z80doc_all_tests_passed -- --nocapture`.
  For `z80full`: `cargo test -p machine --features slow-tests --release z80full_all_tests_passed -- --nocapture` (fixture checked in; also `./scripts/fetch_z80test.sh` if missing).
- **System tests** (Bobrowski Minfo / ULA test 3, Rak Timing Test, Sinclair ROM boot): slow; not in `./scripts/check.sh`. `./scripts/run_system_tests.sh` (see `tests/fixtures/system/README.md`). Failures are real accuracy misses — do not stub them. Optional for routine PRs; **required before a `vX.Y.Z` release**.
- **Before tagging a release:** run `./scripts/run_slow_tests.sh` (z80doc + system-tests + z80full). See [docs/RELEASE.md](docs/RELEASE.md).
- **GUI**: logic lives in `app` as a library. Headless tests use `EmulatorSession` (no display) plus an egui `Context::run` smoke test — no xvfb required for CI.

## Stack commands (non-interactive)

```bash
gh stack init <branch>
gh stack add <branch>
gh stack submit --auto --open
gh stack view --json
gh stack sync --prune
```
