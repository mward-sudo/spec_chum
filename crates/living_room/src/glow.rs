//! Dominant-colour fill + fake wall-bounce lights driven by the Spectrum framebuffer.

use bevy::prelude::*;

use crate::crt::CrtPhosphor;
use crate::quality::{self, LightPreset};

#[derive(Resource, Debug, Default)]
pub struct FrameGlow {
    pub color: LinearRgba,
    pub intensity: f32,
}

impl FrameGlow {
    pub fn update_from_rgba(&mut self, rgba: &[u8], width: u32, height: u32) {
        let mut r = 0u64;
        let mut g = 0u64;
        let mut b = 0u64;
        let mut n = 0u64;
        // Sparse sample for speed.
        let step = 8usize;
        let w = width as usize;
        let h = height as usize;
        for y in (0..h).step_by(step) {
            for x in (0..w).step_by(step) {
                let i = (y * w + x) * 4;
                if i + 2 >= rgba.len() {
                    continue;
                }
                r += u64::from(rgba[i]);
                g += u64::from(rgba[i + 1]);
                b += u64::from(rgba[i + 2]);
                n += 1;
            }
        }
        if n == 0 {
            return;
        }
        let rf = (r as f32 / n as f32) / 255.0;
        let gf = (g as f32 / n as f32) / 255.0;
        let bf = (b as f32 / n as f32) / 255.0;
        let lum = 0.2126 * rf + 0.7152 * gf + 0.0722 * bf;
        self.color = LinearRgba::rgb(rf.max(0.02), gf.max(0.02), bf.max(0.02));
        // Soft room spill only — high values wash the phosphor via bloom when
        // zoomed out (#233). CRT emissive carries the tube; spill tints walls.
        self.intensity = 1_080.0 + lum * 2_880.0;
    }
}

/// Tint + intensity scale driven by [`FrameGlow`].
///
/// Default `intensity_scale` is `1.0` so Skein/Blender-inserted markers drive
/// full phosphor spill until an explicit scale is authored.
#[derive(Component, Reflect, Debug)]
#[reflect(Component, Default)]
pub struct GlowDriven {
    pub intensity_scale: f32,
}

impl Default for GlowDriven {
    fn default() -> Self {
        Self {
            intensity_scale: 1.0,
        }
    }
}

/// Warm ambient brightness for procedural Current (matches historical glow baseline).
#[must_use]
pub fn procedural_ambient_brightness(bright_debug: bool) -> f32 {
    76.5 * if bright_debug { 14.0 } else { 1.0 }
}

/// Softer fallback ambient when a Skein room owns lighting.
#[must_use]
pub fn skein_fallback_ambient_brightness(bright_debug: bool) -> f32 {
    40.0 * if bright_debug { 14.0 } else { 1.0 }
}

/// Primary fill near the phosphor face (also tracks phosphor transform).
#[derive(Component, Reflect, Debug, Clone, Copy, Default)]
#[reflect(Component, Default)]
pub struct CrtFillLight;

/// Constant warm room lamp — not CRT-tinted (stays tungsten).
/// Reserved for future fixture tagging; wall sconces currently carry their own lights.
#[derive(Component, Reflect, Debug, Clone, Copy, Default)]
#[reflect(Component, Default)]
// Marker reserved for future sconce tagging (#171 / living-room polish).
#[allow(dead_code)]
pub struct IncandescentLamp;

#[derive(Debug, Default)]
pub struct GlowPlugin;

impl Plugin for GlowPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FrameGlow>()
            .add_systems(Startup, spawn_fill_lights)
            .add_systems(Update, (sync_fill_origin, sync_glow_tints).chain());
    }
}

fn spawn_fill_lights(
    mut commands: Commands,
    #[cfg(feature = "skein")] skein_mode: Option<Res<crate::skein::SkeinRoomMode>>,
) {
    let bright = crate::crt::bright_debug_enabled();
    let hide_crt = crate::crt::hide_crt_enabled();
    // Bright-debug multiplies ambient so bezels + punch edges read in screenshots
    // (aperture debug also skips CRT spill, which otherwise darkens the room).
    let ambient_mul = if bright { 14.0 } else { 1.0 };

    #[cfg(feature = "skein")]
    let skein_owns_lights = skein_mode
        .as_deref()
        .is_some_and(crate::skein::SkeinRoomMode::replaces_procedural);
    #[cfg(not(feature = "skein"))]
    let skein_owns_lights = false;

    if skein_owns_lights {
        bevy::log::info!(
            "Skein room active: skipping procedural CRT fill / sconces (author lights in Blender; \
             tag GlowDriven / CrtFillLight for framebuffer-driven spill)"
        );
    } else if hide_crt {
        bevy::log::info!("SPEC_CHUM_ROOM_HIDE_CRT: skipping CRT spill lights");
    } else {
        let min = quality::light_preset() == LightPreset::Min;
        // Primary CRT spill — phosphor-driven colour via GlowDriven.
        // Keep the emitter off the camera↔CRT axis so mirror glass does not pick up
        // a dead-centre specular hotspot (#149); room spill still reads from the tube.
        commands.spawn((
            PointLight {
                color: Color::srgb(0.4, 0.45, 0.35),
                // Placeholder until first FrameGlow sync (~1.1k–4.0k lm).
                intensity: 1_800.0,
                range: 5.0,
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::from_translation(crt_fill_offset(Vec3::new(0.0, 1.17, -1.15))),
            GlowDriven {
                intensity_scale: 1.0,
            },
            CrtFillLight,
            bevy::camera::visibility::RenderLayers::layer(0).with(1),
            Name::new("crt_fill_light"),
        ));
        if !min {
            commands.spawn((
                PointLight {
                    color: Color::srgb(0.35, 0.38, 0.32),
                    intensity: 810.0,
                    range: 7.0,
                    shadow_maps_enabled: false,
                    ..default()
                },
                Transform::from_xyz(0.0, 1.55, -1.55),
                GlowDriven {
                    intensity_scale: 0.45,
                },
                // Temporary #149: New suppresses wall-bounce only (sconces stay lit).
                crate::scene_variant::DynamicRoomFillLight,
                bevy::camera::visibility::RenderLayers::layer(0).with(1),
                Name::new("crt_wall_bounce"),
            ));
        }
    } // end procedural CRT spill lights

    if bright {
        // Neutral key from the sofa / camera side so TV bezels and punch rim are visible.
        commands.spawn((
            SpotLight {
                color: Color::srgb(0.95, 0.95, 1.0),
                intensity: 28_000.0,
                range: 18.0,
                outer_angle: 1.15,
                inner_angle: 0.5,
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::from_xyz(0.0, 1.65, 1.55).looking_at(Vec3::new(0.0, 1.22, -1.35), Vec3::Y),
            Name::new("bright_debug_key"),
        ));
        commands.spawn((
            PointLight {
                color: Color::srgb(0.9, 0.92, 1.0),
                intensity: 12_000.0,
                range: 16.0,
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::from_xyz(0.35, 1.9, -0.2),
            Name::new("bright_debug_fill"),
        ));
        bevy::log::info!(
            "SPEC_CHUM_ROOM_BRIGHT_DEBUG/APERTURE_DEBUG: boosted ambient×{ambient_mul} + key/fill"
        );
    }

    // Room tungsten: procedural sconces in `room.rs`, or Blender lights in Skein room mode.
    // Keep a tiny warm ambient so furniture isn't pure black in CRT shadow.
    commands.insert_resource(GlobalAmbientLight {
        color: if bright {
            Color::srgb(0.55, 0.55, 0.58)
        } else {
            // Warm tungsten fill — readable furniture without CRT glare (#233).
            Color::srgb(0.26, 0.20, 0.13)
        },
        brightness: if skein_owns_lights {
            skein_fallback_ambient_brightness(bright)
        } else {
            procedural_ambient_brightness(bright)
        },
        ..default()
    });
}

/// World offset of the CRT fill emitter relative to the phosphor centre.
///
/// Fully on-axis +Z whitened the aperture; a large off-axis bias killed glass read.
/// Mild right/down/into-room offset keeps a soft edge sheen without a centre blob (#149).
fn crt_fill_offset(phosphor: Vec3) -> Vec3 {
    phosphor + Vec3::new(0.18, -0.08, 0.22)
}

fn sync_fill_origin(
    phosphor: Query<&GlobalTransform, With<CrtPhosphor>>,
    mut fill: Query<&mut Transform, With<CrtFillLight>>,
) {
    let origin = phosphor.iter().next().map_or_else(
        || crt_fill_offset(Vec3::new(0.0, 1.17, -1.15)),
        |g| crt_fill_offset(g.translation()),
    );

    for mut tf in &mut fill {
        tf.translation = origin;
    }
}

fn sync_glow_tints(
    glow: Res<FrameGlow>,
    variant: Res<crate::scene_variant::SceneVariant>,
    mut points: Query<(
        &GlowDriven,
        &mut PointLight,
        Option<&crate::scene_variant::DynamicRoomFillLight>,
    )>,
    mut spots: Query<(
        &GlowDriven,
        &mut SpotLight,
        Option<&crate::scene_variant::DynamicRoomFillLight>,
    )>,
) {
    let tint = Color::from(glow.color);
    let base = glow.intensity;
    // Temporary #149: New keeps CRT spill + sconces; zeros GlowDriven wall-bounce only.
    let suppress_wall_bounce = matches!(*variant, crate::scene_variant::SceneVariant::New);

    for (driven, mut light, dynamic) in &mut points {
        if suppress_wall_bounce && dynamic.is_some() {
            light.intensity = 0.0;
            continue;
        }
        light.color = tint;
        light.intensity = base * driven.intensity_scale;
    }
    for (driven, mut light, dynamic) in &mut spots {
        if suppress_wall_bounce && dynamic.is_some() {
            light.intensity = 0.0;
            continue;
        }
        light.color = tint;
        light.intensity = base * driven.intensity_scale;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glow_driven_default_is_unit_scale() {
        assert!((GlowDriven::default().intensity_scale - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn red_border_dominates_glow() {
        let mut glow = FrameGlow::default();
        // 8x8 solid red.
        let mut rgba = vec![0u8; 8 * 8 * 4];
        for px in rgba.as_chunks_mut::<4>().0 {
            px[0] = 255;
            px[3] = 255;
        }
        glow.update_from_rgba(&rgba, 8, 8);
        assert!(glow.color.red > glow.color.green);
        assert!(glow.color.red > glow.color.blue);
        assert!(glow.intensity > 1_080.0);
    }
}
