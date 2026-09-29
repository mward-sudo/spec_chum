//! Shared infrastructure for the native GTK and Win32 shells.

pub mod audio;
pub mod commands;

pub use audio::OutputStream;
pub use commands::*;
