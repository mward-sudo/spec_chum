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
- Link a PR with `Closes #N` only after checking the implementation against every current acceptance criterion and recording criterion-by-criterion evidence in the PR or issue. The implementation being merged must satisfy every criterion; partial or unverified work uses `Refs #N` and leaves the issue open. For the full closure gate, including user-facing verification, see [`.cursor/rules/github-issues.mdc`](.cursor/rules/github-issues.mdc).
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

Coding assistants: read [`AGENTS.md`](AGENTS.md) and applicable [`.cursor/rules/`](.cursor/rules/) for shared workflow, project constraints, issue tracking, Graphify, and PR gates. Topic guides are indexed in [`docs/README.md`](docs/README.md). Human-only contributors can skip this section.

## CodeRabbit — in-editor review (Cursor plugin)

For **local / in-editor** review while iterating (staged, uncommitted, or branch diffs), use the **CodeRabbit Cursor plugin** — the preferred path in Cursor. Ask to review your changes; agents route generic review requests to CodeRabbit via the plugin skill.

This **complements** the GitHub PR workflow below. A local review can serve as the CodeRabbit review when GitHub is rate-limited for more than 10 minutes. GitHub review status remains preferred when it can complete within that window. Do not run both surfaces repeatedly by default. On-demand / label skips are not rate limits; request a review through the other surface.

**CLI / path scope:** [`.coderabbit.yaml`](.coderabbit.yaml) `reviews.path_filters` excludes `graphify-out/**` (checked-in knowledge graph — still committed; review-only skip). Local CLI also respects that YAML; to further scope a review away from graph artifacts, use `--dir` (e.g. `coderabbit review --agent --dir crates` or `--dir apps`). There is no separate exclude-path CLI flag.

## CodeRabbit — review when ready (usage)

Repo config: [`.coderabbit.yaml`](.coderabbit.yaml) ([auto-review docs](https://docs.coderabbit.ai/configuration/auto-review), [review commands](https://docs.coderabbit.ai/reference/review-commands)). Automatic reviews are **off** so iteration pushes do not burn allowance. `path_filters` skips lockfiles, `.cursor/plans/`, and `graphify-out/**`.

Review limits change; check CodeRabbit's [plan](https://docs.coderabbit.ai/management/plans) and [rate-limit guidance](https://docs.coderabbit.ai/management/rate-limits) when needed. Do not enable paid usage-based reviews or upgrade without the user's explicit approval.

1. Keep the PR as a **draft** while iterating (bot-review check skips CR completeness on drafts).
2. When merge-candidate: mark **Ready for review** and request a first pass with `@coderabbitai full review` or label `coderabbit-review`. If GitHub reports a rate limit beyond 10 minutes, try local CodeRabbit CLI / IDE review. Once a review is pending or in progress, let it complete regardless of elapsed time. Undraft alone does not request a review; on-demand / label skips are not rate limits.
3. Batch fixes locally and push once when the fix set is ready. Then request one **incremental** pass with `@coderabbitai review` to cover the new HEAD. Avoid repeated review requests while fixes are still in progress; do not burn another full review unless the prior full review never completed or CodeRabbit asks for one.
4. Disposition **every** actionable finding (threads **and** outside-diff / summary nits): fix in code, reply wontfix with reason and resolve, or open a follow-up GitHub issue (preferred for deferred nits) then resolve — never leave them hanging. Then run the merge gate below.

**Rate limits do not block implementation.** Continue work while CodeRabbit is unavailable. Apply the 10-minute rate-limit fallback below; a pending review is not rate-limited and must complete.

**Human trigger preferred:** CodeRabbit may ignore review commands and thread replies from other GitHub bots (`Skipped: comment is from another GitHub bot`). Prefer a human `@coderabbitai …` comment when agents’ requests are ignored. Label `coderabbit-review` remains a backup.

`auto_incremental_review` in `.coderabbit.yaml` controls **automatic** re-review on later pushes when auto-review/labels apply; it is not a prerequisite for the manual `@coderabbitai review` command (per CodeRabbit docs). Keep it `true` so label-triggered follow-ups stay incremental-friendly.

If the YAML and CodeRabbit GitHub app UI disagree, keep **Automatic Reviews** off in the app so committed config wins. Undraft alone may not trigger a review — always comment or label.

## Review check before merge

For a ready PR, request one CodeRabbit review on the current HEAD and disposition all actionable findings. A completed local review supports the documented fallback when GitHub is rate-limited beyond 10 minutes; it does **not** change GitHub status. Wait for pending reviews regardless of duration. On-demand skips are not rate limits.

**Hold policy (ready / non-draft):**

- **Hard-fail gate 1:** CodeRabbit commit status pending, in progress/queued, failed/errored, **missing** (never requested / no status), or **on-demand / label skip** (`Review skipped: excluded by label configuration`, `Review skipped: on demand`, etc. — same as never requested). Soft-pass ≠ skip-without-request.
- **Rate limit:** reset **≤10m** means wait and retry; **>10m** means try another review surface. If all available surfaces remain rate-limited beyond 10m, main-agent diff review plus a one-line PR note is sufficient. An unparsable reset soft-passes CI but still requires main-agent review and a note. A local review does not satisfy GitHub status; CI cannot see it.
- **Hard-fail gate 2:** unresolved actionable bot review threads (unless user waives via `rmw` / `rmcw`, `--waive`, or label). Always disposition actionable findings from any completed review. The review timing rule does not waive unresolved threads.

**Drafts:** `./scripts/check_pr_reviews.sh` skips CodeRabbit HEAD completeness but still fails on unresolved bot threads. Do not merge drafts.

**CI:** workflow **Bot review threads** (`.github/workflows/pr-bot-reviews.yml`) runs `./scripts/check_pr_reviews.sh`. The required published commit status is `unresolved bot threads`; a failing status blocks merge. Re-run the job after resolving threads if GitHub did not trigger it. CI cannot see local review evidence.

**Agents / local (mandatory before merge):**

```bash
./scripts/check_pr_reviews.sh          # current branch PR
./scripts/check_pr_reviews.sh 87       # explicit PR
./scripts/check_pr_reviews.sh --self-test   # classify soft-pass / hold cases
# Explicit waiver only when the user asked for one:
./scripts/check_pr_reviews.sh 87 --waive "user waived nit: comma-only"
# Or add PR label: waive-bot-reviews
```

The script checks ready-PR review status, then unresolved bot `reviewThreads`; it exits non-zero unless waived. Drafts skip review-status completeness but still fail on unresolved threads. Agent checklist: `.cursor/rules/pr-review-merge.mdc`.

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
