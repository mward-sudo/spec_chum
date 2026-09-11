//! Static ROM path catalog and model metadata for host pickers.

use crate::Model;

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
        Model::SpectrumPlus3e => &["roms/plus3e/plus3e.rom"],
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

/// Expected main-ROM byte length for `model`.
#[must_use]
pub fn expected_main_rom_bytes(model: Model) -> usize {
    match model {
        Model::Spectrum16K | Model::Spectrum48 | Model::TimexTC2048 | Model::TimexTS2068 => {
            16 * 1024
        }
        Model::Spectrum128 | Model::SpectrumPlus2 | Model::Pentagon128 => 32 * 1024,
        Model::SpectrumPlus2A | Model::SpectrumPlus3 | Model::SpectrumPlus3e => 64 * 1024,
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
        Model::SpectrumPlus3e => "+3e",
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
        Model::SpectrumPlus3e => "Spectrum +3e (enhanced)",
        Model::Pentagon128 => "Pentagon 128",
        Model::TimexTC2048 => "Timex TC2048",
        Model::TimexTS2068 => "Timex TS2068",
    }
}

/// Canonical UI order for every host picker / menu.
pub const ALL_MODELS: [Model; 10] = [
    Model::Spectrum16K,
    Model::Spectrum48,
    Model::Spectrum128,
    Model::SpectrumPlus2,
    Model::SpectrumPlus2A,
    Model::SpectrumPlus3,
    Model::SpectrumPlus3e,
    Model::Pentagon128,
    Model::TimexTC2048,
    Model::TimexTS2068,
];
