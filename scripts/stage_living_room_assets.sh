#!/usr/bin/env bash
# Copy living_room Bevy assets into a staged SpecChumMac.app Resources tree.
# Usage: stage_living_room_assets.sh <repo-root> <dest-living_room_assets-dir>
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <repo-root> <dest-living_room_assets-dir>" >&2
  exit 2
fi

ROOT="$1"
DEST="$2"
SRC="$ROOT/crates/living_room/assets"

if [[ ! -d "$SRC" ]]; then
  echo "warning: missing $SRC (run ./scripts/fetch_living_room_assets.sh)" >&2
  exit 0
fi

mkdir -p "$(dirname "$DEST")"
rm -rf "$DEST"
if command -v rsync >/dev/null 2>&1; then
  rsync -a --delete "$SRC/" "$DEST/"
else
  mkdir -p "$DEST"
  cp -R "$SRC/." "$DEST/"
fi
