//! Thin egui/eframe adapter for [`control_plane::OwnWindowCapturer`] (#239).

use control_plane::OwnWindowCapturer;

/// Update capturer from an eframe [`Frame`] window handle (no focus change).
#[cfg(target_os = "macos")]
pub fn refresh_window_id_from_frame(capturer: &OwnWindowCapturer, frame: &eframe::Frame) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = frame.window_handle() else {
        return;
    };
    if let RawWindowHandle::AppKit(appkit) = handle.as_raw() {
        let ns_view = appkit.ns_view.as_ptr();
        if let Some(id) = control_plane::cg_window_id_from_ns_view(ns_view) {
            capturer.set_window_id(id);
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn refresh_window_id_from_frame(_capturer: &OwnWindowCapturer, _frame: &eframe::Frame) {}
