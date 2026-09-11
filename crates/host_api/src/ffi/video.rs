//! Framebuffer snapshot C ABI.

use std::os::raw::{c_uint, c_void};
use std::ptr;

use super::{session_mut, FB_META, FB_SNAPSHOT};

/// Pointer to RGBA8 framebuffer snapshot (row-major). Valid until the next call.
#[no_mangle]
pub extern "C" fn sc_framebuffer_ptr(handle: *mut c_void) -> *const u8 {
    let Some(s) = session_mut(handle) else {
        return ptr::null();
    };
    let fb = s.framebuffer();
    let w = s.width() as c_uint;
    let h = s.height() as c_uint;
    let key = handle as usize;
    FB_META.with(|meta| *meta.borrow_mut() = (key, w, h));
    FB_SNAPSHOT.with(|cell| {
        let mut buf = cell.borrow_mut();
        buf.clear();
        buf.extend_from_slice(fb);
        buf.as_ptr()
    })
}

fn snapshotted_fb_dims(handle: *mut c_void) -> Option<(c_uint, c_uint)> {
    if handle.is_null() {
        return None;
    }
    let key = handle as usize;
    FB_SNAPSHOT.with(|cell| {
        if cell.borrow().is_empty() {
            None
        } else {
            FB_META.with(|meta| {
                let (cached_key, w, h) = *meta.borrow();
                if cached_key == key {
                    Some((w, h))
                } else {
                    None
                }
            })
        }
    })
}

#[no_mangle]
pub extern "C" fn sc_framebuffer_width(handle: *mut c_void) -> c_uint {
    if let Some((w, _)) = snapshotted_fb_dims(handle) {
        return w;
    }
    session_mut(handle).map_or(0, |s| s.width() as c_uint)
}

#[no_mangle]
pub extern "C" fn sc_framebuffer_height(handle: *mut c_void) -> c_uint {
    if let Some((_, h)) = snapshotted_fb_dims(handle) {
        return h;
    }
    session_mut(handle).map_or(0, |s| s.height() as c_uint)
}
