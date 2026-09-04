//! Beta Disk Interface / TR-DOS: VG93 (KR1818VG93 / WD1793-class) FDC.
//!
//! Ports (low byte, when TR-DOS is paged):
//! - Classic Beta 128: `0x1F` cmd/status, `0x3F` track, `0x5F` sector, `0x7F`
//!   data, `0xFF` system.
//! - Alone Coder / VfNG **5.04T** (complete ROM): `0x08` cmd/status, `0x28`
//!   track, `0x48` sector, `0x68` data (plus write aliases `0x2A` track,
//!   `0x6A` data). Same VG93 registers; different address decode.
//!
//! System port `0xFF` (Fuse / Beta 128): write selects drive (`D0–D1`), VG93
//! `/RES` (`D2` low = reset), HLT (`D3`), side (`D4` set = side 0), MFM (`D5`).
//! Read: `D7` = INTRQ, `D6` = DRQ. 5.04T mostly polls status on `0x08` instead.
//!
//! Opcode-fetch (M1) paging: ROM in at `0x3C00–0x3DFF` (48K) or `0x3D00–0x3DFF`
//! (128K). The latch stays set across RAM execution until [`BetaDisk::page_trdos`]
//! clears it.

use formats::{TrdImage, TRD_SECTORS_PER_TRACK, TRD_SECTOR_SIZE};

/// TR-DOS / Beta 128 ROM size (16 KiB overlay at `0000–3FFF`).
pub const TRDOS_ROM_SIZE: usize = 16384;

// Fuse `wd_fdc.h`: BUSY=0, IDX/DRQ=1, TRACK0=2, RNF=4, HEAD/SPINUP=5, NOT_READY=7.
const STAT_BUSY: u8 = 0x01;
const STAT_DRQ: u8 = 0x02;
const STAT_TRACK0: u8 = 0x04;
const STAT_RNF: u8 = 0x10;
const STAT_HEAD: u8 = 0x20;
const STAT_NOT_READY: u8 = 0x80;

/// Default system latch: drive 0, HLT, side 0, MFM (typical TR-DOS `OUT (#FF)`).
const SYS_DEFAULT: u8 = 0x3c;

fn trace_fdc(port: u16, write: bool, value: u8) {
    if trace::enabled(trace::Category::DISK) {
        trace::emit(trace::EventKind::DiskFdc { port, write, value });
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Xfer {
    Idle,
    Read,
    Write,
    ReadAddress,
    WriteTrack,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WriteTrackParse {
    Hunt,
    Id { i: u8, buf: [u8; 4] },
    Data { size: usize, chs: (u8, u8, u8) },
}

#[derive(Clone, Debug)]
pub struct BetaDisk {
    pub track: u8,
    pub sector: u8,
    pub status: u8,
    pub system: u8,
    /// When true, TR-DOS is paged and Beta claims VG93 ports (classic
    /// `1Fh`/`3Fh`/`5Fh`/`7Fh`/`FFh`, plus 5.04T aliases `08h`/`28h`/`48h`/`68h`).
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
    /// VG93 commands issued (catalog / RUN diagnostics).
    pub cmd_count: u32,
    /// Successful Type II sector loads.
    pub sector_read_count: u32,
    /// Completed WRITE TRACK / format passes.
    pub write_track_count: u32,
    /// Most recent command bytes (oldest → newest in `[..cmd_count.min(4)]` order is not kept; last four).
    pub recent_cmds: [u8; 4],
    cmd_ring: [u8; 64],
    cmd_ring_len: u8,
    wt_parse: WriteTrackParse,
    wt_pending_id: Option<(u8, u8, u8, u8)>,
    wt_sectors_done: u8,
    wt_fill: u8,
    /// One status read after Type I completion exposes bit 0x04 (TR-DOS `3E30` `AND #04`).
    seek_complete_pulse: bool,
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
            status: STAT_TRACK0 | STAT_HEAD,
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
            cmd_count: 0,
            sector_read_count: 0,
            write_track_count: 0,
            recent_cmds: [0; 4],
            cmd_ring: [0; 64],
            cmd_ring_len: 0,
            wt_parse: WriteTrackParse::Hunt,
            wt_pending_id: None,
            wt_sectors_done: 0,
            wt_fill: 0xe5,
            seek_complete_pulse: false,
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

    /// Overwrite bytes in the loaded TR-DOS ROM image (test / harness hooks).
    pub fn patch_rom(&mut self, addr: u16, bytes: &[u8]) -> Result<(), String> {
        let start = usize::from(addr);
        let end = start.saturating_add(bytes.len());
        if !self.rom_loaded {
            return Err("TR-DOS ROM not loaded".into());
        }
        if end > TRDOS_ROM_SIZE {
            return Err(format!(
                "TR-DOS ROM patch {addr:#06x}+{} out of range",
                bytes.len()
            ));
        }
        self.rom[start..end].copy_from_slice(bytes);
        Ok(())
    }

    /// Recent VG93 command bytes for diagnostics (newest at `cmd_ring_len - 1`).
    #[must_use]
    pub fn command_ring(&self) -> &[u8] {
        let n = usize::from(self.cmd_ring_len.min(64));
        &self.cmd_ring[..n]
    }

    /// Page / hide TR-DOS (enables Beta port decode when true).
    pub fn page_trdos(&mut self, on: bool) {
        self.paged = on;
    }

    /// M1 (opcode fetch) paging used by TR-DOS `USR 15616` (`0x3D00`).
    ///
    /// `page_in_lo` is the start of the page-in window through `0x3DFF`:
    /// `0x3C00` for 48K Beta, `0x3D00` for Beta 128. The latch stays set across
    /// RAM (`PC >= 0x4000`) execution so mixed ROM/RAM paths can resume in TR-DOS
    /// below `0x4000` without re-entering through `3D00–3DFF`. Clear with
    /// [`Self::page_trdos`] (`false`) when leaving DOS.
    pub fn notify_m1(&mut self, pc: u16, page_in_lo: u16) {
        if !self.rom_loaded {
            return;
        }
        if (page_in_lo..0x3e00).contains(&pc) {
            self.paged = true;
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

    /// Map classic Beta / 5.04T low-byte ports onto the five VG93 registers.
    ///
    /// `for_write`: when true, include 5.04T write-only aliases `#2A` (track) and
    /// `#6A` (data). Reads use only the bidirectional ports.
    #[inline]
    fn map_vg93_port(port: u16, for_write: bool) -> Option<u8> {
        Some(match port & 0xff {
            // cmd/status
            0x1f | 0x08 => 0x1f,
            // track (`#2A` is write-only on 5.04T)
            0x3f | 0x28 => 0x3f,
            0x2a if for_write => 0x3f,
            // sector
            0x5f | 0x48 => 0x5f,
            // data (`#6A` is write-only on 5.04T)
            0x7f | 0x68 => 0x7f,
            0x6a if for_write => 0x7f,
            // system (classic `#FF` only; 5.04T polls `#08` status instead)
            0xff => 0xff,
            _ => return None,
        })
    }

    /// `OUT` to Beta ports. Returns true if handled.
    pub fn out_port(&mut self, port: u16, value: u8) -> bool {
        if !self.paged {
            return false;
        }
        let Some(reg) = Self::map_vg93_port(port, true) else {
            return false;
        };
        let handled = match reg {
            0x1f => {
                if self.system & 0x04 == 0 {
                    true
                } else {
                    self.write_command(value);
                    true
                }
            }
            0x3f => {
                self.track = value;
                // TR-DOS / Beta issue SEEK after OUT (#3Fh); latch target for Type I seek.
                self.data_reg = value;
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
                // D2 low holds VG93 /RES (Beta 128; TR-DOS `XOR A; OUT (#FF),A`).
                // SS images: TR-DOS may clear D2 when selecting "side 1" — keep /RES deasserted.
                let mut value = value;
                if self.image.as_ref().is_some_and(|img| img.sides <= 1) && value != 0 {
                    value |= 0x04;
                }
                self.system = value;
                if value & 0x04 == 0 {
                    self.reset_vg93();
                }
                true
            }
            _ => false,
        };
        if handled {
            trace_fdc(port, true, value);
        }
        handled
    }

    /// `IN` from Beta ports.
    pub fn in_port(&mut self, port: u16) -> Option<u8> {
        if !self.paged {
            return None;
        }
        let reg = Self::map_vg93_port(port, false)?;
        let value = match reg {
            0x1f => {
                self.intrq = false;
                // Fuse `wd_fdc_sr_read`: Type I status reads expose an index pulse on bit 1.
                let mut s = self.status;
                if self.seek_complete_pulse {
                    s |= STAT_TRACK0;
                    self.seek_complete_pulse = false;
                }
                if self.type_i_status && self.xfer == Xfer::Idle {
                    s |= STAT_DRQ;
                }
                s
            }
            0x3f => self.track,
            0x5f => self.sector,
            0x7f => self.read_data(),
            0xff => {
                let mut v = 0u8;
                if self.intrq {
                    v |= 0x80;
                }
                if self.drq {
                    v |= 0x40;
                }
                v
            }
            _ => return None,
        };
        trace_fdc(port, false, value);
        Some(value)
    }

    fn reset_vg93(&mut self) {
        self.xfer = Xfer::Idle;
        self.drq = false;
        self.intrq = false;
        self.buffer.clear();
        self.buffer_i = 0;
        self.multiple = false;
        self.type_i_status = true;
        self.wt_parse = WriteTrackParse::Hunt;
        self.wt_pending_id = None;
        self.wt_sectors_done = 0;
        self.seek_complete_pulse = false;
        self.status = self.type_i_status_value();
    }

    fn type_i_status_value(&self) -> u8 {
        let mut s = 0u8;
        if !self.disk_ready() {
            s |= STAT_NOT_READY;
        } else {
            s |= STAT_HEAD;
        }
        if self.track == 0 {
            s |= STAT_TRACK0;
        }
        s
    }

    fn drive(&self) -> u8 {
        self.system & 0x03
    }

    /// Fuse: bit 4 set → head 0 (bottom).
    fn side(&self) -> u8 {
        u8::from(self.system & 0x10 == 0)
    }

    /// Single-sided images ignore the side latch (TR-DOS may OUT `#60` on SS disks).
    fn effective_side(&self) -> u8 {
        if self.image.as_ref().is_some_and(|img| img.sides <= 1) {
            0
        } else {
            self.side()
        }
    }

    fn disk_ready(&self) -> bool {
        self.image.is_some() && self.drive() == 0
    }

    /// WD1793 sector register is usually `1..=16`; TR-DOS also OUTs `0` for the first
    /// physical sector (catalog start sector 0 → VG93 ID 1).
    fn sector_index(&self) -> usize {
        let id = self.sector.max(1);
        if usize::from(id) > TRD_SECTORS_PER_TRACK {
            TRD_SECTORS_PER_TRACK
        } else {
            usize::from(id - 1)
        }
    }

    fn sector_index_for_read_address(&self) -> usize {
        if self.sector == 0 {
            0
        } else {
            self.sector_index()
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
        self.status = self.type_i_status_value();
        // TR-DOS catalog seek (`trdos.rom` `3E30`: `IN A,(#1F); AND #04`) waits for bit
        // 0x04 after SEEK even when the target cylinder is not 0. Fuse steady-state TRK0
        // only reflects `track==0`; pulse seek-complete once on the next status read.
        self.seek_complete_pulse = true;
    }

    fn write_command(&mut self, cmd: u8) {
        // A new command supersedes any pending Type I completion pulse; otherwise a
        // Type II status read would spuriously report bit 0x04.
        self.seek_complete_pulse = false;
        self.intrq = false;
        self.drq = false;
        self.cmd_count = self.cmd_count.saturating_add(1);
        self.recent_cmds.rotate_left(1);
        self.recent_cmds[3] = cmd;
        let i = usize::from(self.cmd_ring_len.min(63));
        self.cmd_ring[i] = cmd;
        self.cmd_ring_len = self.cmd_ring_len.saturating_add(1);

        // Type IV — Force Interrupt (`D0h`–`DFh`). `D8h` is overloaded: TR-DOS issues
        // it as Read Address (`C0h|#18`) after seek; WRITE TRACK completion uses the
        // same byte as Force Interrupt (see `write_track_formats_sector_with_data`).
        if cmd & 0xf0 == 0xd0 {
            if cmd == 0xd8 && self.xfer != Xfer::WriteTrack {
                if !self.disk_ready() {
                    self.finish_not_ready();
                    return;
                }
                self.type_i_status = false;
                self.start_read_address();
                return;
            }
            let was_write_track = self.xfer == Xfer::WriteTrack;
            self.xfer = Xfer::Idle;
            self.buffer.clear();
            self.buffer_i = 0;
            self.drq = false;
            let immediate = cmd & 0x08 != 0;
            if was_write_track {
                self.finish_write_track();
            } else if self.type_i_status {
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

        // WD1793 / KR1818VG93 command classes (Fuse `wd_fdc_cr_write` layout).
        if cmd & 0x80 == 0 {
            match cmd & 0xf0 {
                0x00 => {
                    self.track = 0;
                    self.step_in = false;
                    self.set_type_i_status();
                }
                0x10 => {
                    // Fuse `wd_fdc_type_i`: SEEK sets track from the data register.
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
                _ => {
                    self.status = 0;
                    self.intrq = true;
                }
            }
        } else if cmd & 0x40 == 0 {
            self.multiple = cmd & 0x10 != 0;
            self.type_i_status = false;
            if cmd & 0x20 != 0 {
                self.start_write_sector();
            } else {
                self.start_read_sector();
            }
        } else if (cmd & 0xc0) == 0xc0 {
            // TR-DOS sets command bit 4 for side/MFM modifiers (`E9h` read track, `F9h` ≠ write).
            self.type_i_status = false;
            if (cmd & 0x30) != 0x10 {
                if cmd & 0x20 == 0 {
                    self.start_read_address();
                } else if cmd == 0xf9 || cmd & 0x10 == 0 {
                    // TR-DOS `F9h` is read-track with modifiers (not write-track).
                    self.start_read_track();
                } else {
                    self.start_write_track();
                }
            } else {
                self.status = 0;
                self.intrq = true;
            }
        } else {
            self.status = 0;
            self.intrq = true;
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

    fn start_write_track(&mut self) {
        self.wt_parse = WriteTrackParse::Hunt;
        self.wt_pending_id = None;
        self.wt_sectors_done = 0;
        self.buffer.clear();
        self.buffer_i = 0;
        self.xfer = Xfer::WriteTrack;
        self.drq = true;
        self.intrq = false;
        self.status = STAT_BUSY | STAT_DRQ;
    }

    fn finish_write_track(&mut self) {
        self.xfer = Xfer::Idle;
        self.drq = false;
        self.intrq = true;
        self.status = 0;
        self.wt_parse = WriteTrackParse::Hunt;
        self.wt_pending_id = None;
        self.buffer.clear();
        self.buffer_i = 0;
        if self.wt_sectors_done > 0 {
            self.write_track_count = self.write_track_count.saturating_add(1);
        }
        self.wt_sectors_done = 0;
    }

    fn sector_size_from_code(n: u8) -> usize {
        match n {
            0 => 128,
            1 => 256,
            2 => 512,
            3 => 1024,
            _ => TRD_SECTOR_SIZE,
        }
    }

    fn commit_write_track_sector(&mut self, track: u8, side: u8, sector_id: u8, data: &[u8]) {
        if sector_id == 0 || sector_id as usize > TRD_SECTORS_PER_TRACK {
            return;
        }
        let mut sec = [0u8; TRD_SECTOR_SIZE];
        let n = data.len().min(TRD_SECTOR_SIZE);
        sec[..n].copy_from_slice(&data[..n]);
        if sec[n..].iter().all(|&b| b == 0) && n < TRD_SECTOR_SIZE {
            sec[n..].fill(self.wt_fill);
        }
        if self
            .image
            .as_mut()
            .is_some_and(|img| img.write_sector_chs(track, side, sector_id - 1, &sec))
        {
            self.wt_sectors_done = self.wt_sectors_done.saturating_add(1);
        }
    }

    fn write_track_byte(&mut self, value: u8) {
        match self.wt_parse {
            WriteTrackParse::Hunt => {
                if value == 0xfe {
                    self.wt_parse = WriteTrackParse::Id { i: 0, buf: [0; 4] };
                } else if value == 0xfb {
                    let (c, h, r, n) = self.wt_pending_id.take().unwrap_or((
                        self.track,
                        self.side(),
                        self.sector.max(1),
                        0x01,
                    ));
                    let size = Self::sector_size_from_code(n);
                    self.buffer.clear();
                    self.wt_parse = WriteTrackParse::Data {
                        size,
                        chs: (c, h, r),
                    };
                }
            }
            WriteTrackParse::Id { i, mut buf } => {
                buf[i as usize] = value;
                if i < 3 {
                    self.wt_parse = WriteTrackParse::Id { i: i + 1, buf };
                } else {
                    self.wt_pending_id = Some((buf[0], buf[1], buf[2], buf[3]));
                    self.wt_parse = WriteTrackParse::Hunt;
                }
            }
            WriteTrackParse::Data { size, chs } => {
                if self.buffer.len() < size {
                    self.buffer.push(value);
                }
                if self.buffer.len() >= size {
                    let data = self.buffer.clone();
                    self.commit_write_track_sector(chs.0, chs.1, chs.2, &data);
                    self.buffer.clear();
                    self.wt_parse = WriteTrackParse::Hunt;
                }
            }
        }
        if self.wt_sectors_done >= TRD_SECTORS_PER_TRACK as u8 {
            self.finish_write_track();
        }
    }

    fn start_read_sector(&mut self) {
        if !self.load_sector_to_buffer() {
            return;
        }
        self.xfer = Xfer::Read;
        self.drq = true;
        self.intrq = false;
        // BUSY+DRQ: Alone Coder 5.04T `0975h` waits for status bit 0 then drains on DRQ.
        self.status = STAT_BUSY | STAT_DRQ;
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
            .read_sector_chs(self.track, self.effective_side(), idx as u8)
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
        self.status = STAT_BUSY | STAT_DRQ;
    }

    fn start_read_address(&mut self) {
        let track = self.track;
        let side = self.effective_side();
        let Some(img) = self.image.as_ref() else {
            self.finish_not_ready();
            return;
        };
        // TR-DOS often leaves sector reg at 0 for address-scan; WD1793 still returns
        // the next ID field on the track (first sector for our linear image model).
        let sec_idx = self.sector_index_for_read_address();
        if sec_idx >= TRD_SECTORS_PER_TRACK {
            self.finish_rnf();
            return;
        }
        if img.read_sector_chs(track, side, sec_idx as u8).is_none() {
            self.finish_rnf();
            return;
        }
        let sector_id = sec_idx as u8 + 1;
        self.buffer = vec![track, side, sector_id, 0x01, 0xf3, 0xfc];
        self.buffer_i = 0;
        self.xfer = Xfer::ReadAddress;
        self.drq = true;
        self.intrq = false;
        self.status = STAT_BUSY | STAT_DRQ;
    }

    /// Type III read track: TR-DOS issues `E0h|seek` (`F9h` for seek `19h`) to verify the
    /// index but never drains the DRQ byte stream (the ROM has no `IN` from `7Fh` on this
    /// path). Instant completion matches index-to-index behaviour without rotation.
    fn start_read_track(&mut self) {
        let Some(img) = self.image.as_ref() else {
            self.finish_not_ready();
            return;
        };
        let track = self.track;
        let side = self.effective_side();
        if img.read_sector_chs(track, side, 0).is_none() {
            self.finish_rnf();
            return;
        }
        self.xfer = Xfer::Idle;
        self.drq = false;
        self.intrq = true;
        self.type_i_status = true;
        self.status = self.type_i_status_value();
        self.buffer.clear();
        self.buffer_i = 0;
    }

    /// Latch track / sector then load sector into the data buffer.
    pub fn load_sector_to_buffer(&mut self) -> bool {
        let Some(img) = self.image.as_ref() else {
            self.finish_not_ready();
            return false;
        };
        if usize::from(self.sector.max(1)) > TRD_SECTORS_PER_TRACK {
            self.finish_rnf();
            return false;
        }
        let sec_idx = self.sector_index();
        let Some(sec) = img.read_sector_chs(self.track, self.effective_side(), sec_idx as u8)
        else {
            self.finish_rnf();
            return false;
        };
        self.buffer = sec.to_vec();
        self.buffer_i = 0;
        self.drq = true;
        self.intrq = false;
        self.status = STAT_BUSY | STAT_DRQ;
        self.sector_read_count = self.sector_read_count.saturating_add(1);
        debug_assert_eq!(self.buffer.len(), TRD_SECTOR_SIZE);
        true
    }

    fn write_data(&mut self, value: u8) {
        match self.xfer {
            Xfer::WriteTrack => {
                self.write_track_byte(value);
                self.data_reg = value;
                if self.xfer == Xfer::WriteTrack {
                    self.drq = true;
                    self.status = STAT_BUSY | STAT_DRQ;
                }
            }
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
                    self.status = STAT_BUSY | STAT_DRQ;
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
        let side = self.effective_side();
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
                self.status = STAT_BUSY | STAT_DRQ;
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
        if next == 0 || usize::from(next) > TRD_SECTORS_PER_TRACK {
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
            self.status = STAT_BUSY | STAT_DRQ;
        }
        v
    }

    fn complete_read_byte_stream(&mut self) {
        if self.xfer == Xfer::Read && self.multiple {
            self.advance_multi();
            if self.xfer == Xfer::Read {
                // Preserve STAT_NOT_READY / STAT_RNF from a failed continuation load.
                if !self.load_sector_to_buffer() {
                    return;
                }
                self.xfer = Xfer::Read;
                self.drq = true;
                self.status = STAT_BUSY | STAT_DRQ;
                return;
            }
        }
        self.xfer = Xfer::Idle;
        self.drq = false;
        self.intrq = true;
        self.status = 0;
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
        assert_eq!(beta.in_port(0x001f), Some(STAT_BUSY | STAT_DRQ));
        assert_eq!(beta.in_port(0x007f), Some(0x12));
        assert_eq!(beta.in_port(0x007f), Some(0x34));
    }

    /// Multi-sector Type-II: a failed continuation load must keep STAT_NOT_READY.
    #[test]
    fn multi_sector_read_preserves_not_ready_on_continuation() {
        let mut beta = BetaDisk::new();
        beta.insert(one_track_image(0xaa, 0xbb));
        beta.page_trdos(true);
        beta.out_port(0x003f, 0);
        beta.out_port(0x005f, 1);
        beta.out_port(0x001f, 0x90); // Read Sector, multiple
        assert_eq!(beta.in_port(0x001f), Some(STAT_BUSY | STAT_DRQ));
        for _ in 0..(TRD_SECTOR_SIZE - 1) {
            let _ = beta.in_port(0x007f);
        }
        beta.image = None;
        let _ = beta.in_port(0x007f); // last byte → advance + failed load
        assert_eq!(
            beta.status, STAT_NOT_READY,
            "continuation failure must not be wiped to success status 0"
        );
        assert_eq!(beta.in_port(0x001f), Some(STAT_NOT_READY));
    }

    /// Alone Coder / VfNG 5.04T uses `#08/#28/#48/#68` instead of `#1F/#3F/#5F/#7F`.
    #[test]
    fn port_protocol_504t_aliases_read_sector() {
        let img = one_track_image(0x56, 0x78);
        let mut beta = BetaDisk::new();
        beta.insert(img);
        beta.page_trdos(true);
        beta.out_port(0x0028, 0); // track
        beta.out_port(0x0048, 1); // sector
        beta.out_port(0x0008, 0x80); // read sector
        assert_eq!(beta.in_port(0x0008), Some(STAT_BUSY | STAT_DRQ));
        assert_eq!(beta.in_port(0x0068), Some(0x56));
        assert_eq!(beta.in_port(0x0068), Some(0x78));
        assert_eq!(beta.track, 0);
        assert_eq!(beta.sector, 1);
    }

    #[test]
    fn port_protocol_504t_write_only_aliases() {
        let mut beta = BetaDisk::new();
        beta.insert(one_track_image(0, 0));
        beta.page_trdos(true);
        assert!(beta.out_port(0x002a, 7)); // write-only track alias
        assert_eq!(beta.track, 7);
        assert_eq!(beta.in_port(0x0028), Some(7));
        assert_eq!(beta.in_port(0x002a), None, "#2A is write-only");
        assert!(beta.out_port(0x006a, 0xab)); // write-only data alias
        assert_eq!(beta.data_reg, 0xab);
        assert_eq!(beta.in_port(0x006a), None, "#6A is write-only");
    }

    #[test]
    fn restore_seek_and_track0_status() {
        let mut beta = BetaDisk::new();
        beta.insert(one_track_image(0, 0));
        beta.page_trdos(true);
        beta.out_port(0x003f, 5);
        beta.out_port(0x001f, 0x18);
        assert_eq!(beta.track, 5);
        assert_eq!(beta.in_port(0x00ff).unwrap() & 0x80, 0x80);
        let st = beta.in_port(0x001f).unwrap();
        assert_ne!(st & STAT_TRACK0, 0, "seek-complete pulse");
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
    fn trdos_seek_compare_and_track0_pulse() {
        let mut beta = BetaDisk::new();
        beta.insert(TrdImage::synthetic_trdos_boot_basic());
        beta.page_trdos(true);
        beta.out_port(0x00ff, 0x3c);
        beta.out_port(0x007f, 1);
        beta.out_port(0x001f, 0x19);
        assert_eq!(
            beta.in_port(0x00ff).unwrap() & 0x80,
            0x80,
            "INTRQ after seek"
        );
        assert_eq!(beta.in_port(0x003f), Some(1));
        let st = beta.in_port(0x001f).unwrap();
        assert_ne!(st & STAT_TRACK0, 0, "seek-complete pulse (3E30 AND #04)");
    }

    #[test]
    fn seek_compare_out7f_in3f_matches_after_seek() {
        let mut beta = BetaDisk::new();
        beta.insert(TrdImage::synthetic_trdos_boot_basic());
        beta.page_trdos(true);
        beta.out_port(0x00ff, 0x3c);
        beta.out_port(0x003f, 0);
        beta.out_port(0x007f, 1);
        assert_eq!(beta.track, 0, "OUT #7F must not move track register");
        assert_eq!(beta.in_port(0x003f), Some(0));
        beta.out_port(0x001f, 0x1b);
        assert_eq!(beta.track, 1);
        assert_eq!(beta.in_port(0x003f), Some(1));
    }

    #[test]
    fn seek_uses_track_register_not_sector_latch() {
        let mut beta = BetaDisk::new();
        beta.insert(TrdImage::synthetic_trdos_boot_basic());
        beta.page_trdos(true);
        beta.out_port(0x003f, 0);
        beta.out_port(0x005f, 0);
        beta.out_port(0x001f, 0x19);
        assert_eq!(beta.track, 0, "SEEK must not use sector number as cylinder");
        beta.out_port(0x003f, 1);
        beta.out_port(0x007f, 1);
        beta.out_port(0x001f, 0x19);
        assert_eq!(beta.track, 1);
    }

    #[test]
    fn read_track_e9_modifier_is_read_not_write() {
        let mut beta = BetaDisk::new();
        beta.insert(one_track_image(0xaa, 0xbb));
        beta.page_trdos(true);
        beta.out_port(0x003f, 0);
        beta.out_port(0x001f, 0xe9);
        assert_eq!(beta.in_port(0x00ff), Some(0x80), "read track completes");
        assert_eq!(
            beta.in_port(0x00ff).unwrap() & 0x40,
            0,
            "no DRQ during idle read track"
        );
        assert_ne!(
            beta.in_port(0x001f).unwrap() & STAT_DRQ,
            0,
            "Fuse Type I index pulse on status read"
        );
        beta.out_port(0x001f, 0xf0);
        assert_eq!(
            beta.in_port(0x00ff),
            Some(0x40),
            "write track waits for data"
        );
    }

    #[test]
    fn read_track_completes_without_drq() {
        let mut beta = BetaDisk::new();
        beta.insert(one_track_image(0xaa, 0xbb));
        beta.page_trdos(true);
        beta.out_port(0x003f, 0);
        beta.out_port(0x001f, 0xe9);
        assert_eq!(beta.in_port(0x00ff), Some(0x80));
        assert_eq!(beta.in_port(0x00ff).unwrap() & 0x40, 0);
        assert_ne!(beta.in_port(0x001f).unwrap() & STAT_DRQ, 0);
    }

    #[test]
    fn trdos_catalog_seek_read_boot_sector() {
        let mut beta = BetaDisk::new();
        beta.insert(TrdImage::synthetic_trdos_boot_basic());
        beta.page_trdos(true);
        beta.out_port(0x00ff, 0x3c);
        // Track 1 / sector 1 (boot body) — TR-DOS catalog then load path.
        beta.out_port(0x003f, 1);
        beta.out_port(0x005f, 1);
        beta.out_port(0x001f, 0x19);
        assert_eq!(beta.in_port(0x00ff).unwrap() & 0x80, 0x80);
        beta.out_port(0x001f, 0xc0);
        assert_eq!(beta.in_port(0x007f), Some(1)); // C
        assert_eq!(beta.in_port(0x007f), Some(0)); // H
        assert_eq!(beta.in_port(0x007f), Some(1)); // R
        beta.out_port(0x001f, 0x80);
        assert_eq!(beta.in_port(0x007f), Some(0x00)); // boot BASIC
        assert_eq!(beta.in_port(0x007f), Some(0x0a));
        assert_eq!(beta.sector_read_count, 1);
    }

    #[test]
    fn ss_disk_read_with_default_system_port() {
        let mut beta = BetaDisk::new();
        beta.insert(TrdImage::synthetic_trdos_boot_basic());
        beta.page_trdos(true);
        beta.out_port(0x00ff, 0x3c);
        assert_eq!(beta.side(), 0);
        beta.out_port(0x003f, 1);
        beta.out_port(0x005f, 1);
        beta.out_port(0x001f, 0x80);
        assert!(beta.sector_read_count > 0);
        assert_eq!(beta.in_port(0x007f), Some(0x00)); // boot BASIC line marker
    }

    #[test]
    fn read_address_returns_id_field() {
        let mut beta = BetaDisk::new();
        beta.insert(one_track_image(0, 0));
        beta.page_trdos(true);
        beta.out_port(0x003f, 0);
        beta.out_port(0x005f, 0);
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
        assert!(beta.paged, "latch stays set across RAM execution");
        assert_eq!(beta.read_rom(0), Some(0x42));
        beta.page_trdos(false);
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
    fn sector_zero_reads_first_physical_sector() {
        let mut beta = BetaDisk::new();
        beta.insert(one_track_image(0xab, 0xcd));
        beta.page_trdos(true);
        beta.out_port(0x003f, 0);
        beta.out_port(0x005f, 0);
        beta.out_port(0x001f, 0x80);
        assert_eq!(beta.in_port(0x007f), Some(0xab));
        beta.out_port(0x005f, 1);
        beta.out_port(0x001f, 0x80);
        assert_eq!(beta.in_port(0x007f), Some(0xab));
    }

    #[test]
    fn read_address_with_sector_zero_returns_first_id() {
        let mut beta = BetaDisk::new();
        beta.insert(one_track_image(0xab, 0xcd));
        beta.page_trdos(true);
        beta.out_port(0x003f, 0);
        beta.out_port(0x005f, 0);
        beta.out_port(0x001f, 0xc0);
        assert_eq!(beta.in_port(0x007f), Some(0));
        assert_eq!(beta.in_port(0x007f), Some(0));
        assert_eq!(beta.in_port(0x007f), Some(1));
        assert_eq!(beta.in_port(0x007f), Some(1));
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
        assert!(beta.paged, "latch stays set across RAM on 128K");
        beta.page_trdos(false);
        beta.notify_m1(0x3c00, 0x3c00);
        assert!(beta.paged, "48K window pages at 0x3C00");
    }

    #[test]
    fn system_port_bit2_low_resets_vg93() {
        let mut beta = BetaDisk::new();
        beta.insert(one_track_image(0xaa, 0xbb));
        beta.page_trdos(true);
        beta.out_port(0x003f, 0);
        beta.out_port(0x005f, 1);
        beta.out_port(0x001f, 0x80);
        assert_eq!(beta.in_port(0x00ff), Some(0x40), "DRQ during Type II");
        assert_eq!(beta.cmd_count, 1);
        beta.out_port(0x00ff, 0x00);
        assert_eq!(beta.in_port(0x00ff), Some(0x00), "reset clears DRQ/INTRQ");
        assert_eq!(
            beta.in_port(0x00ff).unwrap() & 0x40,
            0,
            "DRQ line clear after /RES"
        );
        beta.out_port(0x001f, 0x80);
        assert_eq!(
            beta.in_port(0x00ff),
            Some(0x00),
            "commands ignored while /RES low"
        );
        assert_eq!(beta.sector_read_count, 1, "no second read while /RES held");
        beta.out_port(0x00ff, SYS_DEFAULT);
        beta.out_port(0x001f, 0x80);
        assert_eq!(beta.in_port(0x007f), Some(0xaa));
        assert_eq!(beta.sector_read_count, 2);
    }

    #[test]
    fn read_synthetic_boot_basic_from_track_1() {
        let mut beta = BetaDisk::new();
        beta.insert(TrdImage::synthetic_trdos_boot_basic());
        beta.page_trdos(true);
        beta.out_port(0x003f, 1);
        beta.out_port(0x005f, 1);
        beta.out_port(0x001f, 0x80);
        assert_eq!(beta.in_port(0x007f), Some(0x00));
        assert_eq!(beta.in_port(0x007f), Some(0x0a));
        assert_eq!(beta.in_port(0x007f), Some(0x17));
        assert_eq!(beta.in_port(0x007f), Some(0x00));
        assert_eq!(beta.in_port(0x007f), Some(0xf4)); // POKE
    }

    fn feed_write_track_sector(beta: &mut BetaDisk, track: u8, sector: u8, fill: u8) {
        beta.out_port(0x007f, 0xfe);
        beta.out_port(0x007f, track);
        beta.out_port(0x007f, 0);
        beta.out_port(0x007f, sector);
        beta.out_port(0x007f, 0x01);
        beta.out_port(0x007f, 0xf7);
        beta.out_port(0x007f, 0xfb);
        for _ in 0..TRD_SECTOR_SIZE {
            beta.out_port(0x007f, fill);
        }
        beta.out_port(0x007f, 0xf7);
    }

    #[test]
    fn write_track_formats_sector_with_data() {
        let mut beta = BetaDisk::new();
        beta.insert(one_track_image(0, 0));
        beta.page_trdos(true);
        beta.out_port(0x003f, 0);
        beta.out_port(0x001f, 0xf0);
        assert_eq!(beta.in_port(0x00ff), Some(0x40));
        feed_write_track_sector(&mut beta, 0, 2, 0xcd);
        beta.out_port(0x001f, 0xd8);
        assert_eq!(beta.in_port(0x00ff), Some(0x80));
        assert_eq!(beta.write_track_count, 1);
        let sec = beta.image.as_ref().unwrap().read_sector(0, 1).unwrap();
        assert_eq!(sec[0], 0xcd);
        assert_eq!(sec[255], 0xcd);
    }

    #[test]
    fn write_track_auto_completes_full_track() {
        let mut beta = BetaDisk::new();
        beta.insert(one_track_image(0xaa, 0xbb));
        beta.page_trdos(true);
        beta.out_port(0x003f, 0);
        beta.out_port(0x001f, 0xf0);
        for sec in 1..=TRD_SECTORS_PER_TRACK as u8 {
            feed_write_track_sector(&mut beta, 0, sec, sec);
        }
        assert_eq!(beta.in_port(0x00ff), Some(0x80));
        assert_eq!(beta.write_track_count, 1);
        let img = beta.image.as_ref().unwrap();
        for sec in 0..TRD_SECTORS_PER_TRACK {
            assert_eq!(img.read_sector(0, sec).unwrap()[0], (sec + 1) as u8);
        }
    }

    #[test]
    fn write_track_rnf_without_image() {
        let mut beta = BetaDisk::new();
        beta.page_trdos(true);
        beta.out_port(0x001f, 0xf0);
        assert_eq!(beta.status, STAT_NOT_READY);
    }

    #[test]
    fn ss_disk_read_ignores_side_one_latch() {
        let mut beta = BetaDisk::new();
        beta.insert(TrdImage::synthetic_trdos_boot_basic());
        beta.page_trdos(true);
        beta.out_port(0x00ff, 0x60);
        assert_eq!(beta.side(), 1);
        beta.out_port(0x003f, 1);
        beta.out_port(0x005f, 1);
        beta.out_port(0x001f, 0x80);
        assert!(beta.sector_read_count > 0);
        assert_eq!(beta.in_port(0x007f), Some(0x00));
    }

    #[test]
    fn trdos_read_track_f9_completes_on_status_poll_without_drain() {
        let mut beta = BetaDisk::new();
        beta.insert(TrdImage::synthetic_trdos_boot_basic());
        beta.page_trdos(true);
        beta.out_port(0x003f, 1);
        beta.out_port(0x001f, 0xf9);
        assert_eq!(beta.in_port(0x00ff), Some(0x80));
        assert_eq!(beta.in_port(0x001f).unwrap() & STAT_DRQ, STAT_DRQ);
        assert_eq!(beta.sector_read_count, 0);
    }

    #[test]
    fn beta_fdc_emits_disk_trace() {
        trace::clear();
        trace::enable(trace::Category::DISK);
        let mut beta = BetaDisk::new();
        beta.page_trdos(true);
        beta.out_port(0x003f, 5);
        beta.in_port(0x003f);
        trace::disable();
        let events = trace::snapshot();
        assert!(
            events.iter().any(|e| matches!(
                e.kind,
                trace::EventKind::DiskFdc {
                    port: 0x003f,
                    write: true,
                    value: 5
                }
            )),
            "expected OUT track trace: {events:?}"
        );
        assert!(
            events.iter().any(|e| matches!(
                e.kind,
                trace::EventKind::DiskFdc {
                    port: 0x003f,
                    write: false,
                    value: 5
                }
            )),
            "expected IN track trace: {events:?}"
        );
    }
}
