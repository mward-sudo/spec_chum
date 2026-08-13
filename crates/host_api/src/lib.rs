//! Thin host API for native shells (SwiftUI) and future cores.
//!
//! Safe Rust surface lives in [`session`]. The C ABI in [`ffi`] is for FFI only
//! and requires a narrowly scoped `unsafe` allow.

#![allow(clippy::pedantic)]

pub mod ffi;
pub mod session;

pub use session::{HostError, HostSession, ModelId};
