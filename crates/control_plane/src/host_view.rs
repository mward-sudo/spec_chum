//! Optional host-attached view state for agent screenshots (#239).

use std::sync::Arc;

use parking_lot::Mutex;

use crate::error::{ApiError, ApiResult};

/// Captures **this process’s** OS window as PNG without changing focus or z-order.
///
/// Implementations must target a registered window id owned by this PID only.
pub trait HostWindowCapture: Send + Sync {
    fn capture_window_png(&self) -> ApiResult<Vec<u8>>;
}

/// Live panel size + optional window capturer registered by egui / SpecChumMac.
#[derive(Default)]
pub struct HostViewState {
    /// Last egui central-panel size in points (integer).
    pub panel_w: Option<u32>,
    pub panel_h: Option<u32>,
    window: Option<Arc<dyn HostWindowCapture>>,
}

impl std::fmt::Debug for HostViewState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostViewState")
            .field("panel_w", &self.panel_w)
            .field("panel_h", &self.panel_h)
            .field("window_capture", &self.window.is_some())
            .finish()
    }
}

impl HostViewState {
    pub fn set_panel_size(&mut self, w: u32, h: u32) {
        if w > 0 && h > 0 {
            self.panel_w = Some(w);
            self.panel_h = Some(h);
        }
    }

    pub fn set_window_capture(&mut self, capture: Option<Arc<dyn HostWindowCapture>>) {
        self.window = capture;
    }

    pub fn capture_window_png(&self) -> ApiResult<Vec<u8>> {
        match &self.window {
            Some(c) => c.capture_window_png(),
            None => Err(ApiError::Unavailable(
                "host window capture unavailable (no GUI window registered; standalone agent has no OS window)"
                    .into(),
            )),
        }
    }
}

/// Shared host-view slot on [`crate::ControlPlane`].
pub type SharedHostView = Arc<Mutex<HostViewState>>;

pub fn new_shared_host_view() -> SharedHostView {
    Arc::new(Mutex::new(HostViewState::default()))
}
