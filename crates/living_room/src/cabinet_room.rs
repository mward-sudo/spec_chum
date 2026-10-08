//! Fixed-camera, flat-room comparison for the Spectrum Cabinet prototype (#558).
//!
//! The backdrop is deliberately authored from simple Bevy rectangles so this
//! experiment has no external artwork dependency. The television cabinet and
//! phosphor remain live 3D entities from `room` and `crt`.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RoomPresentation {
    #[default]
    ThreeDimensional,
    FixedCabinet,
}

impl RoomPresentation {
    #[must_use]
    pub fn from_env_value(value: Option<&str>) -> Self {
        match value
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "fixed-cabinet" | "fixed_cabinet" | "2d" => Self::FixedCabinet,
            _ => Self::ThreeDimensional,
        }
    }

    #[must_use]
    pub fn from_environment() -> Self {
        Self::from_env_value(std::env::var("SPEC_CHUM_ROOM_PRESENTATION").ok().as_deref())
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::ThreeDimensional => "3d",
            Self::FixedCabinet => "fixed-cabinet",
        }
    }
}

/// Back wall sits behind the CRT/cabinet and extends beyond the 16:9 hero view.
pub const BACKDROP_CENTER_Y: f32 = 1.22;
pub const BACKDROP_DEPTH: f32 = crate::room::TV_STAND_POS.z - 0.78;
pub const HERO_CENTER_Y: f32 = 0.82;
pub const HERO_FRAME_W: f32 = 1.55;
pub const HERO_FRAME_H: f32 = 1.62;
pub const HERO_FRAME_FILL: f32 = 0.78;

#[derive(Component, Debug)]
struct CabinetBackdrop;

/// Spawn the flat, layered wall behind the live TV. Shapes are unlit so the
/// static composition remains legible while CRT spill is added independently.
pub fn spawn_backdrop(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
) {
    let wall = material(materials, Color::srgb(0.12, 0.16, 0.19));
    let inset = material(materials, Color::srgb(0.20, 0.22, 0.21));
    let curtain = material(materials, Color::srgb(0.20, 0.10, 0.10));
    let trim = material(materials, Color::srgb(0.30, 0.19, 0.11));
    let spill_mask = images.add(spill_mask());
    let screen_spill = materials.add(StandardMaterial {
        base_color: Color::srgba(0.42, 0.78, 1.0, 0.48),
        base_color_texture: Some(spill_mask),
        emissive: LinearRgba::rgb(0.025, 0.095, 0.18),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        ..default()
    });

    let wall_z = BACKDROP_DEPTH;
    spawn_panel(
        commands,
        meshes,
        wall.clone(),
        "cabinet_backdrop_wall",
        Vec3::new(0.0, BACKDROP_CENTER_Y, wall_z),
        Vec2::new(4.2, 2.45),
    );
    // Low-contrast wall panels frame, but do not compete with, the screen.
    spawn_panel(
        commands,
        meshes,
        inset,
        "cabinet_backdrop_wall_panel",
        Vec3::new(0.0, BACKDROP_CENTER_Y, wall_z + 0.012),
        Vec2::new(3.35, 1.95),
    );
    for x in [-1.72, 1.72] {
        spawn_panel(
            commands,
            meshes,
            curtain.clone(),
            "cabinet_backdrop_curtain",
            Vec3::new(x, 1.28, wall_z + 0.024),
            Vec2::new(0.46, 2.25),
        );
    }
    spawn_panel(
        commands,
        meshes,
        trim,
        "cabinet_backdrop_shelf",
        Vec3::new(0.0, 0.19, wall_z + 0.03),
        Vec2::new(3.55, 0.12),
    );
    // A restrained authored spill halo peeks around the cabinet; it never
    // replaces the CRT pixels and is layered behind the television geometry.
    spawn_panel(
        commands,
        meshes,
        screen_spill,
        "cabinet_screen_spill",
        Vec3::new(0.0, crate::crt::crt_screen_world_center().y, wall_z + 0.04),
        Vec2::new(0.62, 0.44),
    );
}

fn spill_mask() -> Image {
    const SIZE: u32 = 64;
    let mut pixels = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = (x as f32 + 0.5) / SIZE as f32 * 2.0 - 1.0;
            let dy = (y as f32 + 0.5) / SIZE as f32 * 2.0 - 1.0;
            let alpha = (1.0 - dx.hypot(dy)).clamp(0.0, 1.0).powi(2);
            pixels.extend_from_slice(&[255, 255, 255, (alpha * 255.0) as u8]);
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixels,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
    image
}

fn material(materials: &mut Assets<StandardMaterial>, color: Color) -> Handle<StandardMaterial> {
    materials.add(StandardMaterial {
        base_color: color,
        unlit: true,
        ..default()
    })
}

fn spawn_panel(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    material: Handle<StandardMaterial>,
    name: &'static str,
    position: Vec3,
    size: Vec2,
) {
    commands.spawn((
        Mesh3d(meshes.add(Rectangle::new(size.x, size.y))),
        MeshMaterial3d(material),
        Transform::from_translation(position),
        CabinetBackdrop,
        Name::new(name),
    ));
}

/// Fit the authored cabinet hero bounds to every positive viewport aspect.
#[must_use]
pub fn camera_distance(fov_y: f32, aspect: f32) -> f32 {
    let aspect = if aspect.is_finite() && aspect > 0.0 {
        aspect
    } else {
        16.0 / 9.0
    };
    let visible_height =
        (HERO_FRAME_H / HERO_FRAME_FILL).max(HERO_FRAME_W / (aspect * HERO_FRAME_FILL));
    visible_height / (2.0 * (fov_y * 0.5).tan())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presentation_defaults_to_existing_3d_and_accepts_fixed_mode() {
        assert_eq!(
            RoomPresentation::from_env_value(None),
            RoomPresentation::ThreeDimensional
        );
        assert_eq!(
            RoomPresentation::from_env_value(Some("fixed-cabinet")),
            RoomPresentation::FixedCabinet
        );
    }

    #[test]
    fn fixed_camera_fits_the_hero_at_narrow_wide_tall_and_small_aspects() {
        let fov = 0.85_f32;
        for (width, height) in [
            (320.0_f32, 900.0_f32),
            (450.0, 1000.0),
            (800.0, 800.0),
            (1280.0, 720.0),
            (2560.0, 1080.0),
            (320.0, 240.0),
        ] {
            let aspect = width / height;
            let distance = camera_distance(fov, aspect);
            let visible_height = 2.0 * distance * (fov * 0.5).tan();
            let visible_width = visible_height * aspect;
            assert!(visible_height * HERO_FRAME_FILL >= HERO_FRAME_H - 0.001);
            assert!(visible_width * HERO_FRAME_FILL >= HERO_FRAME_W - 0.001);
            assert!(visible_height * HERO_FRAME_FILL >= crate::crt::PHOSPHOR_H);
            assert!(visible_width * HERO_FRAME_FILL >= crate::crt::PHOSPHOR_W);
        }
        assert!(camera_distance(fov, 320.0 / 900.0) > camera_distance(fov, 1280.0 / 720.0));
    }

    #[test]
    fn flat_layers_are_behind_the_live_tv_anchor() {
        let backdrop = std::hint::black_box(BACKDROP_DEPTH);
        let tv = std::hint::black_box(crate::room::TV_STAND_POS.z);
        let screen = std::hint::black_box(crate::crt::crt_screen_world_center().z);
        assert!(backdrop < tv);
        assert!(backdrop < screen);
    }

    #[test]
    fn spill_mask_fades_to_transparent_at_its_edges() {
        let image = spill_mask();
        let pixels = image.data.expect("generated spill mask has pixel data");
        let pixel = |x: usize, y: usize| pixels[(y * 64 + x) * 4 + 3];
        assert!(pixel(32, 32) > 240);
        assert_eq!(pixel(0, 0), 0);
        assert!(pixel(32, 8) < pixel(32, 20));
    }
}
