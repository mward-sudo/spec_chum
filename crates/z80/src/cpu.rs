//! Z80 CPU core.

use crate::bus::{Io, Memory};
use crate::opcodes;
use crate::registers::Registers;

/// Z80 CPU with cycle-counted instruction execution.
#[derive(Clone, Debug, Default)]
pub struct Cpu {
    pub regs: Registers,
    /// Absolute T-state counter (host may wrap/reset per frame).
    pub t: u64,
    /// When true, maskable interrupts are not accepted (between EI and end of following insn).
    pub(crate) interrupt_deferred: bool,
}

impl Cpu {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        self.regs.reset();
        self.t = 0;
        self.interrupt_deferred = false;
    }

    /// Execute one instruction (or HALT idle of 4 T). Returns T-states consumed.
    pub fn step<M: Memory, I: Io>(&mut self, mem: &mut M, io: &mut I) -> u32 {
        // `interrupt_deferred` suppresses IRQ acceptance for one instruction after EI.
        // Cleared at the beginning of the instruction following EI.
        self.interrupt_deferred = false;

        if self.regs.halted {
            self.regs.inc_r();
            self.add_t(4);
            return 4;
        }

        opcodes::execute(self, mem, io)
    }

    /// Accept a maskable interrupt if enabled. Returns T-states of ACK sequence, or 0.
    pub fn interrupt<M: Memory>(&mut self, mem: &mut M) -> u32 {
        if self.interrupt_deferred || !self.regs.iff1 {
            return 0;
        }
        self.regs.halted = false;
        self.regs.iff1 = false;
        self.regs.iff2 = false;
        self.regs.inc_r();

        match self.regs.im {
            0 => {
                self.push(mem, self.regs.pc);
                self.regs.pc = 0x0038;
                self.regs.memptr = 0x0038;
                self.add_t(13);
                13
            }
            1 => {
                self.push(mem, self.regs.pc);
                self.regs.pc = 0x0038;
                self.regs.memptr = 0x0038;
                self.add_t(13);
                13
            }
            _ => {
                self.push(mem, self.regs.pc);
                let vec = (u16::from(self.regs.i) << 8) | 0x00ff;
                let lo = self.read_mem(mem, vec);
                let hi = self.read_mem(mem, vec.wrapping_add(1));
                let addr = u16::from(hi) << 8 | u16::from(lo);
                self.regs.pc = addr;
                self.regs.memptr = addr;
                self.add_t(19);
                19
            }
        }
    }

    #[inline]
    pub(crate) fn add_t(&mut self, dt: u32) {
        self.t = self.t.wrapping_add(u64::from(dt));
    }

    #[inline]
    pub(crate) fn read_mem<M: Memory>(&mut self, mem: &mut M, addr: u16) -> u8 {
        let (v, wait) = mem.read(addr, self.t);
        self.add_t(3 + wait);
        v
    }

    #[inline]
    pub(crate) fn write_mem<M: Memory>(&mut self, mem: &mut M, addr: u16, value: u8) {
        let wait = mem.write(addr, value, self.t);
        self.add_t(3 + wait);
    }

    #[inline]
    pub(crate) fn fetch_opcode<M: Memory>(&mut self, mem: &mut M) -> u8 {
        let pc = self.regs.pc;
        let (v, wait) = mem.read(pc, self.t);
        self.regs.pc = pc.wrapping_add(1);
        self.regs.inc_r();
        self.add_t(4 + wait);
        v
    }

    #[inline]
    pub(crate) fn fetch8<M: Memory>(&mut self, mem: &mut M) -> u8 {
        let pc = self.regs.pc;
        let v = self.read_mem(mem, pc);
        self.regs.pc = pc.wrapping_add(1);
        v
    }

    #[inline]
    pub(crate) fn fetch16<M: Memory>(&mut self, mem: &mut M) -> u16 {
        let lo = self.fetch8(mem);
        let hi = self.fetch8(mem);
        u16::from(hi) << 8 | u16::from(lo)
    }

    #[inline]
    pub(crate) fn push<M: Memory>(&mut self, mem: &mut M, value: u16) {
        self.regs.sp = self.regs.sp.wrapping_sub(1);
        self.write_mem(mem, self.regs.sp, (value >> 8) as u8);
        self.regs.sp = self.regs.sp.wrapping_sub(1);
        self.write_mem(mem, self.regs.sp, value as u8);
    }

    #[inline]
    pub(crate) fn pop<M: Memory>(&mut self, mem: &mut M) -> u16 {
        let lo = self.read_mem(mem, self.regs.sp);
        self.regs.sp = self.regs.sp.wrapping_add(1);
        let hi = self.read_mem(mem, self.regs.sp);
        self.regs.sp = self.regs.sp.wrapping_add(1);
        u16::from(hi) << 8 | u16::from(lo)
    }

    #[inline]
    pub(crate) fn in_port<I: Io>(&mut self, io: &mut I, port: u16) -> u8 {
        let (v, wait) = io.in_port(port, self.t);
        self.add_t(4 + wait);
        self.regs.memptr = port.wrapping_add(1);
        v
    }

    #[inline]
    pub(crate) fn out_port<I: Io>(&mut self, io: &mut I, port: u16, value: u8) {
        let wait = io.out_port(port, value, self.t);
        self.add_t(4 + wait);
        self.regs.memptr = (port & 0xff00) | (port.wrapping_add(1) & 0x00ff);
    }
}
