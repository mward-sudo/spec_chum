//! Embedded loopback agent HTTP on the live SpecChumMac session (#210 / #221).
//!
//! Host view parity with egui (#239): registers [`OwnWindowCapturer`] and accepts
//! panel-size / window-id updates from Swift without changing OS focus or z-order.

use std::collections::HashMap;
use std::os::raw::{c_int, c_void};
use std::sync::{Arc, LazyLock, Mutex};

use agent_server::embedded::{spawn, EmbeddedServer};
use control_plane::{ControlPlane, HostWindowCapture, OwnWindowCapturer, ServerConfig};
use spec_chum_host::handle::share_session_arc;

struct EmbedState {
    _server: EmbeddedServer,
    plane: Arc<ControlPlane>,
    capturer: Arc<OwnWindowCapturer>,
}

static EMBEDDED: LazyLock<Mutex<HashMap<usize, EmbedState>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn session_key(handle: *mut c_void) -> Option<usize> {
    let shared = share_session_arc(handle)?;
    Some(Arc::as_ptr(&shared) as usize)
}

/// Start loopback agent HTTP on the live `sc_*` session (same machine as the GUI).
///
/// Reads `SPEC_CHUM_AGENT_TOKEN`, `SPEC_CHUM_AGENT_INSECURE`, and `SPEC_CHUM_AGENT_PORT`
/// from the environment (same as egui `SPEC_CHUM_AGENT=1`). Idempotent per session.
#[no_mangle]
#[allow(unsafe_code)] // C ABI export; callers honor handle lifetime.
pub extern "C" fn sc_agent_embed_start(handle: *mut c_void) -> c_int {
    let Some(shared) = share_session_arc(handle) else {
        return -1;
    };
    let key = Arc::as_ptr(&shared) as usize;
    let Ok(guard) = EMBEDDED.lock() else {
        return -1;
    };
    if guard.contains_key(&key) {
        return 0;
    }
    drop(guard);
    let plane = Arc::new(ControlPlane::from_shared(shared));
    let capturer = OwnWindowCapturer::new();
    plane.set_window_capture(Some(Arc::clone(&capturer) as Arc<dyn HostWindowCapture>));
    let config = ServerConfig::from_env();
    match spawn(config, Arc::clone(&plane)) {
        Ok(server) => {
            let Ok(mut g) = EMBEDDED.lock() else {
                return -1;
            };
            g.insert(
                key,
                EmbedState {
                    _server: server,
                    plane,
                    capturer,
                },
            );
            eprintln!(
                "spec-chum: SpecChumMac agent embedded on http://127.0.0.1:{}",
                std::env::var("SPEC_CHUM_AGENT_PORT").unwrap_or_else(|_| "17384".into())
            );
            0
        }
        Err(e) => {
            eprintln!("spec-chum: SpecChumMac agent embed failed: {e}");
            -1
        }
    }
}

/// Stop embedded agent HTTP for this handle (no-op when not started).
#[no_mangle]
#[allow(unsafe_code)] // C ABI export; callers honor handle lifetime.
pub extern "C" fn sc_agent_embed_stop(handle: *mut c_void) -> c_int {
    let Some(key) = session_key(handle) else {
        return -1;
    };
    let Ok(mut g) = EMBEDDED.lock() else {
        return -1;
    };
    g.remove(&key);
    0
}

/// Publish OS `windowNumber` / `CGWindowID` for `/v1/host/window` (no focus change).
#[no_mangle]
#[allow(unsafe_code)] // C ABI export.
pub extern "C" fn sc_agent_set_host_window_id(handle: *mut c_void, window_id: u32) -> c_int {
    let Some(key) = session_key(handle) else {
        return -1;
    };
    let Ok(g) = EMBEDDED.lock() else {
        return -1;
    };
    let Some(state) = g.get(&key) else {
        return -1;
    };
    state.capturer.set_window_id(window_id);
    0
}

/// Publish central display panel size in points for `/v1/host/display` live sizing.
#[no_mangle]
#[allow(unsafe_code)] // C ABI export.
pub extern "C" fn sc_agent_set_display_panel_size(
    handle: *mut c_void,
    width: u32,
    height: u32,
) -> c_int {
    let Some(key) = session_key(handle) else {
        return -1;
    };
    let Ok(g) = EMBEDDED.lock() else {
        return -1;
    };
    let Some(state) = g.get(&key) else {
        return -1;
    };
    state.plane.set_display_panel_size(width, height);
    0
}

#[cfg(test)]
mod tests {
    #![allow(unsafe_code)] // test-owned handle drop + env restore

    use std::cell::RefCell;

    use spec_chum_host::handle::{SessionHandle, SessionInner};
    use spec_chum_host::{HostSession, ModelId};

    use super::*;

    #[test]
    fn embed_start_rejects_null() {
        assert_eq!(sc_agent_embed_start(std::ptr::null_mut()), -1);
    }

    #[test]
    fn embed_stop_rejects_null() {
        assert_eq!(sc_agent_embed_stop(std::ptr::null_mut()), -1);
    }

    #[test]
    fn set_host_view_rejects_null() {
        assert_eq!(sc_agent_set_host_window_id(std::ptr::null_mut(), 1), -1);
        assert_eq!(
            sc_agent_set_display_panel_size(std::ptr::null_mut(), 100, 100),
            -1
        );
    }

    #[test]
    fn embed_start_skips_without_auth_config() {
        let saved_insecure = std::env::var("SPEC_CHUM_AGENT_INSECURE").ok();
        let saved_token = std::env::var("SPEC_CHUM_AGENT_TOKEN").ok();
        // SAFETY: single-threaded unit test; no concurrent env readers.
        unsafe {
            std::env::remove_var("SPEC_CHUM_AGENT_INSECURE");
            std::env::remove_var("SPEC_CHUM_AGENT_TOKEN");
        }
        let session = HostSession::new(ModelId::Spectrum48, true);
        let handle = Box::into_raw(Box::new(SessionHandle {
            inner: SessionInner::Local(RefCell::new(session)),
        }));
        let rc = sc_agent_embed_start(handle.cast());
        sc_agent_embed_stop(handle.cast());
        // SAFETY: test-owned handle.
        drop(unsafe { Box::from_raw(handle) });
        // SAFETY: restore prior process env for other tests.
        unsafe {
            match saved_insecure {
                Some(v) => std::env::set_var("SPEC_CHUM_AGENT_INSECURE", v),
                None => std::env::remove_var("SPEC_CHUM_AGENT_INSECURE"),
            }
            match saved_token {
                Some(v) => std::env::set_var("SPEC_CHUM_AGENT_TOKEN", v),
                None => std::env::remove_var("SPEC_CHUM_AGENT_TOKEN"),
            }
        }
        assert_eq!(rc, -1, "missing token/insecure should fail closed");
    }
}
