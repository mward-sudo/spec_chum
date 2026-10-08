//! Intro dolly + scroll/trackpad zoom presets (CRT fill → back-and-up swing).

use std::time::Instant;

use bevy::camera::Hdr;
use bevy::camera::{Exposure, ShadowLodOrigin};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::light::cluster::ClusterConfig;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;

use crate::crt::CrtPhosphor;
use crate::quality;

const TV_SPOT_FADE_SECS: f32 = 0.40;
const CAMERA_MOVE_DELAY_SECS: f32 = 0.0;
const CAMERA_MOVE_SECS: f32 = 4.40;
const INTRO_SECS: f32 = CAMERA_MOVE_DELAY_SECS + CAMERA_MOVE_SECS;
// Keep a failed or empty WorldAssetRoot from leaving the opening on black forever.
const SCENE_READY_TIMEOUT_SECS: f32 = 3.0;
const PRACTICAL_LIGHT_RISE_DELAY_SECS: f32 = 1.15;
const PRACTICAL_LIGHT_RISE_SECS: f32 = 2.00;
const LIGHT_RISE_DELAY_SECS: f32 = 0.55;
const LIGHT_RISE_SECS: f32 = 2.60;
const CRT_POWER_DELAY_SECS: f32 = 3.55;
const CRT_POWER_SECS: f32 = 0.85;
pub(crate) const OPENING_DIM_FACTOR: f32 = 0.0;

/// Vertical FOV (~49°). Shared across zoom presets.
const LOCKED_FOV: f32 = 0.85;

/// Phosphor mesh height in metres (`crt::PHOSPHOR_H` — aperture + geometric overscan).
const PHOSPHOR_H: f32 = crate::crt::PHOSPHOR_H;
const PHOSPHOR_W: f32 = crate::crt::PHOSPHOR_W;
const DEFAULT_VIEWPORT_ASPECT: f32 = 16.0 / 9.0;
const FRONT_WALL_CAMERA_CLEARANCE: f32 = 0.18;

/// Wall-clock duration for one preset→preset swing (independent of Bevy `Time`).
pub const ZOOM_ANIM_SECS: f32 = 0.20;

/// Minimum time between accepting scroll preset steps (trackpad fires many deltas).
const ZOOM_STEP_COOLDOWN_SECS: f32 = 0.10;

/// Trackpad pixel delta that counts as one preset step (standalone Bevy).
const SCROLL_PIXELS_PER_STEP: f32 = 64.0;

fn clamp01(t: f32) -> f32 {
    t.clamp(0.0, 1.0)
}

/// Quintic smootherstep gives the opening a gentle start and a clean settle.
fn ease_in_out_smoother(t: f32) -> f32 {
    let t = clamp01(t);
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// Ease-out cubic (Penner): `1-(1-t)³` — snappy start, soft settle (scroll zoom).
fn ease_out_cubic(t: f32) -> f32 {
    let t = clamp01(t);
    1.0 - (1.0 - t).powi(3)
}

/// Soft dolly with a restrained lateral / vertical arc (opening camera move).
pub fn lerp_eye_pullback_rise(from: Vec3, to: Vec3, t: f32) -> Vec3 {
    let e = ease_in_out_smoother(t);
    let linear = Vec3::new(
        from.x + (to.x - from.x) * e,
        from.y + (to.y - from.y) * e,
        from.z + (to.z - from.z) * e,
    );
    let arc = (std::f32::consts::PI * e).sin();
    linear + Vec3::new(-0.06 * arc, 0.035 * arc, 0.0)
}

/// Eye-position blend with ease-out cubic — scroll/trackpad preset swings.
fn lerp_eye_zoom(from: Vec3, to: Vec3, t: f32) -> Vec3 {
    let e = ease_out_cubic(t);
    Vec3::new(
        from.x + (to.x - from.x) * e,
        from.y + (to.y - from.y) * e,
        from.z + (to.z - from.z) * e,
    )
}

/// Discrete zoom stops. Index 0 = almost full-screen CRT; last = further back and up.
#[derive(Clone, Copy, Debug)]
struct ZoomPreset {
    /// Fraction of viewport height filled by the phosphor.
    crt_fill: f32,
    /// Camera height above the look-at point (metres) — grows as we pull back.
    y_lift: f32,
}

const ZOOM_PRESETS: [ZoomPreset; 5] = [
    // Near-fill CRT — tube almost fills the view (still a slim chrome margin).
    ZoomPreset {
        crt_fill: 0.85,
        y_lift: 0.0,
    },
    // Close CRT — readable glyphs, clear of toolbar / glass footer.
    ZoomPreset {
        crt_fill: 0.58,
        y_lift: 0.0,
    },
    // Sofa mid — tube + cabinet edge.
    ZoomPreset {
        crt_fill: 0.40,
        y_lift: 0.08,
    },
    // Living-room hero (embed / skip-intro default).
    ZoomPreset {
        crt_fill: 0.26,
        y_lift: 0.18,
    },
    // Doorway / room-wide view. Pull farther back so the seating area is in
    // frame as well as the TV; its look target shifts into the room at preset 4.
    ZoomPreset {
        crt_fill: 0.062,
        y_lift: 1.24,
    },
];

/// Number of scroll/trackpad zoom stops.
pub const ZOOM_PRESET_COUNT: u8 = ZOOM_PRESETS.len() as u8;

/// Marker while the intro camera is still moving.
#[derive(Resource, Debug)]
pub struct CameraIntro {
    pub elapsed: f32,
    end_preset: u8,
}

/// Shared lighting and CRT power-up timeline for the scene opening.
#[derive(Resource, Debug, Clone, Copy)]
pub struct OpeningSequence {
    elapsed_secs: f32,
    scene_wait_secs: f32,
    scene_ready: bool,
}

impl Default for OpeningSequence {
    fn default() -> Self {
        Self {
            elapsed_secs: 0.0,
            scene_wait_secs: 0.0,
            scene_ready: false,
        }
    }
}

impl OpeningSequence {
    pub fn light_gain(self) -> f32 {
        let t = ((self.elapsed_secs - LIGHT_RISE_DELAY_SECS) / LIGHT_RISE_SECS).clamp(0.0, 1.0);
        OPENING_DIM_FACTOR + (1.0 - OPENING_DIM_FACTOR) * ease_in_out_smoother(t)
    }

    pub fn practical_light_gain(self) -> f32 {
        let t = ((self.elapsed_secs - PRACTICAL_LIGHT_RISE_DELAY_SECS) / PRACTICAL_LIGHT_RISE_SECS)
            .clamp(0.0, 1.0);
        OPENING_DIM_FACTOR + (1.0 - OPENING_DIM_FACTOR) * ease_in_out_smoother(t)
    }

    pub fn crt_power(self) -> f32 {
        let t = ((self.elapsed_secs - CRT_POWER_DELAY_SECS) / CRT_POWER_SECS).clamp(0.0, 1.0);
        ease_in_out_smoother(t)
    }

    pub fn tv_spot_gain(self) -> f32 {
        let fade_in = ease_out_cubic(self.elapsed_secs / TV_SPOT_FADE_SECS);
        // Hold the localized TV accent while the room lighting rises, then
        // taper it smoothly as the CRT switches on near the camera settle.
        let fade_out = ease_in_out_smoother((self.elapsed_secs - 2.65) / 1.20);
        5_600.0 * fade_in * (1.0 - fade_out)
    }

    fn set_elapsed(&mut self, elapsed_secs: f32) {
        self.elapsed_secs = elapsed_secs;
        self.scene_wait_secs = SCENE_READY_TIMEOUT_SECS;
        self.scene_ready = true;
    }
    fn advance(&mut self, delta_secs: f32, scene_ready: bool) {
        if !self.scene_ready {
            self.scene_wait_secs =
                (self.scene_wait_secs + delta_secs).min(SCENE_READY_TIMEOUT_SECS);
            self.scene_ready = scene_ready || self.scene_wait_secs >= SCENE_READY_TIMEOUT_SECS;
        }
        if self.scene_ready {
            self.elapsed_secs = (self.elapsed_secs + delta_secs).min(INTRO_SECS);
        }
    }

    fn elapsed(self) -> f32 {
        self.elapsed_secs
    }
}

fn advance_opening_sequence(
    time: Res<Time>,
    mut opening: ResMut<OpeningSequence>,
    scene_roots: Query<(&WorldAssetRoot, Option<&Children>)>,
) {
    // Start the timeline on the first rendered frame. Asset instantiation must
    // not leave the opening stalled on black; the camera-mounted spinner reports
    // loading while the scene catches up.
    let root_count = scene_roots.iter().count();
    let scene_ready = root_count > 0
        && scene_roots
            .iter()
            .all(|(_, children)| children.is_some_and(|children| !children.is_empty()));
    opening.advance(time.delta_secs(), scene_ready);
}

/// Preset selected when the opening camera move settles.
const INTRO_DESTINATION_PRESET: u8 = 0;

/// Present once the camera is locked (zoom presets active).
#[derive(Resource, Debug, Default)]
pub struct CameraLocked;

/// Set by Swift FFI / click to skip the intro dolly (headless has no Bevy mouse).
#[derive(Resource, Debug, Default)]
pub struct IntroSkipRequest(pub bool);

/// Optional override for the preset selected when intro finishes.
#[derive(Resource, Debug, Default)]
pub struct PostIntroZoom(pub Option<u8>);

/// 0 = near full-screen CRT (readable glyphs); higher values pull back into the room.
#[derive(Resource, Debug, Clone, Copy)]
pub struct CrtLookBlend(pub f32);

impl Default for CrtLookBlend {
    fn default() -> Self {
        Self(0.0)
    }
}

/// Discrete zoom target + wall-clock eased display index.
#[derive(Resource, Debug)]
pub struct CameraZoom {
    /// Integer preset 0..[`ZOOM_PRESET_COUNT`]-1 (0 = CRT fill).
    pub target: u8,
    /// Animated index used for posing (continuous).
    pub display: f32,
    anim_from: f32,
    anim_to: f32,
    anim_start: Option<Instant>,
    pub(crate) last_step: Option<Instant>,
}

impl Default for CameraZoom {
    fn default() -> Self {
        Self {
            target: 0,
            display: 0.0,
            anim_from: 0.0,
            anim_to: 0.0,
            anim_start: None,
            last_step: None,
        }
    }
}

impl CameraZoom {
    /// `steps > 0` zooms out (further back); `steps < 0` zooms in (toward CRT).
    ///
    /// Coalesces rapid trackpad events and retargets mid-animation from the
    /// current display pose so motion stays a smooth swing.
    pub fn nudge(&mut self, steps: i32) {
        if steps == 0 {
            return;
        }
        let now = Instant::now();
        if let Some(prev) = self.last_step {
            if now.duration_since(prev).as_secs_f32() < ZOOM_STEP_COOLDOWN_SECS {
                return;
            }
        }
        let max = i32::from(ZOOM_PRESET_COUNT.saturating_sub(1));
        let next = i32::from(self.target)
            .saturating_add(steps.signum())
            .clamp(0, max);
        let next_u = next as u8;
        if next_u == self.target && self.anim_start.is_none() {
            return;
        }
        self.last_step = Some(now);
        self.target = next_u;
        self.anim_from = self.display;
        self.anim_to = f32::from(next_u);
        self.anim_start = Some(now);
    }

    /// Snap immediately to `target` (no ease) — intro skip / resize jumps.
    pub fn snap_to_target(&mut self) {
        self.display = f32::from(self.target);
        self.anim_from = self.display;
        self.anim_to = self.display;
        self.anim_start = None;
    }

    /// True when not mid zoom-ease (safe to bake a plate).
    pub fn is_settled(&self) -> bool {
        self.anim_start.is_none()
    }

    /// Jump to a preset index and snap (embed: start on a readable living-room framing).
    pub fn jump_to(&mut self, preset: u8) {
        let max = ZOOM_PRESET_COUNT.saturating_sub(1);
        self.target = preset.min(max);
        self.snap_to_target();
        self.last_step = None;
    }

    fn tick_animation(&mut self) {
        let Some(start) = self.anim_start else {
            self.display = f32::from(self.target);
            return;
        };
        let u = (start.elapsed().as_secs_f32() / ZOOM_ANIM_SECS).clamp(0.0, 1.0);
        // Linear preset index — axis easing happens in `pose_at_zoom`.
        self.display = self.anim_from + (self.anim_to - self.anim_from) * u;
        if u >= 1.0 {
            self.display = self.anim_to;
            self.anim_start = None;
        }
    }
}

#[derive(Resource, Debug, Default)]
struct ZoomScrollAccum(f32);

#[derive(Component, Debug)]
pub struct LivingRoomCamera;

#[derive(Debug, Default)]
pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        let presentation = crate::cabinet_room::RoomPresentation::from_environment();
        bevy::log::info!("SPEC_CHUM_ROOM_PRESENTATION: {}", presentation.label());
        app.init_resource::<IntroSkipRequest>()
            .insert_resource(presentation)
            .init_resource::<PostIntroZoom>()
            .init_resource::<OpeningSequence>()
            .init_resource::<CameraZoom>()
            .init_resource::<CrtLookBlend>()
            .init_resource::<ZoomScrollAccum>()
            .add_systems(Startup, setup_camera)
            .add_systems(
                Update,
                (
                    advance_opening_sequence,
                    animate_tv_spotlight,
                    animate_loading_spinner,
                    skip_intro,
                    update_intro_camera,
                    zoom_from_scroll,
                    apply_zoom_camera,
                    apply_fixed_camera_frame,
                )
                    .chain(),
            );
    }
}

/// Screen centre matching `crt::setup_crt` (TV on console + phosphor offset).
pub(crate) fn screen_look_at() -> Vec3 {
    crate::crt::crt_screen_world_center()
}

fn camera_aspect(camera: &Camera) -> f32 {
    camera
        .logical_viewport_size()
        .filter(|size| size.x.is_finite() && size.y.is_finite() && size.x > 0.0 && size.y > 0.0)
        .map_or(DEFAULT_VIEWPORT_ASPECT, |size| size.x / size.y)
}

fn distance_for_crt_fill(fov: f32, fill: f32, aspect_ratio: f32) -> f32 {
    let fill = fill.clamp(0.05, 0.95);
    let aspect_ratio = if aspect_ratio.is_finite() && aspect_ratio > 0.0 {
        aspect_ratio
    } else {
        DEFAULT_VIEWPORT_ASPECT
    };
    // Perspective FOV is vertical. Increase the visible height when the
    // viewport is too narrow to contain the phosphor's full horizontal span.
    let visible_h = (PHOSPHOR_H / fill).max(PHOSPHOR_W / (aspect_ratio * fill));
    visible_h / (2.0 * (fov * 0.5).tan())
}

fn preset_eye(look: Vec3, preset_index: usize, aspect_ratio: f32) -> Vec3 {
    let preset = ZOOM_PRESETS[preset_index];
    let dist = distance_for_crt_fill(LOCKED_FOV, preset.crt_fill, aspect_ratio);
    let dist = if preset_index == ZOOM_PRESETS.len() - 1 {
        let room_edge = crate::room::ROOM_D * 0.5 - FRONT_WALL_CAMERA_CLEARANCE;
        dist.min((room_edge - look.z).max(0.0))
    } else {
        dist
    };
    look + Vec3::new(0.0, preset.y_lift, dist)
}

fn room_wide_look(look: Vec3) -> Vec3 {
    look + Vec3::new(0.0, -0.21, 2.02)
}

fn intro_look_target(look: Vec3, end_preset: u8, t: f32) -> Vec3 {
    let end_target = if end_preset == ZOOM_PRESET_COUNT - 1 {
        room_wide_look(look)
    } else {
        look
    };
    room_wide_look(look).lerp(end_target, ease_in_out_smoother(t))
}

/// Camera pose for a (possibly fractional) preset index along the back-and-up path.
pub fn pose_at_zoom(t: f32, look: Vec3) -> Transform {
    pose_at_zoom_for_aspect(t, look, DEFAULT_VIEWPORT_ASPECT)
}

fn pose_at_zoom_for_aspect(t: f32, look: Vec3, aspect_ratio: f32) -> Transform {
    let max_i = (ZOOM_PRESETS.len() - 1) as f32;
    let t = t.clamp(0.0, max_i);
    let i0 = t.floor() as usize;
    let i1 = (i0 + 1).min(ZOOM_PRESETS.len() - 1);
    let f = (t - i0 as f32).clamp(0.0, 1.0);

    let p0 = preset_eye(look, i0, aspect_ratio);
    let p1 = preset_eye(look, i1, aspect_ratio);
    let pos = lerp_eye_zoom(p0, p1, f);
    let target = if i1 == ZOOM_PRESETS.len() - 1 && i0 != i1 {
        look.lerp(room_wide_look(look), ease_out_cubic(f))
    } else if i0 == ZOOM_PRESETS.len() - 1 {
        room_wide_look(look)
    } else {
        look
    };
    Transform::from_translation(pos).looking_at(target, Vec3::Y)
}

pub(crate) fn setup_camera(
    mut commands: Commands,
    post_zoom: Res<PostIntroZoom>,
    mut opening: ResMut<OpeningSequence>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    presentation: Res<crate::cabinet_room::RoomPresentation>,
) {
    let look = screen_look_at();
    let end_preset = post_zoom
        .0
        .unwrap_or(INTRO_DESTINATION_PRESET)
        .min(ZOOM_PRESET_COUNT - 1);
    // Keep the high-angle establishing position inside the room shell. Starting
    // above the ceiling made the TV and its spotlight occluded until the camera
    // passed through the ceiling, which read as a sudden lighting pop.
    let fixed_cabinet = *presentation == crate::cabinet_room::RoomPresentation::FixedCabinet;
    let start = if fixed_cabinet {
        // Fixed framing has no intro transition, so begin with the room and
        // CRT fully powered while scene assets continue loading independently.
        opening.elapsed_secs = INTRO_SECS;
        commands.insert_resource(CameraLocked);
        fixed_cabinet_pose(DEFAULT_VIEWPORT_ASPECT)
    } else {
        let start_eye = preset_eye(look, ZOOM_PRESETS.len() - 1, DEFAULT_VIEWPORT_ASPECT);
        commands.insert_resource(CameraIntro {
            elapsed: 0.0,
            end_preset,
        });
        Transform::from_translation(start_eye).looking_at(room_wide_look(look), Vec3::Y)
    };
    commands.insert_resource(CameraZoom::default());
    commands.spawn((
        SpotLight {
            color: Color::srgb(1.0, 0.70, 0.43),
            intensity: 0.0,
            range: 2.4,
            radius: 0.16,
            inner_angle: 0.24,
            outer_angle: 0.68,
            shadow_maps_enabled: false,
            ..default()
        },
        // Use the center sconce bulb as a motivated source for the TV-wall
        // accent rather than shining a hidden lamp directly into the glass.
        Transform::from_translation(Vec3::new(
            0.0,
            1.85 + 0.107,
            -crate::room::ROOM_D * 0.5 + 0.03 + 0.192,
        ))
        .looking_at(
            crate::room::TV_STAND_POS + Vec3::new(0.0, 1.42, 0.28),
            Vec3::Y,
        ),
        OpeningTvSpotlight,
        Name::new("opening_tv_spotlight"),
    ));

    let mut cam = commands.spawn((
        Camera3d::default(),
        Hdr,
        // Headless / image targets have no window camera; mark LOD origin explicitly.
        ShadowLodOrigin,
        // Few lights in a small room — skip tiled cluster allocation (cheap win on Metal).
        ClusterConfig::Single,
        // Midway between INDOOR (7.0) and BLENDER (9.7): furniture readable,
        // CRT phosphor not crushed by room glare when zoomed out (#233).
        Exposure { ev100: 8.2 },
        quality::msaa_samples(),
        Camera {
            clear_color: ClearColorConfig::Custom(if crate::crt::bright_debug_enabled() {
                Color::srgb(0.18, 0.18, 0.20)
            } else {
                Color::srgb(0.012, 0.012, 0.016)
            }),
            ..default()
        },
        Projection::Perspective(PerspectiveProjection {
            fov: LOCKED_FOV,
            ..default()
        }),
        Tonemapping::TonyMcMapface,
        start,
        LivingRoomCamera,
        Name::new("living_room_camera"),
    ));
    if quality::bloom_enabled() {
        cam.insert(Bloom {
            intensity: 0.08,
            max_mip_dimension: quality::bloom_max_mip_dimension(),
            ..Bloom::NATURAL
        });
    }

    // Camera-space loading mark: visible over the black clear while glTF scenes
    // instantiate, then hidden as soon as the complete room is ready.
    let camera_entity = cam.id();
    let spinner = commands
        .spawn((
            Transform::from_xyz(0.82, -0.46, -1.15),
            Visibility::Visible,
            OpeningLoadingSpinner,
            Name::new("room_loading_spinner"),
        ))
        .id();
    let bead_mesh = meshes.add(Sphere::new(0.008).mesh().uv(8, 6));
    commands.entity(spinner).with_children(|parent| {
        for i in 0..8 {
            let angle = i as f32 * std::f32::consts::TAU / 8.0;
            let level = (i + 1) as f32 / 8.0;
            let material = materials.add(StandardMaterial {
                base_color: Color::srgb(
                    0.20 + level * 0.38,
                    0.44 + level * 0.38,
                    0.58 + level * 0.35,
                ),
                emissive: LinearRgba::rgb(0.12 + level * 0.8, 0.3 + level * 1.5, 0.5 + level * 2.0),
                unlit: true,
                ..default()
            });
            parent.spawn((
                Mesh3d(bead_mesh.clone()),
                MeshMaterial3d(material),
                Transform::from_xyz(angle.cos() * 0.042, angle.sin() * 0.042, 0.0),
                Name::new("room_loading_spinner_bead"),
            ));
        }
    });
    commands.entity(camera_entity).add_child(spinner);
}

#[derive(Component)]
struct OpeningLoadingSpinner;

fn animate_loading_spinner(
    time: Res<Time>,
    opening: Res<OpeningSequence>,
    mut spinner: Query<(&mut Transform, &mut Visibility), With<OpeningLoadingSpinner>>,
) {
    let Ok((mut transform, mut visibility)) = spinner.single_mut() else {
        return;
    };
    *visibility = if opening.scene_ready {
        Visibility::Hidden
    } else {
        Visibility::Visible
    };
    transform.rotation = Quat::from_rotation_z(-time.elapsed_secs() * 4.0);
}

#[derive(Component)]
struct OpeningTvSpotlight;

fn animate_tv_spotlight(
    opening: Res<OpeningSequence>,
    mut spots: Query<&mut SpotLight, With<OpeningTvSpotlight>>,
) {
    let gain = opening.tv_spot_gain();
    for mut spot in &mut spots {
        spot.intensity = gain;
    }
}

// Bevy system: intro skip needs input + camera + zoom resources together (#171).
#[allow(clippy::too_many_arguments)]
fn skip_intro(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut skip_req: ResMut<IntroSkipRequest>,
    mut post_zoom: ResMut<PostIntroZoom>,
    intro: Option<ResMut<CameraIntro>>,
    mut commands: Commands,
    mut cams: Query<(&Camera, &mut Transform), With<LivingRoomCamera>>,
    mut zoom: ResMut<CameraZoom>,
    mut opening: ResMut<OpeningSequence>,
) {
    let Some(mut intro) = intro else {
        skip_req.0 = false;
        return;
    };
    let skip = skip_req.0
        || keys.just_pressed(KeyCode::Escape)
        || keys.just_pressed(KeyCode::Space)
        || mouse.just_pressed(MouseButton::Left);
    skip_req.0 = false;
    if !skip {
        return;
    }
    intro.elapsed = INTRO_SECS;
    opening.set_elapsed(INTRO_SECS);
    let preset = post_zoom.0.take().unwrap_or(INTRO_DESTINATION_PRESET);
    *zoom = CameraZoom::default();
    zoom.jump_to(preset);
    if let Ok((camera, mut tf)) = cams.single_mut() {
        *tf = pose_at_zoom_for_aspect(f32::from(preset), screen_look_at(), camera_aspect(camera));
    }
    commands.insert_resource(CameraLocked);
    commands.remove_resource::<CameraIntro>();
}

fn update_intro_camera(
    opening: Res<OpeningSequence>,
    mut post_zoom: ResMut<PostIntroZoom>,
    intro: Option<ResMut<CameraIntro>>,
    mut commands: Commands,
    mut cams: Query<(&Camera, &mut Transform), With<LivingRoomCamera>>,
    phosphor: Query<&GlobalTransform, With<CrtPhosphor>>,
    mut zoom: ResMut<CameraZoom>,
) {
    let Some(mut intro) = intro else {
        return;
    };
    intro.elapsed = opening.elapsed();
    let t = ((intro.elapsed - CAMERA_MOVE_DELAY_SECS) / CAMERA_MOVE_SECS).clamp(0.0, 1.0);

    let look_at = phosphor
        .iter()
        .next()
        .map_or_else(screen_look_at, GlobalTransform::translation);

    let end_preset = post_zoom
        .0
        .unwrap_or(intro.end_preset)
        .min(ZOOM_PRESET_COUNT - 1);
    // Recompute the endpoint from the live CRT transform. The GLTF scene can
    // settle by a few pixels after startup; matching that live target prevents
    // a final-frame correction when the zoom controls take over.
    let Ok((camera, mut tf)) = cams.single_mut() else {
        return;
    };
    let aspect = camera_aspect(camera);
    let end = pose_at_zoom_for_aspect(f32::from(end_preset), look_at, aspect);
    let start_eye = preset_eye(look_at, ZOOM_PRESETS.len() - 1, aspect);
    {
        let pos = lerp_eye_pullback_rise(start_eye, end.translation, t);
        let target = intro_look_target(look_at, end_preset, t);
        *tf = Transform::from_translation(pos).looking_at(target, Vec3::Y);
    }

    if t >= 1.0 {
        let preset = post_zoom.0.take().unwrap_or(intro.end_preset);
        *zoom = CameraZoom::default();
        zoom.jump_to(preset);
        commands.insert_resource(CameraLocked);
        commands.remove_resource::<CameraIntro>();
    }
}

fn zoom_from_scroll(
    mut ev: MessageReader<MouseWheel>,
    mut zoom: ResMut<CameraZoom>,
    mut acc: ResMut<ZoomScrollAccum>,
    locked: Option<Res<CameraLocked>>,
    presentation: Res<crate::cabinet_room::RoomPresentation>,
) {
    if locked.is_none() || *presentation == crate::cabinet_room::RoomPresentation::FixedCabinet {
        return;
    }
    for e in ev.read() {
        let dy = match e.unit {
            MouseScrollUnit::Line => e.y,
            MouseScrollUnit::Pixel => e.y / SCROLL_PIXELS_PER_STEP,
        };
        // Scroll / swipe up → zoom in (toward CRT); down → pull back.
        acc.0 += dy;
    }
    // One preset step per crossing; discard remainder so flicks don't skip stops.
    if acc.0 >= 1.0 {
        acc.0 = 0.0;
        zoom.nudge(-1);
    } else if acc.0 <= -1.0 {
        acc.0 = 0.0;
        zoom.nudge(1);
    }
}

/// Apply current zoom pose (also callable from headless FFI after snap).
// Bevy Queries + zoom resources; splitting obscures the pose update (#171).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn apply_zoom_camera(
    locked: Option<Res<CameraLocked>>,
    mut zoom: ResMut<CameraZoom>,
    mut look_blend: ResMut<CrtLookBlend>,
    mut cams: Query<(&Camera, &mut Transform, Option<&mut Bloom>), With<LivingRoomCamera>>,
    phosphor: Query<&GlobalTransform, With<CrtPhosphor>>,
    mut phosphor_tf: Query<&mut Transform, (With<CrtPhosphor>, Without<LivingRoomCamera>)>,
    mut glass_tf: Query<
        &mut Transform,
        (
            With<crate::crt::CrtGlass>,
            Without<LivingRoomCamera>,
            Without<CrtPhosphor>,
        ),
    >,
    mut glass: Query<&mut MeshMaterial3d<StandardMaterial>, With<crate::crt::CrtGlass>>,
    mut std_mats: ResMut<Assets<StandardMaterial>>,
    presentation: Res<crate::cabinet_room::RoomPresentation>,
) {
    if locked.is_none() || *presentation == crate::cabinet_room::RoomPresentation::FixedCabinet {
        return;
    }
    zoom.tick_animation();

    let look = phosphor
        .iter()
        .next()
        .map_or_else(screen_look_at, GlobalTransform::translation);
    let max_i = f32::from(ZOOM_PRESET_COUNT.saturating_sub(1)).max(1.0);
    let t = (zoom.display / max_i).clamp(0.0, 1.0);
    look_blend.0 = t;
    if let Ok((camera, mut tf, bloom)) = cams.single_mut() {
        *tf = pose_at_zoom_for_aspect(zoom.display, look, camera_aspect(camera));
        if let Some(mut bloom) = bloom {
            // Mild pull-back halation — strong bloom washes the CRT face (#233).
            bloom.intensity = 0.04 + t * 0.06;
        }
    }
    // Slightly flatten the tube when CRT-fill (readable glyphs); full soft dome
    // when pulled back. Never squash hard enough to read as a flat plane.
    let z_scale = 0.72 + t * 0.28;
    for mut ptf in &mut phosphor_tf {
        ptf.scale = Vec3::new(1.0, 1.0, z_scale);
    }
    for mut gtf in &mut glass_tf {
        gtf.scale = Vec3::new(1.0, 1.0, z_scale);
    }
    // Keep the glass reflection stable across the intro → locked-camera handoff.
    // Setting alpha from zoom made it jump from the authored value to zero at preset 0.
    let glass_a = 0.06;
    for handle in &mut glass {
        if let Some(mut mat) = std_mats.get_mut(&handle.0) {
            mat.base_color = Color::srgba(0.55, 0.65, 0.75, glass_a);
        }
    }
}

fn fixed_cabinet_pose(aspect: f32) -> Transform {
    let look = Vec3::new(
        crate::room::TV_STAND_POS.x,
        crate::cabinet_room::HERO_CENTER_Y,
        crate::crt::crt_screen_world_center().z,
    );
    let distance = crate::cabinet_room::camera_distance(LOCKED_FOV, aspect);
    Transform::from_translation(look + Vec3::Z * distance).looking_at(look, Vec3::Y)
}

/// Fixed mode adapts its viewing distance to the resized viewport while
/// preserving its authored center and direction (there are no user controls).
fn apply_fixed_camera_frame(
    presentation: Res<crate::cabinet_room::RoomPresentation>,
    mut cameras: Query<(&Camera, &mut Transform), With<LivingRoomCamera>>,
) {
    if *presentation != crate::cabinet_room::RoomPresentation::FixedCabinet {
        return;
    }
    for (camera, mut transform) in &mut cameras {
        *transform = fixed_cabinet_pose(camera_aspect(camera));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn living_room_exposure_between_indoor_and_blender() {
        // setup_camera inserts ev100 8.2 (#233 glare balance).
        const EV: f32 = 8.2;
        const {
            assert!(EV > Exposure::EV100_INDOOR);
            assert!(EV < Exposure::EV100_BLENDER);
        }
        let _ = EV;
    }

    #[test]
    fn ease_out_cubic_endpoints() {
        assert!((ease_out_cubic(0.0) - 0.0).abs() < f32::EPSILON);
        assert!((ease_out_cubic(1.0) - 1.0).abs() < f32::EPSILON);
        // Snappier than ease-in-out at t=0.25.
        assert!(ease_out_cubic(0.25) > ease_in_out_smoother(0.25));
    }

    #[test]
    fn easing_endpoints() {
        assert!((ease_in_out_smoother(0.0) - 0.0).abs() < f32::EPSILON);
        assert!((ease_in_out_smoother(1.0) - 1.0).abs() < f32::EPSILON);
        let from = Vec3::new(0.0, 0.0, 1.0);
        let to = Vec3::new(0.0, 1.0, 3.0);
        let start = lerp_eye_pullback_rise(from, to, 0.0);
        let end = lerp_eye_pullback_rise(from, to, 1.0);
        assert!(start.distance(from) < 0.001);
        assert!(end.distance(to) < 0.001);
    }

    #[test]
    fn intro_target_matches_selected_preset() {
        let look = Vec3::new(0.2, 1.1, -2.0);
        assert!(intro_look_target(look, 0, 1.0).distance(look) < 0.001);
        assert!(intro_look_target(look, ZOOM_PRESET_COUNT - 2, 1.0).distance(look) < 0.001);
        assert!(
            intro_look_target(look, ZOOM_PRESET_COUNT - 1, 1.0).distance(room_wide_look(look))
                < 0.001
        );
    }

    #[test]
    fn opening_waits_for_scene_or_bounded_timeout() {
        let mut opening = OpeningSequence::default();
        opening.advance(1.0, false);
        assert!(!opening.scene_ready);
        assert_eq!(opening.elapsed(), 0.0);

        opening.advance(1.0, false);
        assert!(!opening.scene_ready);
        opening.advance(1.0, false);
        assert!(opening.scene_ready);
        assert_eq!(opening.elapsed(), 1.0);
    }

    #[test]
    fn ease_in_out_smoother_midpoint() {
        assert!((ease_in_out_smoother(0.5) - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn preset0_is_near_fullscreen_crt() {
        // Closest stop: tube nearly fills the view (still a slim chrome margin).
        assert!(ZOOM_PRESETS[0].crt_fill >= 0.72);
        assert!(ZOOM_PRESETS[0].crt_fill <= 0.85);
        assert_eq!(ZOOM_PRESETS[0].y_lift, 0.0);
        assert_eq!(ZOOM_PRESET_COUNT, 5);
    }

    #[test]
    fn living_room_hero_is_preset_3() {
        assert!((ZOOM_PRESETS[3].crt_fill - 0.26).abs() < f32::EPSILON);
        assert!((ZOOM_PRESETS[3].y_lift - 0.18).abs() < f32::EPSILON);
    }

    #[test]
    fn pullback_raises_and_shrinks_fill() {
        let a = ZOOM_PRESETS[0];
        let last = ZOOM_PRESETS.len() - 1;
        let b = ZOOM_PRESETS[last];
        assert!(b.crt_fill < a.crt_fill);
        assert!(b.y_lift > a.y_lift);
        let look = Vec3::ZERO;
        let near = preset_eye(look, 0, DEFAULT_VIEWPORT_ASPECT);
        let far = preset_eye(look, last, DEFAULT_VIEWPORT_ASPECT);
        assert!(far.z > near.z);
        assert!(far.y > near.y);
    }

    #[test]
    fn nudge_clamps() {
        let mut z = CameraZoom::default();
        z.nudge(-10);
        assert_eq!(z.target, 0);
        // Cooldown would block rapid nudges — advance last_step artificially.
        for _ in 0..i32::from(ZOOM_PRESET_COUNT) {
            z.last_step = None;
            z.nudge(1);
        }
        assert_eq!(z.target, ZOOM_PRESET_COUNT - 1);
    }

    #[test]
    fn anim_eases_toward_target() {
        let mut z = CameraZoom::default();
        z.nudge(1);
        assert!(z.anim_start.is_some());
        assert!((z.anim_to - 1.0).abs() < f32::EPSILON);
        // Simulate near end of anim by rewriting start into the past.
        z.anim_start =
            Instant::now().checked_sub(std::time::Duration::from_secs_f32(ZOOM_ANIM_SECS));
        z.tick_animation();
        assert!((z.display - 1.0).abs() < 0.01);
        assert!(z.anim_start.is_none());
    }

    #[test]
    fn snap_clears_animation() {
        let mut z = CameraZoom::default();
        z.nudge(2);
        assert!(z.anim_start.is_some());
        z.snap_to_target();
        assert!(z.anim_start.is_none());
        assert!((z.display - f32::from(z.target)).abs() < f32::EPSILON);
    }

    #[test]
    fn distance_matches_fill() {
        let fill = 0.28;
        let d = distance_for_crt_fill(LOCKED_FOV, fill, DEFAULT_VIEWPORT_ASPECT);
        let visible_h = 2.0 * d * (LOCKED_FOV * 0.5).tan();
        let got = PHOSPHOR_H / visible_h;
        assert!((got - fill).abs() < 0.01, "fill={got}");
    }

    #[test]
    fn phosphor_bounds_fit_wide_standard_and_narrow_viewports() {
        let fill = ZOOM_PRESETS[0].crt_fill;
        for aspect in [16.0 / 9.0, 4.0 / 3.0, 9.0 / 16.0] {
            let distance = distance_for_crt_fill(LOCKED_FOV, fill, aspect);
            let visible_h = 2.0 * distance * (LOCKED_FOV * 0.5).tan();
            let visible_w = visible_h * aspect;
            let vertical_fill = PHOSPHOR_H / visible_h;
            let horizontal_fill = PHOSPHOR_W / visible_w;

            assert!(
                vertical_fill <= fill + 0.001,
                "aspect={aspect}: {vertical_fill}"
            );
            assert!(
                horizontal_fill <= fill + 0.001,
                "aspect={aspect}: {horizontal_fill}"
            );
            assert!(
                (vertical_fill.max(horizontal_fill) - fill).abs() < 0.001,
                "aspect={aspect}: vertical={vertical_fill}, horizontal={horizontal_fill}"
            );
        }
    }

    #[test]
    fn narrow_viewport_moves_camera_back_without_changing_zoom_preset() {
        let look = Vec3::ZERO;
        let wide = pose_at_zoom_for_aspect(0.0, look, 16.0 / 9.0).translation.z;
        let narrow = pose_at_zoom_for_aspect(0.0, look, 9.0 / 16.0).translation.z;
        assert!(narrow > wide);
    }

    #[test]
    fn room_wide_zoom_stays_inside_front_boundary_at_any_viewport_aspect() {
        let look = screen_look_at();
        let furthest = (ZOOM_PRESETS.len() - 1) as f32;
        let front_limit = crate::room::ROOM_D * 0.5 - FRONT_WALL_CAMERA_CLEARANCE;

        for aspect in [16.0 / 9.0, 4.0 / 3.0, 9.0 / 16.0] {
            let camera = pose_at_zoom_for_aspect(furthest, look, aspect);
            assert!(
                camera.translation.z <= front_limit,
                "aspect={aspect}: camera z={} crossed front limit {front_limit}",
                camera.translation.z
            );
        }
    }
}
