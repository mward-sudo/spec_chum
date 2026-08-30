//! Thin host API for native shells (SwiftUI) and future cores.
//!
//! Safe Rust surface lives in [`session`]. The C ABI in [`ffi`] is for FFI only
//! and requires a narrowly scoped `unsafe` allow.

#![allow(clippy::pedantic)]

pub mod ffi;
pub mod keymap;
pub mod machine_config;
pub mod prefs;
pub mod rom_setup;
pub mod session;

pub use machine_config::{
    apply_user_config, expected_rom_bytes, hardware_compat, new_config_id, validate_main_rom,
    AppliedConfig, HardwareCompat, MachineConfigError, UserMachineConfig, MAX_CUSTOM_CONFIGS,
};
pub use prefs::{
    default_prefs_path, load_prefs, model_rom_path_key, pref_model_slug, save_prefs, update_prefs,
    PrefAyStereo, PrefJoystick, PrefModel, UiPreferences, MAX_RECENT_FILES, MIN_WINDOW_HEIGHT,
    MIN_WINDOW_WIDTH, PREFS_VERSION,
};
pub use rom_setup::{
    install_model_rom, model_rom_available, model_rom_paths_snapshot, rom_setup_json,
    slot_rom_overrides_for_model, sync_model_rom_paths, RomSetupJson, RomSetupSlot,
};
pub use session::{HostError, HostRegs, HostSession, ModelId, AUDIO_SAMPLE_RATE};
