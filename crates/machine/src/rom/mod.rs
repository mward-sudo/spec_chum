//! ROM path catalog and availability for host pickers (#188).
//!
//! Phase A models use the UK primary paths under `roms/` (see `rom_candidates`).
//! Phase B (Pentagon 128) requires user-provided main + TR-DOS ROMs — never fetched.
//! Additional distributable images fetched by `./scripts/fetch_roms.sh` — Timex,
//! `OpenSE`, +3e, Datel, `SpeccyBoot`, regional alternates — are documented in
//! `docs/ROMS.md` (#190); wire `rom_candidates` when those models ship.
//!
//! Layout (#411 cohesion split): [`types`], [`catalog`], [`trdos`], [`resolve`],
//! [`slots`], [`read`]. Public API surface is unchanged.

mod catalog;
mod read;
mod resolve;
mod slots;
mod trdos;
mod types;

#[cfg(test)]
mod tests;

pub use catalog::{
    expected_main_rom_bytes, exrom_candidates, model_label, model_title, requires_exrom,
    requires_trdos_rom, requires_user_rom, rom_candidates, trdos_rom_candidates, ALL_MODELS,
    TRDOS_ROM_INSTALL_PATH,
};
pub use read::{
    read_exrom, read_exrom_with_overrides, read_rom, read_rom_with_overrides, read_trdos_rom,
    read_trdos_rom_with_overrides,
};
pub use resolve::{
    exrom_available, exrom_available_in, main_rom_available, main_rom_available_in,
    resolve_rom_path, resolve_rom_path_in, resolve_trdos_rom_path, resolve_trdos_rom_path_in,
    resolve_trdos_rom_preferring_file_services, rom_available, rom_available_in, rom_path_status,
    search_roots, trdos_rom_available, trdos_rom_available_in, unavailable_reason,
};
pub use slots::{
    install_rom_slot, resolve_exrom_path, resolve_exrom_path_in,
    resolve_exrom_path_in_with_overrides, resolve_rom_path_in_with_overrides,
    resolve_trdos_rom_path_in_with_overrides, rom_available_in_with_overrides,
    rom_slot_descriptors, rom_slot_state, rom_slot_state_with_override, rom_slot_states,
    rom_slot_states_with_overrides, writable_install_root,
};
pub use trdos::{
    trdos_rom_08d2_is_vg93_port_stub, trdos_rom_fills_0800_hole, trdos_rom_has_native_file_services,
};
pub use types::{RomReadError, RomSlotDescriptor, RomSlotKind, RomSlotState, RomSlotStatus};
