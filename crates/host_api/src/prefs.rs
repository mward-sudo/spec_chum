//! Host-local UI / session preferences (issue #186).
//!
//! Shared JSON schema for egui / living_room. macOS SwiftUI mirrors the same
//! fields in `UserDefaults` (`specChum.*` keys). Instant / flash-load is never
//! persisted as a sticky Play default.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use machine::{AyStereoMode, JoystickMode, Model, TapeLoadOptions};
use serde::{Deserialize, Serialize};

use crate::session::ModelId;

/// Bumped when the on-disk shape changes incompatibly.
pub const PREFS_VERSION: u32 = 1;

/// Cap for the recent-files list.
pub const MAX_RECENT_FILES: usize = 12;

/// Minimum egui / native window size (matches `ViewportBuilder` / SwiftUI mins).
pub const MIN_WINDOW_WIDTH: f32 = 480.0;
pub const MIN_WINDOW_HEIGHT: f32 = 400.0;

const DEFAULT_WINDOW_WIDTH: f32 = 780.0;
const DEFAULT_WINDOW_HEIGHT: f32 = 680.0;

/// Versioned preference record written as `ui-prefs.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UiPreferences {
    #[serde(default = "prefs_version")]
    pub version: u32,
    #[serde(default)]
    pub model: PrefModel,
    /// When true, restore Experience (~20s) EAR mode (not Instant flash-load).
    #[serde(default)]
    pub tape_experience: bool,
    /// EAR speed multiplier when not in Experience mode (`1` / `2` / `5` / `10` / `20`).
    #[serde(default = "default_tape_speed")]
    pub tape_ear_speed: u32,
    #[serde(default = "default_volume")]
    pub volume: f32,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub joystick_mode: PrefJoystick,
    #[serde(default)]
    pub kempston_mouse: bool,
    #[serde(default)]
    pub ay_stereo: PrefAyStereo,
    #[serde(default = "default_window_width")]
    pub window_width: f32,
    #[serde(default = "default_window_height")]
    pub window_height: f32,
    #[serde(default)]
    pub recent_files: Vec<String>,
    #[serde(default = "default_true")]
    pub throttle: bool,
}

fn prefs_version() -> u32 {
    PREFS_VERSION
}
fn default_tape_speed() -> u32 {
    1
}
fn default_volume() -> f32 {
    1.0
}
fn default_window_width() -> f32 {
    DEFAULT_WINDOW_WIDTH
}
fn default_window_height() -> f32 {
    DEFAULT_WINDOW_HEIGHT
}
fn default_true() -> bool {
    true
}

impl Default for UiPreferences {
    fn default() -> Self {
        Self {
            version: PREFS_VERSION,
            model: PrefModel::Spectrum48,
            tape_experience: false,
            tape_ear_speed: 1,
            volume: 1.0,
            muted: false,
            joystick_mode: PrefJoystick::Kempston,
            kempston_mouse: false,
            ay_stereo: PrefAyStereo::Mono,
            window_width: DEFAULT_WINDOW_WIDTH,
            window_height: DEFAULT_WINDOW_HEIGHT,
            recent_files: Vec::new(),
            throttle: true,
        }
    }
}

impl UiPreferences {
    /// Normalize clamps / drop bad recent paths / force schema version.
    #[must_use]
    pub fn sanitized(mut self) -> Self {
        self.version = PREFS_VERSION;
        if self.tape_experience {
            self.tape_ear_speed = tape::EXPERIENCE_EAR_SPEED;
        } else {
            // Keep only the speeds the UI offers; unknown → 1x.
            const SPEEDS: &[u32] = &[1, 2, 5, 10, 20];
            if !SPEEDS.contains(&self.tape_ear_speed) {
                self.tape_ear_speed = 1;
            }
        }
        self.volume = self.volume.clamp(0.0, 1.0);
        self.window_width = self.window_width.max(MIN_WINDOW_WIDTH);
        self.window_height = self.window_height.max(MIN_WINDOW_HEIGHT);
        self.recent_files.retain(|p| !p.trim().is_empty());
        self.recent_files.truncate(MAX_RECENT_FILES);
        self
    }

    #[must_use]
    pub fn tape_load_options(&self) -> TapeLoadOptions {
        if self.tape_experience {
            TapeLoadOptions::experience()
        } else {
            TapeLoadOptions::default().with_speed(self.tape_ear_speed)
        }
    }

    /// Record a successfully opened media path (most-recent first).
    pub fn push_recent(&mut self, path: &Path) {
        let Some(s) = path.to_str().map(str::to_owned) else {
            return;
        };
        self.recent_files.retain(|p| p != &s);
        self.recent_files.insert(0, s);
        self.recent_files.truncate(MAX_RECENT_FILES);
    }

    pub fn set_model_from_machine(&mut self, model: Model) {
        self.model = PrefModel::from_model(model);
    }

    pub fn set_model_from_id(&mut self, model: ModelId) {
        self.model = PrefModel::from_model_id(model);
    }

    pub fn set_joystick(&mut self, mode: JoystickMode) {
        self.joystick_mode = PrefJoystick::from_mode(mode);
    }

    pub fn set_ay_stereo(&mut self, mode: AyStereoMode) {
        self.ay_stereo = PrefAyStereo::from_mode(mode);
    }

    pub fn set_tape_from_options(&mut self, opts: TapeLoadOptions) {
        // Never persist Instant flash-load as sticky Play.
        self.tape_experience = opts.experience_load;
        if opts.experience_load {
            self.tape_ear_speed = tape::EXPERIENCE_EAR_SPEED;
        } else {
            self.tape_ear_speed = opts.speed.clamp(1, 64);
            const SPEEDS: &[u32] = &[1, 2, 5, 10, 20];
            if !SPEEDS.contains(&self.tape_ear_speed) {
                // Snap to nearest offered speed.
                self.tape_ear_speed = SPEEDS
                    .iter()
                    .copied()
                    .min_by_key(|s| s.abs_diff(opts.speed))
                    .unwrap_or(1);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrefModel {
    #[default]
    Spectrum48,
    Spectrum128,
    SpectrumPlus2A,
    SpectrumPlus3,
}

impl PrefModel {
    #[must_use]
    pub fn from_model(m: Model) -> Self {
        match m {
            Model::Spectrum48 => Self::Spectrum48,
            Model::Spectrum128 => Self::Spectrum128,
            Model::SpectrumPlus2A => Self::SpectrumPlus2A,
            Model::SpectrumPlus3 => Self::SpectrumPlus3,
        }
    }

    #[must_use]
    pub fn from_model_id(m: ModelId) -> Self {
        Self::from_model(m.to_model())
    }

    #[must_use]
    pub fn to_model(self) -> Model {
        match self {
            Self::Spectrum48 => Model::Spectrum48,
            Self::Spectrum128 => Model::Spectrum128,
            Self::SpectrumPlus2A => Model::SpectrumPlus2A,
            Self::SpectrumPlus3 => Model::SpectrumPlus3,
        }
    }

    #[must_use]
    pub fn to_model_id(self) -> ModelId {
        match self {
            Self::Spectrum48 => ModelId::Spectrum48,
            Self::Spectrum128 => ModelId::Spectrum128,
            Self::SpectrumPlus2A => ModelId::SpectrumPlus2A,
            Self::SpectrumPlus3 => ModelId::SpectrumPlus3,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrefJoystick {
    #[default]
    Kempston,
    SinclairLeft,
    SinclairRight,
    Cursor,
}

impl PrefJoystick {
    #[must_use]
    pub fn from_mode(m: JoystickMode) -> Self {
        match m {
            JoystickMode::Kempston => Self::Kempston,
            JoystickMode::SinclairLeft => Self::SinclairLeft,
            JoystickMode::SinclairRight => Self::SinclairRight,
            JoystickMode::Cursor => Self::Cursor,
        }
    }

    #[must_use]
    pub fn to_mode(self) -> JoystickMode {
        match self {
            Self::Kempston => JoystickMode::Kempston,
            Self::SinclairLeft => JoystickMode::SinclairLeft,
            Self::SinclairRight => JoystickMode::SinclairRight,
            Self::Cursor => JoystickMode::Cursor,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrefAyStereo {
    #[default]
    Mono,
    Acb,
    Abc,
}

impl PrefAyStereo {
    #[must_use]
    pub fn from_mode(m: AyStereoMode) -> Self {
        match m {
            AyStereoMode::Mono => Self::Mono,
            AyStereoMode::Acb => Self::Acb,
            AyStereoMode::Abc => Self::Abc,
        }
    }

    #[must_use]
    pub fn to_mode(self) -> AyStereoMode {
        match self {
            Self::Mono => AyStereoMode::Mono,
            Self::Acb => AyStereoMode::Acb,
            Self::Abc => AyStereoMode::Abc,
        }
    }
}

/// Default on-disk path (`…/spec-chum/ui-prefs.json`), or `SPEC_CHUM_PREFS_PATH`.
#[must_use]
pub fn default_prefs_path() -> PathBuf {
    if let Ok(p) = std::env::var("SPEC_CHUM_PREFS_PATH") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    directories::ProjectDirs::from("dev", "SpecChum", "spec-chum")
        .map(|d| d.config_dir().join("ui-prefs.json"))
        .unwrap_or_else(|| PathBuf::from("ui-prefs.json"))
}

/// Load preferences; missing or corrupt files yield [`UiPreferences::default`].
#[must_use]
pub fn load_prefs(path: &Path) -> UiPreferences {
    let Ok(bytes) = fs::read(path) else {
        return UiPreferences::default();
    };
    match serde_json::from_slice::<UiPreferences>(&bytes) {
        Ok(p) => p.sanitized(),
        Err(_) => UiPreferences::default(),
    }
}

/// Atomically write preferences (temp file + rename). Errors are ignored by callers
/// that only need best-effort persistence; tests assert the `Result`.
pub fn save_prefs(path: &Path, prefs: &UiPreferences) -> std::io::Result<()> {
    let _guard = PrefsLock::acquire(path)?;
    save_prefs_locked(path, prefs)
}

/// Read-modify-write under the same process-wide lock as [`save_prefs`].
pub fn update_prefs(path: &Path, update: impl FnOnce(&mut UiPreferences)) -> std::io::Result<()> {
    let _guard = PrefsLock::acquire(path)?;
    let mut prefs = load_prefs_unlocked(path);
    update(&mut prefs);
    save_prefs_locked(path, &prefs)
}

struct PrefsLock {
    path: PathBuf,
}

impl PrefsLock {
    fn acquire(path: &Path) -> std::io::Result<Self> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        let lock_path = path.with_extension("json.lock");
        for attempt in 0..50 {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)
            {
                Ok(_) => return Ok(Self { path: lock_path }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    thread::sleep(Duration::from_millis(5 * (attempt + 1)));
                }
                Err(e) => return Err(e),
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "prefs lock timeout",
        ))
    }
}

impl Drop for PrefsLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn load_prefs_unlocked(path: &Path) -> UiPreferences {
    let Ok(bytes) = fs::read(path) else {
        return UiPreferences::default();
    };
    match serde_json::from_slice::<UiPreferences>(&bytes) {
        Ok(p) => p.sanitized(),
        Err(_) => UiPreferences::default(),
    }
}

fn save_prefs_locked(path: &Path, prefs: &UiPreferences) -> std::io::Result<()> {
    let prefs = prefs.clone().sanitized();
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("json.tmp.{}", std::process::id()));
    let json = serde_json::to_vec_pretty(&prefs)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(&json)?;
        f.sync_all()?;
    }
    match fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_prefs_path(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!("spec-chum-prefs-{label}-{nanos}.json"))
    }

    #[test]
    fn round_trip_preserves_fields() {
        let path = temp_prefs_path("round");
        let mut p = UiPreferences {
            model: PrefModel::Spectrum128,
            tape_experience: true,
            volume: 0.4,
            muted: true,
            joystick_mode: PrefJoystick::Cursor,
            kempston_mouse: true,
            ay_stereo: PrefAyStereo::Acb,
            window_width: 900.0,
            window_height: 700.0,
            throttle: false,
            ..UiPreferences::default()
        };
        p.push_recent(Path::new("/tmp/game.tap"));
        save_prefs(&path, &p).expect("save");
        let loaded = load_prefs(&path);
        let _ = fs::remove_file(&path);
        assert_eq!(loaded.model, PrefModel::Spectrum128);
        assert!(loaded.tape_experience);
        assert!((loaded.volume - 0.4).abs() < f32::EPSILON);
        assert!(loaded.muted);
        assert_eq!(loaded.joystick_mode, PrefJoystick::Cursor);
        assert!(loaded.kempston_mouse);
        assert_eq!(loaded.ay_stereo, PrefAyStereo::Acb);
        assert!(!loaded.throttle);
        assert_eq!(loaded.recent_files, vec!["/tmp/game.tap".to_string()]);
        assert!(loaded.tape_load_options().experience_load);
    }

    #[test]
    fn corrupt_file_falls_back_to_defaults() {
        let path = temp_prefs_path("bad");
        fs::write(&path, b"{not json").expect("write");
        let loaded = load_prefs(&path);
        let _ = fs::remove_file(&path);
        assert_eq!(loaded, UiPreferences::default());
    }

    #[test]
    fn missing_file_falls_back_to_defaults() {
        let path = temp_prefs_path("missing");
        let _ = fs::remove_file(&path);
        assert_eq!(load_prefs(&path), UiPreferences::default());
    }

    #[test]
    fn window_size_clamped_to_minimum() {
        let p = UiPreferences {
            window_width: 10.0,
            window_height: 10.0,
            ..UiPreferences::default()
        };
        let s = p.sanitized();
        assert_eq!(s.window_width, MIN_WINDOW_WIDTH);
        assert_eq!(s.window_height, MIN_WINDOW_HEIGHT);
    }

    #[test]
    fn tape_options_never_sticky_flash() {
        let mut p = UiPreferences::default();
        p.set_tape_from_options(TapeLoadOptions {
            flash_load: true,
            speed: 1,
            experience_load: false,
        });
        let opts = p.tape_load_options();
        assert!(!opts.flash_load);
        assert!(!opts.experience_load);
        assert_eq!(opts.speed, 1);
    }

    #[test]
    fn recent_files_most_recent_first_deduped() {
        let mut p = UiPreferences::default();
        p.push_recent(Path::new("/a.tap"));
        p.push_recent(Path::new("/b.tzx"));
        p.push_recent(Path::new("/a.tap"));
        assert_eq!(
            p.recent_files,
            vec!["/a.tap".to_string(), "/b.tzx".to_string()]
        );
    }
}
