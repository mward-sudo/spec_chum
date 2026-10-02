//! Spectrum Next CPU-visible memory and the MMU `NextRegs`.
//!
//! This bus is deliberately separate from the classic 128K buses: each CPU
//! address range has its own 8 KiB page register, including the ROM slots.

use crate::{require_rom_size, RomLoadError};

const PAGE_SIZE: usize = 0x2000;
pub const NEXT_ROM_SIZE: usize = 0x10000;
pub const NEXT_RAM_PAGE_COUNT: usize = 224;
const RESET_PAGES: [u8; 8] = [0xff, 0xff, 10, 11, 4, 5, 0, 1];
const PORT_NEXTREG_SELECT: u16 = 0x243b;
const PORT_NEXTREG_ACCESS: u16 = 0x253b;

/// CPU-visible Spectrum Next memory and the subset of `NextRegs` implemented by
/// this core slice. Unimplemented registers read as an undriven data bus.
#[derive(Clone, Debug)]
pub struct NextBus {
    rom: Box<[u8; NEXT_ROM_SIZE]>,
    ram: Vec<u8>,
    mmu: [u8; 8],
    selected_nextreg: u8,
}

impl NextBus {
    /// Construct a fully expanded 2 MiB Next memory map with a 64 KiB ROM image.
    pub fn new(rom: &[u8]) -> Result<Self, RomLoadError> {
        require_rom_size(rom, "Spectrum Next ROM", NEXT_ROM_SIZE)?;
        let mut rom_bytes = Box::new([0; NEXT_ROM_SIZE]);
        rom_bytes.copy_from_slice(rom);
        Ok(Self {
            rom: rom_bytes,
            ram: vec![0; NEXT_RAM_PAGE_COUNT * PAGE_SIZE],
            mmu: RESET_PAGES,
            selected_nextreg: 0,
        })
    }

    /// Restore soft-reset MMU registers while retaining physical RAM and ROM.
    pub fn reset(&mut self) {
        self.mmu = RESET_PAGES;
        self.selected_nextreg = 0;
    }

    /// Read a CPU-visible address through the current 8 KiB mapping.
    #[must_use]
    pub fn read(&self, addr: u16) -> u8 {
        let slot = usize::from(addr) / PAGE_SIZE;
        let offset = usize::from(addr) % PAGE_SIZE;
        match self.mmu[slot] {
            0xff if slot < 2 => self.rom[slot * PAGE_SIZE + offset],
            page if usize::from(page) < NEXT_RAM_PAGE_COUNT => {
                self.ram[usize::from(page) * PAGE_SIZE + offset]
            }
            _ => 0xff,
        }
    }

    /// Write a CPU-visible address. ROM and absent pages are not writable.
    pub fn write(&mut self, addr: u16, value: u8) {
        let slot = usize::from(addr) / PAGE_SIZE;
        let page = usize::from(self.mmu[slot]);
        if page < NEXT_RAM_PAGE_COUNT {
            let offset = usize::from(addr) % PAGE_SIZE;
            self.ram[page * PAGE_SIZE + offset] = value;
        }
    }

    /// Read one implemented `NextReg`; unimplemented registers float high.
    #[must_use]
    pub fn read_nextreg(&self, register: u8) -> u8 {
        if (0x50..=0x57).contains(&register) {
            self.mmu[usize::from(register - 0x50)]
        } else {
            0xff
        }
    }

    /// Update the MMU `NextRegs`. Other registers are reserved for later slices.
    pub fn write_nextreg(&mut self, register: u8, value: u8) {
        if (0x50..=0x57).contains(&register) {
            self.mmu[usize::from(register - 0x50)] = value;
        }
    }

    /// Read a physical RAM page, independent of its CPU slot mapping.
    #[must_use]
    pub fn read_ram_page(&self, page: u8, offset: usize) -> Option<u8> {
        (usize::from(page) < NEXT_RAM_PAGE_COUNT && offset < PAGE_SIZE)
            .then(|| self.ram[usize::from(page) * PAGE_SIZE + offset])
    }

    /// Load guest bytes into a physical RAM page for core boot tests or media.
    /// Returns false if the page or byte range is outside installed RAM.
    pub fn load_ram_page(&mut self, page: u8, offset: usize, bytes: &[u8]) -> bool {
        let Some(end) = offset.checked_add(bytes.len()) else {
            return false;
        };
        if usize::from(page) >= NEXT_RAM_PAGE_COUNT || end > PAGE_SIZE {
            return false;
        }
        let start = usize::from(page) * PAGE_SIZE + offset;
        self.ram[start..start + bytes.len()].copy_from_slice(bytes);
        true
    }

    /// Select port state is available for debugger inspection.
    #[must_use]
    pub fn selected_nextreg(&self) -> u8 {
        self.selected_nextreg
    }

    /// CPU port read, including the Next register select/access ports.
    #[must_use]
    pub fn in_port(&self, port: u16) -> u8 {
        match port {
            PORT_NEXTREG_SELECT => self.selected_nextreg,
            PORT_NEXTREG_ACCESS => self.read_nextreg(self.selected_nextreg),
            _ => 0xff,
        }
    }

    /// CPU port write, including the Next register select/access ports.
    pub fn out_port(&mut self, port: u16, value: u8) {
        match port {
            PORT_NEXTREG_SELECT => self.selected_nextreg = value,
            PORT_NEXTREG_ACCESS => self.write_nextreg(self.selected_nextreg, value),
            _ => {}
        }
    }
}
