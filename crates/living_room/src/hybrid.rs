//! Hybrid present — bake static room once per zoom stop; live TV every frame.
//!
//! **Experimental / default off** (`SPEC_CHUM_ROOM_HYBRID=1`). Early measurements
//! that claimed full-room PBR needed hybrid for 60 Hz were dominated by a
//! **blocking CPU readback** harness — not the shipping IOSurface present path.
//! Full 3D at default quality already meets the 60 Hz budget on M4-class hardware;
//! prefer Blender lightmaps (#149) over extending these plates.
//!
//! Plate is a **3D unlit quad** parented to the living-room camera (not Camera2d) —
//! same `RenderTarget::Image` as the live pass, so headless / IOSurface present works.
//! Markers [`RoomStatic`] / [`LiveTv`] are always available for scene tagging even
//! when the plugin is idle.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::RenderLayers;
use bevy::camera::RenderTarget;
use bevy::light::cluster::ClusterConfig;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
use bevy::render::view::Msaa;

use crate::camera::{CameraLocked, CameraZoom, LivingRoomCamera, ZOOM_PRESET_COUNT};
use crate::quality;

/// Bake layer — static room only.
pub const LAYER_ROOM: usize = 1;
/// Live layer — TV / cabinet / CRT + plate (default layer 0).
pub const LAYER_LIVE: usize = 0;

/// Distance of the plate quad in camera space (metres along −Z).
const PLATE_DISTANCE: f32 = 8.0;
/// Vertical FOV must match [`crate::camera`] locked FOV.
const PLATE_FOV: f32 = 0.85;

/// Static room content (hidden while a plate is shown).
#[derive(Component, Debug, Clone, Copy)]
pub struct RoomStatic;

/// Live 3D TV stand + cabinet (phosphor is a child of the cabinet).
#[derive(Component, Debug, Clone, Copy)]
pub struct LiveTv;

#[derive(Component, Debug)]
struct HybridBakeCamera;

#[derive(Component, Debug)]
struct HybridPlateMesh;

#[derive(Resource, Debug)]
pub(crate) struct HybridPlates {
    images: [Option<Handle<Image>>; ZOOM_PRESET_COUNT as usize],
    width: u32,
    height: u32,
    showing: Option<u8>,
    /// After intro, bake every preset once before entering idle display.
    warmup_next: u8,
}

#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
enum HybridPhase {
    /// Intro dolly — full 3D both layers.
    #[default]
    Full3d,
    /// Baking preset `warmup_next` (or a single dirty preset).
    Baking { preset: u8, frames_left: u8 },
    /// Plate + live TV.
    Idle,
}

#[derive(Debug, Default)]
pub struct HybridPlugin;

impl Plugin for HybridPlugin {
    fn build(&self, app: &mut App) {
        if !quality::hybrid_enabled() {
            return;
        }
        bevy::log::info!(
            "SPEC_CHUM_ROOM_HYBRID: camera-space plates + live TV ({})",
            quality::preset_label()
        );
        app.init_resource::<HybridPhase>()
            .add_systems(Startup, setup_hybrid.after(crate::camera::setup_camera))
            .add_systems(
                Update,
                (
                    propagate_room_render_layers,
                    hybrid_state_machine,
                    sync_bake_camera_pose,
                    sync_plate_size,
                )
                    .chain(),
            );
    }
}

fn setup_hybrid(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    main: Query<Entity, With<LivingRoomCamera>>,
) {
    let (w, h) = (
        crate::headless::DEFAULT_ROOM_W,
        crate::headless::DEFAULT_ROOM_H,
    );
    commands.insert_resource(HybridPlates {
        images: std::array::from_fn(|_| None),
        width: w,
        height: h,
        showing: None,
        warmup_next: 0,
    });

    let Ok(cam_entity) = main.single() else {
        bevy::log::warn!("hybrid: no LivingRoomCamera at setup");
        return;
    };

    commands.entity(cam_entity).insert((
        RenderLayers::layer(LAYER_LIVE).with(LAYER_ROOM),
        ClusterConfig::Single,
    ));

    let placeholder = images.add(Image::new_uninit(
        Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        TextureFormat::Bgra8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    ));
    let mat = materials.add(StandardMaterial {
        base_color_texture: Some(placeholder),
        unlit: true,
        perceptual_roughness: 1.0,
        reflectance: 0.0,
        alpha_mode: AlphaMode::Opaque,
        // Quad is camera-parented; never cull it on a winding/handedness surprise.
        cull_mode: None,
        ..default()
    });

    let (pw, ph) = plate_extents(w, h);
    let plate = commands
        .spawn((
            Mesh3d(meshes.add(Rectangle::new(pw, ph))),
            MeshMaterial3d(mat),
            // `Rectangle` lies in local XY facing +Z, which already faces a camera
            // looking down −Z. Any `looking_at` here flips it away from the viewer.
            Transform::from_xyz(0.0, 0.0, -PLATE_DISTANCE),
            Visibility::Hidden,
            RenderLayers::layer(LAYER_LIVE),
            HybridPlateMesh,
            Name::new("hybrid_plate_mesh"),
        ))
        .id();
    commands.entity(cam_entity).add_child(plate);

    commands.spawn((
        Camera3d::default(),
        Msaa::Off,
        ClusterConfig::Single,
        Camera {
            order: -2,
            is_active: false,
            clear_color: ClearColorConfig::Custom(Color::srgb(0.012, 0.012, 0.016)),
            ..default()
        },
        Projection::Perspective(PerspectiveProjection {
            fov: PLATE_FOV,
            ..default()
        }),
        RenderLayers::layer(LAYER_ROOM),
        HybridBakeCamera,
        Name::new("hybrid_bake_camera"),
    ));
}

fn plate_extents(pixel_w: u32, pixel_h: u32) -> (f32, f32) {
    let aspect = pixel_w.max(1) as f32 / pixel_h.max(1) as f32;
    let half_h = (PLATE_FOV * 0.5).tan() * PLATE_DISTANCE;
    let half_w = half_h * aspect;
    (half_w * 2.0, half_h * 2.0)
}

fn propagate_room_render_layers(
    roots: Query<Entity, With<RoomStatic>>,
    children: Query<&Children>,
    layers: Query<&RenderLayers>,
    mut commands: Commands,
) {
    let want = RenderLayers::layer(LAYER_ROOM);
    for root in &roots {
        let mut stack = vec![root];
        while let Some(e) = stack.pop() {
            if layers.get(e).ok() != Some(&want) {
                commands.entity(e).insert(want.clone());
            }
            if let Ok(ch) = children.get(e) {
                for child in ch.iter() {
                    stack.push(child);
                }
            }
        }
    }
}

fn ensure_plate_image(
    plates: &mut HybridPlates,
    images: &mut Assets<Image>,
    preset: u8,
) -> Handle<Image> {
    let idx = preset as usize;
    if let Some(h) = &plates.images[idx] {
        return h.clone();
    }
    let mut image = Image::new_uninit(
        Extent3d {
            width: plates.width.max(1),
            height: plates.height.max(1),
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        TextureFormat::Bgra8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.usage |=
        TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING;
    let handle = images.add(image);
    plates.images[idx] = Some(handle.clone());
    handle
}

fn set_vis(commands: &mut Commands, entities: impl IntoIterator<Item = Entity>, visible: bool) {
    let v = if visible {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };
    for e in entities {
        commands.entity(e).insert(v);
    }
}

fn show_plate(
    commands: &mut Commands,
    materials: &mut Assets<StandardMaterial>,
    plate: &mut Query<
        (&mut MeshMaterial3d<StandardMaterial>, &mut Visibility),
        With<HybridPlateMesh>,
    >,
    plates: &HybridPlates,
    preset: u8,
) {
    let Some(handle) = plates.images[preset as usize].clone() else {
        return;
    };
    if let Ok((mat_h, mut vis)) = plate.single_mut() {
        if let Some(mut mat) = materials.get_mut(&mat_h.0) {
            mat.base_color_texture = Some(handle);
        }
        *vis = Visibility::Visible;
    }
    let _ = commands;
}

// Bevy hybrid phase machine: many Queries + resources by design (#171).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn hybrid_state_machine(
    locked: Option<Res<CameraLocked>>,
    zoom: Res<CameraZoom>,
    mut phase: ResMut<HybridPhase>,
    mut plates: ResMut<HybridPlates>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
    room: Query<Entity, With<RoomStatic>>,
    live: Query<Entity, With<LiveTv>>,
    mut bake_cam: Query<(Entity, &mut Camera), With<HybridBakeCamera>>,
    mut plate: Query<
        (&mut MeshMaterial3d<StandardMaterial>, &mut Visibility),
        With<HybridPlateMesh>,
    >,
    mut main_cam: Query<&mut Camera, (With<LivingRoomCamera>, Without<HybridBakeCamera>)>,
    mut main_layers: Query<&mut RenderLayers, With<LivingRoomCamera>>,
) {
    let room_ids: Vec<Entity> = room.iter().collect();
    let live_ids: Vec<Entity> = live.iter().collect();

    // Intro: full 3D, no plate.
    if locked.is_none() {
        if *phase != HybridPhase::Full3d {
            *phase = HybridPhase::Full3d;
            plates.showing = None;
            plates.warmup_next = 0;
            set_vis(&mut commands, room_ids.iter().copied(), true);
            set_vis(&mut commands, live_ids.iter().copied(), true);
            if let Ok((_, mut c)) = bake_cam.single_mut() {
                c.is_active = false;
            }
            if let Ok((_, mut vis)) = plate.single_mut() {
                *vis = Visibility::Hidden;
            }
            if let Ok(mut layers) = main_layers.single_mut() {
                *layers = RenderLayers::layer(LAYER_LIVE).with(LAYER_ROOM);
            }
            if let Ok(mut c) = main_cam.single_mut() {
                c.is_active = true;
                c.clear_color = ClearColorConfig::Custom(Color::srgb(0.012, 0.012, 0.016));
            }
        }
        return;
    }

    if matches!(*phase, HybridPhase::Full3d) {
        // Kick warmup bake of preset 0.
        start_bake(
            &mut phase,
            &mut plates,
            &mut images,
            &mut commands,
            &room_ids,
            &live_ids,
            &mut bake_cam,
            &mut plate,
            &mut main_cam,
            0,
        );
        return;
    }

    match *phase {
        HybridPhase::Full3d => {}
        HybridPhase::Baking {
            preset: p,
            frames_left,
        } => {
            if frames_left > 1 {
                *phase = HybridPhase::Baking {
                    preset: p,
                    frames_left: frames_left - 1,
                };
                return;
            }
            if let Ok((_, mut cam)) = bake_cam.single_mut() {
                cam.is_active = false;
            }
            bevy::log::info!("hybrid: baked zoom plate {p}");

            // Continue warmup through all presets, then idle.
            let next = p.saturating_add(1);
            if next < ZOOM_PRESET_COUNT {
                plates.warmup_next = next;
                start_bake(
                    &mut phase,
                    &mut plates,
                    &mut images,
                    &mut commands,
                    &room_ids,
                    &live_ids,
                    &mut bake_cam,
                    &mut plate,
                    &mut main_cam,
                    next,
                );
                return;
            }

            plates.showing = Some(zoom.target);
            apply_hybrid_display(
                &mut commands,
                &mut materials,
                &room_ids,
                &live_ids,
                &mut plate,
                &mut main_cam,
                &mut main_layers,
                &plates,
                zoom.target,
            );
            *phase = HybridPhase::Idle;
        }
        HybridPhase::Idle => {
            let preset = zoom.target;
            if plates.showing == Some(preset) {
                return;
            }
            if plates.images[preset as usize].is_some() {
                plates.showing = Some(preset);
                apply_hybrid_display(
                    &mut commands,
                    &mut materials,
                    &room_ids,
                    &live_ids,
                    &mut plate,
                    &mut main_cam,
                    &mut main_layers,
                    &plates,
                    preset,
                );
                return;
            }
            // Shouldn't happen after warmup; bake on demand.
            start_bake(
                &mut phase,
                &mut plates,
                &mut images,
                &mut commands,
                &room_ids,
                &live_ids,
                &mut bake_cam,
                &mut plate,
                &mut main_cam,
                preset,
            );
        }
    }
}

// Bake kickoff shares plate/camera entity sets with the hybrid machine (#171).
#[allow(clippy::too_many_arguments)]
fn start_bake(
    phase: &mut HybridPhase,
    plates: &mut HybridPlates,
    images: &mut Assets<Image>,
    commands: &mut Commands,
    room_ids: &[Entity],
    live_ids: &[Entity],
    bake_cam: &mut Query<(Entity, &mut Camera), With<HybridBakeCamera>>,
    plate: &mut Query<
        (&mut MeshMaterial3d<StandardMaterial>, &mut Visibility),
        With<HybridPlateMesh>,
    >,
    main_cam: &mut Query<&mut Camera, (With<LivingRoomCamera>, Without<HybridBakeCamera>)>,
    preset: u8,
) {
    let handle = ensure_plate_image(plates, images, preset);
    set_vis(commands, room_ids.iter().copied(), true);
    set_vis(commands, live_ids.iter().copied(), false);
    if let Ok((_, mut vis)) = plate.single_mut() {
        *vis = Visibility::Hidden;
    }
    if let Ok((entity, mut cam)) = bake_cam.single_mut() {
        cam.is_active = true;
        commands
            .entity(entity)
            .insert(RenderTarget::Image(handle.into()));
    }
    // Pause main present during bake frames (brief).
    if let Ok(mut c) = main_cam.single_mut() {
        c.is_active = false;
    }
    *phase = HybridPhase::Baking {
        preset,
        frames_left: 3,
    };
}

// Visibility/layer updates across room + plate entity sets (#171).
#[allow(clippy::too_many_arguments)]
fn apply_hybrid_display(
    commands: &mut Commands,
    materials: &mut Assets<StandardMaterial>,
    room_ids: &[Entity],
    live_ids: &[Entity],
    plate: &mut Query<
        (&mut MeshMaterial3d<StandardMaterial>, &mut Visibility),
        With<HybridPlateMesh>,
    >,
    main_cam: &mut Query<&mut Camera, (With<LivingRoomCamera>, Without<HybridBakeCamera>)>,
    main_layers: &mut Query<&mut RenderLayers, With<LivingRoomCamera>>,
    plates: &HybridPlates,
    preset: u8,
) {
    set_vis(commands, room_ids.iter().copied(), false);
    set_vis(commands, live_ids.iter().copied(), true);
    show_plate(commands, materials, plate, plates, preset);
    if let Ok(mut layers) = main_layers.single_mut() {
        *layers = RenderLayers::layer(LAYER_LIVE);
    }
    if let Ok(mut c) = main_cam.single_mut() {
        // Plate fills the view; clear unused edges if any.
        c.clear_color = ClearColorConfig::Custom(Color::srgb(0.012, 0.012, 0.016));
        c.is_active = true;
        c.order = 0;
    }
}

fn sync_bake_camera_pose(
    phase: Res<HybridPhase>,
    zoom: Res<CameraZoom>,
    main: Query<&Transform, With<LivingRoomCamera>>,
    phosphor: Query<&GlobalTransform, With<crate::crt::CrtPhosphor>>,
    mut bake: Query<&mut Transform, (With<HybridBakeCamera>, Without<LivingRoomCamera>)>,
) {
    if !matches!(*phase, HybridPhase::Baking { .. }) {
        return;
    }
    // Bake from the *target* preset pose (not mid-animation), so plates match idle stops.
    let look = phosphor
        .iter()
        .next()
        .map_or_else(crate::camera::screen_look_at, GlobalTransform::translation);
    let preset = match *phase {
        HybridPhase::Baking { preset, .. } => preset,
        _ => zoom.target,
    };
    let pose = crate::camera::pose_at_zoom(f32::from(preset), look);
    if let Ok(mut dst) = bake.single_mut() {
        *dst = pose;
    }
    let _ = main;
}

fn sync_plate_size(
    plates: Res<HybridPlates>,
    mut meshes: ResMut<Assets<Mesh>>,
    plate: Query<&Mesh3d, With<HybridPlateMesh>>,
) {
    if !plates.is_changed() {
        return;
    }
    let Ok(mesh_h) = plate.single() else {
        return;
    };
    let (pw, ph) = plate_extents(plates.width, plates.height);
    if let Some(mut mesh) = meshes.get_mut(&mesh_h.0) {
        *mesh = Mesh::from(Rectangle::new(pw, ph));
    }
}

/// Headless resize: keep plate pixel size in sync (bake targets).
pub(crate) fn bind_present_target(width: u32, height: u32, plates: Option<&mut HybridPlates>) {
    if let Some(plates) = plates {
        plates.width = width.max(1);
        plates.height = height.max(1);
        // Force re-bake at new resolution on next idle cycle.
        plates.images = std::array::from_fn(|_| None);
        plates.showing = None;
        plates.warmup_next = 0;
    }
}
