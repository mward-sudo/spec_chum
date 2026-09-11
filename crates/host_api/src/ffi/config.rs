//! User machine config JSON C ABI.

use std::ffi::CStr;
use std::os::raw::{c_char, c_int, c_void};

use super::{clear_last_error, session_mut, set_last_error};

/// Apply a [`crate::machine_config::UserMachineConfig`] from JSON (#187).
#[no_mangle]
pub extern "C" fn sc_apply_user_config_json(handle: *mut c_void, json: *const c_char) -> c_int {
    clear_last_error();
    let Some(mut s) = session_mut(handle) else {
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
