#!/usr/bin/env bash
# Bake #149's static room atlas and UV1 glTF. Output is staged with SpecChumMac.
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ ! -d crates/living_room/assets/polyhaven/models ]]; then
  echo "error: Poly Haven assets missing — run ./scripts/fetch_living_room_assets.sh" >&2
  exit 1
fi
./scripts/check_living_room_assets.sh "$(pwd)"

if [[ -n "${BLENDER:-}" && -x "${BLENDER}" ]]; then
  BLENDER_BIN="${BLENDER}"
elif command -v blender >/dev/null 2>&1; then
  BLENDER_BIN="$(command -v blender)"
elif [[ -x /Applications/Blender.app/Contents/MacOS/Blender ]]; then
  BLENDER_BIN=/Applications/Blender.app/Contents/MacOS/Blender
else
  echo "error: Blender 4.2+ is required (or set BLENDER=/path/to/Blender)" >&2
  exit 1
fi

"${BLENDER_BIN}" --background --factory-startup --python-exit-code 1 \
  --python scripts/blender/bake_living_room_lightmaps.py -- \
  --assets "$(pwd)/crates/living_room/assets" \
  --output "$(pwd)/crates/living_room/assets/lightmaps" \
  --size "${SPEC_CHUM_LIGHTMAP_SIZE:-2048}" \
  --samples "${SPEC_CHUM_LIGHTMAP_SAMPLES:-32}" \
  --threads "${SPEC_CHUM_LIGHTMAP_THREADS:-4}"
