//! Opt-in [Skein](https://bevyskein.dev/) bridge for Blender ↔ Bevy scene editing.
//!
//! Enabled only with `--features skein` on the standalone `spec-chum-room` harness.
//! SpecChumMac builds with `--no-default-features` and never links this path.
//!
//! When a configured export exists, that glTF **is the room** (procedural
//! `room.rs` / glow fill lights are skipped). CRT phosphor still attaches in
//! code to a Blender-tagged [`TelevisionCabinet`].

use std::path::PathBuf;

use bevy::prelude::*;
use bevy_skein::{SkeinAppExt, SkeinPlugin};

use crate::glow::{CrtFillLight, GlowDriven, IncandescentLamp};
use crate::hybrid::{LiveTv, RoomStatic};
use crate::room::TelevisionCabinet;
use crate::scene_variant::DynamicRoomFillLight;

/// Default Bevy asset path for a Skein export (under `crates/living_room/assets/`).
pub const DEFAULT_SKEIN_SCENE: &str = "skein/living_room_edit.gltf#Scene0";

/// Resolved Skein room choice for this process.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub enum SkeinRoomMode {
    /// Feature on but forced off (`SPEC_CHUM_ROOM_SKEIN_SCENE=off`) or no file —
    /// use procedural `room.rs`.
    Procedural,
    /// Load this Bevy asset label as the full room (skip procedural spawn).
    Room(String),
}

impl SkeinRoomMode {
    /// Whether procedural room meshes / glow fill lights should be skipped.
    #[must_use]
    pub fn replaces_procedural(&self) -> bool {
        matches!(self, Self::Room(_))
    }
}

/// Wire Skein into the standalone living-room app (BRP always on for release).
///
/// `SkeinPlugin::default()` only enables BRP under `debug_assertions`, but this
/// crate is normally built `--release` (Bevy debug is multi‑GB). Force
/// `handle_brp: true` so Blender can fetch the type registry at `127.0.0.1:15702`.
pub fn add_skein_plugins(app: &mut App) {
    let mode = resolve_skein_room_mode();
    match &mode {
        SkeinRoomMode::Room(label) => {
            bevy::log::info!(
                "Skein room mode: loading {label} as the room (procedural room.rs skipped)"
            );
        }
        SkeinRoomMode::Procedural => {
            bevy::log::info!(
                "Skein feature on — procedural room (no export / SPEC_CHUM_ROOM_SKEIN_SCENE=off)"
            );
        }
    }

    app.insert_resource(mode)
        .add_plugins(SkeinPlugin { handle_brp: true })
        .register_type::<TelevisionCabinet>()
        .register_type::<RoomStatic>()
        .register_type::<LiveTv>()
        .register_type::<CrtFillLight>()
        .register_type::<IncandescentLamp>()
        .register_type::<GlowDriven>()
        .register_type::<DynamicRoomFillLight>();

    // Presets show up in Blender's Skein UI for quick light authoring.
    app.insert_skein_preset(
        "CRT fill (soft)",
        PointLight {
            color: Color::srgb(0.4, 0.45, 0.35),
            intensity: 1_800.0,
            range: 5.0,
            shadow_maps_enabled: false,
            ..default()
        },
    )
    .insert_skein_preset(
        "Warm sconce",
        PointLight {
            color: Color::srgb(1.0, 0.82, 0.55),
            intensity: 2_400.0,
            range: 4.5,
            shadow_maps_enabled: false,
            ..default()
        },
    );

    bevy::log::info!(
        "Skein BRP registry at http://127.0.0.1:15702 — fetch from Blender, \
         export full room to assets/skein/ (see docs/LIVING_ROOM.md)"
    );
}

/// Resolve [`SkeinRoomMode`] from `SPEC_CHUM_ROOM_SKEIN_SCENE` + on-disk export.
///
/// Presence of the glTF file selects Room mode immediately (Bevy then loads the
/// asset). We intentionally do **not** wait for a successful scene load before
/// skipping procedural spawn — that would require a dual-room fallback path and
/// is out of scope for the editor opt-in. Missing/broken loads leave an empty
/// room; use `SPEC_CHUM_ROOM_SKEIN_SCENE=off` to force procedural.
///
/// | Env | Behaviour |
/// | --- | --- |
/// | unset / empty | [`DEFAULT_SKEIN_SCENE`] if file exists → Room; else Procedural |
/// | `off` / `0` / `false` / `no` | Procedural |
/// | other path | that asset label if file exists → Room; else Procedural + warn |
#[must_use]
pub fn resolve_skein_room_mode() -> SkeinRoomMode {
    let raw = std::env::var("SPEC_CHUM_ROOM_SKEIN_SCENE").unwrap_or_default();
    let trimmed = raw.trim();
    if matches!(trimmed, "0" | "off" | "false" | "no") {
        return SkeinRoomMode::Procedural;
    }

    let label = if trimmed.is_empty() {
        DEFAULT_SKEIN_SCENE.to_owned()
    } else {
        normalize_scene_label(trimmed)
    };

    let file_part = label.split('#').next().unwrap_or(label.as_str());
    let on_disk = asset_file_path(file_part);
    if on_disk.is_file() {
        SkeinRoomMode::Room(label)
    } else {
        if !trimmed.is_empty() {
            bevy::log::warn!(
                "SPEC_CHUM_ROOM_SKEIN_SCENE={trimmed}: file missing at {} — procedural room",
                on_disk.display()
            );
        }
        SkeinRoomMode::Procedural
    }
}

fn normalize_scene_label(raw: &str) -> String {
    if raw.contains('#') {
        raw.to_owned()
    } else {
        format!("{raw}#Scene0")
    }
}

fn asset_file_path(file_part: &str) -> PathBuf {
    crate::resolve_asset_root().join(file_part)
}

/// Spawn the Skein-exported glTF as the living room.
///
/// Returns `true` when spawned (caller must skip procedural `setup_room` body).
pub fn spawn_skein_room_if_active(
    commands: &mut Commands,
    asset_server: &AssetServer,
    mode: &SkeinRoomMode,
) -> bool {
    let SkeinRoomMode::Room(label) = mode else {
        return false;
    };

    bevy::log::info!("Spawning Skein room scene: {label}");
    commands.spawn((
        WorldAssetRoot(asset_server.load(label.clone())),
        Name::new("skein_room"),
    ));
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_skein_scene_path_is_under_assets_skein() {
        assert!(DEFAULT_SKEIN_SCENE.starts_with("skein/"));
        assert!(DEFAULT_SKEIN_SCENE.contains(".gltf"));
    }

    #[test]
    fn normalize_appends_scene0() {
        assert_eq!(
            normalize_scene_label("skein/foo.gltf"),
            "skein/foo.gltf#Scene0"
        );
        assert_eq!(
            normalize_scene_label("skein/foo.gltf#Scene1"),
            "skein/foo.gltf#Scene1"
        );
    }

    #[test]
    fn room_mode_replaces_procedural() {
        assert!(!SkeinRoomMode::Procedural.replaces_procedural());
        assert!(SkeinRoomMode::Room("x".into()).replaces_procedural());
    }

    #[test]
    fn asset_file_path_joins_root() {
        let p = asset_file_path("skein/living_room_edit.gltf");
        assert!(p.to_string_lossy().ends_with("skein/living_room_edit.gltf"));
    }
}
