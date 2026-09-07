#!/usr/bin/env bash
# Verify Poly Haven living-room assets against the checked-in manifest (#368).
# Usage: check_living_room_assets.sh [repo-root]
# Exit 0 when complete; exit 1 when incomplete (unless ALLOW_EMPTY is set).
set -euo pipefail

ROOT="${1:-$(cd "$(dirname "$0")/.." && pwd)}"
ASSETS="$ROOT/crates/living_room/assets"
POLY="$ASSETS/polyhaven"
MANIFEST="$ASSETS/polyhaven.manifest"

if [[ "${SPEC_CHUM_ALLOW_EMPTY_LIVING_ROOM_ASSETS:-}" == "1" ]]; then
  echo "warning: SPEC_CHUM_ALLOW_EMPTY_LIVING_ROOM_ASSETS=1 — skipping Poly Haven check" >&2
  exit 0
fi

if [[ ! -f "$MANIFEST" ]]; then
  echo "error: missing manifest $MANIFEST" >&2
  exit 1
fi

missing=()
while IFS= read -r line || [[ -n "$line" ]]; do
  # Strip comments / blanks
  line="${line%%#*}"
  line="${line%"${line##*[![:space:]]}"}"
  line="${line#"${line%%[![:space:]]*}"}"
  [[ -z "$line" ]] && continue
  if [[ ! -f "$POLY/$line" ]]; then
    missing+=("$line")
  fi
done <"$MANIFEST"

if [[ ${#missing[@]} -eq 0 ]]; then
  exit 0
fi

echo "error: living-room Poly Haven assets incomplete under $POLY" >&2
echo "  missing ${#missing[@]} path(s) from polyhaven.manifest (showing up to 8):" >&2
for p in "${missing[@]:0:8}"; do
  echo "    - $p" >&2
done
echo "  run: ./scripts/fetch_living_room_assets.sh" >&2
echo "  or set SPEC_CHUM_ALLOW_EMPTY_LIVING_ROOM_ASSETS=1 for intentional empty stage" >&2
exit 1
