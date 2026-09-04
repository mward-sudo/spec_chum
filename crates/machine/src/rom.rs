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

use bus::{TIMEX_EXROM_SIZE, TRDOS_ROM_SIZE};

use crate::Model;

/// Which ROM image a setup slot refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RomSlotKind {
    Main,
    Trdos,
    /// Timex TS2068 / TC2068 EX-ROM (8 KiB).
    ExRom,
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
        Model::TimexTC2048 => &["roms/timex/tc2048.rom"],
        Model::TimexTS2068 => &["roms/timex/tc2068-0.rom"],
        Model::Spectrum128 => &["roms/128/spec128uk.rom"],
        Model::SpectrumPlus2 => &["roms/plus2/plus2uk.rom"],
        Model::SpectrumPlus2A => &["roms/plus2a/plus2a.rom", "roms/plus3/plus3.rom"],
        Model::SpectrumPlus3 => &["roms/plus3/plus3.rom"],
        Model::Pentagon128 => &["roms/pentagon/pentagon.rom", "roms/pentagon/128p.rom"],
    }
}

/// Relative EX-ROM search paths (TS2068 / TC2068; first hit wins).
#[must_use]
pub fn exrom_candidates(_model: Model) -> &'static [&'static str] {
    &["roms/timex/tc2068-1.rom"]
}

/// Canonical install / picker path for user-provided TR-DOS (Pentagon).
pub const TRDOS_ROM_INSTALL_PATH: &str = "roms/pentagon/trdos.rom";

/// Relative TR-DOS ROM search paths (Pentagon / Beta attach).
///
/// Complete dumps (`*-5.04t` / `*-complete`) are listed first so native
/// `08D2h` / `0D6Bh` file-load services win over the usual hole-filled 5.04
/// image when both are present ([#140](https://github.com/mward-sudo/spec_chum/issues/140)).
#[must_use]
pub fn trdos_rom_candidates(_model: Model) -> &'static [&'static str] {
    &[
        "roms/pentagon/trdos-5.04t.rom",
        "roms/pentagon/trdos-complete.rom",
        "roms/trdos/trdos-5.04t.rom",
        "roms/trdos/trdos-complete.rom",
        TRDOS_ROM_INSTALL_PATH,
        "roms/trdos/trdos.rom",
        "roms/trdos.rom",
    ]
}

/// True when `08D2h` is Alone Coder / VfNG **5.04T** VG93 port-remap stub
/// (`LD A,#2C; JP 0897h`), not a classic TR-DOS file-load service.
#[must_use]
pub fn trdos_rom_08d2_is_vg93_port_stub(data: &[u8]) -> bool {
    data.get(0x08d2..0x08d7) == Some(&[0x3e, 0x2c, 0xc3, 0x97, 0x08][..])
}

/// True when a 16 KiB TR-DOS image has real code in the usual 5.04 hole region.
///
/// Alone Coder **5.04T** fills `0800h`+ with VG93 helpers (including a port stub at
/// `08D2h`); classic file-load at `08D2h` is still absent — see
/// [`trdos_rom_has_native_file_services`].
///
/// Rejects FF padding **and** blank (all-zero) images — probes must be non-zero
/// program bytes, not an empty buffer of the right length.
#[must_use]
pub fn trdos_rom_fills_0800_hole(data: &[u8]) -> bool {
    data.len() == TRDOS_ROM_SIZE
        && data.get(0x08d2).is_some_and(|b| !matches!(*b, 0x00 | 0xff))
        && data.get(0x0d6b).is_some_and(|b| !matches!(*b, 0x00 | 0xff))
}

/// True when a 16 KiB TR-DOS image has a classic RUN file-load service at `08D2h`.
///
/// Many circulating **Ver 5.04** dumps leave `0800h`–`0E71h` as FF padding.
/// Alone Coder / VfNG **5.04T** fills that hole with VG93 port remapping (`0897h`
/// trampoline; `08D2h` is `LD A,#2C; JP 0897h`) — not the stock file loader — so
/// post-match `19ECh` still needs the FDC/`LINE-NEW` stand-in for `boot`.
#[must_use]
pub fn trdos_rom_has_native_file_services(data: &[u8]) -> bool {
    trdos_rom_fills_0800_hole(data) && !trdos_rom_08d2_is_vg93_port_stub(data)
}

/// Expected main-ROM byte length for `model`.
#[must_use]
pub fn expected_main_rom_bytes(model: Model) -> usize {
    match model {
        Model::Spectrum16K | Model::Spectrum48 | Model::TimexTC2048 | Model::TimexTS2068 => {
            16 * 1024
        }
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

/// True when the model needs the Timex EX-ROM (8 KiB) before boot.
#[must_use]
pub fn requires_exrom(model: Model) -> bool {
    matches!(model, Model::TimexTS2068)
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
        Model::TimexTC2048 => "TC2048",
        Model::TimexTS2068 => "TS2068",
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
        Model::TimexTC2048 => "Timex TC2048",
        Model::TimexTS2068 => "Timex TS2068",
    }
}

/// Canonical UI order for every host picker / menu.
pub const ALL_MODELS: [Model; 9] = [
    Model::Spectrum16K,
    Model::Spectrum48,
    Model::Spectrum128,
    Model::SpectrumPlus2,
    Model::SpectrumPlus2A,
    Model::SpectrumPlus3,
    Model::Pentagon128,
    Model::TimexTC2048,
    Model::TimexTS2068,
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

/// Load Timex EX-ROM bytes when required and available.
pub fn read_exrom_with_overrides(
    model: Model,
    overrides: &BTreeMap<String, PathBuf>,
) -> Result<Vec<u8>, String> {
    if !requires_exrom(model) {
        return Err(format!("{} does not use an EX-ROM", model_title(model)));
    }
    let roots = search_roots();
    let path = resolve_exrom_path_in_with_overrides(model, &roots, overrides).ok_or_else(|| {
        format!(
            "EX-ROM for {} not found; {}",
            model_title(model),
            unavailable_reason(model)
        )
    })?;
    let data = std::fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    if data.len() != TIMEX_EXROM_SIZE {
        return Err(format!(
            "EX-ROM must be {TIMEX_EXROM_SIZE} bytes, got {} ({})",
            data.len(),
            path.display()
        ));
    }
    Ok(data)
}

/// Load Timex EX-ROM bytes when required and available.
pub fn read_exrom(model: Model) -> Result<Vec<u8>, String> {
    read_exrom_with_overrides(model, &BTreeMap::new())
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
                Model::TimexTC2048,
                Model::TimexTS2068,
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
    fn timex_tc2048_uses_fetchable_rom_path() {
        assert!(!requires_user_rom(Model::TimexTC2048));
        assert_eq!(
            rom_candidates(Model::TimexTC2048),
            &["roms/timex/tc2048.rom"]
        );
    }

    #[test]
    fn timex_ts2068_requires_home_and_exrom_slots() {
        assert!(!requires_user_rom(Model::TimexTS2068));
        assert!(requires_exrom(Model::TimexTS2068));
        assert_eq!(
            rom_candidates(Model::TimexTS2068),
            &["roms/timex/tc2068-0.rom"]
        );
        let slots = rom_slot_descriptors(Model::TimexTS2068);
        assert_eq!(slots.len(), 2);
        assert_eq!(slots[0].id, "main");
        assert_eq!(slots[1].id, "exrom");
        assert_eq!(slots[1].expected_bytes, TIMEX_EXROM_SIZE);
    }

    #[test]
    fn pentagon_has_two_rom_slots() {
        let slots = rom_slot_descriptors(Model::Pentagon128);
        assert_eq!(slots.len(), 2);
        assert_eq!(slots[0].id, "main");
        assert_eq!(slots[1].id, "trdos");
        assert_eq!(slots[1].expected_bytes, TRDOS_ROM_SIZE);
        assert_eq!(slots[1].install_path, TRDOS_ROM_INSTALL_PATH);
        assert!(slots[1]
            .search_paths
            .contains(&"roms/pentagon/trdos-5.04t.rom"));
    }

    #[test]
    fn trdos_native_file_services_rejects_ff_hole() {
        let blank = vec![0u8; TRDOS_ROM_SIZE];
        assert!(
            !trdos_rom_fills_0800_hole(&blank),
            "all-zero 16KiB must not count as a filled hole"
        );
        assert!(!trdos_rom_has_native_file_services(&blank));
        let mut hole = vec![0u8; TRDOS_ROM_SIZE];
        hole[0x08d2] = 0xff;
        hole[0x0d6b] = 0xff;
        assert!(!trdos_rom_has_native_file_services(&hole));
        assert!(!trdos_rom_fills_0800_hole(&hole));
        hole[0x08d2] = 0xe7; // plausible RST #20
        hole[0x0d6b] = 0xc9;
        assert!(trdos_rom_fills_0800_hole(&hole));
        assert!(trdos_rom_has_native_file_services(&hole));
        // Alone Coder 5.04T: VG93 port stub at 08D2h is not a file-load service.
        hole[0x08d2..0x08d7].copy_from_slice(&[0x3e, 0x2c, 0xc3, 0x97, 0x08]);
        assert!(trdos_rom_08d2_is_vg93_port_stub(&hole));
        assert!(trdos_rom_fills_0800_hole(&hole));
        assert!(!trdos_rom_has_native_file_services(&hole));
        assert!(!trdos_rom_has_native_file_services(&[0u8; 8]));
    }

    #[test]
    fn resolve_trdos_prefers_complete_over_hole() {
        let dir = std::env::temp_dir().join(format!("spec_chum_trdos_pref_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let pent = dir.join("roms/pentagon");
        fs::create_dir_all(&pent).expect("tmpdir");
        let mut hole = vec![0u8; TRDOS_ROM_SIZE];
        hole[0x08d2] = 0xff;
        hole[0x0d6b] = 0xff;
        fs::write(pent.join("trdos.rom"), &hole).expect("hole");
        let mut complete = vec![0x00; TRDOS_ROM_SIZE];
        complete[0x08d2] = 0xe7;
        complete[0x0d6b] = 0xc9;
        fs::write(pent.join("trdos-5.04t.rom"), &complete).expect("complete");
        let got = resolve_trdos_rom_preferring_file_services(
            std::slice::from_ref(&dir),
            trdos_rom_candidates(Model::Pentagon128),
        )
        .expect("resolve");
        assert_eq!(got, pent.join("trdos-5.04t.rom"));
        let _ = fs::remove_dir_all(&dir);
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
