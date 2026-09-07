#!/usr/bin/env bash
# Copy living_room Bevy assets into a staged SpecChumMac.app Resources tree.
# Usage: stage_living_room_assets.sh <repo-root> <dest-living_room_assets-dir>
#
# Incomplete Poly Haven trees hard-fail (#368) unless
# SPEC_CHUM_ALLOW_EMPTY_LIVING_ROOM_ASSETS=1. Prefer ensure_living_room_assets.sh
# (auto-fetch) before calling this from interactive build/run scripts.
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <repo-root> <dest-living_room_assets-dir>" >&2
  exit 2
fi

ROOT="$1"
DEST="$2"
SRC="$ROOT/crates/living_room/assets"

if [[ ! -d "$SRC" ]]; then
  if [[ "${SPEC_CHUM_ALLOW_EMPTY_LIVING_ROOM_ASSETS:-}" == "1" ]]; then
    echo "warning: missing $SRC — empty living_room_assets allowed" >&2
    mkdir -p "$DEST"
    exit 0
  fi
  echo "error: missing $SRC (run ./scripts/fetch_living_room_assets.sh)" >&2
  exit 1
fi

"$ROOT/scripts/check_living_room_assets.sh" "$ROOT"

mkdir -p "$(dirname "$DEST")"
rm -rf "$DEST"
if command -v rsync >/dev/null 2>&1; then
  rsync -a --delete "$SRC/" "$DEST/"
else
  mkdir -p "$DEST"
  cp -R "$SRC/." "$DEST/"
fi
