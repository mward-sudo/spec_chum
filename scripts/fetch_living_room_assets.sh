#!/usr/bin/env bash
# Fetch CC0 Poly Haven assets for the experimental living-room host (#146).
# See crates/living_room/assets/CREDITS.
set -euo pipefail
cd "$(dirname "$0")/.."

ROOT="crates/living_room/assets/polyhaven"
UA="SpecChum/0.2 (living-room asset fetch; +https://github.com/mward-sudo/spec_chum)"

download() {
  local url="$1"
  local dest="$2"
  mkdir -p "$(dirname "$dest")"
  if [[ -f "$dest" ]]; then
    echo "  skip $(basename "$dest")"
    return 0
  fi
  echo "  get $(basename "$dest")"
  local tmp="${dest}.partial"
  # Write to a temp path first so a failed curl cannot leave a partial as "valid".
  if ! curl -fsSL -A "$UA" -o "$tmp" "$url"; then
    rm -f "$tmp"
    echo "error: failed to download $url" >&2
    return 1
  fi
  mv -f "$tmp" "$dest"
}

echo "==> Poly Haven models (1k glTF)"
# television_02 — vintage CRT
download "https://dl.polyhaven.org/file/ph-assets/Models/gltf/1k/television_02/television_02_1k.gltf" \
  "$ROOT/models/television_02/television_02_1k.gltf"
download "https://dl.polyhaven.org/file/ph-assets/Models/gltf/8k/television_02/television_02.bin" \
  "$ROOT/models/television_02/television_02.bin"
download "https://dl.polyhaven.org/file/ph-assets/Models/jpg/1k/television_02/television_02_diff_1k.jpg" \
  "$ROOT/models/television_02/textures/television_02_diff_1k.jpg"
download "https://dl.polyhaven.org/file/ph-assets/Models/jpg/1k/television_02/television_02_nor_gl_1k.jpg" \
  "$ROOT/models/television_02/textures/television_02_nor_gl_1k.jpg"
download "https://dl.polyhaven.org/file/ph-assets/Models/jpg/1k/television_02/television_02_arm_1k.jpg" \
  "$ROOT/models/television_02/textures/television_02_arm_1k.jpg"

# Punch painted screen faces so the bezel is real geometry for CRT overscan (#146).
echo "==> Punch television_02 screen aperture"
python3 scripts/punch_tv_screen_aperture.py

# sofa_03 — vintage lounge sofa
download "https://dl.polyhaven.org/file/ph-assets/Models/gltf/1k/sofa_03/sofa_03_1k.gltf" \
  "$ROOT/models/sofa_03/sofa_03_1k.gltf"
download "https://dl.polyhaven.org/file/ph-assets/Models/gltf/8k/sofa_03/sofa_03.bin" \
  "$ROOT/models/sofa_03/sofa_03.bin"
download "https://dl.polyhaven.org/file/ph-assets/Models/jpg/1k/sofa_03/sofa_03_diff_1k.jpg" \
  "$ROOT/models/sofa_03/textures/sofa_03_diff_1k.jpg"
download "https://dl.polyhaven.org/file/ph-assets/Models/jpg/1k/sofa_03/sofa_03_nor_gl_1k.jpg" \
  "$ROOT/models/sofa_03/textures/sofa_03_nor_gl_1k.jpg"
download "https://dl.polyhaven.org/file/ph-assets/Models/jpg/1k/sofa_03/sofa_03_rough_1k.jpg" \
  "$ROOT/models/sofa_03/textures/sofa_03_rough_1k.jpg"

# ArmChair_01
download "https://dl.polyhaven.org/file/ph-assets/Models/gltf/1k/ArmChair_01/ArmChair_01_1k.gltf" \
  "$ROOT/models/ArmChair_01/ArmChair_01_1k.gltf"
download "https://dl.polyhaven.org/file/ph-assets/Models/gltf/4k/ArmChair_01/ArmChair_01.bin" \
  "$ROOT/models/ArmChair_01/ArmChair_01.bin"
download "https://dl.polyhaven.org/file/ph-assets/Models/jpg/1k/ArmChair_01/Armchair_01_diff_1k.jpg" \
  "$ROOT/models/ArmChair_01/textures/Armchair_01_diff_1k.jpg"
download "https://dl.polyhaven.org/file/ph-assets/Models/jpg/1k/ArmChair_01/Armchair_01_nor_gl_1k.jpg" \
  "$ROOT/models/ArmChair_01/textures/Armchair_01_nor_gl_1k.jpg"
download "https://dl.polyhaven.org/file/ph-assets/Models/jpg/1k/ArmChair_01/Armchair_01_arm_1k.jpg" \
  "$ROOT/models/ArmChair_01/textures/Armchair_01_arm_1k.jpg"

# ClassicConsole_01 — teak-ish TV stand
download "https://dl.polyhaven.org/file/ph-assets/Models/gltf/1k/ClassicConsole_01/ClassicConsole_01_1k.gltf" \
  "$ROOT/models/ClassicConsole_01/ClassicConsole_01_1k.gltf"
download "https://dl.polyhaven.org/file/ph-assets/Models/gltf/4k/ClassicConsole_01/ClassicConsole_01.bin" \
  "$ROOT/models/ClassicConsole_01/ClassicConsole_01.bin"
download "https://dl.polyhaven.org/file/ph-assets/Models/jpg/1k/ClassicConsole_01/ClassicConsole_01_diff_1k.jpg" \
  "$ROOT/models/ClassicConsole_01/textures/ClassicConsole_01_diff_1k.jpg"
download "https://dl.polyhaven.org/file/ph-assets/Models/jpg/1k/ClassicConsole_01/ClassicConsole_01_nor_gl_1k.jpg" \
  "$ROOT/models/ClassicConsole_01/textures/ClassicConsole_01_nor_gl_1k.jpg"
download "https://dl.polyhaven.org/file/ph-assets/Models/jpg/1k/ClassicConsole_01/ClassicConsole_01_arm_1k.jpg" \
  "$ROOT/models/ClassicConsole_01/textures/ClassicConsole_01_arm_1k.jpg"

# industrial_wall_sconce — brass/copper vintage wall light (80s living-room sconces)
download "https://dl.polyhaven.org/file/ph-assets/Models/gltf/1k/industrial_wall_sconce/industrial_wall_sconce_1k.gltf" \
  "$ROOT/models/industrial_wall_sconce/industrial_wall_sconce_1k.gltf"
download "https://dl.polyhaven.org/file/ph-assets/Models/gltf/4k/industrial_wall_sconce/industrial_wall_sconce.bin" \
  "$ROOT/models/industrial_wall_sconce/industrial_wall_sconce.bin"
download "https://dl.polyhaven.org/file/ph-assets/Models/jpg/1k/industrial_wall_sconce/industrial_wall_sconce_diff_1k.jpg" \
  "$ROOT/models/industrial_wall_sconce/textures/industrial_wall_sconce_diff_1k.jpg"
download "https://dl.polyhaven.org/file/ph-assets/Models/jpg/1k/industrial_wall_sconce/industrial_wall_sconce_nor_gl_1k.jpg" \
  "$ROOT/models/industrial_wall_sconce/textures/industrial_wall_sconce_nor_gl_1k.jpg"
download "https://dl.polyhaven.org/file/ph-assets/Models/jpg/1k/industrial_wall_sconce/industrial_wall_sconce_arm_1k.jpg" \
  "$ROOT/models/industrial_wall_sconce/textures/industrial_wall_sconce_arm_1k.jpg"
download "https://dl.polyhaven.org/file/ph-assets/Models/jpg/1k/industrial_wall_sconce/industrial_wall_sconce_bulb_diff_1k.jpg" \
  "$ROOT/models/industrial_wall_sconce/textures/industrial_wall_sconce_bulb_diff_1k.jpg"

# modern_wooden_cabinet — low sideboard used as 80s TV stand
download "https://dl.polyhaven.org/file/ph-assets/Models/gltf/1k/modern_wooden_cabinet/modern_wooden_cabinet_1k.gltf" \
  "$ROOT/models/modern_wooden_cabinet/modern_wooden_cabinet_1k.gltf"
download "https://dl.polyhaven.org/file/ph-assets/Models/gltf/4k/modern_wooden_cabinet/modern_wooden_cabinet.bin" \
  "$ROOT/models/modern_wooden_cabinet/modern_wooden_cabinet.bin"
download "https://dl.polyhaven.org/file/ph-assets/Models/jpg/1k/modern_wooden_cabinet/modern_wooden_cabinet_diff_1k.jpg" \
  "$ROOT/models/modern_wooden_cabinet/textures/modern_wooden_cabinet_diff_1k.jpg"
download "https://dl.polyhaven.org/file/ph-assets/Models/jpg/1k/modern_wooden_cabinet/modern_wooden_cabinet_nor_gl_1k.jpg" \
  "$ROOT/models/modern_wooden_cabinet/textures/modern_wooden_cabinet_nor_gl_1k.jpg"
download "https://dl.polyhaven.org/file/ph-assets/Models/jpg/1k/modern_wooden_cabinet/modern_wooden_cabinet_arm_1k.jpg" \
  "$ROOT/models/modern_wooden_cabinet/textures/modern_wooden_cabinet_arm_1k.jpg"

# Floor toys / 80s clutter
download "https://dl.polyhaven.org/file/ph-assets/Models/gltf/1k/rubber_duck_toy/rubber_duck_toy_1k.gltf" \
  "$ROOT/models/rubber_duck_toy/rubber_duck_toy_1k.gltf"
download "https://dl.polyhaven.org/file/ph-assets/Models/gltf/4k/rubber_duck_toy/rubber_duck_toy.bin" \
  "$ROOT/models/rubber_duck_toy/rubber_duck_toy.bin"
download "https://dl.polyhaven.org/file/ph-assets/Models/jpg/1k/rubber_duck_toy/rubber_duck_toy_diff_1k.jpg" \
  "$ROOT/models/rubber_duck_toy/textures/rubber_duck_toy_diff_1k.jpg"
download "https://dl.polyhaven.org/file/ph-assets/Models/jpg/1k/rubber_duck_toy/rubber_duck_toy_nor_gl_1k.jpg" \
  "$ROOT/models/rubber_duck_toy/textures/rubber_duck_toy_nor_gl_1k.jpg"
download "https://dl.polyhaven.org/file/ph-assets/Models/jpg/1k/rubber_duck_toy/rubber_duck_toy_arm_1k.jpg" \
  "$ROOT/models/rubber_duck_toy/textures/rubber_duck_toy_arm_1k.jpg"

download "https://dl.polyhaven.org/file/ph-assets/Models/gltf/1k/dirty_football/dirty_football_1k.gltf" \
  "$ROOT/models/dirty_football/dirty_football_1k.gltf"
download "https://dl.polyhaven.org/file/ph-assets/Models/gltf/4k/dirty_football/dirty_football.bin" \
  "$ROOT/models/dirty_football/dirty_football.bin"
download "https://dl.polyhaven.org/file/ph-assets/Models/jpg/1k/dirty_football/dirty_football_diff_1k.jpg" \
  "$ROOT/models/dirty_football/textures/dirty_football_diff_1k.jpg"
download "https://dl.polyhaven.org/file/ph-assets/Models/jpg/1k/dirty_football/dirty_football_nor_gl_1k.jpg" \
  "$ROOT/models/dirty_football/textures/dirty_football_nor_gl_1k.jpg"
download "https://dl.polyhaven.org/file/ph-assets/Models/jpg/1k/dirty_football/dirty_football_arm_1k.jpg" \
  "$ROOT/models/dirty_football/textures/dirty_football_arm_1k.jpg"

download "https://dl.polyhaven.org/file/ph-assets/Models/gltf/1k/gamepad/gamepad_1k.gltf" \
  "$ROOT/models/gamepad/gamepad_1k.gltf"
download "https://dl.polyhaven.org/file/ph-assets/Models/gltf/4k/gamepad/gamepad.bin" \
  "$ROOT/models/gamepad/gamepad.bin"
download "https://dl.polyhaven.org/file/ph-assets/Models/jpg/1k/gamepad/gamepad_diff_1k.jpg" \
  "$ROOT/models/gamepad/textures/gamepad_diff_1k.jpg"
download "https://dl.polyhaven.org/file/ph-assets/Models/jpg/1k/gamepad/gamepad_nor_gl_1k.jpg" \
  "$ROOT/models/gamepad/textures/gamepad_nor_gl_1k.jpg"
download "https://dl.polyhaven.org/file/ph-assets/Models/jpg/1k/gamepad/gamepad_arm_1k.jpg" \
  "$ROOT/models/gamepad/textures/gamepad_arm_1k.jpg"

download "https://dl.polyhaven.org/file/ph-assets/Models/gltf/1k/portable_cassette_player/portable_cassette_player_1k.gltf" \
  "$ROOT/models/portable_cassette_player/portable_cassette_player_1k.gltf"
download "https://dl.polyhaven.org/file/ph-assets/Models/gltf/4k/portable_cassette_player/portable_cassette_player.bin" \
  "$ROOT/models/portable_cassette_player/portable_cassette_player.bin"
download "https://dl.polyhaven.org/file/ph-assets/Models/jpg/1k/portable_cassette_player/portable_cassette_player_diff_1k.jpg" \
  "$ROOT/models/portable_cassette_player/textures/portable_cassette_player_diff_1k.jpg"
download "https://dl.polyhaven.org/file/ph-assets/Models/jpg/1k/portable_cassette_player/portable_cassette_player_nor_gl_1k.jpg" \
  "$ROOT/models/portable_cassette_player/textures/portable_cassette_player_nor_gl_1k.jpg"
download "https://dl.polyhaven.org/file/ph-assets/Models/jpg/1k/portable_cassette_player/portable_cassette_player_arm_1k.jpg" \
  "$ROOT/models/portable_cassette_player/textures/portable_cassette_player_arm_1k.jpg"

echo "==> Poly Haven PBR textures (1k JPG)"
for name in dirty_carpet floral_jacquard american_walnut_veneer beige_wall_001 velour_velvet; do
  for map in diff nor_gl rough arm; do
    download \
      "https://dl.polyhaven.org/file/ph-assets/Textures/jpg/1k/${name}/${name}_${map}_1k.jpg" \
      "$ROOT/textures/${name}/${name}_${map}_1k.jpg"
  done
done

echo "==> OK — assets under $ROOT"
du -sh "$ROOT"/* 2>/dev/null || true
