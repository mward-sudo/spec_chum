#!/usr/bin/env bash
# A/B matrix for living-room quality knobs (issue #146 / Bevy perf recommendations).
# Soft budget: prints FAIL lines but does not abort the matrix.
set -euo pipefail
cd "$(dirname "$0")/.."

OUT="${1:-tmp-perf-capture/room-quality-matrix.txt}"
mkdir -p "$(dirname "$OUT")"
: >"$OUT"

export SPEC_CHUM_ROOM_PERF_SOFT=1
export RUST_LOG="${RUST_LOG:-error}"

echo "==> building room_perf (release)"
cargo build -p living_room --example room_perf --release

run_one() {
  local label="$1"
  shift
  echo "" | tee -a "$OUT"
  echo "=== $label ===" | tee -a "$OUT"
  env "$@" ./target/release/examples/room_perf 960 540 2>&1 \
    | tee -a "$OUT" \
    | rg -e 'quality:|tick-only|FAIL:|WARN:|OK:' || true
}

# Baseline look (defaults after restore).
run_one "default (bloom on, msaa2, lights full)" \
  SPEC_CHUM_ROOM_BLOOM=1 SPEC_CHUM_ROOM_MSAA=2 SPEC_CHUM_ROOM_LIGHTS=full

# Recommendation: kill bloom.
run_one "no bloom" \
  SPEC_CHUM_ROOM_BLOOM=0 SPEC_CHUM_ROOM_MSAA=2 SPEC_CHUM_ROOM_LIGHTS=full

# Cheaper bloom mips.
run_one "bloom mips=256" \
  SPEC_CHUM_ROOM_BLOOM=1 SPEC_CHUM_ROOM_BLOOM_MIPS=256 SPEC_CHUM_ROOM_MSAA=2 SPEC_CHUM_ROOM_LIGHTS=full

# MSAA ladder.
run_one "msaa off" \
  SPEC_CHUM_ROOM_BLOOM=1 SPEC_CHUM_ROOM_MSAA=0 SPEC_CHUM_ROOM_LIGHTS=full
run_one "msaa 4" \
  SPEC_CHUM_ROOM_BLOOM=1 SPEC_CHUM_ROOM_MSAA=4 SPEC_CHUM_ROOM_LIGHTS=full

# Light count.
run_one "lights min" \
  SPEC_CHUM_ROOM_BLOOM=1 SPEC_CHUM_ROOM_MSAA=2 SPEC_CHUM_ROOM_LIGHTS=min

# Stacked "perf" profile (what we briefly shipped).
run_one "stacked perf (no bloom, msaa2, lights min)" \
  SPEC_CHUM_ROOM_BLOOM=0 SPEC_CHUM_ROOM_MSAA=2 SPEC_CHUM_ROOM_LIGHTS=min

# Aggressive stack for 120 Hz chase.
run_one "aggressive (no bloom, msaa0, lights min)" \
  SPEC_CHUM_ROOM_BLOOM=0 SPEC_CHUM_ROOM_MSAA=0 SPEC_CHUM_ROOM_LIGHTS=min

echo "" | tee -a "$OUT"
echo "Wrote $OUT" | tee -a "$OUT"
