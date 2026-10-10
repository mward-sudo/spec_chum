//! Bundled machine illustrations keyed by the host model catalog.

/// Return the bundled image for a catalog image key.
///
/// The bytes are embedded so native shells can show the picker after packaging,
/// without depending on a source-tree-relative asset path at runtime.
#[must_use]
pub fn machine_image_bytes(key: &str) -> Option<&'static [u8]> {
    match key {
        "spectrum48" => Some(include_bytes!("../../../assets/machines/spectrum48.jpg")),
        "spectrum128" => Some(include_bytes!("../../../assets/machines/spectrum128.jpg")),
        "plus2" => Some(include_bytes!("../../../assets/machines/plus2.jpg")),
        "plus2a_black" => Some(include_bytes!("../../../assets/machines/plus2a_black.jpg")),
        "plus3" => Some(include_bytes!("../../../assets/machines/plus3.jpg")),
        "tc2048" => Some(include_bytes!("../../../assets/machines/tc2048.jpg")),
        "ts2068" => Some(include_bytes!("../../../assets/machines/ts2068.jpg")),
        "pentagon128_candidate" => Some(include_bytes!(
            "../../../assets/machines/pentagon128_candidate.png"
        )),
        "scorpion_zs256_candidate" => Some(include_bytes!(
            "../../../assets/machines/scorpion_zs256_candidate.png"
        )),
        "spectrum_next_candidate" => Some(include_bytes!(
            "../../../assets/machines/spectrum_next_candidate.png"
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::machine_image_bytes;

    #[test]
    fn catalog_image_keys_have_nonempty_embedded_assets() {
        for model in crate::host_model_catalog() {
            let Some(key) = model.image_key else {
                continue;
            };
            let bytes = machine_image_bytes(key)
                .unwrap_or_else(|| panic!("catalog image key {key} has no embedded asset"));
            assert!(!bytes.is_empty(), "embedded asset {key} is empty");
        }
    }
}
