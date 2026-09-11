//! Status / last-error / string free C ABI.

use std::ffi::CString;
use std::os::raw::{c_char, c_void};
use std::ptr;

use super::{session_mut, LAST_ERROR};

/// Heap-allocated UTF-8 C string; free with [`sc_string_free`].
#[no_mangle]
pub extern "C" fn sc_status(handle: *mut c_void) -> *mut c_char {
    let Some(mut s) = session_mut(handle) else {
        return ptr::null_mut();
    };
    CString::new(s.status().replace('\0', "")).map_or(ptr::null_mut(), CString::into_raw)
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
    // sc_media_title / sc_media_sha512 / sc_inspect_json / sc_debug_dump /
    // sc_debug_dump_json.
    drop(unsafe { CString::from_raw(s) });
}
