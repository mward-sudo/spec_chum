//! Result-phase helpers (invalid command + 7-byte ST0/ST1/ST2/C/H/R/N).

use super::{Phase, Plus3Fdc, ST0_IC_INVALID};

impl Plus3Fdc {
    pub(super) fn enter_invalid(&mut self) {
        self.result = vec![ST0_IC_INVALID];
        self.result_index = 0;
        self.phase = Phase::Result;
        self.cmd.clear();
    }

    pub(super) fn set_result_7(&mut self, st0: u8, st1: u8, st2: u8, c: u8, h: u8, r: u8, n: u8) {
        self.result = vec![st0, st1, st2, c, h, r, n];
        self.result_index = 0;
    }
}
