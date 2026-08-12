# Contributing

## Workflow

- Track work in GitHub Issues (milestones M0–M4).
- Use stacked PRs via `gh stack` (one concern per branch).
- Link PRs with `Closes #N` / `Refs #N`.

## TDD

- Z80: Fuse `tests.in` / `tests.expected` before merging opcode groups.
- Contention / floating bus: table-driven unit tests required.
- Integration tests that need ROMs must skip cleanly when `roms/` is missing.

## Stack commands (non-interactive)

```bash
gh stack init <branch>
gh stack add <branch>
gh stack submit --auto --open
gh stack view --json
gh stack sync --prune
```
