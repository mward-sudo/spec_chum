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
    pub fn step<B: Memory + Io>(&mut self, bus: &mut B) -> u32 {
        // `interrupt_deferred` suppresses IRQ acceptance for one instruction after EI.
        // Cleared at the beginning of the instruction following EI.
        self.interrupt_deferred = false;

        if self.regs.halted {
            self.regs.inc_r();
            self.add_t(4);
            return 4;
        }

        opcodes::execute(self, bus)
    }

    /// Accept a maskable interrupt if enabled. Returns T-states of ACK sequence, or 0.
    ///
    /// The returned value includes contention waits from stack / IM2 vector accesses so
    /// hosts can advance ULA/`frame_t` in lockstep with `cpu.t`.
    pub fn interrupt<M: Memory>(&mut self, mem: &mut M) -> u32 {
        if self.interrupt_deferred || !self.regs.iff1 {
            return 0;
        }
        let t0 = self.t;
        // While halted, PC sits on the HALT opcode. Accepting INT advances PC onto the
        // following instruction before the return address is pushed (Undocumented Z80).
        // Only bump PC when it still addresses HALT — hosts may redirect PC while the
        // halted flag is still set (test USR entry, debugger poke, etc.).
        if self.regs.halted {
            self.regs.halted = false;
            let op = mem.read(self.regs.pc, self.t).0;
            if op == 0x76 {
                self.regs.pc = self.regs.pc.wrapping_add(1);
            }
        }
        self.regs.iff1 = false;
        self.regs.iff2 = false;
        self.regs.inc_r();

        // Nominal breakdown (uncontended): IM0/1 = 7T ack + 6T push; IM2 adds 6T vector.
        // `push` / `read_mem` already account for their memory cycles (+ contention).
        match self.regs.im {
            0 | 1 => {
                self.push(mem, self.regs.pc);
                self.regs.pc = 0x0038;
                self.regs.memptr = 0x0038;
                self.add_t(7);
            }
            _ => {
                self.push(mem, self.regs.pc);
                let vec = (u16::from(self.regs.i) << 8) | 0x00ff;
                let lo = self.read_mem(mem, vec);
                let hi = self.read_mem(mem, vec.wrapping_add(1));
                let addr = u16::from(hi) << 8 | u16::from(lo);
                self.regs.pc = addr;
                self.regs.memptr = addr;
                self.add_t(7);
            }
        }
        (self.t.wrapping_sub(t0)) as u32
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::FlatMem;

    #[test]
    fn interrupt_while_halted_resumes_after_halt() {
        let mut cpu = Cpu::new();
        let mut mem = FlatMem::new();
        mem.data[0x1000] = 0x76; // HALT
        mem.data[0x1001] = 0x00; // NOP after HALT
        cpu.regs.pc = 0x1000;
        cpu.regs.sp = 0xfffd;
        cpu.regs.iff1 = true;
        cpu.regs.im = 1;
        cpu.step(&mut mem); // enter HALT (PC rewound to 0x1000)
        assert!(cpu.regs.halted);
        assert_eq!(cpu.regs.pc, 0x1000);
        let t = cpu.interrupt(&mut mem);
        assert_eq!(t, 13, "uncontended IM1 IRQ is 13 T");
        assert!(!cpu.regs.halted);
        // Return address on stack must be the instruction after HALT.
        let ret = u16::from(mem.data[cpu.regs.sp as usize])
            | (u16::from(mem.data[cpu.regs.sp.wrapping_add(1) as usize]) << 8);
        assert_eq!(ret, 0x1001);
    }

    #[test]
    fn interrupt_im2_uncontended_is_19_t() {
        let mut cpu = Cpu::new();
        let mut mem = FlatMem::new();
        // Spectrum INTACK data bus is 0xFF → vector at (I<<8)|0xFF.
        mem.data[0xfeff] = 0x00;
        mem.data[0xff00] = 0x40; // → 0x4000
        cpu.regs.i = 0xfe;
        cpu.regs.sp = 0xfffd;
        cpu.regs.iff1 = true;
        cpu.regs.im = 2;
        cpu.regs.pc = 0x0100;
        let t = cpu.interrupt(&mut mem);
        assert_eq!(t, 19);
        assert_eq!(cpu.regs.pc, 0x4000);
    }

    #[test]
    fn interrupt_while_halted_does_not_skip_redirected_pc() {
        let mut cpu = Cpu::new();
        let mut mem = FlatMem::new();
        mem.data[0x1000] = 0x76;
        mem.data[0x8000] = 0xdd; // not HALT
        cpu.regs.pc = 0x1000;
        cpu.regs.sp = 0xfffd;
        cpu.regs.iff1 = true;
        cpu.regs.im = 1;
        cpu.step(&mut mem);
        assert!(cpu.regs.halted);
        // Host redirects PC while halted (debugger / test USR poke).
        cpu.regs.pc = 0x8000;
        let _ = cpu.interrupt(&mut mem);
        let ret = u16::from(mem.data[cpu.regs.sp as usize])
            | (u16::from(mem.data[cpu.regs.sp.wrapping_add(1) as usize]) << 8);
        assert_eq!(ret, 0x8000);
    }
}
