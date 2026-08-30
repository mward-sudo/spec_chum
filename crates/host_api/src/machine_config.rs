//! User-defined machine configurations (#187).
//!
//! Serializable profiles: base model + optional hardware + ROM override.
//! Apply logic is shared by egui and tests; macOS parity is follow-up #169.

use std::path::{Path, PathBuf};

use machine::{AyStereoMode, JoystickMode, Machine, Model};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::prefs::{PrefAyStereo, PrefJoystick, PrefModel};

/// Errors from validating or applying a [`UserMachineConfig`].
#[derive(Debug, Error)]
pub enum MachineConfigError {
    #[error("configuration name is required")]
    NameRequired,
    #[error("ROM for {model} must be {expected} bytes, got {actual}")]
    RomSize {
        model: String,
        expected: usize,
        actual: usize,
    },
    #[error("read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("Missing ROM for {model} — {reason}")]
    MissingRom { model: String, reason: String },
    #[error("{peripheral} ROM not found: {path}")]
    PeripheralRomMissing {
        peripheral: &'static str,
        path: String,
    },
    #[error("{peripheral}: {message}")]
    Peripheral {
        peripheral: &'static str,
        message: String,
    },
    #[error("machine: {0}")]
    Machine(String),
}

/// Maximum saved custom profiles.
pub const MAX_CUSTOM_CONFIGS: usize = 32;

/// Expected main-ROM byte length for a base model.
#[must_use]
pub fn expected_rom_bytes(model: PrefModel) -> usize {
    match model {
        PrefModel::Spectrum16K | PrefModel::Spectrum48 => 16 * 1024,
        PrefModel::Spectrum128 | PrefModel::SpectrumPlus2 | PrefModel::Pentagon128 => 32 * 1024,
        PrefModel::SpectrumPlus2A | PrefModel::SpectrumPlus3 => 64 * 1024,
    }
}

/// Validate ROM size for `model` before booting.
pub fn validate_main_rom(data: &[u8], model: PrefModel) -> Result<(), MachineConfigError> {
    let expected = expected_rom_bytes(model);
    if data.len() == expected {
        Ok(())
    } else {
        Err(MachineConfigError::RomSize {
            model: machine::model_title(model.to_model()).to_string(),
            expected,
            actual: data.len(),
        })
    }
}

/// Which optional hardware toggles are valid for a base model.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HardwareCompat {
    pub multiface: bool,
    pub divmmc: bool,
    pub interface1: bool,
    pub beta: bool,
    pub ay_stereo: bool,
    pub kempston_mouse: bool,
    pub joystick: bool,
}

#[must_use]
pub fn hardware_compat(model: PrefModel) -> HardwareCompat {
    let m = model.to_model();
    HardwareCompat {
        multiface: matches!(m, Model::Spectrum16K | Model::Spectrum48),
        divmmc: matches!(
            m,
            Model::Spectrum16K
                | Model::Spectrum48
                | Model::Spectrum128
                | Model::SpectrumPlus2
                | Model::Pentagon128
        ),
        interface1: matches!(
            m,
            Model::Spectrum16K
                | Model::Spectrum48
                | Model::Spectrum128
                | Model::SpectrumPlus2
                | Model::Pentagon128
        ),
        beta: matches!(
            m,
            Model::Spectrum16K
                | Model::Spectrum48
                | Model::Spectrum128
                | Model::SpectrumPlus2
                | Model::Pentagon128
        ),
        ay_stereo: matches!(
            m,
            Model::Spectrum128
                | Model::SpectrumPlus2
                | Model::SpectrumPlus2A
                | Model::SpectrumPlus3
                | Model::Pentagon128
        ),
        kempston_mouse: true,
        joystick: true,
    }
}

/// A named user profile persisted in [`crate::prefs::UiPreferences`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UserMachineConfig {
    pub id: String,
    pub name: String,
    pub base: PrefModel,
    /// When set, load this file instead of the default autoload ROM for `base`.
    #[serde(default)]
    pub custom_rom_path: Option<String>,
    #[serde(default)]
    pub joystick_mode: PrefJoystick,
    #[serde(default)]
    pub kempston_mouse: bool,
    #[serde(default)]
    pub ay_stereo: PrefAyStereo,
    #[serde(default)]
    pub attach_multiface: bool,
    #[serde(default)]
    pub multiface_rom_path: Option<String>,
    #[serde(default)]
    pub attach_divmmc: bool,
    #[serde(default)]
    pub divmmc_eeprom_path: Option<String>,
    #[serde(default)]
    pub attach_interface1: bool,
    #[serde(default)]
    pub interface1_rom_path: Option<String>,
    #[serde(default)]
    pub attach_beta: bool,
    #[serde(default)]
    pub trdos_rom_path: Option<String>,
}

impl UserMachineConfig {
    #[must_use]
    pub fn new_named(name: impl Into<String>, base: PrefModel) -> Self {
        Self {
            id: new_config_id(),
            name: name.into(),
            base,
            custom_rom_path: None,
            joystick_mode: PrefJoystick::Kempston,
            kempston_mouse: false,
            ay_stereo: PrefAyStereo::Mono,
            attach_multiface: false,
            multiface_rom_path: None,
            attach_divmmc: false,
            divmmc_eeprom_path: None,
            attach_interface1: false,
            interface1_rom_path: None,
            attach_beta: false,
            trdos_rom_path: None,
        }
    }

    /// Drop incompatible hardware flags and empty path strings.
    #[must_use]
    pub fn sanitized(mut self) -> Self {
        self.name = self.name.trim().to_string();
        if self.name.is_empty() {
            self.name = "Untitled".into();
        }
        self.id = self.id.trim().to_string();
        if self.id.is_empty() {
            self.id = new_config_id();
        }
        trim_opt(&mut self.custom_rom_path);
        trim_opt(&mut self.multiface_rom_path);
        trim_opt(&mut self.divmmc_eeprom_path);
        trim_opt(&mut self.interface1_rom_path);
        trim_opt(&mut self.trdos_rom_path);

        let compat = hardware_compat(self.base);
        if !compat.multiface {
            self.attach_multiface = false;
            self.multiface_rom_path = None;
        }
        if !compat.divmmc {
            self.attach_divmmc = false;
            self.divmmc_eeprom_path = None;
        }
        if !compat.interface1 {
            self.attach_interface1 = false;
            self.interface1_rom_path = None;
        }
        if !compat.beta {
            self.attach_beta = false;
            self.trdos_rom_path = None;
        }
        if !compat.ay_stereo {
            self.ay_stereo = PrefAyStereo::Mono;
        }
        if !self.attach_multiface {
            self.multiface_rom_path = None;
        }
        if !self.attach_divmmc {
            self.divmmc_eeprom_path = None;
        }
        if !self.attach_interface1 {
            self.interface1_rom_path = None;
        }
        if !self.attach_beta {
            self.trdos_rom_path = None;
        }
        self
    }

    /// Validate name, compatibility, and ROM paths before save/apply.
    pub fn validate(&self) -> Result<(), MachineConfigError> {
        let c = self.clone().sanitized();
        if c.name.is_empty() {
            return Err(MachineConfigError::NameRequired);
        }
        if let Some(path) = &c.custom_rom_path {
            validate_rom_file(path, c.base)?;
        }
        if c.attach_multiface {
            if let Some(path) = &c.multiface_rom_path {
                if !Path::new(path).is_file() {
                    return Err(MachineConfigError::PeripheralRomMissing {
                        peripheral: "Multiface",
                        path: path.clone(),
                    });
                }
            }
        }
        if c.attach_beta {
            if let Some(path) = &c.trdos_rom_path {
                if !Path::new(path).is_file() {
                    return Err(MachineConfigError::PeripheralRomMissing {
                        peripheral: "TR-DOS",
                        path: path.clone(),
                    });
                }
            }
        }
        Ok(())
    }
}

fn trim_opt(s: &mut Option<String>) {
    if let Some(v) = s {
        let t = v.trim().to_string();
        if t.is_empty() {
            *s = None;
        } else {
            *v = t;
        }
    }
}

#[must_use]
pub fn new_config_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("cfg-{nanos}")
}

fn validate_rom_file(path: &str, model: PrefModel) -> Result<(), MachineConfigError> {
    let data = std::fs::read(path).map_err(|e| MachineConfigError::Io {
        path: path.to_string(),
        source: e,
    })?;
    validate_main_rom(&data, model)
}

/// Resolve main ROM bytes: custom path or default autoload for `base`.
pub fn resolve_main_rom(
    config: &UserMachineConfig,
    roots: &[PathBuf],
) -> Result<(Vec<u8>, String), MachineConfigError> {
    let config = config.clone().sanitized();
    if let Some(custom) = &config.custom_rom_path {
        let data = std::fs::read(custom).map_err(|e| MachineConfigError::Io {
            path: custom.clone(),
            source: e,
        })?;
        validate_main_rom(&data, config.base)?;
        return Ok((data, custom.clone()));
    }
    let model = config.base.to_model();
    let path = machine::resolve_rom_path_in(model, roots).ok_or_else(|| {
        MachineConfigError::MissingRom {
            model: machine::model_title(model).to_string(),
            reason: machine::unavailable_reason(model).to_string(),
        }
    })?;
    let path_display = path.display().to_string();
    let data = std::fs::read(&path).map_err(|e| MachineConfigError::Io {
        path: path_display.clone(),
        source: e,
    })?;
    validate_main_rom(&data, config.base)?;
    Ok((data, path_display))
}

fn build_machine(model: Model, rom: &[u8]) -> Result<Machine, MachineConfigError> {
    match model {
        Model::Spectrum16K => Machine::new_16k(rom),
        Model::Spectrum48 => Machine::new_48k(rom),
        Model::Spectrum128 => Machine::new_128k(rom),
        Model::SpectrumPlus2 => Machine::new_plus2(rom),
        Model::SpectrumPlus2A => Machine::new_plus2a(rom),
        Model::SpectrumPlus3 => Machine::new_plus3(rom),
        Model::Pentagon128 => {
            let trdos = machine::read_trdos_rom(Model::Pentagon128)
                .map_err(|e| MachineConfigError::Machine(format!("TR-DOS ROM: {e}")))?;
            Machine::new_pentagon128(rom, &trdos)
        }
    }
    .map_err(MachineConfigError::Machine)
}

/// Build a [`Machine`] from a user config and attach enabled peripherals.
///
/// Peripherals that need a ROM/image stay off when the file is missing (status notes).
pub fn apply_user_config(
    config: &UserMachineConfig,
    roots: &[PathBuf],
) -> Result<AppliedConfig, MachineConfigError> {
    let config = config.clone().sanitized();
    config.validate()?;
    let model = config.base.to_model();
    let (rom, rom_label) = resolve_main_rom(&config, roots)?;
    let mut machine = build_machine(model, &rom)?;
    let mut notes = Vec::new();

    if config.attach_multiface && hardware_compat(config.base).multiface {
        match &config.multiface_rom_path {
            Some(path) if Path::new(path).is_file() => {
                let data = std::fs::read(path).map_err(|e| MachineConfigError::Io {
                    path: path.clone(),
                    source: e,
                })?;
                machine
                    .attach_multiface(&data)
                    .map_err(|e| MachineConfigError::Peripheral {
                        peripheral: "Multiface",
                        message: e,
                    })?;
            }
            _ => notes.push("Multiface enabled but no ROM — not attached"),
        }
    }

    if config.attach_divmmc && hardware_compat(config.base).divmmc {
        machine
            .attach_divmmc()
            .map_err(|e| MachineConfigError::Peripheral {
                peripheral: "DivMMC",
                message: e,
            })?;
        if let Some(path) = &config.divmmc_eeprom_path {
            if Path::new(path).is_file() {
                let data = std::fs::read(path).map_err(|e| MachineConfigError::Io {
                    path: path.clone(),
                    source: e,
                })?;
                machine.attach_divmmc_eeprom(&data).map_err(|e| {
                    MachineConfigError::Peripheral {
                        peripheral: "DivMMC EEPROM",
                        message: e,
                    }
                })?;
            } else {
                notes.push("DivMMC EEPROM path missing — stub only");
            }
        }
    }

    if config.attach_interface1 && hardware_compat(config.base).interface1 {
        let if1 = machine
            .attach_interface1()
            .map_err(|e| MachineConfigError::Peripheral {
                peripheral: "Interface 1",
                message: e.to_string(),
            })?;
        let mut loaded = if1.rom_loaded;
        if !loaded {
            if let Some(path) = &config.interface1_rom_path {
                if Path::new(path).is_file() {
                    let data = std::fs::read(path).map_err(|e| MachineConfigError::Io {
                        path: path.clone(),
                        source: e,
                    })?;
                    if1.load_rom(&data)
                        .map_err(|e| MachineConfigError::Peripheral {
                            peripheral: "Interface 1",
                            message: e.to_string(),
                        })?;
                    loaded = true;
                }
            }
            if !loaded {
                for root in roots {
                    for cand in ["roms/if1.rom", "roms/if1-2.rom", "roms/interface1.rom"] {
                        let path = root.join(cand);
                        if let Ok(data) = std::fs::read(&path) {
                            if if1.load_rom(&data).is_ok() {
                                loaded = true;
                                break;
                            }
                        }
                    }
                    if loaded {
                        break;
                    }
                }
            }
        }
        if !loaded {
            notes.push("Interface 1 attached (no ROM loaded)");
        }
    }

    if config.attach_beta && hardware_compat(config.base).beta {
        machine
            .attach_beta()
            .map_err(|e| MachineConfigError::Peripheral {
                peripheral: "Beta",
                message: e,
            })?;
        if let Some(path) = &config.trdos_rom_path {
            if Path::new(path).is_file() {
                let data = std::fs::read(path).map_err(|e| MachineConfigError::Io {
                    path: path.clone(),
                    source: e,
                })?;
                machine
                    .load_trdos_rom(&data)
                    .map_err(|e| MachineConfigError::Peripheral {
                        peripheral: "TR-DOS",
                        message: e,
                    })?;
            } else {
                notes.push("Beta attached but TR-DOS ROM missing");
            }
        } else {
            notes.push("Beta attached (no TR-DOS ROM)");
        }
    }

    if hardware_compat(config.base).ay_stereo {
        machine.set_ay_stereo_mode(config.ay_stereo.to_mode());
    }

    let mut status = format!("Config “{}” — loaded {rom_label}", config.name);
    if !notes.is_empty() {
        status.push_str("; ");
        status.push_str(&notes.join("; "));
    }

    Ok(AppliedConfig {
        machine,
        model,
        joystick_mode: config.joystick_mode.to_mode(),
        kempston_mouse: config.kempston_mouse,
        ay_stereo: config.ay_stereo.to_mode(),
        status,
    })
}

/// Result of [`apply_user_config`].
#[derive(Debug)]
pub struct AppliedConfig {
    pub machine: Machine,
    pub model: Model,
    pub joystick_mode: JoystickMode,
    pub kempston_mouse: bool,
    pub ay_stereo: AyStereoMode,
    pub status: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rom_size_matrix() {
        assert_eq!(expected_rom_bytes(PrefModel::Spectrum48), 16384);
        assert_eq!(expected_rom_bytes(PrefModel::Spectrum128), 32768);
        assert_eq!(expected_rom_bytes(PrefModel::SpectrumPlus3), 65536);
    }

    #[test]
    fn validate_main_rom_rejects_wrong_size() {
        assert!(validate_main_rom(&[0u8; 100], PrefModel::Spectrum48).is_err());
        assert!(validate_main_rom(&[0u8; 16384], PrefModel::Spectrum48).is_ok());
    }

    #[test]
    fn plus3_strips_divmmc_and_multiface() {
        let cfg = UserMachineConfig {
            attach_multiface: true,
            attach_divmmc: true,
            attach_interface1: true,
            attach_beta: true,
            ..UserMachineConfig::new_named("Plus3", PrefModel::SpectrumPlus3)
        }
        .sanitized();
        assert!(!cfg.attach_multiface);
        assert!(!cfg.attach_divmmc);
        assert!(!cfg.attach_interface1);
        assert!(!cfg.attach_beta);
    }

    #[test]
    fn forty8k_keeps_divmmc_if1_beta() {
        let compat = hardware_compat(PrefModel::Spectrum48);
        assert!(compat.multiface);
        assert!(compat.divmmc);
        assert!(compat.interface1);
        assert!(compat.beta);
        assert!(!compat.ay_stereo);
    }

    #[test]
    fn apply_builtin_rom_when_no_override() {
        let roots = machine::search_roots();
        if !machine::rom_available_in(Model::Spectrum48, &roots) {
            return;
        }
        let cfg = UserMachineConfig::new_named("Plain 48K", PrefModel::Spectrum48);
        let applied = apply_user_config(&cfg, &roots).expect("apply");
        assert_eq!(applied.model, Model::Spectrum48);
        assert!(!applied.machine.has_divmmc());
    }

    #[test]
    fn apply_rejects_bad_custom_rom_size() {
        let roots = machine::search_roots();
        let dir = std::env::temp_dir();
        let bad = dir.join(format!("spec-chum-bad-rom-{}", std::process::id()));
        std::fs::write(&bad, vec![0u8; 100]).expect("write");
        let cfg = UserMachineConfig {
            custom_rom_path: Some(bad.to_string_lossy().into_owned()),
            ..UserMachineConfig::new_named("Bad", PrefModel::Spectrum48)
        };
        let err = apply_user_config(&cfg, &roots).expect_err("bad size");
        assert!(
            matches!(
                err,
                MachineConfigError::RomSize {
                    expected: 16384,
                    ..
                }
            ),
            "{err}"
        );
        let _ = std::fs::remove_file(&bad);
    }
}
