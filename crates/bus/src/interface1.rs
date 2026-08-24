//! Interface 1 shadow ROM page mechanism + Microdrive MDR slot.

use formats::MdrImage;

/// Typical IF1 shadow ROM size (8K).
pub const IF1_ROM_SIZE: usize = 8192;

#[derive(Clone, Debug)]
pub struct Interface1 {
    pub rom: [u8; IF1_ROM_SIZE],
    pub rom_paged: bool,
    pub rom_loaded: bool,
    pub mdr: Option<MdrImage>,
}

impl Default for Interface1 {
    fn default() -> Self {
        Self::new()
    }
}

impl Interface1 {
    #[must_use]
    pub fn new() -> Self {
        Self {
            rom: [0; IF1_ROM_SIZE],
            rom_paged: false,
            rom_loaded: false,
            mdr: None,
        }
    }

    pub fn load_rom(&mut self, data: &[u8]) -> Result<(), String> {
        if data.len() != IF1_ROM_SIZE {
            return Err(format!(
                "IF1 ROM must be {IF1_ROM_SIZE} bytes, got {}",
                data.len()
            ));
        }
        self.rom.copy_from_slice(data);
        self.rom_loaded = true;
        Ok(())
    }

    pub fn page_rom(&mut self, on: bool) {
        self.rom_paged = on;
    }

    pub fn insert_mdr(&mut self, cart: MdrImage) {
        self.mdr = Some(cart);
    }

    /// Shadow ROM visible at `0000–1FFF` when paged.
    #[must_use]
    pub fn read_rom(&self, addr: u16) -> Option<u8> {
        if !self.rom_paged || addr as usize >= IF1_ROM_SIZE {
            return None;
        }
        Some(self.rom[addr as usize])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mdr_roundtrip_via_if1() {
        let mut if1 = Interface1::new();
        let mut cart = MdrImage::blank();
        cart.write_sector(3, &[0xaa, 0xbb]).unwrap();
        if1.insert_mdr(cart);
        assert_eq!(if1.mdr.as_ref().unwrap().read_sector(3).unwrap()[0], 0xaa);
    }

    #[test]
    fn shadow_rom_page() {
        let mut if1 = Interface1::new();
        if1.rom[0x10] = 0x55;
        assert!(if1.read_rom(0x10).is_none());
        if1.page_rom(true);
        assert_eq!(if1.read_rom(0x10), Some(0x55));
    }
}
