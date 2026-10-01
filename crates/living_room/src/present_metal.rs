//! macOS: import an IOSurface as a wgpu texture for zero-copy present.

#![cfg(target_os = "macos")]
#![allow(unsafe_code)]

use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
use bevy::render::renderer::RenderDevice;
use objc2_io_surface::IOSurfaceRef;
use objc2_metal::{
    MTLDevice, MTLPixelFormat, MTLTextureDescriptor, MTLTextureType, MTLTextureUsage,
};
use std::os::raw::c_void;
use thiserror::Error;
use wgpu::hal::api::Metal;
use wgpu::hal::CopyExtent;

/// Errors from [`import_iosurface_texture`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PresentIosurfaceError {
    #[error("null IOSurface")]
    NullSurface,
    #[error("wgpu device is not Metal")]
    NotMetal,
    #[error("MTLDevice newTextureWithDescriptor:iosurface:plane: failed")]
    TextureCreateFailed,
    #[error("IOSurface bytesPerRow {actual} is smaller than required {minimum}")]
    InvalidRowStride { actual: usize, minimum: usize },
    #[error("IOSurface bytesPerRow {0} is not aligned to 16 bytes")]
    UnalignedRowStride(usize),
}

fn validate_iosurface_row_stride(
    bytes_per_row: usize,
    width: u32,
) -> Result<(), PresentIosurfaceError> {
    let minimum_bytes_per_row = (width as usize).saturating_mul(4);
    if bytes_per_row < minimum_bytes_per_row {
        return Err(PresentIosurfaceError::InvalidRowStride {
            actual: bytes_per_row,
            minimum: minimum_bytes_per_row,
        });
    }
    if !bytes_per_row.is_multiple_of(16) {
        return Err(PresentIosurfaceError::UnalignedRowStride(bytes_per_row));
    }
    Ok(())
}

/// Import `iosurface` (IOSurfaceRef) into a wgpu texture on Bevy's MTLDevice.
///
/// # Safety
///
/// `iosurface` must be a live `IOSurfaceRef` retained by the caller for as long
/// as the returned texture is used. Width/height must match the surface.
pub fn import_iosurface_texture(
    render_device: &RenderDevice,
    iosurface: *mut c_void,
    width: u32,
    height: u32,
) -> Result<wgpu::Texture, PresentIosurfaceError> {
    if iosurface.is_null() {
        return Err(PresentIosurfaceError::NullSurface);
    }
    let width = width.max(1);
    let height = height.max(1);

    let wgpu_dev = render_device.wgpu_device();
    // SAFETY: Bevy's device is Metal on macOS; HAL borrow is for this call only.
    let Some(hal_dev) = (unsafe { wgpu_dev.as_hal::<Metal>() }) else {
        return Err(PresentIosurfaceError::NotMetal);
    };
    let mtl_device = hal_dev.raw_device().clone();

    // SAFETY: pointer from Swift is a live IOSurfaceRef (CFType).
    let surface = unsafe { &*iosurface.cast::<IOSurfaceRef>() };
    let bytes_per_row = surface.bytes_per_row();
    validate_iosurface_row_stride(bytes_per_row, width)?;

    let desc = MTLTextureDescriptor::new();
    desc.setTextureType(MTLTextureType::Type2D);
    // Match CALayer / IOSurface BGRA and Bevy headless target.
    desc.setPixelFormat(MTLPixelFormat::BGRA8Unorm_sRGB);
    // SAFETY: width/height are validated positive dimensions for a 2D texture.
    unsafe {
        desc.setWidth(width as usize);
        desc.setHeight(height as usize);
    }
    // COPY_DST / render / sample only — BGRA8Unorm_sRGB is not shader-writable on
    // every Metal GPU family, and ShaderWrite can make IOSurface texture create fail.
    desc.setUsage(MTLTextureUsage::ShaderRead | MTLTextureUsage::RenderTarget);

    let Some(mtl_tex) = mtl_device.newTextureWithDescriptor_iosurface_plane(&desc, surface, 0)
    else {
        return Err(PresentIosurfaceError::TextureCreateFailed);
    };

    let copy_size = CopyExtent {
        width,
        height,
        depth: 1,
    };
    // SAFETY: texture created on the same MTLDevice as wgpu; format matches descriptor.
    let hal_tex = unsafe {
        wgpu::hal::metal::Device::texture_from_raw(
            mtl_tex,
            wgpu::TextureFormat::Bgra8UnormSrgb,
            MTLTextureType::Type2D,
            1,
            1,
            copy_size,
        )
    };

    let desc = wgpu::TextureDescriptor {
        label: Some("living_room_iosurface_present"),
        size: Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Bgra8UnormSrgb,
        usage: TextureUsages::COPY_DST
            | TextureUsages::TEXTURE_BINDING
            | TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    };

    // SAFETY: hal texture matches descriptor; ownership moves into wgpu.
    Ok(unsafe { wgpu_dev.create_texture_from_hal::<Metal>(hal_tex, &desc) })
}

#[cfg(test)]
mod tests {
    use super::{validate_iosurface_row_stride, PresentIosurfaceError};

    #[test]
    fn iosurface_stride_accepts_padded_unaligned_width() {
        assert_eq!(validate_iosurface_row_stride(3424, 854), Ok(()));
    }

    #[test]
    fn iosurface_stride_rejects_too_small_row() {
        assert_eq!(
            validate_iosurface_row_stride(3408, 854),
            Err(PresentIosurfaceError::InvalidRowStride {
                actual: 3408,
                minimum: 3416,
            })
        );
    }

    #[test]
    fn iosurface_stride_rejects_unaligned_row() {
        assert_eq!(
            validate_iosurface_row_stride(3416, 854),
            Err(PresentIosurfaceError::UnalignedRowStride(3416))
        );
    }

    #[test]
    fn present_iosurface_error_display_matches_legacy_strings() {
        assert_eq!(
            PresentIosurfaceError::NullSurface.to_string(),
            "null IOSurface"
        );
        assert_eq!(
            PresentIosurfaceError::NotMetal.to_string(),
            "wgpu device is not Metal"
        );
        assert_eq!(
            PresentIosurfaceError::TextureCreateFailed.to_string(),
            "MTLDevice newTextureWithDescriptor:iosurface:plane: failed"
        );
        assert_eq!(
            PresentIosurfaceError::InvalidRowStride {
                actual: 3408,
                minimum: 3416
            }
            .to_string(),
            "IOSurface bytesPerRow 3408 is smaller than required 3416"
        );
        assert_eq!(
            PresentIosurfaceError::UnalignedRowStride(3416).to_string(),
            "IOSurface bytesPerRow 3416 is not aligned to 16 bytes"
        );
    }
}
