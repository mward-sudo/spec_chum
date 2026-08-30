//! Spec Chum bus — 48K/128K/+2A/+3 memory maps and port decode.

#![allow(clippy::pedantic)]

mod ay;
mod beta_disk;
mod divmmc;
mod interface1;
mod kempston;
mod kempston_mouse;
mod multiface;
mod plus3;
mod timex;

pub use ay::{Ay8912, StereoMode};
pub use beta_disk::{BetaDisk, TRDOS_ROM_SIZE};
pub use divmmc::{
    DivMmc, PORT_CONTROL as DIVMMC_PORT_CONTROL, PORT_SPI_CS as DIVMMC_PORT_SPI_CS,
    PORT_SPI_DATA as DIVMMC_PORT_SPI_DATA,
};
pub use interface1::{Interface1, Interface1RomError, IF1_ROM_SIZE, MICRODRIVE_COUNT};
pub use kempston::Kempston;
pub use kempston_mouse::{
    KempstonMouse, PORT_BUTTONS as MOUSE_PORT_BUTTONS, PORT_X as MOUSE_PORT_X,
    PORT_Y as MOUSE_PORT_Y,
};
pub use multiface::{multiface1_port_match, Multiface1, MULTIFACE1_SIZE};
pub use plus3::{is_contended_bank_plus3, BusPlus3};
pub use timex::{timex_joystick_mask, TimexScld, TIMEX_EXROM_SIZE};

use ula::{
    contention_delay, contention_delay_128, floating_bus_byte, floating_bus_byte_128, Ula48,
    FRAME_TSTATES_48,
};

fn emit_floating_sampled(port: u16, frame_t: u32, value: u8) {
    if !trace::enabled(trace::Category::BUS) {
        return;
    }
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if n.is_multiple_of(256) {
        trace::emit(trace::EventKind::BusFloating {
            port,
            frame_t,
            value,
        });
    }
}

/// Keyboard matrix: 8 rows × 5 keys (active low).
#[derive(Clone, Debug)]
pub struct Keyboard {
    /// Bits 0–4 active-low per row; rows selected by A8–A15 of port address.
    pub rows: [u8; 8],
}

impl Default for Keyboard {
    fn default() -> Self {
        Self { rows: [0x1f; 8] }
    }
}

impl Keyboard {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        self.rows = [0x1f; 8];
    }

    /// Set a key pressed (true) or released in Spectrum matrix coordinates.
    pub fn set_key(&mut self, row: usize, bit: u8, pressed: bool) {
        if row >= 8 || bit > 4 {
            return;
        }
        if pressed {
            self.rows[row] &= !(1 << bit);
        } else {
            self.rows[row] |= 1 << bit;
        }
    }

    #[must_use]
    pub fn read(&self, port_hi: u8) -> u8 {
        let mut v = 0x1f;
        for (i, row) in self.rows.iter().enumerate() {
            if port_hi & (1 << i) == 0 {
                v &= row;
            }
        }
        v
    }
}

/// 48K Spectrum memory + ULA port FE.
#[derive(Clone, Debug)]
pub struct Bus48 {
    pub rom: [u8; 16384],
    pub ram: [u8; 49152],
    /// When true, only 16 KiB RAM is mapped at `0x4000..0x8000` (#188).
    pub ram16k: bool,
    /// Timex SCLD ports (#192) — TC2048 and TS2068.
    pub timex: bool,
    /// Timex TS2068 / TC2068: horizontal MMU + EX-ROM + AY on F5/F6.
    pub timex_2068: bool,
    pub timex_scld: TimexScld,
    /// EX-ROM image (8 KiB); mirrored into every paged EX-ROM chunk (Fuse-compatible).
    pub timex_exrom: [u8; TIMEX_EXROM_SIZE],
    /// AY-3-8912 (active on TS2068; unused on plain 48K / TC2048).
    pub ay: Ay8912,
    pub keyboard: Keyboard,
    pub ear: bool,
    pub mic: bool,
    pub beeper: bool,
    pub border: u8,
    /// Frame-relative T-state (0..69888).
    pub frame_t: u32,
    pub ula: Ula48,
    /// Timestamped beeper edges: (frame_t, level).
    pub beeper_edges: Vec<(u32, bool)>,
    pub kempston: Kempston,
    pub mouse: KempstonMouse,
    /// Optional Multiface 1 (ROM attached separately).
    pub multiface: Option<Multiface1>,
    /// Optional DivMMC (control port `0xE3`).
    pub divmmc: Option<DivMmc>,
    /// Optional Interface 1 + Microdrive.
    pub interface1: Option<Interface1>,
    /// Optional Beta Disk / TR-DOS (VG93 ports when paged).
    pub beta: Option<BetaDisk>,
}

impl Default for Bus48 {
    fn default() -> Self {
        Self::new()
    }
}

impl Bus48 {
    #[must_use]
    pub fn new() -> Self {
        Self {
            rom: [0; 16384],
            ram: [0; 49152],
            ram16k: false,
            timex: false,
            timex_2068: false,
            timex_scld: TimexScld::new(),
            timex_exrom: [0xFF; TIMEX_EXROM_SIZE],
            ay: Ay8912::new(),
            keyboard: Keyboard::new(),
            ear: false,
            mic: false,
            beeper: false,
            border: 0,
            frame_t: 0,
            ula: Ula48::new(),
            beeper_edges: Vec::new(),
            kempston: Kempston::new(),
            mouse: KempstonMouse::new(),
            multiface: None,
            divmmc: None,
            interface1: None,
            beta: None,
        }
    }

    pub fn load_rom(&mut self, data: &[u8]) -> Result<(), String> {
        if data.len() != 16384 {
            return Err(format!("48K ROM must be 16384 bytes, got {}", data.len()));
        }
        self.rom.copy_from_slice(data);
        Ok(())
    }

    /// Load the 8 KiB Timex EX-ROM (TS2068 / TC2068).
    pub fn load_timex_exrom(&mut self, data: &[u8]) -> Result<(), String> {
        if data.len() != TIMEX_EXROM_SIZE {
            return Err(format!(
                "Timex EX-ROM must be {TIMEX_EXROM_SIZE} bytes, got {}",
                data.len()
            ));
        }
        self.timex_exrom.copy_from_slice(data);
        Ok(())
    }

    /// Horizontal MMU overlay: EX-ROM or empty DOCK when HSR bit is set.
    #[inline]
    fn timex_mmu_read(&self, addr: u16) -> Option<u8> {
        if !self.timex_2068 {
            return None;
        }
        let chunk = TimexScld::chunk_of(addr);
        if !self.timex_scld.chunk_paged(chunk) {
            return None;
        }
        if self.timex_scld.use_exrom() {
            Some(self.timex_exrom[(addr as usize) & (TIMEX_EXROM_SIZE - 1)])
        } else {
            // Empty dock / no cartridge — Fuse fills 0xFF.
            Some(0xFF)
        }
    }

    #[inline]
    fn timex_mmu_blocks_write(&self, addr: u16) -> bool {
        self.timex_2068 && self.timex_scld.chunk_paged(TimexScld::chunk_of(addr))
    }

    /// Attach Multiface 1 with an 8 KiB ROM image (creates the peripheral if absent).
    pub fn attach_multiface(&mut self, rom: &[u8]) -> Result<(), String> {
        let mut mf = Multiface1::new();
        mf.load_rom(rom)?;
        self.multiface = Some(mf);
        Ok(())
    }

    /// Attach a DivMMC (creates default peripheral if absent).
    pub fn attach_divmmc(&mut self) -> &mut DivMmc {
        self.divmmc.get_or_insert_with(DivMmc::new)
    }

    /// M1 opcode-fetch hook for DivMMC automap.
    pub fn notify_divmmc_m1(&mut self, pc: u16) {
        if let Some(d) = self.divmmc.as_mut() {
            d.notify_m1(pc);
        }
    }

    /// Attach Interface 1 (creates default peripheral if absent).
    pub fn attach_interface1(&mut self) -> &mut Interface1 {
        self.interface1.get_or_insert_with(Interface1::new)
    }

    /// Attach Beta Disk / TR-DOS (creates default peripheral if absent).
    pub fn attach_beta(&mut self) -> &mut BetaDisk {
        self.beta.get_or_insert_with(BetaDisk::new)
    }

    /// M1 paging for an attached Beta / TR-DOS ROM (48K window `0x3C00–0x3DFF`).
    pub fn notify_beta_m1(&mut self, pc: u16) {
        if let Some(beta) = self.beta.as_mut() {
            beta.notify_m1(pc, 0x3c00);
        }
    }

    #[inline]
    #[must_use]
    pub fn is_contended(addr: u16) -> bool {
        (0x4000..0x8000).contains(&addr)
    }

    #[inline]
    #[must_use]
    pub fn contend_at(&self, addr: u16) -> u32 {
        if Self::is_contended(addr) {
            contention_delay(self.frame_t)
        } else {
            0
        }
    }

    #[inline]
    #[must_use]
    pub fn read(&self, addr: u16) -> u8 {
        // Multiface NMI overlay wins over DivMMC / IF1 when paged (button press).
        if let Some(mf) = self.multiface.as_ref() {
            if let Some(v) = mf.read(addr) {
                return v;
            }
        }
        if let Some(d) = self.divmmc.as_ref() {
            if let Some(v) = d.read_overlay(addr) {
                return v;
            }
        }
        if let Some(if1) = self.interface1.as_ref() {
            if let Some(v) = if1.read_rom(addr) {
                return v;
            }
        }
        if let Some(beta) = self.beta.as_ref() {
            if let Some(v) = beta.read_rom(addr) {
                return v;
            }
        }
        if let Some(v) = self.timex_mmu_read(addr) {
            return v;
        }
        if addr < 0x4000 {
            self.rom[addr as usize]
        } else if self.ram16k && addr >= 0x8000 {
            0xFF
        } else {
            self.ram[addr as usize - 0x4000]
        }
    }

    #[inline]
    pub fn write(&mut self, addr: u16, value: u8) {
        if let Some(mf) = self.multiface.as_mut() {
            if mf.write(addr, value) {
                return;
            }
        }
        if let Some(d) = self.divmmc.as_mut() {
            if d.write_overlay(addr, value) {
                return;
            }
        }
        // Paged EX-ROM / empty DOCK are not writable; ULA screen still uses home RAM.
        if self.timex_mmu_blocks_write(addr) {
            return;
        }
        if addr >= 0x4000 && !(self.ram16k && addr >= 0x8000) {
            self.ram[addr as usize - 0x4000] = value;
        }
    }

    /// Screen bitmap+attrs live at 0x4000 in RAM bank.
    #[must_use]
    pub fn screen_bytes(&self) -> &[u8] {
        &self.ram[0..6912]
    }

    pub fn in_fe(&self, port: u16) -> u8 {
        let keys = self.keyboard.read((port >> 8) as u8);
        let mut v = 0xa0 | keys; // bits 7,5 often 1; bit 6 = EAR
        if self.ear {
            v |= 0x40;
        }
        // Floating bus when ULA not driving keyboard-only — classic: odd ports
        if port & 1 != 0 {
            // Fully decoded FE only when A0=0
        }
        if trace::enabled(trace::Category::BUS) {
            // Subsample FE polls — ROM LD-BYTES hammers this port.
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
        v
    }

    pub fn out_fe(&mut self, value: u8) {
        self.border = value & 7;
        self.ula.set_border(self.frame_t, self.border);
        let beep = value & 0x10 != 0;
        self.beeper = beep;
        // Mix MIC/beeper with EAR so tape load tones reach the speaker path.
        self.push_speaker_level(beep || self.ear);
        self.mic = value & 0x08 != 0;
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
    }

    /// Record a speaker edge when the mixed EAR∥beeper level changes.
    pub fn push_speaker_level(&mut self, level: bool) {
        if self.beeper_edges.last().map(|&(_, l)| l) != Some(level) {
            self.beeper_edges.push((self.frame_t, level));
        }
    }

    pub fn in_port(&mut self, port: u16) -> u8 {
        // Multiface 1: matching IN pages by A7 as a side effect (MAME-style: does not
        // consume the expansion-bus cycle). Prefer Beta/DivMMC data when they claim.
        let mf_data = if let Some(mf) = self.multiface.as_mut() {
            let joy = self.kempston.read();
            mf.in_port(port, joy)
        } else {
            None
        };
        if let Some(beta) = self.beta.as_mut() {
            if let Some(v) = beta.in_port(port) {
                return v;
            }
        }
        // DivMMC before IF1: both claim low bytes 0xE3/0xE7/0xEB.
        if let Some(d) = self.divmmc.as_mut() {
            if let Some(v) = d.in_port(port) {
                return v;
            }
        }
        if let Some(v) = mf_data {
            return v;
        }
        if let Some(if1) = self.interface1.as_mut() {
            if let Some(v) = if1.in_port(port) {
                return v;
            }
        }
        // Kempston joystick (partial decode on low byte 0x1f) when Beta/MF not claiming it
        if port & 0xff == 0x1f {
            return self.kempston.read();
        }
        if let Some(v) = self.mouse.read_port(port) {
            return v;
        }
        if self.timex_2068 {
            match port & 0xff {
                // Register select is write-only (WoS / Fuse: IN F5 → 0xFF).
                0xf5 => return 0xff,
                // Data port: normal AY read, or Timex joysticks when R14 selected.
                0xf6 => {
                    if self.ay.selected != 14 {
                        return self.ay.read_data();
                    }
                    // R7 bit 6 = Port A direction (1 = output → return latched R14).
                    if self.ay.regs[7] & 0x40 != 0 {
                        return self.ay.regs[14];
                    }
                    let mut ret = 0xffu8;
                    // Active-high press bits AND-NOT into the active-low read (Fuse).
                    // Host Kempston is remapped onto both Timex sticks for Phase 2a.
                    let joy = timex::timex_joystick_mask(
                        self.kempston.up,
                        self.kempston.down,
                        self.kempston.left,
                        self.kempston.right,
                        self.kempston.fire,
                    );
                    if port & 0x0100 != 0 {
                        ret &= !joy;
                    }
                    if port & 0x0200 != 0 {
                        ret &= !joy;
                    }
                    return ret;
                }
                _ => {}
            }
        }
        if self.timex {
            if let Some(v) = self.timex_scld.in_port(port) {
                return v;
            }
        }
        if port & 1 == 0 {
            return self.in_fe(port);
        }
        // Floating bus on unattached ports
        let v = floating_bus_byte(self.frame_t, self.screen_bytes()).unwrap_or(0xff);
        emit_floating_sampled(port, self.frame_t, v);
        v
    }

    pub fn out_port(&mut self, port: u16, value: u8) {
        // Multiface clears NMI pending on matching OUT but does not consume the cycle
        // (Beta / other peripherals still see the same port write).
        if let Some(mf) = self.multiface.as_mut() {
            let _ = mf.out_port(port, value);
        }
        if let Some(beta) = self.beta.as_mut() {
            if beta.out_port(port, value) {
                return;
            }
        }
        // DivMMC before IF1: both claim low bytes 0xE3/0xE7/0xEB.
        if let Some(d) = self.divmmc.as_mut() {
            if d.out_port(port, value) {
                return;
            }
        }
        if let Some(if1) = self.interface1.as_mut() {
            if if1.out_port(port, value) {
                return;
            }
        }
        if self.timex_2068 {
            match port & 0xff {
                0xf5 => {
                    self.ay.select(value);
                    return;
                }
                0xf6 => {
                    self.ay.write_data(value);
                    return;
                }
                _ => {}
            }
        }
        if self.timex && self.timex_scld.out_port(port, value) {
            return;
        }
        if port & 1 == 0 {
            self.out_fe(value);
        }
    }

    pub fn advance_frame_t(&mut self, dt: u32) {
        self.frame_t = (self.frame_t + dt) % FRAME_TSTATES_48;
    }
}

/// 128K bus with 7FFD paging + AY ports.
#[derive(Clone, Debug)]
pub struct Bus128 {
    pub rom: [[u8; 16384]; 2],
    pub banks: [[u8; 16384]; 8],
    pub page: u8,
    pub locked: bool,
    pub keyboard: Keyboard,
    pub ear: bool,
    pub beeper: bool,
    pub border: u8,
    pub frame_t: u32,
    pub ay: Ay8912,
    pub beeper_edges: Vec<(u32, bool)>,
    pub ula: Ula48,
    pub kempston: Kempston,
    pub mouse: KempstonMouse,
    pub divmmc: Option<DivMmc>,
    pub interface1: Option<Interface1>,
    pub beta: Option<BetaDisk>,
}

impl Default for Bus128 {
    fn default() -> Self {
        Self::new()
    }
}

impl Bus128 {
    #[must_use]
    pub fn new() -> Self {
        Self {
            rom: [[0; 16384]; 2],
            banks: [[0; 16384]; 8],
            page: 0,
            locked: false,
            keyboard: Keyboard::new(),
            ear: false,
            beeper: false,
            border: 0,
            frame_t: 0,
            ay: Ay8912::new(),
            beeper_edges: Vec::new(),
            ula: Ula48::new(),
            kempston: Kempston::new(),
            mouse: KempstonMouse::new(),
            divmmc: None,
            interface1: None,
            beta: None,
        }
    }

    /// Attach a DivMMC (creates default peripheral if absent).
    pub fn attach_divmmc(&mut self) -> &mut DivMmc {
        self.divmmc.get_or_insert_with(DivMmc::new)
    }

    /// M1 opcode-fetch hook for DivMMC automap.
    pub fn notify_divmmc_m1(&mut self, pc: u16) {
        if let Some(d) = self.divmmc.as_mut() {
            d.notify_m1(pc);
        }
    }

    pub fn attach_interface1(&mut self) -> &mut Interface1 {
        self.interface1.get_or_insert_with(Interface1::new)
    }

    pub fn attach_beta(&mut self) -> &mut BetaDisk {
        self.beta.get_or_insert_with(BetaDisk::new)
    }

    /// M1 paging for an attached Beta / TR-DOS ROM (128K window `0x3D00–0x3DFF`).
    pub fn notify_beta_m1(&mut self, pc: u16) {
        if let Some(beta) = self.beta.as_mut() {
            beta.notify_m1(pc, 0x3d00);
        }
    }

    /// Compatibility: selected AY register index.
    #[must_use]
    pub fn ay_reg(&self) -> u8 {
        self.ay.selected
    }

    /// Compatibility: raw AY register file.
    #[must_use]
    pub fn ay_regs(&self) -> &[u8; 16] {
        &self.ay.regs
    }

    pub fn load_rom128(&mut self, data: &[u8]) -> Result<(), String> {
        if data.len() != 32768 {
            return Err(format!("128 ROM must be 32768 bytes, got {}", data.len()));
        }
        self.rom[0].copy_from_slice(&data[0..16384]);
        self.rom[1].copy_from_slice(&data[16384..32768]);
        Ok(())
    }

    #[inline]
    fn rom_num(&self) -> usize {
        usize::from(self.page & 0x10 != 0)
    }

    #[inline]
    fn screen_bank(&self) -> usize {
        if self.page & 0x08 != 0 {
            7
        } else {
            5
        }
    }

    #[inline]
    fn paged_bank(&self) -> usize {
        usize::from(self.page & 7)
    }

    #[must_use]
    pub fn read(&self, addr: u16) -> u8 {
        if let Some(d) = self.divmmc.as_ref() {
            if let Some(v) = d.read_overlay(addr) {
                return v;
            }
        }
        if let Some(if1) = self.interface1.as_ref() {
            if let Some(v) = if1.read_rom(addr) {
                return v;
            }
        }
        if let Some(beta) = self.beta.as_ref() {
            if let Some(v) = beta.read_rom(addr) {
                return v;
            }
        }
        match addr {
            0x0000..=0x3fff => self.rom[self.rom_num()][addr as usize],
            0x4000..=0x7fff => self.banks[5][addr as usize - 0x4000],
            0x8000..=0xbfff => self.banks[2][addr as usize - 0x8000],
            0xc000..=0xffff => self.banks[self.paged_bank()][addr as usize - 0xc000],
        }
    }

    pub fn write(&mut self, addr: u16, value: u8) {
        if let Some(d) = self.divmmc.as_mut() {
            if d.write_overlay(addr, value) {
                return;
            }
        }
        match addr {
            0x0000..=0x3fff => {}
            0x4000..=0x7fff => self.banks[5][addr as usize - 0x4000] = value,
            0x8000..=0xbfff => self.banks[2][addr as usize - 0x8000] = value,
            0xc000..=0xffff => self.banks[self.paged_bank()][addr as usize - 0xc000] = value,
        }
    }

    #[must_use]
    pub fn screen_bytes(&self) -> &[u8] {
        &self.banks[self.screen_bank()][0..6912]
    }

    #[inline]
    #[must_use]
    pub fn is_contended_bank(bank: usize) -> bool {
        matches!(bank, 1 | 3 | 5 | 7)
    }

    /// True when the RAM bank currently at `0xC000` is contended (1/3/5/7).
    #[must_use]
    pub fn c000_contended(&self) -> bool {
        Self::is_contended_bank(self.paged_bank())
    }

    #[must_use]
    pub fn contend_at(&self, addr: u16) -> u32 {
        let contended = match addr {
            0x4000..=0x7fff => true,
            0xc000..=0xffff => Self::is_contended_bank(self.paged_bank()),
            _ => false,
        };
        if contended {
            contention_delay_128(self.frame_t)
        } else {
            0
        }
    }

    pub fn out_7ffd(&mut self, value: u8) {
        if self.locked {
            return;
        }
        self.page = value;
        if value & 0x20 != 0 {
            self.locked = true;
        }
        if trace::enabled(trace::Category::BUS) {
            trace::emit(trace::EventKind::BusPort7ffd { value });
        }
    }

    pub fn in_port(&mut self, port: u16) -> u8 {
        if let Some(beta) = self.beta.as_mut() {
            if let Some(v) = beta.in_port(port) {
                return v;
            }
        }
        // DivMMC before IF1: both claim low bytes 0xE3/0xE7/0xEB.
        if let Some(d) = self.divmmc.as_mut() {
            if let Some(v) = d.in_port(port) {
                return v;
            }
        }
        if let Some(if1) = self.interface1.as_mut() {
            if let Some(v) = if1.in_port(port) {
                return v;
            }
        }
        if port & 0xff == 0x1f {
            return self.kempston.read();
        }
        if let Some(v) = self.mouse.read_port(port) {
            return v;
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
        let v = floating_bus_byte_128(self.frame_t, self.screen_bytes()).unwrap_or(0xff);
        emit_floating_sampled(port, self.frame_t, v);
        v
    }

    /// Record a speaker edge when the mixed EAR∥beeper level changes.
    pub fn push_speaker_level(&mut self, level: bool) {
        if self.beeper_edges.last().map(|&(_, l)| l) != Some(level) {
            self.beeper_edges.push((self.frame_t, level));
        }
    }

    pub fn out_port(&mut self, port: u16, value: u8) {
        if let Some(beta) = self.beta.as_mut() {
            if beta.out_port(port, value) {
                return;
            }
        }
        // DivMMC before IF1: both claim low bytes 0xE3/0xE7/0xEB.
        if let Some(d) = self.divmmc.as_mut() {
            if d.out_port(port, value) {
                return;
            }
        }
        if let Some(if1) = self.interface1.as_mut() {
            if if1.out_port(port, value) {
                return;
            }
        }
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
        // 7FFD: A15=0, A1=0
        if port & 0x8002 == 0 {
            self.out_7ffd(value);
            return;
        }
        // FFFD select / BFFD data
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rom_ram_map() {
        let mut b = Bus48::new();
        b.rom[0] = 0xAA;
        b.write(0x4000, 0x55);
        assert_eq!(b.read(0), 0xAA);
        assert_eq!(b.read(0x4000), 0x55);
        b.write(0x0000, 0x11);
        assert_eq!(b.read(0), 0xAA, "ROM not writable");
    }

    #[test]
    fn ram16k_maps_only_low_ram() {
        let mut b = Bus48::new();
        b.ram16k = true;
        b.write(0x4000, 0x12);
        b.write(0x7fff, 0x34);
        b.write(0x8000, 0x56);
        assert_eq!(b.read(0x4000), 0x12);
        assert_eq!(b.read(0x7fff), 0x34);
        assert_eq!(b.read(0x8000), 0xFF, "16K RAM does not decode above 0x7FFF");
        assert_eq!(b.ram[0], 0x12);
        assert_eq!(b.ram[0x3fff], 0x34);
        assert_eq!(b.ram[0x4000], 0, "writes above 16K RAM are ignored");
    }

    #[test]
    fn timex_2068_exrom_pages_chunk0() {
        let mut b = Bus48::new();
        b.timex = true;
        b.timex_2068 = true;
        b.rom[0] = 0x11;
        b.rom[0x2000] = 0x22;
        b.timex_exrom[0] = 0xAA;
        b.timex_exrom[1] = 0xBB;
        assert_eq!(b.read(0x0000), 0x11, "home ROM before paging");
        b.out_port(0x00FF, 0x80); // EX-ROM
        b.out_port(0x00F4, 0x01); // chunk 0
        assert_eq!(b.read(0x0000), 0xAA);
        assert_eq!(b.read(0x0001), 0xBB);
        assert_eq!(b.read(0x2000), 0x22, "chunk 1 still home when bit clear");
        // Same EX-ROM mirrored when chunk 1 is also selected.
        b.out_port(0x00F4, 0x03);
        assert_eq!(b.read(0x2000), 0xAA);
        b.write(0x0000, 0x55);
        assert_eq!(b.read(0x0000), 0xAA, "EX-ROM not writable");
    }

    #[test]
    fn timex_2068_empty_dock_reads_ff() {
        let mut b = Bus48::new();
        b.timex = true;
        b.timex_2068 = true;
        b.rom[0] = 0x11;
        b.out_port(0x00FF, 0x00); // DOCK
        b.out_port(0x00F4, 0x01);
        assert_eq!(b.read(0x0000), 0xFF);
    }

    #[test]
    fn timex_2068_ay_ports_f5_f6() {
        let mut b = Bus48::new();
        b.timex = true;
        b.timex_2068 = true;
        b.out_port(0x00F5, 0x07);
        b.out_port(0x00F6, 0x38);
        assert_eq!(b.ay.selected, 0x07);
        assert_eq!(b.ay.regs[7], 0x38);
        // Register select is write-only.
        assert_eq!(b.in_port(0x00F5), 0xFF);
        assert_eq!(b.in_port(0x00F6), 0x38);
    }

    #[test]
    fn timex_2068_ay_r14_joysticks_via_f6() {
        let mut b = Bus48::new();
        b.timex = true;
        b.timex_2068 = true;
        // Select R14; R7 bit 6 clear → Port A input (joysticks).
        b.out_port(0x00F5, 14);
        b.out_port(0x00F6, 0); // unused; ensure R14 latched low when output later
        b.out_port(0x00F5, 7);
        b.out_port(0x00F6, 0x00); // bit 6 clear
        b.out_port(0x00F5, 14);

        assert_eq!(b.in_port(0x00F5), 0xFF, "IN F5 always floating");
        assert_eq!(b.in_port(0x00F6), 0xFF, "no stick strobe → idle high");

        b.kempston.up = true;
        b.kempston.fire = true;
        // Fuse Timex mask: up=0x01, fire=0x80 → active-low read clears those bits.
        assert_eq!(b.in_port(0x01F6), !0x81u8, "left stick (A8)");
        assert_eq!(b.in_port(0x02F6), !0x81u8, "right stick (A9)");
        assert_eq!(b.in_port(0x00F6), 0xFF, "neither strobe");

        // R7 bit 6 set → Port A output; return latched R14, ignore sticks.
        b.out_port(0x00F5, 7);
        b.out_port(0x00F6, 0x40);
        b.out_port(0x00F5, 14);
        b.out_port(0x00F6, 0xFF);
        assert_eq!(b.in_port(0x01F6), 0xFF, "output direction ignores sticks");
    }

    #[test]
    fn keyboard_row() {
        let mut k = Keyboard::new();
        k.set_key(0, 0, true); // Caps shift row
        assert_eq!(k.read(0xfe), 0x1e);
    }

    #[test]
    fn page_7ffd() {
        let mut b = Bus128::new();
        b.banks[3][0] = 0x42;
        b.out_7ffd(0x03);
        assert_eq!(b.read(0xc000), 0x42);
        b.out_7ffd(0x20); // lock
        b.out_7ffd(0x00);
        assert_eq!(b.page & 7, 0); // still locked at previous? lock after write
                                   // actually we locked with bank 0 from 0x20
        assert!(b.locked);
    }

    #[test]
    fn contend_128_differs_from_48_at_paper_start() {
        let mut b = Bus128::new();
        b.frame_t = ula::PAPER_START_128;
        assert_eq!(b.contend_at(0x4000), 6);
        b.frame_t = ula::PAPER_START_48;
        assert_eq!(b.contend_at(0x4000), 0);
    }

    #[test]
    fn kempston_port_1f() {
        let mut b = Bus48::new();
        b.kempston.fire = true;
        b.kempston.right = true;
        assert_eq!(b.in_port(0x001f), 0x11);
    }

    #[test]
    fn kempston_mouse_ports_after_delta_and_buttons() {
        let mut b = Bus48::new();
        b.mouse.set_delta(20, -4);
        b.mouse.set_buttons(true, true, false);
        assert_eq!(b.in_port(MOUSE_PORT_X), 20);
        assert_eq!(b.in_port(MOUSE_PORT_Y), 4);
        assert_eq!(b.in_port(MOUSE_PORT_BUTTONS), 0xfc); // D0+D1 clear
    }

    #[test]
    fn multiface_nmi_overlays_synthetic_rom() {
        let mut b = Bus48::new();
        b.rom[0x66] = 0x11; // Spectrum ROM at NMI vector
        let mut mf_rom = [0u8; MULTIFACE1_SIZE];
        mf_rom[0x66] = 0xc3; // JP …
        b.attach_multiface(&mf_rom).unwrap();
        assert_eq!(b.read(0x0066), 0x11);
        let mf = b.multiface.as_mut().unwrap();
        mf.nmi();
        mf.page_on_nmi_vector();
        assert_eq!(b.read(0x0066), 0xc3);
        b.write(0x2000, 0x5a);
        assert_eq!(b.read(0x2000), 0x5a);
        assert_eq!(b.in_port(0x001f), 0, "IN 1Fh pages out");
        assert_eq!(b.read(0x0066), 0x11, "IN 1Fh hides Multiface");
    }

    #[test]
    fn multiface_in_9f_pages_back_in() {
        let mut b = Bus48::new();
        b.rom[0] = 0x11;
        let mut mf_rom = [0u8; MULTIFACE1_SIZE];
        mf_rom[0] = 0xaa;
        b.attach_multiface(&mf_rom).unwrap();
        b.multiface.as_mut().unwrap().page_in();
        assert_eq!(b.read(0x0000), 0xaa);
        let _ = b.in_port(0x001f);
        assert_eq!(b.read(0x0000), 0x11);
        let _ = b.in_port(0x009f);
        assert_eq!(b.read(0x0000), 0xaa, "IN 9Fh pages Multiface back in");
    }

    #[test]
    fn multiface_kempston_on_in_1f_while_attached() {
        let mut b = Bus48::new();
        b.attach_multiface(&[0u8; MULTIFACE1_SIZE]).unwrap();
        b.kempston.fire = true;
        b.kempston.right = true;
        b.multiface.as_mut().unwrap().page_in();
        assert_eq!(b.in_port(0x001f), 0x11);
        assert!(!b.multiface.as_ref().unwrap().paged);
    }

    #[test]
    fn multiface_and_beta_share_port_1f_cycle() {
        // MF1 side effects chain with the expansion bus; Beta keeps data/command
        // priority when TR-DOS is paged (MAME mface1 + interface style).
        let mut raw = vec![0u8; formats::TRD_SECTOR_SIZE * formats::TRD_SECTORS_PER_TRACK];
        raw[0] = 0x12;
        raw[1] = 0x34;
        let img = formats::TrdImage::parse(&raw).unwrap();
        let mut b = Bus48::new();
        b.attach_multiface(&[0u8; MULTIFACE1_SIZE]).unwrap();
        {
            let mf = b.multiface.as_mut().unwrap();
            mf.page_in();
            mf.nmi_pending = true;
        }
        let beta = b.attach_beta();
        beta.insert(img);
        beta.page_trdos(true);

        b.out_port(0x003f, 0);
        b.out_port(0x005f, 1);
        b.out_port(0x001f, 0x80); // read sector — MF clears NMI; Beta still gets command
        assert!(
            !b.multiface.as_ref().unwrap().nmi_pending,
            "OUT 1Fh clears MF NMI pending"
        );
        assert!(
            b.multiface.as_ref().unwrap().paged,
            "OUT does not page Multiface out"
        );

        b.kempston.fire = true;
        assert_eq!(
            b.in_port(0x001f),
            0x02,
            "IN 1Fh returns Beta status, not Kempston"
        );
        assert!(
            !b.multiface.as_ref().unwrap().paged,
            "IN 1Fh still pages Multiface out as a side effect"
        );
        assert_eq!(b.in_port(0x007f), 0x12);
    }

    #[test]
    fn multiface_nmi_beats_divmmc_conmem() {
        let mut b = Bus48::new();
        b.rom[0x66] = 0x11;
        let d = b.attach_divmmc();
        d.eeprom[0x66] = 0x77;
        d.out_port(DIVMMC_PORT_CONTROL, 0x80); // CONMEM
        let mut mf_rom = [0u8; MULTIFACE1_SIZE];
        mf_rom[0x66] = 0xc3;
        b.attach_multiface(&mf_rom).unwrap();
        let mf = b.multiface.as_mut().unwrap();
        mf.nmi();
        mf.page_on_nmi_vector();
        assert_eq!(
            b.read(0x0066),
            0xc3,
            "Multiface NMI must win over DivMMC CONMEM"
        );
    }

    #[test]
    fn divmmc_eeprom_fixture_automaps_when_present() {
        // Optional local fixture — not committed. Place ≥8 KiB ESXDOS at
        // `roms/esxdos.rom` or `roms/divmmc.rom` to exercise real-image automap.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let path = ["roms/esxdos.rom", "roms/divmmc.rom"]
            .into_iter()
            .map(|rel| root.join(rel))
            .find(|p| p.is_file());
        let Some(path) = path else {
            eprintln!("skipping: no roms/esxdos.rom or roms/divmmc.rom");
            return;
        };
        let data = std::fs::read(path).expect("read eeprom fixture");
        let mut b = Bus48::new();
        let d = b.attach_divmmc();
        d.attach_eeprom(&data).expect("attach eeprom");
        b.notify_divmmc_m1(0x0000);
        assert!(b.divmmc.as_ref().unwrap().automap);
        assert_eq!(b.read(0x0000), data[0]);
    }

    #[test]
    fn divmmc_automap_via_notify_m1() {
        let mut b = Bus48::new();
        let d = b.attach_divmmc();
        d.attach_eeprom(&[0x5au8; 8192]).unwrap();
        let rom0 = b.rom[0];
        assert_eq!(b.read(0x0000), rom0);
        b.notify_divmmc_m1(0x0008);
        assert_eq!(b.read(0x0000), 0x5a);
    }

    #[test]
    fn divmmc_conmem_overlays_via_bus48() {
        let mut b = Bus48::new();
        b.rom[0] = 0x11;
        let d = b.attach_divmmc();
        d.ram[0] = 0x5a; // page 0 @ 0x2000 when CONMEM
        d.out_port(DIVMMC_PORT_CONTROL, 0x80); // CONMEM, page 0
        assert_eq!(b.read(0x2000), 0x5a);
        b.write(0x2001, 0x42);
        assert_eq!(b.divmmc.as_ref().unwrap().ram[1], 0x42);
        b.divmmc.as_mut().unwrap().eeprom[0] = 0x77;
        assert_eq!(b.read(0x0000), 0x77);
    }

    #[test]
    fn interface1_microdrive_ports_via_bus48() {
        let mut b = Bus48::new();
        let if1 = b.attach_interface1();
        let mut cart = formats::MdrImage::blank();
        cart.sectors[0][0] = 0x42;
        if1.insert_mdr(cart);
        b.out_port(0x00ef, 0x02);
        b.out_port(0x00ef, 0x00);
        let _ = b.in_port(0x00ef);
        assert_eq!(b.in_port(0x00e7), 0x42);
    }

    #[test]
    fn interface1_shadow_rom_mirror_via_bus48() {
        let mut b = Bus48::new();
        b.rom[0x10] = 0x11;
        let if1 = b.attach_interface1();
        let mut rom = [0u8; IF1_ROM_SIZE];
        rom[0x10] = 0x55;
        if1.load_rom(&rom).unwrap();
        if1.page_rom(true);
        assert_eq!(b.read(0x0010), 0x55);
        assert_eq!(b.read(0x2010), 0x55);
        b.interface1.as_mut().unwrap().page_rom(false);
        assert_eq!(b.read(0x0010), 0x11);
    }

    #[test]
    fn divmmc_control_beats_interface1_on_shared_e3() {
        let mut b = Bus48::new();
        let _ = b.attach_interface1();
        let _ = b.attach_divmmc();
        b.out_port(DIVMMC_PORT_CONTROL, 0x80);
        assert_eq!(b.divmmc.as_ref().unwrap().control, 0x80);
        assert_eq!(b.in_port(DIVMMC_PORT_CONTROL), 0x80);
    }

    #[test]
    fn beta_ports_when_trdos_paged_via_bus48() {
        let mut raw = vec![0u8; formats::TRD_SECTOR_SIZE * formats::TRD_SECTORS_PER_TRACK];
        raw[0] = 0x12;
        raw[1] = 0x34;
        let img = formats::TrdImage::parse(&raw).unwrap();
        let mut b = Bus48::new();
        b.kempston.fire = true;
        // Without TR-DOS paged, 0x1f is Kempston
        assert_eq!(b.in_port(0x001f), 0x10);
        let beta = b.attach_beta();
        beta.insert(img);
        beta.page_trdos(true);
        b.out_port(0x003f, 0); // track
        b.out_port(0x005f, 1); // sector 1 → index 0
        b.out_port(0x001f, 0x80); // read sector
        assert_eq!(b.in_port(0x001f), 0x02); // DRQ
        assert_eq!(b.in_port(0x007f), 0x12);
        assert_eq!(b.in_port(0x007f), 0x34);
    }

    #[test]
    fn beta_trdos_rom_overlays_when_paged() {
        let mut rom = [0u8; crate::TRDOS_ROM_SIZE];
        rom[0] = 0x42;
        let mut b = Bus48::new();
        b.rom[0] = 0x11;
        let beta = b.attach_beta();
        beta.load_rom(&rom).unwrap();
        beta.page_trdos(true);
        assert_eq!(b.read(0x0000), 0x42);
        b.beta.as_mut().unwrap().page_trdos(false);
        assert_eq!(b.read(0x0000), 0x11);
    }

    #[test]
    fn kempston_port_1f_untouched_when_beta_attached_but_not_paged() {
        let mut b = Bus48::new();
        b.kempston.fire = true;
        b.attach_beta();
        assert_eq!(b.in_port(0x001f), 0x10);
    }

    #[test]
    fn bus128_m1_pages_trdos_at_3d00_not_3c00() {
        let mut rom = [0u8; crate::TRDOS_ROM_SIZE];
        rom[0] = 0x42;
        let mut b = Bus128::new();
        b.rom[0][0] = 0x11;
        let beta = b.attach_beta();
        beta.load_rom(&rom).unwrap();
        b.notify_beta_m1(0x3c00);
        assert!(!b.beta.as_ref().unwrap().paged);
        assert_eq!(b.read(0x0000), 0x11);
        b.notify_beta_m1(0x3d00);
        assert!(b.beta.as_ref().unwrap().paged);
        assert_eq!(b.read(0x0000), 0x42);
    }
}
