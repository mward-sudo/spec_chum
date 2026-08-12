#!/usr/bin/env bash
# Fetch ZX Spectrum system ROMs from spectrumforeveryone/zx-roms.
# ROMs are NOT redistributed by this project.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ROM_DIR="${ROOT}/roms"
# Pinned for reproducibility (update deliberately when refreshing ROMs).
ZX_ROMS_REF="${ZX_ROMS_REF:-720ee29bc699393275d799d0d5adc4b31210b127}"
REPO_URL="https://github.com/spectrumforeveryone/zx-roms.git"

mkdir -p "$ROM_DIR"
echo "Fetching zx-roms @ ${ZX_ROMS_REF}"

CACHE="${ROOT}/.rom-cache"
mkdir -p "$CACHE"
if [[ ! -d "$CACHE/zx-roms/.git" ]]; then
  git clone --filter=blob:none --sparse "$REPO_URL" "$CACHE/zx-roms"
fi
(
  cd "$CACHE/zx-roms"
  git fetch --depth 1 origin "$ZX_ROMS_REF" 2>/dev/null || git fetch origin
  git checkout -q "$ZX_ROMS_REF" 2>/dev/null || {
    git fetch --depth 1 origin master || git fetch --depth 1 origin main
    git checkout -q "$ZX_ROMS_REF"
  }
  git sparse-checkout set spectrum16-48 spectrum128-plus2 spectrum-plus3
)

copy_rom() {
  local src="$1" dest="$2"
  if [[ ! -f "$src" ]]; then
    echo "warn: missing $src" >&2
    return 1
  fi
  cp -f "$src" "$dest"
  echo "  ok $(basename "$dest") ($(wc -c < "$dest" | tr -d ' ') bytes)"
}

echo "Installing ROMs into ${ROM_DIR}"
copy_rom "$CACHE/zx-roms/spectrum16-48/spec48.rom" "$ROM_DIR/spec48.rom"

if [[ -d "$CACHE/zx-roms/spectrum128-plus2/128" ]]; then
  mkdir -p "$ROM_DIR/128"
  while IFS= read -r f; do
    copy_rom "$f" "$ROM_DIR/128/$(basename "$f")" || true
  done < <(find "$CACHE/zx-roms/spectrum128-plus2/128" -name '*.rom' -maxdepth 1)
fi
if [[ -d "$CACHE/zx-roms/spectrum128-plus2/plus2" ]]; then
  mkdir -p "$ROM_DIR/plus2"
  while IFS= read -r f; do
    copy_rom "$f" "$ROM_DIR/plus2/$(basename "$f")" || true
  done < <(find "$CACHE/zx-roms/spectrum128-plus2/plus2" -name '*.rom' -maxdepth 1)
fi

echo "$ZX_ROMS_REF" > "$ROM_DIR/.zx-roms-ref"
echo "Done. ROM dir: $ROM_DIR"
