#!/usr/bin/env bash
# Optional gate for the experimental Bevy living-room host (#146).
set -euo pipefail
cd "$(dirname "$0")/.."

export RUSTFLAGS="${RUSTFLAGS:--Dwarnings}"

# Virtualized CI runners often miss the hard 60 Hz budget; soft-fail there only.
# Local runs stay hard-fail unless the caller sets SPEC_CHUM_ROOM_PERF_SOFT=1.
if [[ "${CI:-}" == "true" ]]; then
  export SPEC_CHUM_ROOM_PERF_SOFT=1
fi

echo "==> living_room fmt"
cargo fmt -p living_room -- --check

echo "==> living_room clippy"
cargo clippy -p living_room --all-targets -- -D warnings

echo "==> living_room test"
cargo test -p living_room

echo "==> living_room headless perf (release, 50 Hz budget)"
cargo run -p living_room --example room_perf --release

echo "==> OK"
