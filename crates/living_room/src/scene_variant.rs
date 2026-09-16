//! Temporary #149 Current ↔ New scene A/B harness.
//!
//! SpecChumMac exposes a toolbar toggle that calls [`crate::ffi::sc_room_set_scene_variant`].
//! **Mac living-room path only** — not egui / Windows / Linux shells.
//!
//! Remove this module, the FFI, the SpecChumMac button, and the New WIP marker once
//! Blender lightmaps + Bevy `Lightmap` / `EnvironmentMapLight` land and #149 closes.

use bevy::prelude::*;

use crate::quality;

/// Which living-room lighting/scene path is active.
///
/// - [`Current`](Self::Current) — pre-#149 baseline (dynamic PBR fill + sconces).
/// - [`New`](Self::New) — lightmap WIP stub (brighter warm ambient + lit sconces +
///   cyan marker; wall-bounce still suppressed as the lightmap stand-in).
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SceneVariant {
    #[default]
    Current = 0,
    New = 1,
}

impl SceneVariant {
    pub fn from_u32(v: u32) -> Option<Self> {
        match v {
            0 => Some(Self::Current),
            1 => Some(Self::New),
            _ => None,
        }
    }

    pub fn as_u32(self) -> u32 {
        self as u32
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::New => "new",
        }
    }
}

/// Entity visible only while [`SceneVariant`] matches.
#[derive(Component, Debug, Clone, Copy)]
pub struct SceneVariantOnly(pub SceneVariant);

/// Dynamic room fill tagged for the #149 A/B harness.
///
/// - Sconce bulbs (no [`crate::glow::GlowDriven`]): stay lit on both Current and New.
/// - CRT wall-bounce ([`crate::glow::GlowDriven`]): still suppressed on New as the
///   lightmap stub until baked fill lands.
///
/// Temporary until baked lightmaps replace these lights (#149).
#[derive(Component, Debug, Clone, Copy)]
pub struct DynamicRoomFillLight;

#[derive(Debug, Default)]
pub struct SceneVariantPlugin;

impl Plugin for SceneVariantPlugin {
    fn build(&self, app: &mut App) {
        let initial = quality::scene_variant();
        bevy::log::info!(
            "SPEC_CHUM_ROOM_SCENE: variant={} (temporary #149 A/B; remove when lightmaps ship)",
            initial.label()
        );
        app.insert_resource(initial)
            .add_systems(Startup, spawn_new_variant_wip_marker)
            .add_systems(Update, apply_scene_variant);
    }
}

/// Cyan emissive strip on the TV wall — only visible in New so the toggle is obvious.
fn spawn_new_variant_wip_marker(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    variant: Res<SceneVariant>,
) {
    let mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.15, 0.85, 0.95),
        emissive: LinearRgba::rgb(0.4, 2.2, 2.8),
        unlit: true,
        perceptual_roughness: 1.0,
        metallic: 0.0,
        ..default()
    });
    let visible = matches!(*variant, SceneVariant::New);
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(1.6, 0.08, 0.04))),
        MeshMaterial3d(mat),
        Transform::from_xyz(0.0, 2.05, -crate::room::ROOM_D * 0.5 + 0.08),
        if visible {
            Visibility::Visible
        } else {
            Visibility::Hidden
        },
        SceneVariantOnly(SceneVariant::New),
        crate::hybrid::RoomStatic,
        Name::new("scene_new_lightmap_wip_marker"),
    ));
}

fn apply_scene_variant(
    variant: Res<SceneVariant>,
    mut ambient: ResMut<GlobalAmbientLight>,
    mut only: Query<(&SceneVariantOnly, &mut Visibility)>,
    // Sconce bulbs (no GlowDriven). CRT wall-bounce is handled in glow::sync_glow_tints.
    mut fill_lights: Query<
        &mut PointLight,
        (With<DynamicRoomFillLight>, Without<crate::glow::GlowDriven>),
    >,
    mut fill_intensity: Local<Vec<f32>>,
) {
    if !variant.is_changed() && !fill_intensity.is_empty() {
        return;
    }

    if fill_intensity.is_empty() {
        fill_intensity.extend(fill_lights.iter().map(|l| l.intensity));
    }

    match *variant {
        SceneVariant::Current => {
            // Match glow.rs ambient — preserve SPEC_CHUM_ROOM_BRIGHT_DEBUG.
            let bright = crate::crt::bright_debug_enabled();
            ambient.color = if bright {
                Color::srgb(0.55, 0.55, 0.58)
            } else {
                Color::srgb(0.26, 0.20, 0.13)
            };
            ambient.brightness = 76.5 * if bright { 14.0 } else { 1.0 };
            for (i, mut light) in fill_lights.iter_mut().enumerate() {
                if let Some(&base) = fill_intensity.get(i) {
                    light.intensity = base;
                }
            }
        }
        SceneVariant::New => {
            // Warmer, brighter ambient stub — stand-in for EnvironmentMapLight (#149).
            // Distinct from Current (creamier tint + higher brightness) but not
            // cool/muddy; sconces stay lit (visual feedback on the A/B harness).
            let bright = crate::crt::bright_debug_enabled();
            ambient.color = if bright {
                Color::srgb(0.58, 0.54, 0.48)
            } else {
                // Cream-warm vs Current tungsten (0.26, 0.20, 0.13).
                Color::srgb(0.36, 0.28, 0.18)
            };
            ambient.brightness = 118.0 * if bright { 14.0 } else { 1.0 };
            for (i, mut light) in fill_lights.iter_mut().enumerate() {
                if let Some(&base) = fill_intensity.get(i) {
                    light.intensity = base;
                }
            }
        }
    }

    for (only_var, mut vis) in &mut only {
        *vis = if only_var.0 == *variant {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_variant_u32_roundtrip() {
        assert_eq!(SceneVariant::from_u32(0), Some(SceneVariant::Current));
        assert_eq!(SceneVariant::from_u32(1), Some(SceneVariant::New));
        assert_eq!(SceneVariant::from_u32(2), None);
        assert_eq!(SceneVariant::Current.as_u32(), 0);
        assert_eq!(SceneVariant::New.as_u32(), 1);
    }
}
