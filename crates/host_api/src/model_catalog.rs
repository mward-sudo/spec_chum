//! Typed host adapter for the machine-owned model and ROM catalog.

use serde::Serialize;

use crate::{ModelId, PrefModel};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HostModelDescriptor {
    /// Stable C ABI numeric identifier; independent of picker order.
    pub id: u32,
    pub preference_slug: &'static str,
    pub label: &'static str,
    pub title: &'static str,
    /// Presentation asset key. None means hosts must show a labeled fallback.
    pub image_key: Option<&'static str>,
    pub image_description: Option<&'static str>,
    /// Display copy sourced from the emulated machine's known hardware.
    pub memory_sound_summary: Option<&'static str>,
    pub compatible_peripherals: crate::HardwareCompat,
    /// Current verified asset availability; selection still revalidates at boot.
    pub available: bool,
    pub expected_main_rom_bytes: usize,
    pub requires_user_rom: bool,
    pub requires_trdos_rom: bool,
    pub requires_exrom: bool,
    pub rom_slots: Vec<HostRomSlotDescriptor>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HostRomSlotDescriptor {
    pub id: &'static str,
    pub label: &'static str,
    pub install_path: &'static str,
    pub search_paths: Vec<&'static str>,
    pub expected_bytes: usize,
    pub user_provided: bool,
}

#[must_use]
pub fn host_model_catalog() -> Vec<HostModelDescriptor> {
    let rom_paths = crate::rom_setup::model_rom_paths_snapshot();
    ModelId::ALL
        .into_iter()
        .map(|id| {
            let model = id.to_model();
            let preference = PrefModel::from_model_id(id);
            let machine_slots = machine::rom_slot_descriptors(model);
            HostModelDescriptor {
                id: id.numeric_id(),
                preference_slug: preference.slug(),
                label: machine::model_label(model),
                title: machine::model_title(model),
                image_key: Some(model_image_key(model)),
                image_description: Some(model_image_description(model)),
                memory_sound_summary: Some(model_summary(model)),
                compatible_peripherals: crate::hardware_compat(preference),
                available: crate::rom_setup::model_rom_available(id, &rom_paths),
                expected_main_rom_bytes: machine::expected_main_rom_bytes(model),
                requires_user_rom: machine::requires_user_rom(model),
                requires_trdos_rom: machine::requires_trdos_rom(model),
                requires_exrom: machine::requires_exrom(model),
                rom_slots: machine_slots
                    .into_iter()
                    .map(|slot| HostRomSlotDescriptor {
                        id: slot.id,
                        label: slot.label,
                        install_path: slot.install_path,
                        search_paths: slot.search_paths.to_vec(),
                        expected_bytes: slot.expected_bytes,
                        user_provided: slot.user_provided,
                    })
                    .collect(),
            }
        })
        .collect()
}

/// Catalog key for a bundled computer image. Generated illustrations are explicitly
/// identified in `model_image_description`; several variants share an exterior.
const fn model_image_key(model: machine::Model) -> &'static str {
    use machine::Model;
    match model {
        Model::Spectrum16K | Model::Spectrum48 => "spectrum48",
        Model::Spectrum128 => "spectrum128",
        Model::SpectrumPlus2 => "plus2",
        Model::SpectrumPlus2A => "plus2a_black",
        Model::SpectrumPlus3 | Model::SpectrumPlus3e => "plus3",
        Model::TimexTC2048 => "tc2048",
        Model::TimexTS2068 => "ts2068",
        Model::Pentagon128 => "pentagon128_candidate",
        Model::ScorpionZs256 => "scorpion_zs256_candidate",
        Model::SpectrumNext => "spectrum_next_candidate",
    }
}

/// Short visual label for screen readers; model facts remain in `memory_sound_summary`.
const fn model_image_description(model: machine::Model) -> &'static str {
    use machine::Model;
    match model {
        Model::Spectrum16K => {
            "Original black rubber-key ZX Spectrum exterior shared with 48K"
        }
        Model::Spectrum48 => "Black rubber-key Sinclair ZX Spectrum",
        Model::Spectrum128 => "Sinclair ZX Spectrum 128K with numeric keypad and 128K badge",
        Model::SpectrumPlus2 => "Grey ZX Spectrum +2 with integrated cassette deck",
        Model::SpectrumPlus2A => {
            "Black ZX Spectrum +2A family case with integrated cassette deck"
        }
        Model::SpectrumPlus3 | Model::SpectrumPlus3e => {
            "Black ZX Spectrum +3 case with integrated floppy disk drive"
        }
        Model::TimexTC2048 => "Timex Computer 2048 with white keys and black case",
        Model::TimexTS2068 => "Timex Sinclair 2068 with grey case and white keys",
        Model::Pentagon128 => "Generated illustrative candidate of a Pentagon-style 128K clone; exterior varies by build",
        Model::ScorpionZs256 => "Generated illustrative candidate of a Scorpion ZS-256-style clone; exterior varies by build",
        Model::SpectrumNext => "Generated illustrative candidate of a cased Spectrum Next; shown as a concept, not a specific revision",
    }
}

/// Model distinctions confirmed by the machine implementation and its ROM/media policy.
/// This copy is for the picker only; it is not used to configure emulation.
const fn model_summary(model: machine::Model) -> &'static str {
    use machine::Model;
    match model {
        Model::Spectrum16K => "16 KiB RAM · 48K ULA timing",
        Model::Spectrum48 => "48 KiB RAM · beeper audio",
        Model::Spectrum128 => "128 KiB RAM · AY sound",
        Model::SpectrumPlus2 => "128 KiB RAM · grey +2 ROM",
        Model::SpectrumPlus2A => "+2A gate array · no built-in disk interface",
        Model::SpectrumPlus3 => "+3 gate array · built-in disk interface",
        Model::SpectrumPlus3e => "+3 hardware · enhanced +3e firmware",
        Model::Pentagon128 => "128K clone · TR-DOS ROM required",
        Model::ScorpionZs256 => "256K clone · TR-DOS ROM required",
        Model::TimexTC2048 => "48K-class hardware · SCLD ports",
        Model::TimexTS2068 => "Home ROM + EX-ROM · horizontal MMU · AY",
        Model::SpectrumNext => "Spectrum Next · verified System/Next assets",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_covers_picker_order_stable_ids_slugs_titles_and_rom_slots() {
        let catalog = host_model_catalog();
        assert_eq!(catalog.len(), machine::ALL_MODELS.len());

        for ((descriptor, id), model) in catalog.iter().zip(ModelId::ALL).zip(machine::ALL_MODELS) {
            assert_eq!(descriptor.id, id.numeric_id());
            assert_eq!(id.to_model(), model);
            assert_eq!(
                descriptor.preference_slug,
                PrefModel::from_model_id(id).slug()
            );
            assert_eq!(
                descriptor.preference_slug,
                crate::pref_model_slug(PrefModel::from_model_id(id))
            );
            assert_eq!(descriptor.label, machine::model_label(model));
            assert_eq!(descriptor.title, machine::model_title(model));
            assert!(descriptor.memory_sound_summary.is_some());
            assert_eq!(
                descriptor.compatible_peripherals,
                crate::hardware_compat(PrefModel::from_model_id(id))
            );
            let (expected_image_key, expected_image_description) = match model {
                machine::Model::Spectrum16K => (
                    Some("spectrum48"),
                    Some("Original black rubber-key ZX Spectrum exterior shared with 48K"),
                ),
                machine::Model::Spectrum48 => (
                    Some("spectrum48"),
                    Some("Black rubber-key Sinclair ZX Spectrum"),
                ),
                machine::Model::Spectrum128 => (
                    Some("spectrum128"),
                    Some("Sinclair ZX Spectrum 128K with numeric keypad and 128K badge"),
                ),
                machine::Model::SpectrumPlus2 => (
                    Some("plus2"),
                    Some("Grey ZX Spectrum +2 with integrated cassette deck"),
                ),
                machine::Model::SpectrumPlus2A => (
                    Some("plus2a_black"),
                    Some("Black ZX Spectrum +2A family case with integrated cassette deck"),
                ),
                machine::Model::SpectrumPlus3 | machine::Model::SpectrumPlus3e => (
                    Some("plus3"),
                    Some("Black ZX Spectrum +3 case with integrated floppy disk drive"),
                ),
                machine::Model::TimexTC2048 => (
                    Some("tc2048"),
                    Some("Timex Computer 2048 with white keys and black case"),
                ),
                machine::Model::TimexTS2068 => (
                    Some("ts2068"),
                    Some("Timex Sinclair 2068 with grey case and white keys"),
                ),
                machine::Model::Pentagon128 => (Some("pentagon128_candidate"), Some("Generated illustrative candidate of a Pentagon-style 128K clone; exterior varies by build")),
                machine::Model::ScorpionZs256 => (Some("scorpion_zs256_candidate"), Some("Generated illustrative candidate of a Scorpion ZS-256-style clone; exterior varies by build")),
                machine::Model::SpectrumNext => (Some("spectrum_next_candidate"), Some("Generated illustrative candidate of a cased Spectrum Next; shown as a concept, not a specific revision")),
            };
            assert_eq!(descriptor.image_key, expected_image_key);
            assert_eq!(descriptor.image_description, expected_image_description);
            assert_eq!(
                descriptor.expected_main_rom_bytes,
                machine::expected_main_rom_bytes(model)
            );
            assert_eq!(
                descriptor.requires_user_rom,
                machine::requires_user_rom(model)
            );
            assert_eq!(
                descriptor.requires_trdos_rom,
                machine::requires_trdos_rom(model)
            );
            assert_eq!(descriptor.requires_exrom, machine::requires_exrom(model));
            assert_eq!(
                descriptor.compatible_peripherals.plus3_disk,
                model.has_plus3_disk()
            );
            assert_eq!(
                descriptor.compatible_peripherals.timex_dock,
                model == machine::Model::TimexTS2068
            );

            let slots = machine::rom_slot_descriptors(model);
            assert_eq!(descriptor.rom_slots.len(), slots.len());
            for (actual, expected) in descriptor.rom_slots.iter().zip(slots) {
                assert_eq!(actual.id, expected.id);
                assert_eq!(actual.label, expected.label);
                assert_eq!(actual.install_path, expected.install_path);
                assert_eq!(actual.search_paths, expected.search_paths);
                assert_eq!(actual.expected_bytes, expected.expected_bytes);
                assert_eq!(actual.user_provided, expected.user_provided);
            }
        }

        assert_eq!(
            ModelId::ALL.map(ModelId::numeric_id),
            [5, 0, 1, 4, 3, 2, 9, 6, 10, 7, 8, 11]
        );
    }
}
