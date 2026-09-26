//! C ABI access to the shared typed model catalog.

use std::os::raw::c_char;

use super::{clear_last_error, heap_cstring, set_last_error};

/// Heap JSON for the canonical host model catalog; free the result with `sc_string_free`.
#[no_mangle]
pub extern "C" fn sc_model_catalog_json() -> *mut c_char {
    clear_last_error();
    match serde_json::to_string(&crate::host_model_catalog()) {
        Ok(json) => heap_cstring(&json),
        Err(error) => {
            set_last_error(format!("model catalog json: {error}"));
            std::ptr::null_mut()
        }
    }
}
