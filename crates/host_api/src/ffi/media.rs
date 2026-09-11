//! Snapshot / disk media and title C ABI.

use std::ffi::CStr;
use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_void};
use std::path::Path;
use std::ptr;

use super::{clear_last_error, session_mut, set_last_error};

/// Heap-allocated UTF-8 display title for the inserted tape; free with [`sc_string_free`].
#[no_mangle]
pub extern "C" fn sc_media_title(handle: *mut c_void) -> *mut c_char {
    let Some(mut s) = session_mut(handle) else {
        return ptr::null_mut();
    };
    let Some(title) = s.media_title() else {
        return ptr::null_mut();
    };
    CString::new(title.replace('\0', "")).map_or(ptr::null_mut(), CString::into_raw)
}

/// Heap-allocated lowercase SHA-512 hex of the inserted tape; free with [`sc_string_free`].
#[no_mangle]
pub extern "C" fn sc_media_sha512(handle: *mut c_void) -> *mut c_char {
    let Some(mut s) = session_mut(handle) else {
        return ptr::null_mut();
    };
    let Some(hash) = s.media_sha512() else {
        return ptr::null_mut();
    };
    CString::new(hash.replace('\0', "")).map_or(ptr::null_mut(), CString::into_raw)
}

#[no_mangle]
pub extern "C" fn sc_load_snapshot(handle: *mut c_void, path: *const c_char) -> c_int {
    clear_last_error();
    let Some(mut s) = session_mut(handle) else {
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
    let Some(mut s) = session_mut(handle) else {
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
    let Some(mut s) = session_mut(handle) else {
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
    let Some(mut s) = session_mut(handle) else {
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
    let Some(mut s) = session_mut(handle) else {
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
