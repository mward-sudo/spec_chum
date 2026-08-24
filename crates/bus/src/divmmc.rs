//! DivMMC-style control port + SRAM paging + SD byte store.
//!
//! Port `0xE3` (control): bit7=CONMEM, bit6=MAPRAM, bits0–5=RAM page.
//! Ports `0xE7` / `0xEB` are SPI CS / data stubs over the attached SD image.
//! ESXDOS ROM boot is out of scope — attach optional EEPROM separately later.

/// DivMMC control / paging register.
pub const PORT_CONTROL: u16 = 0x00e3;
/// SPI chip-select (active low bit0 typical); we only track the latch.
pub const PORT_SPI_CS: u16 = 0x00e7;
/// SPI data (byte exchange against SD image cursor).
pub const PORT_SPI_DATA: u16 = 0x00eb;

const PAGE_SIZE: usize = 8192;
/// Foundational SRAM: 16 × 8 KiB pages (enough for paging tests; real cards are larger).
pub const RAM_PAGES: usize = 16;

#[derive(Clone, Debug)]
pub struct DivMmc {
    pub control: u8,
    pub spi_cs: u8,
    /// Optional 8 KiB EEPROM (ESXDOS); zeroed until attached.
    pub eeprom: [u8; PAGE_SIZE],
    pub eeprom_loaded: bool,
    /// Contiguous SRAM (`RAM_PAGES * 8K`).
    pub ram: Vec<u8>,
    /// Flat SD/MMC image.
    pub sd: Vec<u8>,
    /// SPI read/write cursor into `sd`.
    pub sd_cursor: usize,
}

impl Default for DivMmc {
    fn default() -> Self {
        Self::new()
    }
}

impl DivMmc {
    #[must_use]
    pub fn new() -> Self {
        Self {
            control: 0,
            spi_cs: 0xff,
            eeprom: [0; PAGE_SIZE],
            eeprom_loaded: false,
            ram: vec![0u8; RAM_PAGES * PAGE_SIZE],
            sd: Vec::new(),
            sd_cursor: 0,
        }
    }

    pub fn attach_sd(&mut self, data: Vec<u8>) {
        self.sd = data;
        self.sd_cursor = 0;
    }

    pub fn attach_eeprom(&mut self, data: &[u8]) -> Result<(), String> {
        if data.len() != PAGE_SIZE {
            return Err(format!(
                "DivMMC EEPROM must be {PAGE_SIZE} bytes, got {}",
                data.len()
            ));
        }
        self.eeprom.copy_from_slice(data);
        self.eeprom_loaded = true;
        Ok(())
    }

    #[must_use]
    pub fn conmem(&self) -> bool {
        self.control & 0x80 != 0
    }

    #[must_use]
    pub fn mapram(&self) -> bool {
        self.control & 0x40 != 0
    }

    #[must_use]
    pub fn ram_page(&self) -> usize {
        usize::from(self.control & 0x3f) % RAM_PAGES
    }

    pub fn out_port(&mut self, port: u16, value: u8) -> bool {
        match port & 0xff {
            0xe3 => {
                self.control = value;
                true
            }
            PORT_SPI_CS => {
                self.spi_cs = value;
                if value & 1 != 0 {
                    // CS released — reset cursor for next transfer (test convenience).
                    self.sd_cursor = 0;
                }
                true
            }
            PORT_SPI_DATA => {
                // Write byte into SD image at cursor (no-op past EOF).
                if self.sd_cursor < self.sd.len() {
                    self.sd[self.sd_cursor] = value;
                }
                self.sd_cursor = self.sd_cursor.saturating_add(1);
                true
            }
            _ => false,
        }
    }

    pub fn in_port(&mut self, port: u16) -> Option<u8> {
        match port & 0xff {
            0xe3 => Some(self.control),
            PORT_SPI_CS => Some(self.spi_cs),
            PORT_SPI_DATA => {
                let v = self.sd.get(self.sd_cursor).copied().unwrap_or(0xff);
                self.sd_cursor = self.sd_cursor.saturating_add(1);
                Some(v)
            }
            _ => None,
        }
    }

    /// Memory overlay when CONMEM is set: `0000–1FFF` EEPROM (or RAM page 3 if MAPRAM),
    /// `2000–3FFF` selected RAM page.
    #[must_use]
    pub fn read_overlay(&self, addr: u16) -> Option<u8> {
        if !self.conmem() || addr >= 0x4000 {
            return None;
        }
        if addr < 0x2000 {
            if self.mapram() {
                let off = 3 * PAGE_SIZE + addr as usize;
                Some(self.ram[off])
            } else {
                Some(self.eeprom[addr as usize])
            }
        } else {
            let off = self.ram_page() * PAGE_SIZE + (addr as usize - 0x2000);
            Some(self.ram[off])
        }
    }

    pub fn write_overlay(&mut self, addr: u16, value: u8) -> bool {
        if !self.conmem() || addr >= 0x4000 {
            return false;
        }
        if addr < 0x2000 {
            if self.mapram() {
                let off = 3 * PAGE_SIZE + addr as usize;
                self.ram[off] = value;
                true
            } else {
                // EEPROM not writable via CPU.
                true
            }
        } else {
            let off = self.ram_page() * PAGE_SIZE + (addr as usize - 0x2000);
            self.ram[off] = value;
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_port_conmem_shows_ram_page() {
        let mut d = DivMmc::new();
        d.ram[PAGE_SIZE] = 0x5a;
        d.out_port(PORT_CONTROL, 0x80 | 1); // CONMEM + page 1
        assert!(d.conmem());
        assert_eq!(d.ram_page(), 1);
        assert_eq!(d.read_overlay(0x2000), Some(0x5a));
        assert!(d.write_overlay(0x2001, 0x42));
        assert_eq!(d.ram[PAGE_SIZE + 1], 0x42);
    }

    #[test]
    fn mapram_uses_page_3_in_lower_8k() {
        let mut d = DivMmc::new();
        d.ram[3 * PAGE_SIZE + 0x10] = 0x77;
        d.out_port(PORT_CONTROL, 0xc0); // CONMEM|MAPRAM, page 0
        assert_eq!(d.read_overlay(0x0010), Some(0x77));
    }

    #[test]
    fn sd_spi_data_port_reads_image() {
        let mut d = DivMmc::new();
        d.attach_sd(vec![0x11, 0x22, 0x33]);
        d.out_port(PORT_SPI_CS, 0xfe); // CS active
        assert_eq!(d.in_port(PORT_SPI_DATA), Some(0x11));
        assert_eq!(d.in_port(PORT_SPI_DATA), Some(0x22));
    }
}
