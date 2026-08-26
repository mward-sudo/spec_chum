//! GPU→CPU readback of the headless render target (adapted from Bevy's headless_renderer example).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use bevy::{
    image::TextureFormatPixelInfo,
    prelude::*,
    render::{
        render_asset::RenderAssets,
        render_resource::{
            Buffer, BufferDescriptor, BufferUsages, CommandEncoderDescriptor, Extent3d, MapMode,
            PollType, TexelCopyBufferInfo, TexelCopyBufferLayout, TextureFormat,
        },
        renderer::{RenderContext, RenderDevice, RenderGraph, RenderQueue},
        texture::GpuImage,
        Extract, Render, RenderApp, RenderSystems,
    },
};
use crossbeam_channel::{Receiver, Sender};

/// Latest RGBA8 frame for FFI (may be one frame behind).
#[derive(Resource, Clone, Debug)]
pub struct LatestRoomFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<std::sync::Mutex<Vec<u8>>>,
}

impl LatestRoomFrame {
    pub fn new(width: u32, height: u32) -> Self {
        let n = (width * height * 4) as usize;
        Self {
            width,
            height,
            rgba: Arc::new(std::sync::Mutex::new(vec![0u8; n])),
        }
    }
}

#[derive(Resource, Deref)]
struct MainWorldReceiver(Receiver<Vec<u8>>);

#[derive(Resource, Deref)]
struct RenderWorldSender(Sender<Vec<u8>>);

#[derive(Clone, Component)]
struct ImageCopier {
    buffer: Buffer,
    enabled: Arc<AtomicBool>,
    /// Outstanding `map_async` from a prior frame that timed out; polled again later.
    pending_map: Arc<Mutex<Option<Receiver<()>>>>,
    src_image: Handle<Image>,
    width: u32,
    height: u32,
}

#[derive(Clone, Default, Resource, Deref, DerefMut)]
struct ImageCopiers(pub Vec<ImageCopier>);

#[derive(Resource, Clone, Copy, Default)]
struct SkipCpuReadback(bool);

#[derive(Debug, Default)]
pub struct ImageCopyPlugin;

impl Plugin for ImageCopyPlugin {
    fn build(&self, app: &mut App) {
        let (s, r) = crossbeam_channel::unbounded();
        app.insert_resource(MainWorldReceiver(r))
            .add_systems(Update, drain_copied_frames);

        // Match Bevy's `headless_renderer` example: copy runs in the camera
        // render graph; CPU map runs after the full Render schedule.
        let render_app = app.sub_app_mut(RenderApp);
        render_app
            .init_resource::<ImageCopiers>()
            .init_resource::<SkipCpuReadback>()
            .insert_resource(RenderWorldSender(s))
            .add_systems(ExtractSchedule, image_copy_extract)
            .add_systems(RenderGraph, image_copy_driver)
            .add_systems(
                Render,
                receive_image_from_buffer.after(RenderSystems::Render),
            );
    }
}

/// Drop stale copiers before rebuilding the headless render target (in-place resize).
pub(crate) fn despawn_image_copiers(world: &mut World) {
    let entities: Vec<Entity> = {
        let mut query = world.query_filtered::<Entity, With<ImageCopier>>();
        query.iter(world).collect()
    };
    for entity in entities {
        world.despawn(entity);
    }
}

/// Spawn a GPU→CPU copier for `src` (must have `COPY_SRC` usage).
pub fn spawn_image_copier(
    commands: &mut Commands,
    render_device: &RenderDevice,
    src: Handle<Image>,
    width: u32,
    height: u32,
) {
    let size = Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };
    let padded = RenderDevice::align_copy_bytes_per_row((width as usize) * 4);
    let buffer = render_device.create_buffer(&BufferDescriptor {
        label: Some("living_room_image_copy"),
        size: (padded as u64) * u64::from(height),
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    commands.spawn(ImageCopier {
        buffer,
        enabled: Arc::new(AtomicBool::new(true)),
        pending_map: Arc::new(Mutex::new(None)),
        src_image: src,
        width,
        height,
    });
    let _ = size;
}

fn image_copy_extract(
    mut commands: Commands,
    image_copiers: Extract<Query<&ImageCopier>>,
    present: Extract<Option<Res<crate::present::PresentTarget>>>,
    simulate: Extract<Option<Res<crate::present::SimulatePresentPath>>>,
) {
    commands.insert_resource(ImageCopiers(image_copiers.iter().cloned().collect()));
    let skip =
        present.as_ref().is_some_and(|p| p.is_set()) || simulate.as_ref().is_some_and(|s| s.0);
    commands.insert_resource(SkipCpuReadback(skip));
}

fn image_copy_driver(
    render_context: RenderContext,
    image_copiers: Option<Res<ImageCopiers>>,
    render_queue: Res<RenderQueue>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    skip: Option<Res<SkipCpuReadback>>,
) {
    // IOSurface present path: skip CPU readback (was blocking the main thread every frame).
    if skip.is_some_and(|s| s.0) {
        return;
    }
    let Some(image_copiers) = image_copiers else {
        return;
    };
    for image_copier in image_copiers.iter() {
        if !image_copier.enabled.load(Ordering::Relaxed) {
            continue;
        }
        let Some(src_image) = gpu_images.get(&image_copier.src_image) else {
            continue;
        };
        let mut encoder = render_context
            .render_device()
            .create_command_encoder(&CommandEncoderDescriptor::default());
        let padded_bytes_per_row =
            RenderDevice::align_copy_bytes_per_row((image_copier.width as usize) * 4);
        encoder.copy_texture_to_buffer(
            src_image.texture.as_image_copy(),
            TexelCopyBufferInfo {
                buffer: &image_copier.buffer,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row as u32),
                    rows_per_image: Some(image_copier.height),
                },
            },
            src_image.texture_descriptor.size,
        );
        render_queue.submit(std::iter::once(encoder.finish()));
    }
}

fn receive_image_from_buffer(
    image_copiers: Option<Res<ImageCopiers>>,
    render_device: Res<RenderDevice>,
    sender: Option<Res<RenderWorldSender>>,
    skip: Option<Res<SkipCpuReadback>>,
) {
    if skip.is_some_and(|s| s.0) {
        return;
    }
    let Some(image_copiers) = image_copiers else {
        return;
    };
    let Some(sender) = sender else {
        return;
    };
    for image_copier in image_copiers.0.iter() {
        // Finish a previously timed-out map before starting a new one.
        let pending = image_copier
            .pending_map
            .lock()
            .ok()
            .and_then(|mut slot| slot.take());
        if let Some(r) = pending {
            let mut mapped = false;
            for _ in 0..10 {
                let _ = render_device.poll(PollType::Wait {
                    submission_index: None,
                    timeout: Some(std::time::Duration::from_millis(50)),
                });
                if r.try_recv().is_ok() {
                    mapped = true;
                    break;
                }
            }
            if mapped {
                let buffer_slice = image_copier.buffer.slice(..);
                let data = buffer_slice.get_mapped_range().to_vec();
                let _ = sender.send(data);
                image_copier.buffer.unmap();
                image_copier.enabled.store(true, Ordering::Relaxed);
            } else if let Ok(mut slot) = image_copier.pending_map.lock() {
                // Still outstanding — retry next frame; leave enabled=false.
                *slot = Some(r);
            }
            continue;
        }

        if !image_copier.enabled.load(Ordering::Relaxed) {
            continue;
        }
        let buffer_slice = image_copier.buffer.slice(..);
        let (s, r) = crossbeam_channel::bounded(1);
        // Disable until unmap — overlapping map+submit panics wgpu ("still mapped").
        image_copier.enabled.store(false, Ordering::Relaxed);
        buffer_slice.map_async(MapMode::Read, move |result| {
            if result.is_ok() {
                let _ = s.send(());
            }
        });
        // Bounded wait — never hang the embed host on GPU stalls.
        let mut mapped = false;
        for _ in 0..10 {
            let _ = render_device.poll(PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_millis(50)),
            });
            if r.try_recv().is_ok() {
                mapped = true;
                break;
            }
        }
        if !mapped {
            // Keep the receiver and retry on later frames (do not abandon permanently).
            if let Ok(mut slot) = image_copier.pending_map.lock() {
                *slot = Some(r);
            }
            bevy::log::warn!("living_room image copy map timed out; retrying next frame");
            continue;
        }
        let data = buffer_slice.get_mapped_range().to_vec();
        let _ = sender.send(data);
        image_copier.buffer.unmap();
        image_copier.enabled.store(true, Ordering::Relaxed);
    }
}

fn drain_copied_frames(receiver: Res<MainWorldReceiver>, latest: Res<LatestRoomFrame>) {
    let mut image_data = Vec::new();
    while let Ok(data) = receiver.try_recv() {
        image_data = data;
    }
    if image_data.is_empty() {
        return;
    }
    let row_bytes =
        (latest.width as usize) * TextureFormat::Bgra8UnormSrgb.pixel_size().unwrap_or(4);
    let aligned = RenderDevice::align_copy_bytes_per_row(row_bytes);
    let mut tight = Vec::with_capacity(row_bytes * latest.height as usize);
    if row_bytes == aligned {
        tight = image_data;
    } else {
        for row in image_data.chunks(aligned).take(latest.height as usize) {
            let n = row_bytes.min(row.len());
            tight.extend_from_slice(&row[..n]);
        }
    }
    if let Ok(mut g) = latest.rgba.lock() {
        *g = tight;
    }
}
