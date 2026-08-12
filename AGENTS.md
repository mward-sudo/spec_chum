# Spec Chum — agent notes

Cross-tool project facts for LLM-assisted work. Cursor-specific rules live in `.cursor/rules/`.

## What this is

From-scratch ZX Spectrum emulator in Rust + egui. Cycle-accurate Z80 and ULA timing are first-class goals. System ROMs are **not** in git — use `./scripts/fetch_roms.sh`.

## Crate map

| Crate | Role |
| --- | --- |
| `z80` | CPU core (own implementation) |
| `bus` | Memory / I/O interconnect |
| `ula` | Video, contention, floating bus, border |
| `tape` | Tape loading |
| `formats` | Snapshot / tape file formats |
| `machine` | Wired Spectrum models |
| `app` | egui / eframe frontend binary |

Prefer changes inside the owning crate. Cross-crate refactors need a clear reason.

## Hard constraints

- Do **not** edit plan files under `.cursor/plans/` (or similar).
- Do **not** commit ROM binaries (`roms/`, `*.rom`).
- Library crates: `thiserror` for public errors; no bare `unwrap` in non-test code.
- Binary (`app`): `anyhow` is fine for top-level error context.
- `unsafe` is denied workspace-wide; only introduce it with a documented `SAFETY` rationale and a narrowly scoped `#[allow(unsafe_code)]`.

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
- **Open [#17](https://github.com/mward-sudo/spec_chum/issues/17):** `crates/machine/src/z80test.rs` is a stub — real Patrik Rak z80test TAP runner (z80doc then z80full) required before closing. See `.cursor/rules/z80test-issue-17.mdc`.

## PR / stack

Track work in GitHub Issues (milestones M0–M4). Prefer small PRs (one concern). See `CONTRIBUTING.md` for `gh stack` commands.

### Stay in sync with issues

Before implementing: `gh issue list` / `gh issue view N` for related work. Prefer extending existing issues over duplicates. Link PRs with `Closes #N` / `Refs #N`. Do not close until acceptance criteria are truly met (see [#17](https://github.com/mward-sudo/spec_chum/issues/17) z80test). When work discovers gaps, update or reopen the issue rather than silently diverging. Cursor rule: `.cursor/rules/github-issues.mdc`.
