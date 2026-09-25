#!/usr/bin/env bash
# Generate crates/living_room/assets/skein/living_room_edit.blend (+ .gltf)
# matching the procedural living-room layout (room.rs / glow.rs).
#
# Outputs are gitignored (see .gitignore). Requires Blender on PATH or
# /Applications/Blender.app on macOS.
set -euo pipefail
cd "$(dirname "$0")/.."

BLEND_PY="scripts/blender/generate_living_room_edit.py"
TAG_PY="scripts/blender/tag_living_room_skein_gltf.py"
OUT_DIR="crates/living_room/assets/skein"
BLEND_OUT="${OUT_DIR}/living_room_edit.blend"
GLTF_OUT="${OUT_DIR}/living_room_edit.gltf"
ASSETS="crates/living_room/assets"

find_blender() {
  if [[ -n "${BLENDER:-}" && -x "${BLENDER}" ]]; then
    echo "${BLENDER}"
    return 0
  fi
  if command -v blender >/dev/null 2>&1; then
    command -v blender
    return 0
  fi
  local mac="/Applications/Blender.app/Contents/MacOS/Blender"
  if [[ -x "${mac}" ]]; then
    echo "${mac}"
    return 0
  fi
  return 1
}

if [[ ! -d "${ASSETS}/polyhaven/models" ]]; then
  echo "error: Poly Haven assets missing — run ./scripts/fetch_living_room_assets.sh" >&2
  exit 1
fi

if [[ ! -f "${ASSETS}/polyhaven/models/television_02/television_02_aperture.gltf" ]]; then
  echo "error: television_02_aperture.gltf missing — run ./scripts/fetch_living_room_assets.sh" >&2
  exit 1
fi

if ! BLENDER_BIN="$(find_blender)"; then
  echo "error: Blender not found on PATH (or /Applications/Blender.app)." >&2
  echo "Install Blender 4.2+, or set BLENDER=/path/to/Blender, then re-run:" >&2
  echo "  ./scripts/generate_living_room_blend.sh" >&2
  exit 1
fi

mkdir -p "${OUT_DIR}"

echo "==> Blender: ${BLENDER_BIN}"
echo "==> Writing ${BLEND_OUT} (+ glTF)"
"${BLENDER_BIN}" --background --factory-startup --python "${BLEND_PY}" -- \
  --assets "$(pwd)/${ASSETS}" \
  --blend "$(pwd)/${BLEND_OUT}" \
  --gltf "$(pwd)/${GLTF_OUT}"

echo "==> Injecting BEVY_skein tags into ${GLTF_OUT}"
python3 "${TAG_PY}" "${GLTF_OUT}"

echo ""
echo "Done."
echo "  Blend: ${BLEND_OUT}"
echo "  glTF:  ${GLTF_OUT}"
echo ""
echo "Open the .blend in Blender to edit. After changes, export glTF to the"
echo "same path (extras / BEVY_skein on), or re-run this script to regenerate."
echo "Run: cargo run -p living_room --release --features skein"
echo "Docs: docs/LIVING_ROOM.md (Scene editing with Skein)"
