//! C ABI for Spec Chum host sessions.
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

/// Active model id (`SC_MODEL_*`). Returns `UINT_MAX` on a null handle.
#[no_mangle]
pub extern "C" fn sc_get_model(handle: *mut c_void) -> c_uint {
    clear_last_error();
    let Some(s) = session_mut(handle) else {
        set_last_error("null handle");
        return c_uint::MAX;
    };
    s.model() as c_uint
}

/// Returns 1 when required ROM slots are present (persisted paths or workspace search).
#[no_mangle]
pub extern "C" fn sc_model_rom_available(model: c_uint) -> c_int {
    clear_last_error();
    let Some(model) = ModelId::from_u32(model) else {
        set_last_error("invalid model id");
        return 0;
    };
    let paths = crate::rom_setup::model_rom_paths_snapshot();
    i32::from(crate::rom_setup::model_rom_available(model, &paths))
}

/// Heap JSON describing required ROM slots + status; free with [`sc_string_free`].
#[no_mangle]
pub extern "C" fn sc_model_rom_setup_json(model: c_uint) -> *mut c_char {
    clear_last_error();
    let Some(model) = ModelId::from_u32(model) else {
        set_last_error("invalid model id");
        return ptr::null_mut();
    };
    let paths = crate::rom_setup::model_rom_paths_snapshot();
    match serde_json::to_string(&crate::rom_setup::rom_setup_json(model, &paths)) {
        Ok(json) => heap_cstring(json),
        Err(e) => {
            set_last_error(format!("rom setup json: {e}"));
            ptr::null_mut()
        }
    }
}

/// Replace the process-global persisted ROM path map (macOS UserDefaults mirror).
#[no_mangle]
pub extern "C" fn sc_sync_model_rom_paths_json(json: *const c_char) -> c_int {
    clear_last_error();
    if json.is_null() {
        crate::rom_setup::sync_model_rom_paths(std::collections::BTreeMap::new());
        return 0;
    }
    // SAFETY: caller provides a valid NUL-terminated C string.
    let cstr = unsafe { CStr::from_ptr(json) };
    let Ok(text) = cstr.to_str() else {
        set_last_error("json not utf-8");
        return -1;
    };
    match serde_json::from_str::<std::collections::BTreeMap<String, String>>(text) {
        Ok(map) => {
            crate::rom_setup::sync_model_rom_paths(map);
            0
        }
        Err(e) => {
            set_last_error(format!("model rom paths json: {e}"));
            -1
        }
    }
}

/// Heap JSON map of persisted ROM paths; free with [`sc_string_free`].
#[no_mangle]
pub extern "C" fn sc_model_rom_paths_json() -> *mut c_char {
    clear_last_error();
    match serde_json::to_string(&crate::rom_setup::model_rom_paths_snapshot()) {
        Ok(json) => heap_cstring(json),
        Err(e) => {
            set_last_error(format!("model rom paths json: {e}"));
            ptr::null_mut()
        }
    }
}

/// Validate `source_path`, persist it, and best-effort copy into `roms/`. Returns 0 on success.
#[no_mangle]
pub extern "C" fn sc_install_model_rom(
    model: c_uint,
    slot_id: *const c_char,
    source_path: *const c_char,
) -> c_int {
    clear_last_error();
    let Some(model) = ModelId::from_u32(model) else {
        set_last_error("invalid model id");
        return -1;
    };
    if slot_id.is_null() || source_path.is_null() {
        set_last_error("null slot_id or source_path");
        return -1;
    }
    // SAFETY: caller provides valid NUL-terminated C strings.
    let slot = unsafe { CStr::from_ptr(slot_id) };
    let source = unsafe { CStr::from_ptr(source_path) };
    let Ok(slot_id) = slot.to_str() else {
        set_last_error("slot_id not utf-8");
        return -1;
    };
    let Ok(source_path) = source.to_str() else {
        set_last_error("source_path not utf-8");
        return -1;
    };
    let mut paths = crate::rom_setup::model_rom_paths_snapshot();
    match crate::rom_setup::install_model_rom(model, slot_id, Path::new(source_path), &mut paths) {
        Ok(_) => 0,
        Err(e) => {
            set_last_error(e);
            -1
        }
    }
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
pub extern "C" fn sc_load_snapshot(handle: *mut c_void, path: *const c_char) -> c_int {
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
    match s.load_snapshot(Path::new(path)) {
        Ok(()) => 0,
        Err(e) => {
            set_last_error(e.to_string());
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn sc_load_rzx(handle: *mut c_void, path: *const c_char) -> c_int {
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
    match s.load_rzx(Path::new(path)) {
        Ok(()) => 0,
        Err(e) => {
            set_last_error(e.to_string());
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn sc_load_dsk(handle: *mut c_void, path: *const c_char) -> c_int {
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
    match s.load_dsk(Path::new(path)) {
        Ok(()) => 0,
        Err(e) => {
            set_last_error(e.to_string());
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn sc_load_trd(handle: *mut c_void, path: *const c_char) -> c_int {
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
    match s.load_trd(Path::new(path)) {
        Ok(()) => 0,
        Err(e) => {
            set_last_error(e.to_string());
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn sc_load_trdos_rom(handle: *mut c_void, path: *const c_char) -> c_int {
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
    match s.load_trdos_rom(Path::new(path)) {
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

/// Read tape load options (flash + speed). Out-params may be null to skip.
///
/// Preserves the historical three-argument C ABI. Prefer
/// [`sc_tape_get_load_options_ex`] when experience mode is needed.
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

/// Read tape load options including experience mode (#82). Out-params may be null.
#[no_mangle]
pub extern "C" fn sc_tape_get_load_options_ex(
    handle: *mut c_void,
    flash_load: *mut c_int,
    speed: *mut c_uint,
    experience_load: *mut c_int,
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
        if !experience_load.is_null() {
            *experience_load = i32::from(opts.experience_load);
        }
    }
    0
}

/// Set instant flash-load and EAR speed multiplier (1..64).
///
/// Preserves the historical three-argument C ABI. When `flash_load` is enabled,
/// clears `experience_load` so Instant can override a prior Experience selection
/// (options normalization prefers Experience when both flags are set and would
/// otherwise drop flash). Use [`sc_tape_set_load_options_ex`] to set experience
/// mode explicitly.
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
    let flash = flash_load != 0;
    let experience_load = if flash {
        false
    } else {
        s.tape_load_options()
            .map(|o| o.experience_load)
            .unwrap_or(false)
    };
    match s.set_tape_load_options(machine::TapeLoadOptions {
        flash_load: flash,
        speed,
        experience_load,
    }) {
        Ok(()) => 0,
        Err(e) => {
            set_last_error(e.to_string());
            -1
        }
    }
}

/// Set instant flash-load, EAR speed multiplier (1..64), and experience mode (#82).
#[no_mangle]
pub extern "C" fn sc_tape_set_load_options_ex(
    handle: *mut c_void,
    flash_load: c_int,
    speed: c_uint,
    experience_load: c_int,
) -> c_int {
    clear_last_error();
    let Some(s) = session_mut(handle) else {
        set_last_error("null handle");
        return -1;
    };
    match s.set_tape_load_options(machine::TapeLoadOptions {
        flash_load: flash_load != 0,
        speed,
        experience_load: experience_load != 0,
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

#[no_mangle]
pub extern "C" fn sc_set_joystick_mode(handle: *mut c_void, mode: c_uint) -> c_int {
    clear_last_error();
    let Some(s) = session_mut(handle) else {
        set_last_error("null handle");
        return -1;
    };
    // Validate before truncating c_uint → u8 (e.g. 256 must not become Kempston).
    if mode > u8::MAX as c_uint {
        set_last_error("invalid joystick mode");
        return -1;
    }
    let Some(mode) = machine::JoystickMode::from_u8(mode as u8) else {
        set_last_error("invalid joystick mode");
        return -1;
    };
    match s.set_joystick_mode(mode) {
        Ok(()) => 0,
        Err(e) => {
            set_last_error(e.to_string());
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn sc_set_joystick(handle: *mut c_void, mask: c_uint) -> c_int {
    clear_last_error();
    let Some(s) = session_mut(handle) else {
        set_last_error("null handle");
        return -1;
    };
    match s.set_joystick(mask as u8) {
        Ok(()) => 0,
        Err(e) => {
            set_last_error(e.to_string());
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn sc_clear_joystick(handle: *mut c_void) -> c_int {
    clear_last_error();
    let Some(s) = session_mut(handle) else {
        set_last_error("null handle");
        return -1;
    };
    match s.clear_joystick() {
        Ok(()) => 0,
        Err(e) => {
            set_last_error(e.to_string());
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn sc_set_mouse_delta(handle: *mut c_void, dx: c_int, dy: c_int) -> c_int {
    clear_last_error();
    let Some(s) = session_mut(handle) else {
        set_last_error("null handle");
        return -1;
    };
    let dx = dx.clamp(i32::from(i8::MIN), i32::from(i8::MAX)) as i8;
    let dy = dy.clamp(i32::from(i8::MIN), i32::from(i8::MAX)) as i8;
    match s.set_mouse_delta(dx, dy) {
        Ok(()) => 0,
        Err(e) => {
            set_last_error(e.to_string());
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn sc_set_mouse_buttons(
    handle: *mut c_void,
    left: c_int,
    right: c_int,
    middle: c_int,
) -> c_int {
    clear_last_error();
    let Some(s) = session_mut(handle) else {
        set_last_error("null handle");
        return -1;
    };
    match s.set_mouse_buttons(left != 0, right != 0, middle != 0) {
        Ok(()) => 0,
        Err(e) => {
            set_last_error(e.to_string());
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn sc_attach_multiface(handle: *mut c_void, path: *const c_char) -> c_int {
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
    match s.attach_multiface(Path::new(path)) {
        Ok(()) => 0,
        Err(e) => {
            set_last_error(e.to_string());
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn sc_multiface_nmi(handle: *mut c_void) -> c_int {
    clear_last_error();
    let Some(s) = session_mut(handle) else {
        set_last_error("null handle");
        return -1;
    };
    match s.multiface_nmi() {
        Ok(()) => 0,
        Err(e) => {
            set_last_error(e.to_string());
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn sc_has_multiface(handle: *mut c_void) -> c_int {
    let Some(s) = session_mut(handle) else {
        return 0;
    };
    i32::from(s.has_multiface())
}

/* Interface 1 + Microdrive (48K/128K). Attach/load/insert return 0 ok, -1 error. */
#[no_mangle]
pub extern "C" fn sc_attach_interface1(handle: *mut c_void) -> c_int {
    clear_last_error();
    let Some(s) = session_mut(handle) else {
        set_last_error("null handle");
        return -1;
    };
    match s.attach_interface1() {
        Ok(()) => 0,
        Err(e) => {
            set_last_error(e.to_string());
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn sc_load_interface1_rom(handle: *mut c_void, path: *const c_char) -> c_int {
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
    match s.load_interface1_rom(Path::new(path)) {
        Ok(()) => 0,
        Err(e) => {
            set_last_error(e.to_string());
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn sc_insert_mdr(handle: *mut c_void, path: *const c_char) -> c_int {
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
    match s.insert_mdr(Path::new(path)) {
        Ok(()) => 0,
        Err(e) => {
            set_last_error(e.to_string());
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn sc_has_interface1(handle: *mut c_void) -> c_int {
    // Returns 1 if Interface 1 is attached, 0 if absent or handle is null.
    let Some(s) = session_mut(handle) else {
        return 0;
    };
    i32::from(s.has_interface1())
}

#[no_mangle]
pub extern "C" fn sc_attach_divmmc(handle: *mut c_void) -> c_int {
    clear_last_error();
    let Some(s) = session_mut(handle) else {
        set_last_error("null handle");
        return -1;
    };
    match s.attach_divmmc() {
        Ok(()) => 0,
        Err(e) => {
            set_last_error(e.to_string());
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn sc_load_divmmc_sd(handle: *mut c_void, path: *const c_char) -> c_int {
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
    match s.load_divmmc_sd(Path::new(path)) {
        Ok(()) => 0,
        Err(e) => {
            set_last_error(e.to_string());
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn sc_load_divmmc_eeprom(handle: *mut c_void, path: *const c_char) -> c_int {
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
    match s.load_divmmc_eeprom(Path::new(path)) {
        Ok(()) => 0,
        Err(e) => {
            set_last_error(e.to_string());
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn sc_has_divmmc(handle: *mut c_void) -> c_int {
    let Some(s) = session_mut(handle) else {
        return 0;
    };
    i32::from(s.has_divmmc())
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
    // SAFETY: string from `CString::into_raw` via sc_status / sc_last_error /
    // sc_inspect_json / sc_debug_dump / sc_debug_dump_json.
    drop(unsafe { CString::from_raw(s) });
}

/// Apply `SPEC_CHUM_DEBUG` / `SPEC_CHUM_TRACE` (idempotent).
#[no_mangle]
pub extern "C" fn sc_debug_init_from_env() {
    trace::init_from_env();
}

/// Set enabled trace categories (bitmask: cpu=1, bus=2, tape=4, ula=8, machine=16, ay=32, disk=64, mem=128).
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

fn heap_cstring(s: String) -> *mut c_char {
    CString::new(s.replace('\0', ""))
        .map(CString::into_raw)
        .unwrap_or(ptr::null_mut())
}

fn break_reason_code(reason: machine::BreakReason) -> c_int {
    match reason {
        machine::BreakReason::None => 0,
        machine::BreakReason::Pc(_) => 1,
        machine::BreakReason::Mem { .. } => 2,
        machine::BreakReason::Port { .. } => 3,
        machine::BreakReason::Halt => 4,
        machine::BreakReason::Budget => 5,
    }
}

fn require_u16(addr: c_uint, what: &str) -> Option<u16> {
    if addr > 0xffff {
        set_last_error(format!("{what} out of range"));
        None
    } else {
        Some(addr as u16)
    }
}

/// Peek one memory byte. Returns 0 on success, -1 on error.
#[no_mangle]
pub extern "C" fn sc_peek(handle: *mut c_void, addr: c_uint, out: *mut u8) -> c_int {
    clear_last_error();
    let Some(s) = session_mut(handle) else {
        set_last_error("null handle");
        return -1;
    };
    if out.is_null() {
        set_last_error("null out");
        return -1;
    }
    let Some(addr) = require_u16(addr, "addr") else {
        return -1;
    };
    match s.peek(addr) {
        Ok(value) => {
            // SAFETY: caller-provided out pointer.
            unsafe {
                *out = value;
            }
            0
        }
        Err(e) => {
            set_last_error(e.to_string());
            -1
        }
    }
}

/// Poke one memory byte. Returns 0 on success, -1 on error.
#[no_mangle]
pub extern "C" fn sc_poke(handle: *mut c_void, addr: c_uint, value: u8) -> c_int {
    clear_last_error();
    let Some(s) = session_mut(handle) else {
        set_last_error("null handle");
        return -1;
    };
    let Some(addr) = require_u16(addr, "addr") else {
        return -1;
    };
    match s.poke(addr, value) {
        Ok(()) => 0,
        Err(e) => {
            set_last_error(e.to_string());
            -1
        }
    }
}

/// Heap-allocated UTF-8 JSON of [`machine::Inspect`]; free with [`sc_string_free`].
#[no_mangle]
pub extern "C" fn sc_inspect_json(handle: *mut c_void) -> *mut c_char {
    clear_last_error();
    let Some(s) = session_mut(handle) else {
        set_last_error("null handle");
        return ptr::null_mut();
    };
    match s.inspect_json() {
        Ok(json) => heap_cstring(json),
        Err(e) => {
            set_last_error(e.to_string());
            ptr::null_mut()
        }
    }
}

/// Fill `pc,sp,af,bc,de,hl,ix,iy`. Null out-params are skipped. Returns 0 on success.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn sc_regs(
    handle: *mut c_void,
    pc: *mut u16,
    sp: *mut u16,
    af: *mut u16,
    bc: *mut u16,
    de: *mut u16,
    hl: *mut u16,
    ix: *mut u16,
    iy: *mut u16,
) -> c_int {
    clear_last_error();
    let Some(s) = session_mut(handle) else {
        set_last_error("null handle");
        return -1;
    };
    let r = match s.regs() {
        Ok(r) => r,
        Err(e) => {
            set_last_error(e.to_string());
            return -1;
        }
    };
    // SAFETY: optional out-params from caller.
    unsafe {
        if !pc.is_null() {
            *pc = r.pc;
        }
        if !sp.is_null() {
            *sp = r.sp;
        }
        if !af.is_null() {
            *af = r.af;
        }
        if !bc.is_null() {
            *bc = r.bc;
        }
        if !de.is_null() {
            *de = r.de;
        }
        if !hl.is_null() {
            *hl = r.hl;
        }
        if !ix.is_null() {
            *ix = r.ix;
        }
        if !iy.is_null() {
            *iy = r.iy;
        }
    }
    0
}

/// One [`machine::Machine::step_once`]. Returns 0 on success, -1 if no machine.
#[no_mangle]
pub extern "C" fn sc_step(handle: *mut c_void) -> c_int {
    clear_last_error();
    let Some(s) = session_mut(handle) else {
        set_last_error("null handle");
        return -1;
    };
    match s.step() {
        Ok(()) => 0,
        Err(e) => {
            set_last_error(e.to_string());
            -1
        }
    }
}

/// Set debugger paused (no-op on null handle / no machine).
#[no_mangle]
pub extern "C" fn sc_set_paused(handle: *mut c_void, paused: c_int) {
    if let Some(s) = session_mut(handle) {
        s.set_paused(paused != 0);
    }
}

/// Add a PC breakpoint. Returns 0 on success, -1 on error.
#[no_mangle]
pub extern "C" fn sc_add_breakpoint(handle: *mut c_void, pc: c_uint) -> c_int {
    clear_last_error();
    let Some(s) = session_mut(handle) else {
        set_last_error("null handle");
        return -1;
    };
    let Some(pc) = require_u16(pc, "pc") else {
        return -1;
    };
    match s.add_breakpoint(pc) {
        Ok(()) => 0,
        Err(e) => {
            set_last_error(e.to_string());
            -1
        }
    }
}

/// Run until a break. Returns reason: 0 none, 1 pc, 2 mem, 3 port, 4 halt, 5 budget, -1 error.
#[no_mangle]
pub extern "C" fn sc_run_until_break(handle: *mut c_void, max_insns: c_uint) -> c_int {
    clear_last_error();
    let Some(s) = session_mut(handle) else {
        set_last_error("null handle");
        return -1;
    };
    match s.run_until_break(max_insns) {
        Ok(reason) => break_reason_code(reason),
        Err(e) => {
            set_last_error(e.to_string());
            -1
        }
    }
}

/// Heap-allocated UTF-8 JSON dump of the trace ring; free with [`sc_string_free`].
#[no_mangle]
pub extern "C" fn sc_debug_dump_json() -> *mut c_char {
    heap_cstring(trace::dump_json())
}

/// Apply a [`crate::machine_config::UserMachineConfig`] from JSON (#187).
#[no_mangle]
pub extern "C" fn sc_apply_user_config_json(handle: *mut c_void, json: *const c_char) -> c_int {
    clear_last_error();
    let Some(s) = session_mut(handle) else {
        set_last_error("null handle");
        return -1;
    };
    if json.is_null() {
        set_last_error("null json");
        return -1;
    }
    // SAFETY: caller provides a valid NUL-terminated C string.
    let cstr = unsafe { CStr::from_ptr(json) };
    let Ok(text) = cstr.to_str() else {
        set_last_error("json not utf-8");
        return -1;
    };
    let config: crate::machine_config::UserMachineConfig = match serde_json::from_str(text) {
        Ok(c) => c,
        Err(e) => {
            set_last_error(format!("invalid config json: {e}"));
            return -1;
        }
    };
    match s.apply_user_config(&config) {
        Ok(()) => 0,
        Err(e) => {
            set_last_error(e.to_string());
            -1
        }
    }
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
        assert_eq!(sc_peek(ptr::null_mut(), 0, &mut out), -1);
        assert_eq!(out, 0x5A);

        let h = sc_create(0, 1);
        assert!(!h.is_null());
        assert_eq!(sc_peek(h, 0, &mut out), -1);
        assert_eq!(sc_peek(h, 0x1_0000, &mut out), -1);
        assert_eq!(sc_step(h), -1);
        assert_eq!(sc_run_until_break(h, 1), -1);
        sc_destroy(h);
    }
}
