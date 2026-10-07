//! Spectrum Next CPU-visible memory and the MMU `NextRegs`.
//!
//! This bus is deliberately separate from the classic 128K buses: each CPU
//! address range has its own 8 KiB page register, including the ROM slots.

use std::path::Path;

use crate::ay::PsgModel;
use crate::next_copper::NextCopper;
use crate::next_dma::{DmaAction, Endpoint, NextDma};
pub use crate::next_sd::NextSdError;
use crate::next_sd::{SdSpi, PORT_NEXT_SD_CS, PORT_NEXT_SD_DATA};
use crate::next_sprites::NextSprites;
use crate::next_video::NextVideo;
use crate::{require_rom_size, Ay8912, Kempston, Keyboard, RomLoadError, StereoMode, VideoColor};

const PAGE_SIZE: usize = 0x2000;
const CONFIG_BANK_SIZE: usize = 0x4000;
// The first 256 KiB of SRAM holds ROM and firmware storage, before RAM page 0.
const CONFIG_RESERVED_BANKS: usize = 16;
const TSTATES_PER_LINE: u64 = ula::T_LINE_48 as u64;
const LINES_PER_FRAME: u64 = ula::LINES_48 as u64;
const TSTATES_PER_LINE_128: u64 = ula::T_LINE_128 as u64;
const LINES_PER_FRAME_128: u64 = ula::LINES_128 as u64;
const TSTATES_PER_LINE_PENTAGON: u64 = ula::T_LINE_PENTAGON as u64;
const LINES_PER_FRAME_PENTAGON: u64 = ula::LINES_PENTAGON as u64;
pub const NEXT_ROM_SIZE: usize = 0x10000;
pub const NEXT_RAM_PAGE_COUNT: usize = 224;
const NEXT_MACHINE_ID: u8 = 0x0a;
const NEXT_CORE_VERSION: u8 = 0x32;
const NEXT_CORE_SUBMINOR: u8 = 0x04;
const NEXT_BOARD_ID: u8 = 0x00;
const RESET_PAGES: [u8; 8] = [0xff, 0xff, 10, 11, 4, 5, 0, 1];
const PORT_NEXTREG_SELECT: u16 = 0x243b;
const PORT_NEXTREG_ACCESS: u16 = 0x253b;
const PORT_LAYER2_CONTROL: u16 = 0x123b;
const PORT_SPRITE_SELECT: u16 = 0x303b;
const PORT_AY_DATA_MASK: u16 = 0xc007;
const PORT_AY_SELECT_MASK: u16 = 0xc007;
const PORT_AY_SELECT: u16 = 0xc005;
const PORT_AY_DATA: u16 = 0x8005;

/// CPU-visible Spectrum Next memory and the subset of `NextRegs` implemented by
/// this core slice. Unimplemented registers read as an undriven data bus.
#[derive(Debug)]
pub struct NextBus {
    rom: Box<[u8; NEXT_ROM_SIZE]>,
    ram: Vec<u8>,
    mmu: [u8; 8],
    selected_nextreg: u8,
    zx128_mapping: u8,
    zx128_rom_low: u8,
    zx128_allram_low: u8,
    zx128_locked: bool,
    zx128_bank_high: u8,
    boot_rom: Option<Box<[u8; 8192]>>,
    boot_enabled: bool,
    machine_type: u8,
    display_timing: u8,
    display_screen_bank: u8,
    border: u8,
    config_mapping: u8,
    next_video: NextVideo,
    copper: NextCopper,
    dma: NextDma,
    next_sprites: NextSprites,
    peripheral2: u8,
    peripheral3: u8,
    peripheral6: u8,
    /// Held 8-bit DAC outputs in A, B, C, D order.
    dac_values: [u8; 4],
    /// Timestamped output snapshots consumed by the machine's audio sampler.
    dac_writes: Vec<(u64, [u8; 4], bool)>,
    ay: [Ay8912; 3],
    /// Raw Next chip selector: 3 = AY0, 2 = AY1, 1 = AY2, 0 = reserved.
    ay_chip_select: u8,
    /// Per-chip output enables: bit 0 = left, bit 1 = right.
    ay_channel_enable: [u8; 3],
    reset_register: u8,
    reset_pending: u8,
    divmmc_nmi_pending: bool,
    divmmc_conmem: bool,
    divmmc_automapped: bool,
    divmmc_unmap_pending: bool,
    divmmc_control: u8,
    divmmc_automap_pending: bool,
    divmmc_entry_points: u8,
    divmmc_entry_points_valid: u8,
    divmmc_entry_timing: u8,
    divmmc_entry_points_1: u8,
    config_reserved_sram: Vec<u8>,
    alternate_roms: [Box<[u8; CONFIG_BANK_SIZE]>; 2],
    alternate_rom_register: u8,
    peripheral5: u8,
    sd_spi: Option<SdSpi>,
    sd_cs: u8,
    /// Host-driven Spectrum keyboard matrix, used by ULA port `$FE`.
    pub keyboard: Keyboard,
    pub kempston: Kempston,
}

/// Cloneable renderer state used to preserve Copper changes across one frame.
#[derive(Clone, Debug)]
pub struct NextBusVideoState {
    next_video: NextVideo,
    next_sprites: NextSprites,
    display_screen_bank: u8,
    border: u8,
}

/// Borrowed RAM plus one immutable Next video-register state for rendering.
#[derive(Debug)]
pub struct NextBusVideoRenderer<'a> {
    ram: &'a [u8],
    next_video: &'a NextVideo,
    next_sprites: &'a NextSprites,
    display_screen_bank: u8,
    border: u8,
}

impl NextBusVideoRenderer<'_> {
    #[must_use]
    pub fn read_ram_page(&self, page: u8, offset: usize) -> Option<u8> {
        (usize::from(page) < NEXT_RAM_PAGE_COUNT && offset < PAGE_SIZE)
            .then(|| self.ram[usize::from(page) * PAGE_SIZE + offset])
    }

    #[must_use]
    pub fn layer2_video_pixel(&self, x: usize, y: usize) -> Option<VideoColor> {
        if !self.next_video.visible() || !self.next_video.standard_mode() || x >= 256 || y >= 192 {
            return None;
        }
        let pixel = self.next_video.layer2_pixel(self.ram, x, y);
        let color = self
            .next_video
            .layer2_color(self.next_video.palette_index(pixel));
        (!color.transparent).then_some(color)
    }

    #[must_use]
    pub fn lores_video_pixel(&self, index: u8) -> Option<VideoColor> {
        (self.next_video.priority() & 0x80 != 0 && self.next_video.lores_256_color_mode())
            .then(|| self.next_video.lores_color(index))
            .filter(|color| !color.transparent)
    }

    #[must_use]
    pub fn radastan_video_pixel(&self, pixel: u8) -> Option<VideoColor> {
        (self.next_video.priority() & 0x80 != 0 && self.next_video.radastan_mode())
            .then(|| self.next_video.radastan_color(pixel))
            .filter(|color| !color.transparent)
    }

    #[must_use]
    pub fn radastan_display_file_offset(&self) -> usize {
        self.next_video.radastan_display_file_offset()
    }

    #[must_use]
    pub fn tilemap_video_pixel(&self, x: usize, y: usize) -> Option<(VideoColor, bool)> {
        self.next_video.tilemap_pixel(self.ram, x, y)
    }

    /// Render only the requested rows of the 320×256 sprite surface.
    #[must_use]
    pub fn render_sprite_rows(&self, rows: std::ops::Range<usize>) -> Vec<Option<VideoColor>> {
        self.next_sprites
            .render_surface_rows(self.next_video, self.next_video.priority(), rows)
    }

    #[must_use]
    pub fn video_layer_order(&self) -> u8 {
        self.next_video.priority() >> 2 & 0x07
    }

    #[must_use]
    pub fn lores_256_color_enabled(&self) -> bool {
        self.next_video.priority() & 0x80 != 0 && self.next_video.lores_256_color_mode()
    }

    #[must_use]
    pub fn radastan_lores_enabled(&self) -> bool {
        self.next_video.priority() & 0x80 != 0 && self.next_video.radastan_mode()
    }

    #[must_use]
    pub fn display_screen_bank(&self) -> u8 {
        self.display_screen_bank
    }

    #[must_use]
    pub fn border(&self) -> u8 {
        self.border
    }
}

impl NextBus {
    fn sd_selected(cs: u8) -> bool {
        matches!(cs & 0x8f, 0x8e | 0x8d)
    }

    /// Construct a fully expanded 2 MiB Next memory map with a 64 KiB ROM image.
    pub fn new(rom: &[u8]) -> Result<Self, RomLoadError> {
        require_rom_size(rom, "Spectrum Next ROM", NEXT_ROM_SIZE)?;
        let mut rom_bytes = Box::new([0; NEXT_ROM_SIZE]);
        rom_bytes.copy_from_slice(rom);
        let mut bus = Self {
            rom: rom_bytes,
            ram: vec![0; NEXT_RAM_PAGE_COUNT * PAGE_SIZE],
            mmu: [0; 8],
            selected_nextreg: 0,
            zx128_mapping: 0,
            zx128_rom_low: 0,
            zx128_allram_low: 0,
            zx128_locked: false,
            zx128_bank_high: 0,
            boot_rom: None,
            boot_enabled: false,
            machine_type: 0,
            display_timing: 0,
            display_screen_bank: 0,
            border: 0,
            config_mapping: 0,
            next_video: NextVideo::new(),
            copper: NextCopper::new(),
            dma: NextDma::default(),
            next_sprites: NextSprites::new(),
            peripheral2: 0,
            peripheral3: 0,
            peripheral6: 0,
            dac_values: [0x80; 4],
            dac_writes: Vec::new(),
            ay: std::array::from_fn(|_| Ay8912::new()),
            ay_chip_select: 3,
            ay_channel_enable: [0x03; 3],
            reset_register: 0,
            reset_pending: 0,
            divmmc_nmi_pending: false,
            divmmc_conmem: false,
            divmmc_automapped: false,
            divmmc_unmap_pending: false,
            divmmc_control: 0,
            divmmc_automap_pending: false,
            divmmc_entry_points: 0,
            divmmc_entry_points_valid: 0,
            divmmc_entry_timing: 0,
            divmmc_entry_points_1: 0,
            config_reserved_sram: vec![0; CONFIG_RESERVED_BANKS * CONFIG_BANK_SIZE - NEXT_ROM_SIZE],
            alternate_roms: [
                Box::new([0; CONFIG_BANK_SIZE]),
                Box::new([0; CONFIG_BANK_SIZE]),
            ],
            alternate_rom_register: 0,
            peripheral5: 0,
            sd_spi: None,
            sd_cs: 0,
            keyboard: Keyboard::new(),
            kempston: Kempston::new(),
        };
        bus.hard_reset();
        Ok(bus)
    }

    /// Return hardware to its hard-reset IPL/configuration state.
    pub fn hard_reset(&mut self) {
        self.mmu = RESET_PAGES;
        self.selected_nextreg = 0;
        self.zx128_mapping = 0x08;
        self.zx128_rom_low = 0;
        self.zx128_allram_low = 0;
        self.zx128_locked = false;
        self.zx128_bank_high = 0;
        self.machine_type = 0;
        self.display_timing = 0;
        self.display_screen_bank = 5;
        self.config_mapping = 0;
        self.alternate_rom_register = 0;
        self.next_video.reset();
        self.copper.reset();
        self.dma.reset();
        self.next_sprites.reset();
        self.peripheral2 = 0;
        self.peripheral3 = 0x10;
        self.peripheral6 = 0;
        self.dac_values = [0x80; 4];
        self.dac_writes.clear();
        self.ay = std::array::from_fn(|_| Ay8912::new());
        for ay in &mut self.ay {
            ay.set_model(PsgModel::Ym2149);
        }
        self.ay_chip_select = 3;
        self.ay_channel_enable = [0x03; 3];
        self.reset_register = 2;
        self.reset_pending = 0;
        self.divmmc_nmi_pending = false;
        self.divmmc_conmem = false;
        self.divmmc_automapped = false;
        self.divmmc_unmap_pending = false;
        self.divmmc_control = 0;
        self.divmmc_automap_pending = false;
        self.divmmc_entry_points = 0x83;
        self.divmmc_entry_points_valid = 0x01;
        self.divmmc_entry_timing = 0;
        self.divmmc_entry_points_1 = 0xcd;
        self.peripheral5 = 0x01;
        self.boot_enabled = self.boot_rom.is_some();
        self.sd_cs = 0xff;
        if let Some(sd) = &mut self.sd_spi {
            sd.reset();
        }
    }

    /// Reset transient hardware state while preserving the selected machine.
    pub fn soft_reset(&mut self) {
        let machine_type = self.machine_type;
        let display_timing = self.display_timing;
        let boot_enabled = self.boot_enabled;
        let peripheral3 = self.peripheral3;
        let mut next_sprites = self.next_sprites.clone();
        next_sprites.reset_preserving_memory();
        let peripheral5 = self.peripheral5;
        let alternate_rom_reset = (self.alternate_rom_register & 0x0f) * 0x11;
        let mut next_video = self.next_video.clone();
        next_video.reset_preserving_palette();
        self.hard_reset();
        self.next_video = next_video;
        self.next_sprites = next_sprites;
        self.machine_type = machine_type;
        self.display_timing = display_timing;
        self.boot_enabled = boot_enabled;
        self.peripheral3 = peripheral3 & !0x40;
        self.peripheral5 = peripheral5;
        self.alternate_rom_register = alternate_rom_reset;
        self.peripheral6 = 0xa0;
        self.reset_register = 0x01;
    }

    /// Enable an 8 KiB IPL ROM over address 0 after reset.
    pub fn install_boot_rom(&mut self, bytes: &[u8]) -> Result<(), RomLoadError> {
        require_rom_size(bytes, "Spectrum Next IPL ROM", 8192)?;
        let mut rom = Box::new([0; 8192]);
        rom.copy_from_slice(bytes);
        self.hard_reset();
        self.boot_rom = Some(rom);
        self.machine_type = 0;
        self.config_mapping = 0;
        self.peripheral5 = 0x01;
        self.boot_enabled = true;
        self.reset_register = 0x02;
        Ok(())
    }

    /// Attach a mutable file-backed SD card image, read a sector at a time.
    pub fn attach_sd_file(&mut self, path: &Path) -> Result<(), NextSdError> {
        self.sd_spi = Some(SdSpi::open(path)?);
        self.sd_cs = 0xff;
        Ok(())
    }

    /// Read a CPU-visible address through the current 8 KiB mapping.
    #[must_use]
    pub fn read(&self, addr: u16) -> u8 {
        if self.boot_enabled && addr < 0x2000 {
            if let Some(boot_rom) = &self.boot_rom {
                return boot_rom[usize::from(addr)];
            }
        }
        if self.divmmc_mapped() && addr < 0x4000 {
            return self.read_divmmc(usize::from(addr));
        }
        if self.alternate_rom_reads() && addr < 0x4000 && self.rom_slot_selected(addr) {
            return self.alternate_roms[self.alternate_rom_bank()][usize::from(addr)];
        }
        if self.config_window_active() && addr < 0x4000 {
            return self.read_config_window(usize::from(addr));
        }
        let slot = usize::from(addr) / PAGE_SIZE;
        let offset = usize::from(addr) % PAGE_SIZE;
        match self.mmu_page(slot) {
            0xff if slot < 2 => {
                let rom_bank = usize::from(self.zx128_mapping & 0x03);
                self.rom[rom_bank * CONFIG_BANK_SIZE + slot * PAGE_SIZE + offset]
            }
            page if usize::from(page) < NEXT_RAM_PAGE_COUNT => {
                self.ram[usize::from(page) * PAGE_SIZE + offset]
            }
            _ => 0xff,
        }
    }

    /// Read the opcode bus, including Next automap triggers on M1 fetches.
    #[must_use]
    pub fn read_opcode(&mut self, addr: u16) -> u8 {
        self.update_divmmc_mapping_on_fetch(addr);
        let opcode = self.read(addr);
        if self.divmmc_automap_pending {
            self.divmmc_automapped = true;
            self.divmmc_automap_pending = false;
        }
        if self.divmmc_unmap_pending {
            self.divmmc_automapped = false;
            self.divmmc_unmap_pending = false;
        }
        opcode
    }

    fn update_divmmc_mapping_on_fetch(&mut self, addr: u16) {
        let entries = self.divmmc_entry_points_1;
        if entries & 0x40 != 0 && (0x1ff8..=0x1fff).contains(&addr) {
            self.divmmc_unmap_pending = true;
            return;
        }
        if self.peripheral5 & 0x10 == 0 {
            return;
        }

        let (enabled, instant) = if addr <= 0x38 && addr & 0x07 == 0 {
            let entry = (addr / 8) as u8;
            let mask = 1 << entry;
            if self.divmmc_entry_points & mask == 0 {
                return;
            }
            let requires_rom3 = self.divmmc_entry_points_valid & mask == 0;
            if requires_rom3 && !self.rom3_selected() {
                return;
            }
            (true, self.divmmc_entry_timing & mask != 0)
        } else if (0x3d00..=0x3dff).contains(&addr) && entries & 0x80 != 0 {
            (self.rom3_selected(), true)
        } else {
            let (address, mask) = match addr {
                0x0066 if entries & 0x02 != 0 => (true, 0x02),
                0x0066 if entries & 0x01 != 0 => (true, 0x01),
                0x056a if entries & 0x20 != 0 => (true, 0x20),
                0x04d7 if entries & 0x10 != 0 => (true, 0x10),
                0x0562 if entries & 0x08 != 0 => (true, 0x08),
                0x04c6 if entries & 0x04 != 0 => (true, 0x04),
                _ => (false, 0),
            };
            if !address || (mask != 0x01 && mask != 0x02 && !self.rom3_selected()) {
                return;
            }
            let instant = mask == 0x02;
            (true, instant)
        };

        if enabled {
            if instant {
                self.divmmc_automapped = true;
            } else {
                self.divmmc_automap_pending = true;
            }
        }
    }

    fn rom3_selected(&self) -> bool {
        !self.boot_enabled
            && !self.config_window_active()
            && !self.divmmc_mapped()
            && self.mmu_page(0) == 0xff
            && self.mmu_page(1) == 0xff
            && self.zx128_mapping & 0x03 == 0x03
    }

    /// Write a CPU-visible address. ROM and absent pages are not writable.
    pub fn write(&mut self, addr: u16, value: u8) {
        if self.divmmc_mapped() && addr < 0x4000 {
            self.write_divmmc(usize::from(addr), value);
            return;
        }
        if self.config_window_active() && addr < 0x4000 {
            self.write_config_window(usize::from(addr), value);
            return;
        }
        if self.alternate_rom_writes() && addr < 0x4000 && self.rom_slot_selected(addr) {
            self.alternate_roms[self.alternate_rom_bank()][usize::from(addr)] = value;
            return;
        }
        let slot = usize::from(addr) / PAGE_SIZE;
        let page = usize::from(self.mmu_page(slot));
        if page < NEXT_RAM_PAGE_COUNT {
            let offset = usize::from(addr) % PAGE_SIZE;
            self.ram[page * PAGE_SIZE + offset] = value;
        }
    }

    /// Read one implemented `NextReg`; unimplemented registers float high.
    #[must_use]
    pub fn read_nextreg(&self, register: u8) -> u8 {
        if register == 0x1c {
            return (self.next_video.clip_index() << 6) | (self.next_sprites.clip_index() << 2);
        }
        if let Some(value) = self.next_sprites.read_register(register) {
            return value;
        }
        if let Some(value) = self.next_video.palette_register(register) {
            return value;
        }
        if let Some(value) = self.next_video.read_register(register) {
            return value;
        }
        match register {
            0x00 => NEXT_MACHINE_ID,
            0x01 => NEXT_CORE_VERSION,
            0x02 => self.reset_register,
            0x03 => {
                (u8::from(self.next_video.palette_second_write_pending()) << 7)
                    | (self.display_timing << 4)
                    | self.machine_type
            }
            0x08 => (self.peripheral3 & 0x7f) | (u8::from(!self.zx128_locked) << 7),
            0x1e | 0x1f => 0,
            0x61 => self.copper.address_low(),
            0x62 => self.copper.control(),
            0x06 => self.peripheral6,
            0x09 => self.peripheral2,
            0x0a => self.peripheral5,
            0x0e => NEXT_CORE_SUBMINOR,
            0x0f => NEXT_BOARD_ID,
            0x8c => self.alternate_rom_register,
            0xb8 => self.divmmc_entry_points,
            0xb9 => self.divmmc_entry_points_valid,
            0xba => self.divmmc_entry_timing,
            0xbb => self.divmmc_entry_points_1,
            0x50..=0x57 => self.mmu[usize::from(register - 0x50)],
            0x8e => self.zx128_mapping,
            _ => 0xff,
        }
    }

    /// Advance the enabled Next AY chips by CPU T-states.
    pub fn advance_audio(&mut self, tstates: u32) {
        match self.peripheral6 & 0x03 {
            0 | 1 => {
                // All AY oscillators keep running. Turbo Sound controls which
                // chip is selected and which chips reach the output mixer.
                for ay in &mut self.ay {
                    ay.advance(tstates);
                }
            }
            // ZXN-8950 is a separate follow-up; mode 2 suspends AY synthesis.
            2 => {}
            // Mode 3 holds every AY in reset.
            _ => {}
        }
    }

    /// Current mixed AY sample as mono, left and right amplitudes.
    #[must_use]
    pub fn audio_sample(&self) -> (f32, f32, f32) {
        if matches!(self.peripheral6 & 0x03, 2 | 3) {
            return (0.0, 0.0, 0.0);
        }
        let turbo_sound = self.peripheral3 & 0x02 != 0;
        let first_chip = if turbo_sound {
            0
        } else if let Some(index) = self.selected_ay_index() {
            index
        } else {
            return (0.0, 0.0, 0.0);
        };
        let last_chip = if turbo_sound { 3 } else { first_chip + 1 };
        let chip_count = last_chip - first_chip;
        let stereo = if self.peripheral3 & 0x20 != 0 {
            StereoMode::Acb
        } else {
            StereoMode::Abc
        };
        let mut left = 0.0;
        let mut right = 0.0;
        for index in first_chip..last_chip {
            let ay = &self.ay[index];
            let mut levels = ay.channel_levels();
            for (channel, level) in levels.iter_mut().enumerate() {
                if ay.regs[8 + channel] == 0 {
                    // Center unused channels because host PCM treats 0.5 as
                    // silence; zero would introduce a negative DC component.
                    *level = 0.5;
                }
            }
            let (mut chip_left, mut chip_right) = if self.peripheral2 & (1 << (index + 5)) != 0 {
                let mono = (levels[0] + levels[1] + levels[2]) / 3.0;
                (mono, mono)
            } else {
                let (left, center, right) = if stereo == StereoMode::Acb {
                    (levels[0], levels[2], levels[1])
                } else {
                    (levels[0], levels[1], levels[2])
                };
                ((left + center * 0.5) / 1.5, (right + center * 0.5) / 1.5)
            };
            let enabled = self.ay_channel_enable[index];
            if enabled & 0x01 == 0 {
                chip_left = 0.5;
            }
            if enabled & 0x02 == 0 {
                chip_right = 0.5;
            }
            left += chip_left - 0.5;
            right += chip_right - 0.5;
        }
        // Keep a single-chip tune at the classic AY amplitude while allowing
        // each enabled chip to contribute equally to Turbo Sound output.
        left = 0.5 + left / chip_count as f32;
        right = 0.5 + right / chip_count as f32;
        (f32::midpoint(left, right), left, right)
    }

    /// Whether `NextReg` `$09` bit 2 silences the complete HDMI audio mix.
    #[must_use]
    pub fn audio_muted(&self) -> bool {
        self.peripheral2 & 0x04 != 0
    }

    /// Held DAC outputs, in A, B, C, D order.
    #[must_use]
    pub fn dac_values(&self) -> [u8; 4] {
        self.dac_values
    }

    /// Whether the four Next DAC output pins are enabled by `NextReg` `$08.3`.
    #[must_use]
    pub fn dacs_enabled(&self) -> bool {
        self.peripheral3 & 0x08 != 0
    }

    /// Append timestamped DAC writes to the machine's pending audio queue.
    pub fn drain_dac_writes_into(&mut self, pending: &mut Vec<(u64, [u8; 4], bool)>) {
        pending.append(&mut self.dac_writes);
    }

    fn write_dac(&mut self, dac: usize, value: u8, t: u64) {
        self.dac_values[dac] = value;
        self.dac_writes
            .push((t, self.dac_values, self.dacs_enabled()));
    }

    fn write_dac_alias(&mut self, port: u8, value: u8, t: u64) -> bool {
        let (first, second) = match port {
            0x1f | 0xf1 | 0x3f => (Some(0), None),
            0x0f | 0xf3 => (Some(1), None),
            0xdf | 0xfb => (Some(0), Some(3)),
            0xb3 => (Some(1), Some(2)),
            0x4f | 0xf9 => (Some(2), None),
            0x5f => (Some(3), None),
            _ => return false,
        };
        if let Some(dac) = first {
            self.write_dac(dac, value, t);
        }
        if let Some(dac) = second {
            self.write_dac(dac, value, t);
        }
        true
    }

    /// Whether an AY amplitude register is configured to produce audio.
    #[must_use]
    pub fn audio_configured(&self) -> bool {
        if !matches!(self.peripheral6 & 0x03, 0 | 1) {
            return false;
        }
        if self.peripheral3 & 0x02 != 0 {
            self.ay
                .iter()
                .any(|ay| ay.regs[8..11].iter().any(|value| *value != 0))
        } else {
            self.selected_ay()
                .is_some_and(|ay| ay.regs[8..11].iter().any(|value| *value != 0))
        }
    }

    /// The AY currently addressed by `$FFFD` / `$BFFD` (0 = AY0, 2 = AY2).
    #[must_use]
    pub fn selected_ay(&self) -> Option<&Ay8912> {
        self.selected_ay_index().map(|index| &self.ay[index])
    }

    /// Read-only access to an individual AY chip for inspection and tests.
    #[must_use]
    pub fn ay_chip(&self, index: usize) -> Option<&Ay8912> {
        self.ay.get(index)
    }

    fn selected_ay_index(&self) -> Option<usize> {
        match self.ay_chip_select {
            3 => Some(0),
            2 => Some(1),
            1 => Some(2),
            _ => None,
        }
    }

    fn read_ay_data(&self) -> u8 {
        self.selected_ay().map_or(0xff, Ay8912::read_data)
    }

    fn write_ay_select(&mut self, value: u8) {
        let turbo_sound = self.peripheral3 & 0x02 != 0;
        if value & 0xe0 == 0 {
            if let Some(index) = self.selected_ay_index() {
                self.ay[index].select(value);
            }
            return;
        }
        if !turbo_sound || value & 0x80 == 0 || value & 0x1c != 0x1c {
            return;
        }
        let select = value & 0x03;
        self.ay_chip_select = select;
        if let Some(index) = self.selected_ay_index() {
            self.ay_channel_enable[index] = ((value >> 6) & 1) | ((value >> 4) & 2);
        }
    }

    fn write_ay_data(&mut self, value: u8) {
        if self.peripheral6 & 0x03 == 3 {
            return;
        }
        if let Some(index) = self.selected_ay_index() {
            let ay = &mut self.ay[index];
            if ay.selected < 14 {
                ay.write_data(value);
            }
        }
    }

    /// Read time-dependent Next registers at the CPU's absolute T-state.
    #[must_use]
    pub fn read_nextreg_at(&self, register: u8, t: u64) -> u8 {
        if matches!(register, 0x1e | 0x1f) {
            let (tstates_per_line, lines_per_frame, _) = self.display_geometry();
            let line = ((t / tstates_per_line) % lines_per_frame) as u16;
            return if register == 0x1e {
                (line >> 8) as u8 & 0x01
            } else {
                line as u8
            };
        }
        self.read_nextreg(register)
    }

    /// Update implemented boot, display-bank, peripheral, and MMU registers.
    pub fn write_nextreg(&mut self, register: u8, value: u8) {
        self.write_nextreg_at(register, value, 0);
    }

    /// Update a `NextReg` while preserving the CPU T-state for timed audio writes.
    pub fn write_nextreg_at(&mut self, register: u8, value: u8, t: u64) {
        match register {
            0x60 => {
                self.copper.write_data(value);
                return;
            }
            0x61 => {
                self.copper.write_address_low(value);
                return;
            }
            0x62 => {
                self.copper.write_control(value, t);
                return;
            }
            _ => {}
        }
        match register {
            0x2c => self.write_dac(1, value, t),
            0x2d => {
                self.write_dac(0, value, t);
                self.write_dac(3, value, t);
            }
            0x2e => self.write_dac(2, value, t),
            _ => {}
        }
        if register == 0x1c {
            self.next_sprites
                .write_register(register, value, self.peripheral2 & 0x10 != 0);
            self.next_video.write_register(register, value);
            return;
        }
        if self.next_video.write_palette_register(register, value) {
            return;
        }
        let sprite_index_lockstep = self.peripheral2 & 0x10 != 0;
        if self
            .next_sprites
            .write_register(register, value, sprite_index_lockstep)
        {
            return;
        }
        if self.next_video.write_register(register, value) {
            return;
        }
        match register {
            0x02 => {
                self.reset_pending = value & 0x03;
                if value & 0x04 != 0 {
                    self.reset_register |= 0x04;
                    self.divmmc_nmi_pending = true;
                }
                if value & 0x04 == 0 {
                    self.reset_register &= !0x04;
                }
            }
            0x08 => {
                self.peripheral3 = value & 0x7f;
                self.dac_writes
                    .push((t, self.dac_values, self.dacs_enabled()));
                if value & 0x80 != 0 {
                    self.zx128_locked = false;
                }
            }
            0x09 => {
                self.peripheral2 = value;
                if value & 0x08 != 0 {
                    self.divmmc_control &= !0x40;
                }
            }
            0x06 => {
                self.peripheral6 = value;
                let model = if value & 0x01 == 0 {
                    PsgModel::Ym2149
                } else {
                    PsgModel::Ay8912
                };
                for ay in &mut self.ay {
                    ay.set_model(model);
                }
                if value & 0x03 == 0x03 {
                    for ay in &mut self.ay {
                        ay.reset();
                    }
                }
            }
            0x03 if self.machine_type == 0 => {
                self.boot_enabled = false;
                if value & 0x80 != 0 {
                    self.display_timing = (value >> 4) & 0x07;
                }
                self.machine_type = value & 0x07;
            }
            0x04 if self.machine_type == 0 && !self.boot_enabled => {
                self.config_mapping = value & 0x7f;
            }
            0xb8 => self.divmmc_entry_points = value,
            0xb9 => self.divmmc_entry_points_valid = value,
            0xba => self.divmmc_entry_timing = value,
            0xbb => self.divmmc_entry_points_1 = value,
            0x8e => {
                self.write_128_mapping(value, value & 0x08 != 0);
            }
            0x0a => {
                self.peripheral5 = if self.machine_type == 0 {
                    value & 0xfb
                } else {
                    (self.peripheral5 & 0xe0) | (value & 0x1b)
                };
                if self.peripheral5 & 0x10 == 0 {
                    self.divmmc_automapped = false;
                    self.divmmc_automap_pending = false;
                    self.divmmc_unmap_pending = false;
                }
                if let Some(sd) = &mut self.sd_spi {
                    sd.select(Self::sd_selected(self.sd_cs));
                }
            }
            0x8c => self.alternate_rom_register = value,
            0x50..=0x57 => self.mmu[usize::from(register - 0x50)] = value,
            _ => {}
        }
    }

    /// Read a physical RAM page, independent of its CPU slot mapping.
    #[must_use]
    pub fn read_ram_page(&self, page: u8, offset: usize) -> Option<u8> {
        (usize::from(page) < NEXT_RAM_PAGE_COUNT && offset < PAGE_SIZE)
            .then(|| self.ram[usize::from(page) * PAGE_SIZE + offset])
    }

    /// Read a standard-mode Layer 2 pixel from physical RAM.
    #[must_use]
    pub fn layer2_pixel(&self, x: usize, y: usize) -> u8 {
        self.next_video.layer2_pixel(&self.ram, x, y)
    }

    /// Return an opaque Layer 2 pixel color when the standard layer is visible.
    #[must_use]
    pub fn layer2_video_pixel(&self, x: usize, y: usize) -> Option<VideoColor> {
        if !self.next_video.visible() || !self.next_video.standard_mode() || x >= 256 || y >= 192 {
            return None;
        }
        let pixel = self.next_video.layer2_pixel(&self.ram, x, y);
        let color = self
            .next_video
            .layer2_color(self.next_video.palette_index(pixel));
        (!color.transparent).then_some(color)
    }

    /// Return a standard 256-colour `LoRes` pixel when `NextReg` `$15` enables it.
    #[must_use]
    pub fn lores_video_pixel(&self, index: u8) -> Option<VideoColor> {
        self.lores_256_color_enabled()
            .then(|| self.next_video.lores_color(index))
            .filter(|color| !color.transparent)
    }

    /// Return a Radastan `LoRes` pixel using the selected ULA palette and offset.
    #[must_use]
    pub fn radastan_video_pixel(&self, pixel: u8) -> Option<VideoColor> {
        self.radastan_lores_enabled()
            .then(|| self.next_video.radastan_color(pixel))
            .filter(|color| !color.transparent)
    }

    /// Return a standard 40×32 tilemap pixel and its priority over ULA.
    #[must_use]
    pub fn tilemap_video_pixel(&self, x: usize, y: usize) -> Option<(VideoColor, bool)> {
        self.next_video.tilemap_pixel(&self.ram, x, y)
    }

    /// Whether standard 256-colour `LoRes` currently replaces ULA paper.
    #[must_use]
    pub fn lores_256_color_enabled(&self) -> bool {
        self.next_video.priority() & 0x80 != 0 && self.next_video.lores_256_color_mode()
    }

    /// Whether Radastan's packed 16-colour `LoRes` replaces ULA paper.
    #[must_use]
    pub fn radastan_lores_enabled(&self) -> bool {
        self.next_video.priority() & 0x80 != 0 && self.next_video.radastan_mode()
    }

    /// Offset of the selected Radastan display file within the ULA RAM bank.
    #[must_use]
    pub fn radastan_display_file_offset(&self) -> usize {
        self.next_video.radastan_display_file_offset()
    }

    /// Return the standard sprite layer on its 320×256 display surface.
    #[must_use]
    pub fn render_sprite_surface(&self) -> Vec<Option<VideoColor>> {
        self.next_sprites
            .render_surface(&self.next_video, self.next_video.priority())
    }

    /// Three-bit ordering selector from `NextReg` `$15` bits 4–2.
    #[must_use]
    pub fn video_layer_order(&self) -> u8 {
        self.next_video.priority() >> 2 & 0x07
    }

    /// Physical ULA screen bank selected by the `$7FFD` display latch.
    #[must_use]
    pub fn display_screen_bank(&self) -> u8 {
        self.display_screen_bank
    }

    #[must_use]
    pub fn border(&self) -> u8 {
        self.border
    }

    /// Spectrum display timing selected through `NextReg` `$03`.
    #[must_use]
    pub fn display_timing(&self) -> u8 {
        self.display_timing
    }

    fn display_geometry(&self) -> (u64, u64, u64) {
        match self.display_timing {
            2 | 3 => (
                TSTATES_PER_LINE_128,
                LINES_PER_FRAME_128,
                u64::from(ula::INT_LENGTH_128),
            ),
            4 => (
                TSTATES_PER_LINE_PENTAGON,
                LINES_PER_FRAME_PENTAGON,
                u64::from(ula::INT_LENGTH_PENTAGON),
            ),
            // Timing 0 is internal; 5–7 are reserved. Keep the existing 48K
            // fallback until those modes have defined raster behavior.
            _ => (
                TSTATES_PER_LINE,
                LINES_PER_FRAME,
                u64::from(ula::INT_LENGTH_48),
            ),
        }
    }

    /// Frame period and active ULA interrupt pulse in 3.5 MHz CPU T-states.
    #[must_use]
    pub fn frame_interrupt_timing(&self) -> (u64, u64) {
        let (tstates_per_line, lines_per_frame, interrupt_tstates) = self.display_geometry();
        (tstates_per_line * lines_per_frame, interrupt_tstates)
    }

    /// Advance Copper execution to an absolute CPU T-state.
    pub fn advance_copper(&mut self, time: u64) -> Vec<(u64, u8, u8)> {
        let (frame_tstates, _) = self.frame_interrupt_timing();
        let (tstates_per_line, lines_per_frame, _) = self.display_geometry();
        self.copper.advance(
            time,
            frame_tstates as u32,
            tstates_per_line as u32,
            lines_per_frame as u32,
        )
    }

    /// Whether a `NextReg` can change a value consumed by the frame renderer.
    /// `$14` is the global transparency colour, not the ULA border colour.
    #[must_use]
    pub fn nextreg_affects_video(register: u8) -> bool {
        matches!(
            register,
            0x12 | 0x14 | 0x15 | 0x19 | 0x1b | 0x1c | 0x2f..=0x31 | 0x34..=0x41
                | 0x43..=0x44 | 0x4b..=0x4c | 0x69..=0x70 | 0x75..=0x79
        )
    }

    /// Capture renderer state after a Copper display-register write.
    #[must_use]
    pub fn video_state_snapshot(&self) -> NextBusVideoState {
        NextBusVideoState {
            next_video: self.next_video.clone(),
            next_sprites: self.next_sprites.clone(),
            display_screen_bank: self.display_screen_bank,
            border: self.border,
        }
    }

    /// Borrow current RAM and render state.
    pub fn video_renderer(&self) -> NextBusVideoRenderer<'_> {
        self.video_renderer_with_state_parts(
            &self.next_video,
            &self.next_sprites,
            self.display_screen_bank,
            self.border,
        )
    }

    /// Borrow RAM with a previously captured Copper video state.
    pub fn video_renderer_with_state<'a>(
        &'a self,
        state: &'a NextBusVideoState,
    ) -> NextBusVideoRenderer<'a> {
        self.video_renderer_with_state_parts(
            &state.next_video,
            &state.next_sprites,
            state.display_screen_bank,
            state.border,
        )
    }

    fn video_renderer_with_state_parts<'a>(
        &'a self,
        next_video: &'a NextVideo,
        next_sprites: &'a NextSprites,
        display_screen_bank: u8,
        border: u8,
    ) -> NextBusVideoRenderer<'a> {
        NextBusVideoRenderer {
            ram: &self.ram,
            next_video,
            next_sprites,
            display_screen_bank,
            border,
        }
    }

    fn config_window_active(&self) -> bool {
        self.boot_rom.is_some() && !self.boot_enabled && self.machine_type == 0
    }

    fn read_divmmc(&self, offset: usize) -> u8 {
        if offset < PAGE_SIZE {
            if self.divmmc_control & 0x40 != 0 {
                self.config_reserved_sram[0x10000 + 3 * PAGE_SIZE + offset]
            } else {
                self.config_reserved_sram[offset]
            }
        } else {
            self.config_reserved_sram[Self::divmmc_ram_index(
                usize::from(self.divmmc_control & 0x0f),
                offset - PAGE_SIZE,
            )]
        }
    }

    fn write_divmmc(&mut self, offset: usize, value: u8) {
        if offset < PAGE_SIZE {
            return;
        }
        let bank = usize::from(self.divmmc_control & 0x0f);
        if self.divmmc_control & 0x40 != 0 && bank == 3 {
            return;
        }
        let index = Self::divmmc_ram_index(bank, offset - PAGE_SIZE);
        self.config_reserved_sram[index] = value;
    }

    fn divmmc_ram_index(bank: usize, offset: usize) -> usize {
        0x10000 + bank * PAGE_SIZE + offset
    }

    fn divmmc_mapped(&self) -> bool {
        self.divmmc_conmem || self.divmmc_automapped
    }

    fn write_128_paging(&mut self, port: u16, value: u8) {
        let port = if port & 0xf003 == 0xd001 {
            0xdffd
        } else if port & 0xf003 == 0x1001 {
            0x1ffd
        } else if port & 0x8003 == 0x0001 {
            0x7ffd
        } else {
            return;
        };
        match port {
            0x7ffd => {
                if self.zx128_locked {
                    return;
                }
                self.display_screen_bank = if value & 0x08 != 0 { 7 } else { 5 };
                if self.zx128_mapping & 0x04 == 0 {
                    self.zx128_rom_low = (value & 0x10) >> 4;
                }
                let rom_or_allram_low = if self.zx128_mapping & 0x04 == 0 {
                    self.zx128_rom_low
                } else {
                    self.zx128_allram_low
                };
                let mapping =
                    (self.zx128_mapping & 0x8e) | 0x08 | ((value & 0x07) << 4) | rom_or_allram_low;
                self.write_128_mapping(mapping, true);
                if value & 0x20 != 0 {
                    self.zx128_locked = true;
                }
            }
            0xdffd => {
                if self.zx128_locked {
                    return;
                }
                self.zx128_bank_high = (value & 0x0e) << 3;
                let mapping = (self.zx128_mapping & 0x7f) | (value & 0x01) << 7;
                self.write_128_mapping(mapping, true);
            }
            0x1ffd => {
                if self.zx128_locked {
                    return;
                }
                let mode = (value & 0x01) << 2;
                if mode != 0 {
                    self.zx128_allram_low = (value & 0x02) >> 1;
                }
                let low_select = if mode == 0 {
                    self.zx128_rom_low
                } else {
                    self.zx128_allram_low
                };
                let mapping =
                    (self.zx128_mapping & !0x07) | 0x08 | mode | (value & 0x04) >> 1 | low_select;
                self.write_128_mapping(mapping, true);
            }
            _ => {}
        }
    }

    fn write_128_mapping(&mut self, value: u8, update_ram_bank: bool) {
        let was_special = self.zx128_mapping & 0x04 != 0;
        let is_special = value & 0x04 != 0;
        if is_special {
            self.zx128_allram_low = value & 0x01;
        } else {
            self.zx128_rom_low = value & 0x01;
        }
        let ram_bank = if value & 0x08 != 0 {
            value & 0xf0
        } else {
            self.zx128_mapping & 0xf0
        };
        let low_select = if is_special {
            self.zx128_allram_low
        } else {
            self.zx128_rom_low
        };
        self.zx128_mapping = ram_bank | (value & 0x06) | low_select | 0x08;
        if !is_special {
            if was_special {
                self.mmu[2] = 10;
                self.mmu[3] = 11;
                self.mmu[4] = 4;
                self.mmu[5] = 5;
            }
            self.mmu[0] = 0xff;
            self.mmu[1] = 0xff;
            if update_ram_bank {
                self.update_128_ram_bank();
            }
        }
    }

    fn update_128_ram_bank(&mut self) {
        if self.zx128_mapping & 0x04 == 0 {
            let bank = ((self.zx128_mapping >> 4) & 0x07)
                | ((self.zx128_mapping & 0x80) >> 4)
                | self.zx128_bank_high;
            self.mmu[6] = bank * 2;
            self.mmu[7] = bank * 2 + 1;
        }
    }

    fn mmu_page(&self, slot: usize) -> u8 {
        if self.zx128_mapping & 0x04 != 0 {
            let banks = match self.zx128_mapping & 0x03 {
                0 => [0, 1, 2, 3],
                1 => [4, 5, 6, 7],
                2 => [4, 5, 6, 3],
                _ => [4, 7, 6, 3],
            };
            banks[slot / 2] * 2 + (slot % 2) as u8
        } else {
            self.mmu[slot]
        }
    }

    // Config mapping uses raw SRAM bank numbers; MMU RAM pages begin at bank 16.
    fn read_config_window(&self, offset: usize) -> u8 {
        let bank = usize::from(self.config_mapping);
        if bank < 4 {
            self.rom[bank * CONFIG_BANK_SIZE + offset]
        } else if bank == 6 || bank == 7 {
            self.alternate_roms[bank - 6][offset]
        } else if bank < CONFIG_RESERVED_BANKS {
            self.config_reserved_sram[(bank - 4) * CONFIG_BANK_SIZE + offset]
        } else {
            let first_page = (bank - CONFIG_RESERVED_BANKS) * 2;
            let page = first_page + offset / PAGE_SIZE;
            self.ram[page * PAGE_SIZE + offset % PAGE_SIZE]
        }
    }

    fn write_config_window(&mut self, offset: usize, value: u8) {
        let bank = usize::from(self.config_mapping);
        if bank < 4 {
            self.rom[bank * CONFIG_BANK_SIZE + offset] = value;
        } else if bank == 6 || bank == 7 {
            self.alternate_roms[bank - 6][offset] = value;
        } else if bank < CONFIG_RESERVED_BANKS {
            self.config_reserved_sram[(bank - 4) * CONFIG_BANK_SIZE + offset] = value;
        } else {
            let first_page = (bank - CONFIG_RESERVED_BANKS) * 2;
            let page = first_page + offset / PAGE_SIZE;
            self.ram[page * PAGE_SIZE + offset % PAGE_SIZE] = value;
        }
    }

    fn alternate_rom_reads(&self) -> bool {
        self.alternate_rom_register & 0xc0 == 0x80
    }

    fn alternate_rom_writes(&self) -> bool {
        self.alternate_rom_register & 0xc0 == 0xc0
    }

    fn alternate_rom_bank(&self) -> usize {
        if self.alternate_rom_register & 0x20 != 0 {
            1
        } else if self.alternate_rom_register & 0x10 != 0 {
            0
        } else {
            usize::from(self.zx128_rom_low != 0)
        }
    }

    fn rom_slot_selected(&self, addr: u16) -> bool {
        let slot = usize::from(addr) / PAGE_SIZE;
        self.mmu_page(slot) == 0xff
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

    /// Whether the FPGA IPL currently overlays the low 8 KiB.
    #[must_use]
    pub fn is_boot_rom_enabled(&self) -> bool {
        self.boot_enabled
    }

    /// Consume a pending `DivMMC` NMI generated through `NextReg` `$02`.
    pub fn take_divmmc_nmi(&mut self) -> bool {
        std::mem::take(&mut self.divmmc_nmi_pending)
    }

    /// Consume a hard- or soft-reset request from `NextReg` `$02`.
    pub fn take_reset_request(&mut self) -> Option<u8> {
        let request = std::mem::take(&mut self.reset_pending);
        (request != 0).then_some(if request & 0x02 != 0 { 0x02 } else { 0x01 })
    }

    /// Record the hard/soft reset cause after restoring the hardware state.
    pub fn set_reset_status(&mut self, status: u8) {
        self.reset_register = status & 0x03;
    }

    /// CPU port read, including the Next register select/access ports.
    #[must_use]
    pub fn in_port(&mut self, port: u16) -> u8 {
        self.in_port_at(port, 0)
    }

    /// CPU port read with its absolute T-state, for timed devices such as SD SPI.
    pub fn in_port_at(&mut self, port: u16, t: u64) -> u8 {
        match port {
            _ if port & 0xff == 0x6b => self.dma.read(),
            _ if Self::is_128_paging_port(port) => 0xff,
            _ if port & 0xc00f == 0x8005 => {
                (self.ay_chip_select << 6) | self.selected_ay().map_or(0, |ay| ay.selected & 0x1f)
            }
            _ if port & PORT_AY_SELECT_MASK == PORT_AY_SELECT => self.read_ay_data(),
            _ if port & PORT_AY_DATA_MASK == PORT_AY_DATA => self.read_ay_data(),
            PORT_NEXTREG_SELECT => self.selected_nextreg,
            PORT_NEXTREG_ACCESS => self.read_nextreg_at(self.selected_nextreg, t),
            PORT_LAYER2_CONTROL => u8::from(self.next_video.visible()) << 1,
            PORT_SPRITE_SELECT => 0,
            _ if port & 0xff == 0xe3 => {
                (self.divmmc_control & 0x7f) | (u8::from(self.divmmc_conmem) << 7)
            }
            _ if port & 0xff == PORT_NEXT_SD_CS => self.sd_cs,
            _ if port & 0xff == PORT_NEXT_SD_DATA => self
                .sd_spi
                .as_mut()
                .map_or(0xff, |sd| sd.exchange_at(0xff, t)),
            _ if port & 0xff == 0x1f => self.kempston.read(),
            _ if port & 1 == 0 => 0xa0 | self.keyboard.read((port >> 8) as u8),
            _ => 0xff,
        }
    }

    /// CPU port write, including the Next register select/access ports.
    pub fn out_port(&mut self, port: u16, value: u8) {
        self.out_port_at(port, value, 0);
    }

    /// CPU port write with its absolute T-state, for timed devices such as SD SPI.
    pub fn out_port_at(&mut self, port: u16, value: u8, t: u64) -> u32 {
        if self.write_dac_alias(port as u8, value, t) {
            return 0;
        }
        if port & 0xff == 0x6b {
            return self.write_dma(value, t);
        }
        match port {
            _ if Self::is_128_paging_port(port) => self.write_128_paging(port, value),
            _ if port & PORT_AY_SELECT_MASK == PORT_AY_SELECT => self.write_ay_select(value),
            _ if port & PORT_AY_DATA_MASK == PORT_AY_DATA => self.write_ay_data(value),
            PORT_NEXTREG_SELECT => {
                self.selected_nextreg = value;
            }
            PORT_NEXTREG_ACCESS => self.write_nextreg_at(self.selected_nextreg, value, t),
            PORT_LAYER2_CONTROL => self.next_video.write_visibility_port(value),
            PORT_SPRITE_SELECT => self
                .next_sprites
                .select_port(value, self.peripheral2 & 0x10 != 0),
            _ if port & 0xff == 0x57 => self
                .next_sprites
                .write_attribute_port(value, self.peripheral2 & 0x10 != 0),
            _ if port & 0xff == 0x5b => self.next_sprites.write_pattern_port(value),
            _ if port & 0xff == 0xe3 => {
                self.divmmc_control = (value & 0x8f) | (self.divmmc_control & 0x40);
                if value & 0x40 != 0 {
                    self.divmmc_control |= 0x40;
                }
                self.divmmc_conmem = value & 0x80 != 0;
            }
            _ if port & 0xff == PORT_NEXT_SD_CS => {
                self.sd_cs = value;
                if let Some(sd) = &mut self.sd_spi {
                    sd.select(Self::sd_selected(value));
                }
            }
            _ if port & 0xff == PORT_NEXT_SD_DATA => {
                if let Some(sd) = &mut self.sd_spi {
                    let _ = sd.exchange_at(value, t);
                }
            }
            _ if port & 0xff == 0xff => self.next_video.write_timex_video_port(value),
            _ if port & 1 == 0 => self.border = value & 0x07,
            _ => {}
        }
        0
    }

    /// Advance burst-mode DMA events through an absolute machine T-state.
    pub fn advance_dma(&mut self, through_t: u64) {
        while self.dma.active() && self.dma.uses_burst_wait() {
            let Some(at) = self.dma.scheduled_t().filter(|at| *at <= through_t) else {
                break;
            };
            self.transfer_dma_byte(at);
        }
    }

    fn write_dma(&mut self, value: u8, t: u64) -> u32 {
        let action = self.dma.write(value);
        match action {
            DmaAction::Reset | DmaAction::Disable | DmaAction::None => 0,
            DmaAction::Load => {
                self.dma.loaded();
                0
            }
            DmaAction::Continue => {
                self.dma.continued();
                0
            }
            DmaAction::Enable => {
                self.dma.enable(t);
                if self.dma.active() && !self.dma.uses_burst_wait() {
                    let byte_count = usize::from(self.dma.remaining());
                    let stall = self.dma.continuous_stall();
                    let byte_period = self.dma.period(self.dma.byte_cycles());
                    for index in 0..byte_count {
                        self.transfer_dma_byte(t.saturating_add(index as u64 * byte_period));
                    }
                    stall
                } else {
                    0
                }
            }
        }
    }

    fn transfer_dma_byte(&mut self, t: u64) {
        let mut dma = std::mem::take(&mut self.dma);
        if !dma.active() {
            self.dma = dma;
            return;
        }
        let (port_a, port_b, endpoint_a, endpoint_b, a_to_b) = dma.transfer_ports();
        let (source, source_endpoint, destination, destination_endpoint) = if a_to_b {
            (port_a, endpoint_a, port_b, endpoint_b)
        } else {
            (port_b, endpoint_b, port_a, endpoint_a)
        };
        let value = self.dma_read_endpoint(source, source_endpoint, t);
        self.dma_write_endpoint(destination, destination_endpoint, value, t);
        let byte_cycles = dma.byte_cycles();
        dma.complete_byte(t, byte_cycles);
        self.dma = dma;
    }

    fn dma_read_endpoint(&mut self, address: u16, endpoint: Endpoint, t: u64) -> u8 {
        match endpoint {
            Endpoint::Memory => self.read(address),
            Endpoint::Io => self.in_port_at(address, t),
        }
    }

    fn dma_write_endpoint(&mut self, address: u16, endpoint: Endpoint, value: u8, t: u64) {
        match endpoint {
            Endpoint::Memory => self.write(address, value),
            Endpoint::Io => {
                let _ = self.out_port_at(address, value, t);
            }
        }
    }

    fn is_128_paging_port(port: u16) -> bool {
        port & 0x8003 == 0x0001 || port & 0xf003 == 0xd001 || port & 0xf003 == 0x1001
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::next_sd::SECTOR_SIZE;

    #[test]
    fn copper_stops_current_batch_when_move_changes_control_mode() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        for byte in [0x62, 0x00, 0x15, 0x08] {
            bus.write_nextreg(0x60, byte);
        }
        bus.write_nextreg(0x62, 0x40);

        let writes = bus.advance_copper(10);
        assert_eq!(writes, [(0, 0x62, 0x00)]);
        bus.write_nextreg_at(0x62, 0x00, writes[0].0);

        assert_eq!(bus.advance_copper(20), []);
    }

    #[test]
    fn sprite_clip_window_writes_are_tracked_as_video_state() {
        assert!(NextBus::nextreg_affects_video(0x19));
    }

    fn rgb3(value: u8) -> u8 {
        (u16::from(value) * 255 / 7) as u8
    }

    #[test]
    fn specdrum_ports_update_all_dac_aliases_and_keep_values_held() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        for (port, value, expected) in [
            (0x1fu8, 0x11, [0x11, 0x80, 0x80, 0x80]),
            (0xf1, 0x12, [0x12, 0x80, 0x80, 0x80]),
            (0x3f, 0x13, [0x13, 0x80, 0x80, 0x80]),
            (0x0f, 0x21, [0x13, 0x21, 0x80, 0x80]),
            (0xf3, 0x22, [0x13, 0x22, 0x80, 0x80]),
            (0xdf, 0x31, [0x31, 0x22, 0x80, 0x31]),
            (0xfb, 0x32, [0x32, 0x22, 0x80, 0x32]),
            (0xb3, 0x41, [0x32, 0x41, 0x41, 0x32]),
            (0x4f, 0x51, [0x32, 0x41, 0x51, 0x32]),
            (0xf9, 0x52, [0x32, 0x41, 0x52, 0x32]),
            (0x5f, 0x61, [0x32, 0x41, 0x52, 0x61]),
        ] {
            bus.out_port_at(u16::from(port), value, 1234);
            assert_eq!(bus.dac_values(), expected, "port ${port:02x}");
        }
        assert_eq!(bus.dac_values(), [0x32, 0x41, 0x52, 0x61]);
    }

    #[test]
    fn dac_nextreg_mirrors_write_the_documented_channels() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        bus.write_nextreg_at(0x2c, 0x22, 12);
        bus.write_nextreg_at(0x2d, 0x33, 34);
        bus.write_nextreg_at(0x2e, 0x44, 56);
        assert_eq!(bus.dac_values(), [0x33, 0x22, 0x44, 0x33]);
        let mut pending = Vec::new();
        let write_capacity = bus.dac_writes.capacity();
        bus.drain_dac_writes_into(&mut pending);
        assert_eq!(pending.len(), 4);
        assert_eq!(bus.dac_writes.capacity(), write_capacity);
        assert_eq!(bus.read_nextreg(0x2c), 0xff, "Pi I2S read source is absent");
        assert_eq!(bus.read_nextreg(0x2d), 0xff);
        assert_eq!(bus.read_nextreg(0x2e), 0xff);
    }

    #[test]
    fn dac_output_gate_is_controlled_by_peripheral_three_bit_three() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        assert!(!bus.dacs_enabled());
        bus.write_nextreg(0x08, 0x18);
        assert!(bus.dacs_enabled());
        bus.write_nextreg(0x08, 0x10);
        assert!(!bus.dacs_enabled());
    }

    #[test]
    fn turbo_sound_ports_keep_three_independent_chips_and_report_selection() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        bus.write_nextreg(0x08, 0x12); // Enable Turbo Sound.

        for (control, marker, status) in
            [(0xff, 0x11, 0xc8), (0xfe, 0x22, 0x88), (0xfd, 0x33, 0x48)]
        {
            bus.out_port(0xfffd, control);
            bus.out_port(0xfffd, 8);
            bus.out_port(0xbffd, marker);
            assert_eq!(bus.in_port(0xbff5), status);
            assert_eq!(bus.in_port(0xfffd), marker & 0x1f);
            assert_eq!(bus.in_port(0xbffd), marker & 0x1f);
        }
        for (index, expected) in [0x11, 0x02, 0x13].into_iter().enumerate() {
            assert_eq!(bus.ay_chip(index).map(|ay| ay.regs[8]), Some(expected));
        }

        bus.out_port(0x8001, 0x00); // Does not match the documented $BFFD mask.
        assert_eq!(bus.ay_chip(2).map(|ay| ay.regs[8]), Some(0x13));
        assert_eq!(bus.in_port(0x8001), 0xff);
    }

    fn program_tone_b(bus: &mut NextBus, control: u8) {
        bus.out_port(0xfffd, control);
        for (register, value) in [(2, 1), (3, 0), (7, 0x3d), (9, 0x0f)] {
            bus.out_port(0xfffd, register);
            bus.out_port(0xbffd, value);
        }
    }

    #[test]
    fn turbo_sound_obeys_stereo_mapping_channel_enable_and_chip_mono() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        bus.write_nextreg(0x08, 0x32); // Turbo Sound + ACB.
        program_tone_b(&mut bus, 0xff); // AY0, both channels enabled.
        bus.advance_audio(32);
        let (acb_mono, acb_left, acb_right) = bus.audio_sample();
        assert!(acb_mono > 0.0);
        assert!(
            (acb_left - 0.5).abs() < 1e-6,
            "ACB routes channel B to the right"
        );
        assert!(acb_right > 0.5);

        bus.write_nextreg(0x08, 0x12); // ABC: B is centered.
        let (_, centered_left, centered_right) = bus.audio_sample();
        assert!((centered_left - centered_right).abs() < 1e-6);

        bus.out_port(0xfffd, 0xbf); // AY0, right output only.
        let (_, muted_left, right_only) = bus.audio_sample();
        assert!((muted_left - 0.5).abs() < 1e-6);
        assert!(right_only > 0.0);

        bus.out_port(0xfffd, 0xff); // Restore both outputs.
        bus.write_nextreg(0x09, 0x20); // AY0 mono.
        let (_, mono_left, mono_right) = bus.audio_sample();
        assert!((mono_left - mono_right).abs() < 1e-6);
    }

    #[test]
    fn three_chip_mix_is_normalized_to_single_chip_amplitude() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        bus.write_nextreg(0x08, 0x12);
        for control in [0xff, 0xfe, 0xfd] {
            program_tone_b(&mut bus, control);
        }
        bus.advance_audio(32);
        let three_chip = bus.audio_sample();
        bus.write_nextreg(0x08, 0x10); // Keep selected AY2 as the single chip.
        let one_chip = bus.audio_sample();
        assert!((three_chip.0 - one_chip.0).abs() < 1e-6);
        assert!((three_chip.1 - one_chip.1).abs() < 1e-6);
        assert!((three_chip.2 - one_chip.2).abs() < 1e-6);
    }

    #[test]
    fn ay_mode_hold_resets_chips_and_suspends_io_and_synthesis() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        assert_eq!(
            bus.read_nextreg(0x06) & 0x03,
            0,
            "hard reset selects YM mode"
        );
        bus.write_nextreg(0x08, 0x12);
        bus.out_port(0xfffd, 0xfe); // AY1.
        bus.out_port(0xfffd, 8);
        bus.out_port(0xbffd, 0x0f);
        assert_eq!(bus.ay_chip(1).map(|ay| ay.regs[8]), Some(0x0f));

        bus.write_nextreg(0x06, 0x03); // Hold every AY in reset.
        assert_eq!(bus.ay_chip(1).map(|ay| ay.regs[8]), Some(0));
        bus.out_port(0xfffd, 8);
        bus.out_port(0xbffd, 0x0f);
        assert_eq!(bus.ay_chip(1).map(|ay| ay.regs[8]), Some(0));
        bus.advance_audio(32 * 100);
        assert_eq!(bus.audio_sample(), (0.0, 0.0, 0.0));

        bus.write_nextreg(0x06, 0x01); // AY mode resumes.
        bus.out_port(0xbffd, 0x0f);
        assert_eq!(bus.ay_chip(1).map(|ay| ay.regs[8]), Some(0x0f));

        bus.hard_reset();
        assert_eq!(bus.read_nextreg(0x06) & 0x03, 0);
        assert_eq!(bus.read_nextreg(0x06) & 0xa0, 0);
        bus.soft_reset();
        assert_eq!(bus.read_nextreg(0x06), 0xa0);
        assert_eq!(bus.read_nextreg(0x06) & 0x03, 0, "soft reset keeps YM mode");
    }

    #[test]
    fn nextreg_audio_mode_selects_the_psg_dac_response() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        program_tone_b(&mut bus, 0xff);
        bus.out_port(0xfffd, 9);
        bus.out_port(0xbffd, 1);
        bus.advance_audio(32);
        let ym_level = bus.ay_chip(0).expect("selected chip").channel_levels()[1];

        bus.write_nextreg(0x06, 0x01);
        let ay_level = bus.ay_chip(0).expect("selected chip").channel_levels()[1];

        assert!(
            (ym_level - ay_level).abs() > 1e-6,
            "YM and AY modes use distinct DAC curves"
        );
    }

    #[test]
    fn peripheral4_bit_two_reports_hdmi_audio_mute() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        assert!(!bus.audio_muted());
        bus.write_nextreg(0x09, 0x04);
        assert!(bus.audio_muted());
        bus.write_nextreg(0x09, 0xfb);
        assert!(!bus.audio_muted());
    }

    #[test]
    fn disabling_turbo_sound_freezes_chip_selection_but_keeps_active_chip_running() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        bus.write_nextreg(0x08, 0x12);
        bus.out_port(0xfffd, 0xfe); // Select AY1.
        program_tone_b(&mut bus, 0xfe);
        bus.advance_audio(32);
        bus.write_nextreg(0x08, 0x10); // Disable Turbo Sound.
        let held_sample = bus.audio_sample();
        assert!(held_sample.0 > 0.0);
        bus.advance_audio(32 * 19);
        assert_ne!(
            bus.audio_sample(),
            held_sample,
            "selected AY keeps clocking"
        );
        assert_eq!(bus.in_port(0xbff5) & 0xc0, 0x80);
        // Register accesses still address the active chip, while chip switching
        // commands are ignored with Turbo Sound disabled.
        bus.out_port(0xfffd, 0xff);
        bus.out_port(0xfffd, 8);
        bus.out_port(0xbffd, 0x07);
        assert_eq!(bus.ay_chip(1).map(|ay| ay.regs[8]), Some(0x07));
        assert_eq!(bus.in_port(0xbff5) & 0xc0, 0x80);
    }

    #[test]
    fn nonselected_ay_oscillators_keep_running_with_turbo_sound_disabled() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        bus.write_nextreg(0x08, 0x12);
        program_tone_b(&mut bus, 0xff); // AY0.
        program_tone_b(&mut bus, 0xfe); // AY1.
        bus.advance_audio(32);
        bus.out_port(0xfffd, 0xff); // Select AY0.
        bus.write_nextreg(0x08, 0x10); // Disable Turbo Sound.

        let other_chip_before = bus.ay_chip(1).map(Ay8912::sample_mono);
        bus.advance_audio(32);
        assert_ne!(
            bus.ay_chip(1).map(Ay8912::sample_mono),
            other_chip_before,
            "all chip oscillators keep advancing while Turbo Sound is disabled"
        );
        assert_eq!(bus.in_port(0xbff5) & 0xc0, 0xc0);
        bus.out_port(0xfffd, 0xfe); // Chip-selection commands are frozen.
        assert_eq!(bus.in_port(0xbff5) & 0xc0, 0xc0);
    }

    #[test]
    fn boot_rom_overlays_slot_zero_until_nextreg_configuration_write() {
        let main_rom = vec![0x33; NEXT_ROM_SIZE];
        let boot_rom = vec![0x11; 8192];
        let mut bus = NextBus::new(&main_rom).expect("valid main ROM");
        bus.install_boot_rom(&boot_rom).expect("valid IPL ROM");
        assert_eq!(bus.read(0), 0x11);
        assert_eq!(bus.read_nextreg(0x02) & 0x03, 0x02);
        assert_eq!(bus.read(0x1fff), 0x11);
        assert_eq!(bus.read(0x2000), 0x33);
        assert_eq!(bus.read_nextreg(0x03), 0);
        assert_eq!(bus.read_nextreg(0x04), 0xff);
        bus.write_nextreg(0x03, 0x03);
        assert_eq!(bus.read(0), 0x33);
        assert_eq!(bus.read_nextreg(0x03), 3);
        bus.hard_reset();
        assert_eq!(bus.read(0), 0x11);
    }

    #[test]
    fn config_mapping_uses_raw_sram_banks_without_rewiring_the_mmu() {
        let mut rom = vec![0; NEXT_ROM_SIZE];
        rom[0x8000] = 0x42;
        let mut bus = NextBus::new(&rom).expect("valid ROM");
        let mut boot_rom = vec![0; 8192];
        boot_rom[0] = 0x42;
        bus.install_boot_rom(&boot_rom).expect("valid IPL ROM");
        bus.write_nextreg(0x04, 2);
        assert_eq!(bus.read(0), 0x42);
        bus.write_nextreg(0x03, 0);
        bus.write_nextreg(0x04, 2);
        assert!(bus.load_ram_page(4, 0, &[0x11]));
        assert!(bus.load_ram_page(5, 0, &[0x22]));
        assert_eq!(bus.read(0), 0x42);
        assert_eq!(bus.read(0x2000), 0);
        assert_eq!(bus.read_nextreg(0x50), 0xff);
        assert_eq!(bus.read_nextreg(0x51), 0xff);
        bus.write(3, 0x99);
        assert_eq!(bus.read(3), 0x99);
        assert_eq!(bus.read_ram_page(4, 0), Some(0x11));

        bus.write_nextreg(0x04, 18);
        assert_eq!(bus.read(0), 0x11);
        assert_eq!(bus.read(0x2000), 0x22);
    }

    #[test]
    fn config_mapping_pages_six_and_seven_are_alternate_roms() {
        for (page, marker, bank) in [(6, 0x61, 0x90), (7, 0x72, 0xa0)] {
            let mut bus = NextBus::new(&vec![0x33; NEXT_ROM_SIZE]).expect("valid ROM");
            bus.install_boot_rom(&vec![0; 8192]).expect("valid IPL ROM");
            bus.write_nextreg(0x03, 0); // Leave the IPL in config mode.
            bus.write_nextreg(0x04, page);
            bus.write(0x0048, marker);
            bus.write_nextreg(0x03, 3); // Exit config mode.
            bus.write_nextreg(0x8c, bank);
            assert_eq!(bus.read(0x0048), marker);
        }
    }

    #[test]
    fn alternate_rom_write_and_read_redirect_roundtrip() {
        let mut rom = vec![0; NEXT_ROM_SIZE];
        rom[0] = 0x33;
        rom[1] = 0x44;
        let mut bus = NextBus::new(&rom).expect("valid ROM");
        bus.write_nextreg(0x03, 3);
        bus.write_nextreg(0x8c, 0xd0); // Enable write redirect, lock Alt-ROM 0.
        bus.write(1, 0x99);
        assert_eq!(bus.read(1), 0x44, "write mode leaves normal ROM visible");
        bus.write_nextreg(0x8c, 0x90); // Switch to Alt-ROM 0 read redirect.
        assert_eq!(bus.read(1), 0x99);
        assert_eq!(bus.read_nextreg(0x8c), 0x90);
    }

    #[test]
    fn alternate_rom_write_redirect_leaves_ram_mapped_slots_writable() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        bus.write_nextreg(0x03, 3);
        bus.write_nextreg(0x50, 20);
        bus.write_nextreg(0x8c, 0xd0); // Alt-ROM enabled for writes only.

        bus.write(1, 0x99);

        assert_eq!(bus.read_ram_page(20, 1), Some(0x99));
    }

    #[test]
    fn spectrum_128_rom_mapping_register_selects_each_16k_rom_bank() {
        let mut rom = vec![0; NEXT_ROM_SIZE];
        for bank in 0..4 {
            let base = bank * CONFIG_BANK_SIZE;
            rom[base] = 0x10 + bank as u8;
            rom[base + 0x2000] = 0x20 + bank as u8;
        }
        let mut bus = NextBus::new(&rom).expect("valid ROM");

        for bank in 0..4u8 {
            let mapping = 0x08 | (bank & 0x03);
            bus.write_nextreg(0x8e, mapping);
            assert_eq!(bus.read_nextreg(0x8e), mapping);
            assert_eq!(bus.read(0x0000), 0x10 + bank);
            assert_eq!(bus.read(0x2000), 0x20 + bank);
        }
    }

    #[test]
    fn spectrum_128_mapping_register_updates_or_preserves_ram_bank_by_write_bit() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        assert_eq!(bus.read_nextreg(0x56), 0);
        assert_eq!(bus.read_nextreg(0x57), 1);
        assert!(bus.load_ram_page(18, 0, &[0x98]));
        assert!(bus.load_ram_page(19, 0, &[0x99]));
        bus.write_nextreg(0x50, 20);
        bus.write_nextreg(0x51, 21);

        bus.write_nextreg(0x8e, 0x98); // Update 128K bank to 9.
        assert_eq!(bus.read_nextreg(0x50), 0xff);
        assert_eq!(bus.read_nextreg(0x51), 0xff);
        assert_eq!(bus.read_nextreg(0x56), 18);
        assert_eq!(bus.read_nextreg(0x57), 19);
        assert_eq!(bus.read(0xc000), 0x98);
        assert_eq!(bus.read(0xe000), 0x99);

        bus.write_nextreg(0x8e, 0x10); // Bit 3 clear: don't change RAM bank.
        assert_eq!(bus.read_nextreg(0x8e), 0x98); // Read bit 3 is always set.
        assert_eq!(bus.read_nextreg(0x56), 18);
        assert_eq!(bus.read_nextreg(0x57), 19);
        assert_eq!(bus.read(0xc000), 0x98);
        assert_eq!(bus.read(0xe000), 0x99);
    }

    #[test]
    fn spectrum_128_special_mapping_selects_the_four_all_ram_layouts() {
        let layouts = [[0, 1, 2, 3], [4, 5, 6, 7], [4, 5, 6, 3], [4, 7, 6, 3]];
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        for bank in 0..8u8 {
            for half in 0..2u8 {
                let page = bank * 2 + half;
                assert!(bus.load_ram_page(page, 0, &[bank]));
            }
        }

        for (selection, banks) in layouts.into_iter().enumerate() {
            bus.write_nextreg(0x8e, 0x0c | selection as u8);
            for (slot, bank) in banks.into_iter().enumerate() {
                assert_eq!(bus.read((slot * 0x4000) as u16), bank);
                assert_eq!(bus.read((slot * 0x4000 + 0x2000) as u16), bank);
            }
        }
    }

    #[test]
    fn even_ports_read_the_active_low_keyboard_matrix() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        bus.keyboard.set_key(6, 0, true);
        assert_eq!(bus.in_port(0xfefe), 0xbf);
        assert_eq!(bus.in_port(0xbffe), 0xbe);
        assert_eq!(bus.in_port(0xfdfe), 0xbf);
        assert_eq!(bus.in_port(0xfdff), 0xff);
    }

    #[test]
    fn spectrum_space_key_uses_row_seven_bit_zero() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        bus.keyboard.set_key(7, 0, true);
        assert_eq!(bus.in_port(0x7ffe), 0xbe);
        assert_eq!(bus.in_port(0xfefe), 0xbf);
    }

    #[test]
    fn active_video_line_registers_follow_the_312_line_raster() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        bus.out_port(PORT_NEXTREG_SELECT, 0x1f);
        assert_eq!(
            bus.in_port_at(PORT_NEXTREG_ACCESS, 230 * TSTATES_PER_LINE),
            230
        );
        assert_eq!(
            bus.in_port_at(PORT_NEXTREG_ACCESS, 312 * TSTATES_PER_LINE),
            0
        );
        bus.out_port(PORT_NEXTREG_SELECT, 0x1e);
        assert_eq!(
            bus.in_port_at(PORT_NEXTREG_ACCESS, 256 * TSTATES_PER_LINE),
            1
        );
    }

    #[test]
    fn display_timing_selects_frame_and_scanline_geometry() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        for (timing, line_tstates, lines, interrupt_tstates) in [
            (0, 224, 312, 32),
            (1, 224, 312, 32),
            (2, 228, 311, 36),
            (3, 228, 311, 36),
            (4, 224, 320, 32),
            (5, 224, 312, 32),
            (6, 224, 312, 32),
            (7, 224, 312, 32),
        ] {
            bus.hard_reset();
            bus.write_nextreg(0x03, 0x80 | timing << 4 | 3);
            let frame_tstates = line_tstates * lines;
            assert_eq!(
                bus.frame_interrupt_timing(),
                (frame_tstates, interrupt_tstates),
                "timing {timing}"
            );
            assert_eq!(
                bus.read_nextreg_at(0x1f, frame_tstates),
                0,
                "timing {timing}"
            );
            assert_eq!(
                bus.read_nextreg_at(0x1f, (lines - 1) * line_tstates),
                (lines - 1) as u8,
                "timing {timing}"
            );
        }
    }

    #[test]
    fn layer2_active_ram_bank_is_readable_and_resets_to_bank_eight() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        assert_eq!(bus.read_nextreg(0x12), 8);
        bus.write_nextreg(0x12, 0x88);
        assert_eq!(bus.read_nextreg(0x12), 8);
        bus.write_nextreg(0x12, 9);
        assert_eq!(bus.read_nextreg(0x12), 9);
        bus.hard_reset();
        assert_eq!(bus.read_nextreg(0x12), 8);
    }

    #[test]
    fn layer2_visibility_is_mirrored_between_register_and_control_port() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        bus.out_port(PORT_LAYER2_CONTROL, 0x02);
        assert_eq!(bus.read_nextreg(0x69), 0x80);
        assert_eq!(bus.in_port(PORT_LAYER2_CONTROL), 0x02);

        bus.write_nextreg(0x69, 0x02);
        assert_eq!(bus.in_port(PORT_LAYER2_CONTROL), 0);
        bus.write_nextreg(0x69, 0x80);
        assert_eq!(bus.in_port(PORT_LAYER2_CONTROL), 0x02);
        bus.write_nextreg(0x69, 0);
        assert_eq!(bus.in_port(PORT_LAYER2_CONTROL), 0);

        bus.out_port(PORT_LAYER2_CONTROL, 0x12);
        assert_eq!(bus.read_nextreg(0x69), 0);
        bus.out_port(PORT_LAYER2_CONTROL, 0x10);
        assert_eq!(bus.read_nextreg(0x69), 0);

        bus.out_port(PORT_LAYER2_CONTROL, 0x02);
        assert_eq!(bus.read_nextreg(0x69), 0x80);
        bus.out_port(PORT_LAYER2_CONTROL, 0x17);
        assert_eq!(bus.read_nextreg(0x69), 0x80);
    }

    #[test]
    fn timex_display_file_selector_is_aliased_by_nextreg_69_and_port_ff() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");

        bus.out_port(0x12ff, 0x25);
        assert_eq!(bus.read_nextreg(0x69), 0x25);
        assert_eq!(bus.radastan_display_file_offset(), 0x2000);

        bus.write_nextreg(0x69, 0x12);
        assert_eq!(bus.read_nextreg(0x69), 0x12);
        assert_eq!(bus.radastan_display_file_offset(), 0);
    }

    #[test]
    fn layer2_palettes_reset_to_identity_rgb332_with_auto_increment_enabled() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        assert_eq!(bus.read_nextreg(0x43), 0);

        for palette_control in [0x10, 0x14] {
            bus.write_nextreg(0x43, palette_control);
            for index in 0..=u8::MAX {
                bus.write_nextreg(0x40, index);
                assert_eq!(bus.read_nextreg(0x41), index);
                assert_eq!(
                    bus.read_nextreg(0x44),
                    u8::from(index & 0x03 != 0),
                    "palette control {palette_control:#04x}, index {index:#04x}"
                );
            }
        }
    }

    #[test]
    fn standard_layer2_pixels_cross_the_three_physical_bank_boundaries() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        bus.write_nextreg(0x12, 8);
        for (offset, value) in [
            (0, 0x11),
            (16_383, 0x22),
            (16_384, 0x33),
            (32_768, 0x44),
            (49_151, 0x55),
        ] {
            let page = 16 + offset / PAGE_SIZE;
            let page_offset = offset % PAGE_SIZE;
            assert!(bus.load_ram_page(page as u8, page_offset, &[value]));
            assert_eq!(bus.layer2_pixel(offset % 256, offset / 256), value);
        }
    }

    #[test]
    fn layer2_palette_selection_increment_and_nine_bit_readback() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        bus.write_nextreg(0x43, 0x10); // Layer 2 palette 1, auto-increment.
        bus.write_nextreg(0x40, 7);
        bus.write_nextreg(0x44, 0b1010_1110); // R=5, G=3, B low=2.
        assert_eq!(bus.read_nextreg(0x03) & 0x80, 0x80);
        bus.write_nextreg(0x44, 0x81); // Priority plus blue MSB.
        assert_eq!(bus.read_nextreg(0x03) & 0x80, 0);
        assert_eq!(bus.read_nextreg(0x40), 8);
        bus.write_nextreg(0x40, 7);
        assert_eq!(bus.read_nextreg(0x44), 0x81);
        assert_eq!(bus.read_nextreg(0x40), 7, "9-bit reads do not increment");
        assert_eq!(bus.read_nextreg(0x41), 0b1010_1110);
        assert_eq!(
            bus.next_video.layer2_color(7),
            VideoColor {
                rgb: [rgb3(5), rgb3(3), rgb3(6)],
                priority: true,
                transparent: false,
            }
        );
        bus.soft_reset();
        assert_eq!(
            bus.next_video.layer2_color(7),
            VideoColor {
                rgb: [rgb3(5), rgb3(3), rgb3(6)],
                priority: true,
                transparent: false,
            }
        );

        bus.write_nextreg(0x70, 3);
        assert_eq!(bus.next_video.palette_index(0x1a), 0x4a);

        bus.write_nextreg(0x43, 0x00); // ULA palette 1 is independently selected.
        bus.write_nextreg(0x40, 7);
        assert_eq!(bus.read_nextreg(0x41), 7);

        bus.write_nextreg(0x43, 0x50); // Write the second Layer 2 palette.
        bus.write_nextreg(0x40, 7);
        bus.write_nextreg(0x41, 0b0011_1001);
        bus.write_nextreg(0x43, 0x54); // Select and display second Layer 2 palette.
        bus.write_nextreg(0x40, 7);
        assert_eq!(
            bus.read_nextreg(0x44),
            0x01,
            "8-bit blue extension reads back"
        );
        assert_eq!(
            bus.next_video.layer2_color(7),
            VideoColor {
                rgb: [rgb3(1), rgb3(6), rgb3(5)],
                priority: false,
                transparent: false,
            }
        );

        bus.write_nextreg(0x43, 0x10); // Display first palette again.
        assert_eq!(
            bus.next_video.layer2_color(7),
            VideoColor {
                rgb: [rgb3(5), rgb3(3), rgb3(6)],
                priority: true,
                transparent: false,
            }
        );
    }

    #[test]
    fn eight_bit_palette_read_uses_selected_index_without_incrementing() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        bus.write_nextreg(0x43, 0x10); // Layer 2 palette 1, auto-increment enabled.
        bus.write_nextreg(0x40, 3);
        bus.write_nextreg(0x41, 0b1010_1110);
        assert_eq!(bus.read_nextreg(0x40), 4, "write auto-increments");

        bus.write_nextreg(0x40, 3);
        assert_eq!(bus.read_nextreg(0x41), 0b1010_1110);
        assert_eq!(bus.read_nextreg(0x40), 3, "$41 read does not increment");

        bus.write_nextreg(0x40, 4);
        bus.write_nextreg(0x41, 0b0011_1001);
        bus.write_nextreg(0x40, 3);
        assert_eq!(bus.read_nextreg(0x41), 0b1010_1110);
    }

    #[test]
    fn sprite_ports_keep_pattern_and_attribute_cursors_independent() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        bus.out_port(PORT_SPRITE_SELECT, 0);
        for _ in 0..128 {
            bus.out_port(0x005b, 1);
        }
        bus.out_port(PORT_SPRITE_SELECT, 0x80);
        for _ in 0..128 {
            bus.out_port(0x005b, 2);
        }

        for attribute in [32, 32, 0, 0x80] {
            bus.out_port(0x0057, attribute);
        }
        bus.write_nextreg(0x15, 1);

        let surface = bus.render_sprite_surface();
        assert_eq!(surface[32 * 320 + 32].unwrap().rgb, [0, 0, 182]);
        assert_eq!(surface[40 * 320 + 32].unwrap().rgb, [0, 0, 218]);
        assert!(surface[48 * 320 + 32].is_none());
    }

    #[test]
    fn sprite_ports_wrap_attribute_slots_and_nextreg_lockstep_is_explicit() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        bus.out_port(PORT_SPRITE_SELECT, 0);
        bus.out_port(0x005b, 1);
        bus.out_port(PORT_SPRITE_SELECT, 127);
        for value in [32, 32, 0, 0x80, 64, 32, 0, 0x80] {
            bus.out_port(0x0057, value);
        }
        bus.write_nextreg(0x15, 1);
        let surface = bus.render_sprite_surface();
        assert!(surface[32 * 320 + 32].is_some());
        assert!(surface[32 * 320 + 64].is_some());

        bus.write_nextreg(0x34, 5);
        bus.out_port(PORT_SPRITE_SELECT, 127);
        for value in [32, 32, 0, 0x80] {
            bus.out_port(0x0057, value);
        }
        assert_eq!(
            bus.read_nextreg(0x34),
            5,
            "interfaces are independent by default"
        );

        bus.write_nextreg(0x09, 0x10);
        bus.write_nextreg(0x34, 127);
        assert_eq!(bus.read_nextreg(0x34), 127);
        assert_eq!(bus.in_port(PORT_SPRITE_SELECT), 0);
        bus.out_port(PORT_SPRITE_SELECT, 0);
        assert_eq!(bus.read_nextreg(0x34), 0);

        bus.write_nextreg(0x34, 127);
        bus.write_nextreg(0x75, 32);
        assert_eq!(bus.read_nextreg(0x34), 0, "post-increment wraps at 128");
    }

    #[test]
    fn sprite_pattern_write_pointer_wraps_at_16_kibibytes() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        bus.out_port(PORT_SPRITE_SELECT, 0);
        bus.out_port(0x005b, 1);
        for _ in 0..16 * 1024 - 1 {
            bus.out_port(0x005b, 2);
        }
        bus.out_port(0x005b, 3);
        bus.out_port(PORT_SPRITE_SELECT, 0);
        for value in [32, 32, 0, 0x80] {
            bus.out_port(0x0057, value);
        }
        bus.write_nextreg(0x15, 1);
        assert_eq!(
            bus.render_sprite_surface()[32 * 320 + 32].unwrap().rgb,
            [0, 0, 255]
        );
    }

    #[test]
    fn extended_8bit_sprite_uses_ninth_y_bit_and_wraps_at_512() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        bus.out_port(PORT_SPRITE_SELECT, 0);
        for _ in 0..16 {
            bus.out_port(0x005b, 0);
        }
        bus.out_port(0x005b, 0xe0);
        for value in [32, 255, 0, 0xc0, 1] {
            bus.out_port(0x0057, value);
        }
        bus.write_nextreg(0x15, 0x03);

        let surface = bus.render_sprite_surface();
        assert_eq!(surface[32].unwrap().rgb, [255, 0, 0]);
        assert!(surface[40 * 320 + 32].is_none());
    }

    #[test]
    fn four_byte_sprite_attributes_ignore_stale_extended_y_bit() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        bus.out_port(PORT_SPRITE_SELECT, 0);
        bus.out_port(0x005b, 0xe0);
        for value in [32, 32, 0, 0xc0, 1] {
            bus.out_port(0x0057, value);
        }
        bus.out_port(PORT_SPRITE_SELECT, 0);
        for value in [32, 32, 0, 0x80] {
            bus.out_port(0x0057, value);
        }
        bus.write_nextreg(0x15, 1);

        let surface = bus.render_sprite_surface();
        assert_eq!(surface[32 * 320 + 32].unwrap().rgb, [255, 0, 0]);
        assert!(surface[0].is_none());
    }

    #[test]
    fn sprite_palette_transparency_uses_source_index_and_palette_selection() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        bus.write_nextreg(0x43, 0x20); // Sprite palette 1.
        bus.write_nextreg(0x40, 18);
        bus.write_nextreg(0x41, 0b1110_0000);
        bus.write_nextreg(0x40, 18);
        bus.write_nextreg(0x44, 0b1110_0000);
        bus.write_nextreg(0x44, 0x81); // Sprite palettes ignore the reserved priority bit.
        bus.write_nextreg(0x40, 18);
        assert_eq!(bus.read_nextreg(0x44), 0x01);
        bus.write_nextreg(0x43, 0x60); // Sprite palette 2.
        bus.write_nextreg(0x40, 18);
        bus.write_nextreg(0x41, 0);
        bus.write_nextreg(0x4b, 1);
        bus.out_port(PORT_SPRITE_SELECT, 0);
        bus.out_port(0x005b, 1);
        bus.out_port(0x005b, 2);
        for attribute in [32, 32, 0x10, 0x80] {
            bus.out_port(0x0057, attribute);
        }
        bus.write_nextreg(0x15, 1);

        let surface = bus.render_sprite_surface();
        assert!(surface[32 * 320 + 32].is_none());
        assert_eq!(surface[32 * 320 + 33].unwrap().rgb, [255, 0, 145]);

        bus.write_nextreg(0x43, 0x28); // Display the second sprite palette.
        assert_eq!(
            bus.render_sprite_surface()[32 * 320 + 33]
                .expect("visible sprite pixel")
                .rgb,
            [0, 0, 0],
        );
    }

    #[test]
    fn sprite_clip_window_and_over_border_gate_follow_nextregs() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        bus.out_port(PORT_SPRITE_SELECT, 0);
        bus.out_port(0x005b, 1);
        bus.out_port(PORT_SPRITE_SELECT, 0);
        for attribute in [20, 5, 0, 0x80] {
            bus.out_port(0x0057, attribute);
        }
        bus.write_nextreg(0x15, 1);
        assert!(bus.render_sprite_surface()[5 * 320 + 20].is_none());
        bus.write_nextreg(0x19, 10);
        bus.write_nextreg(0x19, 20);
        bus.write_nextreg(0x19, 5);
        bus.write_nextreg(0x19, 9);
        bus.write_nextreg(0x1c, 0x02);
        bus.write_nextreg(0x15, 0x23); // Enabled, over border, clipped.

        let surface = bus.render_sprite_surface();
        assert!(surface[5 * 320 + 19].is_none());
        assert!(surface[5 * 320 + 20].is_some());
        assert!(surface[4 * 320 + 20].is_none());
    }

    #[test]
    fn divmmc_rom_automaps_on_configured_delayed_and_instant_m1_triggers() {
        let configured_bus = || {
            let mut rom = vec![0; NEXT_ROM_SIZE];
            rom[0x67] = 0x76;
            let mut bus = NextBus::new(&rom).expect("valid ROM");
            bus.install_boot_rom(&vec![0; 8192]).expect("valid IPL");
            bus.write_nextreg(0x03, 0);
            bus.write_nextreg(0x04, 4);
            for (offset, byte) in [0x00, 0x3e, 0x55, 0x76].into_iter().enumerate() {
                bus.write(offset as u16 + 0x66, byte);
            }
            bus.write_nextreg(0x03, 3);
            bus.write_nextreg(0x0a, 0x11);
            bus
        };

        let mut delayed = configured_bus();
        assert_eq!(
            delayed.read_opcode(0x66),
            0x00,
            "delayed trigger uses main ROM"
        );
        assert_eq!(
            delayed.read(0x67),
            0x3e,
            "delayed mapping takes effect after the trigger M1 fetch"
        );
        assert_eq!(
            delayed.read_opcode(0x67),
            0x3e,
            "next M1 fetch uses DivMMC ROM"
        );
        assert_eq!(delayed.read(0x68), 0x55);

        let mut instant = configured_bus();
        instant.write_nextreg(0xbb, 0x02);
        assert_eq!(
            instant.read_opcode(0x66),
            0x00,
            "instant trigger still fetches main ROM"
        );
        assert_eq!(instant.read_opcode(0x67), 0x3e);
    }

    #[test]
    fn divmmc_rst_automap_and_return_window_follow_nextreg_controls() {
        let mut rom = vec![0x22; NEXT_ROM_SIZE];
        rom[3 * CONFIG_BANK_SIZE + 0x08] = 0xcf; // ROM 3 RST $08.
        let mut bus = NextBus::new(&rom).expect("valid ROM");
        bus.config_reserved_sram[..PAGE_SIZE].fill(0x33);
        bus.write_nextreg(0x8e, 0x0b); // ROM 3 selected.
        bus.write_nextreg(0x0a, 0x10); // Enable DivMMC automapping.
        bus.write_nextreg(0xb8, 0x02); // RST $08 entry point.
        bus.write_nextreg(0xb9, 0x02); // RST $08 valid outside ROM 3.
        bus.write_nextreg(0xba, 0x02); // RST $08 maps instantly.
        assert_eq!(bus.read_opcode(0x0008), 0x33);
        assert_eq!(bus.read_opcode(0x0009), 0x33);
        assert_eq!(bus.in_port(0x00e3) & 0x80, 0);

        bus.write_nextreg(0xbb, 0x40); // Delayed unmap on $1FF8-$1FFF.
        assert_eq!(bus.read_opcode(0x1ff8), 0x33);
        assert_eq!(bus.read_opcode(0x1fff), 0x22);
    }

    #[test]
    fn rom3_only_rst_automapping_requires_rom3_to_be_visible() {
        let mut rom = vec![0x22; NEXT_ROM_SIZE];
        rom[3 * CONFIG_BANK_SIZE + 0x38] = 0xff; // ROM 3 RST $38.
        let mut bus = NextBus::new(&rom).expect("valid ROM");
        bus.config_reserved_sram[..PAGE_SIZE].fill(0x33);
        bus.write_nextreg(0x03, 3); // +3 personality alone does not mean ROM3 is present.
        bus.write_nextreg(0x0a, 0x10);
        bus.write_nextreg(0xb8, 0x80); // RST $38.
        bus.write_nextreg(0xb9, 0x00); // RST $38 only automaps in ROM3.
        bus.write_nextreg(0x50, 2); // Hide ROM with MMU RAM.
        assert_eq!(bus.read_opcode(0x38), 0x00);
        assert_eq!(bus.read_opcode(0x39), 0x00);

        bus.write_nextreg(0x8e, 0x0b); // Expose ROM3.
        assert_eq!(bus.read_opcode(0x38), 0xff);
        assert_eq!(bus.read_opcode(0x39), 0x33);
    }

    #[test]
    fn rst_automap_uses_fetch_address_even_when_opcode_is_not_rst() {
        let mut rom = vec![0x22; NEXT_ROM_SIZE];
        rom[3 * CONFIG_BANK_SIZE] = 0xf3; // The official Next ROM starts with DI.
        let mut bus = NextBus::new(&rom).expect("valid ROM");
        bus.config_reserved_sram[..PAGE_SIZE].fill(0x33);
        bus.write_nextreg(0x8e, 0x0b); // ROM 3 selected.
        assert_eq!(bus.read_opcode(0x0000), 0xf3);
        assert_eq!(bus.read(0x0001), 0x22, "automap is disabled at reset");
        bus.write_nextreg(0x0a, 0x10); // Enable DivMMC automapping.
        assert_eq!(bus.read_opcode(0x0000), 0xf3);
        assert_eq!(bus.read(0x0001), 0x33, "delayed mapping follows M1");
    }

    #[test]
    fn divmmc_control_maps_rom_and_selected_ram_bank() {
        let mut rom = vec![0x33; NEXT_ROM_SIZE];
        rom[0x2000] = 0x44;
        let mut bus = NextBus::new(&rom).expect("valid ROM");
        bus.install_boot_rom(&vec![0; 8192]).expect("valid IPL");
        bus.write_nextreg(0x03, 0); // Leave IPL; enter configuration mapping.
        bus.write_nextreg(0x04, 4); // DivMMC ROM is raw SRAM bank four.
        bus.write(0, 0xaa);
        bus.write_nextreg(0x03, 3);

        bus.out_port(0x00e3, 0x85); // CONMEM, RAM bank five.
        assert_eq!(bus.in_port(0x00e3), 0x85);
        assert_eq!(bus.read(0), 0xaa);
        assert_eq!(bus.read(0x2000), 0);
        bus.write(0x2000, 0x55);
        assert_eq!(bus.read(0x2000), 0x55);

        bus.out_port(0x00e3, 0x82); // Change the selected RAM bank.
        assert_eq!(bus.read(0), 0xaa);
        assert_eq!(bus.read(0x2000), 0);
        bus.out_port(0x00e3, 0);
        assert_eq!(bus.read(0), 0x33);
        assert_eq!(bus.read(0x2000), 0x44);
    }

    #[test]
    fn divmmc_ram_aliases_config_sram_banks_eight_through_fifteen() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        bus.install_boot_rom(&vec![0; 8192]).expect("valid IPL");
        bus.write_nextreg(0x03, 0);
        bus.write_nextreg(0x04, 8); // Physical $020000: DivMMC RAM bank zero.
        bus.write(0, 0x61);
        bus.write_nextreg(0x04, 4); // DivMMC ROM mapping leaves config mode.
        bus.write_nextreg(0x03, 3);
        bus.out_port(0x00e3, 0x80);
        assert_eq!(bus.read(0x2000), 0x61);
        bus.write(0x2001, 0x62);
        bus.hard_reset();
        bus.write_nextreg(0x03, 0);
        bus.write_nextreg(0x04, 8);
        assert_eq!(bus.read(1), 0x62);
    }

    #[test]
    fn divmmc_conmem_readback_is_independent_of_automapping() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        bus.divmmc_automapped = true;
        assert_eq!(bus.in_port(0x00e3) & 0x80, 0);
        bus.out_port(0x00e3, 0x81);
        assert_eq!(bus.in_port(0x00e3) & 0x80, 0x80);
        bus.out_port(0x00e3, 0x01);
        assert_eq!(bus.in_port(0x00e3) & 0x80, 0);
        assert!(bus.divmmc_mapped());
    }

    #[test]
    fn legacy_paging_ports_sync_with_nextreg_and_preserve_full_dffd_bank() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        bus.out_port(0x5ffd, 0x05); // Decoded alias of $7ffd.
        assert_eq!(bus.read_nextreg(0x8e), 0x58);
        bus.out_port(0xd001, 0x06); // Decoded alias of $dffd.
        assert_eq!(bus.read_nextreg(0x8e), 0x58);
        assert_eq!(bus.read_nextreg(0x56), 106);
        assert_eq!(bus.read_nextreg(0x57), 107);
        bus.out_port(0xdffd, 0x0e);
        assert_eq!(bus.read_nextreg(0x56), 234);
        assert_eq!(bus.read_nextreg(0x57), 235);
        assert_eq!(bus.read(0xc000), 0xff); // Absent bank stays absent, not clamped.
    }

    #[test]
    fn rom_and_all_ram_low_selectors_survive_mode_changes_independently() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        bus.out_port(0x7ffd, 0x10);
        bus.out_port(0x1ffd, 0x00);
        assert_eq!(bus.read_nextreg(0x8e) & 0x03, 0x01);

        for bank in 4..8u8 {
            assert!(bus.load_ram_page(bank * 2, 0, &[bank]));
        }
        bus.out_port(0x1ffd, 0x03);
        assert_eq!(bus.read(0), 4);
        bus.out_port(0x7ffd, 0x00);
        assert_eq!(bus.read(0), 4);
        bus.out_port(0x1ffd, 0x00);
        assert_eq!(bus.read_nextreg(0x8e) & 0x03, 0x01);
    }

    #[test]
    fn paging_lock_is_reported_and_unlocked_through_peripheral_three() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        bus.out_port(0x7ffd, 0x25);
        assert_eq!(bus.read_nextreg(0x08) & 0x80, 0);
        let locked_bank = bus.read_nextreg(0x8e) & 0xf0;
        bus.out_port(0x7ffd, 0x03);
        bus.out_port(0xdffd, 0x06);
        assert_eq!(bus.read_nextreg(0x8e) & 0xf0, locked_bank);
        let locked_mapping = bus.read_nextreg(0x8e);
        bus.out_port(0x1ffd, 0x01);
        assert_eq!(bus.read_nextreg(0x8e), locked_mapping);
        bus.write_nextreg(0x08, 0x80);
        assert_ne!(bus.read_nextreg(0x08) & 0x80, 0);
        bus.out_port(0x7ffd, 0x03);
        bus.out_port(0xdffd, 0x06);
        assert_eq!(bus.read_nextreg(0x56), 102);
    }

    #[test]
    fn all_ram_overlay_keeps_mmu_registers_and_restores_middle_slots() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        bus.write_nextreg(0x52, 30);
        bus.write_nextreg(0x53, 31);
        bus.write_nextreg(0x54, 32);
        bus.write_nextreg(0x55, 33);
        bus.out_port(0x1ffd, 0x01); // Special layout 0.
        assert_eq!(bus.read_nextreg(0x52), 30);
        assert_eq!(bus.read_nextreg(0x54), 32);
        assert_eq!(bus.read_nextreg(0x50), 0xff);
        bus.out_port(0x1ffd, 0x00);
        assert_eq!(bus.read_nextreg(0x52), 10);
        assert_eq!(bus.read_nextreg(0x53), 11);
        assert_eq!(bus.read_nextreg(0x54), 4);
        assert_eq!(bus.read_nextreg(0x55), 5);
    }

    #[test]
    fn divmmc_mapram_is_sticky_until_nextreg_nine_clears_it() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        bus.out_port(0x00e3, 0xc5); // CONMEM, MAPRAM and bank five.
        assert_eq!(bus.read(0), 0);
        bus.write(0x0100, 0x6a);
        assert_eq!(bus.read(0x0100), 0);

        bus.out_port(0x00e3, 0x80); // MAPRAM cannot be cleared through $E3.
        assert_eq!(bus.in_port(0x00e3), 0xc0);
        bus.config_reserved_sram[0x10000 + 3 * PAGE_SIZE] = 0x71;
        bus.out_port(0x00e3, 0xc3);
        bus.write(0x2000, 0x72);
        assert_eq!(bus.read(0x2000), 0x71);
        bus.out_port(0x00e3, 0xc0);
        bus.write(0, 0x73);
        assert_eq!(bus.read(0), 0x71);
        bus.write_nextreg(0x09, 0x08);
        assert_eq!(bus.read_nextreg(0x09), 0x08);
        assert_eq!(bus.in_port(0x00e3), 0x80);
        bus.out_port(0x00e3, 0x80);
        assert_eq!(bus.read(0x0100), 0);
    }

    #[test]
    fn divmmc_mapram_clears_on_bus_reset() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        bus.out_port(0x00e3, 0x40);
        bus.hard_reset();
        assert_eq!(bus.in_port(0x00e3), 0);
        bus.out_port(0x00e3, 0x40);
        bus.soft_reset();
        assert_eq!(bus.in_port(0x00e3), 0);
    }

    #[test]
    fn disabling_divmmc_automap_clears_current_mapping() {
        let rom = vec![0x22; NEXT_ROM_SIZE];
        let mut bus = NextBus::new(&rom).expect("valid ROM");
        bus.config_reserved_sram[..PAGE_SIZE].fill(0x33);
        bus.write_nextreg(0x0a, 0x10);
        assert_eq!(bus.read_opcode(0), 0x22);
        assert_eq!(bus.read(1), 0x33);
        bus.write_nextreg(0x0a, 0);
        assert_eq!(bus.read(1), 0x22, "config-mode disable releases automap");

        bus.write_nextreg(0x03, 3);
        bus.write_nextreg(0x0a, 0x10);
        assert_eq!(bus.read_opcode(0), 0x22);
        assert_eq!(bus.read(1), 0x33);
        bus.write_nextreg(0x0a, 0);
        assert_eq!(bus.read(1), 0x22, "machine-mode disable releases automap");
    }

    #[test]
    fn boot_register_profile_and_peripheral_defaults_match_the_pinned_core() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        assert_eq!(bus.read_nextreg(0x00), NEXT_MACHINE_ID);
        assert_eq!(bus.read_nextreg(0x01), NEXT_CORE_VERSION);
        assert_eq!(bus.read_nextreg(0x0e), NEXT_CORE_SUBMINOR);
        assert_eq!(bus.read_nextreg(0x0f), NEXT_BOARD_ID);
        assert_eq!(bus.read_nextreg(0x0a), 0x01);
        bus.write_nextreg(0x0a, 0xff);
        assert_eq!(bus.read_nextreg(0x0a), 0xfb);
        bus.write_nextreg(0x03, 0x03);
        bus.write_nextreg(0x0a, 0);
        assert_eq!(bus.read_nextreg(0x0a), 0xe0);
    }

    #[test]
    fn next_sd_reads_bounded_sectors_from_file() {
        let path =
            std::env::temp_dir().join(format!("spec-chum-next-sd-{}.img", std::process::id()));
        std::fs::write(&path, [0x5a; SECTOR_SIZE]).expect("create SD fixture");
        let mut sd = SdSpi::open(&path).expect("open sector-aligned image");
        let mut read = [0; SECTOR_SIZE];
        sd.read_sector(0, &mut read).expect("read sector 0");
        assert_eq!(read, [0x5a; SECTOR_SIZE]);
        assert!(sd.read_sector(1, &mut read).is_err());
        std::fs::remove_file(path).expect("remove SD fixture");
    }

    #[test]
    fn next_sd_spi_matches_ipl_initialization_and_reads_a_block() {
        let path =
            std::env::temp_dir().join(format!("spec-chum-next-spi-{}.img", std::process::id()));
        let mut first_block = vec![0x5a; SECTOR_SIZE];
        first_block[73] = 0x4c;
        let mut image = first_block.clone();
        image.extend([0xa5; SECTOR_SIZE]);
        std::fs::write(&path, image).expect("create SD fixture");
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        bus.attach_sd_file(&path).expect("attach SD image");
        assert_eq!(bus.in_port(0x20e7), 0xff);
        bus.out_port(0x30e7, 0xfc); // Both cards selected: no SPI device responds.
        assert_eq!(bus.in_port(0x10e7), 0xfc);
        bus.out_port(0x12eb, 0x40);
        assert_eq!(bus.in_port(0x34eb), 0xff);
        bus.out_port(0x10e7, 0xfd);
        assert_eq!(bus.in_port(0x10e7), 0xfd);
        let command = |bus: &mut NextBus, index: u8, argument: u32| {
            for byte in [
                0x40 | index,
                (argument >> 24) as u8,
                (argument >> 16) as u8,
                (argument >> 8) as u8,
                argument as u8,
                0x95,
            ] {
                bus.out_port(0x12eb, byte);
            }
            for _ in 0..4 {
                let response = bus.in_port(0x34eb);
                if response != 0xff {
                    return response;
                }
            }
            0xff
        };
        assert_eq!(command(&mut bus, 0, 0), 1);
        assert_eq!(command(&mut bus, 8, 0x1aa), 1);
        assert_eq!(
            (0..4)
                .map(|_| bus.in_port(PORT_NEXT_SD_DATA))
                .collect::<Vec<_>>(),
            [0, 0, 1, 0xaa]
        );
        assert_eq!(command(&mut bus, 55, 0), 1);
        assert_eq!(command(&mut bus, 41, 0x4000_0000), 0);
        assert_eq!(command(&mut bus, 58, 0), 0);
        assert_eq!(
            (0..4)
                .map(|_| bus.in_port(PORT_NEXT_SD_DATA))
                .collect::<Vec<_>>(),
            [0xc0, 0xff, 0x80, 0]
        );
        assert_eq!(command(&mut bus, 9, 0), 0);
        assert_eq!(bus.in_port(PORT_NEXT_SD_DATA), 0xfe);
        let csd = (0..18)
            .map(|_| bus.in_port(PORT_NEXT_SD_DATA))
            .collect::<Vec<_>>();
        assert_eq!(csd[0], 0x40, "CMD9 must return an SDHC CSD v2 register");
        assert_eq!(csd[5] & 0x0f, 9, "CSD must describe 512-byte sectors");
        assert_eq!(&csd[7..10], &[0, 0, 0], "2 sectors round to C_SIZE 0");
        assert_eq!(csd[15] & 1, 1, "CSD end bit must be set");
        assert_eq!(command(&mut bus, 17, 0), 0);
        let mut token = 0xff;
        for _ in 0..4 {
            token = bus.in_port(PORT_NEXT_SD_DATA);
            if token != 0xff {
                break;
            }
        }
        assert_eq!(token, 0xfe);
        for byte in first_block {
            assert_eq!(bus.in_port(PORT_NEXT_SD_DATA), byte);
        }
        assert_eq!(bus.in_port(PORT_NEXT_SD_DATA), 0xff);
        assert_eq!(bus.in_port(PORT_NEXT_SD_DATA), 0xff);

        // MMC_Read consumes exactly 512 data bytes and two CRC bytes, then
        // issues another CMD17 without deselecting the card.
        assert_eq!(command(&mut bus, 17, 1), 0);
        let mut token = 0xff;
        for _ in 0..4 {
            token = bus.in_port(PORT_NEXT_SD_DATA);
            if token != 0xff {
                break;
            }
        }
        assert_eq!(token, 0xfe);
        for _ in 0..SECTOR_SIZE {
            assert_eq!(bus.in_port(PORT_NEXT_SD_DATA), 0xa5);
        }
        assert_eq!(bus.in_port(PORT_NEXT_SD_DATA), 0xff);
        assert_eq!(bus.in_port(PORT_NEXT_SD_DATA), 0xff);
        std::fs::remove_file(path).expect("remove SD fixture");
    }

    #[test]
    fn next_sd_spi_streams_multiple_blocks_until_stop_transmission() {
        let path = std::env::temp_dir().join(format!(
            "spec-chum-next-multiblock-{}.img",
            std::process::id()
        ));
        let mut image = vec![0x5a; SECTOR_SIZE];
        image.extend([0xa5; SECTOR_SIZE]);
        std::fs::write(&path, image).expect("create SD fixture");
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        bus.attach_sd_file(&path).expect("attach SD image");
        bus.out_port(PORT_NEXT_SD_CS, 0xfd);

        let command = |bus: &mut NextBus, index: u8, argument: u32| {
            for byte in [
                0x40 | index,
                (argument >> 24) as u8,
                (argument >> 16) as u8,
                (argument >> 8) as u8,
                argument as u8,
                0x95,
            ] {
                bus.out_port(PORT_NEXT_SD_DATA, byte);
            }
            for _ in 0..4 {
                let response = bus.in_port(PORT_NEXT_SD_DATA);
                if response != 0xff {
                    return response;
                }
            }
            0xff
        };

        assert_eq!(command(&mut bus, 0, 0), 1);
        assert_eq!(command(&mut bus, 55, 0), 1);
        assert_eq!(command(&mut bus, 41, 0x4000_0000), 0);
        assert_eq!(command(&mut bus, 18, 0), 0);
        assert_eq!(bus.in_port(PORT_NEXT_SD_DATA), 0xff);
        assert_eq!(bus.in_port(PORT_NEXT_SD_DATA), 0xfe);
        for _ in 0..SECTOR_SIZE {
            assert_eq!(bus.in_port(PORT_NEXT_SD_DATA), 0x5a);
        }
        assert_eq!(bus.in_port(PORT_NEXT_SD_DATA), 0xff);
        assert_eq!(bus.in_port(PORT_NEXT_SD_DATA), 0xff);
        for byte in [0x4c, 0, 0, 0, 0, 0x01] {
            bus.out_port(PORT_NEXT_SD_DATA, byte);
        }
        assert_eq!(bus.in_port(PORT_NEXT_SD_DATA), 0xff);
        for _ in 1..8 {
            assert_eq!(bus.in_port(PORT_NEXT_SD_DATA), 0xff);
        }
        assert_eq!(bus.in_port(PORT_NEXT_SD_DATA), 0);
        assert_eq!(command(&mut bus, 17, 1), 0);
        let _ = bus.in_port(PORT_NEXT_SD_DATA);
        let mut token = 0xff;
        for _ in 0..4 {
            token = bus.in_port(PORT_NEXT_SD_DATA);
            if token != 0xff {
                break;
            }
        }
        assert_eq!(token, 0xfe);
        for _ in 0..SECTOR_SIZE {
            assert_eq!(bus.in_port(PORT_NEXT_SD_DATA), 0xa5);
        }

        std::fs::remove_file(path).expect("remove SD fixture");
    }

    #[test]
    fn next_sd_spi_persists_single_and_multiple_block_writes() {
        let path =
            std::env::temp_dir().join(format!("spec-chum-next-write-{}.img", std::process::id()));
        std::fs::write(&path, [0; SECTOR_SIZE * 3]).expect("create SD fixture");
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        bus.attach_sd_file(&path).expect("attach SD image");
        bus.out_port(PORT_NEXT_SD_CS, 0xfd);

        let command = |bus: &mut NextBus, index: u8, argument: u32| {
            for byte in [
                0x40 | index,
                (argument >> 24) as u8,
                (argument >> 16) as u8,
                (argument >> 8) as u8,
                argument as u8,
                0x95,
            ] {
                bus.out_port(PORT_NEXT_SD_DATA, byte);
            }
            for _ in 0..4 {
                let response = bus.in_port(PORT_NEXT_SD_DATA);
                if response != 0xff {
                    return response;
                }
            }
            0xff
        };
        assert_eq!(command(&mut bus, 0, 0), 1);
        assert_eq!(command(&mut bus, 55, 0), 1);
        assert_eq!(command(&mut bus, 41, 0x4000_0000), 0);

        let write_block = |bus: &mut NextBus, token: u8, bytes: &[u8; SECTOR_SIZE]| {
            bus.out_port(PORT_NEXT_SD_DATA, token);
            for byte in bytes {
                bus.out_port(PORT_NEXT_SD_DATA, *byte);
            }
            bus.out_port(PORT_NEXT_SD_DATA, 0xff);
            bus.out_port(PORT_NEXT_SD_DATA, 0xff);
            assert_eq!(bus.in_port(PORT_NEXT_SD_DATA), 0x05);
            assert_eq!(bus.in_port(PORT_NEXT_SD_DATA), 0x00);
            assert_eq!(bus.in_port(PORT_NEXT_SD_DATA), 0xff);
        };
        assert_eq!(command(&mut bus, 24, 2), 0);
        write_block(&mut bus, 0xfe, &[0x5a; SECTOR_SIZE]);
        assert_eq!(command(&mut bus, 13, 0), 0);
        assert_eq!(bus.in_port(PORT_NEXT_SD_DATA), 0);

        assert_eq!(command(&mut bus, 25, 0), 0);
        write_block(&mut bus, 0xfc, &[0xa5; SECTOR_SIZE]);
        write_block(&mut bus, 0xfc, &[0x3c; SECTOR_SIZE]);
        bus.out_port(PORT_NEXT_SD_DATA, 0xfd);
        assert_eq!(bus.in_port(PORT_NEXT_SD_DATA), 0x00);

        let image = std::fs::read(&path).expect("read back SD image");
        assert_eq!(&image[0..SECTOR_SIZE], &[0xa5; SECTOR_SIZE]);
        assert_eq!(&image[SECTOR_SIZE..SECTOR_SIZE * 2], &[0x3c; SECTOR_SIZE]);
        assert_eq!(&image[SECTOR_SIZE * 2..], &[0x5a; SECTOR_SIZE]);
        std::fs::remove_file(path).expect("remove SD fixture");
    }

    #[test]
    fn next_sd_card_responds_to_either_available_sd_select_line() {
        let path =
            std::env::temp_dir().join(format!("spec-chum-next-sd-swap-{}.img", std::process::id()));
        std::fs::write(&path, [0x5a; SECTOR_SIZE]).expect("create SD fixture");
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        bus.attach_sd_file(&path).expect("attach SD image");

        let command = |bus: &mut NextBus| {
            for byte in [0x40, 0, 0, 0, 0, 0x95] {
                bus.out_port(PORT_NEXT_SD_DATA, byte);
            }
            for _ in 0..4 {
                let response = bus.in_port(PORT_NEXT_SD_DATA);
                if response != 0xff {
                    return response;
                }
            }
            0xff
        };

        bus.out_port(PORT_NEXT_SD_CS, 0xfe);
        assert_eq!(command(&mut bus), 1);
        bus.out_port(PORT_NEXT_SD_CS, 0xff);
        bus.out_port(PORT_NEXT_SD_CS, 0xfd);
        assert_eq!(command(&mut bus), 1);
        bus.out_port(PORT_NEXT_SD_CS, 0xfc);
        assert_eq!(command(&mut bus), 0xff);

        std::fs::remove_file(path).expect("remove SD fixture");
    }

    #[test]
    fn zxn_dma_uses_partial_decode_on_6b_without_aliasing_0b() {
        let mut bus = NextBus::new(&vec![0; NEXT_ROM_SIZE]).expect("valid ROM");
        bus.out_port_at(0x126b, 0xc3, 0);
        assert_eq!(bus.in_port_at(0xab6b, 0), 0x3a);
        assert_eq!(bus.in_port_at(0xab0b, 0), 0xff);
    }
}
