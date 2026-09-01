//! Own-window OS capture for agent `GET /v1/host/window` (#239).
//!
//! Safety rules:
//! - Capture only by a registered window id owned by this process.
//! - Never use frontmost / desktop / focused-window APIs.
//! - Never activate, focus, or change z-order as part of capture.

#![allow(unsafe_code)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use control_plane::{ApiError, ApiResult, HostWindowCapture};

/// Holds the last known OS window id for this app (0 = unset).
#[derive(Debug, Default)]
pub struct OwnWindowCapturer {
    window_id: AtomicU32,
}

impl OwnWindowCapturer {
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn set_window_id(&self, id: u32) {
        self.window_id.store(id, Ordering::Relaxed);
    }
}

impl HostWindowCapture for OwnWindowCapturer {
    fn capture_window_png(&self) -> ApiResult<Vec<u8>> {
        let id = self.window_id.load(Ordering::Relaxed);
        if id == 0 {
            return Err(ApiError::Unavailable(
                "host window id not registered yet".into(),
            ));
        }
        capture_own_window_png(id)
    }
}

#[cfg(target_os = "macos")]
fn capture_own_window_png(window_id: u32) -> ApiResult<Vec<u8>> {
    macos::capture_window_png(window_id)
}

#[cfg(not(target_os = "macos"))]
fn capture_own_window_png(_window_id: u32) -> ApiResult<Vec<u8>> {
    Err(ApiError::Unavailable(
        "host window capture not implemented on this OS yet".into(),
    ))
}

/// Update capturer from an eframe [`Frame`] window handle (no focus change).
#[cfg(target_os = "macos")]
pub fn refresh_window_id_from_frame(capturer: &OwnWindowCapturer, frame: &eframe::Frame) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = frame.window_handle() else {
        return;
    };
    if let RawWindowHandle::AppKit(appkit) = handle.as_raw() {
        // SAFETY: ns_view is valid for the frame lifetime; we only read windowNumber.
        let ns_view = appkit.ns_view.as_ptr();
        if let Some(id) = macos::cg_window_id_from_ns_view(ns_view) {
            capturer.set_window_id(id);
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn refresh_window_id_from_frame(_capturer: &OwnWindowCapturer, _frame: &eframe::Frame) {}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::c_void;

    use control_plane::{ApiError, ApiResult};
    use core_foundation::base::TCFType;
    use core_foundation::number::CFNumber;
    use core_graphics::display::{
        kCGWindowImageBoundsIgnoreFraming, kCGWindowListOptionIncludingWindow, CGDisplay,
        CGRectNull, CGWindowID,
    };
    use core_graphics::image::CGImage;
    use core_graphics::window::kCGWindowOwnerPID;
    use objc2::msg_send;
    use objc2::runtime::AnyObject;

    pub fn cg_window_id_from_ns_view(ns_view: *mut c_void) -> Option<u32> {
        if ns_view.is_null() {
            return None;
        }
        // SAFETY: caller guarantees ns_view is a live NSView*.
        unsafe {
            let view: *mut AnyObject = ns_view.cast();
            let window: *mut AnyObject = msg_send![view, window];
            if window.is_null() {
                return None;
            }
            let number: isize = msg_send![window, windowNumber];
            if number <= 0 {
                return None;
            }
            Some(number as u32)
        }
    }

    /// Verify `window_id` is owned by this process, then snapshot **only** that window.
    ///
    /// Does not activate or reorder the window (`CGWindowListCreateImage` is read-only).
    pub fn capture_window_png(window_id: u32) -> ApiResult<Vec<u8>> {
        verify_owned_by_self(window_id)?;
        let image = unsafe {
            CGDisplay::screenshot(
                CGRectNull,
                kCGWindowListOptionIncludingWindow,
                window_id as CGWindowID,
                kCGWindowImageBoundsIgnoreFraming,
            )
        }
        .ok_or_else(|| {
            ApiError::Unavailable(format!(
                "CGWindowListCreateImage failed for window id {window_id}"
            ))
        })?;
        cgimage_to_png(&image)
    }

    fn verify_owned_by_self(window_id: u32) -> ApiResult<()> {
        use core_foundation::base::CFIndex;
        use core_foundation::dictionary::CFDictionaryRef;
        use core_foundation::number::CFNumberRef;

        let infos = CGDisplay::window_list_info(
            kCGWindowListOptionIncludingWindow,
            Some(window_id as CGWindowID),
        )
        .ok_or_else(|| ApiError::Unavailable("CGWindowListCopyWindowInfo failed".into()))?;
        if infos.is_empty() {
            return Err(ApiError::Unavailable(format!(
                "window id {window_id} not found"
            )));
        }
        // SAFETY: index 0 exists; window info entries are CFDictionary.
        let dict_ptr = unsafe { infos.get_unchecked(0 as CFIndex) };
        let dict: CFDictionaryRef = (*dict_ptr) as CFDictionaryRef;
        if dict.is_null() {
            return Err(ApiError::Unavailable("window info dict null".into()));
        }
        // SAFETY: kCGWindowOwnerPID is a permanent CoreGraphics string constant.
        let pid_val = unsafe {
            core_foundation::dictionary::CFDictionaryGetValue(dict, kCGWindowOwnerPID as *const _)
        };
        if pid_val.is_null() {
            return Err(ApiError::Unavailable(
                "window info missing owner PID".into(),
            ));
        }
        let pid_num: CFNumber =
            unsafe { TCFType::wrap_under_get_rule(pid_val as CFNumberRef as _) };
        let Some(pid) = pid_num.to_i64() else {
            return Err(ApiError::Unavailable("owner PID not numeric".into()));
        };
        let self_pid = i64::from(std::process::id());
        if pid != self_pid {
            return Err(ApiError::Unavailable(format!(
                "refusing capture: window {window_id} owned by pid {pid}, not self ({self_pid})"
            )));
        }
        Ok(())
    }

    fn cgimage_to_png(image: &CGImage) -> ApiResult<Vec<u8>> {
        let w = image.width();
        let h = image.height();
        if w == 0 || h == 0 {
            return Err(ApiError::Png("empty CGImage".into()));
        }
        let bytes_per_row = image.bytes_per_row();
        let data = image.data();
        let slice = data.bytes();
        // CGImage is typically BGRA; convert to RGBA for PNG.
        let mut rgba = vec![0u8; w * h * 4];
        for y in 0..h {
            let row = y * bytes_per_row;
            for x in 0..w {
                let i = row + x * 4;
                if i + 3 >= slice.len() {
                    break;
                }
                let o = (y * w + x) * 4;
                rgba[o] = slice[i + 2];
                rgba[o + 1] = slice[i + 1];
                rgba[o + 2] = slice[i];
                rgba[o + 3] = slice[i + 3];
            }
        }
        control_plane::encode_rgba_png(&rgba, w, h)
    }
}
