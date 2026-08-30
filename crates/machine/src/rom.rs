//! ROM path catalog and availability for host pickers (#188).

use std::path::PathBuf;

use crate::Model;

/// Relative ROM search paths for a model (first hit wins).
#[must_use]
pub fn rom_candidates(model: Model) -> &'static [&'static str] {
    match model {
        Model::Spectrum16K | Model::Spectrum48 => &["roms/spec48.rom"],
        Model::Spectrum128 => &["roms/128/spec128uk.rom"],
        Model::SpectrumPlus2 => &["roms/plus2/plus2uk.rom"],
        Model::SpectrumPlus2A => &["roms/plus2a/plus2a.rom", "roms/plus3/plus3.rom"],
        Model::SpectrumPlus3 => &["roms/plus3/plus3.rom"],
    }
}

/// Models that only boot after the user supplies a ROM (future clones / user-provided).
#[must_use]
pub fn requires_user_rom(_model: Model) -> bool {
    // Phase A official models are auto-fetched; Pentagon etc. will return true later.
    false
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
    }
}

/// Every selectable model in UI order.
pub const ALL_MODELS: [Model; 6] = [
    Model::Spectrum16K,
    Model::Spectrum48,
    Model::Spectrum128,
    Model::SpectrumPlus2,
    Model::SpectrumPlus2A,
    Model::SpectrumPlus3,
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

/// True when any candidate ROM exists under `search_roots()`.
#[must_use]
pub fn rom_available(model: Model) -> bool {
    rom_available_in(model, &search_roots())
}

/// True when any candidate ROM exists under the given roots.
#[must_use]
pub fn rom_available_in(model: Model, roots: &[PathBuf]) -> bool {
    if requires_user_rom(model) {
        return false;
    }
    for root in roots {
        for rel in rom_candidates(model) {
            if root.join(rel).is_file() {
                return true;
            }
        }
    }
    false
}

/// First resolved ROM path, if any.
#[must_use]
pub fn resolve_rom_path(model: Model) -> Option<PathBuf> {
    resolve_rom_path_in(model, &search_roots())
}

#[must_use]
pub fn resolve_rom_path_in(model: Model, roots: &[PathBuf]) -> Option<PathBuf> {
    for root in roots {
        for rel in rom_candidates(model) {
            let path = root.join(rel);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

/// Hint shown when a model is disabled in the picker.
#[must_use]
pub fn unavailable_reason(model: Model) -> &'static str {
    if requires_user_rom(model) {
        return "Supply a ROM for this model (see Help → ROMs)";
    }
    let paths = rom_candidates(model);
    if paths.len() == 1 {
        return "Add roms/… or run ./scripts/fetch_roms.sh";
    }
    "Add a ROM under roms/ or run ./scripts/fetch_roms.sh"
}

/// Load ROM bytes for `model` when available.
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

#[cfg(test)]
mod tests {
    use super::*;

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
