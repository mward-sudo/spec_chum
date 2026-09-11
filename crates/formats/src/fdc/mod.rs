//! +3 `µPD765` (`NEC 765A`) floppy controller — command / execution / result.
//!
//! Port `2FFD` is the main status register (MSR); `3FFD` is the data register.
//! Non-DMA programmed I/O only (the +3 has no Terminal Count pulse).
//!
//! Supported commands: SPECIFY, SENSE DRIVE STATUS, SENSE INTERRUPT STATUS,
//! RECALIBRATE, SEEK, READ ID, READ DATA / READ DELETED DATA, WRITE DATA /
//! WRITE DELETED DATA, FORMAT TRACK. Unknown opcodes return ST0=`0x80`
//! (invalid-command interrupt code) as a single-byte result phase.
//!
//! **Unsupported (intentionally invalid):** SCAN EQUAL (`0x11`), SCAN LOW OR
//! EQUAL (`0x19`), SCAN HIGH OR EQUAL (`0x1d`), READ TRACK (`0x02`) — including
//! MT/SK opcode variants. No known +3DOS / Loader path needs SCAN; CP/M disk
//! tools that issue SCAN still get `ST0=0x80`. Copy-protected or non-standard
//! DSK geometry is not modelled.
//!
//! Layout (#345 cohesion split): [`commands`] (opcode dispatch), [`sector`]
//! (READ/WRITE/FORMAT + DSK I/O), [`result`] (result-phase helpers). Behaviour
//! is unchanged from the former single-file module.

mod commands;
mod result;
mod sector;

#[cfg(test)]
mod tests;

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
enum Phase {
    Idle,
    Command,
    ExecRead,
    ExecWrite,
    ExecFormat,
    Result,
}

/// +3 `µPD765` controller with in-memory `DSK` backing.
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

    /// Write a command, parameter, or execution-phase data byte (port `3FFD`).
    pub fn write_command_byte(&mut self, value: u8) {
        match self.phase {
            Phase::Idle => {
                self.cmd.clear();
                self.cmd.push(value);
                match commands::command_len(value) {
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
}
