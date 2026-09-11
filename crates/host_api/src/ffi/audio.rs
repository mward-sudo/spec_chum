//! Host audio PCM snapshot C ABI.

use std::os::raw::{c_uint, c_void};
use std::ptr;

use super::{session_mut, AUDIO_SNAPSHOT};

/// Pointer to mono f32 PCM snapshot from the last `sc_run_frame` (valid until next call).
#[no_mangle]
pub extern "C" fn sc_audio_ptr(handle: *mut c_void) -> *const f32 {
    let Some(s) = session_mut(handle) else {
        return ptr::null();
    };
    let pcm = s.audio_pcm();
    AUDIO_SNAPSHOT.with(|cell| {
        let mut buf = cell.borrow_mut();
        buf.clear();
        buf.extend_from_slice(pcm);
        buf.as_ptr()
    })
}

/// Number of mono samples in [`sc_audio_ptr`].
#[no_mangle]
pub extern "C" fn sc_audio_frames(handle: *mut c_void) -> c_uint {
    session_mut(handle).map_or(0, |s| s.audio_pcm().len() as c_uint)
}

/// Host audio sample rate (Hz).
#[no_mangle]
pub extern "C" fn sc_audio_sample_rate(_handle: *mut c_void) -> c_uint {
    crate::session::AUDIO_SAMPLE_RATE
}
