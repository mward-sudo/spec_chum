//! +2A / +3 gate array: port `1FFD`, special RAM modes, contended banks 4–7.
//!
//! Floating bus is inactive on these machines (unattached reads return `0xFF`).
//! Frame timing matches 128K (228 T/line, 70908 T/frame).

use ula::{contention_delay_128, Ula48, FRAME_TSTATES_128};

use crate::{Ay8912, Keyboard};

/// Contended RAM banks on +2A/+3 (unlike 128K’s 1/3/5/7).
#[inline]
#[must_use]
pub fn is_contended_bank_plus3(bank: usize) -> bool {
    matches!(bank, 4..=7)
}

/// +2A/+3 memory map with `7FFD` + `1FFD` paging.
#[derive(Clone, Debug)]
pub struct BusPlus3 {
    pub rom: [[u8; 16384]; 4],
    pub banks: [[u8; 16384]; 8],
    /// Last `7FFD` write (bits used when not in special RAM mode).
    pub page_7ffd: u8,
    /// Last `1FFD` write.
    pub page_1ffd: u8,
    pub locked: bool,
    pub keyboard: Keyboard,
    pub ear: bool,
    pub beeper: bool,
    pub border: u8,
    pub frame_t: u32,
    pub ay: Ay8912,
    pub beeper_edges: Vec<(u32, bool)>,
    pub ula: Ula48,
    pub kempston: crate::Kempston,
    pub fdc: formats::Plus3Fdc,
}

impl Default for BusPlus3 {
    fn default() -> Self {
        Self::new()
    }
}

impl BusPlus3 {
    #[must_use]
    pub fn new() -> Self {
        Self {
            rom: [[0; 16384]; 4],
            banks: [[0; 16384]; 8],
            page_7ffd: 0,
            page_1ffd: 0,
            locked: false,
            keyboard: Keyboard::new(),
            ear: false,
            beeper: false,
            border: 0,
            frame_t: 0,
            ay: Ay8912::new(),
            beeper_edges: Vec::new(),
            ula: Ula48::new(),
            kempston: crate::Kempston::new(),
            fdc: formats::Plus3Fdc::new(),
        }
    }

    /// Load 64 KiB ROM image (4 × 16K: ROM0..ROM3).
    pub fn load_rom64(&mut self, data: &[u8]) -> Result<(), String> {
        if data.len() != 65536 {
            return Err(format!(
                "+2A/+3 ROM must be 65536 bytes, got {}",
                data.len()
            ));
        }
        for i in 0..4 {
            self.rom[i].copy_from_slice(&data[i * 16384..(i + 1) * 16384]);
        }
        Ok(())
    }

    #[inline]
    #[must_use]
    pub fn special_paging(&self) -> bool {
        self.page_1ffd & 0x01 != 0
    }

    /// ROM selected in normal paging mode (0..3).
    #[must_use]
    pub fn rom_num(&self) -> usize {
        let lo = usize::from(self.page_7ffd & 0x10 != 0);
        let hi = usize::from(self.page_1ffd & 0x04 != 0) << 1;
        hi | lo
    }

    #[inline]
    fn screen_bank(&self) -> usize {
        if self.page_7ffd & 0x08 != 0 {
            7
        } else {
            5
        }
    }

    /// Banks mapped at 0000,4000,8000,C000 when special paging is active.
    #[must_use]
    pub fn special_banks(&self) -> [usize; 4] {
        match (self.page_1ffd >> 1) & 0x03 {
            0 => [0, 1, 2, 3],
            1 => [4, 5, 6, 7],
            2 => [4, 5, 6, 3],
            _ => [4, 7, 6, 3],
        }
    }

    fn bank_at(&self, addr: u16) -> (bool, usize, usize) {
        // returns (is_rom, bank_or_rom_index, offset)
        let off = (addr as usize) & 0x3fff;
        if self.special_paging() {
            let banks = self.special_banks();
            let idx = match addr {
                0x0000..=0x3fff => 0,
                0x4000..=0x7fff => 1,
                0x8000..=0xbfff => 2,
                0xc000..=0xffff => 3,
            };
            (false, banks[idx], off)
        } else {
            match addr {
                0x0000..=0x3fff => (true, self.rom_num(), off),
                0x4000..=0x7fff => (false, 5, off),
                0x8000..=0xbfff => (false, 2, off),
                0xc000..=0xffff => (false, usize::from(self.page_7ffd & 7), off),
            }
        }
    }

    #[must_use]
    pub fn read(&self, addr: u16) -> u8 {
        let (is_rom, bank, off) = self.bank_at(addr);
        if is_rom {
            self.rom[bank][off]
        } else {
            self.banks[bank][off]
        }
    }

    pub fn write(&mut self, addr: u16, value: u8) {
        let (is_rom, bank, off) = self.bank_at(addr);
        if !is_rom {
            self.banks[bank][off] = value;
        }
    }

    #[must_use]
    pub fn screen_bytes(&self) -> &[u8] {
        &self.banks[self.screen_bank()][0..6912]
    }

    #[must_use]
    pub fn contend_at(&self, addr: u16) -> u32 {
        let (is_rom, bank, _) = self.bank_at(addr);
        if is_rom {
            return 0;
        }
        if is_contended_bank_plus3(bank) {
            contention_delay_128(self.frame_t)
        } else {
            0
        }
    }

    pub fn out_7ffd(&mut self, value: u8) {
        if self.locked {
            return;
        }
        self.page_7ffd = value;
        if value & 0x20 != 0 {
            self.locked = true;
        }
        if trace::enabled(trace::Category::BUS) {
            trace::emit(trace::EventKind::BusPort7ffd { value });
        }
    }

    pub fn out_1ffd(&mut self, value: u8) {
        if self.locked {
            return;
        }
        self.page_1ffd = value;
        if trace::enabled(trace::Category::BUS) {
            trace::emit(trace::EventKind::BusPort1ffd { value });
        }
    }

    pub fn in_port(&mut self, port: u16) -> u8 {
        if port & 0xff == 0x1f {
            return self.kempston.read();
        }
        if port & 1 == 0 {
            let keys = self.keyboard.read((port >> 8) as u8);
            let mut v = 0xa0 | keys;
            if self.ear {
                v |= 0x40;
            }
            if trace::enabled(trace::Category::BUS) {
                static FE_IN_N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
                let n = FE_IN_N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if n.is_multiple_of(1024) {
                    trace::emit(trace::EventKind::BusPortFe {
                        write: false,
                        value: v,
                        ear: self.ear,
                    });
                }
            }
            return v;
        }
        // AY register read
        if port & 0xc002 == 0xc000 {
            return self.ay.read_data();
        }
        // FDC data 3FFD — A15=0, A14=0, A13=1, A12=1, A1=0
        if port & 0xf002 == 0x3000 {
            let v = self.fdc.read_data_byte();
            if trace::enabled(trace::Category::DISK) {
                trace::emit(trace::EventKind::DiskFdc {
                    port,
                    write: false,
                    value: v,
                });
            }
            return v;
        }
        // FDC status 2FFD — A15=0, A14=0, A13=1, A12=0, A1=0
        if port & 0xf002 == 0x2000 {
            return if self.fdc.data_remaining() > 0 {
                0xc0 // RQM + DIO
            } else {
                0x80 // RQM
            };
        }
        // No floating bus on +2A/+3
        0xff
    }

    /// Record a speaker edge when the mixed EAR∥beeper level changes.
    pub fn push_speaker_level(&mut self, level: bool) {
        if self.beeper_edges.last().map(|&(_, l)| l) != Some(level) {
            self.beeper_edges.push((self.frame_t, level));
        }
    }

    pub fn out_port(&mut self, port: u16, value: u8) {
        if port & 1 == 0 {
            self.border = value & 7;
            self.ula.set_border(self.frame_t, self.border);
            let beep = value & 0x10 != 0;
            self.beeper = beep;
            self.push_speaker_level(beep || self.ear);
            if trace::enabled(trace::Category::BUS) {
                trace::emit(trace::EventKind::BusPortFe {
                    write: true,
                    value,
                    ear: self.ear,
                });
            }
            if trace::enabled(trace::Category::ULA) {
                trace::emit(trace::EventKind::UlaBorder {
                    color: self.border,
                    frame_t: self.frame_t,
                });
            }
            return;
        }
        // Amstrad +2A/+3 paging (partial decode from FAQ / 128kreference):
        // 1FFD: A15=0, A14=0, A13=0, A12=1, A1=0
        // 7FFD: A15=0, A14=1, A1=0  (tighter than Toastrack 128K)
        // Previous code treated any A13=1 as 1FFD, so OUT 7FFDh (A13=1) corrupted paging.
        if port & 0xf002 == 0x1000 {
            self.out_1ffd(value);
            return;
        }
        if port & 0xc002 == 0x4000 {
            self.out_7ffd(value);
            return;
        }
        // FDC data 3FFD (command bytes ignored until full µPD765 is wired)
        if port & 0xf002 == 0x3000 {
            if trace::enabled(trace::Category::DISK) {
                trace::emit(trace::EventKind::DiskFdc {
                    port,
                    write: true,
                    value,
                });
            }
            let _ = value;
            return;
        }
        if port & 0xc002 == 0xc000 {
            self.ay.select(value);
            if trace::enabled(trace::Category::AY) {
                trace::emit(trace::EventKind::AySelect { reg: value & 0x0f });
            }
            return;
        }
        if port & 0xc002 == 0x8000 {
            let reg = self.ay.selected;
            self.ay.write_data(value);
            if trace::enabled(trace::Category::AY) {
                trace::emit(trace::EventKind::AyWrite { reg, value });
            }
        }
    }

    pub fn advance_frame_t(&mut self, dt: u32) {
        self.frame_t = (self.frame_t + dt) % FRAME_TSTATES_128;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn special_mode_banks() {
        let mut b = BusPlus3::new();
        b.out_1ffd(0x01); // special, config 0
        assert_eq!(b.special_banks(), [0, 1, 2, 3]);
        b.banks[0][0] = 0x11;
        b.banks[3][0] = 0x33;
        assert_eq!(b.read(0x0000), 0x11);
        assert_eq!(b.read(0xc000), 0x33);
        b.out_1ffd(0x01 | (1 << 1)); // config 1 → 4,5,6,7
        assert_eq!(b.special_banks(), [4, 5, 6, 7]);
    }

    #[test]
    fn rom_select_uses_1ffd_and_7ffd() {
        let mut b = BusPlus3::new();
        b.rom[0][0] = 0xa0;
        b.rom[1][0] = 0xa1;
        b.rom[2][0] = 0xa2;
        b.rom[3][0] = 0xa3;
        assert_eq!(b.read(0), 0xa0);
        b.out_7ffd(0x10);
        assert_eq!(b.read(0), 0xa1);
        b.out_1ffd(0x04);
        assert_eq!(b.read(0), 0xa3);
    }

    #[test]
    fn contended_banks_are_4_through_7() {
        let mut b = BusPlus3::new();
        b.frame_t = ula::PAPER_START_128;
        b.out_7ffd(0x04); // bank 4 at C000
        assert_eq!(b.contend_at(0xc000), 6);
        b.out_7ffd(0x01); // bank 1 — not contended on +3
        assert_eq!(b.contend_at(0xc000), 0);
        // 0x4000 is always bank 5 in normal mode → contended
        assert_eq!(b.contend_at(0x4000), 6);
    }

    #[test]
    fn no_floating_bus() {
        let mut b = BusPlus3::new();
        b.banks[5][0] = 0x3c;
        b.frame_t = ula::PAPER_START_128;
        assert_eq!(b.in_port(0x00ff), 0xff);
    }

    #[test]
    fn lock_blocks_both_ports() {
        let mut b = BusPlus3::new();
        b.out_7ffd(0x20);
        assert!(b.locked);
        b.out_7ffd(0x03);
        assert_eq!(b.page_7ffd & 7, 0);
        b.out_1ffd(0x01);
        assert!(!b.special_paging());
    }

    #[test]
    fn out_7ffd_address_does_not_hit_1ffd() {
        let mut b = BusPlus3::new();
        // Real ROM uses OUT (C),A with BC=7FFD. Mis-decoding A13 as 1FFD breaks boot.
        b.out_port(0x7ffd, 0x10); // ROM1 via bit 4
        assert_eq!(b.page_7ffd, 0x10);
        assert_eq!(b.page_1ffd, 0);
        assert!(!b.special_paging());
        b.out_port(0x1ffd, 0x04); // ROM high bit
        assert_eq!(b.page_1ffd, 0x04);
        assert_eq!(b.rom_num(), 3); // hi|lo = 2|1
    }

    #[test]
    fn partial_decode_matches_amstrad_masks() {
        let mut b = BusPlus3::new();
        b.out_port(0x3ffd, 0); // FDC data — must not touch paging
        assert_eq!(b.page_7ffd, 0);
        assert_eq!(b.page_1ffd, 0);
        b.out_port(0x7ffd, 0x05);
        assert_eq!(b.page_7ffd, 0x05);
    }
}
