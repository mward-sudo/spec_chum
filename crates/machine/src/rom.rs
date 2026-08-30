//! ROM path catalog and availability for host pickers (#188).
//!
//! Phase A models use the UK primary paths under `roms/` (see `rom_candidates`).
//! Phase B (Pentagon 128) requires user-provided main + TR-DOS ROMs — never fetched.
//! Additional distributable images fetched by `./scripts/fetch_roms.sh` — Timex,
//! OpenSE, +3e, Datel, SpeccyBoot, regional alternates — are documented in
//! `docs/ROMS.md` (#190); wire `rom_candidates` when those models ship.

use std::path::PathBuf;

use bus::TRDOS_ROM_SIZE;

use crate::Model;

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
    if !main_rom_available_in(model, roots) {
        return false;
    }
    trdos_rom_available_in(model, roots)
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
pub fn read_rom(model: Model) -> Result<Vec<u8>, String> {
    let path = resolve_rom_path(model).ok_or_else(|| {
        format!(
            "ROM for {} not found; {}",
            model_title(model),
            unavailable_reason(model)
        )
    })?;
    std::fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))
}

/// Load TR-DOS ROM bytes when required and available.
pub fn read_trdos_rom(model: Model) -> Result<Vec<u8>, String> {
    if !requires_trdos_rom(model) {
        return Err(format!("{} does not use a TR-DOS ROM", model_title(model)));
    }
    let path = resolve_trdos_rom_path(model).ok_or_else(|| {
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
}
