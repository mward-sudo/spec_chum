//! C ABI for Spec Chum host sessions.
//!
//! Domain modules keep each file reviewable while preserving a single C header
//! (`include/spec_chum_host.h`) as the ABI source of truth (#345).
//!
//! # Safety
//!
//! Callers must:
//! - Pass handles returned by [`sc_create`] only to these functions.
//! - Treat framebuffer pointers as valid only until the next mutating call
//!   (especially [`sc_set_border`] / [`sc_destroy`]).
//! - Free strings from [`sc_status`] / [`sc_last_error`] / [`sc_inspect_json`] /
//!   [`sc_debug_dump`] / [`sc_debug_dump_json`] with [`sc_string_free`].

#![allow(unsafe_code)]
// C ABI entry points cannot be `unsafe fn` for C callers; validity is documented.
#![allow(clippy::not_unsafe_ptr_arg_deref)]
#![allow(unused_mut)] // SessionAccess mut binding required for mutating calls; many sc_* are read-only.

mod audio;
mod config;
mod debug;
mod hardware;
mod input;
mod media;
mod rom;
mod session;
mod status;
mod tape;
mod video;

use std::cell::RefCell;
use std::ffi::CString;
use std::os::raw::{c_char, c_uint, c_void};
use std::ptr;
use std::sync::Mutex;

thread_local! {
    static LAST_ERROR: Mutex<Option<CString>> = const { Mutex::new(None) };
    static FB_SNAPSHOT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static FB_META: RefCell<(usize, c_uint, c_uint)> = const { RefCell::new((0, 0, 0)) };
    static AUDIO_SNAPSHOT: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
}

pub(super) fn set_last_error(msg: impl Into<String>) {
    let s = CString::new(msg.into().replace('\0', "")).unwrap_or_default();
    LAST_ERROR.with(|slot| {
        if let Ok(mut g) = slot.lock() {
            *g = Some(s);
        }
    });
}

pub(super) fn clear_last_error() {
    LAST_ERROR.with(|slot| {
        if let Ok(mut g) = slot.lock() {
            *g = None;
        }
    });
}

pub(super) fn session_mut(handle: *mut c_void) -> Option<crate::handle::SessionAccess<'static>> {
    crate::handle::session_access(handle)
}

pub(super) fn heap_cstring(s: &str) -> *mut c_char {
    CString::new(s.replace('\0', "")).map_or(ptr::null_mut(), CString::into_raw)
}

// Re-export entry points so `host_api::ffi::sc_*` paths stay stable for Rust callers/tests.
pub use audio::*;
pub use config::*;
pub use debug::*;
pub use hardware::*;
pub use input::*;
pub use media::*;
pub use rom::*;
pub use session::*;
pub use status::*;
pub use tape::*;
pub use video::*;

#[cfg(test)]
#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;
    use std::path::PathBuf;

    fn workspace_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    #[test]
    fn ffi_create_destroy_and_run() {
        let rom_path = workspace_root().join("roms/spec48.rom");
        if !rom_path.exists() {
            eprintln!("skip: rom missing");
            return;
        }
        let h = sc_create(0, 1);
        assert!(!h.is_null());
        let path = CString::new(rom_path.to_str().expect("utf8")).expect("cstr");
        assert_eq!(sc_load_rom(h, path.as_ptr()), 0);
        sc_run_frame(h);
        assert_eq!(sc_framebuffer_width(h), 352);
        assert_eq!(sc_framebuffer_height(h), 296);
        assert!(!sc_framebuffer_ptr(h).is_null());
        let status = sc_status(h);
        assert!(!status.is_null());
        sc_string_free(status);
        sc_destroy(h);
    }

    #[test]
    fn ffi_bad_model_returns_null() {
        let h = sc_create(99, 1);
        assert!(h.is_null());
        let err = sc_last_error();
        assert!(!err.is_null());
        sc_string_free(err);
    }

    #[test]
    fn ffi_mouse_delta_and_buttons_smoke() {
        let h = sc_create(0, 1);
        assert!(!h.is_null());
        // No ROM yet — must fail cleanly.
        assert_eq!(sc_set_mouse_delta(h, 1, 0), -1);
        assert_eq!(sc_set_mouse_buttons(h, 1, 0, 0), -1);
        sc_destroy(h);
    }

    #[test]
    fn ffi_joystick_mode_rejects_truncated_overflow() {
        let h = sc_create(0, 1);
        assert!(!h.is_null());
        assert_eq!(sc_set_joystick_mode(h, 256), -1);
        let err = sc_last_error();
        assert!(!err.is_null());
        sc_string_free(err);
        assert_eq!(sc_get_model(h), 0);
        sc_destroy(h);
    }

    #[test]
    fn ffi_debug_dump_json_and_peek_null() {
        let dump = sc_debug_dump_json();
        assert!(!dump.is_null());
        sc_string_free(dump);

        let mut out: u8 = 0x5A;
        assert_eq!(sc_peek(ptr::null_mut(), 0, &raw mut out), -1);
        assert_eq!(out, 0x5A);

        let h = sc_create(0, 1);
        assert!(!h.is_null());
        assert_eq!(sc_peek(h, 0, &raw mut out), -1);
        assert_eq!(sc_peek(h, 0x1_0000, &raw mut out), -1);
        assert_eq!(sc_step(h), -1);
        assert_eq!(sc_run_until_break(h, 1), -1);
        sc_destroy(h);
    }
}
