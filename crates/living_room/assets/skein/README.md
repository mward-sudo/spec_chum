# Skein room exports (local / optional)

With `--features skein`, if this file exists it **replaces** the procedural room:

```text
living_room_edit.gltf
living_room_edit.bin   # if separate
living_room_edit.blend # editable starter (optional)
```

## Generate starter files

```bash
./scripts/fetch_living_room_assets.sh   # once
./scripts/generate_living_room_blend.sh
```

Then open `living_room_edit.blend`, edit, export glTF here (or re-run the generator).
See `docs/LIVING_ROOM.md` → *Scene editing with Skein*.

Tag the CRT cabinet empty (`television_02`) with Bevy component `TelevisionCabinet`
before a Blender re-export (the generator’s tagger already injects it on first write).

| Env | Effect |
| --- | --- |
| (unset) | Use `living_room_edit.gltf` when present; else procedural |
| `SPEC_CHUM_ROOM_SKEIN_SCENE=skein/other.gltf` | That export as the room |
| `SPEC_CHUM_ROOM_SKEIN_SCENE=off` | Force procedural |

Do not commit large binaries / `.blend` here (gitignored). See `docs/LIVING_ROOM.md`.
