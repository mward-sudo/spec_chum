# Contributing

> Human contributor guide. LLM assistant policy is isolated in the
> [AI / agent-assisted work](#ai--agent-assisted-work) section below, plus
> [`AGENTS.md`](AGENTS.md) and [`.cursor/`](.cursor/). Product docs:
> [`docs/README.md`](docs/README.md).

## Workflow

- Track substantive work in GitHub Issues (milestones M0–M4); tiny local fixes need not create a standalone issue unless the user or another project policy requires tracking.
- Before substantive implementation, check related issues (`gh issue list` / `gh issue view N`). Prefer extending an existing issue over opening a duplicate; tiny self-contained fixes do not need an issue by default.
- Use stacked PRs via `gh stack` when work naturally splits into independently reviewable concerns; keep a single focused PR for a single concern.
- For features, prefer vertical slices that deliver a testable user-visible behavior through all required layers. Split broad work into small, ordered slices with explicit outcomes. Use horizontal changes when the work is inherently layer-wide or accuracy-focused (for example CPU/timing fixes, shared contract migrations, or mechanical refactors); every shape still needs checks appropriate to its behavior and scope.
- Link PRs with `Closes #N` / `Refs #N`.
- Do not close an issue until its acceptance criteria are truly met (placeholder/stub PRs must use `Refs`, not `Closes`).
- When work discovers gaps, update or reopen the issue rather than silently diverging from the tracker.

## Rust practices

- **Edition**: 2021 (workspace). Toolchain: stable with `rustfmt` + `clippy` (`rust-toolchain.toml`).
- **Lints**: shared via `[workspace.lints]` in the root `Cargo.toml`; every crate sets `lints.workspace = true`.
- **`unsafe`**: denied workspace-wide. Only allow with a narrow `#[allow(unsafe_code)]` and a `SAFETY` comment.
- **`#[allow]`**: every new allow needs a one-line rationale (prefer refactor); link an issue when temporary. See [docs/TESTING.md](docs/TESTING.md) and [#171](https://github.com/mward-sudo/spec_chum/issues/171).
- **Errors**: `thiserror` in library crates; `anyhow` is fine in `app`.
- **Formatting / Clippy**: `rustfmt.toml` and `clippy.toml` at the repo root. CI / `./scripts/check.sh` run `cargo fmt --check` and `cargo clippy --workspace --all-targets --exclude living_room -- -D warnings`.
- **API design**: prefer the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/) for public surfaces.

Local quality gate (same as CI intent):

```bash
./scripts/check_crates.sh    # while iterating — debug clippy/test for changed crates only
./scripts/check.sh           # before merge — full workspace (excl. living_room)
```

Living room / SpecChumMac Bevy gate uses **release** by default (`./scripts/check_living_room.sh`; set `SPEC_CHUM_ROOM_DEBUG=1` only if you need debug Bevy symbols).

Full **test tier matrix** (fast / living-room / slow / opt-in), lint inventory, and
provable-correctness rules: [docs/TESTING.md](docs/TESTING.md)
([#171](https://github.com/mward-sudo/spec_chum/issues/171)).

Optional native macOS SwiftUI shell compile ([#68](https://github.com/mward-sudo/spec_chum/issues/68)): CI job **`macos-shell`** on `macos-latest` runs `./scripts/build_macos_app.sh`. It is a separate job from Linux **`check`** (does not block Rust fmt/clippy/test). See [docs/MACOS_NATIVE.md](docs/MACOS_NATIVE.md).

GitHub Release binaries (macOS / Linux / Windows) are produced by tagging
`vX.Y.Z`. See [docs/RELEASE.md](docs/RELEASE.md).

## AI / agent-assisted work

> **Audience:** LLM coding assistants and humans using them. Human-only contributors
> can skip this section. Product how-tos live under [`docs/`](docs/README.md).

- Read `AGENTS.md` for crate boundaries, hard constraints, and the shared **Agentic coding loop** (scope, progress records, handoffs, and human decision points). Keep issue intent, PR implementation/review state, and active handoff notes aligned; avoid standalone progress files for short tasks.
- **graphify:** after Rust changes run `./scripts/graphify_update.sh` and commit `graphify-out/` when the graph changes; optional `./scripts/graphify_install_hooks.sh` for post-commit refresh. Skip one automatic refresh with `GRAPHIFY_SKIP_HOOK=1 git commit …`, or uninstall the hook with `./scripts/graphify_install_hooks.sh uninstall`. See `AGENTS.md` → “graphify knowledge graph”.
- Cursor project rules live in `.cursor/rules/` (always-on project policy + Rust globs), including `github-issues.mdc` for tracker sync and `pr-review-merge.mdc` for bot review gates.
- For substantive work, assistants should consult related issues so implementation follows tracked acceptance criteria. See `AGENTS.md` for the proportional task record and handoff approach.
- Assistants should be **clippy-first**: iterate with `./scripts/check_crates.sh`; run `./scripts/check.sh` for merge-ready/full-workspace confidence or when project policy requires it. Match verification to the change and report exact checks/results; do not claim checks that were not run.
- Keep PRs focused. When multiple agents work in parallel, assign non-overlapping ownership and integrate through the coordinating agent; avoid delegation for work that is faster to do directly.
- **Emulator debugging automation:** [Agent Debug HTTP API](docs/AGENT_DEBUG_API.md) is **implemented** ([#210](https://github.com/mward-sudo/spec_chum/issues/210)) — `spec_chum --serve` / `spec-chum-agent` / `spec-chum-debug --serve`; guest framebuffer as 1:1 PNG. Also see [DEBUGGING.md](docs/DEBUGGING.md) and `.cursor/skills/spec-chum-debugging/SKILL.md`.
- Do **not** edit plan files under `.cursor/plans/` (or similar).
- **Before merge / finish PR:** agent checklist in `.cursor/rules/pr-review-merge.mdc`. **CI soft-pass ≠ merge-ready.** One completed CodeRabbit review is enough; do not spend quota on redundant local + GitHub reviews by default. The existing dual-rate-limit path is an explicit exception that permits merging without a completed review only when both surfaces are rate-limited **>10m**. Detail below.

## CodeRabbit — in-editor review (Cursor plugin)

For **local / in-editor** review while iterating (staged, uncommitted, or branch diffs), use the **CodeRabbit Cursor plugin** — the preferred path in Cursor. Ask to review your changes; agents route generic review requests to CodeRabbit via the plugin skill.

This **complements** the GitHub PR workflow below. Use local review for useful pre-PR feedback or as a supplement; the ready-PR merge gate requires a GitHub review status on the current HEAD. Do not run both surfaces repeatedly by default. On-demand / label skips hard-fail — request a review.

**CLI / path scope:** [`.coderabbit.yaml`](.coderabbit.yaml) `reviews.path_filters` excludes `graphify-out/**` (checked-in knowledge graph — still committed; review-only skip). Local CLI also respects that YAML; to further scope a review away from graph artifacts, use `--dir` (e.g. `coderabbit review --agent --dir crates` or `--dir apps`). There is no separate exclude-path CLI flag.

## CodeRabbit — review when ready (usage)

Repo config: [`.coderabbit.yaml`](.coderabbit.yaml) ([auto-review docs](https://docs.coderabbit.ai/configuration/auto-review), [review commands](https://docs.coderabbit.ai/reference/review-commands)). Automatic reviews are **off** so iteration pushes do not burn allowance. `path_filters` skips lockfiles, `.cursor/plans/`, and `graphify-out/**`.

### Free OSS review limits and quota-conscious use

CodeRabbit currently gives public repositories free OSS access without a minimum star count. For public repositories with fewer than 10 stars, reviews must be triggered manually; Spec Chum currently has 0 stars (checked 2026-09-26). The OSS PR review allowance varies with project popularity (currently 1–10 PR reviews per developer per rolling hour, additionally scoped per repository); the CLI allowance is 3 reviews per developer per hour, and IDE review allowance is 1 per developer per hour. OSS PRs are subject to a 100–300 file limit per review, depending on popularity and after path exclusions. Each PR review event, including `@coderabbitai review` / `full review`, uses one PR slot; local CLI reviews use the separate CLI allowance. Batch changes and avoid duplicate surfaces. Check available capacity without starting a review with `@coderabbitai rate limit`. Check the current CodeRabbit [plan/rate-limit details](https://docs.coderabbit.ai/management/plans) and [review rate-limit guidance](https://docs.coderabbit.ai/management/rate-limits) because the exact allowance can change.

Do not authorize CodeRabbit usage-based reviews, `--use-credits`, a paid plan upgrade, or paid over-limit review without the user's explicit approval. If the included allowance is exhausted, wait for it to refill rather than spending credits. Usage-based reviews cost $0.25 per reviewed file when enabled on an eligible paid plan.

Workflow:

See `AGENTS.md` → “Jev-guided workflow” for intake routing and genuine loop-boundary calls. Jev does not review code or replace deterministic checks or the required CodeRabbit review process for a ready PR.

1. Keep the PR as a **draft** while iterating (bot-review check skips CR completeness on drafts).
2. When merge-candidate: mark **Ready for review**, then **must** request a first pass with `@coderabbitai full review` or label `coderabbit-review`. Undraft alone does not request a review and must not soft-pass the merge gate.
3. Batch fixes locally and push once when the fix set is ready. Then request one **incremental** pass with `@coderabbitai review` to cover the new HEAD. Avoid repeated review requests while fixes are still in progress; do not burn another full review unless the prior full review never completed or CodeRabbit asks for one.
4. Disposition **every** actionable finding (threads **and** outside-diff / summary nits): fix in code, reply wontfix with reason and resolve, or open a follow-up GitHub issue (preferred for deferred nits) then resolve — never leave them hanging. Then run the merge gate below.

**Rate limits do not block implementation.** Continue coding, testing, updating the draft PR, and preparing the release candidate while CodeRabbit is unavailable. The rate-limit hold applies only to merging under the ready-PR gate; use the existing dual-rate-limit exception if its conditions are met. Do not idle or repeatedly poll while there is useful work to do.

**Human trigger preferred:** CodeRabbit may ignore review commands and thread replies from other GitHub bots (`Skipped: comment is from another GitHub bot`). Prefer a human `@coderabbitai …` comment when agents’ requests are ignored. Label `coderabbit-review` remains a backup.

`auto_incremental_review` in `.coderabbit.yaml` controls **automatic** re-review on later pushes when auto-review/labels apply; it is not a prerequisite for the manual `@coderabbitai review` command (per CodeRabbit docs). Keep it `true` so label-triggered follow-ups stay incremental-friendly.

If the YAML and CodeRabbit GitHub app UI disagree, keep **Automatic Reviews** off in the app so committed config wins. Undraft alone may not trigger a review — always comment or label.

## Review check before merge

**Trap for agents:** CI gate-1 soft-pass (GitHub rate-limit >10m / unparsable) is **not** merge permission. Normally, merge needs GitHub `Review completed` on the current HEAD. The existing dual-rate-limit exception permits merge without completion only when **both** local and GitHub review surfaces are rate-limited (**>10m** / long-unknown) and a one-line PR note records it. Soft-pass ≠ on-demand/label skip. Short checklist: `.cursor/rules/pr-review-merge.mdc`.

Lesson from [#83](https://github.com/mward-sudo/spec_chum/pull/83): do not ignore CodeRabbit. For **ready** PRs, require GitHub `Review completed` for the current HEAD. If GitHub is rate-limited, only the documented dual-rate-limit exception allows merge without completion. Local review is useful feedback but cannot satisfy the script/CI GitHub-status gate. Always disposition unresolved actionable bot threads.

**Hold policy (ready / non-draft):**

- **Hard-fail gate 1:** CodeRabbit commit status pending, in progress/queued, failed/errored, **missing** (never requested / no status), or **on-demand / label skip** (`Review skipped: excluded by label configuration`, `Review skipped: on demand`, etc. — same as never requested). Soft-pass ≠ skip-without-request.
- **Soft-pass gate 1 (CI) / merge rules:** `Review rate limited` (or similar *after* a request; often green) — **not** `Review completed`. CI soft-passes when reset is **>10m** (parsed from status description such as “Next included review available in N minutes”) **or** unparsable (loud warning; prefer soft-pass over brittle false reds); **HOLDs** when reset is reliably parsed as **≤10m**. CI cannot see local CR. **Merge** requires GitHub `Review completed` on HEAD or the documented **dual** rate-limit exception: both local and GitHub are rate-limited with resets **>10m** (or long/unknown on both), documented in a **one-line PR comment**. When GitHub has no `Review completed` on the current HEAD and either side reports **≤10m** or can review now, wait/retry; do not merge on unilateral GitHub rate-limit.
- **Rate-limit merge decision (reported reset):** When GitHub has no `Review completed` on the current HEAD and **either** side can review now or reports reset **≤10 minutes**, the merge is not ready under the ordinary path. Keep implementation and verification moving; retry review when useful. Dual soft-pass merge (no completed CR) only when **both** local and GitHub are rate-limited **and** resets are **>10 minutes** (or long/unknown on both): document on the PR with a **one-line comment**, then merge. **Do not** open “Revisit CodeRabbit on PR #…” issues (practice dropped — board clutter; optional gratis CR later needs no tracker). User may already declare both sides >10m → merge immediately (e.g. [#304](https://github.com/mward-sudo/spec_chum/pull/304)). **Do not** sleep a 40–50m dual reset.
- **Hard-fail gate 2:** unresolved actionable bot review threads (unless user waives via `rmw` / `rmcw`, `--waive`, or label). GitHub `Review completed` and dual-rate-limit (>10m on both sides) paths need no waiver (`rm` / `rmc`).

**Drafts:** `./scripts/check_pr_reviews.sh` skips CodeRabbit HEAD completeness but still fails on unresolved bot threads. Do not merge drafts.

**CI:** workflow **Bot review threads** (`.github/workflows/pr-bot-reviews.yml`) runs `./scripts/check_pr_reviews.sh` on PRs, when reviews/comments arrive, and when CodeRabbit updates its **commit status** (so a prior pending/in-progress red check clears on `Review completed` / terminal rate-limited without a manual re-run). Uses the default `GITHUB_TOKEN` (`statuses: write` + `checks: write`). The ruleset required context is the API-published **commit status** **`unresolved bot threads`** (a same-named check-run is also published for Checks UI; job name is `bot-review-gate` so cancelled runs cannot block merge — [#318](https://github.com/mward-sudo/spec_chum/issues/318), [#323](https://github.com/mward-sudo/spec_chum/issues/323)). Treat a failing **published** status as blocking for merge. After resolving threads, re-run that job if GitHub did not re-trigger it. On-demand/label skips hard-fail; GitHub rate-limited soft-passes CI when reset **>10m** or unparsable, HOLDs when parsed **≤10m** (agents must still confirm GitHub `Review completed` or the dual-rate-limit exception; soft-pass ≠ skip).

**Agents / local (mandatory before merge):**

```bash
./scripts/check_pr_reviews.sh          # current branch PR
./scripts/check_pr_reviews.sh 87       # explicit PR
./scripts/check_pr_reviews.sh --self-test   # classify soft-pass / hold cases
# Explicit waiver only when the user asked for one:
./scripts/check_pr_reviews.sh 87 --waive "user waived nit: comma-only"
# Or add PR label: waive-bot-reviews
```

The script (1) on ready PRs checks CodeRabbit on HEAD — hard-fails pending/missing/error/on-demand-skip and rate-limit with parsed reset ≤10m; **soft-passes** GitHub rate-limited when reset >10m or unparsable (stderr warning that merge still needs GitHub completion or the dual-rate-limit exception; drafts skip this step; CI cannot see local CR); then (2) paginates GraphQL `reviewThreads`, prints unresolved bot comment URLs, and exits non-zero unless waived. Cursor rule: `.cursor/rules/pr-review-merge.mdc`.

## TDD

See [docs/TESTING.md](docs/TESTING.md) for the full tier matrix and gate inventory.

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
