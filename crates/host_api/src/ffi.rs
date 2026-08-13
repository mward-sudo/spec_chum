//! C ABI for Spec Chum host sessions.
//!
//! # Safety
//!
//! Callers must:
//! - Pass handles returned by [`sc_create`] only to these functions.
//! - Treat framebuffer pointers as valid only until the next mutating call
//!   (especially [`sc_set_border`] / [`sc_destroy`]).
//! - Free strings from [`sc_status`] / [`sc_last_error`] with [`sc_string_free`].

#![allow(unsafe_code)]
// C ABI entry points cannot be `unsafe fn` for C callers; validity is documented.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_uint, c_void};
use std::path::Path;
use std::ptr;
use std::sync::Mutex;

use crate::session::{HostSession, ModelId};

thread_local! {
    static LAST_ERROR: Mutex<Option<CString>> = const { Mutex::new(None) };
}

fn set_last_error(msg: impl Into<String>) {
    let s = CString::new(msg.into().replace('\0', "")).unwrap_or_default();
    LAST_ERROR.with(|slot| {
        if let Ok(mut g) = slot.lock() {
            *g = Some(s);
        }
    });
}

fn clear_last_error() {
    LAST_ERROR.with(|slot| {
        if let Ok(mut g) = slot.lock() {
            *g = None;
        }
    });
}

fn session_mut<'a>(handle: *mut c_void) -> Option<&'a mut HostSession> {
    if handle.is_null() {
        None
    } else {
        // SAFETY: handle was created by `sc_create` as `Box<HostSession>`.
        Some(unsafe { &mut *(handle.cast::<HostSession>()) })
    }
}

/// Create a host session. Returns an opaque handle, or null on failure.
#[no_mangle]
pub extern "C" fn sc_create(model: c_uint, with_border: c_int) -> *mut c_void {
    clear_last_error();
    let Some(model) = ModelId::from_u32(model) else {
        set_last_error("invalid model id");
        return ptr::null_mut();
    };
    let session = HostSession::new(model, with_border != 0);
    Box::into_raw(Box::new(session)).cast()
}

/// Destroy a session created by [`sc_create`].
#[no_mangle]
pub extern "C" fn sc_destroy(handle: *mut c_void) {
    if handle.is_null() {
        return;
    }
    // SAFETY: handle from `sc_create`; unique ownership.
    drop(unsafe { Box::from_raw(handle.cast::<HostSession>()) });
}

#[no_mangle]
pub extern "C" fn sc_set_model(handle: *mut c_void, model: c_uint) -> c_int {
    clear_last_error();
    let Some(s) = session_mut(handle) else {
        set_last_error("null handle");
        return -1;
    };
    let Some(model) = ModelId::from_u32(model) else {
        set_last_error("invalid model id");
        return -1;
    };
    s.set_model(model);
    0
}

#[no_mangle]
pub extern "C" fn sc_load_rom(handle: *mut c_void, path: *const c_char) -> c_int {
    clear_last_error();
    let Some(s) = session_mut(handle) else {
        set_last_error("null handle");
        return -1;
    };
    if path.is_null() {
        set_last_error("null path");
        return -1;
    }
    // SAFETY: caller provides a valid NUL-terminated C string.
    let cstr = unsafe { CStr::from_ptr(path) };
    let Ok(path) = cstr.to_str() else {
        set_last_error("path not utf-8");
        return -1;
    };
    match s.load_rom_path(Path::new(path)) {
        Ok(()) => 0,
        Err(e) => {
            set_last_error(e.to_string());
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn sc_load_rom_bytes(handle: *mut c_void, data: *const u8, len: usize) -> c_int {
    clear_last_error();
    let Some(s) = session_mut(handle) else {
        set_last_error("null handle");
        return -1;
    };
    if data.is_null() || len == 0 {
        set_last_error("empty rom");
        return -1;
    }
    // SAFETY: caller guarantees `data` points to `len` readable bytes.
    let slice = unsafe { std::slice::from_raw_parts(data, len) };
    match s.load_rom_bytes(slice) {
        Ok(()) => 0,
        Err(e) => {
            set_last_error(e.to_string());
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn sc_reset(handle: *mut c_void) -> c_int {
    clear_last_error();
    let Some(s) = session_mut(handle) else {
        set_last_error("null handle");
        return -1;
    };
    match s.reset() {
        Ok(()) => 0,
        Err(e) => {
            set_last_error(e.to_string());
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn sc_set_running(handle: *mut c_void, running: c_int) {
    if let Some(s) = session_mut(handle) {
        s.set_running(running != 0);
    }
}

#[no_mangle]
pub extern "C" fn sc_set_border(handle: *mut c_void, with_border: c_int) {
    if let Some(s) = session_mut(handle) {
        s.set_border(with_border != 0);
    }
}

#[no_mangle]
pub extern "C" fn sc_run_frame(handle: *mut c_void) {
    if let Some(s) = session_mut(handle) {
        s.run_frame();
    }
}

/// Pointer to RGBA8 framebuffer (row-major). Invalidated by destroy / border change.
#[no_mangle]
pub extern "C" fn sc_framebuffer_ptr(handle: *mut c_void) -> *const u8 {
    session_mut(handle).map_or(ptr::null(), |s| s.framebuffer().as_ptr())
}

#[no_mangle]
pub extern "C" fn sc_framebuffer_width(handle: *mut c_void) -> c_uint {
    session_mut(handle).map_or(0, |s| s.width() as c_uint)
}

#[no_mangle]
pub extern "C" fn sc_framebuffer_height(handle: *mut c_void) -> c_uint {
    session_mut(handle).map_or(0, |s| s.height() as c_uint)
}

#[no_mangle]
pub extern "C" fn sc_open_tape(handle: *mut c_void, path: *const c_char) -> c_int {
    clear_last_error();
    let Some(s) = session_mut(handle) else {
        set_last_error("null handle");
        return -1;
    };
    if path.is_null() {
        set_last_error("null path");
        return -1;
    }
    // SAFETY: valid NUL-terminated C string from caller.
    let cstr = unsafe { CStr::from_ptr(path) };
    let Ok(path) = cstr.to_str() else {
        set_last_error("path not utf-8");
        return -1;
    };
    match s.open_tape(Path::new(path)) {
        Ok(()) => 0,
        Err(e) => {
            set_last_error(e.to_string());
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn sc_tape_play(handle: *mut c_void) -> c_int {
    clear_last_error();
    let Some(s) = session_mut(handle) else {
        set_last_error("null handle");
        return -1;
    };
    match s.play_tape() {
        Ok(()) => 0,
        Err(e) => {
            set_last_error(e.to_string());
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn sc_tape_pause(handle: *mut c_void) -> c_int {
    clear_last_error();
    let Some(s) = session_mut(handle) else {
        set_last_error("null handle");
        return -1;
    };
    match s.pause_tape() {
        Ok(()) => 0,
        Err(e) => {
            set_last_error(e.to_string());
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn sc_tape_rewind(handle: *mut c_void) -> c_int {
    clear_last_error();
    let Some(s) = session_mut(handle) else {
        set_last_error("null handle");
        return -1;
    };
    match s.rewind_tape() {
        Ok(()) => 0,
        Err(e) => {
            set_last_error(e.to_string());
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn sc_tape_playing(handle: *mut c_void) -> c_int {
    session_mut(handle).is_some_and(|s| s.tape_playing()) as c_int
}

#[no_mangle]
pub extern "C" fn sc_has_tape(handle: *mut c_void) -> c_int {
    session_mut(handle).is_some_and(|s| s.has_tape()) as c_int
}

/// Fill out-params with tape progress. Returns 0 on success, -1 if no tape/handle.
#[no_mangle]
pub extern "C" fn sc_tape_progress(
    handle: *mut c_void,
    block_index: *mut c_uint,
    block_count: *mut c_uint,
    pulse_index: *mut c_uint,
    pulse_count: *mut c_uint,
) -> c_int {
    let Some(s) = session_mut(handle) else {
        return -1;
    };
    let Some(p) = s.tape_progress() else {
        return -1;
    };
    // SAFETY: caller-provided out pointers; null means skip that field.
    unsafe {
        if !block_index.is_null() {
            *block_index = p.block_index;
        }
        if !block_count.is_null() {
            *block_count = p.block_count;
        }
        if !pulse_index.is_null() {
            *pulse_index = p.pulse_index;
        }
        if !pulse_count.is_null() {
            *pulse_count = p.pulse_count;
        }
    }
    0
}

/// Read tape load options. `flash_load`/`speed` may be null to skip.
#[no_mangle]
pub extern "C" fn sc_tape_get_load_options(
    handle: *mut c_void,
    flash_load: *mut c_int,
    speed: *mut c_uint,
) -> c_int {
    let Some(s) = session_mut(handle) else {
        return -1;
    };
    let Some(opts) = s.tape_load_options() else {
        return -1;
    };
    // SAFETY: optional out-params from caller.
    unsafe {
        if !flash_load.is_null() {
            *flash_load = i32::from(opts.flash_load);
        }
        if !speed.is_null() {
            *speed = opts.speed;
        }
    }
    0
}

/// Set instant flash-load and EAR speed multiplier (clamped to 1..=64).
#[no_mangle]
pub extern "C" fn sc_tape_set_load_options(
    handle: *mut c_void,
    flash_load: c_int,
    speed: c_uint,
) -> c_int {
    clear_last_error();
    let Some(s) = session_mut(handle) else {
        set_last_error("null handle");
        return -1;
    };
    match s.set_tape_load_options(machine::TapeLoadOptions {
        flash_load: flash_load != 0,
        speed,
    }) {
        Ok(()) => 0,
        Err(e) => {
            set_last_error(e.to_string());
            -1
        }
    }
}

/// Pointer to mono f32 PCM from the last `sc_run_frame` (valid until next mutating call).
#[no_mangle]
pub extern "C" fn sc_audio_ptr(handle: *mut c_void) -> *const f32 {
    session_mut(handle)
        .map(|s| s.audio_pcm().as_ptr())
        .unwrap_or(ptr::null())
}

/// Number of mono samples in [`sc_audio_ptr`].
#[no_mangle]
pub extern "C" fn sc_audio_frames(handle: *mut c_void) -> c_uint {
    session_mut(handle)
        .map(|s| s.audio_pcm().len() as c_uint)
        .unwrap_or(0)
}

/// Host audio sample rate (Hz).
#[no_mangle]
pub extern "C" fn sc_audio_sample_rate(_handle: *mut c_void) -> c_uint {
    crate::session::AUDIO_SAMPLE_RATE
}

#[no_mangle]
pub extern "C" fn sc_set_key(
    handle: *mut c_void,
    row: c_uint,
    bit: c_uint,
    pressed: c_int,
) -> c_int {
    clear_last_error();
    let Some(s) = session_mut(handle) else {
        set_last_error("null handle");
        return -1;
    };
    match s.set_key(row as usize, bit as u8, pressed != 0) {
        Ok(()) => 0,
        Err(e) => {
            set_last_error(e.to_string());
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn sc_clear_keys(handle: *mut c_void) -> c_int {
    clear_last_error();
    let Some(s) = session_mut(handle) else {
        set_last_error("null handle");
        return -1;
    };
    match s.clear_keys() {
        Ok(()) => 0,
        Err(e) => {
            set_last_error(e.to_string());
            -1
        }
    }
}

/// Heap-allocated UTF-8 C string; free with [`sc_string_free`].
#[no_mangle]
pub extern "C" fn sc_status(handle: *mut c_void) -> *mut c_char {
    let Some(s) = session_mut(handle) else {
        return ptr::null_mut();
    };
    CString::new(s.status().replace('\0', ""))
        .map(CString::into_raw)
        .unwrap_or(ptr::null_mut())
}

/// Heap-allocated last error; free with [`sc_string_free`]. May be null.
#[no_mangle]
pub extern "C" fn sc_last_error() -> *mut c_char {
    LAST_ERROR.with(|slot| {
        slot.lock()
            .ok()
            .and_then(|mut g| g.take())
            .map_or_else(ptr::null_mut, CString::into_raw)
    })
}

#[no_mangle]
pub extern "C" fn sc_string_free(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    // SAFETY: string from `CString::into_raw` via sc_status / sc_last_error.
    drop(unsafe { CString::from_raw(s) });
}

/// Apply `SPEC_CHUM_DEBUG` / `SPEC_CHUM_TRACE` (idempotent).
#[no_mangle]
pub extern "C" fn sc_debug_init_from_env() {
    trace::init_from_env();
}

/// Set enabled trace categories (bitmask: cpu=1, bus=2, tape=4, ula=8, machine=16).
#[no_mangle]
pub extern "C" fn sc_debug_set_categories(cats: c_uint) {
    trace::enable(trace::Category::from_bits(u64::from(cats)));
}

#[no_mangle]
pub extern "C" fn sc_debug_get_categories() -> c_uint {
    trace::categories().bits() as c_uint
}

#[no_mangle]
pub extern "C" fn sc_debug_clear() {
    trace::clear();
}

/// Heap-allocated UTF-8 dump of the ring; free with [`sc_string_free`].
#[no_mangle]
pub extern "C" fn sc_debug_dump() -> *mut c_char {
    let s = trace::dump_string().replace('\0', "");
    CString::new(s)
        .map(CString::into_raw)
        .unwrap_or(ptr::null_mut())
}

/// Write the ring dump to `path`. Returns 0 on success.
#[no_mangle]
pub extern "C" fn sc_debug_dump_to_file(path: *const c_char) -> c_int {
    clear_last_error();
    if path.is_null() {
        set_last_error("null path");
        return -1;
    }
    // SAFETY: caller-provided C string.
    let cstr = unsafe { CStr::from_ptr(path) };
    let Ok(path) = cstr.to_str() else {
        set_last_error("path not utf-8");
        return -1;
    };
    match trace::dump_to_file(path) {
        Ok(()) => 0,
        Err(e) => {
            set_last_error(e.to_string());
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn sc_debug_event_count() -> c_uint {
    trace::len() as c_uint
}

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
}
