//! Fixed-camera, flat-room comparison for the Spectrum Cabinet prototype (#558).
//!
//! The room, television, and cabinet use a generated 2D art plate; the
//! CRT screen and restrained lighting remain live runtime effects.

use bevy::prelude::*;

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
pub const BACKDROP_DEPTH: f32 = crate::room::TV_STAND_POS.z - 0.78;
pub const HERO_CENTER_Y: f32 = 0.82;
/// Small downward viewing angle from the approved room composition.
pub const HERO_CAMERA_PITCH: f32 = 0.069_813_17;
/// Continuous base wall covering viewport margins for portrait through ultrawide.
pub const BACKDROP_BASE_SIZE: Vec2 = Vec2::new(8.0, 9.0);
pub const ROOM_PLATE_WIDTH: f32 = 2.20;
pub const ROOM_PLATE_HEIGHT: f32 = ROOM_PLATE_WIDTH * 941.0 / 1672.0;
/// Pixel width of the 17–20 inch CRT opening in the authored room plate.
const CRT_OPENING_WIDTH_PX: f32 = 278.0;
/// The illustrated television glass opening is scaled to the live 4:3 CRT.
pub const FIXED_CRT_SCALE: f32 =
    ROOM_PLATE_WIDTH * CRT_OPENING_WIDTH_PX / 1672.0 / crate::crt::PHOSPHOR_W;
pub const HERO_FRAME_W: f32 = ROOM_PLATE_WIDTH;
pub const HERO_FRAME_H: f32 = ROOM_PLATE_HEIGHT;
pub const HERO_FRAME_FILL: f32 = 0.95;
const SCREEN_CENTER_X: f32 = 828.0;
const SCREEN_CENTER_Y: f32 = 438.5;
const SPECTRUM_48K_WIDTH: f32 = 0.25;
const SPECTRUM_48K_HEIGHT: f32 = SPECTRUM_48K_WIDTH * 1966.0 / 3130.0;
/// Tilt the authentic cutout toward the tabletop to match the plate's camera projection.
const SPECTRUM_48K_TABLETOP_TILT: f32 = 0.872_664_63;
const SPECTRUM_48K_DEPTH_OFFSET: f32 = 0.08;
const SPECTRUM_48K_SHADOW_DEPTH_OFFSET: f32 = 0.035;
const SPECTRUM_48K_IMAGE_CENTER_X: f32 = 836.0;
const SPECTRUM_48K_IMAGE_CENTER_Y: f32 = 667.0;
const SCREEN_OFFSET_X: f32 = ROOM_PLATE_WIDTH * (836.0 - SCREEN_CENTER_X) / 1672.0;
const SCREEN_OFFSET_Y: f32 = ROOM_PLATE_HEIGHT * (470.5 - SCREEN_CENTER_Y) / 941.0;

#[derive(Component, Debug)]
struct CabinetBackdrop;

#[derive(Component, Debug)]
pub(crate) struct CabinetBackdropBase;

/// World-space center of the 2D room illustration, used to center its fixed view.
#[must_use]
pub fn room_plate_center() -> Vec3 {
    let screen = crate::crt::crt_screen_world_center();
    Vec3::new(
        screen.x + SCREEN_OFFSET_X,
        screen.y - SCREEN_OFFSET_Y,
        screen.z - 0.02,
    )
}

/// Spawn the reference-matched 2D room plate and a scalable
/// warm backdrop for viewport margins. The illustrated TV remains static; only
/// its screen opening is rendered live by the CRT plugin.
pub fn spawn_backdrop(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    asset_server: &AssetServer,
) {
    let backdrop_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.012, 0.035, 0.026),
        unlit: true,
        ..default()
    });
    let plate_material = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(
            asset_server.load("spectrum_cabinet/room_plate_reference_candidate.png"),
        ),
        unlit: true,
        ..default()
    });
    let screen_spill_material = materials.add(StandardMaterial {
        base_color: Color::srgba(0.10, 0.88, 0.55, 0.18),
        base_color_texture: Some(asset_server.load("spectrum_cabinet/screen_spill.png")),
        alpha_mode: AlphaMode::Blend,
        emissive: LinearRgba::rgb(0.002, 0.035, 0.012),
        unlit: true,
        ..default()
    });

    commands.spawn((
        Mesh3d(meshes.add(Rectangle::new(BACKDROP_BASE_SIZE.x, BACKDROP_BASE_SIZE.y))),
        MeshMaterial3d(backdrop_material),
        Transform::from_xyz(0.0, HERO_CENTER_Y, BACKDROP_DEPTH - 0.01),
        CabinetBackdropBase,
        CabinetBackdrop,
        Name::new("cabinet_backdrop_base"),
    ));

    let screen = crate::crt::crt_screen_world_center();
    commands.spawn((
        Mesh3d(meshes.add(Rectangle::new(ROOM_PLATE_WIDTH, ROOM_PLATE_HEIGHT))),
        MeshMaterial3d(plate_material),
        Transform::from_translation(room_plate_center()),
        CabinetBackdrop,
        Name::new("spectrum_cabinet_room_plate"),
    ));
    commands.spawn((
        Mesh3d(meshes.add(Rectangle::new(0.84, 0.60))),
        MeshMaterial3d(screen_spill_material),
        Transform::from_translation(screen + Vec3::new(0.0, 0.025, -0.012)),
        CabinetBackdrop,
        Name::new("spectrum_cabinet_green_screen_spill"),
    ));

    let spectrum_x = ROOM_PLATE_WIDTH * (SPECTRUM_48K_IMAGE_CENTER_X - 836.0) / 1672.0;
    let spectrum_y =
        ROOM_PLATE_HEIGHT * 0.5 - SPECTRUM_48K_IMAGE_CENTER_Y * ROOM_PLATE_HEIGHT / 941.0;
    let spectrum_shadow_material = materials.add(StandardMaterial {
        base_color: Color::srgba(0.004, 0.008, 0.006, 0.48),
        base_color_texture: Some(asset_server.load("spectrum_cabinet/screen_spill.png")),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        ..default()
    });
    commands.spawn((
        Mesh3d(meshes.add(Rectangle::new(SPECTRUM_48K_WIDTH * 1.20, 0.08))),
        MeshMaterial3d(spectrum_shadow_material),
        Transform::from_translation(
            room_plate_center()
                + Vec3::new(
                    spectrum_x,
                    spectrum_y - 0.012,
                    SPECTRUM_48K_SHADOW_DEPTH_OFFSET,
                ),
        )
        .with_rotation(Quat::from_rotation_x(SPECTRUM_48K_TABLETOP_TILT)),
        CabinetBackdrop,
        Name::new("spectrum_48k_contact_shadow"),
    ));
    let spectrum_material = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(asset_server.load("spectrum_cabinet/spectrum_48k_cc0.png")),
        alpha_mode: AlphaMode::Blend,
        cull_mode: None,
        unlit: true,
        ..default()
    });
    commands.spawn((
        Mesh3d(meshes.add(Rectangle::new(SPECTRUM_48K_WIDTH, SPECTRUM_48K_HEIGHT))),
        MeshMaterial3d(spectrum_material),
        Transform::from_translation(
            room_plate_center() + Vec3::new(spectrum_x, spectrum_y, SPECTRUM_48K_DEPTH_OFFSET),
        )
        .with_rotation(Quat::from_rotation_x(SPECTRUM_48K_TABLETOP_TILT)),
        CabinetBackdrop,
        Name::new("spectrum_48k_hardware_layer"),
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
    let half_visible_height = HERO_FRAME_H / (2.0 * HERO_FRAME_FILL);
    let half_visible_width = HERO_FRAME_W / (2.0 * HERO_FRAME_FILL);
    let tan_half_fov = (fov_y * 0.5).tan();
    let sin_pitch = HERO_CAMERA_PITCH.sin();
    let cos_pitch = HERO_CAMERA_PITCH.cos();
    let vertical_fit = half_visible_height * (cos_pitch / tan_half_fov + sin_pitch);
    let horizontal_fit =
        half_visible_width / (aspect * tan_half_fov) + half_visible_height * sin_pitch;
    vertical_fit.max(horizontal_fit)
}

/// World-space wall extent needed at the wall's greater camera depth.
#[must_use]
pub fn viewport_background_size(fov_y: f32, aspect: f32) -> Vec2 {
    let aspect = if aspect.is_finite() && aspect > 0.0 {
        aspect
    } else {
        16.0 / 9.0
    };
    let distance = camera_distance(fov_y, aspect);
    let screen_z = crate::crt::crt_screen_world_center().z;
    let base_z = BACKDROP_DEPTH - 0.012;
    let camera_to_base = distance + screen_z - base_z;
    let visible_height = 2.0 * camera_to_base * (fov_y * 0.5).tan() * 1.05;
    Vec2::new(visible_height * aspect, visible_height)
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
            assert!(
                visible_height * HERO_FRAME_FILL
                    >= crate::crt::PHOSPHOR_H * FIXED_CRT_SCALE - 0.001
            );
            assert!(
                visible_width * HERO_FRAME_FILL >= crate::crt::PHOSPHOR_W * FIXED_CRT_SCALE - 0.001
            );
        }
        assert!(camera_distance(fov, 320.0 / 900.0) > camera_distance(fov, 1280.0 / 720.0));
    }

    #[test]
    fn base_wall_covers_every_supported_viewport_aspect() {
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
            let required = viewport_background_size(fov, aspect);
            assert!(BACKDROP_BASE_SIZE.y >= required.y);
            assert!(BACKDROP_BASE_SIZE.x >= required.x);
        }
    }

    #[test]
    fn viewport_background_size_covers_extreme_aspects() {
        let fov = 0.85_f32;
        for aspect in [0.1_f32, 0.35, 1.0, 2.37, 10.0] {
            let size = viewport_background_size(fov, aspect);
            let distance = camera_distance(fov, aspect);
            let camera_to_base =
                distance + crate::crt::crt_screen_world_center().z - (BACKDROP_DEPTH - 0.012);
            let projected_height = 2.0 * camera_to_base * (fov * 0.5).tan();
            assert!(size.y >= projected_height);
            assert!(size.x >= projected_height * aspect);
        }
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
    fn spectrum_hardware_layer_asset_is_rgba_with_expected_dimensions() {
        let png = include_bytes!("../assets/spectrum_cabinet/spectrum_48k_cc0.png");
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(
            u32::from_be_bytes(png[16..20].try_into().expect("PNG width bytes")),
            3130
        );
        assert_eq!(
            u32::from_be_bytes(png[20..24].try_into().expect("PNG height bytes")),
            1966
        );
        assert_eq!(png[25], 6, "hardware cutout keeps RGBA transparency");
    }

    #[test]
    fn spectrum_hardware_layer_preserves_source_aspect_ratio() {
        let source_aspect = 3130.0_f32 / 1966.0;
        let layer_aspect = SPECTRUM_48K_WIDTH / SPECTRUM_48K_HEIGHT;
        assert!((layer_aspect - source_aspect).abs() < f32::EPSILON * 4.0);
    }

    #[test]
    fn spill_mask_asset_has_expected_png_dimensions() {
        let png = include_bytes!("../assets/spectrum_cabinet/screen_spill.png");
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(
            u32::from_be_bytes(png[16..20].try_into().expect("PNG width bytes")),
            128
        );
        assert_eq!(
            u32::from_be_bytes(png[20..24].try_into().expect("PNG height bytes")),
            128
        );
    }
}
