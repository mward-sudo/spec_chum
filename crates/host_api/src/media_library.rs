//! Shared, local-only policy for browsing and opening recent user media.
//!
//! This module reads file names and existence only. It does not parse, copy, or
//! index user media; opening remains an explicit [`HostSession`] action.

use std::collections::HashSet;
use std::path::Path;

use machine::Machine;
use serde::Serialize;

use crate::machine_config::hardware_compat;
use crate::prefs::PrefModel;
use crate::session::{HostError, HostSession, ModelId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaCategory {
    Tape,
    Snapshot,
    Recording,
    Disk,
}

impl MediaCategory {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "tape" => Some(Self::Tape),
            "snapshot" => Some(Self::Snapshot),
            "recording" => Some(Self::Recording),
            "disk" => Some(Self::Disk),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaFormat {
    Tap,
    Tzx,
    Sna,
    Z80,
    Rzx,
    Dsk,
    Trd,
}

impl MediaFormat {
    pub fn from_path(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?;
        if extension.eq_ignore_ascii_case("tap") {
            Some(Self::Tap)
        } else if extension.eq_ignore_ascii_case("tzx") {
            Some(Self::Tzx)
        } else if extension.eq_ignore_ascii_case("sna") {
            Some(Self::Sna)
        } else if extension.eq_ignore_ascii_case("z80") {
            Some(Self::Z80)
        } else if extension.eq_ignore_ascii_case("rzx") {
            Some(Self::Rzx)
        } else if extension.eq_ignore_ascii_case("dsk") {
            Some(Self::Dsk)
        } else if extension.eq_ignore_ascii_case("trd") {
            Some(Self::Trd)
        } else {
            None
        }
    }

    pub fn category(self) -> MediaCategory {
        match self {
            Self::Tap | Self::Tzx => MediaCategory::Tape,
            Self::Sna | Self::Z80 => MediaCategory::Snapshot,
            Self::Rzx => MediaCategory::Recording,
            Self::Dsk | Self::Trd => MediaCategory::Disk,
        }
    }
}

/// A display hint based on known machine state, not a claim that file parsing will succeed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaCompatibility {
    Ready,
    /// RZX may carry its own snapshot, so opening can still succeed without a machine.
    MayRequireMachine,
    RequiresMachine,
    RequiresPlus3,
    RequiresBetaModel,
    RequiresBeta,
    RequiresTrdosRom,
    UnsupportedOnNext,
}

impl MediaCompatibility {
    pub fn can_open(self) -> bool {
        matches!(self, Self::Ready | Self::MayRequireMachine)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MediaEntry {
    /// Original path string from recent-file storage. Never normalized or rewritten.
    pub path: String,
    pub name: String,
    pub format: MediaFormat,
    pub category: MediaCategory,
    pub available: bool,
    pub compatibility: MediaCompatibility,
}

fn trdos_rom_loaded(session: &HostSession) -> bool {
    match session.machine() {
        Some(Machine::Spec48 { bus, .. }) => bus.beta.as_ref().is_some_and(|beta| beta.rom_loaded),
        Some(Machine::Spec128 { bus, .. }) => bus.beta.as_ref().is_some_and(|beta| beta.rom_loaded),
        _ => false,
    }
}

fn compatibility(format: MediaFormat, session: &HostSession) -> MediaCompatibility {
    match format {
        MediaFormat::Rzx if session.machine().is_none() => MediaCompatibility::MayRequireMachine,
        MediaFormat::Tap | MediaFormat::Tzx | MediaFormat::Trd
            if session.model() == ModelId::SpectrumNext =>
        {
            MediaCompatibility::UnsupportedOnNext
        }
        MediaFormat::Dsk if !session.model().to_model().has_plus3_disk() => {
            MediaCompatibility::RequiresPlus3
        }
        MediaFormat::Trd
            if !hardware_compat(PrefModel::from_model(session.model().to_model())).beta =>
        {
            MediaCompatibility::RequiresBetaModel
        }
        MediaFormat::Tap | MediaFormat::Tzx | MediaFormat::Dsk | MediaFormat::Trd
            if session.machine().is_none() =>
        {
            MediaCompatibility::RequiresMachine
        }
        MediaFormat::Trd if !session.has_beta() => MediaCompatibility::RequiresBeta,
        MediaFormat::Trd if !trdos_rom_loaded(session) => MediaCompatibility::RequiresTrdosRom,
        MediaFormat::Sna
        | MediaFormat::Z80
        | MediaFormat::Rzx
        | MediaFormat::Tap
        | MediaFormat::Tzx
        | MediaFormat::Dsk
        | MediaFormat::Trd => MediaCompatibility::Ready,
    }
}

/// Return supported recent items in stored order, with optional name/path search and category.
/// Duplicate paths are collapsed; unsupported formats are omitted. This never opens media.
pub fn query_recent_media(
    recent_paths: &[String],
    search: &str,
    category: Option<MediaCategory>,
    session: &HostSession,
) -> Vec<MediaEntry> {
    let search = search.trim().to_lowercase();
    let mut seen = HashSet::new();
    recent_paths
        .iter()
        .filter_map(|original| {
            let path = Path::new(original);
            let format = MediaFormat::from_path(path)?;
            if category.is_some_and(|selected| selected != format.category())
                || !seen.insert(original.as_str())
            {
                return None;
            }
            let name = path.file_name()?.to_string_lossy().into_owned();
            if !search.is_empty()
                && !name.to_lowercase().contains(&search)
                && !original.to_lowercase().contains(&search)
            {
                return None;
            }
            Some(MediaEntry {
                path: original.clone(),
                name,
                format,
                category: format.category(),
                available: path.is_file(),
                compatibility: compatibility(format, session),
            })
        })
        .collect()
}

impl HostSession {
    /// Open a supported media path through the current machine and peripheral policy.
    /// The caller records success in its own recent-file store; this never edits that store.
    pub fn open_media_path(&mut self, path: &Path) -> Result<(), HostError> {
        let format = MediaFormat::from_path(path)
            .ok_or_else(|| HostError::Message("unsupported media format".into()))?;
        if !path.is_file() {
            return Err(HostError::Message(format!(
                "media file is unavailable: {}",
                path.display()
            )));
        }
        let status = compatibility(format, self);
        if !status.can_open() {
            return Err(HostError::Message(format!(
                "cannot open {}: {}",
                path.display(),
                match status {
                    MediaCompatibility::RequiresMachine => "load a machine first",
                    MediaCompatibility::RequiresPlus3 => "select a +3 or +3e first",
                    MediaCompatibility::RequiresBetaModel => "select a Beta-compatible model first",
                    MediaCompatibility::RequiresBeta => "attach Beta Disk first",
                    MediaCompatibility::RequiresTrdosRom => "load a TR-DOS ROM first",
                    MediaCompatibility::UnsupportedOnNext => "unsupported on ZX Spectrum Next",
                    MediaCompatibility::Ready | MediaCompatibility::MayRequireMachine => {
                        unreachable!("can_open checked above")
                    }
                }
            )));
        }
        match format {
            MediaFormat::Tap | MediaFormat::Tzx => self.open_tape(path),
            MediaFormat::Sna | MediaFormat::Z80 => self.load_snapshot(path),
            MediaFormat::Rzx => self.load_rzx(path),
            MediaFormat::Dsk => self.load_dsk(path),
            MediaFormat::Trd => self.load_trd(path),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_support_is_exact_and_case_insensitive() {
        for (extension, expected) in [
            ("TAP", MediaFormat::Tap),
            ("tzx", MediaFormat::Tzx),
            ("Sna", MediaFormat::Sna),
            ("Z80", MediaFormat::Z80),
            ("RZX", MediaFormat::Rzx),
            ("dsk", MediaFormat::Dsk),
            ("trd", MediaFormat::Trd),
        ] {
            assert_eq!(
                MediaFormat::from_path(Path::new(&format!("game.{extension}"))),
                Some(expected)
            );
        }
        assert_eq!(MediaFormat::from_path(Path::new("game.szx")), None);
    }

    #[test]
    fn query_searches_and_preserves_original_recent_paths() {
        let session = HostSession::new(ModelId::Spectrum48, false);
        let paths = vec![
            "/Missing/Foo.TAP".into(),
            "/Missing/Foo.TAP".into(),
            "/Missing/Other.Z80".into(),
            "/Missing/Unknown.SZX".into(),
        ];
        let entries = query_recent_media(&paths, "foo", Some(MediaCategory::Tape), &session);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "/Missing/Foo.TAP");
        assert_eq!(entries[0].name, "Foo.TAP");
        assert!(!entries[0].available);
        assert_eq!(
            entries[0].compatibility,
            MediaCompatibility::RequiresMachine
        );
    }

    #[test]
    fn disk_hints_follow_current_model_and_peripherals() {
        let plus3 = HostSession::new(ModelId::SpectrumPlus3, false);
        let spec48 = HostSession::new(ModelId::Spectrum48, false);
        let disk = ["/Missing/game.dsk".into(), "/Missing/game.trd".into()];
        let plus3_entries = query_recent_media(&disk, "", None, &plus3);
        assert_eq!(
            plus3_entries[0].compatibility,
            MediaCompatibility::RequiresMachine
        );
        assert_eq!(
            plus3_entries[1].compatibility,
            MediaCompatibility::RequiresBetaModel
        );
        let spec48_entries = query_recent_media(&disk, "", None, &spec48);
        assert_eq!(
            spec48_entries[0].compatibility,
            MediaCompatibility::RequiresPlus3
        );
        assert_eq!(
            spec48_entries[1].compatibility,
            MediaCompatibility::RequiresMachine
        );
    }

    #[test]
    fn trd_requires_attached_beta_and_loaded_trdos_rom() {
        let mut session = HostSession::new(ModelId::Spectrum48, false);
        session
            .load_rom_bytes(&[0; 16 * 1024])
            .expect("synthetic main ROM should load");
        let recent = ["/Missing/game.trd".into()];
        let status =
            |session: &HostSession| query_recent_media(&recent, "", None, session)[0].compatibility;
        assert_eq!(status(&session), MediaCompatibility::RequiresBeta);
        session.attach_beta().expect("Beta should attach on 48K");
        assert_eq!(status(&session), MediaCompatibility::RequiresTrdosRom);
        let beta = session
            .machine_mut()
            .and_then(Machine::beta_mut)
            .expect("attached Beta");
        beta.load_rom(&[0; 16 * 1024])
            .expect("synthetic TR-DOS ROM should load");
        assert_eq!(status(&session), MediaCompatibility::Ready);
    }

    #[test]
    fn unsupported_open_does_not_change_machine_state() {
        let mut session = HostSession::new(ModelId::Spectrum48, false);
        let before = session.status().to_owned();
        let error = session.open_media_path(Path::new("/Missing/game.szx"));
        assert!(error.is_err());
        assert_eq!(session.status(), before);
    }

    #[test]
    fn opening_supported_tape_uses_existing_session_loader() {
        let mut session = HostSession::new(ModelId::Spectrum48, false);
        session
            .load_rom_bytes(&[0; 16 * 1024])
            .expect("synthetic main ROM should load");
        let path =
            std::env::temp_dir().join(format!("spec_chum_media_open_{}.tap", std::process::id()));
        std::fs::write(&path, [1, 0, 0]).expect("write TAP fixture");
        session.open_media_path(&path).expect("open supported TAP");
        assert!(session.status().contains("Inserted TAP"));
        std::fs::remove_file(&path).expect("remove TAP fixture");
    }
}
