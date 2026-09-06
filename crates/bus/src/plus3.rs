//! +2A / +3 gate array: port `1FFD`, special RAM modes, contended banks 4–7.
//!
//! Floating bus is inactive on these machines (unattached reads return `0xFF`).
//! Frame timing matches 128K (228 T/line, 70908 T/frame).
//!
//! +2A and +3 share this gate array. The ROM detects a disk interface by probing
//! FDC ports (`2FFD`/`3FFD`); when [`BusPlus3::disk_interface`] is false those
//! ports read as `0xFF` so menu **Loader** uses tape (real +2A). When true, the
//! `µPD765` path is present and **Loader** is +3DOS disk (real +3).
//!
//! Port `1FFD` bit 3 is the floppy motor; [`formats::Plus3Fdc::set_motor`] is
//! updated on every `1FFD` write. FDC I/O is not Sinclair ULA-contended.

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
    /// T-states after `OUT #7FFD` before the ULA display bank updates (Amstrad: 2).
    pub screen_switch_delay: u32,
    /// Frame length used when a delayed screen switch spills into the next frame.
    pub frame_tstates: u32,
    /// `(t, bank)` applied at the start of the next frame after `begin_frame`.
    pub pending_screen_switch: Option<(u32, u8)>,
    pub kempston: crate::Kempston,
    pub mouse: crate::KempstonMouse,
    pub fdc: formats::Plus3Fdc,
    /// When false (+2A), FDC ports float at `0xFF` so the ROM treats disk as absent.
    pub disk_interface: bool,
}

impl Default for BusPlus3 {
    fn default() -> Self {
        Self::new()
    }
}

impl BusPlus3 {
    #[must_use]
    pub fn new() -> Self {
        Self::new_with_disk(true)
    }

    /// +3-class bus; `disk_interface` enables `µPD765` ports (`true` = +3, `false` = +2A).
    #[must_use]
    pub fn new_with_disk(disk_interface: bool) -> Self {
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
            screen_switch_delay: 2,
            frame_tstates: FRAME_TSTATES_128,
            pending_screen_switch: None,
            kempston: crate::Kempston::new(),
            mouse: crate::KempstonMouse::new(),
            fdc: formats::Plus3Fdc::new(),
            disk_interface,
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
        let old_screen = self.page_7ffd & 0x08;
        self.page_7ffd = value;
        if value & 0x20 != 0 {
            self.locked = true;
        }
        let new_screen = value & 0x08;
        if old_screen != new_screen {
            let bank = if new_screen != 0 { 7u8 } else { 5 };
            self.schedule_display_screen_bank(bank);
        }
        if trace::enabled(trace::Category::BUS) {
            trace::emit(trace::EventKind::BusPort7ffd { value });
        }
    }

    fn schedule_display_screen_bank(&mut self, bank: u8) {
        let t = self.frame_t.saturating_add(self.screen_switch_delay);
        let fl = self.frame_tstates.max(1);
        if t >= fl {
            self.pending_screen_switch = Some((t - fl, bank));
        } else {
            self.ula.set_display_screen_bank(t, bank);
        }
    }

    /// Apply a screen-bank switch that spilled past the previous frame boundary.
    pub fn apply_pending_screen_switch(&mut self) {
        if let Some((t, bank)) = self.pending_screen_switch.take() {
            self.ula.set_display_screen_bank(t, bank);
        }
    }

    pub fn out_1ffd(&mut self, value: u8) {
        if self.locked {
            return;
        }
        self.page_1ffd = value;
        // Bit 3: floppy motor. Drive-ready (ST3) follows motor ∧ disk inserted.
        self.fdc.set_motor(value & 0x08 != 0);
        if trace::enabled(trace::Category::BUS) {
            trace::emit(trace::EventKind::BusPort1ffd { value });
        }
    }

    pub fn in_port(&mut self, port: u16) -> u8 {
        if let Some(v) = self.mouse.read_port(port) {
            return v;
        }
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
            if !self.disk_interface {
                return 0xff;
            }
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
            if !self.disk_interface {
                return 0xff;
            }
            return self.fdc.main_status();
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
        // FDC data 3FFD — command / parameter bytes
        if port & 0xf002 == 0x3000 {
            if !self.disk_interface {
                return;
            }
            self.fdc.write_command_byte(value);
            if trace::enabled(trace::Category::DISK) {
                trace::emit(trace::EventKind::DiskFdc {
                    port,
                    write: true,
                    value,
                });
            }
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

    #[test]
    fn fdc_read_data_protocol_via_ports() {
        let mut b = BusPlus3::new();
        b.fdc.insert(formats::DskImage::synthetic_one_sector());
        // µPD765 READ DATA: opcode, HD/US, C, H, R, N, EOT, GPL, DTL
        for byte in [0x06u8, 0x00, 0x00, 0x00, 0xc1, 0x01, 0x09, 0x2a, 0xff] {
            b.out_port(0x3ffd, byte);
        }
        assert_eq!(b.in_port(0x2ffd) & 0xc0, 0xc0);
        assert_eq!(b.in_port(0x3ffd), 0x42);
        assert_eq!(b.in_port(0x3ffd), 0x43);
    }

    #[test]
    fn plus2a_fdc_ports_float() {
        let mut b = BusPlus3::new_with_disk(false);
        assert!(!b.disk_interface);
        assert_eq!(b.in_port(0x2ffd), 0xff);
        assert_eq!(b.in_port(0x3ffd), 0xff);
        b.out_port(0x3ffd, 0x06);
        assert_eq!(b.in_port(0x2ffd), 0xff);
    }

    #[test]
    fn plus3_fdc_status_rqm_when_present() {
        let mut b = BusPlus3::new_with_disk(true);
        assert!(b.disk_interface);
        assert_eq!(b.in_port(0x2ffd) & 0x80, 0x80);
    }

    #[test]
    fn fdc_motor_bit_on_1ffd_affects_st3() {
        let mut b = BusPlus3::new();
        b.fdc.insert(formats::DskImage::synthetic_plus3_data());
        // SENSE DRIVE STATUS, unit 0 — motor off → not ready.
        b.out_port(0x3ffd, 0x04);
        b.out_port(0x3ffd, 0x00);
        assert_eq!(b.in_port(0x2ffd) & 0xd0, 0xd0, "result phase");
        let st3 = b.in_port(0x3ffd);
        assert_eq!(st3 & 0x20, 0, "ready bit clear with motor off");

        b.out_port(0x1ffd, 0x08); // motor on
        assert!(b.fdc.motor_on());
        b.out_port(0x3ffd, 0x04);
        b.out_port(0x3ffd, 0x00);
        let st3 = b.in_port(0x3ffd);
        assert_eq!(st3 & 0x20, 0x20, "ready when motor on + disk");

        b.fdc.set_write_protect(true);
        b.out_port(0x3ffd, 0x04);
        b.out_port(0x3ffd, 0x00);
        let st3 = b.in_port(0x3ffd);
        assert_eq!(st3 & 0x40, 0x40, "write-protect in ST3");
    }
}
