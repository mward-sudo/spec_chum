//! Multiface 1 (48K): 8K ROM + 8K RAM overlay at `0000–3FFF`, paged by the NMI button.
//!
//! Real Multiface ROM images are **not** shipped in-tree — attach via bytes/path like
//! system ROMs. When paged, CPU fetches at the NMI vector (`0x0066`) come from MF ROM.

use std::path::Path;

/// Multiface One memory size (ROM and RAM each).
pub const MULTIFACE1_SIZE: usize = 8192;

/// Romantic Instruments Multiface 1 overlay for 48K machines.
#[derive(Clone, Debug)]
pub struct Multiface1 {
    pub rom: [u8; MULTIFACE1_SIZE],
    pub ram: [u8; MULTIFACE1_SIZE],
    /// True after the red button pages MF in (until hidden).
    pub paged: bool,
    /// Set when a ROM image has been attached (empty ROM is still valid for tests).
    pub rom_loaded: bool,
}

impl Default for Multiface1 {
    fn default() -> Self {
        Self::new()
    }
}

impl Multiface1 {
    #[must_use]
    pub fn new() -> Self {
        Self {
            rom: [0; MULTIFACE1_SIZE],
            ram: [0; MULTIFACE1_SIZE],
            paged: false,
            rom_loaded: false,
        }
    }

    /// Load an 8 KiB Multiface ROM image from memory.
    pub fn load_rom(&mut self, data: &[u8]) -> Result<(), String> {
        if data.len() != MULTIFACE1_SIZE {
            return Err(format!(
                "Multiface 1 ROM must be {MULTIFACE1_SIZE} bytes, got {}",
                data.len()
            ));
        }
        self.rom.copy_from_slice(data);
        self.rom_loaded = true;
        Ok(())
    }

    /// Load Multiface ROM from a filesystem path.
    pub fn load_rom_path(&mut self, path: &Path) -> Result<(), String> {
        let data = std::fs::read(path).map_err(|e| e.to_string())?;
        self.load_rom(&data)
    }

    /// Red button / NMI: page Multiface ROM/RAM over `0000–3FFF` (caller should then raise NMI).
    pub fn nmi(&mut self) {
        self.paged = true;
    }

    /// Alias for [`Self::nmi`].
    pub fn press_button(&mut self) {
        self.nmi();
    }

    /// Hide Multiface memory (restore Spectrum ROM at `0000`).
    pub fn hide(&mut self) {
        self.paged = false;
    }

    /// `OUT` to low byte `0x3F` hides Multiface One (classic port decode).
    #[must_use]
    pub fn out_port(&mut self, port: u16, _value: u8) -> bool {
        if !self.paged {
            return false;
        }
        if port & 0xff == 0x3f {
            self.hide();
            return true;
        }
        false
    }

    /// Read through the MF overlay when paged; `None` if MF does not own `addr`.
    #[must_use]
    pub fn read(&self, addr: u16) -> Option<u8> {
        if !self.paged {
            return None;
        }
        match addr {
            0x0000..=0x1fff => Some(self.rom[addr as usize]),
            0x2000..=0x3fff => Some(self.ram[addr as usize - 0x2000]),
            _ => None,
        }
    }

    /// Write to MF RAM when paged; returns true if handled.
    pub fn write(&mut self, addr: u16, value: u8) -> bool {
        if !self.paged {
            return false;
        }
        if (0x2000..=0x3fff).contains(&addr) {
            self.ram[addr as usize - 0x2000] = value;
            return true;
        }
        // ROM region ignores writes while paged.
        addr < 0x2000
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_pages_rom_and_ram() {
        let mut mf = Multiface1::new();
        mf.rom[0x66] = 0x3e;
        mf.ram[0] = 0;
        assert!(mf.read(0x0066).is_none());
        mf.nmi();
        assert_eq!(mf.read(0x0066), Some(0x3e));
        assert!(mf.write(0x2000, 0xaa));
        assert_eq!(mf.read(0x2000), Some(0xaa));
        mf.hide();
        assert!(mf.read(0x0066).is_none());
    }

    #[test]
    fn out_3f_hides() {
        let mut mf = Multiface1::new();
        mf.nmi();
        assert!(mf.out_port(0x003f, 0));
        assert!(!mf.paged);
    }

    #[test]
    fn load_rom_size_check() {
        let mut mf = Multiface1::new();
        assert!(mf.load_rom(&[0u8; 10]).is_err());
        assert!(mf.load_rom(&[0u8; MULTIFACE1_SIZE]).is_ok());
        assert!(mf.rom_loaded);
    }
}
