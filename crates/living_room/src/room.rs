//! Small dark UK 1980s living room — Poly Haven CC0 models + PBR textures.

use bevy::math::Affine2;
use bevy::prelude::*;

/// Period-inspired family living room: ~4.2 × 5.25 × 2.5 m (about 22 m²).
pub const ROOM_W: f32 = 4.2;
pub const ROOM_D: f32 = 5.25;
pub const ROOM_H: f32 = 2.5;

/// World-space wallpaper tile size (metres per texture repeat).
const WALLPAPER_TILE_M: f32 = 0.55;

/// World pose of the CRT cabinet / console (phosphor overlay uses the same constant).
/// Against the back wall; locked cam frames CRT at ~50% vertical fill with room visible.
pub const TV_STAND_POS: Vec3 = Vec3::new(0.0, 0.0, -ROOM_D * 0.5 + 0.55);

/// Marker on the `television_02` scene root — phosphor is placed in this local space.
#[derive(Component, Reflect, Debug, Clone, Copy, Default)]
#[reflect(Component, Default)]
pub struct TelevisionCabinet;

/// Base output for a sconce, scaled by the scene-opening light fade.
#[derive(Component, Debug, Clone, Copy)]
pub(crate) struct OpeningSconceLight(pub f32);

/// Center TV-wall sconce contributes the opening's early, localized TV accent.
#[derive(Component, Debug, Clone, Copy, Default)]
pub(crate) struct OpeningTvAccent;

/// Emission for the visible bulb overlay, scaled with the sconce light.
#[derive(Component, Debug, Clone, Copy)]
pub(crate) struct OpeningSconceBulb(pub Vec3);

#[derive(Debug, Default)]
pub struct RoomPlugin;

impl Plugin for RoomPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_room);
    }
}

fn setup_room(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    asset_server: Res<AssetServer>,
    #[cfg(feature = "skein")] skein_mode: Option<Res<crate::skein::SkeinRoomMode>>,
    presentation: Res<crate::cabinet_room::RoomPresentation>,
) {
    if *presentation == crate::cabinet_room::RoomPresentation::FixedCabinet {
        crate::cabinet_room::spawn_backdrop(
            &mut commands,
            &mut meshes,
            &mut materials,
            &mut images,
        );
        spawn_live_television(&mut commands, &asset_server);
        return;
    }
    #[cfg(feature = "skein")]
    if let Some(mode) = skein_mode.as_deref() {
        if crate::skein::spawn_skein_room_if_active(&mut commands, &asset_server, mode) {
            return;
        }
    }

    setup_procedural_room(&mut commands, &mut meshes, &mut materials, &asset_server);
}

fn setup_procedural_room(
    commands: &mut Commands,
    meshes: &mut ResMut<'_, Assets<Mesh>>,
    materials: &mut ResMut<'_, Assets<StandardMaterial>>,
    asset_server: &AssetServer,
) {
    let carpet = pbr_material(
        materials,
        asset_server,
        "polyhaven/textures/dirty_carpet/dirty_carpet",
        0.0,
        None,
    );
    // Cuboid faces use 0–1 UVs, so tile in world metres via uv_transform.
    // Back/front faces are ROOM_W×ROOM_H; left/right are ROOM_D×ROOM_H.
    let wallpaper_back = pbr_material(
        materials,
        asset_server,
        "polyhaven/textures/floral_jacquard/floral_jacquard",
        0.0,
        Some(Vec2::new(
            ROOM_W / WALLPAPER_TILE_M,
            ROOM_H / WALLPAPER_TILE_M,
        )),
    );
    let wallpaper_side = pbr_material(
        materials,
        asset_server,
        "polyhaven/textures/floral_jacquard/floral_jacquard",
        0.0,
        Some(Vec2::new(
            ROOM_D / WALLPAPER_TILE_M,
            ROOM_H / WALLPAPER_TILE_M,
        )),
    );
    let plaster = pbr_material(
        materials,
        asset_server,
        "polyhaven/textures/beige_wall_001/beige_wall_001",
        0.0,
        None,
    );
    let walnut = pbr_material(
        materials,
        asset_server,
        "polyhaven/textures/american_walnut_veneer/american_walnut_veneer",
        0.05,
        None,
    );
    let curtain_mat = pbr_material(
        materials,
        asset_server,
        "polyhaven/textures/velour_velvet/velour_velvet",
        0.0,
        None,
    );

    // Floor / ceiling
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(ROOM_W, ROOM_D))),
        MeshMaterial3d(carpet),
        Transform::from_xyz(0.0, 0.0, 0.0),
        Name::new("carpet"),
        crate::hybrid::RoomStatic,
    ));
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(ROOM_W, ROOM_D))),
        MeshMaterial3d(plaster.clone()),
        Transform::from_xyz(0.0, ROOM_H, 0.0)
            .with_rotation(Quat::from_rotation_x(std::f32::consts::PI)),
        Name::new("ceiling"),
        crate::hybrid::RoomStatic,
    ));

    let wall_t = 0.06;
    // The camera's widest zoom looks back into the room from the front opening.
    // Keep the shell open there so the front wall cannot occlude the CRT.
    for (name, pos, size, wallpaper) in [
        (
            "wall_back",
            Vec3::new(0.0, ROOM_H * 0.5, -ROOM_D * 0.5),
            Vec3::new(ROOM_W, ROOM_H, wall_t),
            wallpaper_back.clone(),
        ),
        (
            "wall_left",
            Vec3::new(-ROOM_W * 0.5, ROOM_H * 0.5, 0.0),
            Vec3::new(wall_t, ROOM_H, ROOM_D),
            wallpaper_side.clone(),
        ),
        (
            "wall_right",
            Vec3::new(ROOM_W * 0.5, ROOM_H * 0.5, 0.0),
            Vec3::new(wall_t, ROOM_H, ROOM_D),
            wallpaper_side.clone(),
        ),
    ] {
        commands.spawn((
            Mesh3d(meshes.add(Cuboid::new(size.x, size.y, size.z))),
            MeshMaterial3d(wallpaper),
            Transform::from_translation(pos),
            Name::new(name),
            crate::hybrid::RoomStatic,
        ));
    }

    // Drawn curtains flanking the TV wall.
    for x in [-1.65f32, 1.65] {
        commands.spawn((
            Mesh3d(meshes.add(Cuboid::new(0.4, 2.0, 0.06))),
            MeshMaterial3d(curtain_mat.clone()),
            Transform::from_xyz(x, 1.15, -ROOM_D * 0.5 + 0.06),
            Name::new("curtain"),
            crate::hybrid::RoomStatic,
        ));
    }

    // Skirting board (walnut veneer).
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(ROOM_W - 0.12, 0.09, 0.03))),
        MeshMaterial3d(walnut.clone()),
        Transform::from_xyz(0.0, 0.045, -ROOM_D * 0.5 + 0.04),
        Name::new("skirting"),
        crate::hybrid::RoomStatic,
    ));

    // --- Poly Haven glTF furniture ---
    spawn_live_television(commands, asset_server);

    // Sofa faces the TV, leaving a clear walkway along each side of the room.
    commands.spawn((
        WorldAssetRoot(asset_server.load("polyhaven/models/sofa_03/sofa_03_1k.gltf#Scene0")),
        Transform::from_xyz(0.0, 0.0, 0.50)
            .with_rotation(Quat::from_rotation_y(std::f32::consts::PI))
            .with_scale(Vec3::splat(0.80)),
        Name::new("sofa"),
        crate::hybrid::RoomStatic,
    ));

    // A small walnut side table gives the plant a clear purpose and keeps it
    // tucked against the left wall, outside the TV and sofa circulation paths.
    let plant_table_pos = Vec3::new(-1.81, 0.0, -1.45);
    let plant_table_root = commands
        .spawn((
            Transform::from_translation(plant_table_pos),
            Visibility::default(),
            Name::new("plant_side_table"),
            crate::hybrid::RoomStatic,
        ))
        .id();
    commands.entity(plant_table_root).with_children(|table| {
        table.spawn((
            Mesh3d(meshes.add(Cuboid::new(0.54, 0.035, 0.44))),
            MeshMaterial3d(walnut.clone()),
            Transform::from_xyz(0.0, 0.4825, 0.0),
            Name::new("plant_side_table_top"),
        ));
        table.spawn((
            Mesh3d(meshes.add(Cuboid::new(0.42, 0.09, 0.018))),
            MeshMaterial3d(walnut.clone()),
            Transform::from_xyz(0.0, 0.405, -0.17),
            Name::new("plant_side_table_back_apron"),
        ));
        table.spawn((
            Mesh3d(meshes.add(Cuboid::new(0.42, 0.09, 0.018))),
            MeshMaterial3d(walnut.clone()),
            Transform::from_xyz(0.0, 0.405, 0.17),
            Name::new("plant_side_table_front_apron"),
        ));
        for x in [-0.225f32, 0.225] {
            for z in [-0.17f32, 0.17] {
                table.spawn((
                    Mesh3d(meshes.add(Cuboid::new(0.045, 0.46, 0.045))),
                    MeshMaterial3d(walnut.clone()),
                    Transform::from_xyz(x, 0.23, z),
                    Name::new("plant_side_table_leg"),
                ));
            }
        }
        table.spawn((
            WorldAssetRoot(
                asset_server
                    .load("polyhaven/models/potted_plant_04/potted_plant_04_1k.gltf#Scene0"),
            ),
            Transform::from_xyz(0.0, 0.50, 0.0).with_scale(Vec3::splat(0.85)),
            Name::new("living_room_plant"),
        ));
    });

    setup_room_dressing(commands, meshes, materials, asset_server);
}

fn spawn_live_television(commands: &mut Commands, asset_server: &AssetServer) {
    // Low teak-ish sideboard as an 80s TV stand (replaces ornate ClassicConsole_01).
    let tv_stand_scale = 0.85;
    let tv_stand_top = 0.68 * tv_stand_scale;
    commands.spawn((
        WorldAssetRoot(
            asset_server.load(
                "polyhaven/models/modern_wooden_cabinet/modern_wooden_cabinet_1k.gltf#Scene0",
            ),
        ),
        Transform::from_translation(TV_STAND_POS).with_scale(Vec3::splat(tv_stand_scale)),
        Name::new("tv_stand"),
        crate::hybrid::LiveTv,
    ));

    // Vintage CRT on the console. Painted glass is punched out
    // (`television_02_aperture` via `scripts/punch_tv_screen_aperture.py`) so both
    // the outer cabinet bevel and inner screen bezel remain; phosphor sits behind
    // the inner rim (`crt`).
    commands.spawn((
        WorldAssetRoot(
            asset_server.load("polyhaven/models/television_02/television_02_aperture.gltf#Scene0"),
        ),
        Transform::from_translation(TV_STAND_POS + Vec3::new(0.0, tv_stand_top, 0.05)),
        TelevisionCabinet,
        Name::new("television_02"),
        crate::hybrid::LiveTv,
    ));
}

fn setup_room_dressing(
    commands: &mut Commands,
    meshes: &mut ResMut<'_, Assets<Mesh>>,
    materials: &mut ResMut<'_, Assets<StandardMaterial>>,
    asset_server: &AssetServer,
) {
    spawn_polyhaven_wall_sconces(commands, meshes, materials, asset_server);
    spawn_video_cassette_deck(commands, meshes, materials);
    spawn_floor_toys(commands, asset_server);
    spawn_spectrum_joystick(commands, meshes, materials);
}

/// Detailed 1980s VHS deck on the cabinet top, clear of the sliding doors.
fn spawn_video_cassette_deck(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    let chassis = materials.add(StandardMaterial {
        base_color: Color::srgb(0.075, 0.085, 0.095),
        metallic: 0.32,
        perceptual_roughness: 0.48,
        ..default()
    });
    let fascia = materials.add(StandardMaterial {
        base_color: Color::srgb(0.20, 0.21, 0.22),
        metallic: 0.38,
        perceptual_roughness: 0.36,
        ..default()
    });
    let display = materials.add(StandardMaterial {
        base_color: Color::srgb(0.08, 0.14, 0.10),
        emissive: LinearRgba::rgb(0.12, 0.34, 0.18),
        perceptual_roughness: 0.25,
        ..default()
    });
    let trim = materials.add(StandardMaterial {
        base_color: Color::srgb(0.42, 0.44, 0.43),
        metallic: 0.78,
        perceptual_roughness: 0.28,
        ..default()
    });
    let red = materials.add(StandardMaterial {
        base_color: Color::srgb(0.64, 0.055, 0.035),
        metallic: 0.12,
        perceptual_roughness: 0.3,
        ..default()
    });
    let tv_stand_top = 0.68 * 0.85;
    let origin = TV_STAND_POS + Vec3::new(0.66, tv_stand_top + 0.0475, 0.0);
    let root = commands
        .spawn((
            Transform::from_translation(origin),
            Visibility::default(),
            Name::new("vhs_video_cassette_recorder"),
            crate::hybrid::RoomStatic,
        ))
        .id();
    commands.entity(root).with_children(|c| {
        c.spawn((
            Mesh3d(meshes.add(Cuboid::new(0.54, 0.095, 0.36))),
            MeshMaterial3d(chassis.clone()),
            Transform::IDENTITY,
            Name::new("vhs_deck_body"),
        ));
        c.spawn((
            Mesh3d(meshes.add(Cuboid::new(0.51, 0.060, 0.012))),
            MeshMaterial3d(fascia.clone()),
            Transform::from_xyz(0.0, 0.0, 0.178),
            Name::new("vhs_brushed_front_fascia"),
        ));
        c.spawn((
            Mesh3d(meshes.add(Cuboid::new(0.205, 0.030, 0.005))),
            MeshMaterial3d(chassis.clone()),
            Transform::from_xyz(-0.105, 0.0, 0.187),
            Name::new("vhs_cassette_bay"),
        ));
        c.spawn((
            Mesh3d(meshes.add(Cuboid::new(0.195, 0.022, 0.005))),
            MeshMaterial3d(fascia.clone()),
            Transform::from_xyz(-0.105, 0.002, 0.191),
            Name::new("vhs_tape_door"),
        ));
        c.spawn((
            Mesh3d(meshes.add(Cuboid::new(0.14, 0.003, 0.002))),
            MeshMaterial3d(chassis.clone()),
            Transform::from_xyz(-0.105, -0.004, 0.194),
            Name::new("vhs_eject_seam"),
        ));
        c.spawn((
            Mesh3d(meshes.add(Cuboid::new(0.082, 0.027, 0.004))),
            MeshMaterial3d(chassis.clone()),
            Transform::from_xyz(0.155, 0.008, 0.188),
            Name::new("vhs_counter_bezel"),
        ));
        c.spawn((
            Mesh3d(meshes.add(Cuboid::new(0.072, 0.019, 0.003))),
            MeshMaterial3d(display.clone()),
            Transform::from_xyz(0.155, 0.008, 0.192),
            Name::new("vhs_green_clock_display"),
        ));
        // Seven-segment counter detail, visible in the wide room preset.
        for (x, y, w, h) in [
            (0.137, 0.014, 0.010, 0.002),
            (0.137, 0.008, 0.010, 0.002),
            (0.137, 0.002, 0.010, 0.002),
            (0.132, 0.011, 0.002, 0.004),
            (0.142, 0.011, 0.002, 0.004),
            (0.132, 0.005, 0.002, 0.004),
            (0.142, 0.005, 0.002, 0.004),
        ] {
            c.spawn((
                Mesh3d(meshes.add(Cuboid::new(w, h, 0.001))),
                MeshMaterial3d(display.clone()),
                Transform::from_xyz(x, y, 0.194),
                Name::new("vhs_counter_segment"),
            ));
        }
        let button_mesh = meshes.add(Cylinder::new(0.009, 0.009));
        for (x, material, name) in [
            (-0.235, trim.clone(), "vhs_rewind"),
            (-0.200, trim.clone(), "vhs_play"),
            (-0.165, trim.clone(), "vhs_stop"),
            (0.225, trim.clone(), "vhs_power"),
            (0.258, red.clone(), "vhs_record"),
        ] {
            c.spawn((
                Mesh3d(button_mesh.clone()),
                MeshMaterial3d(material),
                Transform::from_xyz(x, 0.0, 0.188)
                    .with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
                Name::new(name),
            ));
        }
    });
}

/// Poly Haven [industrial_wall_sconce](https://polyhaven.com/a/industrial_wall_sconce) (CC0) —
/// brass/copper vintage wall light (~0.3 m). Local +Z faces into the room.
fn spawn_polyhaven_wall_sconces(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    asset_server: &AssetServer,
) {
    let sconce = asset_server
        .load("polyhaven/models/industrial_wall_sconce/industrial_wall_sconce_1k.gltf#Scene0");

    let wall_z = -ROOM_D * 0.5 + 0.03;
    let wall_x = ROOM_W * 0.5 - 0.03;
    // (position, into-room normal, name)
    let mounts = [
        (
            Vec3::new(-1.32, 1.55, wall_z),
            Vec3::Z,
            "wall_sconce_tv_left",
            true,
        ),
        (
            Vec3::new(0.0, 1.85, wall_z),
            Vec3::Z,
            "wall_sconce_tv_centre",
            true,
        ),
        (
            Vec3::new(1.32, 1.55, wall_z),
            Vec3::Z,
            "wall_sconce_tv_right",
            true,
        ),
        (
            Vec3::new(-wall_x, 1.45, 0.55),
            Vec3::X,
            "wall_sconce_left",
            false,
        ),
        (
            Vec3::new(wall_x, 1.45, 0.55),
            -Vec3::X,
            "wall_sconce_right",
            false,
        ),
    ];

    let min_lights = crate::quality::light_preset() == crate::quality::LightPreset::Min;
    let bulb_mesh = meshes.add(Sphere::new(0.035).mesh().uv(16, 12));
    // The source glTF bulb material is diffuse only, so its light source vanishes
    // in the dark room. Add a small warm emitter over the model's bulb glass.
    let bulb_emissive = Vec3::new(2.5, 1.05, 0.28);
    let initial_light_gain = crate::camera::OPENING_DIM_FACTOR;
    let bulb_material = StandardMaterial {
        base_color: Color::srgb(1.0, 0.72, 0.38),
        emissive: LinearRgba::rgb(
            bulb_emissive.x * initial_light_gain,
            bulb_emissive.y * initial_light_gain,
            bulb_emissive.z * initial_light_gain,
        ),
        unlit: true,
        ..default()
    };
    let regular_bulb_material = materials.add(bulb_material.clone());
    let tv_accent_bulb_material = materials.add(bulb_material);
    for (pos, into_room, name, lit) in mounts {
        let rot = Quat::from_rotation_arc(Vec3::Z, into_room.normalize());
        commands.spawn((
            WorldAssetRoot(sconce.clone()),
            Transform::from_translation(pos).with_rotation(rot),
            Name::new(name),
            crate::hybrid::RoomStatic,
        ));
        // Lights are *not* parented under RoomStatic — hybrid hides the room mesh
        // after baking, but the live TV still needs the three TV-wall bulbs.
        let use_light = lit && (!min_lights || name == "wall_sconce_tv_centre");
        if use_light {
            // Matches the centre of the bulb material in the Poly Haven model.
            // Using the same offset for all mounts keeps the point source inside
            // the visible glass instead of floating above/in front of the fixture.
            let bulb_local = Vec3::new(0.0, 0.107, 0.192);
            let intensity = if name == "wall_sconce_tv_centre" {
                5_400.0
            } else {
                6_500.0
            };
            let bulb_world = pos + rot * bulb_local;
            let accent = name == "wall_sconce_tv_centre";
            let bulb = (
                Mesh3d(bulb_mesh.clone()),
                MeshMaterial3d(if accent {
                    tv_accent_bulb_material.clone()
                } else {
                    regular_bulb_material.clone()
                }),
                Transform::from_translation(bulb_world),
                OpeningSconceBulb(bulb_emissive),
            );
            if accent {
                commands.spawn((
                    bulb,
                    OpeningTvAccent,
                    Name::new(format!("{name}_visible_bulb")),
                ));
            } else {
                commands.spawn((bulb, Name::new(format!("{name}_visible_bulb"))));
            }
            let light = PointLight {
                color: Color::srgb(1.0, 0.72, 0.38),
                // Room fill only — keep CRT exposure/spill at #238 (#233).
                intensity: intensity * initial_light_gain,
                range: 6.0,
                radius: 0.08,
                shadow_maps_enabled: false,
                ..default()
            };
            if accent {
                commands.spawn((
                    light,
                    Transform::from_translation(bulb_world),
                    OpeningSconceLight(intensity),
                    OpeningTvAccent,
                    crate::scene_variant::DynamicRoomFillLight,
                    bevy::camera::visibility::RenderLayers::layer(0).with(1),
                    Name::new(format!("{name}_bulb")),
                ));
            } else {
                commands.spawn((
                    light,
                    Transform::from_translation(bulb_world),
                    OpeningSconceLight(intensity),
                    crate::scene_variant::DynamicRoomFillLight,
                    bevy::camera::visibility::RenderLayers::layer(0).with(1),
                    Name::new(format!("{name}_bulb")),
                ));
            }
        }
    }
}

/// A small pair of toys tucked beside the sofa reads as a deliberate play spot.
fn spawn_floor_toys(commands: &mut Commands, asset_server: &AssetServer) {
    // y offsets lift meshes whose AABB dips below 0 so they sit on the carpet.
    let toys = [
        (
            "polyhaven/models/dirty_football/dirty_football_1k.gltf#Scene0",
            Vec3::new(1.38, 0.0, 0.08),
            Quat::from_rotation_y(0.2),
            Vec3::splat(0.70),
            "toy_football",
        ),
        (
            "polyhaven/models/rubber_duck_toy/rubber_duck_toy_1k.gltf#Scene0",
            Vec3::new(1.16, 0.0, 0.08),
            Quat::from_rotation_y(-0.3),
            Vec3::splat(0.55),
            "toy_rubber_duck",
        ),
    ];
    for (path, pos, rot, scale, name) in toys {
        commands.spawn((
            WorldAssetRoot(asset_server.load(path)),
            Transform::from_translation(pos)
                .with_rotation(rot)
                .with_scale(scale),
            Name::new(name),
            crate::hybrid::RoomStatic,
        ));
    }
}

/// Competition Pro–style stick (no CC0 Spectrum/Kempston model found).
/// Black base, dual red fire buttons, ball-top shaft — typical ZX Spectrum look.
fn spawn_spectrum_joystick(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    let black = materials.add(StandardMaterial {
        base_color: Color::srgb(0.08, 0.08, 0.09),
        perceptual_roughness: 0.55,
        metallic: 0.05,
        ..default()
    });
    let red = materials.add(StandardMaterial {
        base_color: Color::srgb(0.85, 0.12, 0.1),
        perceptual_roughness: 0.4,
        metallic: 0.05,
        ..default()
    });
    let base = meshes.add(Cuboid::new(0.11, 0.035, 0.11));
    let shaft = meshes.add(Cylinder::new(0.012, 0.07));
    let ball = meshes.add(Sphere::new(0.028).mesh().uv(16, 12));
    let btn = meshes.add(Cylinder::new(0.014, 0.01));

    let root = commands
        .spawn((
            Transform::from_xyz(0.55, 0.0, -0.08).with_rotation(Quat::from_rotation_y(0.35)),
            Visibility::default(),
            Name::new("spectrum_joystick"),
            crate::hybrid::RoomStatic,
        ))
        .id();

    commands.entity(root).with_children(|c| {
        c.spawn((
            Mesh3d(base),
            MeshMaterial3d(black.clone()),
            Transform::from_xyz(0.0, 0.0175, 0.0),
            Name::new("joy_base"),
        ));
        c.spawn((
            Mesh3d(shaft),
            MeshMaterial3d(black.clone()),
            Transform::from_xyz(0.0, 0.06, 0.0),
            Name::new("joy_shaft"),
        ));
        c.spawn((
            Mesh3d(ball),
            MeshMaterial3d(black),
            Transform::from_xyz(0.0, 0.105, 0.0),
            Name::new("joy_ball"),
        ));
        for (x, name) in [(-0.028f32, "joy_fire_l"), (0.028, "joy_fire_r")] {
            c.spawn((
                Mesh3d(btn.clone()),
                MeshMaterial3d(red.clone()),
                Transform::from_xyz(x, 0.038, 0.032),
                Name::new(name),
            ));
        }
    });
}

fn pbr_material(
    materials: &mut Assets<StandardMaterial>,
    assets: &AssetServer,
    stem: &str,
    metallic: f32,
    uv_tiles: Option<Vec2>,
) -> Handle<StandardMaterial> {
    let uv_transform = match uv_tiles {
        Some(tiles) => Affine2::from_scale(tiles),
        None => Affine2::IDENTITY,
    };
    materials.add(StandardMaterial {
        base_color_texture: Some(assets.load(format!("{stem}_diff_1k.jpg"))),
        normal_map_texture: Some(assets.load(format!("{stem}_nor_gl_1k.jpg"))),
        metallic_roughness_texture: Some(assets.load(format!("{stem}_arm_1k.jpg"))),
        perceptual_roughness: 1.0,
        metallic,
        reflectance: 0.1,
        uv_transform,
        ..default()
    })
}
