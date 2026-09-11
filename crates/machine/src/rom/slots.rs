//! ROM slot descriptors, override-aware resolve, and install.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use bus::{TIMEX_EXROM_SIZE, TRDOS_ROM_SIZE};

use crate::Model;

use super::catalog::{
    expected_main_rom_bytes, exrom_candidates, requires_exrom, requires_trdos_rom,
    requires_user_rom, rom_candidates, trdos_rom_candidates, TRDOS_ROM_INSTALL_PATH,
};
use super::resolve::{
    resolve_first_in, resolve_trdos_rom_preferring_file_services, rom_path_status, search_roots,
};
use super::types::{RomReadError, RomSlotDescriptor, RomSlotKind, RomSlotState, RomSlotStatus};

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
            install_path: TRDOS_ROM_INSTALL_PATH,
            search_paths: trdos_rom_candidates(model),
            expected_bytes: TRDOS_ROM_SIZE,
            user_provided: true,
        });
    }
    if requires_exrom(model) {
        slots.push(RomSlotDescriptor {
            kind: RomSlotKind::ExRom,
            id: "exrom",
            label: "EX-ROM",
            install_path: exrom_candidates(model)[0],
            search_paths: exrom_candidates(model),
            expected_bytes: TIMEX_EXROM_SIZE,
            user_provided: false,
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
    let path = if descriptor.kind == RomSlotKind::Trdos {
        resolve_trdos_rom_preferring_file_services(roots, descriptor.search_paths)
    } else {
        resolve_first_in(roots, descriptor.search_paths)
    };
    if let Some(path) = path {
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

/// First resolved EX-ROM path for Timex 2068 models, if any.
#[must_use]
pub fn resolve_exrom_path(model: Model) -> Option<PathBuf> {
    resolve_exrom_path_in(model, &search_roots())
}

#[must_use]
pub fn resolve_exrom_path_in(model: Model, roots: &[PathBuf]) -> Option<PathBuf> {
    if !requires_exrom(model) {
        return None;
    }
    resolve_first_in(roots, exrom_candidates(model))
}

#[must_use]
pub fn resolve_exrom_path_in_with_overrides(
    model: Model,
    roots: &[PathBuf],
    overrides: &BTreeMap<String, PathBuf>,
) -> Option<PathBuf> {
    if !requires_exrom(model) {
        return None;
    }
    let state = rom_slot_state_with_override(
        rom_slot_descriptors(model)
            .into_iter()
            .find(|d| d.kind == RomSlotKind::ExRom)
            .expect("exrom slot"),
        roots,
        overrides.get("exrom").map(PathBuf::as_path),
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
) -> Result<PathBuf, RomReadError> {
    let descriptor = rom_slot_descriptors(model)
        .into_iter()
        .find(|d| d.id == slot_id)
        .ok_or_else(|| RomReadError::UnknownSlot(slot_id.to_string()))?;
    let data = fs::read(source).map_err(|source_err| RomReadError::Io {
        op: "read",
        path: source.display().to_string(),
        source: source_err,
    })?;
    if data.len() != descriptor.expected_bytes {
        return Err(RomReadError::WrongSize {
            kind: descriptor.label,
            expected: descriptor.expected_bytes,
            got: data.len(),
            path: source.display().to_string(),
        });
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
        fs::create_dir_all(parent).map_err(|source| RomReadError::Io {
            op: "mkdir",
            path: parent.display().to_string(),
            source,
        })?;
    }
    let tmp = dest.with_extension("part");
    fs::write(&tmp, &data).map_err(|source| RomReadError::Io {
        op: "write",
        path: tmp.display().to_string(),
        source,
    })?;
    if fs::rename(&tmp, &dest).is_err() {
        fs::copy(&tmp, &dest).map_err(|source| RomReadError::Io {
            op: "copy",
            path: dest.display().to_string(),
            source,
        })?;
        let _ = fs::remove_file(&tmp);
    }
    Ok(dest)
}
