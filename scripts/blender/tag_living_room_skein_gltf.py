#!/usr/bin/env python3
"""Inject BEVY_skein component extensions into living_room_edit.gltf.

Matches object names produced by generate_living_room_edit.py so a freshly
generated export can replace the procedural room under `--features skein`
without a one-time Blender Fetch Registry pass.

Type paths use the living_room lib crate name (`spec_chum_room`).
Point lights come from KHR_lights_punctual; this script only adds marker /
GlowDriven components so intensities are not doubled. Re-tag in Blender after
Fetch Registry if Reflect paths ever diverge.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

# Crate lib name is spec_chum_room (Cargo.toml [lib] name).
TV = "spec_chum_room::room::TelevisionCabinet"
LIVE = "spec_chum_room::hybrid::LiveTv"
STATIC = "spec_chum_room::hybrid::RoomStatic"
CRT_FILL = "spec_chum_room::glow::CrtFillLight"
GLOW = "spec_chum_room::glow::GlowDriven"
DYN = "spec_chum_room::scene_variant::DynamicRoomFillLight"

# Exact empty / light names from the generator.
TAG_MAP: dict[str, list[dict[str, Any]]] = {
    "television_02": [
        {TV: None},
        {LIVE: None},
    ],
    "tv_stand": [{LIVE: None}],
    "sofa": [{STATIC: None}],
    "armchair": [{STATIC: None}],
    "armchair.001": [{STATIC: None}],
    "wall_sconce_tv_left": [{STATIC: None}],
    "wall_sconce_tv_centre": [{STATIC: None}],
    "wall_sconce_tv_right": [{STATIC: None}],
    "wall_sconce_left": [{STATIC: None}],
    "wall_sconce_right": [{STATIC: None}],
    "toy_football": [{STATIC: None}],
    "toy_rubber_duck": [{STATIC: None}],
    "toy_gamepad": [{STATIC: None}],
    "toy_walkman": [{STATIC: None}],
    "spectrum_joystick": [{STATIC: None}],
    "carpet": [{STATIC: None}],
    "ceiling": [{STATIC: None}],
    "wall_back": [{STATIC: None}],
    "wall_front": [{STATIC: None}],
    "wall_left": [{STATIC: None}],
    "wall_right": [{STATIC: None}],
    "curtain": [{STATIC: None}],
    "curtain.001": [{STATIC: None}],
    "skirting": [{STATIC: None}],
}

# Marker tags only — Bevy PointLight comes from KHR_lights_punctual.
LIGHT_TAGS: dict[str, list[dict[str, Any]]] = {
    "crt_fill_light": [
        {CRT_FILL: None},
        {GLOW: {"intensity_scale": 1.0}},
    ],
    "crt_wall_bounce": [
        {GLOW: {"intensity_scale": 0.45}},
        {DYN: None},
    ],
    "wall_sconce_tv_left_bulb": [{DYN: None}],
    "wall_sconce_tv_centre_bulb": [{DYN: None}],
    "wall_sconce_tv_right_bulb": [{DYN: None}],
}


def ensure_extensions_used(gltf: dict[str, Any]) -> None:
    used = gltf.setdefault("extensionsUsed", [])
    if "BEVY_skein" not in used:
        used.append("BEVY_skein")


def set_components(node: dict[str, Any], components: list[dict[str, Any]]) -> None:
    """Merge starter tags into BEVY_skein without clobbering Blender-authored ones."""
    ext = node.setdefault("extensions", {})
    skein = ext.setdefault("BEVY_skein", {})
    existing = skein.get("components")
    if not isinstance(existing, list):
        existing = []
    present: set[str] = set()
    for item in existing:
        if isinstance(item, dict):
            present.update(item.keys())
    merged = list(existing)
    for item in components:
        if not isinstance(item, dict) or not item:
            continue
        key = next(iter(item))
        if key in present:
            continue
        merged.append(item)
        present.add(key)
    skein["components"] = merged


def tag_gltf(path: Path) -> int:
    data = json.loads(path.read_text())
    nodes = data.get("nodes") or []
    tagged = 0
    missing_tv = True

    for node in nodes:
        name = node.get("name") or ""

        # Only the placement empty — never mesh children (…001 / _imported).
        if name == "television_02":
            set_components(node, TAG_MAP["television_02"])
            tagged += 1
            missing_tv = False
            continue

        if name.startswith("television_02"):
            # Strip accidental TV tags from non-canonical nodes.
            ext = node.get("extensions") or {}
            if "BEVY_skein" in ext:
                del ext["BEVY_skein"]
                if not ext:
                    node.pop("extensions", None)
            continue

        light = LIGHT_TAGS.get(name)
        if light is None:
            for key, val in LIGHT_TAGS.items():
                if name.startswith(key + "."):
                    light = val
                    break
        if light is not None:
            set_components(node, light)
            tagged += 1
            continue

        comps = TAG_MAP.get(name)
        if comps is None:
            for key, val in TAG_MAP.items():
                if name.startswith(key + "."):
                    comps = val
                    break
        if comps is not None:
            set_components(node, comps)
            tagged += 1

    if missing_tv:
        print(
            "WARN: no node named television_02 — phosphor attach will fail until tagged",
            file=sys.stderr,
        )

    ensure_extensions_used(data)
    path.write_text(json.dumps(data, indent=2) + "\n")
    print(f"Tagged {tagged} nodes in {path}")
    return 0 if not missing_tv else 2


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "gltf",
        nargs="?",
        default="crates/living_room/assets/skein/living_room_edit.gltf",
        type=Path,
    )
    args = ap.parse_args()
    if not args.gltf.is_file():
        print(f"missing {args.gltf}", file=sys.stderr)
        return 1
    return tag_gltf(args.gltf)


if __name__ == "__main__":
    raise SystemExit(main())
