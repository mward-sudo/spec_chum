//! Shared control / inspect / debug backend for Spec Chum agents and hosts.
//!
//! Wraps [`spec_chum_host::HostSession`] as the single source of truth for debugger
//! operations. HTTP (`agent_server`) and CLI clients call into this crate.

mod error;
mod framebuffer;
mod host_view;
mod present;
mod service;
mod window_capture;

pub use error::{ApiError, ApiResult, ErrorBody};
pub use framebuffer::{encode_framebuffer_png, model_slug, parse_model_slug, FramebufferMeta};
pub use host_view::{HostViewState, HostWindowCapture, SharedHostView};
pub use present::{
    compose_nearest_letterbox, encode_rgba_png, fit_size, PresentMeta, PresentPanelSource,
};
pub use service::{
    ControlPlane, HardwareStatusResponse, LastBreakResponse, LastErrorRecord, MemoryMapResponse,
    MemoryRegion, PagingSnapshot, PrefsPatch, ServerConfig, SessionPrefs, TraceFormat,
};
#[cfg(target_os = "macos")]
pub use window_capture::cg_window_id_from_ns_view;
pub use window_capture::OwnWindowCapturer;
