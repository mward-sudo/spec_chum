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
    ModelId::ALL
        .into_iter()
        .map(|id| {
            let model = id.to_model();
            let machine_slots = machine::rom_slot_descriptors(model);
            HostModelDescriptor {
                id: id.numeric_id(),
                preference_slug: PrefModel::from_model_id(id).slug(),
                label: machine::model_label(model),
                title: machine::model_title(model),
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
            [5, 0, 1, 4, 3, 2, 9, 6, 10, 7, 8]
        );
    }
}
