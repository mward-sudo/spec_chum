//! Standalone Spectrum Next CPU/MMU core. Host model selection remains gated
//! until firmware and media integration can boot a real distribution (#523).

use bus::NextBus;
use z80::{Cpu, CpuProfile, Io, Memory};

use crate::MachineBuildError;

/// Core-constructible Spectrum Next with Z80N `NEXTREG` instructions and an
/// eight-slot 8 KiB MMU. Video, media and the rest of Next hardware follow in
/// later issue slices; this type is intentionally outside [`crate::Machine`].
#[derive(Clone, Debug)]
pub struct NextMachine {
    pub cpu: Cpu,
    pub bus: NextBus,
}

impl NextMachine {
    /// Supply the 64 KiB system ROM image explicitly; firmware acquisition is
    /// handled by the later boot-path slice, not by core construction.
    pub fn new(rom: &[u8]) -> Result<Self, MachineBuildError> {
        Ok(Self {
            cpu: Cpu::with_profile(CpuProfile::Z80NNextReg),
            bus: NextBus::new(rom)?,
        })
    }

    /// Soft reset the CPU and MMU without erasing physical RAM or ROM.
    pub fn reset(&mut self) {
        self.cpu.reset();
        self.bus.reset();
    }

    /// Execute one guest instruction through the Next-specific bus.
    pub fn step_once(&mut self) -> u32 {
        let mut io = NextMemIo(&mut self.bus);
        self.cpu.step(&mut io)
    }
}

/// Trait adapter keeps the lower-level bus crate independent of the CPU crate.
struct NextMemIo<'a>(&'a mut NextBus);

impl Memory for NextMemIo<'_> {
    fn read(&mut self, addr: u16, _t: u64) -> (u8, u32) {
        (self.0.read(addr), 0)
    }

    fn write(&mut self, addr: u16, value: u8, _t: u64) -> u32 {
        self.0.write(addr, value);
        0
    }
}

impl Io for NextMemIo<'_> {
    fn in_port(&mut self, port: u16, _t: u64) -> (u8, u32) {
        (self.0.in_port(port), 0)
    }

    fn out_port(&mut self, port: u16, value: u8, _t: u64) -> u32 {
        self.0.out_port(port, value);
        0
    }

    fn nextreg_write(&mut self, register: u8, value: u8, _t: u64) {
        self.0.write_nextreg(register, value);
    }
}

#[cfg(test)]
mod tests {
    use bus::NEXT_ROM_SIZE;

    use super::*;

    fn test_rom() -> Vec<u8> {
        vec![0; NEXT_ROM_SIZE]
    }

    #[test]
    fn reset_map_and_slot_boundaries_match_next_defaults() {
        let mut rom = test_rom();
        rom[0] = 0x40;
        rom[0x1fff] = 0x41;
        rom[0x2000] = 0x42;
        rom[0x3fff] = 0x43;
        let mut machine = NextMachine::new(&rom).expect("test ROM has the required size");
        assert_eq!(machine.cpu.profile(), CpuProfile::Z80NNextReg);

        let pages = [0xff, 0xff, 10, 11, 4, 5, 0, 1];
        for (slot, page) in pages.into_iter().enumerate() {
            assert_eq!(machine.bus.read_nextreg(0x50 + slot as u8), page);
            if slot >= 2 {
                let offset = slot as u8;
                assert!(machine.bus.load_ram_page(page, 0, &[offset]));
                assert!(machine.bus.load_ram_page(page, 0x1fff, &[offset | 0x80]));
                assert_eq!(machine.bus.read((slot * 0x2000) as u16), offset);
                assert_eq!(
                    machine.bus.read((slot * 0x2000 + 0x1fff) as u16),
                    offset | 0x80
                );
            }
        }
        assert_eq!(machine.bus.read(0x0000), 0x40);
        assert_eq!(machine.bus.read(0x1fff), 0x41);
        assert_eq!(machine.bus.read(0x2000), 0x42);
        assert_eq!(machine.bus.read(0x3fff), 0x43);
        machine.bus.write(0, 0xaa);
        assert_eq!(machine.bus.read(0), 0x40);

        machine.bus.write_nextreg(0x50, 223);
        machine.bus.write(0, 0x99);
        assert_eq!(machine.bus.read_ram_page(223, 0), Some(0x99));
        machine.reset();
        assert_eq!(machine.bus.read_nextreg(0x50), 0xff);
        assert_eq!(machine.bus.read(0), 0x40);
        assert_eq!(machine.bus.read_ram_page(223, 0), Some(0x99));
    }

    #[test]
    fn guest_fetches_and_reads_writes_after_nextreg_remaps_pages() {
        let mut rom = test_rom();
        // Remap the current instruction slot from ROM to physical RAM page 2.
        rom[..4].copy_from_slice(&[0xed, 0x91, 0x50, 0x02]);
        let mut machine = NextMachine::new(&rom).expect("test ROM has the required size");
        // The instruction at PC=4 is fetched from page 2 after the first step.
        assert!(machine.bus.load_ram_page(
            2,
            4,
            &[
                0x3e, 0xa5, // LD A,A5
                0xed, 0x91, 0x56, 0x03, // NEXTREG 56,03 (slot 6 -> page 3)
                0x32, 0x00, 0xc1, // LD (C100),A
                0x3e, 0x00, // LD A,00
                0x3a, 0x00, 0xc1, // LD A,(C100)
                0x76, // HALT
            ]
        ));
        assert_eq!(machine.step_once(), 20);
        assert_eq!(machine.bus.read_nextreg(0x50), 2);
        for _ in 0..6 {
            machine.step_once();
        }
        assert!(machine.cpu.regs.halted);
        assert_eq!(machine.cpu.regs.a, 0xa5);
        assert_eq!(machine.bus.read_nextreg(0x56), 3);
        assert_eq!(machine.bus.read_ram_page(3, 0x100), Some(0xa5));
        assert_eq!(machine.bus.read_ram_page(0, 0x100), Some(0));
    }

    #[test]
    fn nextreg_instruction_and_register_ports_share_mmu_state() {
        let mut rom = test_rom();
        rom[..22].copy_from_slice(&[
            0xed, 0x91, 0x54, 0x22, // NEXTREG 54,22
            0x01, 0x3b, 0x24, // LD BC,243B
            0x3e, 0x54, // LD A,54
            0xed, 0x79, // OUT (C),A: select register 54
            0x01, 0x3b, 0x25, // LD BC,253B
            0x3e, 0x23, // LD A,23
            0xed, 0x79, // OUT (C),A: page 23
            0xed, 0x78, // IN A,(C): read it back
            0x76, // HALT
            0x00, // padding
        ]);
        let mut machine = NextMachine::new(&rom).expect("test ROM has the required size");
        assert_eq!(machine.step_once(), 20);
        assert_eq!(machine.bus.read_nextreg(0x54), 0x22);
        for _ in 0..8 {
            machine.step_once();
        }
        assert!(machine.cpu.regs.halted);
        assert_eq!(machine.bus.selected_nextreg(), 0x54);
        assert_eq!(machine.bus.read_nextreg(0x54), 0x23);
        assert_eq!(machine.cpu.regs.a, 0x23);
        assert_eq!(machine.bus.in_port(0x253b), 0x23);
    }
}
