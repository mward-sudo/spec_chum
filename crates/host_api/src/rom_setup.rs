//! ROM setup dialog data for native hosts (#188).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use machine::{Model, RomSlotStatus};
use serde::Serialize;

use crate::prefs::{model_rom_path_key, pref_model_slug, PrefModel};
use crate::session::ModelId;

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

/// Replace the process-global ROM path map (macOS UserDefaults mirror for FFI).
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
    let pref = PrefModel::from_model(model.to_model());
    let prefix = format!("{}_", pref_model_slug(pref));
    rom_paths
        .iter()
        .filter_map(|(key, path)| {
            key.strip_prefix(&prefix)
                .map(|slot| (slot.to_string(), PathBuf::from(path)))
        })
        .collect()
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
    let m = model.to_model();
    let roots = machine::search_roots();
    let overrides = slot_overrides(model, rom_paths);
    machine::rom_available_in_with_overrides(m, &roots, &overrides)
}

/// Validate `source`, persist its absolute path, and best-effort copy into `roms/`.
pub fn install_model_rom(
    model: ModelId,
    slot_id: &str,
    source: &Path,
    rom_paths: &mut BTreeMap<String, String>,
) -> Result<PathBuf, String> {
    let descriptor = machine::rom_slot_descriptors(model.to_model())
        .into_iter()
        .find(|d| d.id == slot_id)
        .ok_or_else(|| format!("unknown ROM slot “{slot_id}”"))?;
    let data = std::fs::read(source).map_err(|e| format!("read {}: {e}", source.display()))?;
    if data.len() != descriptor.expected_bytes {
        return Err(format!(
            "{} must be {} bytes, got {} ({})",
            descriptor.label,
            descriptor.expected_bytes,
            data.len(),
            source.display()
        ));
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
                .map(|d| d.as_nanos())
                .unwrap_or(0)
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
