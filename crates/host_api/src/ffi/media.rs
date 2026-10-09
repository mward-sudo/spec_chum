//! Snapshot / disk media and title C ABI.

use std::ffi::CStr;
use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_void};
use std::path::Path;
use std::ptr;

use super::{clear_last_error, heap_cstring, session_mut, set_last_error};
use crate::media_library::{query_recent_media, MediaCategory};

/// Query supported recent media without changing files or session state.
/// `recent_paths_json` must be a JSON array of UTF-8 path strings; null search
/// and category mean no filter. The returned JSON array is freed with
/// [`super::sc_string_free`].
#[no_mangle]
pub extern "C" fn sc_media_library_json(
    handle: *mut c_void,
    recent_paths_json: *const c_char,
    search: *const c_char,
    category: *const c_char,
) -> *mut c_char {
    clear_last_error();
    let Some(session) = session_mut(handle) else {
        set_last_error("null handle");
        return ptr::null_mut();
    };
    if recent_paths_json.is_null() {
        set_last_error("null recent paths");
        return ptr::null_mut();
    }
    // SAFETY: caller supplies valid NUL-terminated strings or null optional filters.
    let Ok(paths_json) = unsafe { CStr::from_ptr(recent_paths_json) }.to_str() else {
        set_last_error("recent paths are not utf-8");
        return ptr::null_mut();
    };
    let paths: Vec<String> = match serde_json::from_str(paths_json) {
        Ok(paths) => paths,
        Err(error) => {
            set_last_error(format!("invalid recent paths JSON: {error}"));
            return ptr::null_mut();
        }
    };
    let search = if search.is_null() {
        ""
    } else {
        // SAFETY: non-null caller string is NUL-terminated.
        let Ok(search) = (unsafe { CStr::from_ptr(search) }).to_str() else {
            set_last_error("search is not utf-8");
            return ptr::null_mut();
        };
        search
    };
    let category = if category.is_null() {
        None
    } else {
        // SAFETY: non-null caller string is NUL-terminated.
        match unsafe { CStr::from_ptr(category) }.to_str() {
            Ok("" | "all") => None,
            Ok(name) => {
                let Some(category) = MediaCategory::from_name(name) else {
                    set_last_error("invalid media category");
                    return ptr::null_mut();
                };
                Some(category)
            }
            Err(_) => {
                set_last_error("media category is not utf-8");
                return ptr::null_mut();
            }
        }
    };
    match serde_json::to_string(&query_recent_media(&paths, search, category, &session)) {
        Ok(json) => heap_cstring(&json),
        Err(error) => {
            set_last_error(error.to_string());
            ptr::null_mut()
        }
    }
}

/// Open supported user media via the shared format and hardware policy.
#[no_mangle]
pub extern "C" fn sc_open_media(handle: *mut c_void, path: *const c_char) -> c_int {
    clear_last_error();
    let Some(mut session) = session_mut(handle) else {
        set_last_error("null handle");
        return -1;
    };
    if path.is_null() {
        set_last_error("null path");
        return -1;
    }
    // SAFETY: caller supplies a valid NUL-terminated path.
    let Ok(path) = unsafe { CStr::from_ptr(path) }.to_str() else {
        set_last_error("path is not utf-8");
        return -1;
    };
    match session.open_media_path(Path::new(path)) {
        Ok(()) => 0,
        Err(error) => {
            set_last_error(error.to_string());
            -1
        }
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::{sc_create, sc_destroy};

    #[test]
    fn library_json_reports_supported_paths_and_machine_requirements() {
        let handle = sc_create(0, 0);
        assert!(!handle.is_null());
        let paths =
            CString::new(r#"["/Missing/Game.TAP","/Missing/Game.SZX"]"#).expect("valid path JSON");
        let search = CString::new("game").expect("valid search");
        let category = CString::new("tape").expect("valid category");
        let raw = sc_media_library_json(handle, paths.as_ptr(), search.as_ptr(), category.as_ptr());
        assert!(!raw.is_null());
        // SAFETY: JSON pointer is the heap allocation returned by this FFI call.
        let json = unsafe { CString::from_raw(raw) };
        let entries: serde_json::Value =
            serde_json::from_str(json.to_str().expect("UTF-8 JSON")).expect("valid JSON");
        assert_eq!(entries.as_array().map(Vec::len), Some(1));
        assert_eq!(entries[0]["path"], "/Missing/Game.TAP");
        assert_eq!(entries[0]["format"], "tap");
        assert_eq!(entries[0]["compatibility"], "requires_machine");
        sc_destroy(handle);
    }
}
