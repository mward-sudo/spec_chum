#!/usr/bin/env python3
"""Bake static living-room diffuse lighting and export a UV1 glTF for Bevy.

The full procedural-matching scene is built with generate_living_room_edit.py.
Only objects tagged RoomStatic are duplicated into one atlas mesh. The TV,
cabinet and CRT are excluded from the export, but may occlude the static bake.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import bpy

sys.path.insert(0, str(Path(__file__).resolve().parent))
import generate_living_room_edit as room


def parse_args() -> tuple[Path, Path, int, int, int]:
    args = room.argv_after_double_dash()
    assets = room.ASSETS
    output = None
    size = 2048
    samples = 32
    threads = 4
    i = 0
    while i < len(args):
        if args[i] == "--assets" and i + 1 < len(args):
            assets = Path(args[i + 1]).resolve()
            i += 2
        elif args[i] == "--output" and i + 1 < len(args):
            output = Path(args[i + 1]).resolve()
            i += 2
        elif args[i] == "--size" and i + 1 < len(args):
            size = int(args[i + 1])
            i += 2
        elif args[i] == "--samples" and i + 1 < len(args):
            samples = int(args[i + 1])
            i += 2
        elif args[i] == "--threads" and i + 1 < len(args):
            threads = int(args[i + 1])
            i += 2
        else:
            raise ValueError(f"unknown bake argument: {args[i]}")
    if size < 512 or size > 4096 or size & (size - 1):
        raise ValueError("--size must be a power of two from 512 to 4096")
    if samples < 1 or samples > 1024:
        raise ValueError("--samples must be between 1 and 1024")
    if threads < 1 or threads > 16:
        raise ValueError("--threads must be between 1 and 16")
    return assets, output or assets / "lightmaps", size, samples, threads


def is_static_mesh(obj: bpy.types.Object) -> bool:
    if obj.type != "MESH":
        return False
    parent = obj
    while parent is not None:
        if parent.get("spec_chum_role") == "RoomStatic":
            return True
        parent = parent.parent
    return False


def make_atlas_mesh() -> bpy.types.Object:
    bpy.context.view_layer.update()
    source = [obj for obj in bpy.data.objects if is_static_mesh(obj)]
    if not source:
        raise RuntimeError("room generator produced no RoomStatic mesh objects")

    collection = room.ensure_collection("05_LightmapBake")
    copies = []
    for obj in source:
        copy = bpy.data.objects.new(f"bake_{obj.name}", obj.data.copy())
        # Join merges UV layers by name. Keep one common material UV layer so
        # the bake layer is precisely the second layer on the final mesh.
        layers = copy.data.uv_layers
        if not layers:
            layers.new(name="UVMap")
        while len(layers) > 1:
            layers.remove(layers[-1])
        layers[0].name = "UVMap"
        collection.objects.link(copy)
        copy.matrix_world = obj.matrix_world.copy()
        copies.append(copy)
        # The copy participates in the bake; the original would z-fight it.
        obj.hide_render = True
        obj.hide_set(True)

    bpy.ops.object.select_all(action="DESELECT")
    for obj in copies:
        obj.select_set(True)
    bpy.context.view_layer.objects.active = copies[0]
    bpy.ops.object.join()
    atlas_mesh = bpy.context.view_layer.objects.active
    atlas_mesh.name = "spec_chum_room_static_baked"
    atlas_mesh.data.name = "spec_chum_room_static_baked"
    atlas_mesh["spec_chum_role"] = "RoomStatic"

    # Keep source material UVs in TEXCOORD_0; Blender's second UV layer becomes
    # TEXCOORD_1 in glTF and Bevy's Mesh::ATTRIBUTE_UV_1.
    if not atlas_mesh.data.uv_layers:
        atlas_mesh.data.uv_layers.new(name="UVMap")
    atlas_mesh.data.uv_layers.new(name="Lightmap")
    atlas_mesh.data.uv_layers.active = atlas_mesh.data.uv_layers["Lightmap"]
    bpy.ops.object.mode_set(mode="EDIT")
    bpy.ops.mesh.select_all(action="SELECT")
    bpy.ops.uv.smart_project(island_margin=0.015)
    bpy.ops.object.mode_set(mode="OBJECT")
    return atlas_mesh


def bake(
    atlas_mesh: bpy.types.Object, output: Path, size: int, samples: int, threads: int
) -> Path:
    image = bpy.data.images.new("room_static_lightmap", width=size, height=size)
    image.generated_color = (0.0, 0.0, 0.0, 1.0)
    image.file_format = "PNG"

    for slot in atlas_mesh.material_slots:
        material = slot.material
        if material is None:
            material = bpy.data.materials.new("lightmap_plain")
            material.use_nodes = True
            slot.material = material
        if not material.use_nodes:
            material.use_nodes = True
        node = material.node_tree.nodes.new("ShaderNodeTexImage")
        node.name = "__spec_chum_lightmap_bake_target__"
        node.image = image
        material.node_tree.nodes.active = node

    scene = bpy.context.scene
    scene.render.engine = "CYCLES"
    scene.render.threads_mode = "FIXED"
    scene.render.threads = threads
    scene.cycles.samples = samples
    scene.render.bake.use_pass_color = False
    scene.render.bake.use_pass_direct = True
    scene.render.bake.use_pass_indirect = True
    scene.render.bake.margin = 8
    scene.view_settings.view_transform = "Standard"
    scene.view_settings.exposure = 0
    scene.view_settings.gamma = 1

    # The CRT colours change every frame; keep only fixed sconces in the bake.
    for obj in bpy.data.objects:
        if obj.type == "LIGHT":
            obj.hide_render = "sconce" not in obj.name or "_bulb" not in obj.name
            if not obj.hide_render:
                obj.data.use_shadow = True

    bpy.ops.object.select_all(action="DESELECT")
    atlas_mesh.select_set(True)
    bpy.context.view_layer.objects.active = atlas_mesh
    bpy.ops.object.bake(type="DIFFUSE")

    output.mkdir(parents=True, exist_ok=True)
    atlas_path = output / "room_static_lightmap.png"
    image.filepath_raw = str(atlas_path)
    image.save()

    return atlas_path


def export_static(atlas_mesh: bpy.types.Object, output: Path) -> Path:
    # Keep the target image node in the saved .blend for inspection, but omit
    # the disconnected bake-only node from the glTF material export.
    for slot in atlas_mesh.material_slots:
        material = slot.material
        if material and material.node_tree:
            node = material.node_tree.nodes.get("__spec_chum_lightmap_bake_target__")
            if node:
                material.node_tree.nodes.remove(node)
    atlas_mesh.data.uv_layers.active = atlas_mesh.data.uv_layers[0]
    atlas_mesh.data.uv_layers[0].active_render = True
    bpy.ops.object.select_all(action="DESELECT")
    atlas_mesh.select_set(True)
    bpy.context.view_layer.objects.active = atlas_mesh
    gltf_path = output / "room_static.gltf"
    bpy.ops.export_scene.gltf(
        filepath=str(gltf_path),
        export_format="GLTF_SEPARATE",
        export_lights=False,
        export_extras=False,
        export_texcoords=True,
        export_texture_dir="textures",
        use_selection=True,
    )
    return gltf_path


def validate_export(gltf_path: Path, atlas_path: Path) -> None:
    document = json.loads(gltf_path.read_text())
    meshes = [
        mesh for mesh in document.get("meshes", [])
        if mesh.get("name", "").startswith("spec_chum_room_static_baked")
    ]
    primitives = [primitive for mesh in meshes for primitive in mesh.get("primitives", [])]
    if not primitives or not all(
        "TEXCOORD_1" in primitive.get("attributes", {})
        for primitive in primitives
    ):
        raise RuntimeError("export is missing the baked mesh or TEXCOORD_1")
    for item in document.get("buffers", []) + document.get("images", []):
        uri = item.get("uri", "")
        if uri and not uri.startswith("data:") and not (gltf_path.parent / uri).is_file():
            raise RuntimeError(f"glTF dependency missing: {uri}")
    if not atlas_path.is_file() or atlas_path.stat().st_size == 0:
        raise RuntimeError("lightmap atlas was not written")
    total = sum(path.stat().st_size for path in gltf_path.parent.rglob("*") if path.is_file())
    print(f"Generated asset package: {total / (1024 * 1024):.1f} MiB")


def main() -> None:
    assets, output, size, samples, threads = parse_args()
    if not (assets / "polyhaven/models").is_dir():
        raise FileNotFoundError(f"Poly Haven models missing under {assets}")
    room.clear_scene()
    bpy.context.scene.unit_settings.system = "METRIC"
    room.build_room(assets)
    missing_images = [
        f"{image.name}: {bpy.path.abspath(image.filepath)}"
        for image in bpy.data.images
        if image.source == "FILE"
        and not Path(bpy.path.abspath(image.filepath)).is_file()
    ]
    if missing_images:
        raise RuntimeError("missing source textures:\n" + "\n".join(missing_images))
    atlas_mesh = make_atlas_mesh()
    atlas_path = bake(atlas_mesh, output, size, samples, threads)
    # Local editor handoff: inspect UV1, the image and the glTF side by side.
    # .blend files are ignored; the reproducible source is this script.
    blend_path = output / "room_static_bake.blend"
    bpy.ops.file.pack_all()
    bpy.ops.wm.save_as_mainfile(filepath=str(blend_path))
    print(f"BLENDER_BAKE_PROJECT {blend_path}")
    gltf_path = export_static(atlas_mesh, output)
    validate_export(gltf_path, atlas_path)
    print(f"LIGHTMAP_BAKE_OK {gltf_path} {atlas_path}")


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:
        print(f"LIGHTMAP_BAKE_FAILED: {exc}", file=sys.stderr)
        raise
