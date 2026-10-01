//! MetalFX upscale and direct-copy presentation to a `CAMetalDrawable`.
//!
//! The caller supplies Bevy's render queue and calls this after Bevy has submitted
//! its render graph. Both paths encode into that queue, then present the drawable
//! from the same command buffer so Core Animation owns its reuse.

#![cfg(target_os = "macos")]
#![allow(unsafe_code)]

use std::os::raw::c_void;

use bevy::render::renderer::{RenderDevice, RenderQueue};
use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_metal::{
    MTLBlitCommandEncoder, MTLCommandBuffer, MTLCommandEncoder, MTLCommandQueue, MTLDevice,
    MTLOrigin, MTLPixelFormat, MTLSize, MTLStorageMode, MTLTexture, MTLTextureDescriptor,
};
use objc2_metal_fx::{MTLFXSpatialScaler, MTLFXSpatialScalerBase, MTLFXSpatialScalerDescriptor};
use objc2_quartz_core::CAMetalDrawable;
use thiserror::Error;
use wgpu::hal::api::Metal;

/// Which GPU present path was encoded for a frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DrawablePresentMode {
    Direct,
    MetalFx,
}

/// Reuses MetalFX setup and its private output until device or dimensions change.
///
/// The render system must access this exclusively on the dedicated room thread.
#[derive(Default)]
pub struct DrawablePresentCache {
    scaled: Option<CachedScale>,
}

struct CachedScale {
    device: Retained<ProtocolObject<dyn MTLDevice>>,
    source_size: (u32, u32),
    output_size: (u32, u32),
    scaler: Retained<ProtocolObject<dyn MTLFXSpatialScaler>>,
    output_texture: Retained<ProtocolObject<dyn MTLTexture>>,
}

impl std::fmt::Debug for DrawablePresentCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DrawablePresentCache")
            .field("source_size", &self.scaled.as_ref().map(|s| s.source_size))
            .field("output_size", &self.scaled.as_ref().map(|s| s.output_size))
            .finish()
    }
}

impl DrawablePresentCache {
    fn scaled_output(
        &mut self,
        device: &Retained<ProtocolObject<dyn MTLDevice>>,
        source_size: (u32, u32),
        output_size: (u32, u32),
    ) -> Result<&CachedScale, DrawablePresentError> {
        let current = self.scaled.as_ref().is_some_and(|scaled| {
            std::ptr::eq(Retained::as_ptr(&scaled.device), Retained::as_ptr(device))
                && scaled.source_size == source_size
                && scaled.output_size == output_size
        });
        if !current {
            let scaler = make_scaler(device, source_size, output_size)
                .ok_or(DrawablePresentError::MetalFxUnavailable)?;
            let output_desc = MTLTextureDescriptor::new();
            output_desc.setTextureType(objc2_metal::MTLTextureType::Type2D);
            output_desc.setPixelFormat(MTLPixelFormat::BGRA8Unorm_sRGB);
            // SAFETY: `is_valid_upscale` validated both positive dimensions.
            unsafe {
                output_desc.setWidth(output_size.0 as usize);
                output_desc.setHeight(output_size.1 as usize);
            }
            output_desc.setStorageMode(MTLStorageMode::Private);
            // SAFETY: these are the minimum output usage flags reported by the scaler.
            output_desc.setUsage(unsafe { scaler.outputTextureUsage() });
            let output_texture = device
                .newTextureWithDescriptor(&output_desc)
                .ok_or(DrawablePresentError::OutputTextureCreationFailed)?;
            self.scaled = Some(CachedScale {
                device: device.clone(),
                source_size,
                output_size,
                scaler,
                output_texture,
            });
        }
        self.scaled
            .as_ref()
            .ok_or(DrawablePresentError::MetalFxUnavailable)
    }
}

/// A failed drawable present leaves the drawable unpresented for the caller to discard.
#[derive(Debug, Error)]
pub enum DrawablePresentError {
    #[error("null CAMetalDrawable")]
    NullDrawable,
    #[error("source, output, or drawable size is invalid")]
    InvalidSize,
    #[error("source texture does not match the requested render size")]
    SourceSizeMismatch,
    #[error("drawable texture does not match the requested present size")]
    DrawableSizeMismatch,
    #[error("source and drawable must use BGRA8 sRGB")]
    UnsupportedPixelFormat,
    #[error("wgpu device, queue, or texture is not Metal")]
    NotMetal,
    #[error("Metal queue does not belong to Bevy's Metal device")]
    DeviceMismatch,
    #[error("a reduced render size requires MetalFX; reconfigure the Bevy target at present size")]
    NeedsFullResolutionSource,
    #[error("MetalFX is unavailable for this device or these dimensions")]
    MetalFxUnavailable,
    #[error("Bevy's source texture lacks usage required by MetalFX")]
    IncompatibleSourceUsage,
    #[error("failed to allocate the private MetalFX output texture")]
    OutputTextureCreationFailed,
    #[error("failed to allocate a Metal command buffer")]
    CommandBufferCreationFailed,
    #[error("failed to allocate a Metal blit encoder")]
    BlitEncoderCreationFailed,
}

fn is_valid_upscale(source: (u32, u32), output: (u32, u32)) -> bool {
    source.0 > 0
        && source.1 > 0
        && output.0 > 0
        && output.1 > 0
        && source.0 <= output.0
        && source.1 <= output.1
        && source != output
}

fn make_scaler(
    device: &ProtocolObject<dyn MTLDevice>,
    source: (u32, u32),
    output: (u32, u32),
) -> Option<Retained<ProtocolObject<dyn MTLFXSpatialScaler>>> {
    if !is_valid_upscale(source, output) {
        return None;
    }
    // SAFETY: `device` is Bevy's live Metal device; MetalFX accepts the positive
    // dimensions checked above and returns None if the combination is unsupported.
    unsafe {
        if !MTLFXSpatialScalerDescriptor::supportsDevice(device) {
            return None;
        }
        let desc = MTLFXSpatialScalerDescriptor::new();
        desc.setColorTextureFormat(MTLPixelFormat::BGRA8Unorm_sRGB);
        desc.setOutputTextureFormat(MTLPixelFormat::BGRA8Unorm_sRGB);
        desc.setInputWidth(source.0 as usize);
        desc.setInputHeight(source.1 as usize);
        desc.setOutputWidth(output.0 as usize);
        desc.setOutputHeight(output.1 as usize);
        desc.newSpatialScalerWithDevice(device)
    }
}

/// Probe whether the current Bevy Metal device can create a scaler for these sizes.
///
/// Call before shrinking the Bevy render target. The actual source texture must
/// additionally have `TEXTURE_BINDING` usage when presented through MetalFX.
pub fn supports_metalfx(
    render_device: &RenderDevice,
    source_size: (u32, u32),
    output_size: (u32, u32),
) -> bool {
    // SAFETY: this only borrows the HAL device for the duration of this call.
    let Some(hal_device) = (unsafe { render_device.wgpu_device().as_hal::<Metal>() }) else {
        return false;
    };
    let Some(scaler) = make_scaler(hal_device.raw_device(), source_size, output_size) else {
        return false;
    };
    let descriptor = MTLTextureDescriptor::new();
    descriptor.setTextureType(objc2_metal::MTLTextureType::Type2D);
    descriptor.setPixelFormat(MTLPixelFormat::BGRA8Unorm_sRGB);
    // SAFETY: `is_valid_upscale` in make_scaler checked these dimensions.
    unsafe {
        descriptor.setWidth(output_size.0 as usize);
        descriptor.setHeight(output_size.1 as usize);
    }
    descriptor.setStorageMode(MTLStorageMode::Private);
    // SAFETY: MetalFX reports the required output usage for this scaler.
    descriptor.setUsage(unsafe { scaler.outputTextureUsage() });
    hal_device
        .raw_device()
        .newTextureWithDescriptor(&descriptor)
        .is_some()
}

/// Encode MetalFX (when requested) or a direct full-size copy and present.
///
/// `drawable` is a borrowed `CAMetalDrawable` pointer obtained from a live
/// `CAMetalLayer.nextDrawable()` result. The caller must keep that drawable alive
/// until this function returns; the Metal command buffer then retains resources
/// referenced by its commands until GPU completion. The render queue must be the
/// queue associated with `render_device`, and Bevy's render graph must already be
/// submitted before calling this function.
///
/// # Safety
///
/// The caller must pass a valid, live `CAMetalDrawable` pointer. Rust cannot
/// validate an arbitrary foreign pointer before sending Objective-C messages.
pub unsafe fn present_to_drawable(
    render_device: &RenderDevice,
    render_queue: &RenderQueue,
    source: &wgpu::Texture,
    drawable: *mut c_void,
    source_size: (u32, u32),
    output_size: (u32, u32),
    use_metalfx: bool,
    cache: &mut DrawablePresentCache,
) -> Result<DrawablePresentMode, DrawablePresentError> {
    if drawable.is_null() {
        return Err(DrawablePresentError::NullDrawable);
    }
    if source_size.0 == 0 || source_size.1 == 0 || output_size.0 == 0 || output_size.1 == 0 {
        return Err(DrawablePresentError::InvalidSize);
    }
    if (source.width(), source.height()) != source_size {
        return Err(DrawablePresentError::SourceSizeMismatch);
    }
    if source_size != output_size && (!use_metalfx || !is_valid_upscale(source_size, output_size)) {
        return Err(DrawablePresentError::NeedsFullResolutionSource);
    }

    // SAFETY: the caller guarantees this is a live CAMetalDrawable object.
    let drawable = unsafe { &*drawable.cast::<ProtocolObject<dyn CAMetalDrawable>>() };
    let drawable_texture = drawable.texture();
    if (drawable_texture.width(), drawable_texture.height())
        != (output_size.0 as usize, output_size.1 as usize)
    {
        return Err(DrawablePresentError::DrawableSizeMismatch);
    }
    if drawable_texture.pixelFormat() != MTLPixelFormat::BGRA8Unorm_sRGB {
        return Err(DrawablePresentError::UnsupportedPixelFormat);
    }

    // SAFETY: each HAL guard borrows a live wgpu resource. Work is submitted on
    // the same Metal queue; ordinary `commandBuffer()` retains referenced Metal
    // resources until completion (unlike the unretained variant).
    let Some(hal_device) = (unsafe { render_device.wgpu_device().as_hal::<Metal>() }) else {
        return Err(DrawablePresentError::NotMetal);
    };
    let Some(hal_queue) = (unsafe { render_queue.as_hal::<Metal>() }) else {
        return Err(DrawablePresentError::NotMetal);
    };
    let Some(hal_source) = (unsafe { source.as_hal::<Metal>() }) else {
        return Err(DrawablePresentError::NotMetal);
    };
    let metal_device = hal_device.raw_device();
    let metal_queue = hal_queue.as_raw();
    let queue_device = metal_queue.device();
    if !std::ptr::eq(
        Retained::as_ptr(metal_device),
        Retained::as_ptr(&queue_device),
    ) {
        return Err(DrawablePresentError::DeviceMismatch);
    }
    let source_texture = hal_source.raw_handle();
    if source_texture.pixelFormat() != MTLPixelFormat::BGRA8Unorm_sRGB {
        return Err(DrawablePresentError::UnsupportedPixelFormat);
    }

    let copy_source = if source_size == output_size {
        cache.scaled = None;
        None
    } else {
        let scaled = cache.scaled_output(metal_device, source_size, output_size)?;
        // SAFETY: the scaler reports its own required texture usage flags.
        let input_usage = unsafe { scaled.scaler.colorTextureUsage() };
        if !source_texture.usage().contains(input_usage) {
            return Err(DrawablePresentError::IncompatibleSourceUsage);
        }
        Some(scaled)
    };

    let command_buffer = metal_queue
        .commandBuffer()
        .ok_or(DrawablePresentError::CommandBufferCreationFailed)?;
    if let Some(scaled) = copy_source {
        // SAFETY: input and private output textures match the scaler descriptor;
        // source usage was checked, and same-queue order follows Bevy's render submit.
        unsafe {
            scaled.scaler.setColorTexture(Some(source_texture));
            scaled.scaler.setInputContentWidth(source_size.0 as usize);
            scaled.scaler.setInputContentHeight(source_size.1 as usize);
            scaled.scaler.setOutputTexture(Some(&scaled.output_texture));
            scaled.scaler.encodeToCommandBuffer(&command_buffer);
        }
    }

    let blit = command_buffer
        .blitCommandEncoder()
        .ok_or(DrawablePresentError::BlitEncoderCreationFailed)?;
    let blit_source: &ProtocolObject<dyn MTLTexture> =
        copy_source.map_or(source_texture, |scaled| &scaled.output_texture);
    // SAFETY: both textures were checked against `output_size` or created with
    // exactly that size; the copy region is entirely within each texture.
    unsafe {
        blit.copyFromTexture_sourceSlice_sourceLevel_sourceOrigin_sourceSize_toTexture_destinationSlice_destinationLevel_destinationOrigin(
            blit_source,
            0,
            0,
            MTLOrigin { x: 0, y: 0, z: 0 },
            MTLSize {
                width: output_size.0 as usize,
                height: output_size.1 as usize,
                depth: 1,
            },
            &drawable_texture,
            0,
            0,
            MTLOrigin { x: 0, y: 0, z: 0 },
        );
    }
    blit.endEncoding();
    command_buffer.presentDrawable(drawable.as_ref());
    command_buffer.commit();
    Ok(if copy_source.is_some() {
        DrawablePresentMode::MetalFx
    } else {
        DrawablePresentMode::Direct
    })
}
