//! Beta Disk Interface / TR-DOS: VG93 (KR1818VG93 / WD1793-class) FDC.
//!
//! Ports (low byte, when TR-DOS is paged): `0x1F` cmd/status, `0x3F` track,
//! `0x5F` sector, `0x7F` data, `0xFF` system.
//!
//! System port `0xFF` (Fuse / Beta 128): write selects drive (`D0–D1`), HLT
//! (`D3`), side (`D4` set = side 0), MFM (`D5`). Read: `D7` = INTRQ, `D6` = DRQ.
//!
//! Opcode-fetch (M1) paging: ROM in at `0x3C00–0x3DFF` (48K) or `0x3D00–0x3DFF`
//! (128K), out at `PC >= 0x4000`.

use formats::{TrdImage, TRD_SECTORS_PER_TRACK, TRD_SECTOR_SIZE};

/// TR-DOS / Beta 128 ROM size (16 KiB overlay at `0000–3FFF`).
pub const TRDOS_ROM_SIZE: usize = 16384;

const STAT_BUSY: u8 = 0x01;
const STAT_DRQ: u8 = 0x02;
const STAT_TRACK0: u8 = 0x04;
const STAT_RNF: u8 = 0x10;
const STAT_HEAD: u8 = 0x20;
const STAT_NOT_READY: u8 = 0x80;

/// Default system latch: drive 0, HLT, side 0, MFM (typical TR-DOS `OUT (#FF)`).
const SYS_DEFAULT: u8 = 0x3c;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Xfer {
    Idle,
    Read,
    Write,
    ReadAddress,
}

#[derive(Clone, Debug)]
pub struct BetaDisk {
    pub track: u8,
    pub sector: u8,
    pub status: u8,
    pub system: u8,
    /// When true, TR-DOS is paged and Beta claims ports `1Fh`/`3Fh`/`5Fh`/`7Fh`/`FFh`.
    pub paged: bool,
    /// Data register (seek target; first ID byte; Type II scratch).
    pub data_reg: u8,
    buffer: Vec<u8>,
    buffer_i: usize,
    pub image: Option<TrdImage>,
    rom: [u8; TRDOS_ROM_SIZE],
    pub rom_loaded: bool,
    drq: bool,
    intrq: bool,
    /// Last step direction: `true` = in (towards 79).
    step_in: bool,
    xfer: Xfer,
    multiple: bool,
    /// Last command was Type I (status bits TRACK0 / HEAD).
    type_i_status: bool,
}

impl Default for BetaDisk {
    fn default() -> Self {
        Self::new()
    }
}

impl BetaDisk {
    #[must_use]
    pub fn new() -> Self {
        Self {
            track: 0,
            sector: 1,
            status: STAT_TRACK0,
            system: SYS_DEFAULT,
            paged: false,
            data_reg: 0,
            buffer: Vec::new(),
            buffer_i: 0,
            image: None,
            rom: [0; TRDOS_ROM_SIZE],
            rom_loaded: false,
            drq: false,
            intrq: false,
            step_in: true,
            xfer: Xfer::Idle,
            multiple: false,
            type_i_status: true,
        }
    }

    pub fn insert(&mut self, image: TrdImage) {
        self.image = Some(image);
    }

    /// Load a 16 KiB TR-DOS ROM (paged over `0000–3FFF` on M1 into `3C00–3DFF`).
    pub fn load_rom(&mut self, data: &[u8]) -> Result<(), String> {
        if data.len() != TRDOS_ROM_SIZE {
            return Err(format!(
                "TR-DOS ROM must be {TRDOS_ROM_SIZE} bytes, got {}",
                data.len()
            ));
        }
        self.rom.copy_from_slice(data);
        self.rom_loaded = true;
        Ok(())
    }

    /// Page / hide TR-DOS (enables Beta port decode when true).
    pub fn page_trdos(&mut self, on: bool) {
        self.paged = on;
    }

    /// M1 (opcode fetch) paging used by TR-DOS `USR 15616` (`0x3D00`).
    ///
    /// `page_in_lo` is the start of the page-in window through `0x3DFF`:
    /// `0x3C00` for 48K Beta, `0x3D00` for Beta 128. Unpage only on fetches at
    /// `PC >= 0x4000` so TR-DOS can read/write RAM without dropping the ROM.
    /// No-op until a ROM is loaded so port-only tests can latch `paged` manually.
    pub fn notify_m1(&mut self, pc: u16, page_in_lo: u16) {
        if !self.rom_loaded {
            return;
        }
        if (page_in_lo..0x3e00).contains(&pc) {
            self.paged = true;
        } else if pc >= 0x4000 {
            self.paged = false;
        }
    }

    /// Overlay TR-DOS ROM at `0000–3FFF` when paged and a ROM image is loaded.
    #[must_use]
    pub fn read_rom(&self, addr: u16) -> Option<u8> {
        if !self.paged || !self.rom_loaded || addr >= 0x4000 {
            return None;
        }
        Some(self.rom[addr as usize])
    }

    /// `OUT` to Beta ports. Returns true if handled.
    pub fn out_port(&mut self, port: u16, value: u8) -> bool {
        if !self.paged {
            return false;
        }
        match port & 0xff {
            0x1f => {
                self.write_command(value);
                true
            }
            0x3f => {
                self.track = value;
                true
            }
            0x5f => {
                self.sector = value;
                true
            }
            0x7f => {
                self.write_data(value);
                true
            }
            0xff => {
                self.system = value;
                true
            }
            _ => false,
        }
    }

    /// `IN` from Beta ports.
    pub fn in_port(&mut self, port: u16) -> Option<u8> {
        if !self.paged {
            return None;
        }
        match port & 0xff {
            0x1f => {
                self.intrq = false;
                Some(self.status)
            }
            0x3f => Some(self.track),
            0x5f => Some(self.sector),
            0x7f => Some(self.read_data()),
            0xff => {
                let mut v = 0u8;
                if self.intrq {
                    v |= 0x80;
                }
                if self.drq {
                    v |= 0x40;
                }
                Some(v)
            }
            _ => None,
        }
    }

    fn drive(&self) -> u8 {
        self.system & 0x03
    }

    /// Fuse: bit 4 set → head 0 (bottom).
    fn side(&self) -> u8 {
        u8::from(self.system & 0x10 == 0)
    }

    fn disk_ready(&self) -> bool {
        self.image.is_some() && self.drive() == 0
    }

    /// 0-based index for the latched sector ID (`1..=16`). Returns
    /// `TRD_SECTORS_PER_TRACK` for an invalid ID so callers report RNF.
    fn sector_index(&self) -> usize {
        if self.sector == 0 {
            TRD_SECTORS_PER_TRACK
        } else {
            usize::from(self.sector - 1)
        }
    }

    fn finish_not_ready(&mut self) {
        self.xfer = Xfer::Idle;
        self.drq = false;
        self.intrq = true;
        self.buffer.clear();
        self.buffer_i = 0;
        self.status = STAT_NOT_READY;
        self.type_i_status = false;
    }

    fn finish_rnf(&mut self) {
        self.xfer = Xfer::Idle;
        self.drq = false;
        self.intrq = true;
        self.buffer.clear();
        self.buffer_i = 0;
        self.status = STAT_RNF;
        self.type_i_status = false;
    }

    fn set_type_i_status(&mut self) {
        self.type_i_status = true;
        self.xfer = Xfer::Idle;
        self.drq = false;
        self.intrq = true;
        let mut s = 0u8;
        if !self.disk_ready() {
            s |= STAT_NOT_READY;
        }
        if self.track == 0 {
            s |= STAT_TRACK0;
        }
        s |= STAT_HEAD;
        self.status = s;
    }

    fn write_command(&mut self, cmd: u8) {
        self.intrq = false;
        self.drq = false;

        if cmd & 0xf0 == 0xd0 {
            self.xfer = Xfer::Idle;
            self.buffer.clear();
            self.buffer_i = 0;
            self.drq = false;
            let immediate = cmd & 0x08 != 0;
            if self.type_i_status {
                self.set_type_i_status();
            } else {
                self.status &= !STAT_BUSY;
                self.status &= !STAT_DRQ;
            }
            self.intrq = immediate;
            return;
        }

        if !self.disk_ready() {
            self.finish_not_ready();
            return;
        }

        let nibble = cmd & 0xf0;
        match nibble {
            0x00 => {
                self.track = 0;
                self.step_in = false;
                self.set_type_i_status();
            }
            0x10 => {
                self.step_in = self.data_reg >= self.track;
                self.track = self.data_reg;
                self.set_type_i_status();
            }
            0x20 | 0x30 => self.do_step(cmd & 0x10 != 0),
            0x40 | 0x50 => {
                self.step_in = true;
                self.do_step(cmd & 0x10 != 0);
            }
            0x60 | 0x70 => {
                self.step_in = false;
                self.do_step(cmd & 0x10 != 0);
            }
            0x80 | 0x90 => {
                self.multiple = cmd & 0x10 != 0;
                self.type_i_status = false;
                self.start_read_sector();
            }
            0xa0 | 0xb0 => {
                self.multiple = cmd & 0x10 != 0;
                self.type_i_status = false;
                self.start_write_sector();
            }
            0xc0 => {
                self.type_i_status = false;
                self.start_read_address();
            }
            0xe0 | 0xf0 => {
                self.type_i_status = false;
                self.xfer = Xfer::Idle;
                self.intrq = true;
                self.status = 0;
            }
            _ => {
                self.status = 0;
                self.intrq = true;
            }
        }
    }

    fn do_step(&mut self, update_track: bool) {
        if update_track {
            if self.step_in {
                if self.track < 79 {
                    self.track = self.track.saturating_add(1);
                }
            } else if self.track > 0 {
                self.track = self.track.saturating_sub(1);
            }
        }
        self.set_type_i_status();
    }

    fn start_read_sector(&mut self) {
        if !self.load_sector_to_buffer() {
            return;
        }
        self.xfer = Xfer::Read;
        self.drq = true;
        self.intrq = false;
        self.status = STAT_DRQ;
    }

    fn start_write_sector(&mut self) {
        let idx = self.sector_index();
        if idx >= TRD_SECTORS_PER_TRACK {
            self.finish_rnf();
            return;
        }
        let Some(img) = self.image.as_ref() else {
            self.finish_not_ready();
            return;
        };
        if img
            .read_sector_chs(self.track, self.side(), idx as u8)
            .is_none()
        {
            self.finish_rnf();
            return;
        }
        self.buffer = vec![0u8; TRD_SECTOR_SIZE];
        self.buffer_i = 0;
        self.xfer = Xfer::Write;
        self.drq = true;
        self.intrq = false;
        self.status = STAT_DRQ;
    }

    fn start_read_address(&mut self) {
        let idx = self.sector_index();
        if idx >= TRD_SECTORS_PER_TRACK {
            self.finish_rnf();
            return;
        }
        let Some(img) = self.image.as_ref() else {
            self.finish_not_ready();
            return;
        };
        if img
            .read_sector_chs(self.track, self.side(), idx as u8)
            .is_none()
        {
            self.finish_rnf();
            return;
        }
        self.buffer = vec![self.track, self.side(), self.sector.max(1), 0x01, 0, 0];
        self.buffer_i = 0;
        self.xfer = Xfer::ReadAddress;
        self.drq = true;
        self.intrq = false;
        self.status = STAT_DRQ;
    }

    /// Latch track / sector then load sector into the data buffer.
    pub fn load_sector_to_buffer(&mut self) -> bool {
        let Some(img) = self.image.as_ref() else {
            self.finish_not_ready();
            return false;
        };
        let sec_idx = self.sector_index();
        let Some(sec) = img.read_sector_chs(self.track, self.side(), sec_idx as u8) else {
            self.finish_rnf();
            return false;
        };
        self.buffer = sec.to_vec();
        self.buffer_i = 0;
        self.drq = true;
        self.intrq = false;
        self.status = STAT_DRQ;
        debug_assert_eq!(self.buffer.len(), TRD_SECTOR_SIZE);
        true
    }

    fn write_data(&mut self, value: u8) {
        match self.xfer {
            Xfer::Write => {
                if self.buffer_i < self.buffer.len() {
                    self.buffer[self.buffer_i] = value;
                    self.buffer_i += 1;
                }
                self.data_reg = value;
                if self.buffer_i >= self.buffer.len() {
                    self.commit_write_sector();
                } else {
                    self.drq = true;
                    self.status = STAT_DRQ;
                }
            }
            _ => {
                self.data_reg = value;
            }
        }
    }

    fn commit_write_sector(&mut self) {
        let idx = self.sector_index();
        let track = self.track;
        let side = self.side();
        let mut data = [0u8; TRD_SECTOR_SIZE];
        let n = self.buffer.len().min(TRD_SECTOR_SIZE);
        data[..n].copy_from_slice(&self.buffer[..n]);
        let ok = self
            .image
            .as_mut()
            .is_some_and(|img| img.write_sector_chs(track, side, idx as u8, &data));
        if !ok {
            self.finish_rnf();
            return;
        }
        if self.multiple {
            self.advance_multi();
            if self.xfer == Xfer::Write {
                self.buffer.fill(0);
                self.buffer_i = 0;
                self.drq = true;
                self.intrq = false;
                self.status = STAT_DRQ;
                return;
            }
        }
        self.xfer = Xfer::Idle;
        self.drq = false;
        self.intrq = true;
        self.status = 0;
        self.buffer.clear();
        self.buffer_i = 0;
    }

    fn advance_multi(&mut self) {
        let next = self.sector.saturating_add(1);
        if usize::from(next) > TRD_SECTORS_PER_TRACK {
            self.xfer = Xfer::Idle;
            return;
        }
        self.sector = next;
    }

    pub fn read_data(&mut self) -> u8 {
        if !matches!(self.xfer, Xfer::Read | Xfer::ReadAddress) {
            return self.data_reg;
        }
        if self.buffer_i >= self.buffer.len() {
            self.complete_read_byte_stream();
            return 0xff;
        }
        let v = self.buffer[self.buffer_i];
        self.buffer_i += 1;
        self.data_reg = v;
        if self.buffer_i >= self.buffer.len() {
            self.complete_read_byte_stream();
        } else {
            self.drq = true;
            self.status = STAT_DRQ;
        }
        v
    }

    fn complete_read_byte_stream(&mut self) {
        if self.xfer == Xfer::Read && self.multiple {
            self.advance_multi();
            if self.xfer == Xfer::Read && self.load_sector_to_buffer() {
                self.xfer = Xfer::Read;
                return;
            }
        }
        self.xfer = Xfer::Idle;
        self.drq = false;
        self.intrq = true;
        self.status &= !STAT_DRQ;
        self.buffer_i = self.buffer.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use formats::TRD_SECTORS_PER_TRACK;

    fn one_track_image(marker0: u8, marker1: u8) -> TrdImage {
        let mut raw = vec![0u8; TRD_SECTOR_SIZE * TRD_SECTORS_PER_TRACK];
        raw[0] = marker0;
        raw[1] = marker1;
        TrdImage::parse(&raw).unwrap()
    }

    #[test]
    fn port_protocol_reads_sector() {
        let img = one_track_image(0x12, 0x34);
        let mut beta = BetaDisk::new();
        beta.insert(img);
        beta.page_trdos(true);
        beta.out_port(0x003f, 0);
        beta.out_port(0x005f, 1);
        beta.out_port(0x001f, 0x80);
        assert_eq!(beta.in_port(0x001f), Some(0x02));
        assert_eq!(beta.in_port(0x007f), Some(0x12));
        assert_eq!(beta.in_port(0x007f), Some(0x34));
    }

    #[test]
    fn restore_seek_and_track0_status() {
        let mut beta = BetaDisk::new();
        beta.insert(one_track_image(0, 0));
        beta.page_trdos(true);
        beta.out_port(0x007f, 5);
        beta.out_port(0x001f, 0x18);
        assert_eq!(beta.track, 5);
        assert_eq!(beta.in_port(0x00ff).unwrap() & 0x80, 0x80);
        let st = beta.in_port(0x001f).unwrap();
        assert_eq!(st & STAT_TRACK0, 0);
        assert_eq!(beta.in_port(0x00ff).unwrap() & 0x80, 0);
        beta.out_port(0x001f, 0x08);
        assert_eq!(beta.track, 0);
        let st = beta.in_port(0x001f).unwrap();
        assert_ne!(st & STAT_TRACK0, 0);
    }

    #[test]
    fn step_in_and_out_update_track() {
        let mut beta = BetaDisk::new();
        beta.insert(one_track_image(0, 0));
        beta.page_trdos(true);
        beta.out_port(0x001f, 0x58);
        assert_eq!(beta.track, 1);
        beta.out_port(0x001f, 0x78);
        assert_eq!(beta.track, 0);
    }

    #[test]
    fn read_sector_sets_drq_then_intrq_on_port_ff() {
        let img = one_track_image(0xaa, 0xbb);
        let mut beta = BetaDisk::new();
        beta.insert(img);
        beta.page_trdos(true);
        beta.out_port(0x003f, 0);
        beta.out_port(0x005f, 1);
        beta.out_port(0x001f, 0x80);
        assert_eq!(beta.in_port(0x00ff), Some(0x40));
        for _ in 0..TRD_SECTOR_SIZE {
            let _ = beta.in_port(0x007f);
        }
        assert_eq!(beta.in_port(0x00ff), Some(0x80));
    }

    #[test]
    fn write_sector_commits_to_image() {
        let mut beta = BetaDisk::new();
        beta.insert(one_track_image(0, 0));
        beta.page_trdos(true);
        beta.out_port(0x003f, 0);
        beta.out_port(0x005f, 1);
        beta.out_port(0x001f, 0xa0);
        assert_eq!(beta.in_port(0x00ff), Some(0x40));
        for i in 0..TRD_SECTOR_SIZE {
            beta.out_port(0x007f, i as u8);
        }
        assert_eq!(beta.in_port(0x00ff), Some(0x80));
        let sec = beta.image.as_ref().unwrap().read_sector(0, 0).unwrap();
        assert_eq!(sec[0], 0);
        assert_eq!(sec[1], 1);
        assert_eq!(sec[255], 255);
    }

    #[test]
    fn force_interrupt_clears_drq() {
        let mut beta = BetaDisk::new();
        beta.insert(one_track_image(0x11, 0x22));
        beta.page_trdos(true);
        beta.out_port(0x003f, 0);
        beta.out_port(0x005f, 1);
        beta.out_port(0x001f, 0x80);
        assert_eq!(beta.in_port(0x00ff), Some(0x40));
        beta.out_port(0x001f, 0xd0);
        assert_eq!(beta.in_port(0x00ff).unwrap() & 0x40, 0);
    }

    #[test]
    fn disk_not_ready_without_image() {
        let mut beta = BetaDisk::new();
        beta.page_trdos(true);
        assert!(beta.out_port(0x001f, 0x80));
        assert_eq!(beta.status, STAT_NOT_READY);
        assert_eq!(beta.in_port(0x00ff), Some(0x80)); // INTRQ before status read
        assert_eq!(beta.in_port(0x001f), Some(STAT_NOT_READY));
        assert_eq!(beta.in_port(0x00ff), Some(0), "status read clears INTRQ");
    }

    #[test]
    fn drive_b_is_not_ready() {
        let mut beta = BetaDisk::new();
        beta.insert(one_track_image(0, 0));
        beta.page_trdos(true);
        beta.out_port(0x00ff, 0x3d);
        beta.out_port(0x001f, 0x80);
        assert_eq!(beta.status, STAT_NOT_READY);
        assert_eq!(beta.in_port(0x001f), Some(STAT_NOT_READY));
    }

    #[test]
    fn record_not_found_on_bad_sector() {
        let mut beta = BetaDisk::new();
        beta.insert(one_track_image(0, 0));
        beta.page_trdos(true);
        beta.out_port(0x003f, 0);
        beta.out_port(0x005f, 99);
        beta.out_port(0x001f, 0x80);
        assert_eq!(beta.in_port(0x001f), Some(STAT_RNF));
    }

    #[test]
    fn read_address_returns_id_field() {
        let mut beta = BetaDisk::new();
        beta.insert(one_track_image(0, 0));
        beta.page_trdos(true);
        beta.out_port(0x003f, 0);
        beta.out_port(0x005f, 1);
        beta.out_port(0x001f, 0xc0);
        assert_eq!(beta.in_port(0x007f), Some(0));
        assert_eq!(beta.in_port(0x007f), Some(0));
        assert_eq!(beta.in_port(0x007f), Some(1));
        assert_eq!(beta.in_port(0x007f), Some(1));
    }

    #[test]
    fn multiple_read_walks_sectors() {
        let mut raw = vec![0u8; TRD_SECTOR_SIZE * TRD_SECTORS_PER_TRACK];
        raw[0] = 0xa1;
        raw[TRD_SECTOR_SIZE] = 0xa2;
        let img = TrdImage::parse(&raw).unwrap();
        let mut beta = BetaDisk::new();
        beta.insert(img);
        beta.page_trdos(true);
        beta.out_port(0x003f, 0);
        beta.out_port(0x005f, 1);
        beta.out_port(0x001f, 0x90);
        assert_eq!(beta.in_port(0x007f), Some(0xa1));
        for _ in 1..TRD_SECTOR_SIZE {
            let _ = beta.in_port(0x007f);
        }
        assert_eq!(beta.sector, 2);
        assert_eq!(beta.in_port(0x007f), Some(0xa2));
    }

    #[test]
    fn m1_pages_rom_in_at_3d00_and_out_at_4000() {
        let mut rom = [0u8; TRDOS_ROM_SIZE];
        rom[0x3d00] = 0xc3;
        rom[0] = 0x42;
        let mut beta = BetaDisk::new();
        beta.load_rom(&rom).unwrap();
        assert!(!beta.paged);
        assert!(beta.read_rom(0).is_none());
        beta.notify_m1(0x3d00, 0x3c00);
        assert!(beta.paged);
        assert_eq!(beta.read_rom(0), Some(0x42));
        assert_eq!(beta.read_rom(0x3d00), Some(0xc3));
        beta.notify_m1(0x4000, 0x3c00);
        assert!(!beta.paged);
        assert!(beta.read_rom(0).is_none());
    }

    #[test]
    fn m1_does_not_unpage_while_executing_trdos_rom() {
        let mut rom = [0u8; TRDOS_ROM_SIZE];
        rom[0x0100] = 0x76;
        let mut beta = BetaDisk::new();
        beta.load_rom(&rom).unwrap();
        beta.notify_m1(0x3d00, 0x3c00);
        beta.notify_m1(0x0100, 0x3c00);
        assert!(beta.paged);
        assert_eq!(beta.read_rom(0x0100), Some(0x76));
    }

    #[test]
    fn notify_m1_ignored_until_rom_loaded() {
        let mut beta = BetaDisk::new();
        beta.page_trdos(true);
        beta.notify_m1(0x4000, 0x3c00);
        assert!(beta.paged, "port-only attach must keep manual paging");
    }

    #[test]
    fn sector_zero_is_record_not_found() {
        let mut beta = BetaDisk::new();
        beta.insert(one_track_image(0, 0));
        beta.page_trdos(true);
        beta.out_port(0x003f, 0);
        beta.out_port(0x005f, 0);
        beta.out_port(0x001f, 0x80);
        assert_eq!(beta.in_port(0x001f), Some(STAT_RNF));
        // Sector 1 still intact (sector 0 must not alias to it).
        beta.out_port(0x005f, 1);
        beta.out_port(0x001f, 0x80);
        assert_eq!(beta.in_port(0x007f), Some(0));
    }

    #[test]
    fn m1_48k_pages_at_3c00_128k_does_not() {
        let mut rom = [0u8; TRDOS_ROM_SIZE];
        rom[0] = 0x42;
        let mut beta = BetaDisk::new();
        beta.load_rom(&rom).unwrap();
        beta.notify_m1(0x3c00, 0x3d00);
        assert!(!beta.paged, "128K window must ignore 0x3C00");
        beta.notify_m1(0x3d00, 0x3d00);
        assert!(beta.paged);
        beta.notify_m1(0x4000, 0x3d00);
        assert!(!beta.paged);
        beta.notify_m1(0x3c00, 0x3c00);
        assert!(beta.paged, "48K window pages at 0x3C00");
    }
}
