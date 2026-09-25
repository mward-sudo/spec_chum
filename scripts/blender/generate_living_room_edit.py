#!/usr/bin/env python3
"""Build living_room_edit.blend (+ optional glTF) matching procedural room.rs / glow.rs.

Run via scripts/generate_living_room_blend.sh (or Blender --background --python …).

Coordinate note: Bevy / glTF are Y-up. Blender is Z-up. Placement uses bevy_to_blender()
so exported glTF matches room.rs world poses.

Skein: this script cannot call the Skein add-on (needs Fetch Registry + UI). It:
  - names objects to match room.rs
  - stores intended Bevy tags in custom properties + a Text datablock
  - after glTF export, scripts/blender/tag_living_room_skein_gltf.py injects
    BEVY_skein extensions so --features skein works without a manual first export
"""

from __future__ import annotations

import math
import sys
from pathlib import Path

import bpy
from mathutils import Euler, Quaternion, Vector

# ---------------------------------------------------------------------------
# Paths / CLI
# ---------------------------------------------------------------------------

SCRIPT = Path(__file__).resolve()
REPO = SCRIPT.parents[2]
ASSETS = REPO / "crates/living_room/assets"
POLY = ASSETS / "polyhaven"
OUT_DIR = ASSETS / "skein"
BLEND_OUT = OUT_DIR / "living_room_edit.blend"
GLTF_OUT = OUT_DIR / "living_room_edit.gltf"

# Match crates/living_room/src/room.rs
ROOM_W = 3.2
ROOM_D = 3.8
ROOM_H = 2.4
TV_STAND_POS = (0.0, 0.0, -1.35)
TV_STAND_SCALE = 0.85
TV_STAND_TOP = 0.68 * TV_STAND_SCALE
WALL_T = 0.06
WALLPAPER_TILE_M = 0.55


def argv_after_double_dash() -> list[str]:
    if "--" in sys.argv:
        return sys.argv[sys.argv.index("--") + 1 :]
    return []


def parse_args() -> dict:
    args = argv_after_double_dash()
    out = {
        "blend": str(BLEND_OUT),
        "gltf": str(GLTF_OUT),
        "export_gltf": True,
        "assets": str(ASSETS),
    }
    i = 0
    while i < len(args):
        a = args[i]
        if a == "--blend" and i + 1 < len(args):
            out["blend"] = args[i + 1]
            i += 2
        elif a == "--gltf" and i + 1 < len(args):
            out["gltf"] = args[i + 1]
            i += 2
        elif a == "--assets" and i + 1 < len(args):
            out["assets"] = args[i + 1]
            i += 2
        elif a == "--no-gltf":
            out["export_gltf"] = False
            i += 1
        else:
            i += 1
    return out


# ---------------------------------------------------------------------------
# Maths: Bevy Y-up → Blender Z-up
# ---------------------------------------------------------------------------


def bevy_to_blender(x: float, y: float, z: float) -> Vector:
    """Bevy (x,y,z) Y-up → Blender (x, -z, y) Z-up."""
    return Vector((x, -z, y))


def bevy_yaw_to_blender_quat(yaw_y: float) -> Quaternion:
    """Bevy rotation about +Y → Blender rotation about +Z."""
    return Euler((0.0, 0.0, yaw_y), "XYZ").to_quaternion()


def bevy_quat_yz_to_blender(qx: float, qy: float, qz: float, qw: float) -> Quaternion:
    """Convert a Bevy (Y-up) quaternion to Blender (Z-up).

    Conjugate by +90° about +X: Blender = R * q_bevy * R^{-1}.
    """
    r = Quaternion(Vector((1.0, 0.0, 0.0)), math.radians(90.0))
    q = Quaternion((qw, qx, qy, qz))
    return r @ q @ r.inverted()


# ---------------------------------------------------------------------------
# Scene helpers
# ---------------------------------------------------------------------------


def clear_scene() -> None:
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    for block in (bpy.data.meshes, bpy.data.materials, bpy.data.images, bpy.data.lights):
        for b in list(block):
            block.remove(b)


def ensure_collection(name: str) -> bpy.types.Collection:
    col = bpy.data.collections.get(name)
    if col is None:
        col = bpy.data.collections.new(name)
        bpy.context.scene.collection.children.link(col)
    return col


def link_object(obj: bpy.types.Object, col: bpy.types.Collection) -> None:
    # Unlink from scene root if present, link into target collection.
    for c in list(obj.users_collection):
        c.objects.unlink(obj)
    col.objects.link(obj)


def set_skein_hints(obj: bpy.types.Object, tags: list[str]) -> None:
    obj["skein_tags"] = ",".join(tags)
    obj["spec_chum_role"] = tags[0] if tags else ""


def load_image(path: Path) -> bpy.types.Image | None:
    if not path.is_file():
        print(f"WARN: missing texture {path}")
        return None
    return bpy.data.images.load(str(path), check_existing=True)


def make_pbr_material(
    name: str,
    stem: Path,
    *,
    metallic: float = 0.0,
    uv_scale: tuple[float, float] | None = None,
) -> bpy.types.Material:
    """stem is path without `_diff_1k.jpg` suffix."""
    mat = bpy.data.materials.new(name=name)
    mat.use_nodes = True
    nt = mat.node_tree
    nt.nodes.clear()
    out = nt.nodes.new("ShaderNodeOutputMaterial")
    bsdf = nt.nodes.new("ShaderNodeBsdfPrincipled")
    out.location = (300, 0)
    bsdf.location = (0, 0)
    nt.links.new(bsdf.outputs["BSDF"], out.inputs["Surface"])

    diff = load_image(Path(f"{stem}_diff_1k.jpg"))
    nor = load_image(Path(f"{stem}_nor_gl_1k.jpg"))
    arm = load_image(Path(f"{stem}_arm_1k.jpg"))

    tex_coord = nt.nodes.new("ShaderNodeTexCoord")
    mapping = nt.nodes.new("ShaderNodeMapping")
    tex_coord.location = (-800, 0)
    mapping.location = (-600, 0)
    nt.links.new(tex_coord.outputs["UV"], mapping.inputs["Vector"])
    if uv_scale is not None:
        mapping.inputs["Scale"].default_value[0] = uv_scale[0]
        mapping.inputs["Scale"].default_value[1] = uv_scale[1]

    def add_tex(img: bpy.types.Image | None, y: float, non_color: bool) -> bpy.types.Node | None:
        if img is None:
            return None
        node = nt.nodes.new("ShaderNodeTexImage")
        node.image = img
        node.location = (-350, y)
        if non_color:
            img.colorspace_settings.name = "Non-Color"
        nt.links.new(mapping.outputs["Vector"], node.inputs["Vector"])
        return node

    diff_n = add_tex(diff, 200, False)
    nor_n = add_tex(nor, 0, True)
    arm_n = add_tex(arm, -200, True)

    if diff_n:
        nt.links.new(diff_n.outputs["Color"], bsdf.inputs["Base Color"])
    if nor_n:
        normal_map = nt.nodes.new("ShaderNodeNormalMap")
        normal_map.location = (-100, 0)
        nt.links.new(nor_n.outputs["Color"], normal_map.inputs["Color"])
        nt.links.new(normal_map.outputs["Normal"], bsdf.inputs["Normal"])
    if arm_n:
        # ARM: R=AO, G=Roughness, B=Metalness (Poly Haven)
        sep = nt.nodes.new("ShaderNodeSeparateColor")
        sep.location = (-100, -200)
        nt.links.new(arm_n.outputs["Color"], sep.inputs["Color"])
        nt.links.new(sep.outputs["Green"], bsdf.inputs["Roughness"])
        if metallic > 0.0:
            bsdf.inputs["Metallic"].default_value = metallic
        else:
            nt.links.new(sep.outputs["Blue"], bsdf.inputs["Metallic"])
    else:
        bsdf.inputs["Metallic"].default_value = metallic
        bsdf.inputs["Roughness"].default_value = 1.0

    return mat


def add_box(
    name: str,
    size_bevy: tuple[float, float, float],
    pos_bevy: tuple[float, float, float],
    mat: bpy.types.Material | None,
    col: bpy.types.Collection,
    *,
    tags: list[str] | None = None,
) -> bpy.types.Object:
    """size_bevy is full extents in Bevy axes (X,Y,Z); Blender cube default is 2m."""
    bpy.ops.mesh.primitive_cube_add(size=1.0)
    obj = bpy.context.active_object
    obj.name = name
    # Bevy size (sx,sy,sz) → Blender scale on (X,Y,Z) = (sx, sz, sy) / 1.0 since cube=1? 
    # Default cube is 2×2×2; with size=1.0 ops still create 2m cube in recent Blender.
    # Use dimensions explicitly.
    sx, sy, sz = size_bevy
    # Blender axes: X=sx, Y=sz, Z=sy
    obj.scale = (sx / 2.0, sz / 2.0, sy / 2.0)
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    obj.location = bevy_to_blender(*pos_bevy)
    if mat:
        if obj.data.materials:
            obj.data.materials[0] = mat
        else:
            obj.data.materials.append(mat)
    link_object(obj, col)
    if tags:
        set_skein_hints(obj, tags)
    return obj


def add_plane(
    name: str,
    size_xz_bevy: tuple[float, float],
    pos_bevy: tuple[float, float, float],
    mat: bpy.types.Material | None,
    col: bpy.types.Collection,
    *,
    flip_y: bool = False,
    tags: list[str] | None = None,
) -> bpy.types.Object:
    """Horizontal plane: Bevy Plane3d size (ROOM_W, ROOM_D) on XZ at y."""
    w, d = size_xz_bevy
    bpy.ops.mesh.primitive_plane_add(size=1.0)
    obj = bpy.context.active_object
    obj.name = name
    # Default plane is XY in Blender (Z up normal). We want floor: Blender XY with Z=up →
    # Bevy XZ floor. bevy_to_blender maps Bevy XZ → Blender X(-Z wait):
    # Floor in Bevy is XZ; in Blender XY. Scale: X=w, Y=d (Blender Y = -Bevy Z extent).
    obj.scale = (w / 2.0, d / 2.0, 1.0)
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    if flip_y:
        # Ceiling: rotate 180 about X (matches room.rs Quat::from_rotation_x(PI))
        obj.rotation_euler = (math.pi, 0.0, 0.0)
    obj.location = bevy_to_blender(*pos_bevy)
    if mat:
        if obj.data.materials:
            obj.data.materials[0] = mat
        else:
            obj.data.materials.append(mat)
    link_object(obj, col)
    if tags:
        set_skein_hints(obj, tags)
    return obj


def import_gltf(path: Path) -> list[bpy.types.Object]:
    before = set(bpy.data.objects)
    bpy.ops.import_scene.gltf(filepath=str(path))
    return [o for o in bpy.data.objects if o not in before]


def place_imported(
    path: Path,
    name: str,
    pos_bevy: tuple[float, float, float],
    col: bpy.types.Collection,
    *,
    yaw_bevy: float | None = None,
    rot_bevy_quat: Quaternion | None = None,
    scale: float | tuple[float, float, float] = 1.0,
    tags: list[str] | None = None,
) -> bpy.types.Object:
    """Import glTF and parent under an Empty with Bevy-matching transform."""
    if not path.is_file():
        raise FileNotFoundError(path)

    imported = import_gltf(path)
    # Ensure the placement empty owns the canonical name (imports may reuse it).
    if name in bpy.data.objects:
        bpy.data.objects[name].name = f"{name}_imported"
    root = bpy.data.objects.new(name, None)
    root.empty_display_type = "PLAIN_AXES"
    root.empty_display_size = 0.15
    root.location = bevy_to_blender(*pos_bevy)
    if rot_bevy_quat is not None:
        root.rotation_mode = "QUATERNION"
        root.rotation_quaternion = rot_bevy_quat
    elif yaw_bevy is not None:
        root.rotation_mode = "QUATERNION"
        root.rotation_quaternion = bevy_yaw_to_blender_quat(yaw_bevy)
    if isinstance(scale, tuple):
        # Bevy scale (sx,sy,sz) → Blender (sx, sz, sy)
        sx, sy, sz = scale
        root.scale = (sx, sz, sy)
    else:
        root.scale = (scale, scale, scale)

    link_object(root, col)
    if tags:
        set_skein_hints(root, tags)

    # Parent top-level imported objects under root, preserving world pose at import.
    tops = [o for o in imported if o.parent is None]
    for o in tops:
        o.parent = root
        # Keep current world matrix: clear parent inverse so child stays put relative to root
        o.matrix_parent_inverse = root.matrix_world.inverted()

    # Move imported objects into collection
    for o in imported:
        link_object(o, col)

    return root


def add_point_light(
    name: str,
    pos_bevy: tuple[float, float, float],
    *,
    color: tuple[float, float, float],
    bevy_intensity: float,
    bevy_range: float,
    radius: float,
    col: bpy.types.Collection,
    tags: list[str] | None = None,
) -> bpy.types.Object:
    light_data = bpy.data.lights.new(name=name, type="POINT")
    # Rough Blender Power for viewport readability; intended Bevy lumens stored on object.
    light_data.energy = max(bevy_intensity / 80.0, 5.0)
    light_data.color = color
    light_data.shadow_soft_size = radius
    light_data.use_shadow = False
    obj = bpy.data.objects.new(name, light_data)
    obj.location = bevy_to_blender(*pos_bevy)
    obj["bevy_intensity"] = bevy_intensity
    obj["bevy_range"] = bevy_range
    obj["bevy_radius"] = radius
    obj["bevy_color"] = list(color)
    link_object(obj, col)
    if tags:
        set_skein_hints(obj, tags)
    return obj


def add_joystick(col: bpy.types.Collection) -> None:
    """Competition Pro–style stick from room.rs spawn_spectrum_joystick."""
    root = bpy.data.objects.new("spectrum_joystick", None)
    root.location = bevy_to_blender(0.55, 0.0, 0.65)
    root.rotation_mode = "QUATERNION"
    root.rotation_quaternion = bevy_yaw_to_blender_quat(0.35)
    link_object(root, col)
    set_skein_hints(root, ["RoomStatic"])

    black = bpy.data.materials.new("joy_black")
    black.use_nodes = True
    black.node_tree.nodes["Principled BSDF"].inputs["Base Color"].default_value = (
        0.08,
        0.08,
        0.09,
        1.0,
    )
    red = bpy.data.materials.new("joy_red")
    red.use_nodes = True
    red.node_tree.nodes["Principled BSDF"].inputs["Base Color"].default_value = (
        0.85,
        0.12,
        0.1,
        1.0,
    )

    def parent_local(obj: bpy.types.Object, loc_bevy: tuple[float, float, float]) -> None:
        obj.parent = root
        obj.matrix_parent_inverse.identity()
        obj.location = bevy_to_blender(*loc_bevy)

    bpy.ops.mesh.primitive_cube_add(size=1.0)
    base = bpy.context.active_object
    base.name = "joy_base"
    base.scale = (0.11 / 2, 0.11 / 2, 0.035 / 2)
    bpy.ops.object.transform_apply(scale=True)
    base.data.materials.append(black)
    link_object(base, col)
    parent_local(base, (0.0, 0.0175, 0.0))

    bpy.ops.mesh.primitive_cylinder_add(radius=0.012, depth=0.07)
    shaft = bpy.context.active_object
    shaft.name = "joy_shaft"
    shaft.data.materials.append(black)
    link_object(shaft, col)
    parent_local(shaft, (0.0, 0.06, 0.0))

    bpy.ops.mesh.primitive_uv_sphere_add(radius=0.028, segments=16, ring_count=12)
    ball = bpy.context.active_object
    ball.name = "joy_ball"
    ball.data.materials.append(black)
    link_object(ball, col)
    parent_local(ball, (0.0, 0.105, 0.0))

    for x, name in [(-0.028, "joy_fire_l"), (0.028, "joy_fire_r")]:
        bpy.ops.mesh.primitive_cylinder_add(radius=0.014, depth=0.01)
        btn = bpy.context.active_object
        btn.name = name
        btn.data.materials.append(red)
        link_object(btn, col)
        parent_local(btn, (x, 0.038, 0.032))


def write_readme_text() -> None:
    body = """Spec Chum living_room_edit — starter scene
==========================================

Generated to match procedural crates/living_room/src/room.rs + glow.rs.

Edit workflow
-------------
1. Open this .blend in Blender (Skein add-on installed).
2. Start: cargo run -p living_room --release --features skein
3. Blender → Fetch Bevy registry → http://127.0.0.1:15702
4. Select television_02 empty → Insert TelevisionCabinet (+ LiveTv).
5. Optional: apply Skein presets / GlowDriven on CRT spill lights.
6. Export glTF (extras / BEVY_skein) → assets/skein/living_room_edit.gltf

Custom property skein_tags on objects lists intended components.
After generate_living_room_blend.sh, living_room_edit.gltf already has
BEVY_skein tags injected for a first runnable Skein room.

Force procedural: SPEC_CHUM_ROOM_SKEIN_SCENE=off
"""
    text = bpy.data.texts.get("SPEC_CHUM_SKEIN_README")
    if text is None:
        text = bpy.data.texts.new("SPEC_CHUM_SKEIN_README")
    text.clear()
    text.write(body)


def build_room(assets: Path) -> None:
    poly = assets / "polyhaven"
    models = poly / "models"
    textures = poly / "textures"

    shell = ensure_collection("01_RoomShell")
    furniture = ensure_collection("02_Furniture")
    props = ensure_collection("03_Props")
    lights = ensure_collection("04_Lights")

    carpet = make_pbr_material(
        "carpet", textures / "dirty_carpet/dirty_carpet"
    )
    wallpaper_back = make_pbr_material(
        "wallpaper_back",
        textures / "floral_jacquard/floral_jacquard",
        uv_scale=(ROOM_W / WALLPAPER_TILE_M, ROOM_H / WALLPAPER_TILE_M),
    )
    wallpaper_side = make_pbr_material(
        "wallpaper_side",
        textures / "floral_jacquard/floral_jacquard",
        uv_scale=(ROOM_D / WALLPAPER_TILE_M, ROOM_H / WALLPAPER_TILE_M),
    )
    plaster = make_pbr_material(
        "plaster", textures / "beige_wall_001/beige_wall_001"
    )
    walnut = make_pbr_material(
        "walnut",
        textures / "american_walnut_veneer/american_walnut_veneer",
        metallic=0.05,
    )
    curtain_mat = make_pbr_material(
        "curtain", textures / "velour_velvet/velour_velvet"
    )

    add_plane(
        "carpet",
        (ROOM_W, ROOM_D),
        (0.0, 0.0, 0.0),
        carpet,
        shell,
        tags=["RoomStatic"],
    )
    add_plane(
        "ceiling",
        (ROOM_W, ROOM_D),
        (0.0, ROOM_H, 0.0),
        plaster,
        shell,
        flip_y=True,
        tags=["RoomStatic"],
    )

    for name, pos, size, mat in [
        (
            "wall_back",
            (0.0, ROOM_H * 0.5, -ROOM_D * 0.5),
            (ROOM_W, ROOM_H, WALL_T),
            wallpaper_back,
        ),
        (
            "wall_front",
            (0.0, ROOM_H * 0.5, ROOM_D * 0.5),
            (ROOM_W, ROOM_H, WALL_T),
            wallpaper_back,
        ),
        (
            "wall_left",
            (-ROOM_W * 0.5, ROOM_H * 0.5, 0.0),
            (WALL_T, ROOM_H, ROOM_D),
            wallpaper_side,
        ),
        (
            "wall_right",
            (ROOM_W * 0.5, ROOM_H * 0.5, 0.0),
            (WALL_T, ROOM_H, ROOM_D),
            wallpaper_side,
        ),
    ]:
        add_box(name, size, pos, mat, shell, tags=["RoomStatic"])

    for x in (-1.25, 1.25):
        add_box(
            "curtain",
            (0.4, 2.0, 0.06),
            (x, 1.15, -ROOM_D * 0.5 + 0.06),
            curtain_mat,
            shell,
            tags=["RoomStatic"],
        )

    add_box(
        "skirting",
        (ROOM_W - 0.12, 0.09, 0.03),
        (0.0, 0.045, -ROOM_D * 0.5 + 0.04),
        walnut,
        shell,
        tags=["RoomStatic"],
    )

    # --- Furniture (glTF) ---
    place_imported(
        models / "modern_wooden_cabinet/modern_wooden_cabinet_1k.gltf",
        "tv_stand",
        TV_STAND_POS,
        furniture,
        scale=TV_STAND_SCALE,
        tags=["LiveTv"],
    )

    tv_pos = (
        TV_STAND_POS[0],
        TV_STAND_POS[1] + TV_STAND_TOP,
        TV_STAND_POS[2] + 0.05,
    )
    place_imported(
        models / "television_02/television_02_aperture.gltf",
        "television_02",
        tv_pos,
        furniture,
        tags=["TelevisionCabinet", "LiveTv"],
    )

    place_imported(
        models / "sofa_03/sofa_03_1k.gltf",
        "sofa",
        (0.0, 0.0, 1.05),
        furniture,
        yaw_bevy=math.pi,
        scale=0.72,
        tags=["RoomStatic"],
    )

    for x, yaw in [(-1.15, -0.55), (1.15, 0.55)]:
        place_imported(
            models / "ArmChair_01/ArmChair_01_1k.gltf",
            "armchair",
            (x, 0.0, 0.35),
            furniture,
            yaw_bevy=yaw,
            tags=["RoomStatic"],
        )

    # Sconces
    wall_z = -ROOM_D * 0.5 + 0.03
    wall_x = ROOM_W * 0.5 - 0.03
    sconce_path = models / "industrial_wall_sconce/industrial_wall_sconce_1k.gltf"
    mounts = [
        ((-1.32, 1.55, wall_z), (0.0, 0.0, 1.0), "wall_sconce_tv_left", True),
        ((0.0, 1.85, wall_z), (0.0, 0.0, 1.0), "wall_sconce_tv_centre", True),
        ((1.32, 1.55, wall_z), (0.0, 0.0, 1.0), "wall_sconce_tv_right", True),
        ((-wall_x, 1.45, 0.55), (1.0, 0.0, 0.0), "wall_sconce_left", False),
        ((wall_x, 1.45, 0.55), (-1.0, 0.0, 0.0), "wall_sconce_right", False),
    ]
    for pos, into, name, lit in mounts:
        # Bevy Quat::from_rotation_arc(+Z, into_room) — vectors in Bevy Y-up space.
        bevy_q = Vector((0.0, 0.0, 1.0)).rotation_difference(Vector(into).normalized())
        place_imported(
            sconce_path,
            name,
            pos,
            furniture,
            rot_bevy_quat=bevy_quat_yz_to_blender(bevy_q.x, bevy_q.y, bevy_q.z, bevy_q.w),
            tags=["RoomStatic"],
        )
        if lit:
            if name == "wall_sconce_tv_centre":
                bulb_local = Vector((0.0, 0.16, 0.32))
                intensity = 5400.0
            else:
                bulb_local = Vector((0.0, 0.05, 0.16))
                intensity = 6500.0
            rotated = bevy_q @ bulb_local
            bulb_world = (
                pos[0] + rotated.x,
                pos[1] + rotated.y,
                pos[2] + rotated.z,
            )
            add_point_light(
                f"{name}_bulb",
                bulb_world,
                color=(1.0, 0.72, 0.38),
                bevy_intensity=intensity,
                bevy_range=6.0,
                radius=0.08,
                col=lights,
                tags=["DynamicRoomFillLight"],
            )

    # Toys
    toys = [
        (
            "dirty_football/dirty_football_1k.gltf",
            (-0.7, 0.12, 0.55),
            0.7,
            1.15,
            "toy_football",
        ),
        (
            "rubber_duck_toy/rubber_duck_toy_1k.gltf",
            (-0.35, 0.0, 0.45),
            -0.9,
            1.1,
            "toy_rubber_duck",
        ),
        (
            "gamepad/gamepad_1k.gltf",
            (0.55, 0.0, 0.5),
            2.4,
            1.8,
            "toy_gamepad",
        ),
        (
            "portable_cassette_player/portable_cassette_player_1k.gltf",
            (0.85, 0.055, 0.35),
            -0.4,
            1.35,
            "toy_walkman",
        ),
    ]
    for rel, pos, yaw, sc, name in toys:
        place_imported(
            models / rel,
            name,
            pos,
            props,
            yaw_bevy=yaw,
            scale=sc,
            tags=["RoomStatic"],
        )

    add_joystick(props)

    # CRT fill + wall bounce (glow.rs) — positions match procedural defaults
    # crt_fill_offset(phosphor≈(0,1.17,-1.15)) = + (0.18,-0.08,0.22)
    fill_pos = (0.18, 1.17 - 0.08, -1.15 + 0.22)
    add_point_light(
        "crt_fill_light",
        fill_pos,
        color=(0.4, 0.45, 0.35),
        bevy_intensity=1800.0,
        bevy_range=5.0,
        radius=0.0,
        col=lights,
        tags=["CrtFillLight", "GlowDriven"],
    )
    add_point_light(
        "crt_wall_bounce",
        (0.0, 1.55, -1.55),
        color=(0.35, 0.38, 0.32),
        bevy_intensity=810.0,
        bevy_range=7.0,
        radius=0.0,
        col=lights,
        tags=["GlowDriven", "DynamicRoomFillLight"],
    )

    write_readme_text()


def export_gltf(path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    bpy.ops.export_scene.gltf(
        filepath=str(path),
        export_format="GLTF_SEPARATE",
        export_lights=True,
        export_extras=True,
        export_apply=False,
        use_selection=False,
    )


def main() -> None:
    opts = parse_args()
    assets = Path(opts["assets"])
    blend_path = Path(opts["blend"])
    gltf_path = Path(opts["gltf"])

    if not (assets / "polyhaven").is_dir():
        print(
            f"ERROR: polyhaven assets missing under {assets} — "
            "run ./scripts/fetch_living_room_assets.sh",
            file=sys.stderr,
        )
        sys.exit(1)

    clear_scene()
    bpy.context.scene.unit_settings.system = "METRIC"
    bpy.context.scene.unit_settings.scale_length = 1.0

    print(f"Building living room from {assets} …")
    build_room(assets)

    blend_path.parent.mkdir(parents=True, exist_ok=True)
    bpy.ops.wm.save_as_mainfile(filepath=str(blend_path))
    print(f"Wrote {blend_path}")

    if opts["export_gltf"]:
        export_gltf(gltf_path)
        print(f"Wrote {gltf_path}")

    # Signal success for the shell wrapper
    print("GENERATE_LIVING_ROOM_EDIT_OK")


if __name__ == "__main__":
    main()
