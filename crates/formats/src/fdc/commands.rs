//! Command-phase decode and non-transfer opcode handlers.

use super::{Phase, Plus3Fdc, ST0_IC_ABNORMAL, ST0_SE, ST1_ND, ST3_RY, ST3_T0, ST3_WP};

impl Plus3Fdc {
    pub(super) fn execute_command(&mut self) {
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
}

/// Bytes expected for a command opcode (including the opcode byte), or `None`
/// when the opcode is unsupported / invalid.
pub(super) fn command_len(opcode: u8) -> Option<usize> {
    match opcode & 0x1f {
        0x03 => Some(3),
        0x04 => Some(2),
        0x05 | 0x06 | 0x09 | 0x0c => Some(9),
        0x07 => Some(2),
        0x08 => Some(1),
        0x0a => Some(2),
        0x0d => Some(6),
        0x0f => Some(3),
        _ => None,
    }
}
