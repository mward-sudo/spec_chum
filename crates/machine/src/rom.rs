//! ROM path catalog and availability for host pickers (#188).
//!
//! Phase A models use the UK primary paths under `roms/` (see `rom_candidates`).
//! Phase B (Pentagon 128) requires user-provided main + TR-DOS ROMs — never fetched.
//! Additional distributable images fetched by `./scripts/fetch_roms.sh` — Timex,
//! OpenSE, +3e, Datel, SpeccyBoot, regional alternates — are documented in
//! `docs/ROMS.md` (#190); wire `rom_candidates` when those models ship.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use bus::TRDOS_ROM_SIZE;

use crate::Model;

/// Which ROM image a setup slot refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RomSlotKind {
    Main,
    Trdos,
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

/// Relative main-ROM search paths for a model (first hit wins).
#[must_use]
pub fn rom_candidates(model: Model) -> &'static [&'static str] {
    match model {
        Model::Spectrum16K | Model::Spectrum48 => &["roms/spec48.rom"],
        Model::Spectrum128 => &["roms/128/spec128uk.rom"],
        Model::SpectrumPlus2 => &["roms/plus2/plus2uk.rom"],
        Model::SpectrumPlus2A => &["roms/plus2a/plus2a.rom", "roms/plus3/plus3.rom"],
        Model::SpectrumPlus3 => &["roms/plus3/plus3.rom"],
        Model::Pentagon128 => &["roms/pentagon/pentagon.rom", "roms/pentagon/128p.rom"],
    }
}

/// Relative TR-DOS ROM search paths (Pentagon only; first hit wins).
#[must_use]
pub fn trdos_rom_candidates(_model: Model) -> &'static [&'static str] {
    &[
        "roms/pentagon/trdos.rom",
        "roms/trdos/trdos.rom",
        "roms/trdos.rom",
    ]
}

/// Expected main-ROM byte length for `model`.
#[must_use]
pub fn expected_main_rom_bytes(model: Model) -> usize {
    match model {
        Model::Spectrum16K | Model::Spectrum48 => 16 * 1024,
        Model::Spectrum128 | Model::SpectrumPlus2 | Model::Pentagon128 => 32 * 1024,
        Model::SpectrumPlus2A | Model::SpectrumPlus3 => 64 * 1024,
    }
}

/// Models whose main ROM is never auto-fetched (user dumps / clone firmware).
#[must_use]
pub fn requires_user_rom(model: Model) -> bool {
    matches!(model, Model::Pentagon128)
}

/// True when the model needs a separate TR-DOS ROM on disk before boot.
#[must_use]
pub fn requires_trdos_rom(model: Model) -> bool {
    matches!(model, Model::Pentagon128)
}

/// Short picker label.
#[must_use]
pub fn model_label(model: Model) -> &'static str {
    match model {
        Model::Spectrum16K => "16K",
        Model::Spectrum48 => "48K",
        Model::Spectrum128 => "128K",
        Model::SpectrumPlus2 => "+2",
        Model::SpectrumPlus2A => "+2A",
        Model::SpectrumPlus3 => "+3",
        Model::Pentagon128 => "Pentagon",
    }
}

/// Long picker / menu label.
#[must_use]
pub fn model_title(model: Model) -> &'static str {
    match model {
        Model::Spectrum16K => "Spectrum 16K",
        Model::Spectrum48 => "Spectrum 48K",
        Model::Spectrum128 => "Spectrum 128K",
        Model::SpectrumPlus2 => "Spectrum +2 (grey)",
        Model::SpectrumPlus2A => "Spectrum +2A",
        Model::SpectrumPlus3 => "Spectrum +3",
        Model::Pentagon128 => "Pentagon 128",
    }
}

/// Canonical UI order for every host picker / menu (16K → … → +3 → Pentagon).
pub const ALL_MODELS: [Model; 7] = [
    Model::Spectrum16K,
    Model::Spectrum48,
    Model::Spectrum128,
    Model::SpectrumPlus2,
    Model::SpectrumPlus2A,
    Model::SpectrumPlus3,
    Model::Pentagon128,
];

/// Workspace / env / cwd roots tried when autoloading ROMs.
#[must_use]
pub fn search_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }
    if let Ok(env) = std::env::var("SPEC_CHUM_ROOT") {
        roots.push(PathBuf::from(env));
    }
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    roots
}

fn resolve_first_in(roots: &[PathBuf], rel_paths: &[&str]) -> Option<PathBuf> {
    for root in roots {
        for rel in rel_paths {
            let path = root.join(rel);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

/// Classify an on-disk ROM file against the expected slot size.
#[must_use]
pub fn rom_path_status(path: &Path, expected_bytes: usize) -> RomSlotStatus {
    if !path.is_file() {
        return RomSlotStatus::Missing;
    }
    match fs::metadata(path).map(|m| m.len() as usize) {
        Ok(len) if len == expected_bytes => RomSlotStatus::Found,
        Ok(_) => RomSlotStatus::WrongSize,
        Err(_) => RomSlotStatus::Missing,
    }
}

/// True when any candidate main ROM exists under `search_roots()`.
#[must_use]
pub fn main_rom_available(model: Model) -> bool {
    main_rom_available_in(model, &search_roots())
}

#[must_use]
pub fn main_rom_available_in(model: Model, roots: &[PathBuf]) -> bool {
    resolve_first_in(roots, rom_candidates(model)).is_some()
}

/// True when a TR-DOS ROM exists for models that require one.
#[must_use]
pub fn trdos_rom_available(model: Model) -> bool {
    trdos_rom_available_in(model, &search_roots())
}

#[must_use]
pub fn trdos_rom_available_in(model: Model, roots: &[PathBuf]) -> bool {
    if !requires_trdos_rom(model) {
        return true;
    }
    resolve_first_in(roots, trdos_rom_candidates(model)).is_some()
}

/// True when the model can boot (main ROM + any required TR-DOS ROM present).
#[must_use]
pub fn rom_available(model: Model) -> bool {
    rom_available_in(model, &search_roots())
}

#[must_use]
pub fn rom_available_in(model: Model, roots: &[PathBuf]) -> bool {
    rom_available_in_with_overrides(model, roots, &BTreeMap::new())
}

/// First resolved main ROM path, if any.
#[must_use]
pub fn resolve_rom_path(model: Model) -> Option<PathBuf> {
    resolve_rom_path_in(model, &search_roots())
}

#[must_use]
pub fn resolve_rom_path_in(model: Model, roots: &[PathBuf]) -> Option<PathBuf> {
    resolve_first_in(roots, rom_candidates(model))
}

/// First resolved TR-DOS ROM path for clone models, if any.
#[must_use]
pub fn resolve_trdos_rom_path(model: Model) -> Option<PathBuf> {
    resolve_trdos_rom_path_in(model, &search_roots())
}

#[must_use]
pub fn resolve_trdos_rom_path_in(model: Model, roots: &[PathBuf]) -> Option<PathBuf> {
    if !requires_trdos_rom(model) {
        return None;
    }
    resolve_first_in(roots, trdos_rom_candidates(model))
}

/// Hint shown when a model is disabled in the picker.
#[must_use]
pub fn unavailable_reason(model: Model) -> &'static str {
    if requires_trdos_rom(model) {
        if !main_rom_available(model) {
            return "Add roms/pentagon/pentagon.rom (user-provided; see Help → ROMs)";
        }
        if !trdos_rom_available(model) {
            return "Add roms/pentagon/trdos.rom (16 KiB TR-DOS; user-provided)";
        }
    }
    if requires_user_rom(model) {
        return "Supply a ROM for this model (see Help → ROMs)";
    }
    let paths = rom_candidates(model);
    if paths.len() == 1 {
        return "Add roms/… or run ./scripts/fetch_roms.sh";
    }
    "Add a ROM under roms/ or run ./scripts/fetch_roms.sh"
}

/// Load main ROM bytes for `model` when available.
pub fn read_rom_with_overrides(
    model: Model,
    overrides: &BTreeMap<String, PathBuf>,
) -> Result<Vec<u8>, String> {
    let roots = search_roots();
    let path = resolve_rom_path_in_with_overrides(model, &roots, overrides).ok_or_else(|| {
        format!(
            "ROM for {} not found; {}",
            model_title(model),
            unavailable_reason(model)
        )
    })?;
    std::fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))
}

/// Load main ROM bytes for `model` when available.
pub fn read_rom(model: Model) -> Result<Vec<u8>, String> {
    read_rom_with_overrides(model, &BTreeMap::new())
}

/// Catalog slots required to boot `model` (main, then TR-DOS when applicable).
#[must_use]
pub fn rom_slot_descriptors(model: Model) -> Vec<RomSlotDescriptor> {
    let main = RomSlotDescriptor {
        kind: RomSlotKind::Main,
        id: "main",
        label: "Main ROM",
        install_path: rom_candidates(model)[0],
        search_paths: rom_candidates(model),
        expected_bytes: expected_main_rom_bytes(model),
        user_provided: requires_user_rom(model),
    };
    let mut slots = vec![main];
    if requires_trdos_rom(model) {
        slots.push(RomSlotDescriptor {
            kind: RomSlotKind::Trdos,
            id: "trdos",
            label: "TR-DOS ROM",
            install_path: trdos_rom_candidates(model)[0],
            search_paths: trdos_rom_candidates(model),
            expected_bytes: TRDOS_ROM_SIZE,
            user_provided: true,
        });
    }
    slots
}

/// Resolve on-disk status for one slot under `roots`, honoring a persisted override first.
#[must_use]
pub fn rom_slot_state_with_override(
    descriptor: RomSlotDescriptor,
    roots: &[PathBuf],
    override_path: Option<&Path>,
) -> RomSlotState {
    if let Some(path) = override_path {
        let status = rom_path_status(path, descriptor.expected_bytes);
        return RomSlotState {
            descriptor,
            status,
            resolved_path: Some(path.to_path_buf()),
        };
    }
    if let Some(path) = resolve_first_in(roots, descriptor.search_paths) {
        let status = rom_path_status(&path, descriptor.expected_bytes);
        return RomSlotState {
            descriptor,
            status,
            resolved_path: Some(path),
        };
    }
    RomSlotState {
        descriptor,
        status: RomSlotStatus::Missing,
        resolved_path: None,
    }
}

/// Resolve on-disk status for one slot under `roots`.
#[must_use]
pub fn rom_slot_state(
    _model: Model,
    descriptor: RomSlotDescriptor,
    roots: &[PathBuf],
) -> RomSlotState {
    rom_slot_state_with_override(descriptor, roots, None)
}

/// All slot states for `model`, with optional per-slot persisted paths (`main`, `trdos`, …).
#[must_use]
pub fn rom_slot_states_with_overrides(
    model: Model,
    roots: &[PathBuf],
    overrides: &BTreeMap<String, PathBuf>,
) -> Vec<RomSlotState> {
    rom_slot_descriptors(model)
        .into_iter()
        .map(|d| {
            let ov = overrides.get(d.id).map(PathBuf::as_path);
            rom_slot_state_with_override(d, roots, ov)
        })
        .collect()
}

/// All slot states for `model`.
#[must_use]
pub fn rom_slot_states(model: Model, roots: &[PathBuf]) -> Vec<RomSlotState> {
    rom_slot_states_with_overrides(model, roots, &BTreeMap::new())
}

#[must_use]
pub fn rom_available_in_with_overrides(
    model: Model,
    roots: &[PathBuf],
    overrides: &BTreeMap<String, PathBuf>,
) -> bool {
    rom_slot_states_with_overrides(model, roots, overrides)
        .iter()
        .all(|s| s.status == RomSlotStatus::Found)
}

/// First resolved main ROM path, if any (persisted override wins over workspace search).
#[must_use]
pub fn resolve_rom_path_in_with_overrides(
    model: Model,
    roots: &[PathBuf],
    overrides: &BTreeMap<String, PathBuf>,
) -> Option<PathBuf> {
    let state = rom_slot_state_with_override(
        rom_slot_descriptors(model)
            .into_iter()
            .find(|d| d.kind == RomSlotKind::Main)
            .expect("main slot"),
        roots,
        overrides.get("main").map(PathBuf::as_path),
    );
    (state.status == RomSlotStatus::Found).then(|| state.resolved_path.expect("found path"))
}

#[must_use]
pub fn resolve_trdos_rom_path_in_with_overrides(
    model: Model,
    roots: &[PathBuf],
    overrides: &BTreeMap<String, PathBuf>,
) -> Option<PathBuf> {
    if !requires_trdos_rom(model) {
        return None;
    }
    let state = rom_slot_state_with_override(
        rom_slot_descriptors(model)
            .into_iter()
            .find(|d| d.kind == RomSlotKind::Trdos)
            .expect("trdos slot"),
        roots,
        overrides.get("trdos").map(PathBuf::as_path),
    );
    (state.status == RomSlotStatus::Found).then(|| state.resolved_path.expect("found path"))
}

/// Pick the first workspace root suitable for installing ROM files.
#[must_use]
pub fn writable_install_root() -> PathBuf {
    for root in search_roots() {
        let roms = root.join("roms");
        if roms.is_dir() {
            return root;
        }
        if fs::create_dir_all(&roms).is_ok() {
            return root;
        }
    }
    search_roots()
        .into_iter()
        .next()
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Copy `source` into the slot's canonical `install_path` under a workspace root.
pub fn install_rom_slot(
    model: Model,
    slot_id: &str,
    source: &Path,
    roots: &[PathBuf],
) -> Result<PathBuf, String> {
    let descriptor = rom_slot_descriptors(model)
        .into_iter()
        .find(|d| d.id == slot_id)
        .ok_or_else(|| format!("unknown ROM slot “{slot_id}”"))?;
    let data = fs::read(source).map_err(|e| format!("read {}: {e}", source.display()))?;
    if data.len() != descriptor.expected_bytes {
        return Err(format!(
            "{} must be {} bytes, got {} ({})",
            descriptor.label,
            descriptor.expected_bytes,
            data.len(),
            source.display()
        ));
    }
    let root = roots
        .iter()
        .find(|r| {
            fs::create_dir_all(r.join("roms")).is_ok()
                && r.join(descriptor.install_path)
                    .parent()
                    .is_some_and(|p| fs::create_dir_all(p).is_ok())
        })
        .cloned()
        .unwrap_or_else(writable_install_root);
    let dest = root.join(descriptor.install_path);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let tmp = dest.with_extension("part");
    fs::write(&tmp, &data).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    if fs::rename(&tmp, &dest).is_err() {
        fs::copy(&tmp, &dest).map_err(|e| format!("copy {}: {e}", dest.display()))?;
        let _ = fs::remove_file(&tmp);
    }
    Ok(dest)
}

/// Load TR-DOS ROM bytes when required and available.
pub fn read_trdos_rom_with_overrides(
    model: Model,
    overrides: &BTreeMap<String, PathBuf>,
) -> Result<Vec<u8>, String> {
    if !requires_trdos_rom(model) {
        return Err(format!("{} does not use a TR-DOS ROM", model_title(model)));
    }
    let roots = search_roots();
    let path =
        resolve_trdos_rom_path_in_with_overrides(model, &roots, overrides).ok_or_else(|| {
            format!(
                "TR-DOS ROM for {} not found; {}",
                model_title(model),
                unavailable_reason(model)
            )
        })?;
    let data = std::fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    if data.len() != TRDOS_ROM_SIZE {
        return Err(format!(
            "TR-DOS ROM must be {TRDOS_ROM_SIZE} bytes, got {} ({})",
            data.len(),
            path.display()
        ));
    }
    Ok(data)
}

/// Load TR-DOS ROM bytes when required and available.
pub fn read_trdos_rom(model: Model) -> Result<Vec<u8>, String> {
    read_trdos_rom_with_overrides(model, &BTreeMap::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_models_ui_order() {
        assert_eq!(
            ALL_MODELS,
            [
                Model::Spectrum16K,
                Model::Spectrum48,
                Model::Spectrum128,
                Model::SpectrumPlus2,
                Model::SpectrumPlus2A,
                Model::SpectrumPlus3,
                Model::Pentagon128,
            ]
        );
    }

    #[test]
    fn phase_a_models_use_fetchable_paths() {
        assert!(!requires_user_rom(Model::Spectrum16K));
        assert!(!requires_user_rom(Model::SpectrumPlus2));
        assert_eq!(rom_candidates(Model::Spectrum16K), &["roms/spec48.rom"]);
        assert_eq!(
            rom_candidates(Model::SpectrumPlus2),
            &["roms/plus2/plus2uk.rom"]
        );
    }

    #[test]
    fn pentagon_requires_user_main_and_trdos() {
        assert!(requires_user_rom(Model::Pentagon128));
        assert!(requires_trdos_rom(Model::Pentagon128));
        assert!(!rom_available_in(
            Model::Pentagon128,
            &[PathBuf::from("/nonexistent")]
        ));
    }

    #[test]
    fn rom_availability_matches_filesystem() {
        let roots = search_roots();
        let path = resolve_rom_path_in(Model::Spectrum48, &roots);
        if path.is_some() {
            assert!(rom_available_in(Model::Spectrum48, &roots));
        }
        let plus2 = resolve_rom_path_in(Model::SpectrumPlus2, &roots);
        assert_eq!(
            plus2.is_some(),
            rom_available_in(Model::SpectrumPlus2, &roots)
        );
    }

    #[test]
    fn pentagon_has_two_rom_slots() {
        let slots = rom_slot_descriptors(Model::Pentagon128);
        assert_eq!(slots.len(), 2);
        assert_eq!(slots[0].id, "main");
        assert_eq!(slots[1].id, "trdos");
        assert_eq!(slots[1].expected_bytes, TRDOS_ROM_SIZE);
    }

    #[test]
    fn install_rom_slot_validates_size() {
        let dir = std::env::temp_dir().join(format!("spec_chum_rom_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("tmpdir");
        let bad = dir.join("bad.rom");
        fs::write(&bad, [0u8; 8]).expect("write");
        let err = install_rom_slot(Model::Spectrum48, "main", &bad, std::slice::from_ref(&dir))
            .expect_err("size");
        assert!(err.contains("16384"), "{err}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_rom_slot_copies_to_expected_path() {
        let dir = std::env::temp_dir().join(format!("spec_chum_rom_ok_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("tmpdir");
        let src = dir.join("source.rom");
        fs::write(&src, vec![0xAA; 16 * 1024]).expect("write");
        let dest = install_rom_slot(Model::Spectrum48, "main", &src, std::slice::from_ref(&dir))
            .expect("install");
        assert_eq!(dest, dir.join("roms/spec48.rom"));
        assert_eq!(fs::metadata(&dest).expect("meta").len(), 16 * 1024);
        let state = rom_slot_state(
            Model::Spectrum48,
            rom_slot_descriptors(Model::Spectrum48)[0],
            std::slice::from_ref(&dir),
        );
        assert_eq!(state.status, RomSlotStatus::Found);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn override_path_used_when_workspace_empty() {
        let dir = std::env::temp_dir().join(format!("spec_chum_rom_ov_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("tmpdir");
        let src = dir.join("away.rom");
        fs::write(&src, vec![0xCC; 16 * 1024]).expect("write");
        let mut overrides = BTreeMap::new();
        overrides.insert("main".into(), src.clone());
        assert!(rom_available_in_with_overrides(
            Model::Spectrum48,
            std::slice::from_ref(&dir),
            &overrides
        ));
        let _ = fs::remove_dir_all(&dir);
    }
}
