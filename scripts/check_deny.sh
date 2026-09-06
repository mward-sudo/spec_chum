#!/usr/bin/env bash
# Opt-in cargo-deny gate (licenses / advisories / bans / sources). Refs #171.
#
# Not part of `./scripts/check.sh`. CI runs the same checks in the blocking
# `cargo-deny` workflow (egui-transitive advisories ignored-with-reason in deny.toml).
set -euo pipefail
cd "$(dirname "$0")/.."

if ! command -v cargo-deny >/dev/null 2>&1 && ! cargo deny --version >/dev/null 2>&1; then
  echo "error: cargo-deny not installed. Install with:" >&2
  echo "  cargo install cargo-deny --locked" >&2
  exit 1
fi

echo "==> cargo deny check (advisories, licenses, bans, sources)"
cargo deny check
echo "==> OK"
