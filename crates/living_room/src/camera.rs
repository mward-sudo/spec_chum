//! Intro dolly + scroll/trackpad zoom presets (CRT fill → back-and-up swing).

use std::time::Instant;

use bevy::anti_alias::fxaa::{Fxaa, Sensitivity};
use bevy::camera::{Exposure, ShadowLodOrigin};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::light::cluster::ClusterConfig;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;

use crate::crt::CrtPhosphor;
use crate::quality;

const INTRO_SECS: f32 = 1.5;

/// Vertical FOV (~49°). Shared across zoom presets.
const LOCKED_FOV: f32 = 0.85;

/// Phosphor mesh height in metres (`crt::PHOSPHOR_H` — aperture + geometric overscan).
const PHOSPHOR_H: f32 = crate::crt::PHOSPHOR_H;

/// Wall-clock duration for one preset→preset swing (independent of Bevy `Time`).
pub const ZOOM_ANIM_SECS: f32 = 0.20;

/// Minimum time between accepting scroll preset steps (trackpad fires many deltas).
const ZOOM_STEP_COOLDOWN_SECS: f32 = 0.10;

/// Trackpad pixel delta that counts as one preset step (standalone Bevy).
const SCROLL_PIXELS_PER_STEP: f32 = 64.0;

fn clamp01(t: f32) -> f32 {
    t.clamp(0.0, 1.0)
}

/// Ease-in-out cubic (Penner): `4t³` if t<0.5 else `1-(−2t+2)³/2`.
fn ease_in_out_cubic(t: f32) -> f32 {
    let t = clamp01(t);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

/// Ease-out cubic (Penner): `1-(1-t)³` — snappy start, soft settle (scroll zoom).
fn ease_out_cubic(t: f32) -> f32 {
    let t = clamp01(t);
    1.0 - (1.0 - t).powi(3)
}

/// Eye-position blend with ease-in-out cubic on all axes (intro dolly).
pub fn lerp_eye_pullback_rise(from: Vec3, to: Vec3, t: f32) -> Vec3 {
    let e = ease_in_out_cubic(t);
    Vec3::new(
        from.x + (to.x - from.x) * e,
        from.y + (to.y - from.y) * e,
        from.z + (to.z - from.z) * e,
    )
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
        crt_fill: 0.78,
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
    // Doorway / back-and-up swing.
    ZoomPreset {
        crt_fill: 0.13,
        y_lift: 0.55,
    },
];

/// Number of scroll/trackpad zoom stops.
pub const ZOOM_PRESET_COUNT: u8 = ZOOM_PRESETS.len() as u8;

/// Marker while the intro camera is still moving.
#[derive(Resource, Debug)]
pub struct CameraIntro {
    pub elapsed: f32,
    pub start: Transform,
    pub end: Transform,
}

/// Present once the camera is locked (zoom presets active).
#[derive(Resource, Debug, Default)]
pub struct CameraLocked;

/// Set by Swift FFI / click to skip the intro dolly (headless has no Bevy mouse).
#[derive(Resource, Debug, Default)]
pub struct IntroSkipRequest(pub bool);

/// After intro skip/finish, jump to this preset (embed starts on living-room hero).
#[derive(Resource, Debug, Default)]
pub struct PostIntroZoom(pub Option<u8>);

/// 0 = near full-screen CRT (readable glyphs); 1 = pulled-back living-room CRT look.
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
        app.init_resource::<IntroSkipRequest>()
            .init_resource::<PostIntroZoom>()
            .init_resource::<CameraZoom>()
            .init_resource::<CrtLookBlend>()
            .init_resource::<ZoomScrollAccum>()
            .add_systems(Startup, setup_camera)
            .add_systems(
                Update,
                (
                    skip_intro,
                    update_intro_camera,
                    zoom_from_scroll,
                    apply_zoom_camera,
                )
                    .chain(),
            );
    }
}

/// Screen centre matching `crt::setup_crt` (TV on console + phosphor offset).
pub(crate) fn screen_look_at() -> Vec3 {
    crate::crt::crt_screen_world_center()
}

fn distance_for_crt_fill(fov: f32, fill: f32) -> f32 {
    let fill = fill.clamp(0.05, 0.95);
    let visible_h = PHOSPHOR_H / fill;
    visible_h / (2.0 * (fov * 0.5).tan())
}

fn preset_eye(look: Vec3, preset: ZoomPreset) -> Vec3 {
    let dist = distance_for_crt_fill(LOCKED_FOV, preset.crt_fill);
    look + Vec3::new(0.0, preset.y_lift, dist)
}

/// Camera pose for a (possibly fractional) preset index along the back-and-up path.
pub fn pose_at_zoom(t: f32, look: Vec3) -> Transform {
    let max_i = (ZOOM_PRESETS.len() - 1) as f32;
    let t = t.clamp(0.0, max_i);
    let i0 = t.floor() as usize;
    let i1 = (i0 + 1).min(ZOOM_PRESETS.len() - 1);
    let f = (t - i0 as f32).clamp(0.0, 1.0);

    let p0 = preset_eye(look, ZOOM_PRESETS[i0]);
    let p1 = preset_eye(look, ZOOM_PRESETS[i1]);
    let pos = lerp_eye_zoom(p0, p1, f);
    Transform::from_translation(pos).looking_at(look, Vec3::Y)
}

pub(crate) fn setup_camera(mut commands: Commands) {
    let look = screen_look_at();
    let start = Transform::from_xyz(0.15, 1.85, 2.15).looking_at(look, Vec3::Y);
    let end = pose_at_zoom(0.0, look);

    commands.insert_resource(CameraIntro {
        elapsed: 0.0,
        start,
        end,
    });
    commands.insert_resource(CameraZoom::default());

    let mut cam = commands.spawn((
        Camera3d::default(),
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
    if quality::fxaa_enabled() {
        cam.insert(Fxaa {
            enabled: true,
            edge_threshold: Sensitivity::High,
            edge_threshold_min: Sensitivity::High,
        });
    }
    if quality::bloom_enabled() {
        cam.insert(Bloom {
            intensity: 0.08,
            max_mip_dimension: quality::bloom_max_mip_dimension(),
            ..Bloom::NATURAL
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn skip_intro(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut skip_req: ResMut<IntroSkipRequest>,
    mut post_zoom: ResMut<PostIntroZoom>,
    intro: Option<ResMut<CameraIntro>>,
    mut commands: Commands,
    mut cams: Query<&mut Transform, With<LivingRoomCamera>>,
    mut zoom: ResMut<CameraZoom>,
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
    let preset = post_zoom.0.take().unwrap_or(0);
    *zoom = CameraZoom::default();
    zoom.jump_to(preset);
    if let Ok(mut tf) = cams.single_mut() {
        *tf = pose_at_zoom(f32::from(preset), screen_look_at());
    }
    commands.insert_resource(CameraLocked);
    commands.remove_resource::<CameraIntro>();
}

fn update_intro_camera(
    time: Res<Time>,
    mut post_zoom: ResMut<PostIntroZoom>,
    intro: Option<ResMut<CameraIntro>>,
    mut commands: Commands,
    mut cams: Query<&mut Transform, With<LivingRoomCamera>>,
    phosphor: Query<&GlobalTransform, With<CrtPhosphor>>,
    mut zoom: ResMut<CameraZoom>,
) {
    let Some(mut intro) = intro else {
        return;
    };
    intro.elapsed += time.delta_secs();
    let t = (intro.elapsed / INTRO_SECS).clamp(0.0, 1.0);

    let look_at = phosphor
        .iter()
        .next()
        .map_or_else(screen_look_at, GlobalTransform::translation);

    if let Ok(mut tf) = cams.single_mut() {
        let pos = lerp_eye_pullback_rise(intro.start.translation, intro.end.translation, t);
        *tf = Transform::from_translation(pos).looking_at(look_at, Vec3::Y);
    }

    if t >= 1.0 {
        let preset = post_zoom.0.take().unwrap_or(0);
        *zoom = CameraZoom::default();
        zoom.jump_to(preset);
        if let Ok(mut tf) = cams.single_mut() {
            *tf = pose_at_zoom(f32::from(preset), look_at);
        }
        commands.insert_resource(CameraLocked);
        commands.remove_resource::<CameraIntro>();
    }
}

fn zoom_from_scroll(
    mut ev: MessageReader<MouseWheel>,
    mut zoom: ResMut<CameraZoom>,
    mut acc: ResMut<ZoomScrollAccum>,
    locked: Option<Res<CameraLocked>>,
) {
    if locked.is_none() {
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
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn apply_zoom_camera(
    locked: Option<Res<CameraLocked>>,
    mut zoom: ResMut<CameraZoom>,
    mut look_blend: ResMut<CrtLookBlend>,
    mut cams: Query<(&mut Transform, Option<&mut Bloom>), With<LivingRoomCamera>>,
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
) {
    if locked.is_none() {
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
    if let Ok((mut tf, bloom)) = cams.single_mut() {
        *tf = pose_at_zoom(zoom.display, look);
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
    // Fade glass with zoom instead of popping Visibility.
    let glass_a = (t - 0.12).clamp(0.0, 1.0) * 0.10;
    for handle in &mut glass {
        if let Some(mut mat) = std_mats.get_mut(&handle.0) {
            mat.base_color = Color::srgba(0.55, 0.65, 0.75, glass_a);
        }
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
        assert!(ease_out_cubic(0.25) > ease_in_out_cubic(0.25));
    }

    #[test]
    fn easing_endpoints() {
        assert!((ease_in_out_cubic(0.0) - 0.0).abs() < f32::EPSILON);
        assert!((ease_in_out_cubic(1.0) - 1.0).abs() < f32::EPSILON);
        let from = Vec3::new(0.0, 0.0, 1.0);
        let to = Vec3::new(0.0, 1.0, 3.0);
        let start = lerp_eye_pullback_rise(from, to, 0.0);
        let end = lerp_eye_pullback_rise(from, to, 1.0);
        assert!(start.distance(from) < 0.001);
        assert!(end.distance(to) < 0.001);
    }

    #[test]
    fn ease_in_out_cubic_midpoint() {
        assert!((ease_in_out_cubic(0.5) - 0.5).abs() < f32::EPSILON);
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
        let b = ZOOM_PRESETS[ZOOM_PRESETS.len() - 1];
        assert!(b.crt_fill < a.crt_fill);
        assert!(b.y_lift > a.y_lift);
        let look = Vec3::ZERO;
        let near = preset_eye(look, a);
        let far = preset_eye(look, b);
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
        let d = distance_for_crt_fill(LOCKED_FOV, fill);
        let visible_h = 2.0 * d * (LOCKED_FOV * 0.5).tan();
        let got = PHOSPHOR_H / visible_h;
        assert!((got - fill).abs() < 0.01, "fill={got}");
    }
}
