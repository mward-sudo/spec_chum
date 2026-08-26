#!/usr/bin/env bash
# Local / agent quality gate matching CI (fmt + clippy + test).
set -euo pipefail
cd "$(dirname "$0")/.."

export RUSTFLAGS="${RUSTFLAGS:--Dwarnings}"

# Bevy living-room host is opt-in: heavy to compile and not part of the default gate.
# Set SPEC_CHUM_CHECK_LIVING_ROOM=1 to include it (or use ./scripts/check_living_room.sh).
# Do not reuse SPEC_CHUM_LIVING_ROOM — that boots SpecChumMac into living-room display mode.
EXCLUDE_ARGS=()
if [[ "${SPEC_CHUM_CHECK_LIVING_ROOM:-}" != "1" ]]; then
  EXCLUDE_ARGS=(--exclude living_room)
fi

echo "==> cargo fmt --check"
cargo fmt --all -- --check

echo "==> cargo clippy"
cargo clippy --workspace --all-targets "${EXCLUDE_ARGS[@]}" -- -D warnings

echo "==> cargo test"
cargo test --workspace "${EXCLUDE_ARGS[@]}"

echo "==> OK"
