//! Keyboard / joystick / mouse C ABI.

use std::os::raw::{c_int, c_uint, c_void};

use super::{clear_last_error, session_mut, set_last_error};

#[no_mangle]
pub extern "C" fn sc_set_key(
    handle: *mut c_void,
    row: c_uint,
    bit: c_uint,
    pressed: c_int,
) -> c_int {
    clear_last_error();
    let Some(mut s) = session_mut(handle) else {
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
    let Some(mut s) = session_mut(handle) else {
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
    let Some(mut s) = session_mut(handle) else {
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
    s.set_joystick_mode(mode);
    0
}

#[no_mangle]
pub extern "C" fn sc_set_joystick(handle: *mut c_void, mask: c_uint) -> c_int {
    clear_last_error();
    let Some(mut s) = session_mut(handle) else {
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
    let Some(mut s) = session_mut(handle) else {
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
    let Some(mut s) = session_mut(handle) else {
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
    let Some(mut s) = session_mut(handle) else {
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
