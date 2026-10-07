//! Host audio PCM snapshot C ABI.

use std::os::raw::{c_double, c_uint, c_void};
use std::ptr;

use super::{session_mut, AUDIO_SNAPSHOT, AUDIO_STEREO_SNAPSHOT};

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

/// Pointer to interleaved stereo f32 PCM snapshot from the last `sc_run_frame`.
/// Samples are ordered left, right and remain valid until the next call.
#[no_mangle]
pub extern "C" fn sc_audio_stereo_ptr(handle: *mut c_void) -> *const f32 {
    let Some(s) = session_mut(handle) else {
        return ptr::null();
    };
    AUDIO_STEREO_SNAPSHOT.with(|cell| {
        let mut buf = cell.borrow_mut();
        buf.clear();
        buf.extend_from_slice(s.audio_pcm_stereo());
        buf.as_ptr()
    })
}

/// Number of stereo sample frames (each frame contains left and right samples).
#[no_mangle]
pub extern "C" fn sc_audio_stereo_frames(handle: *mut c_void) -> c_uint {
    session_mut(handle).map_or(0, |s| (s.audio_pcm_stereo().len() / 2) as c_uint)
}

/// Host pacing interval in seconds (classic 20 ms; Next follows selected timing).
#[no_mangle]
pub extern "C" fn sc_frame_period_seconds(handle: *mut c_void) -> c_double {
    session_mut(handle).map_or(0.020, |s| s.frame_period_seconds())
}
