//! +3 µPD765 (NEC 765A) floppy controller — command / execution / result.
//!
//! Port `2FFD` is the main status register (MSR); `3FFD` is the data register.
//! Non-DMA programmed I/O only (the +3 has no Terminal Count pulse).
//!
//! Supported commands: SPECIFY, SENSE DRIVE STATUS, SENSE INTERRUPT STATUS,
//! RECALIBRATE, SEEK, READ ID, READ DATA / READ DELETED DATA, WRITE DATA /
//! WRITE DELETED DATA, FORMAT TRACK. Unknown opcodes return ST0=`0x80`.
//!
//! **Unsupported:** SCAN EQUAL/LOW/HIGH, READ TRACK. Copy-protected
//! or non-standard DSK geometry is not modelled.

use crate::dsk::DskImage;

/// MSR: Request for Master.
const MSR_RQM: u8 = 0x80;
/// MSR: Data direction (1 = FDC → CPU).
const MSR_DIO: u8 = 0x40;
/// MSR: Execution mode (non-DMA).
const MSR_EXM: u8 = 0x20;
/// MSR: Command busy.
const MSR_CB: u8 = 0x10;

/// ST0: Seek End.
const ST0_SE: u8 = 0x20;
/// ST0: Interrupt Code = abnormal termination.
const ST0_IC_ABNORMAL: u8 = 0x40;
/// ST0: Interrupt Code = invalid command (also ready-change uses IC=11 → `0xC0`).
const ST0_IC_INVALID: u8 = 0x80;
/// ST0: Ready change after reset (IC=11).
const ST0_READY_CHANGE: u8 = 0xC0;

/// ST1: No Data.
const ST1_ND: u8 = 0x04;
/// ST1: Not Writable.
const ST1_NW: u8 = 0x02;
/// ST1: End of Cylinder.
const ST1_EN: u8 = 0x80;

/// ST2: Wrong Cylinder.
const ST2_WC: u8 = 0x10;

/// ST3: Track 0.
const ST3_T0: u8 = 0x10;
/// ST3: Ready.
const ST3_RY: u8 = 0x20;
/// ST3: Write Protect.
const ST3_WP: u8 = 0x40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RwParams {
    us: u8,
    head: u8,
    c: u8,
    h: u8,
    r: u8,
    n: u8,
    eot: u8,
    dtl: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Idle,
    Command,
    ExecRead,
    ExecWrite,
    ExecFormat,
    Result,
}

/// +3 µPD765 controller with in-memory DSK backing.
#[derive(Clone, Debug)]
pub struct Plus3Fdc {
    pub image: Option<DskImage>,
    pub track: u8,
    pub side: u8,
    pub sector: u8,
    /// Last `read_sector` miss marker (`0x10`) or ST1-ish status for tests.
    pub status: u8,
    pub write_protect: bool,
    /// Completed SEEK / RECALIBRATE commands (Loader smoke).
    pub seek_count: u32,
    /// Completed READ DATA / READ DELETED DATA / READ ID.
    pub read_count: u32,
    /// Completed WRITE DATA / WRITE DELETED DATA.
    pub write_count: u32,
    /// Completed FORMAT TRACK commands.
    pub format_count: u32,
    motor_on: bool,
    pcn: [u8; 4],
    phase: Phase,
    cmd: Vec<u8>,
    cmd_len: usize,
    data_buf: Vec<u8>,
    data_index: usize,
    result: Vec<u8>,
    result_index: usize,
    /// Pending SENSE INTERRUPT STATUS payload `(ST0, PCN)`.
    interrupt: Option<(u8, u8)>,
    write_cyl: u8,
    write_head: u8,
    write_id: u8,
    format_us: u8,
    format_head: u8,
    format_n: u8,
    format_sc: u8,
    format_fill: u8,
    format_ids: Vec<u8>,
}

impl Default for Plus3Fdc {
    fn default() -> Self {
        Self::new()
    }
}

impl Plus3Fdc {
    #[must_use]
    pub fn new() -> Self {
        Self {
            image: None,
            track: 0,
            side: 0,
            sector: 0,
            status: 0,
            write_protect: false,
            seek_count: 0,
            read_count: 0,
            write_count: 0,
            format_count: 0,
            motor_on: false,
            pcn: [0; 4],
            phase: Phase::Idle,
            cmd: Vec::with_capacity(9),
            cmd_len: 0,
            data_buf: Vec::new(),
            data_index: 0,
            result: Vec::new(),
            result_index: 0,
            // Reset interrupt so the first SIS (unit 0) is not invalid.
            interrupt: Some((ST0_READY_CHANGE, 0)),
            write_cyl: 0,
            write_head: 0,
            write_id: 0,
            format_us: 0,
            format_head: 0,
            format_n: 0,
            format_sc: 0,
            format_fill: 0,
            format_ids: Vec::new(),
        }
    }

    pub fn insert(&mut self, image: DskImage) {
        self.image = Some(image);
        self.cmd.clear();
        self.data_buf.clear();
        self.data_index = 0;
        self.result.clear();
        self.result_index = 0;
        self.phase = Phase::Idle;
        self.status = 0;
    }

    /// Clear command state; keep the inserted image. Motor follows the caller.
    pub fn reset_controller(&mut self) {
        let image = self.image.take();
        let wp = self.write_protect;
        *self = Self::new();
        self.image = image;
        self.write_protect = wp;
    }

    pub fn set_motor(&mut self, on: bool) {
        if self.motor_on == on {
            return;
        }
        self.motor_on = on;
        if self.interrupt.is_none() {
            let st0 = if self.drive_ready(0) {
                ST0_READY_CHANGE
            } else {
                ST0_READY_CHANGE | 0x08
            };
            self.interrupt = Some((st0, self.pcn[0]));
        }
    }

    #[must_use]
    pub fn motor_on(&self) -> bool {
        self.motor_on
    }

    pub fn set_write_protect(&mut self, protect: bool) {
        self.write_protect = protect;
    }

    #[must_use]
    pub fn pcn(&self, unit: u8) -> u8 {
        self.pcn[usize::from(unit.min(3))]
    }

    #[must_use]
    pub fn last_result(&self) -> &[u8] {
        &self.result
    }

    fn drive_ready(&self, unit: u8) -> bool {
        unit == 0 && self.motor_on && self.image.is_some()
    }

    fn unit_index(us: u8) -> usize {
        usize::from(us & 3)
    }

    /// Seek and prepare sector buffer; returns true if found.
    pub fn read_sector(&mut self, track: u8, side: u8, sector: u8) -> bool {
        self.track = track;
        self.side = side;
        self.sector = sector;
        self.pcn[0] = track;
        self.data_index = 0;
        if let Some(img) = self.image.as_ref() {
            if let Some(sec) = img
                .find_sector(track, side, sector)
                .or_else(|| img.find_id(track, side, sector))
            {
                self.data_buf = sec.data.clone();
                self.status = 0;
                self.set_result_7(
                    ST0_IC_ABNORMAL,
                    ST1_EN,
                    0,
                    track,
                    side,
                    sector,
                    sec.size_code,
                );
                self.phase = Phase::ExecRead;
                return true;
            }
        }
        self.data_buf.clear();
        self.status = 0x10;
        self.phase = Phase::Idle;
        false
    }

    /// Write a command, parameter, or execution-phase data byte (port `3FFD`).
    pub fn write_command_byte(&mut self, value: u8) {
        match self.phase {
            Phase::Idle => {
                self.cmd.clear();
                self.cmd.push(value);
                match command_len(value) {
                    None => self.enter_invalid(),
                    Some(1) => self.execute_command(),
                    Some(n) => {
                        self.cmd_len = n;
                        self.phase = Phase::Command;
                    }
                }
            }
            Phase::Command => {
                self.cmd.push(value);
                if self.cmd.len() >= self.cmd_len {
                    self.execute_command();
                }
            }
            Phase::ExecWrite => {
                if self.data_index < self.data_buf.len() {
                    self.data_buf[self.data_index] = value;
                    self.data_index += 1;
                }
                if self.data_index >= self.data_buf.len() {
                    self.commit_write();
                    self.phase = Phase::Result;
                    self.result_index = 0;
                }
            }
            Phase::ExecFormat => {
                self.format_ids.push(value);
                let need = usize::from(self.format_sc) * 4;
                if self.format_ids.len() >= need {
                    self.commit_format();
                }
            }
            Phase::ExecRead | Phase::Result => {}
        }
    }

    /// Main status register (`2FFD`).
    #[must_use]
    pub fn main_status(&self) -> u8 {
        match self.phase {
            Phase::Idle => MSR_RQM,
            Phase::Command => MSR_RQM | MSR_CB,
            Phase::ExecRead => MSR_RQM | MSR_DIO | MSR_EXM | MSR_CB,
            Phase::ExecWrite | Phase::ExecFormat => MSR_RQM | MSR_EXM | MSR_CB,
            Phase::Result => MSR_RQM | MSR_DIO | MSR_CB,
        }
    }

    #[must_use]
    pub fn data_remaining(&self) -> usize {
        if matches!(self.phase, Phase::ExecRead | Phase::ExecWrite) {
            self.data_buf.len().saturating_sub(self.data_index)
        } else {
            0
        }
    }

    pub fn read_data_byte(&mut self) -> u8 {
        match self.phase {
            Phase::ExecRead => {
                let b = self.data_buf.get(self.data_index).copied().unwrap_or(0xff);
                if self.data_index < self.data_buf.len() {
                    self.data_index += 1;
                }
                if self.data_index >= self.data_buf.len() {
                    self.phase = Phase::Result;
                    self.result_index = 0;
                }
                b
            }
            Phase::Result => {
                let b = self.result.get(self.result_index).copied().unwrap_or(0xff);
                if self.result_index < self.result.len() {
                    self.result_index += 1;
                }
                if self.result_index >= self.result.len() {
                    self.phase = Phase::Idle;
                }
                b
            }
            _ => 0xff,
        }
    }

    fn enter_invalid(&mut self) {
        self.result = vec![ST0_IC_INVALID];
        self.result_index = 0;
        self.phase = Phase::Result;
        self.cmd.clear();
    }

    fn set_result_7(&mut self, st0: u8, st1: u8, st2: u8, c: u8, h: u8, r: u8, n: u8) {
        self.result = vec![st0, st1, st2, c, h, r, n];
        self.result_index = 0;
    }

    fn execute_command(&mut self) {
        let op = self.cmd.first().copied().unwrap_or(0) & 0x1f;
        match op {
            0x03 => self.cmd_specify(),
            0x04 => self.cmd_sense_drive(),
            0x05 | 0x09 => self.cmd_write_data(),
            0x06 | 0x0c => self.cmd_read_data(),
            0x07 => self.cmd_recalibrate(),
            0x08 => self.cmd_sense_interrupt(),
            0x0a => self.cmd_read_id(),
            0x0d => self.cmd_format_track(),
            0x0f => self.cmd_seek(),
            _ => self.enter_invalid(),
        }
        self.cmd.clear();
    }

    fn cmd_specify(&mut self) {
        self.phase = Phase::Idle;
    }

    fn cmd_sense_drive(&mut self) {
        let us_hd = self.cmd.get(1).copied().unwrap_or(0);
        self.result = vec![self.st3(us_hd)];
        self.result_index = 0;
        self.phase = Phase::Result;
    }

    fn st3(&self, us_hd: u8) -> u8 {
        let us = us_hd & 3;
        let hd = us_hd & 4;
        let mut v = us | hd;
        if self.pcn[Self::unit_index(us)] == 0 {
            v |= ST3_T0;
        }
        if self.drive_ready(us) {
            v |= ST3_RY;
        }
        if self.write_protect {
            v |= ST3_WP;
        }
        v
    }

    fn cmd_sense_interrupt(&mut self) {
        if let Some((st0, pcn)) = self.interrupt.take() {
            self.result = vec![st0, pcn];
            self.result_index = 0;
            self.phase = Phase::Result;
        } else {
            self.enter_invalid();
        }
    }

    fn cmd_recalibrate(&mut self) {
        let us = self.cmd.get(1).copied().unwrap_or(0) & 3;
        let idx = Self::unit_index(us);
        self.pcn[idx] = 0;
        self.track = 0;
        self.seek_count = self.seek_count.saturating_add(1);
        self.interrupt = Some((ST0_SE | us, 0));
        self.phase = Phase::Idle;
    }

    fn cmd_seek(&mut self) {
        let us = self.cmd.get(1).copied().unwrap_or(0) & 3;
        let ncn = self.cmd.get(2).copied().unwrap_or(0);
        let idx = Self::unit_index(us);
        self.pcn[idx] = ncn;
        self.track = ncn;
        self.seek_count = self.seek_count.saturating_add(1);
        self.interrupt = Some((ST0_SE | us, ncn));
        self.phase = Phase::Idle;
    }

    fn cmd_read_id(&mut self) {
        let us_hd = self.cmd.get(1).copied().unwrap_or(0);
        let us = us_hd & 3;
        let head = (us_hd >> 2) & 1;
        let cyl = self.pcn[Self::unit_index(us)];
        self.side = head;
        self.read_count = self.read_count.saturating_add(1);
        if let Some(sec) = self
            .image
            .as_ref()
            .and_then(|img| img.first_sector(cyl, head))
        {
            self.set_result_7(
                us | (head << 2),
                0,
                0,
                sec.track,
                sec.side,
                sec.sector_id,
                sec.size_code,
            );
        } else {
            self.set_result_7(
                ST0_IC_ABNORMAL | us | (head << 2),
                ST1_ND,
                0,
                cyl,
                head,
                0,
                0,
            );
        }
        self.phase = Phase::Result;
    }

    fn cmd_read_data(&mut self) {
        let Some(p) = self.chrn_params() else {
            self.enter_invalid();
            return;
        };
        let phys = self.pcn[Self::unit_index(p.us)];
        self.track = phys;
        self.side = p.h;
        self.sector = p.r;
        self.read_count = self.read_count.saturating_add(1);
        let len = transfer_len(p.n, p.dtl);
        let found = self
            .image
            .as_ref()
            .and_then(|img| {
                img.find_id(phys, p.head, p.r)
                    .or_else(|| img.find_id(phys, p.h, p.r))
            })
            .map(|sec| sec.data.clone());
        match found {
            Some(data) => {
                let mut buf = vec![0u8; len];
                let copy = len.min(data.len());
                buf[..copy].copy_from_slice(&data[..copy]);
                self.data_buf = buf;
                self.data_index = 0;
                let mut st0 = p.us | (p.head << 2);
                let mut st1 = 0;
                if p.r == p.eot {
                    st0 |= ST0_IC_ABNORMAL;
                    st1 |= ST1_EN;
                }
                let st2 = if p.c != phys { ST2_WC } else { 0 };
                self.set_result_7(st0, st1, st2, p.c, p.h, p.r, p.n);
                self.status = 0;
                self.phase = Phase::ExecRead;
            }
            None => {
                self.status = 0x10;
                self.data_buf.clear();
                self.set_result_7(
                    ST0_IC_ABNORMAL | p.us | (p.head << 2),
                    ST1_ND,
                    0,
                    p.c,
                    p.h,
                    p.r,
                    p.n,
                );
                self.phase = Phase::Result;
            }
        }
    }

    fn cmd_write_data(&mut self) {
        let Some(p) = self.chrn_params() else {
            self.enter_invalid();
            return;
        };
        let phys = self.pcn[Self::unit_index(p.us)];
        self.track = phys;
        self.side = p.h;
        self.sector = p.r;
        self.write_count = self.write_count.saturating_add(1);
        if self.write_protect {
            self.set_result_7(
                ST0_IC_ABNORMAL | p.us | (p.head << 2),
                ST1_NW,
                0,
                p.c,
                p.h,
                p.r,
                p.n,
            );
            self.phase = Phase::Result;
            return;
        }
        let exists = self
            .image
            .as_ref()
            .and_then(|img| {
                img.find_id(phys, p.head, p.r)
                    .or_else(|| img.find_id(phys, p.h, p.r))
            })
            .is_some();
        if !exists {
            self.status = 0x10;
            self.set_result_7(
                ST0_IC_ABNORMAL | p.us | (p.head << 2),
                ST1_ND,
                0,
                p.c,
                p.h,
                p.r,
                p.n,
            );
            self.phase = Phase::Result;
            return;
        }
        let len = transfer_len(p.n, p.dtl);
        self.data_buf = vec![0u8; len];
        self.data_index = 0;
        self.write_cyl = phys;
        self.write_head = p.head;
        self.write_id = p.r;
        let mut st0 = p.us | (p.head << 2);
        let mut st1 = 0;
        if p.r == p.eot {
            st0 |= ST0_IC_ABNORMAL;
            st1 |= ST1_EN;
        }
        let st2 = if p.c != phys { ST2_WC } else { 0 };
        self.set_result_7(st0, st1, st2, p.c, p.h, p.r, p.n);
        self.status = 0;
        self.phase = Phase::ExecWrite;
    }

    fn commit_write(&mut self) {
        let cyl = self.write_cyl;
        let head = self.write_head;
        let id = self.write_id;
        if let Some(sec) = self
            .image
            .as_mut()
            .and_then(|img| img.find_id_mut(cyl, head, id))
        {
            let n = self.data_buf.len().min(sec.data.len());
            sec.data[..n].copy_from_slice(&self.data_buf[..n]);
        }
    }

    fn cmd_format_track(&mut self) {
        let us_hd = self.cmd.get(1).copied().unwrap_or(0);
        let us = us_hd & 3;
        let head = (us_hd >> 2) & 1;
        let n = self.cmd.get(2).copied().unwrap_or(0);
        let sc = self.cmd.get(3).copied().unwrap_or(0);
        let fill = self.cmd.get(5).copied().unwrap_or(0xe5);
        let cyl = self.pcn[Self::unit_index(us)];
        self.format_us = us;
        self.format_head = head;
        self.format_n = n;
        self.format_sc = sc;
        self.format_fill = fill;
        self.format_ids.clear();
        if self.write_protect {
            self.set_result_7(
                ST0_IC_ABNORMAL | us | (head << 2),
                ST1_NW,
                0,
                cyl,
                head,
                0,
                n,
            );
            self.phase = Phase::Result;
            return;
        }
        if sc == 0 {
            self.set_result_7(us | (head << 2), 0, 0, cyl, head, 0, n);
            self.format_count = self.format_count.saturating_add(1);
            self.phase = Phase::Result;
            return;
        }
        self.phase = Phase::ExecFormat;
    }

    fn commit_format(&mut self) {
        let us = self.format_us;
        let head = self.format_head;
        let n = self.format_n;
        let fill = self.format_fill;
        let cyl = self.pcn[Self::unit_index(us)];
        let mut entries = Vec::new();
        for chunk in self.format_ids.as_chunks::<4>().0 {
            entries.push((chunk[0], chunk[1], chunk[2], chunk[3]));
        }
        let formatted = self
            .image
            .as_mut()
            .is_some_and(|img| img.format_track(cyl, head, fill, &entries));
        if formatted {
            self.format_count = self.format_count.saturating_add(1);
            let (c, h, r, nn) = entries.last().copied().unwrap_or((cyl, head, 0, n));
            self.set_result_7(us | (head << 2), 0, 0, c, h, r, nn);
        } else {
            self.set_result_7(
                ST0_IC_ABNORMAL | us | (head << 2),
                ST1_ND,
                0,
                cyl,
                head,
                0,
                n,
            );
        }
        self.phase = Phase::Result;
        self.format_ids.clear();
    }

    fn chrn_params(&self) -> Option<RwParams> {
        if self.cmd.len() < 9 {
            return None;
        }
        let us_hd = self.cmd[1];
        Some(RwParams {
            us: us_hd & 3,
            head: (us_hd >> 2) & 1,
            c: self.cmd[2],
            h: self.cmd[3],
            r: self.cmd[4],
            n: self.cmd[5],
            eot: self.cmd[6],
            dtl: self.cmd[8],
        })
    }
}

fn command_len(opcode: u8) -> Option<usize> {
    match opcode & 0x1f {
        0x03 => Some(3),
        0x04 => Some(2),
        0x05 | 0x09 => Some(9),
        0x06 | 0x0c => Some(9),
        0x07 => Some(2),
        0x08 => Some(1),
        0x0a => Some(2),
        0x0d => Some(6),
        0x0f => Some(3),
        _ => None,
    }
}

fn transfer_len(n: u8, dtl: u8) -> usize {
    if n == 0 {
        if dtl == 0xFF {
            128
        } else {
            usize::from(dtl.max(1))
        }
    } else {
        128usize << n.min(6)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loaded(img: DskImage) -> Plus3Fdc {
        let mut fdc = Plus3Fdc::new();
        fdc.insert(img);
        fdc
    }

    fn feed_read_data(fdc: &mut Plus3Fdc, c: u8, h: u8, r: u8) {
        for b in [0x46, 0, c, h, r, 1, 0x09, 0x2a, 0xff] {
            fdc.write_command_byte(b);
        }
    }

    fn drain_result(fdc: &mut Plus3Fdc) -> Vec<u8> {
        assert_eq!(fdc.main_status(), MSR_RQM | MSR_DIO | MSR_CB);
        let mut out = Vec::new();
        while fdc.main_status() & MSR_CB != 0 {
            out.push(fdc.read_data_byte());
            if out.len() > 16 {
                break;
            }
        }
        out
    }

    #[test]
    fn parse_and_read_sector() {
        let mut fdc = loaded(DskImage::synthetic_one_sector());
        assert!(fdc.read_sector(0, 0, 0xc1));
        assert_eq!(fdc.read_data_byte(), 0x42);
        assert_eq!(fdc.read_data_byte(), 0x43);
    }

    #[test]
    fn read_data_command_stream_loads_sector() {
        let mut fdc = loaded(DskImage::synthetic_one_sector());
        feed_read_data(&mut fdc, 0, 0, 0xc1);
        assert!(fdc.data_remaining() > 0);
        assert_eq!(fdc.main_status() & 0xc0, 0xc0);
        assert_eq!(fdc.read_data_byte(), 0x42);
        assert_eq!(fdc.read_data_byte(), 0x43);
    }

    #[test]
    fn multi_sector_dsk_read_two_sectors() {
        let mut fdc = loaded(DskImage::synthetic_two_sectors());
        assert!(fdc.read_sector(0, 0, 0xc1));
        assert_eq!(fdc.read_data_byte(), 0xa1);
        assert_eq!(fdc.read_data_byte(), 0xa2);
        assert!(fdc.read_sector(0, 0, 0xc2));
        assert_eq!(fdc.read_data_byte(), 0xb1);
        assert_eq!(fdc.read_data_byte(), 0xb2);
        assert!(!fdc.read_sector(0, 0, 0xc3), "missing sector id");
        assert_eq!(fdc.status, 0x10);
    }

    #[test]
    fn read_data_command_bytes_load_sector() {
        let mut fdc = loaded(DskImage::synthetic_one_sector());
        for b in [0x06u8, 0, 0, 0, 0xc1, 1, 0x09, 0x2a, 0xff] {
            fdc.write_command_byte(b);
        }
        assert_eq!(fdc.main_status() & 0xc0, 0xc0);
        assert_eq!(fdc.read_data_byte(), 0x42);
        assert_eq!(fdc.read_data_byte(), 0x43);
    }

    #[test]
    fn read_data_nine_byte_phase_selects_correct_sector() {
        let mut fdc = loaded(DskImage::synthetic_two_sectors());
        feed_read_data(&mut fdc, 0, 0, 0xc2);
        assert_eq!(fdc.read_data_byte(), 0xb1);
        assert_eq!(fdc.read_data_byte(), 0xb2);
    }

    #[test]
    fn specify_returns_to_idle_without_result() {
        let mut fdc = Plus3Fdc::new();
        assert_eq!(fdc.main_status(), MSR_RQM);
        fdc.write_command_byte(0x03);
        assert_eq!(fdc.main_status(), MSR_RQM | MSR_CB);
        fdc.write_command_byte(0xaf);
        fdc.write_command_byte(0x03);
        assert_eq!(fdc.main_status(), MSR_RQM);
    }

    fn sis(fdc: &mut Plus3Fdc) -> Vec<u8> {
        fdc.write_command_byte(0x08);
        drain_result(fdc)
    }

    #[test]
    fn seek_then_sis_returns_seek_end_and_pcn() {
        let mut fdc = Plus3Fdc::new();
        fdc.write_command_byte(0x0f);
        fdc.write_command_byte(0x00);
        fdc.write_command_byte(0x05);
        assert_eq!(fdc.main_status(), MSR_RQM);
        assert_eq!(fdc.pcn(0), 5);
        assert_eq!(sis(&mut fdc), vec![ST0_SE, 0x05]);
    }

    #[test]
    fn recalibrate_sets_pcn_zero() {
        let mut fdc = Plus3Fdc::new();
        fdc.write_command_byte(0x0f);
        fdc.write_command_byte(0x00);
        fdc.write_command_byte(0x0c);
        let _ = sis(&mut fdc);
        fdc.write_command_byte(0x07);
        fdc.write_command_byte(0x00);
        assert_eq!(fdc.pcn(0), 0);
        assert_eq!(sis(&mut fdc), vec![ST0_SE, 0x00]);
    }

    #[test]
    fn sense_drive_ready_depends_on_motor_and_disk() {
        let mut fdc = Plus3Fdc::new();
        fdc.write_command_byte(0x04);
        fdc.write_command_byte(0x00);
        let st3 = drain_result(&mut fdc)[0];
        assert_eq!(st3 & ST3_RY, 0, "not ready: no disk, motor off");
        assert_eq!(st3 & ST3_T0, ST3_T0);

        fdc.insert(DskImage::synthetic_plus3_data());
        fdc.write_command_byte(0x04);
        fdc.write_command_byte(0x00);
        let st3 = drain_result(&mut fdc)[0];
        assert_eq!(st3 & ST3_RY, 0, "not ready: motor still off");

        fdc.set_motor(true);
        fdc.write_command_byte(0x04);
        fdc.write_command_byte(0x00);
        let st3 = drain_result(&mut fdc)[0];
        assert_eq!(st3 & ST3_RY, ST3_RY);

        fdc.set_write_protect(true);
        fdc.write_command_byte(0x04);
        fdc.write_command_byte(0x00);
        let st3 = drain_result(&mut fdc)[0];
        assert_eq!(st3 & ST3_WP, ST3_WP);
    }

    #[test]
    fn read_data_result_phase_en_when_r_equals_eot() {
        let mut fdc = loaded(DskImage::synthetic_one_sector());
        for b in [0x46u8, 0, 0, 0, 0xc1, 1, 0xc1, 0x2a, 0xff] {
            fdc.write_command_byte(b);
        }
        assert_eq!(fdc.main_status(), MSR_RQM | MSR_DIO | MSR_EXM | MSR_CB);
        for _ in 0..256 {
            let _ = fdc.read_data_byte();
        }
        let res = drain_result(&mut fdc);
        assert_eq!(res.len(), 7);
        assert_eq!(res[0], ST0_IC_ABNORMAL);
        assert_eq!(res[1], ST1_EN);
        assert_eq!(res[2], 0);
        assert_eq!(&res[3..7], &[0, 0, 0xc1, 1]);
        assert_eq!(fdc.main_status(), MSR_RQM);
    }

    #[test]
    fn read_id_returns_chrn_of_first_sector() {
        let mut fdc = Plus3Fdc::new();
        fdc.insert(DskImage::synthetic_plus3_data());
        fdc.write_command_byte(0x0a);
        fdc.write_command_byte(0x00);
        let res = drain_result(&mut fdc);
        assert_eq!(res.len(), 7);
        assert_eq!(res[0] & 0xc0, 0, "normal termination");
        assert_eq!(&res[3..7], &[0, 0, 1, 2]); // C H R N of first DATA sector
    }

    #[test]
    fn write_data_round_trip() {
        let mut fdc = loaded(DskImage::synthetic_one_sector());
        for b in [0x05u8, 0, 0, 0, 0xc1, 1, 0xc1, 0x2a, 0xff] {
            fdc.write_command_byte(b);
        }
        assert_eq!(fdc.main_status(), MSR_RQM | MSR_EXM | MSR_CB);
        fdc.write_command_byte(0xaa);
        fdc.write_command_byte(0xbb);
        for _ in 2..256 {
            fdc.write_command_byte(0x00);
        }
        let res = drain_result(&mut fdc);
        assert_eq!(res[1], ST1_EN);

        for b in [0x06u8, 0, 0, 0, 0xc1, 1, 0xc1, 0x2a, 0xff] {
            fdc.write_command_byte(b);
        }
        assert_eq!(fdc.read_data_byte(), 0xaa);
        assert_eq!(fdc.read_data_byte(), 0xbb);
    }

    #[test]
    fn invalid_command_st0_0x80() {
        let mut fdc = Plus3Fdc::new();
        fdc.write_command_byte(0x11); // SCAN EQUAL — unsupported
        let res = drain_result(&mut fdc);
        assert_eq!(res, vec![ST0_IC_INVALID]);
        assert_eq!(fdc.main_status(), MSR_RQM);
    }

    #[test]
    fn sis_without_interrupt_is_invalid() {
        let mut fdc = Plus3Fdc::new();
        let _ = sis(&mut fdc);
        assert_eq!(sis(&mut fdc), vec![ST0_IC_INVALID]);
    }

    #[test]
    fn write_protect_skips_execution() {
        let mut fdc = loaded(DskImage::synthetic_one_sector());
        fdc.set_write_protect(true);
        for b in [0x05u8, 0, 0, 0, 0xc1, 1, 0xc1, 0x2a, 0xff] {
            fdc.write_command_byte(b);
        }
        let res = drain_result(&mut fdc);
        assert_eq!(res[1] & ST1_NW, ST1_NW);
        assert_eq!(fdc.main_status(), MSR_RQM);
    }

    #[test]
    fn format_track_replaces_sectors_on_disk() {
        let mut fdc = loaded(DskImage::synthetic_one_sector());
        // FORMAT TRACK: opcode, HD/US, N, SC, GPL, fill
        for b in [0x0du8, 0x00, 0x01, 0x02, 0x2a, 0xe5] {
            fdc.write_command_byte(b);
        }
        assert_eq!(fdc.main_status(), MSR_RQM | MSR_EXM | MSR_CB);
        // Two sectors: C H R N each
        for b in [0x00, 0x00, 0xc1, 0x01, 0x00, 0x00, 0xc2, 0x01] {
            fdc.write_command_byte(b);
        }
        let res = drain_result(&mut fdc);
        assert_eq!(res.len(), 7);
        assert_eq!(res[0] & 0xc0, 0, "normal termination");
        assert_eq!(res[3..7], [0x00, 0x00, 0xc2, 0x01]);
        assert_eq!(fdc.format_count, 1);

        feed_read_data(&mut fdc, 0, 0, 0xc1);
        assert_eq!(fdc.read_data_byte(), 0xe5);
        assert_eq!(fdc.read_data_byte(), 0xe5);
        feed_read_data(&mut fdc, 0, 0, 0xc2);
        assert_eq!(fdc.read_data_byte(), 0xe5);
    }

    #[test]
    fn format_track_write_protect_returns_nw() {
        let mut fdc = loaded(DskImage::synthetic_one_sector());
        fdc.set_write_protect(true);
        for b in [0x0du8, 0x00, 0x01, 0x01, 0x2a, 0xe5] {
            fdc.write_command_byte(b);
        }
        let res = drain_result(&mut fdc);
        assert_eq!(res[1] & ST1_NW, ST1_NW);
        assert_eq!(fdc.main_status(), MSR_RQM);
    }

    fn feed_format_track(fdc: &mut Plus3Fdc, sc: u8, ids: &[(u8, u8, u8, u8)]) {
        for b in [0x0du8, 0x00, 0x01, sc, 0x2a, 0xe5] {
            fdc.write_command_byte(b);
        }
        for &(c, h, r, n) in ids {
            for b in [c, h, r, n] {
                fdc.write_command_byte(b);
            }
        }
    }

    #[test]
    fn format_track_no_image_returns_abnormal_nd() {
        let mut fdc = Plus3Fdc::new();
        feed_format_track(&mut fdc, 1, &[(0, 0, 0xc1, 1)]);
        let res = drain_result(&mut fdc);
        assert_eq!(res.len(), 7);
        assert_eq!(res[0] & ST0_IC_ABNORMAL, ST0_IC_ABNORMAL);
        assert_eq!(res[1] & ST1_ND, ST1_ND);
        assert_eq!(res[3..7], [0, 0, 0, 1]);
        assert_eq!(fdc.format_count, 0);
    }

    #[test]
    fn format_track_out_of_range_returns_abnormal_nd() {
        let mut fdc = loaded(DskImage::synthetic_one_sector());
        fdc.write_command_byte(0x0f);
        fdc.write_command_byte(0x00);
        fdc.write_command_byte(99);
        let _ = sis(&mut fdc);
        feed_format_track(&mut fdc, 1, &[(99, 0, 0xc1, 1)]);
        let res = drain_result(&mut fdc);
        assert_eq!(res.len(), 7);
        assert_eq!(res[0] & ST0_IC_ABNORMAL, ST0_IC_ABNORMAL);
        assert_eq!(res[1] & ST1_ND, ST1_ND);
        assert_eq!(res[3..7], [99, 0, 0, 1]);
        assert_eq!(fdc.format_count, 0);
    }
}
