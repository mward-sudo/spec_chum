//! GPU blit from the headless Bevy Image target into an IOSurface-backed texture.

use std::sync::Arc;

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
    pub width: u32,
    pub height: u32,
}

impl std::fmt::Debug for PresentTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PresentTarget")
            .field("has_texture", &self.texture.is_some())
            .field("width", &self.width)
            .field("height", &self.height)
            .finish()
    }
}

impl PresentTarget {
    pub fn clear(&mut self) {
        self.texture = None;
        self.width = 0;
        self.height = 0;
    }

    pub fn is_set(&self) -> bool {
        self.texture.is_some()
    }
}

#[derive(Resource, Clone)]
struct ExtractedPresent {
    texture: Arc<wgpu::Texture>,
    width: u32,
    height: u32,
    src_image: Handle<Image>,
}

#[derive(Debug, Default)]
pub struct PresentBlitPlugin;

impl Plugin for PresentBlitPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PresentTarget>()
            .init_resource::<SimulatePresentPath>();
        let render_app = app.sub_app_mut(RenderApp);
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
    let Some(tex) = present.texture.clone() else {
        return;
    };
    let Some(target) = target.as_ref() else {
        return;
    };
    commands.insert_resource(ExtractedPresent {
        texture: tex,
        width: present.width,
        height: present.height,
        src_image: target.0.clone(),
    });
}

fn blit_to_present(
    present: Res<ExtractedPresent>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
) {
    let Some(src) = gpu_images.get(&present.src_image) else {
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
        present.texture.as_image_copy(),
        Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    render_queue.submit(std::iter::once(encoder.finish()));
}
