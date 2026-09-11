//! Workspace search roots, path resolve, and availability checks.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use bus::TRDOS_ROM_SIZE;

use crate::Model;

use super::catalog::{
    exrom_candidates, requires_exrom, requires_trdos_rom, requires_user_rom, rom_candidates,
    trdos_rom_candidates,
};
use super::slots::rom_available_in_with_overrides;
use super::trdos::trdos_rom_fills_0800_hole;
use super::types::RomSlotStatus;

/// Workspace / env / cwd / packaged-app roots tried when autoloading ROMs.
///
/// Packaged builds embed redistributable images under `roms/` next to the
/// executable, in a macOS `.app` `Contents/Resources`, or under
/// `share/spec-chum` for Linux FHS / `AppImage` layouts (see `docs/ROMS.md`).
#[must_use]
pub fn search_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut push_unique = |path: PathBuf| {
        if !roots.iter().any(|existing| existing == &path) {
            roots.push(path);
        }
    };
    if let Ok(cwd) = std::env::current_dir() {
        push_unique(cwd);
    }
    if let Ok(env) = std::env::var("SPEC_CHUM_ROOT") {
        push_unique(PathBuf::from(env));
    }
    if let Ok(env) = std::env::var("SPEC_CHUM_ROM_ROOT") {
        push_unique(PathBuf::from(env));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            push_unique(exe_dir.to_path_buf());
            // Spec Chum.app/Contents/MacOS → Contents/Resources (bundled roms/).
            if exe_dir.file_name().is_some_and(|name| name == "MacOS") {
                if let Some(contents) = exe_dir.parent() {
                    push_unique(contents.join("Resources"));
                }
            }
            // /usr/bin/spec_chum → /usr/share/spec-chum (deb / AppImage FHS).
            if let Some(prefix) = exe_dir.parent() {
                push_unique(prefix.join("share/spec-chum"));
            }
        }
    }
    // Dev / `cargo test`: crates/machine → workspace root.
    push_unique(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    roots
}

pub(super) fn resolve_first_in(roots: &[PathBuf], rel_paths: &[&str]) -> Option<PathBuf> {
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
    resolve_trdos_rom_path_in(model, roots).is_some()
}

/// True when a Timex EX-ROM exists for models that require one.
#[must_use]
pub fn exrom_available(model: Model) -> bool {
    exrom_available_in(model, &search_roots())
}

#[must_use]
pub fn exrom_available_in(model: Model, roots: &[PathBuf]) -> bool {
    if !requires_exrom(model) {
        return true;
    }
    resolve_first_in(roots, exrom_candidates(model)).is_some()
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
///
/// Prefers a dump that fills the usual 5.04 `0800h` hole when several candidates
/// exist under `roots` (complete / 5.04T name first, then any non-hole image).
#[must_use]
pub fn resolve_trdos_rom_path(model: Model) -> Option<PathBuf> {
    resolve_trdos_rom_path_in(model, &search_roots())
}

#[must_use]
pub fn resolve_trdos_rom_path_in(model: Model, roots: &[PathBuf]) -> Option<PathBuf> {
    if !requires_trdos_rom(model) {
        return None;
    }
    resolve_trdos_rom_preferring_file_services(roots, trdos_rom_candidates(model))
}

/// Scan `rel_paths` under `roots`; prefer images that fill the `0800h` hole.
#[must_use]
pub fn resolve_trdos_rom_preferring_file_services(
    roots: &[PathBuf],
    rel_paths: &[&str],
) -> Option<PathBuf> {
    let mut fallback: Option<PathBuf> = None;
    for root in roots {
        for rel in rel_paths {
            let path = root.join(rel);
            if !path.is_file() {
                continue;
            }
            let Ok(data) = fs::read(&path) else {
                continue;
            };
            if data.len() != TRDOS_ROM_SIZE {
                continue;
            }
            // Prefer filled-hole dumps (5.04T / true 5.03) over FF-padded 5.04.
            if trdos_rom_fills_0800_hole(&data) {
                return Some(path);
            }
            if fallback.is_none() {
                fallback = Some(path);
            }
        }
    }
    fallback
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
    if requires_exrom(model) {
        if !main_rom_available(model) {
            return "Add roms/timex/tc2068-0.rom or run ./scripts/fetch_roms.sh";
        }
        if !exrom_available(model) {
            return "Add roms/timex/tc2068-1.rom or run ./scripts/fetch_roms.sh";
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
