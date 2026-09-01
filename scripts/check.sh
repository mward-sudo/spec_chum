#!/usr/bin/env bash
# Full workspace quality gate matching CI (fmt + clippy + test) — **debug** profile.
#
# While iterating, prefer scoped debug builds of only the crates you touched:
#   ./scripts/check_crates.sh              # infer from git diff
#   ./scripts/check_crates.sh z80 machine  # explicit
# Use this script before merge / when claiming a task done.
#
# Bevy living-room is excluded by default (use ./scripts/check_living_room.sh —
# release profile). Set SPEC_CHUM_CHECK_LIVING_ROOM=1 to include living_room here
# (still uses that script's release default). Do not reuse SPEC_CHUM_LIVING_ROOM —
# that boots SpecChumMac into living-room display mode.
set -euo pipefail
cd "$(dirname "$0")/.."

export RUSTFLAGS="${RUSTFLAGS:--Dwarnings}"

# Always exclude living_room from the workspace debug pass (Bevy is release-gated).
EXCLUDE_ARGS=(--exclude living_room)
RUN_LIVING_ROOM=0
if [[ "${SPEC_CHUM_CHECK_LIVING_ROOM:-}" == "1" ]]; then
  RUN_LIVING_ROOM=1
fi

echo "==> cargo fmt --check"
cargo fmt --all -- --check

echo "==> cargo clippy (debug, workspace excl. living_room)"
cargo clippy --workspace --all-targets "${EXCLUDE_ARGS[@]}" -- -D warnings

echo "==> cargo test (debug, workspace excl. living_room)"
cargo test --workspace "${EXCLUDE_ARGS[@]}"

if [[ "$RUN_LIVING_ROOM" -eq 1 ]]; then
  echo "==> living_room (via check_living_room.sh)"
  ./scripts/check_living_room.sh
fi

echo "==> OK"
