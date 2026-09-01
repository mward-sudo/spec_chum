#!/usr/bin/env bash
# Optional gate for the Bevy living-room host (#146).
#
# Default profile is **release** — Bevy debug artifacts are multi‑GB and not
# required for SpecChumMac (which always links `target/release/libspec_chum_room.a`).
# Set SPEC_CHUM_ROOM_DEBUG=1 to force debug clippy/test (disk-heavy; avoid unless
# you need debug symbols for living_room specifically).
set -euo pipefail
cd "$(dirname "$0")/.."

export RUSTFLAGS="${RUSTFLAGS:--Dwarnings}"

# Virtualized CI runners often miss the hard 60 Hz budget; soft-fail there only.
# Local runs stay hard-fail unless the caller sets SPEC_CHUM_ROOM_PERF_SOFT=1.
if [[ "${CI:-}" == "true" ]]; then
  export SPEC_CHUM_ROOM_PERF_SOFT=1
fi

PROFILE_ARGS=(--release)
PROFILE_LABEL="release"
if [[ "${SPEC_CHUM_ROOM_DEBUG:-}" == "1" ]]; then
  PROFILE_ARGS=()
  PROFILE_LABEL="debug"
  echo "==> WARNING: SPEC_CHUM_ROOM_DEBUG=1 — Bevy debug build (large disk use)"
fi

echo "==> living_room fmt"
cargo fmt -p living_room -- --check

echo "==> living_room clippy (${PROFILE_LABEL})"
cargo clippy -p living_room --all-targets "${PROFILE_ARGS[@]}" -- -D warnings

echo "==> living_room test (${PROFILE_LABEL})"
cargo test -p living_room "${PROFILE_ARGS[@]}"

echo "==> living_room headless perf (release, 50 Hz budget)"
cargo run -p living_room --example room_perf --release

echo "==> OK"
