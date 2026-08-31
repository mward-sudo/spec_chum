//! Shared control / inspect / debug backend for Spec Chum agents and hosts.
//!
//! Wraps [`spec_chum_host::HostSession`] as the single source of truth for debugger
//! operations. HTTP (`agent_server`) and CLI clients call into this crate.

#![allow(clippy::pedantic)]

mod error;
mod framebuffer;
mod service;

pub use error::{ApiError, ApiResult, ErrorBody};
pub use framebuffer::{encode_framebuffer_png, model_slug, parse_model_slug, FramebufferMeta};
pub use service::{
    ControlPlane, LastBreakResponse, LastErrorRecord, PrefsPatch, ServerConfig, SessionPrefs,
    TraceFormat,
};
