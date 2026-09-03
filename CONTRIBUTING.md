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
./scripts/check_crates.sh    # while iterating — debug clippy/test for changed crates only
./scripts/check.sh           # before merge — full workspace (excl. living_room)
```

Living room / SpecChumMac Bevy gate uses **release** by default (`./scripts/check_living_room.sh`; set `SPEC_CHUM_ROOM_DEBUG=1` only if you need debug Bevy symbols).

Optional native macOS SwiftUI shell compile ([#68](https://github.com/mward-sudo/spec_chum/issues/68)): CI job **`macos-shell`** on `macos-latest` runs `./scripts/build_macos_app.sh`. It is a separate job from Linux **`check`** (does not block Rust fmt/clippy/test). See [docs/MACOS_NATIVE.md](docs/MACOS_NATIVE.md).

GitHub Release binaries (macOS / Linux / Windows) are produced by tagging
`vX.Y.Z`. See [docs/RELEASE.md](docs/RELEASE.md).

## AI / agent-assisted work

- Read `AGENTS.md` for crate boundaries and hard constraints.
- **graphify:** after Rust changes run `./scripts/graphify_update.sh` and commit `graphify-out/` when the graph changes; optional `./scripts/graphify_install_hooks.sh` for post-commit refresh. See `AGENTS.md` → “graphify knowledge graph”.
- Cursor project rules live in `.cursor/rules/` (always-on project policy + Rust globs), including `github-issues.mdc` for tracker sync and `pr-review-merge.mdc` for bot review gates.
- Before coding, agents should consult open issues, so implementations do not drift from tracked acceptance criteria.
- Agents should be **clippy-first**: iterate with `./scripts/check_crates.sh`, then run `./scripts/check.sh` before claiming done; do not “promise” clean code without running the gate.
- Keep PRs small and crate-scoped so parallel agents do not clobber each other.
- **Agent debugging:** today use `spec-chum-debug` + [DEBUGGING.md](docs/DEBUGGING.md). **Planned:** unified localhost [Agent Debug API](docs/AGENT_DEBUG_API.md) ([#210](https://github.com/mward-sudo/spec_chum/issues/210)) — agents export the guest framebuffer as 1:1 PNG (not window capture); `spec-chum-debug` becomes an API client.
- Do **not** edit plan files under `.cursor/plans/` (or similar).
- **Before merge / finish PR:** mark ready if still draft; run local CR (Cursor plugin / CLI); request GitHub CodeRabbit (`@coderabbitai full review` or label) when merge-candidate if not rate-limited; then run `./scripts/check_pr_reviews.sh`. **Hold** on pending/missing/error CodeRabbit, **on-demand / label skips** (review never requested), or unresolved **actionable** bot threads unless the user explicitly waives. **Rate-limited** (after a request) soft-passes gate 1 (not “Review completed”) when local CR was clean and threads are dispositioned. Soft-pass ≠ skip-without-request. See below.

## CodeRabbit — in-editor review (Cursor plugin)

For **local / in-editor** review while iterating (staged, uncommitted, or branch diffs), use the **CodeRabbit Cursor plugin** — the preferred path in Cursor. Ask to review your changes; agents route generic review requests to CodeRabbit via the plugin skill.

This **complements** the GitHub PR workflow below. Ready PRs **must** request `@coderabbitai full review` / `@coderabbitai review` on GitHub when merge-candidate (or label `coderabbit-review`), plus `./scripts/check_pr_reviews.sh` before merge. When GitHub is **rate-limited after a request**, clean local CR + dispositioned threads satisfy the soft-pass path. On-demand / label skips hard-fail — request a review.

**CLI / path scope:** [`.coderabbit.yaml`](.coderabbit.yaml) `reviews.path_filters` excludes `graphify-out/**` (checked-in knowledge graph — still committed; review-only skip). Local CLI also respects that YAML; to further scope a review away from graph artifacts, use `--dir` (e.g. `coderabbit review --agent --dir crates` or `--dir apps`). There is no separate exclude-path CLI flag.

## CodeRabbit — review when ready (usage)

Repo config: [`.coderabbit.yaml`](.coderabbit.yaml) ([auto-review docs](https://docs.coderabbit.ai/configuration/auto-review), [review commands](https://docs.coderabbit.ai/reference/review-commands)). Automatic reviews are **off** so iteration pushes do not burn allowance. `path_filters` skips lockfiles, `.cursor/plans/`, and `graphify-out/**`. Workflow:

1. Keep the PR as a **draft** while iterating (bot-review check skips CR completeness on drafts).
2. When merge-candidate: mark **Ready for review**, then **must** request a first pass with `@coderabbitai full review` or label `coderabbit-review`. Undraft alone does not request a review and must not soft-pass the merge gate.
3. After fixing review findings (commits that move HEAD): request an **incremental** pass with `@coderabbitai review`. Do not burn another full review for post-fix HEAD unless the prior full review never completed or CodeRabbit asks for one.
4. Disposition **every** actionable finding (threads **and** outside-diff / summary nits): fix in code, reply wontfix with reason and resolve, or open a follow-up GitHub issue (preferred for deferred nits) then resolve — never leave them hanging. Then run the merge gate below.

**Human trigger preferred:** CodeRabbit may ignore review commands and thread replies from other GitHub bots (`Skipped: comment is from another GitHub bot`). Prefer a human `@coderabbitai …` comment when agents’ requests are ignored. Label `coderabbit-review` remains a backup.

`auto_incremental_review` in `.coderabbit.yaml` controls **automatic** re-review on later pushes when auto-review/labels apply; it is not a prerequisite for the manual `@coderabbitai review` command (per CodeRabbit docs). Keep it `true` so label-triggered follow-ups stay incremental-friendly.

If the YAML and CodeRabbit GitHub app UI disagree, keep **Automatic Reviews** off in the app so committed config wins. Undraft alone may not trigger a review — always comment or label.

## Review check before merge

Lesson from [#83](https://github.com/mward-sudo/spec_chum/pull/83): do not ignore CodeRabbit. For **ready** PRs, prefer a completed GitHub review on HEAD when quota allows; always disposition unresolved actionable bot threads.

**Hold policy (ready / non-draft):**

- **Hard-fail gate 1:** CodeRabbit commit status pending, in progress/queued, failed/errored, **missing** (never requested / no status), or **on-demand / label skip** (`Review skipped: excluded by label configuration`, `Review skipped: on demand`, etc. — same as never requested). Soft-pass ≠ skip-without-request.
- **Soft-pass gate 1:** `Review rate limited` (or similar quota unavailability *after* a review was requested; often green) — **not** treated as `Review completed`. Script warns loudly and continues. Agents must have run **local** CR (Cursor plugin / `coderabbit review --agent`) and dispositioned findings. Optional revisit issue (e.g. [#181](https://github.com/mward-sudo/spec_chum/issues/181)) when quota resets — not a merge hold. If both local CLI and GitHub stay rate-limited for >10m with no completed review, ask the user before merging. User-explicit `--waive` / label remains available for any hold when the user asks.
- **Hard-fail gate 2:** unresolved actionable bot review threads (unless user waives).

**Drafts:** `./scripts/check_pr_reviews.sh` skips CodeRabbit HEAD completeness but still fails on unresolved bot threads. Do not merge drafts.

**CI:** workflow **Bot review threads** (`.github/workflows/pr-bot-reviews.yml`) runs `./scripts/check_pr_reviews.sh` on PRs (and when reviews/comments arrive) using the default `GITHUB_TOKEN` (`pull-requests: read`). Treat a failing check as blocking for merge. After resolving threads or when CodeRabbit finishes, re-run that job if GitHub did not re-trigger it.

**Agents / local (mandatory before merge):**

```bash
./scripts/check_pr_reviews.sh          # current branch PR
./scripts/check_pr_reviews.sh 87       # explicit PR
./scripts/check_pr_reviews.sh --self-test   # classify soft-pass / hold cases
# Explicit waiver only when the user asked for one:
./scripts/check_pr_reviews.sh 87 --waive "user waived nit: comma-only"
# Or add PR label: waive-bot-reviews
```

The script (1) on ready PRs checks CodeRabbit on HEAD — hard-fails pending/missing/error/on-demand-skip; **soft-passes** rate-limited with stderr warning (drafts skip this step); then (2) paginates GraphQL `reviewThreads`, prints unresolved bot comment URLs, and exits non-zero unless waived. Cursor rule: `.cursor/rules/pr-review-merge.mdc`.

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
