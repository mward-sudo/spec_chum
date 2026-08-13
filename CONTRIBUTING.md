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

## AI / agent-assisted work

- Read `AGENTS.md` for crate boundaries and hard constraints.
- Cursor project rules live in `.cursor/rules/` (always-on project policy + Rust globs), including `github-issues.mdc` for tracker sync and `pr-review-merge.mdc` for bot review gates.
- Before coding, agents should consult open issues so implementations do not drift from tracked acceptance criteria.
- Agents should be **clippy-first**: run `./scripts/check.sh` before claiming done; do not “promise” clean code without running the gate.
- Keep PRs small and crate-scoped so parallel agents do not clobber each other.
- Do **not** edit plan files under `.cursor/plans/` (or similar).
- **Before merge:** check `gh pr view` / review threads. Do not merge with unresolved **actionable** CodeRabbit (or similar bot) comments unless the user explicitly waives them; fix or reply with a wontfix reason, then resolve threads.

## Lightweight review check (optional)

No required CI status for CodeRabbit. Before merging, a quick manual check is enough:

```bash
gh pr view <N> --comments
# or: unresolved threads via gh api graphql (reviewThreads { isResolved })
```

## TDD

- Z80: Fuse `tests.in` / `tests.expected` before merging opcode groups.
- Contention / floating bus: table-driven unit tests required.
- Integration tests that need ROMs must skip cleanly when `roms/` is missing.
- **z80test** (Patrik Rak): `cargo test -p machine --features slow-tests --release z80doc_all_tests_passed -- --nocapture`.
  For `z80full`: `./scripts/fetch_z80test.sh` then `cargo test -p machine --features slow-tests --release z80full -- --nocapture --ignored`.
- **GUI**: logic lives in `app` as a library. Headless tests use `EmulatorSession` (no display) plus an egui `Context::run` smoke test — no xvfb required for CI.

## Stack commands (non-interactive)

```bash
gh stack init <branch>
gh stack add <branch>
gh stack submit --auto --open
gh stack view --json
gh stack sync --prune
```
