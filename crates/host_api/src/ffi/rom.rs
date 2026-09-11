//! ROM setup / install / load C ABI.

use std::ffi::CStr;
use std::os::raw::{c_char, c_int, c_uint, c_void};
use std::path::Path;
use std::ptr;

use crate::session::ModelId;

use super::{clear_last_error, heap_cstring, session_mut, set_last_error};

/// Returns 1 when the model's ROM dumps are never auto-fetched (user must supply paths).
#[no_mangle]
pub extern "C" fn sc_model_requires_user_rom(model: c_uint) -> c_int {
    clear_last_error();
    let Some(model) = ModelId::from_u32(model) else {
        set_last_error("invalid model id");
        return 0;
    };
    i32::from(crate::rom_setup::model_requires_user_rom(model))
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
        Ok(json) => heap_cstring(&json),
        Err(e) => {
            set_last_error(format!("rom setup json: {e}"));
            ptr::null_mut()
        }
    }
}

/// Replace the process-global persisted ROM path map (macOS `UserDefaults` mirror).
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
        Ok(json) => heap_cstring(&json),
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
            set_last_error(e.to_string());
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn sc_load_rom(handle: *mut c_void, path: *const c_char) -> c_int {
    clear_last_error();
    let Some(mut s) = session_mut(handle) else {
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
    let Some(mut s) = session_mut(handle) else {
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
