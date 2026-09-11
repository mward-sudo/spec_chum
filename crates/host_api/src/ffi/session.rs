//! Session lifecycle and model selection C ABI.

use std::os::raw::{c_int, c_uint, c_void};
use std::ptr;

use crate::handle::{SessionHandle, SessionInner};
use crate::session::{HostSession, ModelId};

use super::{clear_last_error, session_mut, set_last_error};

/// Promote session for embedded agent HTTP. Returns 0 on success.
#[no_mangle]
pub extern "C" fn sc_share_session(handle: *mut c_void) -> c_int {
    clear_last_error();
    if crate::handle::share_session_arc(handle).is_some() {
        0
    } else {
        set_last_error("null handle");
        -1
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
    Box::into_raw(Box::new(SessionHandle {
        inner: SessionInner::Local(std::cell::RefCell::new(session)),
    }))
    .cast()
}

/// Destroy a session created by [`sc_create`].
#[no_mangle]
pub extern "C" fn sc_destroy(handle: *mut c_void) {
    if handle.is_null() {
        return;
    }
    // SAFETY: handle from `sc_create`; unique ownership.
    drop(unsafe { Box::from_raw(handle.cast::<SessionHandle>()) });
}

#[no_mangle]
pub extern "C" fn sc_set_model(handle: *mut c_void, model: c_uint) -> c_int {
    clear_last_error();
    let Some(mut s) = session_mut(handle) else {
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
    let Some(mut s) = session_mut(handle) else {
        set_last_error("null handle");
        return c_uint::MAX;
    };
    s.model() as c_uint
}

#[no_mangle]
pub extern "C" fn sc_reset(handle: *mut c_void) -> c_int {
    clear_last_error();
    let Some(mut s) = session_mut(handle) else {
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
    if let Some(mut s) = session_mut(handle) {
        s.set_running(running != 0);
    }
}

#[no_mangle]
pub extern "C" fn sc_set_border(handle: *mut c_void, with_border: c_int) {
    if let Some(mut s) = session_mut(handle) {
        s.set_border(with_border != 0);
    }
}

#[no_mangle]
pub extern "C" fn sc_run_frame(handle: *mut c_void) {
    if let Some(mut s) = session_mut(handle) {
        s.run_frame();
    }
}
