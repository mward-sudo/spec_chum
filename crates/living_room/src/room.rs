//! Small dark UK 1980s living room — Poly Haven CC0 models + PBR textures.

use bevy::math::Affine2;
use bevy::prelude::*;

/// Room: ~3.2 × 3.8 × 2.4 m.
pub const ROOM_W: f32 = 3.2;
pub const ROOM_D: f32 = 3.8;
pub const ROOM_H: f32 = 2.4;

/// World-space wallpaper tile size (metres per texture repeat).
const WALLPAPER_TILE_M: f32 = 0.55;

/// World pose of the CRT cabinet / console (phosphor overlay uses the same constant).
/// Against the back wall; locked cam frames CRT at ~50% vertical fill with room visible.
pub const TV_STAND_POS: Vec3 = Vec3::new(0.0, 0.0, -1.35);

/// Marker on the `television_02` scene root — phosphor is placed in this local space.
#[derive(Component, Debug, Clone, Copy)]
pub struct TelevisionCabinet;

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
    asset_server: Res<AssetServer>,
) {
    let carpet = pbr_material(
        &mut materials,
        &asset_server,
        "polyhaven/textures/dirty_carpet/dirty_carpet",
        0.0,
        None,
    );
    // Cuboid faces use 0–1 UVs, so tile in world metres via uv_transform.
    // Back/front faces are ROOM_W×ROOM_H; left/right are ROOM_D×ROOM_H.
    let wallpaper_back = pbr_material(
        &mut materials,
        &asset_server,
        "polyhaven/textures/floral_jacquard/floral_jacquard",
        0.0,
        Some(Vec2::new(
            ROOM_W / WALLPAPER_TILE_M,
            ROOM_H / WALLPAPER_TILE_M,
        )),
    );
    let wallpaper_side = pbr_material(
        &mut materials,
        &asset_server,
        "polyhaven/textures/floral_jacquard/floral_jacquard",
        0.0,
        Some(Vec2::new(
            ROOM_D / WALLPAPER_TILE_M,
            ROOM_H / WALLPAPER_TILE_M,
        )),
    );
    let plaster = pbr_material(
        &mut materials,
        &asset_server,
        "polyhaven/textures/beige_wall_001/beige_wall_001",
        0.0,
        None,
    );
    let walnut = pbr_material(
        &mut materials,
        &asset_server,
        "polyhaven/textures/american_walnut_veneer/american_walnut_veneer",
        0.05,
        None,
    );
    let curtain_mat = pbr_material(
        &mut materials,
        &asset_server,
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
    for (name, pos, size, wallpaper) in [
        (
            "wall_back",
            Vec3::new(0.0, ROOM_H * 0.5, -ROOM_D * 0.5),
            Vec3::new(ROOM_W, ROOM_H, wall_t),
            wallpaper_back.clone(),
        ),
        (
            "wall_front",
            Vec3::new(0.0, ROOM_H * 0.5, ROOM_D * 0.5),
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
    for x in [-1.25f32, 1.25] {
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
        MeshMaterial3d(walnut),
        Transform::from_xyz(0.0, 0.045, -ROOM_D * 0.5 + 0.04),
        Name::new("skirting"),
        crate::hybrid::RoomStatic,
    ));

    // --- Poly Haven glTF furniture ---
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

    // Sofa facing the TV (scale down slightly for the small room).
    commands.spawn((
        WorldAssetRoot(asset_server.load("polyhaven/models/sofa_03/sofa_03_1k.gltf#Scene0")),
        Transform::from_xyz(0.0, 0.0, 1.05)
            .with_rotation(Quat::from_rotation_y(std::f32::consts::PI))
            .with_scale(Vec3::splat(0.72)),
        Name::new("sofa"),
        crate::hybrid::RoomStatic,
    ));

    // Armchairs flanking the sofa.
    for (x, yaw) in [(-1.15f32, -0.55f32), (1.15, 0.55)] {
        commands.spawn((
            WorldAssetRoot(
                asset_server.load("polyhaven/models/ArmChair_01/ArmChair_01_1k.gltf#Scene0"),
            ),
            Transform::from_xyz(x, 0.0, 0.35).with_rotation(Quat::from_rotation_y(yaw)),
            Name::new("armchair"),
            crate::hybrid::RoomStatic,
        ));
    }

    spawn_polyhaven_wall_sconces(&mut commands, &asset_server);
    spawn_floor_toys(&mut commands, &asset_server);
    spawn_spectrum_joystick(&mut commands, &mut meshes, &mut materials);

    let _ = plaster;
}

/// Poly Haven [industrial_wall_sconce](https://polyhaven.com/a/industrial_wall_sconce) (CC0) —
/// brass/copper vintage wall light (~0.3 m). Local +Z faces into the room.
fn spawn_polyhaven_wall_sconces(commands: &mut Commands, asset_server: &AssetServer) {
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
            let bulb_local = Vec3::new(0.0, 0.05, 0.16);
            let bulb_world = pos + rot * bulb_local;
            commands.spawn((
                PointLight {
                    color: Color::srgb(1.0, 0.72, 0.38),
                    // Room fill only — keep CRT exposure/spill at #238 (#233).
                    intensity: 6_500.0,
                    range: 6.0,
                    radius: 0.08,
                    shadow_maps_enabled: false,
                    ..default()
                },
                Transform::from_translation(bulb_world),
                // Layer 0 = live TV; layer 1 = room bake plate.
                bevy::camera::visibility::RenderLayers::layer(0).with(1),
                Name::new(format!("{name}_bulb")),
            ));
        }
    }
}

/// Floor clutter — CC0 Poly Haven props that read as 80s kid living-room toys.
/// Positions are in the open carpet between sofa and TV (camera-facing).
fn spawn_floor_toys(commands: &mut Commands, asset_server: &AssetServer) {
    // y offsets lift meshes whose AABB dips below 0 so they sit on the carpet.
    let toys = [
        (
            "polyhaven/models/dirty_football/dirty_football_1k.gltf#Scene0",
            Vec3::new(-0.7, 0.12, 0.55),
            Quat::from_rotation_y(0.7),
            Vec3::splat(1.15),
            "toy_football",
        ),
        (
            "polyhaven/models/rubber_duck_toy/rubber_duck_toy_1k.gltf#Scene0",
            Vec3::new(-0.35, 0.0, 0.45),
            Quat::from_rotation_y(-0.9),
            Vec3::splat(1.1),
            "toy_rubber_duck",
        ),
        (
            "polyhaven/models/gamepad/gamepad_1k.gltf#Scene0",
            Vec3::new(0.55, 0.0, 0.5),
            Quat::from_rotation_y(2.4),
            Vec3::splat(1.8),
            "toy_gamepad",
        ),
        (
            "polyhaven/models/portable_cassette_player/portable_cassette_player_1k.gltf#Scene0",
            Vec3::new(0.85, 0.055, 0.35),
            Quat::from_rotation_y(-0.4),
            Vec3::splat(1.35),
            "toy_walkman",
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
            Transform::from_xyz(0.55, 0.0, 0.65).with_rotation(Quat::from_rotation_y(0.35)),
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
