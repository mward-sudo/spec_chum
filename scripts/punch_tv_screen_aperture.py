#!/usr/bin/env python3
"""Punch the painted CRT glass out of television_02; keep both real bezels.

Poly Haven's television_02 is one solid mesh (Cube.001). Visually it has two
bezels that must stay as geometry:

  1. Outer cabinet / frame bevel (plastic body)
  2. Inner screen bezel / glass rim (dark border that clips the picture)

The painted glass face uses a distinct UV island on the diffuse atlas
(u≈0.68–1.0, v≈0.00–0.20). Removing only those forward-facing triangles opens
the hole at the **inner** rim. Spec Chum places an oversized phosphor slightly
behind that rim so overscan is occluded by the real inner bezel — never by a
fake flat lip in front of the outer bevel.

Writes:
  crates/living_room/assets/polyhaven/models/television_02/television_02_aperture.gltf
  crates/living_room/assets/polyhaven/models/television_02/television_02_aperture.bin
  crates/living_room/assets/polyhaven/models/television_02/aperture_metrics.json

Re-run after replacing the source glTF:
  python3 scripts/punch_tv_screen_aperture.py
"""

from __future__ import annotations

import json
import struct
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TV_DIR = ROOT / "crates/living_room/assets/polyhaven/models/television_02"
SRC_GLTF = TV_DIR / "television_02_1k.gltf"
SRC_BIN = TV_DIR / "television_02.bin"
OUT_GLTF = TV_DIR / "television_02_aperture.gltf"
OUT_BIN = TV_DIR / "television_02_aperture.bin"
OUT_METRICS = TV_DIR / "aperture_metrics.json"

# Painted CRT glass UV island on television_02_diff_1k.jpg (not the bezels).
# V must reach ~0.24: bottom glass rows sit at mean v≈0.21–0.225. Capping at 0.20
# left sawteeth; 0.21 still left a curved bottom lip (dark crescent over magenta).
GLASS_U = (0.68, 1.01)
GLASS_V = (0.00, 0.24)


def is_screen_tri(
    t: tuple[int, int, int],
    pts: list[tuple[float, float, float]],
    norms: list[tuple[float, float, float]],
    uvs: list[tuple[float, float]],
) -> bool:
    """True only for the painted glass face (inner aperture), not either bezel."""
    i0, i1, i2 = t
    nz = (norms[i0][2] + norms[i1][2] + norms[i2][2]) / 3.0
    # Glass normals face the viewer; skip any stray backfaces.
    if nz < 0.5:
        return False
    u = (uvs[i0][0] + uvs[i1][0] + uvs[i2][0]) / 3.0
    v = (uvs[i0][1] + uvs[i1][1] + uvs[i2][1]) / 3.0
    if not (GLASS_U[0] <= u <= GLASS_U[1] and GLASS_V[0] <= v <= GLASS_V[1]):
        return False
    # Sanity: glass sits on the front of the tube, not elsewhere on the atlas reuse.
    xs = [pts[i][0] for i in t]
    ys = [pts[i][1] for i in t]
    zs = [pts[i][2] for i in t]
    cx, cy, cz = sum(xs) / 3.0, sum(ys) / 3.0, sum(zs) / 3.0
    if abs(cx) > 0.20 or not (0.14 < cy < 0.42) or cz < 0.08:
        return False
    return True


def main() -> int:
    if not SRC_GLTF.is_file() or not SRC_BIN.is_file():
        print(f"missing source assets under {TV_DIR}", file=sys.stderr)
        return 1

    gltf = json.loads(SRC_GLTF.read_text())
    blob = bytearray(SRC_BIN.read_bytes())

    def accessor_info(i: int):
        a = gltf["accessors"][i]
        bv = gltf["bufferViews"][a["bufferView"]]
        off = bv.get("byteOffset", 0) + a.get("byteOffset", 0)
        return a, off, a["count"], a["componentType"]

    prim = gltf["meshes"][0]["primitives"][0]
    attrs = prim["attributes"]

    _, off, n, _ = accessor_info(attrs["POSITION"])
    vals = struct.unpack_from("<" + "f" * (n * 3), blob, off)
    pts = [(vals[i], vals[i + 1], vals[i + 2]) for i in range(0, len(vals), 3)]

    _, off, n, _ = accessor_info(attrs["NORMAL"])
    nvals = struct.unpack_from("<" + "f" * (n * 3), blob, off)
    norms = [(nvals[i], nvals[i + 1], nvals[i + 2]) for i in range(0, len(nvals), 3)]

    _, off, n, _ = accessor_info(attrs["TEXCOORD_0"])
    uvals = struct.unpack_from("<" + "f" * (n * 2), blob, off)
    uvs = [(uvals[i], uvals[i + 1]) for i in range(0, len(uvals), 2)]

    _, off, n, ct = accessor_info(prim["indices"])
    fmt_by_ct = {5121: "B", 5123: "H", 5125: "I"}
    try:
        fmt = fmt_by_ct[ct]
    except KeyError as e:
        raise SystemExit(f"unsupported index componentType {ct}") from e
    idxs = struct.unpack_from("<" + fmt * n, blob, off)
    tris = [(idxs[i], idxs[i + 1], idxs[i + 2]) for i in range(0, n, 3)]

    screen = [t for t in tris if is_screen_tri(t, pts, norms, uvs)]
    keep = [t for t in tris if not is_screen_tri(t, pts, norms, uvs)]
    if not screen or not keep:
        print("screen classification failed", len(screen), len(keep), file=sys.stderr)
        return 1

    # Safety: drop any leftover forward face whose verts all sit on the glass island.
    # Catches UV-fringe rows if GLASS_V is slightly tight after an asset refresh.
    sv = {i for t in screen for i in t}
    fringe = [
        t
        for t in keep
        if all(i in sv for i in t)
        and (norms[t[0]][2] + norms[t[1]][2] + norms[t[2]][2]) / 3.0 >= 0.5
    ]
    if fringe:
        screen = screen + fringe
        keep = [t for t in keep if t not in set(fringe)]
        sv = {i for t in screen for i in t}
        print(f"removed {len(fringe)} leftover glass-island fringe tris (sawtooth cleanup)")
    # Full glass XY AABB *is* the visible opening: CRT glass curves back in Z at
    # the perimeter, but those edge verts still sit at the inner-bezel opening in XY.
    # A z-filtered "rim-plane" slice shrinks W/H and leaves a gap (see magenta vs lime
    # debug). Phosphor uses full AABB + slight overscan, seated *behind* the rim.
    aperture_w = max(pts[i][0] for i in sv) - min(pts[i][0] for i in sv)
    aperture_h = max(pts[i][1] for i in sv) - min(pts[i][1] for i in sv)
    center = [
        (min(pts[i][0] for i in sv) + max(pts[i][0] for i in sv)) / 2.0,
        (min(pts[i][1] for i in sv) + max(pts[i][1] for i in sv)) / 2.0,
        sum(pts[i][2] for i in sv) / len(sv),
    ]
    z_front = max(pts[i][2] for i in sv)
    z_back = min(pts[i][2] for i in sv)
    # Diagnostic only — smaller z-filtered slice (do not size phosphor to this).
    zs = sorted(pts[i][2] for i in sv)
    z_rim = zs[int(0.40 * (len(zs) - 1))]
    rim_vs = [i for i in sv if pts[i][2] >= z_rim] or list(sv)
    rim_w = max(pts[i][0] for i in rim_vs) - min(pts[i][0] for i in rim_vs)
    rim_h = max(pts[i][1] for i in rim_vs) - min(pts[i][1] for i in rim_vs)

    # Append new index buffer (u16) at end of bin copy.
    new_indices = []
    for t in keep:
        new_indices.extend(t)
    max_idx = max(new_indices) if new_indices else 0
    if max_idx > 0xFFFF:
        out_fmt, out_ct = "I", 5125
    elif max_idx > 0xFF:
        out_fmt, out_ct = "H", 5123
    else:
        out_fmt, out_ct = "B", 5121
    idx_bytes = struct.pack("<" + out_fmt * len(new_indices), *new_indices)
    # Align to 4 bytes.
    pad = (4 - (len(blob) % 4)) % 4
    blob.extend(b"\x00" * pad)
    idx_offset = len(blob)
    blob.extend(idx_bytes)
    if len(blob) % 4:
        blob.extend(b"\x00" * (4 - (len(blob) % 4)))

    # New bufferView + accessor for indices.
    bv_idx = len(gltf["bufferViews"])
    gltf["bufferViews"].append(
        {
            "buffer": 0,
            "byteOffset": idx_offset,
            "byteLength": len(idx_bytes),
            "target": 34963,  # ELEMENT_ARRAY_BUFFER
        }
    )
    acc_idx = len(gltf["accessors"])
    gltf["accessors"].append(
        {
            "bufferView": bv_idx,
            "byteOffset": 0,
            "componentType": out_ct,
            "count": len(new_indices),
            "type": "SCALAR",
            "max": [max(new_indices)],
            "min": [min(new_indices)],
        }
    )
    prim["indices"] = acc_idx
    gltf["buffers"][0]["byteLength"] = len(blob)
    gltf["buffers"][0]["uri"] = "television_02_aperture.bin"
    # Keep relative texture URIs working from the same folder.
    gltf["asset"]["generator"] = "spec_chum punch_tv_screen_aperture.py"
    if "extras" not in gltf:
        gltf["extras"] = {}
    gltf["extras"]["spec_chum_aperture"] = {
        "screen_tris_removed": len(screen),
        "tris_kept": len(keep),
        "aperture_w": aperture_w,
        "aperture_h": aperture_h,
        "center_local": center,
        "z_front": z_front,
        "z_back": z_back,
        "rim_plane_w": rim_w,
        "rim_plane_h": rim_h,
        "z_rim_threshold": z_rim,
        "clip_rim": "inner_screen_bezel",
        "kept": "outer_cabinet_bezel + inner_screen_bezel",
    }

    OUT_BIN.write_bytes(blob)
    OUT_GLTF.write_text(json.dumps(gltf, indent=2) + "\n")
    metrics = {
        "aperture_w": aperture_w,
        "aperture_h": aperture_h,
        "center_local": center,
        "z_front": z_front,
        "z_back": z_back,
        "rim_plane_w": rim_w,
        "rim_plane_h": rim_h,
        "z_rim_threshold": z_rim,
        "screen_tris_removed": len(screen),
        "tris_kept": len(keep),
        "clip_rim": "inner_screen_bezel",
        "note": (
            "aperture_w/h = full glass XY AABB (visible opening). rim_plane_* is a "
            "smaller z-filtered diagnostic — do not size phosphor to it. Phosphor "
            "sits behind z_front with slight overscan under the inner bezel only."
        ),
    }
    OUT_METRICS.write_text(json.dumps(metrics, indent=2) + "\n")
    print(f"wrote {OUT_GLTF.relative_to(ROOT)}")
    print(f"wrote {OUT_BIN.relative_to(ROOT)} ({len(blob)} bytes)")
    print(f"removed {len(screen)} glass tris; kept {len(keep)} (both bezels)")
    print(
        f"aperture {aperture_w:.4f}×{aperture_h:.4f} aspect={aperture_w / aperture_h:.3f} "
        f"centre=({center[0]:.4f},{center[1]:.4f},{center[2]:.4f}) "
        f"(rim-plane diagnostic {rim_w:.4f}×{rim_h:.4f})"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
