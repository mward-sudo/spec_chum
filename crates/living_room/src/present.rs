//! GPU blit from the headless Bevy Image target into an IOSurface-backed texture.

use std::sync::Arc;

#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicBool, Ordering};

use bevy::{
    prelude::*,
    render::{
        render_asset::RenderAssets,
        render_resource::{CommandEncoderDescriptor, Extent3d, TextureFormat},
        renderer::{RenderDevice, RenderQueue},
        texture::GpuImage,
        Extract, Render, RenderApp, RenderSystems,
    },
};

/// Benchmark-only: behave like the IOSurface present path (skip CPU readback, use a
/// non-blocking poll) without an actual surface.
///
/// Without this, headless benchmarks time a blocking `map_async` plus two multi-megabyte
/// copies per frame and attribute the cost to rendering.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct SimulatePresentPath(pub bool);

/// When set, SpecChumMac presents via IOSurface (no CPU readback).
#[derive(Resource, Clone, Default)]
pub struct PresentTarget {
    pub texture: Option<Arc<wgpu::Texture>>,
    /// Per-tick `CAMetalDrawable` pointer. The host retains it until `sc_room_tick_drawable`
    /// returns; Metal's command buffer retains the drawable after presentation is encoded.
    pub drawable_ptr: Option<usize>,
    pub width: u32,
    pub height: u32,
}

impl std::fmt::Debug for PresentTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PresentTarget")
            .field("has_texture", &self.texture.is_some())
            .field("has_drawable", &self.drawable_ptr.is_some())
            .field("width", &self.width)
            .field("height", &self.height)
            .finish()
    }
}

impl PresentTarget {
    pub fn clear(&mut self) {
        self.texture = None;
        self.drawable_ptr = None;
        self.width = 0;
        self.height = 0;
    }

    pub fn is_set(&self) -> bool {
        self.texture.is_some() || self.drawable_ptr.is_some()
    }
}

#[derive(Resource, Clone, Debug, Default)]
pub(crate) struct PresentBlitError(Arc<std::sync::Mutex<Option<String>>>);

impl PresentBlitError {
    pub(crate) fn take(&self) -> Option<String> {
        self.0.lock().ok()?.take()
    }
}

#[derive(Resource, Clone)]
struct ExtractedPresent {
    texture: Option<Arc<wgpu::Texture>>,
    drawable_ptr: Option<usize>,
    width: u32,
    height: u32,
    src_image: Handle<Image>,
}

#[derive(Debug, Default)]
pub struct PresentBlitPlugin;

impl Plugin for PresentBlitPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PresentTarget>()
            .init_resource::<SimulatePresentPath>()
            .init_resource::<PresentBlitError>();
        let present_error = app.world().resource::<PresentBlitError>().clone();
        let render_app = app.sub_app_mut(RenderApp);
        render_app.insert_resource(present_error);
        #[cfg(target_os = "macos")]
        render_app
            .world_mut()
            .insert_non_send(crate::present_drawable::DrawablePresentCache::default());
        render_app
            .add_systems(ExtractSchedule, extract_present_target)
            .add_systems(
                Render,
                blit_to_present
                    .after(RenderSystems::Render)
                    .run_if(resource_exists::<ExtractedPresent>),
            );
    }
}

fn extract_present_target(
    mut commands: Commands,
    present: Extract<Res<PresentTarget>>,
    target: Extract<Option<Res<crate::headless::HeadlessRenderTargetHandle>>>,
) {
    commands.remove_resource::<ExtractedPresent>();
    let texture = present.texture.clone();
    let drawable_ptr = present.drawable_ptr;
    if texture.is_none() && drawable_ptr.is_none() {
        return;
    }
    let Some(target) = target.as_ref() else {
        return;
    };
    commands.insert_resource(ExtractedPresent {
        texture,
        drawable_ptr,
        width: present.width,
        height: present.height,
        src_image: target.0.clone(),
    });
}

#[cfg_attr(target_os = "macos", allow(unsafe_code))]
fn blit_to_present(
    present: Res<ExtractedPresent>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    present_error: Res<PresentBlitError>,
    #[cfg(target_os = "macos")] mut drawable_cache: bevy::ecs::system::NonSendMut<
        crate::present_drawable::DrawablePresentCache,
    >,
) {
    let Some(src) = gpu_images.get(&present.src_image) else {
        return;
    };
    // CAMetalLayer path: MetalFX and presentation share Bevy's Metal queue and command buffer.
    if let Some(drawable) = present.drawable_ptr {
        if let Ok(mut error) = present_error.0.lock() {
            *error = None;
        }
        #[cfg(target_os = "macos")]
        {
            let source_size = (
                src.texture_descriptor.size.width,
                src.texture_descriptor.size.height,
            );
            let output_size = (present.width, present.height);
            // SAFETY: Swift retains the CAMetalDrawable through `sc_room_tick_drawable`; this
            // system runs synchronously during that call, and Metal retains it when presented.
            if let Err(error) = unsafe {
                crate::present_drawable::present_to_drawable(
                    &render_device,
                    &render_queue,
                    &src.texture,
                    drawable as *mut std::ffi::c_void,
                    source_size,
                    output_size,
                    source_size != output_size,
                    &mut drawable_cache,
                )
            } {
                if let Ok(mut last_error) = present_error.0.lock() {
                    *last_error = Some(error.to_string());
                }
                static LOGGED_DRAWABLE_PRESENT_ERROR: AtomicBool = AtomicBool::new(false);
                if !LOGGED_DRAWABLE_PRESENT_ERROR.swap(true, Ordering::Relaxed) {
                    eprintln!("living_room CAMetalDrawable present failed: {error}");
                }
            }
        }
        return;
    }
    let Some(destination) = present.texture.as_ref() else {
        return;
    };
    // Formats must match for copy_texture_to_texture.
    if src.texture_descriptor.format != TextureFormat::Bgra8UnormSrgb {
        return;
    }
    let w = present.width.min(src.texture_descriptor.size.width);
    let h = present.height.min(src.texture_descriptor.size.height);
    if w == 0 || h == 0 {
        return;
    }
    let mut encoder = render_device.create_command_encoder(&CommandEncoderDescriptor {
        label: Some("living_room_present_blit"),
    });
    encoder.copy_texture_to_texture(
        src.texture.as_image_copy(),
        destination.as_image_copy(),
        Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    render_queue.submit(std::iter::once(encoder.finish()));
}
