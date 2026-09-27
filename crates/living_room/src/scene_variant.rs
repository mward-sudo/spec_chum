//! Temporary #149 Current ↔ New scene A/B harness.
//!
//! SpecChumMac exposes a toolbar toggle that calls [`crate::ffi::sc_room_set_scene_variant`].
//! **Mac living-room path only** — not egui / Windows / Linux shells.
//!
//! Remove this module, the FFI, the SpecChumMac button, and the New WIP markers once
//! Blender lightmaps + Bevy `Lightmap` / `EnvironmentMapLight` land and #149 closes.

use bevy::prelude::*;

use crate::camera::{LivingRoomCamera, OpeningSequence};
use crate::quality;
use crate::room::{OpeningSconceBulb, OpeningSconceLight, OpeningTvAccent};

const NEW_ENVIRONMENT_MAP_INTENSITY: f32 = 125.0;

/// Which living-room lighting/scene path is active.
///
/// - [`Current`](Self::Current) — pre-#149 baseline (dynamic PBR fill + sconces).
/// - [`New`](Self::New) — Blender lightmap when baked assets are available;
///   otherwise the earlier IBL + cyan-strip comparison stub.
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
/// - Sconce bulbs (no [`crate::glow::GlowDriven`]): disabled on New with a bake.
/// - CRT wall-bounce ([`crate::glow::GlowDriven`]): live with a bake, suppressed
///   only on the older New stub.
///
/// Temporary while the Current/New comparison remains (#149).
#[derive(Component, Reflect, Debug, Clone, Copy, Default)]
#[reflect(Component, Default)]
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
                (prepare_new_environment_map, spawn_new_variant_wip_marker),
            )
            .add_systems(
                Update,
                (
                    apply_scene_variant.run_if(resource_exists::<NewVariantEnvironmentMap>),
                    animate_opening_lighting,
                )
                    .chain(),
            );
    }
}

#[allow(clippy::too_many_arguments)]
fn animate_opening_lighting(
    opening: Res<OpeningSequence>,
    variant: Res<SceneVariant>,
    baked: Option<Res<crate::baked_room::BakedRoomEnabled>>,
    mut ambient: ResMut<GlobalAmbientLight>,
    mut ambient_base: Local<Option<f32>>,
    mut sconces: Query<(
        &OpeningSconceLight,
        &mut PointLight,
        Option<&OpeningTvAccent>,
    )>,
    mut environment_maps: Query<&mut EnvironmentMapLight, With<LivingRoomCamera>>,
    bulbs: Query<(
        &OpeningSconceBulb,
        &MeshMaterial3d<StandardMaterial>,
        Option<&OpeningTvAccent>,
    )>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // `apply_scene_variant` sets the full-strength baseline before this system
    // scales it. Refresh the baseline when the user switches scene variants.
    if ambient_base.is_none() || variant.is_changed() {
        *ambient_base = Some(ambient.brightness);
    }
    let room_gain = opening.light_gain();
    let practical_gain = opening.practical_light_gain();
    if let Some(base) = *ambient_base {
        ambient.brightness = base * room_gain;
    }

    let baked_sconce_scale = if baked.is_some() && *variant == SceneVariant::New {
        0.4
    } else {
        1.0
    };
    let tv_accent_gain = (opening.tv_spot_gain() / 5_600.0).clamp(0.0, 1.0);
    for (sconce, mut light, accent) in &mut sconces {
        let gain = if accent.is_some() {
            practical_gain.max(tv_accent_gain)
        } else {
            practical_gain
        };
        light.intensity = sconce.0 * baked_sconce_scale * gain;
    }
    for (bulb, material, accent) in &bulbs {
        let gain = if accent.is_some() {
            practical_gain.max(tv_accent_gain)
        } else {
            practical_gain
        };
        let Some(mut material) = materials.get_mut(&material.0) else {
            continue;
        };
        material.emissive = LinearRgba::rgb(bulb.0.x * gain, bulb.0.y * gain, bulb.0.z * gain);
    }
    // New's camera cubemap is a separate illumination path from GlobalAmbientLight.
    // Fade it with the same curve so it cannot reveal the baked room in one step.
    let environment_gain = if *variant == SceneVariant::New {
        room_gain
    } else {
        0.0
    };
    for mut environment in &mut environment_maps {
        environment.intensity = NEW_ENVIRONMENT_MAP_INTENSITY * environment_gain;
    }
}

/// Build a 1×1×6 stub cubemap via Bevy's hemispherical helper (no HDR asset yet).
fn prepare_new_environment_map(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    opening: Res<OpeningSequence>,
) {
    // Warm, muted hemispheres — subtle IBL fill for New without high-key wash.
    // (Earlier 2400 intensity + bright cyan sky blew Exposure ~8.2 into washout.)
    // Soft upper lobe: enough for a gentle glass sheen with moderate CRT glass,
    // not a mirrored sky disc (#149).
    let mut light = EnvironmentMapLight::hemispherical_gradient(
        &mut images,
        Color::srgb(0.07, 0.16, 0.24), // muted cool upper
        Color::srgb(0.50, 0.40, 0.28), // warm horizon
        Color::srgb(0.16, 0.11, 0.06), // dark warm floor bounce
    );
    // Indoor Exposure ~8.2: soft room fill; sconces + cream ambient carry mood.
    // Seed the resource at the opening gain too, so its first insertion onto
    // the camera cannot contribute a full-strength frame before animation runs.
    light.intensity = NEW_ENVIRONMENT_MAP_INTENSITY * opening.light_gain();
    commands.insert_resource(NewVariantEnvironmentMap(light));
}

/// Cyan emissive strip on the TV wall — only visible in New so the toggle is obvious.
fn spawn_new_variant_wip_marker(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    variant: Res<SceneVariant>,
) {
    // A real bake has an obvious A/B difference without this temporary cue.
    let asset_root = crate::resolve_asset_root();
    if asset_root.join("lightmaps/room_static.gltf").is_file()
        && asset_root
            .join("lightmaps/room_static_lightmap.png")
            .is_file()
    {
        return;
    }
    let mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.12, 0.55, 0.65),
        // Dimmer than the first stub — cue only, not another wash source.
        emissive: LinearRgba::rgb(0.15, 0.7, 0.9),
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

// Bevy Queries + resources for Current/New room lighting (#149).
#[allow(clippy::too_many_arguments)]
fn apply_scene_variant(
    mut commands: Commands,
    variant: Res<SceneVariant>,
    env_map: Res<NewVariantEnvironmentMap>,
    baked: Option<Res<crate::baked_room::BakedRoomEnabled>>,
    mut ambient: ResMut<GlobalAmbientLight>,
    mut only: Query<(&SceneVariantOnly, &mut Visibility)>,
    cams: Query<Entity, With<LivingRoomCamera>>,
    // Sconce bulbs (no GlowDriven). CRT wall-bounce is handled in glow::sync_glow_tints.
    mut fill_lights: Query<
        &mut PointLight,
        (With<DynamicRoomFillLight>, Without<crate::glow::GlowDriven>),
    >,
    mut fill_intensity: Local<Vec<f32>>,
    mut fill_bases_ready: Local<bool>,
    #[cfg(feature = "skein")] skein_mode: Option<Res<crate::skein::SkeinRoomMode>>,
) {
    // With zero DynamicRoomFillLight entities (e.g. Skein room), fill_intensity stays
    // empty — track init separately so we do not re-apply EnvMap every frame.
    if !variant.is_changed() && *fill_bases_ready {
        return;
    }

    if !*fill_bases_ready {
        fill_intensity.extend(fill_lights.iter().map(|l| l.intensity));
        *fill_bases_ready = true;
    }

    #[cfg(feature = "skein")]
    let skein_owns_lights = skein_mode
        .as_deref()
        .is_some_and(crate::skein::SkeinRoomMode::replaces_procedural);
    #[cfg(not(feature = "skein"))]
    let skein_owns_lights = false;

    match *variant {
        SceneVariant::Current => {
            // Match glow.rs ambient — preserve SPEC_CHUM_ROOM_BRIGHT_DEBUG.
            // When Skein owns the room, keep the softer fallback (do not re-inflate).
            let bright = crate::crt::bright_debug_enabled();
            ambient.color = if bright {
                Color::srgb(0.55, 0.55, 0.58)
            } else {
                Color::srgb(0.26, 0.20, 0.13)
            };
            ambient.brightness = if skein_owns_lights {
                crate::glow::skein_fallback_ambient_brightness(bright)
            } else {
                crate::glow::procedural_ambient_brightness(bright)
            };
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
            // Moodier cream-warm vs Current tungsten. Keep live sconce fill while
            // the Blender atlas is still under-lit; the baked map alone made the
            // room almost black in the New variant.
            let bright = crate::crt::bright_debug_enabled();
            ambient.color = if bright {
                Color::srgb(0.58, 0.54, 0.48)
            } else {
                Color::srgb(0.30, 0.24, 0.16)
            };
            ambient.brightness = if baked.is_some() {
                // Support the live television and the room surfaces while the
                // point lights provide the visible, localized sconce pools.
                38.0 * if bright { 14.0 } else { 1.0 }
            } else if skein_owns_lights {
                crate::glow::skein_fallback_ambient_brightness(bright)
            } else {
                74.0 * if bright { 14.0 } else { 1.0 }
            };
            for (i, mut light) in fill_lights.iter_mut().enumerate() {
                if let Some(&base) = fill_intensity.get(i) {
                    // Retain a reduced live sconce contribution until the baked
                    // atlas has enough energy to light the room on its own.
                    light.intensity = if baked.is_some() { base * 0.4 } else { base };
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
