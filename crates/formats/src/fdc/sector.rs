//! Sector transfer and FORMAT TRACK — DSK-backed READ/WRITE/FORMAT paths.

use super::{Phase, Plus3Fdc, ST0_IC_ABNORMAL, ST1_EN, ST1_ND, ST1_NW, ST2_WC};

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

impl Plus3Fdc {
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

    pub(super) fn cmd_read_data(&mut self) {
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
        if let Some(data) = found {
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
            let st2 = if p.c == phys { 0 } else { ST2_WC };
            self.set_result_7(st0, st1, st2, p.c, p.h, p.r, p.n);
            self.status = 0;
            self.phase = Phase::ExecRead;
        } else {
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

    pub(super) fn cmd_write_data(&mut self) {
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
        let st2 = if p.c == phys { 0 } else { ST2_WC };
        self.set_result_7(st0, st1, st2, p.c, p.h, p.r, p.n);
        self.status = 0;
        self.phase = Phase::ExecWrite;
    }

    pub(super) fn commit_write(&mut self) {
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

    pub(super) fn cmd_format_track(&mut self) {
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

    pub(super) fn commit_format(&mut self) {
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
