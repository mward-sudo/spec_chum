//! C ABI for the headless living-room renderer (SwiftUI embed).
//!
//! # Safety
//!
//! Callers must:
//! - Serialize all `sc_room_*` calls for a given handle on **one** dedicated
//!   queue/thread (AppKit main is fine; a background serial queue is preferred
//!   so Bevy does not block Spectrum input). Do not call the same handle
//!   concurrently from multiple threads.
//! - Pass handles from [`sc_room_create`] only to these functions.
//! - Treat [`sc_room_frame_ptr`] as valid until the next mutating call.
//! - Free strings from [`sc_room_last_error`] with [`sc_room_string_free`].

#![allow(unsafe_code)]
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_uint, c_void};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;
use std::sync::Mutex;

use crate::crt::{SCREEN_H, SCREEN_W};
use crate::headless::{HeadlessRoom, DEFAULT_ROOM_H, DEFAULT_ROOM_W};

/// Process-wide last error (not thread-local) so AppKit can read failures after
/// a background-queue `sc_room_*` call.
static LAST_ERROR: Mutex<Option<CString>> = Mutex::new(None);

fn set_last_error(msg: impl Into<String>) {
    let s = CString::new(msg.into().replace('\0', "")).unwrap_or_default();
    if let Ok(mut g) = LAST_ERROR.lock() {
        *g = Some(s);
    }
}

fn clear_last_error() {
    if let Ok(mut g) = LAST_ERROR.lock() {
        *g = None;
    }
}

struct RoomHandle {
    room: HeadlessRoom,
    /// Scratch for [`sc_room_frame_ptr`].
    frame: Vec<u8>,
}

fn room_mut<'a>(handle: *mut c_void) -> Option<&'a mut RoomHandle> {
    if handle.is_null() {
        None
    } else {
        // SAFETY: handle from `sc_room_create` as `Box<RoomHandle>`.
        Some(unsafe { &mut *(handle.cast::<RoomHandle>()) })
    }
}

fn catch_ptr(f: impl FnOnce() -> *mut c_void) -> *mut c_void {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(p) => p,
        Err(_) => {
            set_last_error("living_room panic in FFI");
            ptr::null_mut()
        }
    }
}

fn catch_int(f: impl FnOnce() -> c_int) -> c_int {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(c) => c,
        Err(_) => {
            set_last_error("living_room panic in FFI");
            -1
        }
    }
}

fn catch_uint(f: impl FnOnce() -> c_uint) -> c_uint {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(c) => c,
        Err(_) => {
            set_last_error("living_room panic in FFI");
            0
        }
    }
}

fn catch_const_u8(f: impl FnOnce() -> *const u8) -> *const u8 {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(p) => p,
        Err(_) => {
            set_last_error("living_room panic in FFI");
            ptr::null()
        }
    }
}

/// Create a headless room renderer. `width`/`height` 0 → defaults (1920×1080).
#[no_mangle]
pub extern "C" fn sc_room_create(width: c_uint, height: c_uint) -> *mut c_void {
    catch_ptr(|| {
        crate::ensure_host_api_linked::touch();
        clear_last_error();
        let w = if width == 0 { DEFAULT_ROOM_W } else { width };
        let h = if height == 0 { DEFAULT_ROOM_H } else { height };
        match HeadlessRoom::try_new(w, h) {
            Ok(room) => {
                let frame = vec![0u8; (w as usize).saturating_mul(h as usize).saturating_mul(4)];
                Box::into_raw(Box::new(RoomHandle { room, frame })).cast()
            }
            Err(e) => {
                set_last_error(e);
                ptr::null_mut()
            }
        }
    })
}

#[no_mangle]
pub extern "C" fn sc_room_destroy(handle: *mut c_void) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if handle.is_null() {
            return;
        }
        // SAFETY: unique ownership from create.
        drop(unsafe { Box::from_raw(handle.cast::<RoomHandle>()) });
    }));
}

/// Upload Spectrum RGBA (`sc_framebuffer_*`, typically 352×296).
#[no_mangle]
pub extern "C" fn sc_room_set_framebuffer(
    handle: *mut c_void,
    rgba: *const u8,
    len: c_uint,
) -> c_int {
    catch_int(|| {
        clear_last_error();
        let Some(h) = room_mut(handle) else {
            set_last_error("null handle");
            return -1;
        };
        if rgba.is_null() {
            set_last_error("null rgba");
            return -1;
        }
        let expect = (SCREEN_W * SCREEN_H * 4) as usize;
        let len = len as usize;
        if len < expect {
            set_last_error(format!("framebuffer too short: {len} < {expect}"));
            return -1;
        }
        // SAFETY: caller guarantees `len` bytes at `rgba`.
        let slice = unsafe { std::slice::from_raw_parts(rgba, expect) };
        h.room.set_framebuffer(slice);
        0
    })
}

#[no_mangle]
pub extern "C" fn sc_room_skip_intro(handle: *mut c_void) -> c_int {
    catch_int(|| {
        clear_last_error();
        let Some(h) = room_mut(handle) else {
            set_last_error("null handle");
            return -1;
        };
        h.room.request_skip_intro();
        0
    })
}

/// Nudge zoom preset. `steps > 0` = further back / up; `< 0` = toward full-screen CRT.
#[no_mangle]
pub extern "C" fn sc_room_nudge_zoom(handle: *mut c_void, steps: c_int) -> c_int {
    catch_int(|| {
        clear_last_error();
        let Some(h) = room_mut(handle) else {
            set_last_error("null handle");
            return -1;
        };
        h.room.nudge_zoom(steps);
        0
    })
}

/// Current zoom preset index (0 = CRT fill).
#[no_mangle]
pub extern "C" fn sc_room_zoom_preset(handle: *mut c_void) -> c_uint {
    catch_uint(|| {
        room_mut(handle)
            .map(|h| u32::from(h.room.zoom_preset()))
            .unwrap_or(0)
    })
}

/// Set Bevy frame delta before [`sc_room_tick`] (display-link paced embed).
#[no_mangle]
pub extern "C" fn sc_room_set_frame_delta_seconds(handle: *mut c_void, dt: f32) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if let Some(h) = room_mut(handle) {
            h.room.set_frame_delta(dt);
        }
    }));
}

/// Pump one Bevy frame (call after [`sc_room_set_framebuffer`] when FB changed).
#[no_mangle]
pub extern "C" fn sc_room_tick(handle: *mut c_void) -> c_int {
    catch_int(|| {
        clear_last_error();
        let Some(h) = room_mut(handle) else {
            set_last_error("null handle");
            return -1;
        };
        h.room.tick();
        // CPU readback only when no IOSurface present target (tests / fallback).
        if !h.room.has_present_target() {
            let _ = h.room.copy_frame_rgba(&mut h.frame);
        }
        0
    })
}

#[no_mangle]
pub extern "C" fn sc_room_resize(handle: *mut c_void, width: c_uint, height: c_uint) -> c_int {
    catch_int(|| {
        clear_last_error();
        let Some(h) = room_mut(handle) else {
            set_last_error("null handle");
            return -1;
        };
        match h.room.resize(width, height) {
            Ok(()) => {
                let bytes = (h.room.width() as usize)
                    .saturating_mul(h.room.height() as usize)
                    .saturating_mul(4);
                h.frame.resize(bytes, 0);
                0
            }
            Err(e) => {
                set_last_error(e);
                -1
            }
        }
    })
}

#[no_mangle]
pub extern "C" fn sc_room_set_present_iosurface(
    handle: *mut c_void,
    iosurface: *mut c_void,
    width: c_uint,
    height: c_uint,
) -> c_int {
    catch_int(|| {
        clear_last_error();
        let Some(h) = room_mut(handle) else {
            set_last_error("null handle");
            return -1;
        };
        match h.room.set_present_iosurface(iosurface, width, height) {
            Ok(()) => 0,
            Err(e) => {
                set_last_error(e);
                -1
            }
        }
    })
}

#[no_mangle]
pub extern "C" fn sc_room_width(handle: *mut c_void) -> c_uint {
    catch_uint(|| room_mut(handle).map(|h| h.room.width()).unwrap_or(0))
}

#[no_mangle]
pub extern "C" fn sc_room_height(handle: *mut c_void) -> c_uint {
    catch_uint(|| room_mut(handle).map(|h| h.room.height()).unwrap_or(0))
}

/// Pointer to last room RGBA8 frame (valid until next mutating call).
#[no_mangle]
pub extern "C" fn sc_room_frame_ptr(handle: *mut c_void) -> *const u8 {
    catch_const_u8(|| match room_mut(handle) {
        Some(h) => h.frame.as_ptr(),
        None => ptr::null(),
    })
}

#[no_mangle]
pub extern "C" fn sc_room_frame_byte_len(handle: *mut c_void) -> c_uint {
    catch_uint(|| {
        room_mut(handle)
            .map(|h| h.frame.len() as c_uint)
            .unwrap_or(0)
    })
}

/// C layout must match `ScRoomPerfSnapshot` in `spec_chum_room.h`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ScRoomPerfSnapshot {
    pub ticks: u64,
    pub last_tick_us: u64,
    pub avg_window_us: u64,
    pub max_window_us: u64,
    pub max_tick_us: u64,
    pub width: u32,
    pub height: u32,
    pub zoom_preset: u32,
    pub has_present: u8,
    pub thread_hint: u8,
    pub _pad: [u8; 2],
}

#[no_mangle]
pub extern "C" fn sc_room_perf_set_thread_hint(hint: c_uint) {
    crate::perf::set_tick_thread_hint(u64::from(hint));
}

#[no_mangle]
pub extern "C" fn sc_room_perf_snapshot(
    handle: *mut c_void,
    out: *mut ScRoomPerfSnapshot,
) -> c_int {
    catch_int(|| {
        clear_last_error();
        if out.is_null() {
            set_last_error("null ScRoomPerfSnapshot");
            return -1;
        }
        let Some(h) = room_mut(handle) else {
            set_last_error("null handle");
            return -1;
        };
        let p = h.room.perf();
        // SAFETY: caller-owned out pointer, size matches repr(C) struct.
        unsafe {
            *out = ScRoomPerfSnapshot {
                ticks: p.ticks,
                last_tick_us: p.last_tick_us,
                avg_window_us: p.avg_window_us(),
                max_window_us: p.window_max_us(),
                max_tick_us: p.max_tick_us,
                width: h.room.width(),
                height: h.room.height(),
                zoom_preset: u32::from(h.room.zoom_preset()),
                has_present: u8::from(h.room.has_present_target()),
                thread_hint: crate::perf::tick_thread_hint() as u8,
                _pad: [0; 2],
            };
        }
        0
    })
}

/// Heap-allocated last error; free with [`sc_room_string_free`]. May be null.
#[no_mangle]
pub extern "C" fn sc_room_last_error() -> *mut c_char {
    LAST_ERROR
        .lock()
        .ok()
        .and_then(|mut g| g.take())
        .map_or_else(ptr::null_mut, CString::into_raw)
}

#[no_mangle]
pub extern "C" fn sc_room_string_free(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    // SAFETY: string from `CString::into_raw` via `sc_room_last_error`.
    drop(unsafe { CString::from_raw(s) });
}
