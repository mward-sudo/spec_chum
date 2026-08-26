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
use wgpu::hal::api::Metal;
use wgpu::hal::CopyExtent;

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
) -> Result<wgpu::Texture, String> {
    if iosurface.is_null() {
        return Err("null IOSurface".into());
    }
    let width = width.max(1);
    let height = height.max(1);

    let wgpu_dev = render_device.wgpu_device();
    // SAFETY: Bevy's device is Metal on macOS; HAL borrow is for this call only.
    let Some(hal_dev) = (unsafe { wgpu_dev.as_hal::<Metal>() }) else {
        return Err("wgpu device is not Metal".into());
    };
    let mtl_device = hal_dev.raw_device().clone();

    // SAFETY: pointer from Swift is a live IOSurfaceRef (CFType).
    let surface = unsafe { &*iosurface.cast::<IOSurfaceRef>() };

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
        return Err("MTLDevice newTextureWithDescriptor:iosurface:plane: failed".into());
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
