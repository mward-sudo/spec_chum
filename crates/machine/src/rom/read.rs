//! Load main / TR-DOS / EX-ROM bytes from resolved paths.

use std::collections::BTreeMap;
use std::path::PathBuf;

use bus::{TIMEX_EXROM_SIZE, TRDOS_ROM_SIZE};

use crate::Model;

use super::catalog::{model_title, requires_exrom, requires_trdos_rom};
use super::resolve::{search_roots, unavailable_reason};
use super::slots::{
    resolve_exrom_path_in_with_overrides, resolve_rom_path_in_with_overrides,
    resolve_trdos_rom_path_in_with_overrides,
};
use super::types::RomReadError;

/// Load main ROM bytes for `model` when available.
pub fn read_rom_with_overrides(
    model: Model,
    overrides: &BTreeMap<String, PathBuf>,
) -> Result<Vec<u8>, RomReadError> {
    let roots = search_roots();
    let path = resolve_rom_path_in_with_overrides(model, &roots, overrides).ok_or_else(|| {
        RomReadError::NotFound {
            model: model_title(model).to_string(),
            hint: unavailable_reason(model).to_string(),
        }
    })?;
    std::fs::read(&path).map_err(|source| RomReadError::Io {
        op: "read",
        path: path.display().to_string(),
        source,
    })
}

/// Load main ROM bytes for `model` when available.
pub fn read_rom(model: Model) -> Result<Vec<u8>, RomReadError> {
    read_rom_with_overrides(model, &BTreeMap::new())
}

/// Load TR-DOS ROM bytes when required and available.
pub fn read_trdos_rom_with_overrides(
    model: Model,
    overrides: &BTreeMap<String, PathBuf>,
) -> Result<Vec<u8>, RomReadError> {
    if !requires_trdos_rom(model) {
        return Err(RomReadError::TrdosNotApplicable {
            model: model_title(model).to_string(),
        });
    }
    let roots = search_roots();
    let path =
        resolve_trdos_rom_path_in_with_overrides(model, &roots, overrides).ok_or_else(|| {
            RomReadError::TrdosNotFound {
                model: model_title(model).to_string(),
                hint: unavailable_reason(model).to_string(),
            }
        })?;
    let data = std::fs::read(&path).map_err(|source| RomReadError::Io {
        op: "read",
        path: path.display().to_string(),
        source,
    })?;
    if data.len() != TRDOS_ROM_SIZE {
        return Err(RomReadError::WrongSize {
            kind: "TR-DOS ROM",
            expected: TRDOS_ROM_SIZE,
            got: data.len(),
            path: path.display().to_string(),
        });
    }
    Ok(data)
}

/// Load TR-DOS ROM bytes when required and available.
pub fn read_trdos_rom(model: Model) -> Result<Vec<u8>, RomReadError> {
    read_trdos_rom_with_overrides(model, &BTreeMap::new())
}

/// Load Timex EX-ROM bytes when required and available.
pub fn read_exrom_with_overrides(
    model: Model,
    overrides: &BTreeMap<String, PathBuf>,
) -> Result<Vec<u8>, RomReadError> {
    if !requires_exrom(model) {
        return Err(RomReadError::ExromNotApplicable {
            model: model_title(model).to_string(),
        });
    }
    let roots = search_roots();
    let path = resolve_exrom_path_in_with_overrides(model, &roots, overrides).ok_or_else(|| {
        RomReadError::ExromNotFound {
            model: model_title(model).to_string(),
            hint: unavailable_reason(model).to_string(),
        }
    })?;
    let data = std::fs::read(&path).map_err(|source| RomReadError::Io {
        op: "read",
        path: path.display().to_string(),
        source,
    })?;
    if data.len() != TIMEX_EXROM_SIZE {
        return Err(RomReadError::WrongSize {
            kind: "EX-ROM",
            expected: TIMEX_EXROM_SIZE,
            got: data.len(),
            path: path.display().to_string(),
        });
    }
    Ok(data)
}

/// Load Timex EX-ROM bytes when required and available.
pub fn read_exrom(model: Model) -> Result<Vec<u8>, RomReadError> {
    read_exrom_with_overrides(model, &BTreeMap::new())
}
