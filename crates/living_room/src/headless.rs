//! Headless Bevy living-room renderer for SwiftUI embed (no winit / no Bevy chrome).

use bevy::app::SubApps;
use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::renderer::RenderDevice;
use bevy::render::{
    render_resource::{Extent3d, PollType, TextureDimension, TextureFormat, TextureUsages},
    RenderPlugin,
};
use bevy::time::TimeUpdateStrategy;
use bevy::window::ExitCondition;
use bevy::winit::WinitPlugin;
use std::os::raw::c_void;
use std::sync::Arc;
use std::time::Duration;

use thiserror::Error;

use crate::asset_plugin;

use crate::baked_room::BakedRoomPlugin;
use crate::camera::{
    setup_camera, CameraPlugin, IntroSkipRequest, LivingRoomCamera, PostIntroZoom,
};
use crate::crt::CrtPlugin;
use crate::external_fb::{ExternalFramebuffer, ExternalFramebufferPlugin};
use crate::glow::GlowPlugin;
use crate::hybrid::HybridPlugin;
use crate::image_copy::{spawn_image_copier, ImageCopyPlugin, LatestRoomFrame};
use crate::perf::RoomPerf;
use crate::present::{PresentBlitPlugin, PresentTarget};
use crate::room::RoomPlugin;
use crate::scene_variant::{SceneVariant, SceneVariantPlugin};

/// Default offscreen size for SpecChumMac embed (scaled by CALayer).
/// 1920 long-edge: 60 Hz budget on the real present path; the CRT undersamples below this.
pub const DEFAULT_ROOM_W: u32 = 1920;
pub const DEFAULT_ROOM_H: u32 = 1080;

/// Errors from [`HeadlessRoom`] create / resize / IOSurface present bind.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum HeadlessRoomError {
    #[cfg(target_os = "macos")]
    #[error(transparent)]
    Present(#[from] crate::present_metal::PresentIosurfaceError),
    #[error("IOSurface present is only supported on macOS")]
    UnsupportedPlatform,
    #[error("render scale must be between 0.5 and 1.0")]
    InvalidRenderScale,
    #[error("MetalFX spatial scaling is unavailable for this device or resolution")]
    MetalFxUnsupported,
    #[cfg(target_os = "macos")]
    #[error("drawable presentation failed: {0}")]
    PresentDrawable(String),
    /// Poly Haven tree incomplete / missing — do not start Bevy with a black void (#368).
    #[error("{0}")]
    MissingAssets(String),
}

/// Handle to the offscreen Image used as the camera render target (present blit src).
#[derive(Resource, Clone, Debug)]
pub struct HeadlessRenderTargetHandle(pub Handle<Image>);

/// Manually pumped Bevy room for SpecChumMac.
pub struct HeadlessRoom {
    apps: SubApps,
    /// Camera render target dimensions.
    width: u32,
    height: u32,
    /// Native present dimensions supplied by the host.
    present_width: u32,
    present_height: u32,
    render_scale: f32,
    perf: RoomPerf,
}

impl std::fmt::Debug for HeadlessRoom {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HeadlessRoom")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("present_width", &self.present_width)
            .field("present_height", &self.present_height)
            .field("render_scale", &self.render_scale)
            .finish_non_exhaustive()
    }
}

impl HeadlessRoom {
    /// Build plugins, finish, take [`SubApps`]. Does not call `App::run()`.
    pub fn try_new(width: u32, height: u32) -> Result<Self, HeadlessRoomError> {
        let width = width.max(64);
        let height = height.max(64);
        crate::verify_polyhaven_assets(&crate::resolve_asset_root())
            .map_err(HeadlessRoomError::MissingAssets)?;

        let render_plugin = RenderPlugin {
            // Async compile — sync mode beachballs SpecChumMac for seconds at create/toggle.
            synchronous_pipeline_compilation: false,
            ..default()
        };
        let window_plugin = WindowPlugin {
            primary_window: None,
            exit_condition: ExitCondition::DontExit,
            ..default()
        };

        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .set(window_plugin)
                .set(render_plugin)
                .set(asset_plugin())
                .disable::<WinitPlugin>()
                .disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>(),
        )
        .insert_resource(LatestRoomFrame::new(width, height))
        .insert_resource(HeadlessSize { width, height })
        .insert_resource(IntroSkipRequest::default())
        .insert_resource(PresentTarget::default())
        // Display-link host replaces this each tick via `set_frame_delta`.
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            1.0 / 60.0,
        )))
        .add_plugins((
            ExternalFramebufferPlugin,
            CrtPlugin,
            RoomPlugin,
            BakedRoomPlugin,
            CameraPlugin,
            GlowPlugin,
            HybridPlugin,
            SceneVariantPlugin,
            ImageCopyPlugin,
            PresentBlitPlugin,
        ))
        .add_systems(Startup, setup_headless_target.after(setup_camera))
        .add_systems(
            Startup,
            bind_hybrid_headless_targets
                .after(setup_headless_target)
                .run_if(resource_exists::<crate::hybrid::HybridPlates>),
        );

        app.finish();
        app.cleanup();

        let mut apps = std::mem::take(app.sub_apps_mut());
        apps.update();
        // Bounded wait — never hang the UI thread forever on adapter/GPU stalls.
        let _ = apps
            .main
            .world()
            .resource::<RenderDevice>()
            .wgpu_device()
            .poll(PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_millis(500)),
            });

        Ok(Self {
            apps,
            width,
            height,
            present_width: width,
            present_height: height,
            render_scale: 1.0,
            perf: RoomPerf::default(),
        })
    }

    /// Convenience for tests / examples (panics on failure).
    pub fn new(width: u32, height: u32) -> Self {
        Self::try_new(width, height).expect("HeadlessRoom::new")
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn present_width(&self) -> u32 {
        self.present_width
    }

    pub fn present_height(&self) -> u32 {
        self.present_height
    }

    /// Lower the Bevy camera target resolution while keeping the host output size.
    pub fn set_render_scale(&mut self, scale: f32) -> Result<(), HeadlessRoomError> {
        if !scale.is_finite() || !(0.5..=1.0).contains(&scale) {
            return Err(HeadlessRoomError::InvalidRenderScale);
        }
        if scale == self.render_scale {
            return Ok(());
        }
        let (width, height) = scaled_render_size(self.present_width, self.present_height, scale);
        if scale < 1.0 {
            #[cfg(target_os = "macos")]
            {
                let render_device = self.apps.main.world().resource::<RenderDevice>();
                if !crate::present_drawable::supports_metalfx(
                    render_device,
                    (width, height),
                    (self.present_width, self.present_height),
                ) {
                    return Err(HeadlessRoomError::MetalFxUnsupported);
                }
            }
            #[cfg(not(target_os = "macos"))]
            return Err(HeadlessRoomError::UnsupportedPlatform);
        }
        rebuild_headless_render_target(&mut self.apps, width, height);
        self.width = width;
        self.height = height;
        self.render_scale = scale;
        self.apps.update();
        let _ = self
            .apps
            .main
            .world()
            .resource::<RenderDevice>()
            .wgpu_device()
            .poll(PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_millis(500)),
            });
        Ok(())
    }

    pub fn has_present_target(&self) -> bool {
        let real = self
            .apps
            .main
            .world()
            .get_resource::<PresentTarget>()
            .is_some_and(PresentTarget::is_set);
        real || self.simulating_present_path()
    }

    fn simulating_present_path(&self) -> bool {
        self.apps
            .main
            .world()
            .get_resource::<crate::present::SimulatePresentPath>()
            .is_some_and(|s| s.0)
    }

    /// Benchmarks only: take the shipping present path (no CPU readback, non-blocking
    /// poll) without an IOSurface. [`Self::copy_frame_rgba`] goes stale while enabled.
    pub fn set_simulate_present_path(&mut self, on: bool) {
        self.apps
            .main
            .world_mut()
            .insert_resource(crate::present::SimulatePresentPath(on));
    }

    /// Upload Spectrum RGBA onto the phosphor (`SCREEN_W`×`SCREEN_H`).
    ///
    /// Accepts classic 256×192 / 352×296 and Timex hi-res 512×192 / 640×296;
    /// other lengths are nearest-neighbour scaled when dimensions can be inferred,
    /// otherwise the leading `SCREEN_W`×`SCREEN_H` bytes are copied (legacy).
    pub fn set_framebuffer(&mut self, rgba: &[u8]) {
        let mut fb = self
            .apps
            .main
            .world_mut()
            .resource_mut::<ExternalFramebuffer>();
        let expect = fb.rgba.len();
        if let Some((w, h)) = crate::fb_scale::dims_from_rgba_len(rgba.len()) {
            crate::fb_scale::blit_to_crt(&mut fb.rgba, rgba, w, h);
        } else if rgba.len() >= expect {
            fb.rgba.copy_from_slice(&rgba[..expect]);
        } else {
            let n = expect.min(rgba.len());
            fb.rgba[..n].copy_from_slice(&rgba[..n]);
            fb.rgba[n..].fill(0);
        }
    }

    pub fn request_skip_intro(&mut self) {
        // Living-room hero framing (preset 3 after near-fill stop) — not CRT-fill.
        self.apps.main.world_mut().resource_mut::<PostIntroZoom>().0 = Some(3);
        self.apps
            .main
            .world_mut()
            .resource_mut::<IntroSkipRequest>()
            .0 = true;
    }

    /// Scroll/trackpad zoom: `steps > 0` pulls back, `< 0` toward full-screen CRT.
    /// Sets the next preset target; [`apply_zoom_camera`] eases each tick until reached.
    pub fn nudge_zoom(&mut self, steps: i32) {
        let mut zoom = self
            .apps
            .main
            .world_mut()
            .resource_mut::<crate::camera::CameraZoom>();
        // Keep Bevy step cooldown (Swift also coalesces); do not clear `last_step`.
        zoom.nudge(steps);
    }

    pub fn zoom_preset(&self) -> u8 {
        self.apps
            .main
            .world()
            .resource::<crate::camera::CameraZoom>()
            .target
    }

    /// Temporary #149 A/B: switch Current (baseline) ↔ New (lightmap WIP).
    pub fn set_scene_variant(&mut self, variant: SceneVariant) {
        *self.apps.main.world_mut().resource_mut::<SceneVariant>() = variant;
    }

    /// Temporary #149 A/B harness — see [`SceneVariant`].
    pub fn scene_variant(&self) -> SceneVariant {
        *self.apps.main.world().resource::<SceneVariant>()
    }

    /// Drive Bevy `Time` from display-link delta (not Spectrum 50 Hz).
    pub fn set_frame_delta(&mut self, dt_secs: f32) {
        let dt = dt_secs.clamp(1.0 / 240.0, 0.1);
        self.apps
            .main
            .world_mut()
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
                dt,
            )));
    }

    /// Pump one Bevy frame. Present path uses a short poll — a 16 ms Wait was
    /// capping roomHz near 30–60 on ProMotion (120 Hz needs ≤8 ms budget).
    pub fn tick(&mut self) {
        let t0 = std::time::Instant::now();
        self.apps.update();
        if self.has_present_target() {
            // Non-blocking: DisplayLink refreshes the CALayer every frame; waiting
            // for GPU idle here only serialized ticks below refresh rate.
            let _ = self
                .apps
                .main
                .world()
                .resource::<RenderDevice>()
                .wgpu_device()
                .poll(PollType::Poll);
        } else {
            let _ = self
                .apps
                .main
                .world()
                .resource::<RenderDevice>()
                .wgpu_device()
                .poll(PollType::Wait {
                    submission_index: None,
                    timeout: Some(Duration::from_millis(50)),
                });
        }
        let us = t0.elapsed().as_micros().min(u128::from(u64::MAX)) as u64;
        self.perf.record_tick(us);
    }

    /// Pump a frame into a host-owned CAMetalDrawable and present it on Bevy's Metal queue.
    ///
    /// # Safety
    ///
    /// `drawable` must be a live CAMetalDrawable retained by the caller until this method
    /// returns. The present system encodes and commits presentation before the synchronous
    /// render update returns.
    #[cfg(target_os = "macos")]
    #[allow(unsafe_code)]
    pub unsafe fn tick_with_drawable(
        &mut self,
        drawable: *mut c_void,
        width: u32,
        height: u32,
    ) -> Result<(), HeadlessRoomError> {
        if drawable.is_null() {
            self.tick();
            return Err(HeadlessRoomError::MetalFxUnsupported);
        }
        if (width, height) != (self.present_width, self.present_height) {
            self.resize(width, height)?;
        }
        if let Some(mut present) = self
            .apps
            .main
            .world_mut()
            .get_resource_mut::<PresentTarget>()
        {
            present.drawable_ptr = Some(drawable as usize);
            present.width = width.max(1);
            present.height = height.max(1);
        }
        self.tick();
        let error = self
            .apps
            .main
            .world()
            .resource::<crate::present::PresentBlitError>()
            .take()
            .map(HeadlessRoomError::PresentDrawable);
        if let Some(mut present) = self
            .apps
            .main
            .world_mut()
            .get_resource_mut::<PresentTarget>()
        {
            present.drawable_ptr = None;
        }
        error.map_or(Ok(()), Err)
    }

    pub fn perf(&self) -> &RoomPerf {
        &self.perf
    }

    /// Copy latest room frame into `out` (CPU readback path). Returns bytes written.
    pub fn copy_frame_rgba(&self, out: &mut [u8]) -> usize {
        let latest = self.apps.main.world().resource::<LatestRoomFrame>();
        let Ok(g) = latest.rgba.lock() else {
            return 0;
        };
        let n = out.len().min(g.len());
        out[..n].copy_from_slice(&g[..n]);
        n
    }

    /// Resize the offscreen render target in place (caller should skip intro + rebind IOSurface).
    ///
    /// Avoids a full `HeadlessRoom::try_new` / GPU pipeline recompile on stepped window resizes
    /// (was freezing SpecChumMac for seconds when the living-room queue blocked).
    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), HeadlessRoomError> {
        let present_width = width.max(64);
        let present_height = height.max(64);
        if present_width == self.present_width && present_height == self.present_height {
            return Ok(());
        }
        let (width, height) = scaled_render_size(present_width, present_height, self.render_scale);
        rebuild_headless_render_target(&mut self.apps, width, height);
        self.width = width;
        self.height = height;
        self.present_width = present_width;
        self.present_height = present_height;
        self.apps.update();
        let _ = self
            .apps
            .main
            .world()
            .resource::<RenderDevice>()
            .wgpu_device()
            .poll(PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_millis(500)),
            });
        Ok(())
    }

    /// Bind or clear an IOSurface present target (macOS). `iosurface` may be null to clear.
    pub fn set_present_iosurface(
        &mut self,
        iosurface: *mut c_void,
        width: u32,
        height: u32,
    ) -> Result<(), HeadlessRoomError> {
        if iosurface.is_null() {
            if let Some(mut present) = self
                .apps
                .main
                .world_mut()
                .get_resource_mut::<PresentTarget>()
            {
                present.clear();
            }
            return Ok(());
        }
        #[cfg(target_os = "macos")]
        {
            let tex = {
                let render_device = self.apps.main.world().resource::<RenderDevice>();
                crate::present_metal::import_iosurface_texture(
                    render_device,
                    iosurface,
                    width,
                    height,
                )?
            };
            let mut present = self.apps.main.world_mut().resource_mut::<PresentTarget>();
            present.texture = Some(Arc::new(tex));
            present.width = width.max(1);
            present.height = height.max(1);
            Ok(())
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (width, height);
            Err(HeadlessRoomError::UnsupportedPlatform)
        }
    }
}

fn scaled_render_size(present_width: u32, present_height: u32, scale: f32) -> (u32, u32) {
    fn scaled_dimension(pixels: u32, scale: f32) -> u32 {
        let maximum = pixels.max(64);
        if scale == 1.0 {
            return maximum;
        }
        let even = ((pixels as f32 * scale / 2.0).round() as u32).saturating_mul(2);
        even.max(64).min(maximum)
    }

    (
        scaled_dimension(present_width, scale),
        scaled_dimension(present_height, scale),
    )
}

#[derive(Resource, Clone, Copy, Debug)]
struct HeadlessSize {
    width: u32,
    height: u32,
}

fn setup_headless_target(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    render_device: Res<RenderDevice>,
    size: Res<HeadlessSize>,
    cams: Query<Entity, With<LivingRoomCamera>>,
) {
    let w = size.width;
    let h = size.height;
    let handle = create_headless_render_image(&mut images, w, h);
    commands.insert_resource(HeadlessRenderTargetHandle(handle.clone()));
    spawn_image_copier(&mut commands, &render_device, handle.clone(), w, h);

    let target = RenderTarget::Image(handle.into());
    for entity in &cams {
        commands.entity(entity).insert(target.clone());
    }
}

fn create_headless_render_image(images: &mut Assets<Image>, w: u32, h: u32) -> Handle<Image> {
    let extent = Extent3d {
        width: w,
        height: h,
        depth_or_array_layers: 1,
    };
    // BGRA matches macOS IOSurface / CALayer present path.
    let mut image = Image::new_uninit(
        extent,
        TextureDimension::D2,
        TextureFormat::Bgra8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.usage |=
        TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC | TextureUsages::TEXTURE_BINDING;
    images.add(image)
}

/// Rebuild the offscreen camera target without tearing down Bevy / recompiling pipelines.
fn rebuild_headless_render_target(apps: &mut SubApps, width: u32, height: u32) {
    let world = apps.main.world_mut();
    *world.resource_mut::<HeadlessSize>() = HeadlessSize { width, height };

    if let Some(mut present) = world.get_resource_mut::<PresentTarget>() {
        present.clear();
    }

    {
        let bytes = (width as usize)
            .saturating_mul(height as usize)
            .saturating_mul(4);
        let rgba = world.resource::<LatestRoomFrame>().rgba.clone();
        if let Ok(mut buf) = rgba.lock() {
            buf.clear();
            buf.resize(bytes, 0);
        }
        let mut latest = world.resource_mut::<LatestRoomFrame>();
        latest.width = width;
        latest.height = height;
    }

    if let Some(mut plates) = world.get_resource_mut::<crate::hybrid::HybridPlates>() {
        crate::hybrid::bind_present_target(width, height, Some(&mut plates));
    }

    crate::image_copy::despawn_image_copiers(world);

    let handle = {
        let mut images = world.resource_mut::<Assets<Image>>();
        create_headless_render_image(&mut images, width, height)
    };
    world.resource_mut::<HeadlessRenderTargetHandle>().0 = handle.clone();

    {
        let render_device = world.resource::<RenderDevice>().clone();
        spawn_image_copier(
            &mut world.commands(),
            &render_device,
            handle.clone(),
            width,
            height,
        );
        world.flush();
    }

    let target = RenderTarget::Image(handle.into());
    let cam_entities: Vec<Entity> = {
        let mut query = world.query_filtered::<Entity, With<LivingRoomCamera>>();
        query.iter(world).collect()
    };
    for entity in cam_entities {
        world.entity_mut(entity).insert(target.clone());
    }
}

/// Keep hybrid bake plate resolution aligned with the headless present size.
fn bind_hybrid_headless_targets(
    size: Res<HeadlessSize>,
    mut plates: ResMut<crate::hybrid::HybridPlates>,
) {
    crate::hybrid::bind_present_target(size.width, size.height, Some(&mut plates));
}

#[cfg(test)]
mod tests {
    use super::{scaled_render_size, HeadlessRoomError};

    #[test]
    fn unsupported_platform_display_matches_legacy_string() {
        assert_eq!(
            HeadlessRoomError::UnsupportedPlatform.to_string(),
            "IOSurface present is only supported on macOS"
        );
    }

    #[test]
    fn scaled_render_size_preserves_aspect_and_even_dimensions() {
        assert_eq!(scaled_render_size(1920, 1080, 2.0 / 3.0), (1280, 720));
        assert_eq!(scaled_render_size(853, 480, 2.0 / 3.0), (568, 320));
    }

    #[test]
    fn scaled_render_size_keeps_minimum_room_target() {
        assert_eq!(scaled_render_size(64, 64, 0.5), (64, 64));
    }
}
