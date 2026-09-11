//! Trace ring + debugger C ABI.

use std::ffi::CStr;
use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_uint, c_void};
use std::ptr;

use super::{clear_last_error, heap_cstring, session_mut, set_last_error};

/// Trace / debug observability (#210 Phase C): thin C ABI over the global `trace`
/// ring and `HostSession` debug helpers — same sources as `control_plane` HTTP routes.
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
    CString::new(s).map_or(ptr::null_mut(), CString::into_raw)
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
    let Some(mut s) = session_mut(handle) else {
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
    let Some(mut s) = session_mut(handle) else {
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
    let Some(mut s) = session_mut(handle) else {
        set_last_error("null handle");
        return ptr::null_mut();
    };
    match s.inspect_json() {
        Ok(json) => heap_cstring(&json),
        Err(e) => {
            set_last_error(e.to_string());
            ptr::null_mut()
        }
    }
}

/// Fill `pc,sp,af,bc,de,hl,ix,iy`. Null out-params are skipped. Returns 0 on success.
#[no_mangle]
// Flat out-params match the C header; packing would break ABI (#171).
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
    let Some(mut s) = session_mut(handle) else {
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
    let Some(mut s) = session_mut(handle) else {
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
    if let Some(mut s) = session_mut(handle) {
        s.set_paused(paused != 0);
    }
}

/// Add a PC breakpoint. Returns 0 on success, -1 on error.
#[no_mangle]
pub extern "C" fn sc_add_breakpoint(handle: *mut c_void, pc: c_uint) -> c_int {
    clear_last_error();
    let Some(mut s) = session_mut(handle) else {
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
    let Some(mut s) = session_mut(handle) else {
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
    heap_cstring(&trace::dump_json())
}
