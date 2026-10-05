//! ROM setup dialog data for native hosts (#188).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use machine::{Model, RomSlotStatus};
use serde::Serialize;
use thiserror::Error;

use crate::prefs::{model_rom_path_key, slot_rom_overrides, PrefModel};
use crate::session::ModelId;

#[derive(Debug, Error)]
pub enum RomSetupError {
    #[error(transparent)]
    Rom(#[from] machine::RomReadError),
    #[error(transparent)]
    NextAssets(#[from] crate::next_assets::NextAssetError),
    #[error("Choose the official sn-complete-24.11.zip beside boot-30204.bin, GPL3-LICENSE and ASSET-INFO.txt")]
    WrongNextArchive,
}

#[derive(Clone, Debug, Serialize)]
pub struct RomSetupSlot {
    pub id: String,
    pub label: String,
    pub install_path: String,
    pub alternate_paths: Vec<String>,
    pub expected_bytes: usize,
    pub user_provided: bool,
    pub status: String,
    pub resolved_path: Option<String>,
    pub hint: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct RomSetupJson {
    pub model_title: String,
    pub complete: bool,
    pub fetchable: bool,
    pub slots: Vec<RomSetupSlot>,
}

static MODEL_ROM_PATHS: Mutex<BTreeMap<String, String>> = Mutex::new(BTreeMap::new());

/// Replace the process-global ROM path map (macOS `UserDefaults` mirror for FFI).
pub fn sync_model_rom_paths(paths: BTreeMap<String, String>) {
    if let Ok(mut guard) = MODEL_ROM_PATHS.lock() {
        *guard = paths
            .into_iter()
            .filter(|(_, path)| !path.trim().is_empty())
            .collect();
    }
}

/// Snapshot of the process-global ROM path map.
#[must_use]
pub fn model_rom_paths_snapshot() -> BTreeMap<String, String> {
    MODEL_ROM_PATHS
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default()
}

fn status_str(s: RomSlotStatus) -> &'static str {
    match s {
        RomSlotStatus::Missing => "missing",
        RomSlotStatus::Found => "found",
        RomSlotStatus::WrongSize => "wrong_size",
    }
}

fn slot_hint(model: Model, slot_id: &str, user_provided: bool) -> String {
    if slot_id == "trdos" {
        return "16 KiB TR-DOS image (user-provided; see docs/ROMS.md)".into();
    }
    if slot_id == "exrom" {
        return "8 KiB Timex EX-ROM (tc2068-1.rom; fetch with ./scripts/fetch_roms.sh)".into();
    }
    if user_provided {
        return format!(
            "User-provided dump for {} — see docs/ROMS.md",
            machine::model_title(model)
        );
    }
    "Fetch with ./scripts/fetch_roms.sh or choose a file".into()
}

fn slot_overrides(
    model: ModelId,
    rom_paths: &BTreeMap<String, String>,
) -> BTreeMap<String, PathBuf> {
    slot_rom_overrides(PrefModel::from_model(model.to_model()), rom_paths)
}

fn canonical_persist_path(source: &Path) -> PathBuf {
    source
        .canonicalize()
        .unwrap_or_else(|_| source.to_path_buf())
}

/// Per-slot override paths for one built-in model from the global prefs map.
#[must_use]
pub fn slot_rom_overrides_for_model(model: ModelId) -> BTreeMap<String, PathBuf> {
    slot_overrides(model, &model_rom_paths_snapshot())
}

/// JSON payload for macOS / egui ROM setup dialogs.
#[must_use]
pub fn rom_setup_json(model: ModelId, rom_paths: &BTreeMap<String, String>) -> RomSetupJson {
    if model == ModelId::SpectrumNext {
        let checked = crate::next_assets::NextAssets::discover();
        let hint = checked.as_ref().err().map_or_else(
            || "Official System/Next 24.11 and IPL verified. The local SD card is prepared on first launch.".to_string(),
            ToString::to_string,
        );
        return RomSetupJson {
            model_title: "ZX Spectrum Next".into(),
            complete: checked.is_ok(),
            fetchable: false,
            slots: vec![RomSetupSlot {
                id: "next_assets".into(),
                label: "Official System/Next 24.11 assets".into(),
                install_path: "roms/system-next/24.11".into(),
                alternate_paths: Vec::new(),
                expected_bytes: 56_371_963,
                user_provided: true,
                status: if checked.is_ok() { "found" } else { "missing" }.into(),
                resolved_path: checked.ok().map(|assets| assets.directory.display().to_string()),
                hint: format!("{hint} Use Get official assets, or select a previously verified archive with its IPL and notices. Source and license details: https://github.com/mward-sudo/spec_chum/blob/main/docs/ROMS.md."),
            }],
        };
    }
    let m = model.to_model();
    let roots = machine::search_roots();
    let overrides = slot_overrides(model, rom_paths);
    let states = machine::rom_slot_states_with_overrides(m, &roots, &overrides);
    let complete = machine::rom_available_in_with_overrides(m, &roots, &overrides);
    let slots = states
        .into_iter()
        .map(|s| {
            let d = s.descriptor;
            RomSetupSlot {
                id: d.id.to_string(),
                label: d.label.to_string(),
                install_path: d.install_path.to_string(),
                alternate_paths: d.search_paths.iter().map(|p| (*p).to_string()).collect(),
                expected_bytes: d.expected_bytes,
                user_provided: d.user_provided,
                status: status_str(s.status).to_string(),
                resolved_path: s.resolved_path.map(|p| p.display().to_string()),
                hint: slot_hint(m, d.id, d.user_provided),
            }
        })
        .collect();
    RomSetupJson {
        model_title: machine::model_title(m).to_string(),
        complete,
        fetchable: !machine::requires_user_rom(m),
        slots,
    }
}

/// True when the model's ROM dumps are never auto-fetched (user must supply paths).
#[must_use]
pub fn model_requires_user_rom(model: ModelId) -> bool {
    machine::requires_user_rom(model.to_model())
}

/// True when required ROM slots are present (persisted paths or workspace search).
#[must_use]
pub fn model_rom_available(model: ModelId, rom_paths: &BTreeMap<String, String>) -> bool {
    if model == ModelId::SpectrumNext {
        return crate::next_assets::NextAssets::available_cached();
    }
    let m = model.to_model();
    let roots = machine::search_roots();
    let overrides = slot_overrides(model, rom_paths);
    machine::rom_available_in_with_overrides(m, &roots, &overrides)
}

/// Explicit user action: acquire the pinned official set and remember its local path.
pub fn acquire_next_assets() -> Result<PathBuf, RomSetupError> {
    let assets = crate::next_assets::NextAssets::acquire_official()?;
    if let Ok(mut paths) = MODEL_ROM_PATHS.lock() {
        paths.insert(
            model_rom_path_key(PrefModel::SpectrumNext, "next_assets"),
            assets.archive.display().to_string(),
        );
    }
    Ok(assets.archive)
}

/// Validate `source`, persist its absolute path, and best-effort copy into `roms/`.
pub fn install_model_rom(
    model: ModelId,
    slot_id: &str,
    source: &Path,
    rom_paths: &mut BTreeMap<String, String>,
) -> Result<PathBuf, RomSetupError> {
    if model == ModelId::SpectrumNext {
        if slot_id != "next_assets" {
            return Err(machine::RomReadError::UnknownSlot(slot_id.to_string()).into());
        }
        if source
            .file_name()
            .is_none_or(|name| name != "sn-complete-24.11.zip")
        {
            return Err(RomSetupError::WrongNextArchive);
        }
        let directory = source.parent().ok_or(RomSetupError::WrongNextArchive)?;
        let assets = crate::next_assets::NextAssets::discover_in(directory.to_path_buf())?;
        let key = model_rom_path_key(PrefModel::SpectrumNext, slot_id);
        rom_paths.insert(
            key,
            canonical_persist_path(&assets.archive)
                .display()
                .to_string(),
        );
        sync_model_rom_paths(rom_paths.clone());
        return Ok(assets.archive);
    }
    let descriptor = machine::rom_slot_descriptors(model.to_model())
        .into_iter()
        .find(|d| d.id == slot_id)
        .ok_or_else(|| machine::RomReadError::UnknownSlot(slot_id.to_string()))?;
    let data = std::fs::read(source).map_err(|source_err| machine::RomReadError::Io {
        op: "read",
        path: source.display().to_string(),
        source: source_err,
    })?;
    if data.len() != descriptor.expected_bytes {
        return Err(machine::RomReadError::WrongSize {
            kind: descriptor.label,
            expected: descriptor.expected_bytes,
            got: data.len(),
            path: source.display().to_string(),
        }
        .into());
    }
    let persisted = canonical_persist_path(source);
    let key = model_rom_path_key(PrefModel::from_model(model.to_model()), slot_id);
    rom_paths.insert(key, persisted.display().to_string());
    sync_model_rom_paths(rom_paths.clone());

    let roots = machine::search_roots();
    match machine::install_rom_slot(model.to_model(), slot_id, source, &roots) {
        Ok(dest) => Ok(dest),
        Err(_) => Ok(persisted),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn next_setup_requires_the_complete_official_archive_name() {
        let mut paths = BTreeMap::new();
        let error = install_model_rom(
            ModelId::SpectrumNext,
            "next_assets",
            Path::new("some-firmware.bin"),
            &mut paths,
        )
        .expect_err("arbitrary firmware cannot enable Next");
        assert!(matches!(error, RomSetupError::WrongNextArchive));
        assert!(paths.is_empty());
    }

    #[test]
    fn pentagon_requires_user_rom() {
        assert!(model_requires_user_rom(ModelId::Pentagon128));
        assert!(!model_requires_user_rom(ModelId::Spectrum48));
    }

    #[test]
    fn pentagon_rom_setup_has_user_slots() {
        sync_model_rom_paths(BTreeMap::new());
        let doc = rom_setup_json(ModelId::Pentagon128, &BTreeMap::new());
        assert_eq!(doc.slots.len(), 2);
        assert!(!doc.fetchable);
        assert_eq!(doc.slots[0].id, "main");
        assert_eq!(doc.slots[1].id, "trdos");
        assert_eq!(doc.slots[1].expected_bytes, 16 * 1024);
    }

    #[test]
    fn rom_setup_json_serializes() {
        let doc = rom_setup_json(ModelId::Pentagon128, &BTreeMap::new());
        let text = serde_json::to_string(&doc).expect("json");
        assert!(text.contains("trdos"));
        assert!(text.contains("Pentagon"));
    }

    #[test]
    fn persisted_path_wins_over_missing_workspace() {
        let dir = std::env::temp_dir().join(format!(
            "spec_chum_rom_persist_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("tmpdir");
        let src = dir.join("custom.rom");
        std::fs::write(&src, vec![0xBB; 16 * 1024]).expect("write");

        let mut paths = BTreeMap::new();
        paths.insert(
            model_rom_path_key(PrefModel::Spectrum48, "main"),
            src.display().to_string(),
        );
        let doc = rom_setup_json(ModelId::Spectrum48, &paths);
        assert!(doc.complete);
        assert_eq!(doc.slots[0].status, "found");
        assert_eq!(
            doc.slots[0].resolved_path.as_deref(),
            Some(src.to_str().unwrap())
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
