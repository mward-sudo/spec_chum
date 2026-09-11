//! Tape open / transport / load-options C ABI.

use std::ffi::CStr;
use std::os::raw::{c_char, c_int, c_uint, c_void};
use std::path::Path;

use super::{clear_last_error, session_mut, set_last_error};

#[no_mangle]
pub extern "C" fn sc_open_tape(handle: *mut c_void, path: *const c_char) -> c_int {
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
    match s.open_tape(Path::new(path)) {
        Ok(()) => 0,
        Err(e) => {
            set_last_error(e.to_string());
            -1
        }
    }
}

/// Enable/disable opt-in `ZXInfo` online tape title lookup (`enabled != 0`). Default off (#373).
#[no_mangle]
pub extern "C" fn sc_set_online_tape_titles(handle: *mut c_void, enabled: c_int) {
    let Some(mut s) = session_mut(handle) else {
        return;
    };
    s.set_online_tape_titles(enabled != 0);
}

/// `1` when online tape title lookup is enabled, else `0`.
#[no_mangle]
pub extern "C" fn sc_online_tape_titles(handle: *mut c_void) -> c_int {
    let Some(s) = session_mut(handle) else {
        return 0;
    };
    i32::from(s.online_tape_titles())
}

#[no_mangle]
pub extern "C" fn sc_tape_play(handle: *mut c_void) -> c_int {
    clear_last_error();
    let Some(mut s) = session_mut(handle) else {
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
    let Some(mut s) = session_mut(handle) else {
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
    let Some(mut s) = session_mut(handle) else {
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

/// Spectrum frames actually run per host tick right now (EAR turbo only
/// applies while an unfinished deck is playing) — hosts show this so chrome
/// cannot claim 1× while the machine runs faster (#390).
#[no_mangle]
pub extern "C" fn sc_effective_speed_multiplier(handle: *mut c_void) -> c_uint {
    session_mut(handle)
        .and_then(|s| {
            s.machine()
                .map(machine::Machine::effective_speed_multiplier)
        })
        .unwrap_or(1)
}

/// 1 when the inserted deck can serve LD-BYTES flash-load traps (TAP blocks).
///
/// Pulse-only TZX decks return 0: Instant on those loads off EAR at
/// [`machine::INSTANT_EAR_FALLBACK_SPEED`], so hosts can say so (#390).
#[no_mangle]
pub extern "C" fn sc_tape_flash_load_supported(handle: *mut c_void) -> c_int {
    session_mut(handle)
        .and_then(|s| s.machine().map(machine::Machine::tape_supports_flash_load))
        .unwrap_or(false) as c_int
}

/// EAR rate Instant falls back to when the deck cannot flash-load, so hosts do
/// not hardcode a second copy of the policy (#390).
#[no_mangle]
pub extern "C" fn sc_instant_ear_fallback_speed() -> c_uint {
    machine::INSTANT_EAR_FALLBACK_SPEED
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
    let Some(mut s) = session_mut(handle) else {
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
    let Some(mut s) = session_mut(handle) else {
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
    let Some(mut s) = session_mut(handle) else {
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
    let Some(mut s) = session_mut(handle) else {
        set_last_error("null handle");
        return -1;
    };
    let flash = flash_load != 0;
    let experience_load = if flash {
        false
    } else {
        s.tape_load_options().is_some_and(|o| o.experience_load)
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
    let Some(mut s) = session_mut(handle) else {
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
