#!/usr/bin/env bash
# Ensure Poly Haven assets are present before staging SpecChumMac (#368).
# Default: auto-fetch when incomplete (SPEC_CHUM_FETCH_LIVING_ROOM_ASSETS=1).
# Set SPEC_CHUM_FETCH_LIVING_ROOM_ASSETS=0 to require a prior fetch / fail closed.
# SPEC_CHUM_ALLOW_EMPTY_LIVING_ROOM_ASSETS=1 skips the check entirely.
set -euo pipefail

ROOT="${1:-$(cd "$(dirname "$0")/.." && pwd)}"
CHECK="$ROOT/scripts/check_living_room_assets.sh"

if "$CHECK" "$ROOT"; then
  exit 0
fi

# check failed (or printed allow-empty warning with exit 0 — already handled)
fetch_default=1
if [[ "${SPEC_CHUM_FETCH_LIVING_ROOM_ASSETS:-$fetch_default}" != "1" ]]; then
  exit 1
fi

echo "==> Poly Haven assets incomplete — running fetch_living_room_assets.sh"
"$ROOT/scripts/fetch_living_room_assets.sh"
"$CHECK" "$ROOT"
