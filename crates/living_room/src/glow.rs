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
#[derive(Component, Debug)]
pub struct GlowDriven {
    pub intensity_scale: f32,
}

/// Primary fill near the phosphor face (also tracks phosphor transform).
#[derive(Component, Debug)]
pub struct CrtFillLight;

/// Constant warm room lamp — not CRT-tinted (stays tungsten).
/// Reserved for future fixture tagging; wall sconces currently carry their own lights.
#[derive(Component, Debug)]
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

fn spawn_fill_lights(mut commands: Commands) {
    let bright = crate::crt::bright_debug_enabled();
    let hide_crt = crate::crt::hide_crt_enabled();
    // Bright-debug multiplies ambient so bezels + punch edges read in screenshots
    // (aperture debug also skips CRT spill, which otherwise darkens the room).
    let ambient_mul = if bright { 14.0 } else { 1.0 };

    if hide_crt {
        bevy::log::info!("SPEC_CHUM_ROOM_HIDE_CRT: skipping CRT spill lights");
    } else {
        let min = quality::light_preset() == LightPreset::Min;
        // Primary CRT spill — phosphor-driven colour via GlowDriven.
        commands.spawn((
            PointLight {
                color: Color::srgb(0.4, 0.45, 0.35),
                // Placeholder until first FrameGlow sync (~1.1k–4.0k lm).
                intensity: 1_800.0,
                range: 5.0,
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::from_xyz(0.0, 1.17, -1.15),
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
                bevy::camera::visibility::RenderLayers::layer(0).with(1),
                Name::new("crt_wall_bounce"),
            ));
        }
    } // end !hide_crt CRT spill lights

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

    // Room tungsten now comes from visible 1980s wall sconces in `room.rs`.
    // Keep only a tiny warm ambient here so furniture isn't pure black in CRT shadow.
    commands.insert_resource(GlobalAmbientLight {
        color: if bright {
            Color::srgb(0.55, 0.55, 0.58)
        } else {
            // Warm tungsten fill — readable furniture without CRT glare (#233).
            Color::srgb(0.26, 0.20, 0.13)
        },
        brightness: 76.5 * ambient_mul,
        ..default()
    });
}

fn sync_fill_origin(
    phosphor: Query<&GlobalTransform, With<CrtPhosphor>>,
    mut fill: Query<&mut Transform, With<CrtFillLight>>,
) {
    let origin = phosphor
        .iter()
        .next()
        .map(|g| g.translation() + Vec3::new(0.0, 0.0, 0.12))
        .unwrap_or(Vec3::new(0.0, 1.17, -1.15));

    for mut tf in &mut fill {
        tf.translation = origin;
    }
}

fn sync_glow_tints(
    glow: Res<FrameGlow>,
    mut points: Query<(&GlowDriven, &mut PointLight)>,
    mut spots: Query<(&GlowDriven, &mut SpotLight)>,
) {
    let tint = Color::from(glow.color);
    let base = glow.intensity;

    for (driven, mut light) in &mut points {
        light.color = tint;
        light.intensity = base * driven.intensity_scale;
    }
    for (driven, mut light) in &mut spots {
        light.color = tint;
        light.intensity = base * driven.intensity_scale;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
