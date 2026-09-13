//! DivMMC-style control port + SRAM paging + SPI SD card.
//!
//! Port `0xE3` (control): bit7=CONMEM, bit6=MAPRAM (sticky), bits0–5=RAM page.
//! Ports `0xE7` / `0xEB` are SPI CS (active-low bit0) / data.
//!
//! Memory overlay is active when CONMEM is set **or** DivIDE-compatible automap
//! is latched. Delayed entry/exit points (`$0000`, `$0008`, …, `$1FF8–$1FFF`)
//! apply after the opcode M1 byte (via [`Self::after_m1_refresh`]); the
//! `$3D00–$3DFF` trap maps instantly. ESXDOS EEPROM is optional — attach an
//! 8 KiB image for ROM overlay; without it, only MAPRAM / CONMEM RAM paging and
//! SPI SD I/O remain useful.
//!
//! SPI speaks a minimal MMC/SD/SDHC subset (CMD0/8/16/17/24/55/58 + ACMD41) over
//! a flat sector image. ACMD41 HCS (bit 30) selects SDHC block addressing + OCR
//! CCS; without HCS the card stays SDSC (byte-addressed CMD17/24) — matching
//! `DivMMC` Ready / ESXDOS real-card init. Full ESXDOS boot needs a user-supplied
//! EEPROM + FAT SD image (see `docs/ROMS.md`).

use crate::RomLoadError;

/// `DivMMC` control / paging register.
pub const PORT_CONTROL: u16 = 0x00e3;
/// SPI chip-select (active low bit0).
pub const PORT_SPI_CS: u16 = 0x00e7;
/// SPI data (byte exchange against the SD card state machine).
pub const PORT_SPI_DATA: u16 = 0x00eb;

const PAGE_SIZE: usize = 8192;
/// `DivMMC` Ready-compatible SRAM: 64 × 8 KiB pages (512 KiB; control bits 0–5).
pub const RAM_PAGES: usize = 64;
/// SD / MMC block size used by CMD17 / CMD24.
pub const SD_SECTOR_SIZE: usize = 512;
/// ACMD41 Host Capacity Support — request SDHC / block addressing.
const ACMD41_HCS: u32 = 0x4000_0000;

/// DivIDE-compatible **delayed** automap entry points (apply after opcode M1).
const AUTOMAP_ENTRIES_DELAYED: &[u16] = &[0x0000, 0x0008, 0x0038, 0x0066, 0x04c6, 0x0562];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SpiPhase {
    /// Waiting for a command start byte (`0x40 | cmd`).
    Idle,
    /// Collecting 4 argument bytes + CRC.
    Arg { cmd: u8, got: u8, arg: u32 },
    /// N `0xFF` delay bytes before R1.
    WaitR1 { cmd: u8, arg: u32, left: u8 },
    /// Extra bytes after R1 (CMD8 R7 / CMD58 R3).
    SendExtra { bytes: [u8; 4], idx: u8 },
    /// Delay before data token on read.
    WaitToken { lba: u32, left: u8 },
    /// 512 data bytes + 2 CRC after `0xFE`.
    ReadData { lba: u32, idx: u16 },
    /// Expect host data token `0xFE` for CMD24.
    WaitWriteToken { lba: u32 },
    /// Receive 512 data bytes (+ ignore 2 CRC).
    WriteData { lba: u32, idx: u16 },
    /// Data-response token then busy `0x00` then `0xFF`.
    WriteResp { idx: u8 },
}

#[derive(Clone, Debug)]
struct SdSpi {
    phase: SpiPhase,
    /// Card reports idle until ACMD41 succeeds.
    idle: bool,
    /// Next command is an application command (after CMD55).
    app_next: bool,
    /// SDHC (block LBA) when true; SDSC (byte address) when false.
    /// Latched from ACMD41 HCS; cleared by CMD0 / soft reset.
    high_capacity: bool,
}

impl Default for SdSpi {
    fn default() -> Self {
        Self {
            phase: SpiPhase::Idle,
            idle: true,
            app_next: false,
            high_capacity: false,
        }
    }
}

impl SdSpi {
    fn reset_transaction(&mut self) {
        self.phase = SpiPhase::Idle;
    }

    fn soft_reset_card(&mut self) {
        *self = Self::default();
    }

    fn exchange(&mut self, mosi: u8, sd: &mut [u8]) -> u8 {
        // One IN or OUT on the data port = one SPI byte (DivMMC CPLD behaviour).
        self.clock(mosi, sd)
    }

    fn clock(&mut self, mosi: u8, sd: &mut [u8]) -> u8 {
        match self.phase {
            SpiPhase::Idle => {
                if mosi & 0xc0 == 0x40 {
                    let cmd = mosi & 0x3f;
                    self.phase = SpiPhase::Arg {
                        cmd,
                        got: 0,
                        arg: 0,
                    };
                }
                0xff
            }
            SpiPhase::Arg { cmd, got, arg } => {
                if got < 4 {
                    let arg = (arg << 8) | u32::from(mosi);
                    self.phase = SpiPhase::Arg {
                        cmd,
                        got: got + 1,
                        arg,
                    };
                    0xff
                } else {
                    let _ = mosi;
                    self.phase = SpiPhase::WaitR1 { cmd, arg, left: 1 };
                    0xff
                }
            }
            SpiPhase::WaitR1 { cmd, arg, left } => {
                if left > 0 {
                    self.phase = SpiPhase::WaitR1 {
                        cmd,
                        arg,
                        left: left - 1,
                    };
                    0xff
                } else {
                    let (r1, next) = self.begin_response(cmd, arg);
                    self.phase = next;
                    r1
                }
            }
            SpiPhase::SendExtra { bytes, idx } => {
                let v = bytes[idx as usize];
                if idx + 1 >= 4 {
                    self.phase = SpiPhase::Idle;
                } else {
                    self.phase = SpiPhase::SendExtra {
                        bytes,
                        idx: idx + 1,
                    };
                }
                v
            }
            SpiPhase::WaitToken { lba, left } => {
                if left > 0 {
                    self.phase = SpiPhase::WaitToken {
                        lba,
                        left: left - 1,
                    };
                    0xff
                } else {
                    self.phase = SpiPhase::ReadData { lba, idx: 0 };
                    0xfe
                }
            }
            SpiPhase::ReadData { lba, idx } => {
                if idx < 512 {
                    let v = Self::sector_byte(sd, lba, idx as usize);
                    self.phase = SpiPhase::ReadData { lba, idx: idx + 1 };
                    v
                } else if idx < 514 {
                    self.phase = SpiPhase::ReadData { lba, idx: idx + 1 };
                    0xff
                } else {
                    self.phase = SpiPhase::Idle;
                    0xff
                }
            }
            SpiPhase::WaitWriteToken { lba } => {
                if mosi == 0xfe {
                    self.phase = SpiPhase::WriteData { lba, idx: 0 };
                }
                0xff
            }
            SpiPhase::WriteData { lba, idx } => {
                if idx < 512 {
                    Self::write_sector_byte(sd, lba, idx as usize, mosi);
                    self.phase = SpiPhase::WriteData { lba, idx: idx + 1 };
                    0xff
                } else if idx < 514 {
                    self.phase = SpiPhase::WriteData { lba, idx: idx + 1 };
                    0xff
                } else {
                    // CRC complete — data response on this MISO byte.
                    self.phase = SpiPhase::WriteResp { idx: 0 };
                    0x05
                }
            }
            SpiPhase::WriteResp { idx } => {
                if idx == 0 {
                    self.phase = SpiPhase::WriteResp { idx: 1 };
                    0x00 // busy
                } else {
                    self.phase = SpiPhase::Idle;
                    0xff
                }
            }
        }
    }

    fn begin_response(&mut self, cmd: u8, arg: u32) -> (u8, SpiPhase) {
        let is_acmd = self.app_next;
        self.app_next = false;

        if is_acmd && cmd == 41 {
            self.idle = false;
            // DivMMC Ready / ESXDOS: HCS ⇒ SDHC + CCS; clear HCS ⇒ SDSC byte addr.
            self.high_capacity = arg & ACMD41_HCS != 0;
            return (0x00, SpiPhase::Idle);
        }

        match cmd {
            0 => {
                self.soft_reset_card();
                self.idle = true;
                (0x01, SpiPhase::Idle)
            }
            8 => {
                let r1 = if self.idle { 0x01 } else { 0x00 };
                (
                    r1,
                    SpiPhase::SendExtra {
                        bytes: [0x00, 0x00, ((arg >> 8) as u8) & 0x0f, arg as u8],
                        idx: 0,
                    },
                )
            }
            16 => (if self.idle { 0x01 } else { 0x00 }, SpiPhase::Idle),
            17 => {
                if self.idle {
                    (0x01, SpiPhase::Idle)
                } else {
                    (
                        0x00,
                        SpiPhase::WaitToken {
                            lba: self.arg_to_lba(arg),
                            left: 1,
                        },
                    )
                }
            }
            24 => {
                if self.idle {
                    (0x01, SpiPhase::Idle)
                } else {
                    (
                        0x00,
                        SpiPhase::WaitWriteToken {
                            lba: self.arg_to_lba(arg),
                        },
                    )
                }
            }
            55 => {
                self.app_next = true;
                (if self.idle { 0x01 } else { 0x00 }, SpiPhase::Idle)
            }
            58 => {
                let r1 = if self.idle { 0x01 } else { 0x00 };
                // OCR[31]=power-up done; OCR[30]=CCS (SDHC) when high_capacity.
                let ocr0 = if self.idle {
                    0x00
                } else if self.high_capacity {
                    0xc0
                } else {
                    0x80
                };
                (
                    r1,
                    SpiPhase::SendExtra {
                        bytes: [ocr0, 0xff, 0x80, 0x00],
                        idx: 0,
                    },
                )
            }
            _ => (0x04 | if self.idle { 0x01 } else { 0x00 }, SpiPhase::Idle),
        }
    }

    fn arg_to_lba(&self, arg: u32) -> u32 {
        if self.high_capacity {
            arg
        } else {
            arg / SD_SECTOR_SIZE as u32
        }
    }

    fn sector_byte(sd: &[u8], lba: u32, offset: usize) -> u8 {
        let idx = (lba as usize)
            .checked_mul(SD_SECTOR_SIZE)
            .and_then(|base| base.checked_add(offset));
        idx.and_then(|i| sd.get(i).copied()).unwrap_or(0xff)
    }

    fn write_sector_byte(sd: &mut [u8], lba: u32, offset: usize, value: u8) {
        let Some(idx) = (lba as usize)
            .checked_mul(SD_SECTOR_SIZE)
            .and_then(|base| base.checked_add(offset))
        else {
            return;
        };
        if idx < sd.len() {
            sd[idx] = value;
        }
    }
}

#[derive(Clone, Debug)]
pub struct DivMmc {
    pub control: u8,
    pub spi_cs: u8,
    /// DivIDE-compatible automap latch (effective after delayed M1 refresh).
    pub automap: bool,
    /// Pending automap value applied by [`Self::after_m1_refresh`].
    automap_next: bool,
    /// Optional 8 KiB EEPROM (ESXDOS); zeroed until attached.
    pub eeprom: [u8; PAGE_SIZE],
    pub eeprom_loaded: bool,
    /// Contiguous SRAM (`RAM_PAGES * 8K`).
    pub ram: Vec<u8>,
    /// Flat SD/MMC image (byte addressable as LBA × 512).
    pub sd: Vec<u8>,
    spi: SdSpi,
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
            automap: false,
            automap_next: false,
            eeprom: [0; PAGE_SIZE],
            eeprom_loaded: false,
            ram: vec![0u8; RAM_PAGES * PAGE_SIZE],
            sd: Vec::new(),
            spi: SdSpi::default(),
        }
    }

    pub fn attach_sd(&mut self, data: Vec<u8>) {
        self.sd = data;
        self.spi.soft_reset_card();
    }

    /// Attach ESXDOS / `DivMMC` EEPROM. Accepts exactly 8 KiB, or a larger image
    /// (first 8 KiB used). Smaller images are rejected.
    pub fn attach_eeprom(&mut self, data: &[u8]) -> Result<(), RomLoadError> {
        if data.len() < PAGE_SIZE {
            return Err(RomLoadError::TooSmall {
                kind: "DivMMC EEPROM",
                min: PAGE_SIZE,
                got: data.len(),
            });
        }
        self.eeprom.copy_from_slice(&data[..PAGE_SIZE]);
        self.eeprom_loaded = true;
        Ok(())
    }

    /// Soft reset: clear CONMEM + automap; MAPRAM sticky bit is preserved.
    pub fn reset_soft(&mut self) {
        self.control &= 0x40;
        self.automap = false;
        self.automap_next = false;
        self.spi_cs = 0xff;
        self.spi.reset_transaction();
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

    /// Overlay active when CONMEM or automap is latched.
    #[must_use]
    pub fn mapped(&self) -> bool {
        self.conmem() || self.automap
    }

    /// M1 (opcode fetch) automap — `DivIDE` entry / exit points.
    ///
    /// Delayed entries/exits only update [`Self::automap_next`]; call
    /// [`Self::after_m1_refresh`] after the opcode byte so the first M1 byte
    /// still comes from Spectrum ROM (or from `DivMMC` on exit). Instant map for
    /// `$3D00–$3DFF` (TR-DOS trap) applies immediately.
    ///
    /// No-op until EEPROM is attached or MAPRAM is set (hardware needs a ROM
    /// image or MAPRAM before automatic paging engages).
    pub fn notify_m1(&mut self, pc: u16) {
        if !self.eeprom_loaded && !self.mapram() {
            return;
        }
        // Instant map (TR-DOS compatibility page) — before opcode fetch.
        if (0x3d00..=0x3dff).contains(&pc) {
            self.automap = true;
            self.automap_next = true;
            return;
        }
        if AUTOMAP_ENTRIES_DELAYED.contains(&pc) {
            self.automap_next = true;
        } else if (0x1ff8..=0x1fff).contains(&pc) {
            self.automap_next = false;
        }
    }

    /// Apply pending delayed automap after the opcode M1 byte (ULA refresh slot).
    pub fn after_m1_refresh(&mut self) {
        self.automap = self.automap_next;
    }

    #[must_use]
    fn cs_active(&self) -> bool {
        self.spi_cs & 1 == 0
    }

    pub fn out_port(&mut self, port: u16, value: u8) -> bool {
        match port & 0xff {
            0xe3 => {
                let mapram = (self.control & 0x40) | (value & 0x40);
                self.control = (value & !0x40) | mapram;
                true
            }
            PORT_SPI_CS => {
                let was_active = self.cs_active();
                self.spi_cs = value;
                if was_active && !self.cs_active() {
                    self.spi.reset_transaction();
                }
                true
            }
            PORT_SPI_DATA => {
                if self.cs_active() {
                    let _ = self.spi.exchange(value, &mut self.sd);
                }
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
                if self.cs_active() {
                    Some(self.spi.exchange(0xff, &mut self.sd))
                } else {
                    Some(0xff)
                }
            }
            _ => None,
        }
    }

    /// Memory overlay when mapped: `0000–1FFF` EEPROM (or RAM page 3 if MAPRAM),
    /// `2000–3FFF` selected RAM page.
    #[must_use]
    pub fn read_overlay(&self, addr: u16) -> Option<u8> {
        if !self.mapped() || addr >= 0x4000 {
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
        if !self.mapped() || addr >= 0x4000 {
            return false;
        }
        if addr < 0x2000 {
            if self.mapram() {
                // Bank 3 in the lower 8K is write-protected under MAPRAM.
                let _ = value;
                true
            } else {
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

    fn spi_select(d: &mut DivMmc) {
        d.out_port(PORT_SPI_CS, 0xfe);
    }

    fn spi_deselect(d: &mut DivMmc) {
        d.out_port(PORT_SPI_CS, 0xff);
    }

    fn spi_tx(d: &mut DivMmc, mosi: u8) {
        d.out_port(PORT_SPI_DATA, mosi);
    }

    fn spi_rx(d: &mut DivMmc) -> u8 {
        d.in_port(PORT_SPI_DATA).expect("spi data")
    }

    fn spi_cmd(d: &mut DivMmc, cmd: u8, arg: u32, crc: u8) -> u8 {
        spi_tx(d, 0x40 | cmd);
        spi_tx(d, (arg >> 24) as u8);
        spi_tx(d, (arg >> 16) as u8);
        spi_tx(d, (arg >> 8) as u8);
        spi_tx(d, arg as u8);
        spi_tx(d, crc);
        // NCR: one fill then R1
        let _ = spi_rx(d);
        spi_rx(d)
    }

    fn spi_init_ready(d: &mut DivMmc) {
        // ESXDOS / DivMMC Ready SDHC path: CMD0 → CMD8 → ACMD41(HCS).
        spi_select(d);
        assert_eq!(spi_cmd(d, 0, 0, 0x95), 0x01);
        spi_deselect(d);
        spi_select(d);
        assert_eq!(spi_cmd(d, 8, 0x1aa, 0x87), 0x01);
        for _ in 0..4 {
            let _ = spi_rx(d);
        }
        spi_deselect(d);
        spi_select(d);
        assert_eq!(spi_cmd(d, 55, 0, 0x65), 0x01);
        assert_eq!(spi_cmd(d, 41, ACMD41_HCS, 0x77), 0x00);
        spi_deselect(d);
    }

    fn spi_init_sdsc(d: &mut DivMmc) {
        spi_select(d);
        assert_eq!(spi_cmd(d, 0, 0, 0x95), 0x01);
        spi_deselect(d);
        spi_select(d);
        assert_eq!(spi_cmd(d, 8, 0x1aa, 0x87), 0x01);
        for _ in 0..4 {
            let _ = spi_rx(d);
        }
        spi_deselect(d);
        spi_select(d);
        assert_eq!(spi_cmd(d, 55, 0, 0x65), 0x01);
        // No HCS → SDSC byte addressing (legacy cards).
        assert_eq!(spi_cmd(d, 41, 0, 0x77), 0x00);
        spi_deselect(d);
    }

    fn spi_wait_token(d: &mut DivMmc) -> u8 {
        let mut token = 0xff;
        for _ in 0..8 {
            token = spi_rx(d);
            if token == 0xfe {
                break;
            }
        }
        token
    }

    #[test]
    fn control_port_conmem_shows_ram_page() {
        let mut d = DivMmc::new();
        d.ram[PAGE_SIZE] = 0x5a;
        d.out_port(PORT_CONTROL, 0x80 | 1);
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
        d.out_port(PORT_CONTROL, 0xc0);
        assert_eq!(d.read_overlay(0x0010), Some(0x77));
    }

    #[test]
    fn mapram_is_sticky_across_control_writes() {
        let mut d = DivMmc::new();
        d.out_port(PORT_CONTROL, 0x40);
        assert!(d.mapram());
        d.out_port(PORT_CONTROL, 0x80);
        assert!(d.mapram(), "MAPRAM must stick until power-cycle");
        assert!(d.conmem());
        d.reset_soft();
        assert!(d.mapram());
        assert!(!d.conmem());
        assert!(!d.automap);
    }

    #[test]
    fn automap_entry_and_exit_with_eeprom_is_delayed() {
        let mut d = DivMmc::new();
        d.attach_eeprom(&[0x55u8; PAGE_SIZE]).unwrap();
        d.notify_m1(0x0008);
        assert!(!d.automap, "delayed entry must not map before opcode M1");
        assert_eq!(d.read_overlay(0x0000), None);
        d.after_m1_refresh();
        assert!(d.automap);
        assert_eq!(d.read_overlay(0x0000), Some(0x55));
        d.notify_m1(0x1ff8);
        assert!(d.automap, "exit delay keeps map for the opcode byte");
        assert_eq!(d.read_overlay(0x0000), Some(0x55));
        d.after_m1_refresh();
        assert!(!d.automap);
        assert_eq!(d.read_overlay(0x0000), None);
    }

    #[test]
    fn automap_3dxx_is_instant() {
        let mut d = DivMmc::new();
        d.attach_eeprom(&[0x55u8; PAGE_SIZE]).unwrap();
        d.notify_m1(0x3d00);
        assert!(d.automap, "TR-DOS trap maps before opcode fetch");
        assert_eq!(d.read_overlay(0x0000), Some(0x55));
    }

    #[test]
    fn automap_ignored_without_eeprom_or_mapram() {
        let mut d = DivMmc::new();
        d.notify_m1(0x0000);
        d.after_m1_refresh();
        assert!(!d.automap);
    }

    #[test]
    fn sd_spi_cmd17_reads_sector() {
        let mut img = vec![0u8; SD_SECTOR_SIZE * 2];
        img[0] = 0x11;
        img[1] = 0x22;
        img[SD_SECTOR_SIZE] = 0xaa;
        let mut d = DivMmc::new();
        d.attach_sd(img);
        spi_init_ready(&mut d);

        spi_select(&mut d);
        assert_eq!(spi_cmd(&mut d, 17, 1, 0xff), 0x00);
        assert_eq!(spi_wait_token(&mut d), 0xfe);
        assert_eq!(spi_rx(&mut d), 0xaa);
        spi_deselect(&mut d);
    }

    #[test]
    fn sd_spi_cmd24_writes_sector() {
        let mut d = DivMmc::new();
        d.attach_sd(vec![0u8; SD_SECTOR_SIZE]);
        spi_init_ready(&mut d);

        spi_select(&mut d);
        assert_eq!(spi_cmd(&mut d, 24, 0, 0xff), 0x00);
        spi_tx(&mut d, 0xfe);
        spi_tx(&mut d, 0x5a);
        for _ in 1..512 {
            spi_tx(&mut d, 0x00);
        }
        spi_tx(&mut d, 0xff);
        spi_tx(&mut d, 0xff);
        let resp = spi_rx(&mut d);
        assert_eq!(resp & 0x1f, 0x05);
        spi_deselect(&mut d);
        assert_eq!(d.sd[0], 0x5a);
    }

    #[test]
    fn eeprom_accepts_larger_image_prefix() {
        let mut d = DivMmc::new();
        let big = vec![0xa5u8; PAGE_SIZE + 64];
        d.attach_eeprom(&big).unwrap();
        assert!(d.eeprom_loaded);
        assert_eq!(d.eeprom[0], 0xa5);
    }

    /// `DivMMC` Ready ships up to 512 KiB SRAM (64 × 8 KiB); page bits 0–5.
    #[test]
    fn ready_card_sram_pages_reach_512kib() {
        let mut d = DivMmc::new();
        assert_eq!(d.ram.len(), RAM_PAGES * PAGE_SIZE);
        assert_eq!(RAM_PAGES, 64);
        let last = RAM_PAGES - 1;
        d.ram[last * PAGE_SIZE] = 0xbe;
        d.out_port(PORT_CONTROL, 0x80 | (last as u8));
        assert_eq!(d.ram_page(), last);
        assert_eq!(d.read_overlay(0x2000), Some(0xbe));
    }

    /// ESXDOS SDHC init: ACMD41(HCS) ⇒ OCR CCS and block-addressed CMD17.
    #[test]
    fn ready_card_sdhc_cmd58_reports_ccs_and_block_lba() {
        let mut img = vec![0u8; SD_SECTOR_SIZE * 2];
        img[SD_SECTOR_SIZE] = 0xcd;
        let mut d = DivMmc::new();
        d.attach_sd(img);
        spi_init_ready(&mut d);

        spi_select(&mut d);
        assert_eq!(spi_cmd(&mut d, 58, 0, 0xff), 0x00);
        let ocr0 = spi_rx(&mut d);
        assert_eq!(ocr0 & 0xc0, 0xc0, "power-up + CCS for SDHC");
        for _ in 0..3 {
            let _ = spi_rx(&mut d);
        }
        spi_deselect(&mut d);

        spi_select(&mut d);
        assert_eq!(spi_cmd(&mut d, 17, 1, 0xff), 0x00);
        assert_eq!(spi_wait_token(&mut d), 0xfe);
        assert_eq!(spi_rx(&mut d), 0xcd);
        spi_deselect(&mut d);
    }

    /// Legacy SDSC path: ACMD41 without HCS ⇒ no CCS; CMD17 arg is byte address.
    #[test]
    fn ready_card_sdsc_uses_byte_addressing() {
        let mut img = vec![0u8; SD_SECTOR_SIZE * 2];
        img[SD_SECTOR_SIZE] = 0xab;
        let mut d = DivMmc::new();
        d.attach_sd(img);
        spi_init_sdsc(&mut d);

        spi_select(&mut d);
        assert_eq!(spi_cmd(&mut d, 58, 0, 0xff), 0x00);
        let ocr0 = spi_rx(&mut d);
        assert_eq!(ocr0 & 0xc0, 0x80, "power-up without CCS for SDSC");
        for _ in 0..3 {
            let _ = spi_rx(&mut d);
        }
        spi_deselect(&mut d);

        spi_select(&mut d);
        // Byte address of sector 1 (not LBA 1).
        assert_eq!(spi_cmd(&mut d, 17, SD_SECTOR_SIZE as u32, 0xff), 0x00);
        assert_eq!(spi_wait_token(&mut d), 0xfe);
        assert_eq!(spi_rx(&mut d), 0xab);
        spi_deselect(&mut d);
    }

    /// Real `DivMMC` CPLD: raising CS mid-command aborts the SPI transaction.
    #[test]
    fn ready_card_cs_deselect_aborts_spi_command() {
        let mut d = DivMmc::new();
        d.attach_sd(vec![0u8; SD_SECTOR_SIZE]);
        spi_init_ready(&mut d);

        spi_select(&mut d);
        spi_tx(&mut d, 0x40 | 0x11); // start CMD17
        spi_tx(&mut d, 0x00);
        spi_deselect(&mut d); // abort mid-arg

        spi_select(&mut d);
        assert_eq!(
            spi_cmd(&mut d, 58, 0, 0xff),
            0x00,
            "fresh command after CS abort must succeed"
        );
        let ocr0 = spi_rx(&mut d);
        assert_eq!(ocr0 & 0xc0, 0xc0);
        spi_deselect(&mut d);
    }
}
