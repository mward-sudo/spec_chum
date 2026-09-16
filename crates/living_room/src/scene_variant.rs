//! Temporary #149 Current ↔ New scene A/B harness.
//!
//! SpecChumMac exposes a toolbar toggle that calls [`crate::ffi::sc_room_set_scene_variant`].
//! **Mac living-room path only** — not egui / Windows / Linux shells.
//!
//! Remove this module, the FFI, the SpecChumMac button, and the New WIP markers once
//! Blender lightmaps + Bevy `Lightmap` / `EnvironmentMapLight` land and #149 closes.

use bevy::prelude::*;

use crate::camera::LivingRoomCamera;
use crate::quality;

/// Which living-room lighting/scene path is active.
///
/// - [`Current`](Self::Current) — pre-#149 baseline (dynamic PBR fill + sconces).
/// - [`New`](Self::New) — lightmap WIP: cream ambient + sconces + stub
///   [`EnvironmentMapLight`] + cyan strip; wall-bounce still suppressed.
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

/// Procedural stub cubemap for New — real HDR bake comes later (#149).
#[derive(Resource, Clone)]
struct NewVariantEnvironmentMap(EnvironmentMapLight);

impl std::fmt::Debug for NewVariantEnvironmentMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NewVariantEnvironmentMap")
            .field("intensity", &self.0.intensity)
            .finish_non_exhaustive()
    }
}

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
            .add_systems(
                Startup,
                (
                    prepare_new_environment_map,
                    spawn_new_variant_wip_marker,
                    spawn_new_env_reflection_marker,
                ),
            )
            .add_systems(
                Update,
                apply_scene_variant.run_if(resource_exists::<NewVariantEnvironmentMap>),
            );
    }
}

/// Build a 1×1×6 stub cubemap via Bevy's hemispherical helper (no HDR asset yet).
fn prepare_new_environment_map(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    // Cyan-teal upper hemisphere vs cream horizon — reads on chrome without
    // washing the room (2400 blew Exposure ~8.2 + #435 ambient into high-key).
    let mut light = EnvironmentMapLight::hemispherical_gradient(
        &mut images,
        Color::srgb(0.10, 0.34, 0.50), // sky / upper fill (muted; avoids cool wash)
        Color::srgb(0.70, 0.58, 0.42), // cream horizon (keeps #435 warmth)
        Color::srgb(0.24, 0.18, 0.10), // warm floor bounce
    );
    // Indoor Exposure ~8.2: chrome marker still shows IBL; walls stay cream-warm
    // from ambient + sconces (420 still washed the zoomed-out CRT face).
    light.intensity = 180.0;
    commands.insert_resource(NewVariantEnvironmentMap(light));
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

/// Small chrome sphere — New only — so env-map specular reads at a glance.
fn spawn_new_env_reflection_marker(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    variant: Res<SceneVariant>,
) {
    let mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.92, 0.92, 0.95),
        perceptual_roughness: 0.08,
        metallic: 1.0,
        reflectance: 1.0,
        ..default()
    });
    let visible = matches!(*variant, SceneVariant::New);
    commands.spawn((
        Mesh3d(meshes.add(Sphere::new(0.09).mesh().uv(32, 18))),
        MeshMaterial3d(mat),
        // Right of TV, mid-height — catches cyan sky + cream horizon from the stub IBL.
        Transform::from_xyz(0.85, 1.35, -crate::room::ROOM_D * 0.5 + 0.35),
        if visible {
            Visibility::Visible
        } else {
            Visibility::Hidden
        },
        SceneVariantOnly(SceneVariant::New),
        crate::hybrid::RoomStatic,
        Name::new("scene_new_envmap_reflection_marker"),
    ));
}

/// Baseline CRT glass PBR (Current) vs New stub-IBL-safe values.
///
/// Mirror-like glass (`reflectance` 1.0 / roughness 0.04) turns stub IBL + bloom into a
/// white center blob and washes the phosphor face — dial only on New. Bevy maps
/// reflectance 0 → no specular; keep New glass fully rough so the env map cannot
/// mirror a hot highlight onto the bulging CRT face when zoomed out.
const CRT_GLASS_REFLECTANCE_CURRENT: f32 = 1.0;
const CRT_GLASS_ROUGHNESS_CURRENT: f32 = 0.04;
const CRT_GLASS_REFLECTANCE_NEW: f32 = 0.0;
const CRT_GLASS_ROUGHNESS_NEW: f32 = 1.0;

// Bevy Queries + resources for Current/New lighting + CRT glass IBL dial (#149).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn apply_scene_variant(
    mut commands: Commands,
    variant: Res<SceneVariant>,
    env_map: Res<NewVariantEnvironmentMap>,
    mut ambient: ResMut<GlobalAmbientLight>,
    mut only: Query<(&SceneVariantOnly, &mut Visibility)>,
    cams: Query<Entity, With<LivingRoomCamera>>,
    // Sconce bulbs (no GlowDriven). CRT wall-bounce is handled in glow::sync_glow_tints.
    mut fill_lights: Query<
        &mut PointLight,
        (With<DynamicRoomFillLight>, Without<crate::glow::GlowDriven>),
    >,
    glass: Query<&MeshMaterial3d<StandardMaterial>, With<crate::crt::CrtGlass>>,
    mut std_mats: ResMut<Assets<StandardMaterial>>,
    mut fill_intensity: Local<Vec<f32>>,
    mut glass_synced_for: Local<Option<SceneVariant>>,
) {
    // CRT glass may spawn a frame after the first variant apply — retry until tuned.
    let need_room = variant.is_changed() || fill_intensity.is_empty();
    let need_glass = glass_synced_for.as_ref() != Some(&*variant) && !glass.is_empty();
    if !need_room && !need_glass {
        return;
    }

    if need_room {
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
                for entity in &cams {
                    commands.entity(entity).remove::<EnvironmentMapLight>();
                }
            }
            SceneVariant::New => {
                // Keep #435 cream warmth + lit sconces; stub IBL adds fill so ambient
                // sits below the #435-only 118 brightness (avoids high-key wash).
                let bright = crate::crt::bright_debug_enabled();
                ambient.color = if bright {
                    Color::srgb(0.58, 0.54, 0.48)
                } else {
                    // Cream-warm vs Current tungsten (0.26, 0.20, 0.13).
                    Color::srgb(0.34, 0.27, 0.17)
                };
                ambient.brightness = 88.0 * if bright { 14.0 } else { 1.0 };
                for (i, mut light) in fill_lights.iter_mut().enumerate() {
                    if let Some(&base) = fill_intensity.get(i) {
                        light.intensity = base;
                    }
                }
                for entity in &cams {
                    commands.entity(entity).insert(env_map.0.clone());
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

    if need_glass || (need_room && !glass.is_empty()) {
        let (reflectance, roughness) = match *variant {
            SceneVariant::Current => (CRT_GLASS_REFLECTANCE_CURRENT, CRT_GLASS_ROUGHNESS_CURRENT),
            SceneVariant::New => (CRT_GLASS_REFLECTANCE_NEW, CRT_GLASS_ROUGHNESS_NEW),
        };
        for handle in &glass {
            if let Some(mut mat) = std_mats.get_mut(&handle.0) {
                mat.reflectance = reflectance;
                mat.perceptual_roughness = roughness;
            }
        }
        *glass_synced_for = Some(*variant);
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
