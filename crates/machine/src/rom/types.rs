//! ROM error and host-picker slot types.

use std::path::PathBuf;

use thiserror::Error;

/// Filesystem ROM resolve / read / install failures (`#171` Pillar B).
#[derive(Debug, Error)]
pub enum RomReadError {
    #[error("ROM for {model} not found; {hint}")]
    NotFound { model: String, hint: String },
    #[error("TR-DOS ROM for {model} not found; {hint}")]
    TrdosNotFound { model: String, hint: String },
    #[error("EX-ROM for {model} not found; {hint}")]
    ExromNotFound { model: String, hint: String },
    #[error("{model} does not use a TR-DOS ROM")]
    TrdosNotApplicable { model: String },
    #[error("{model} does not use an EX-ROM")]
    ExromNotApplicable { model: String },
    #[error("unknown ROM slot “{0}”")]
    UnknownSlot(String),
    #[error("{kind} must be {expected} bytes, got {got} ({path})")]
    WrongSize {
        kind: &'static str,
        expected: usize,
        got: usize,
        path: String,
    },
    #[error("{op} {path}: {source}")]
    Io {
        op: &'static str,
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// Which ROM image a setup slot refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RomSlotKind {
    Main,
    Trdos,
    /// Timex TS2068 / TC2068 EX-ROM (8 KiB).
    ExRom,
}

/// Host picker / dialog status for one ROM slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RomSlotStatus {
    Missing,
    Found,
    WrongSize,
}

/// Static catalog entry for a model's required ROM slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RomSlotDescriptor {
    pub kind: RomSlotKind,
    pub id: &'static str,
    pub label: &'static str,
    /// Preferred relative path under a workspace root (copy / symlink target).
    pub install_path: &'static str,
    pub search_paths: &'static [&'static str],
    pub expected_bytes: usize,
    /// Never fetched by `./scripts/fetch_roms.sh` (user dump / clone firmware).
    pub user_provided: bool,
}

/// Resolved slot state for UI (status + optional on-disk path).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RomSlotState {
    pub descriptor: RomSlotDescriptor,
    pub status: RomSlotStatus,
    pub resolved_path: Option<PathBuf>,
}
