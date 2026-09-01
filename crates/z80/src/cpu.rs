//! Z80 CPU core.

use crate::bus::{Io, Memory};
use crate::opcodes;
use crate::registers::Registers;

/// Fuse `tests.expected` bus-event kinds (`MC`/`MR`/`MW`/`PC`/`PR`/`PW`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FuseEventKind {
    /// Memory contend probe (start of mem cycle or internal IR/addr cycle).
    Mc,
    /// Memory read completes.
    Mr,
    /// Memory write completes.
    Mw,
    /// Port contend.
    Pc,
    /// Port read.
    Pr,
    /// Port write.
    Pw,
}

impl FuseEventKind {
    #[must_use]
    #[allow(dead_code)] // used by `fuse` test harness formatters
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mc => "MC",
            Self::Mr => "MR",
            Self::Mw => "MW",
            Self::Pc => "PC",
            Self::Pr => "PR",
            Self::Pw => "PW",
        }
    }
}

/// One Fuse bus event (absolute `t`; compare with `t - start`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FuseEvent {
    pub t: u64,
    pub kind: FuseEventKind,
    pub addr: u16,
    pub value: Option<u8>,
}

/// Z80 CPU with cycle-counted instruction execution.
#[derive(Clone, Debug, Default)]
pub struct Cpu {
    pub regs: Registers,
    /// Absolute T-state counter (host may wrap/reset per frame).
    pub t: u64,
    /// When true, maskable interrupts are not accepted (between EI and end of following insn).
    pub(crate) interrupt_deferred: bool,
    /// Optional Fuse bus-event log (None in normal emulation — zero overhead beyond one check).
    pub(crate) fuse_log: Option<Vec<FuseEvent>>,
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
        // IRQ ACK bypasses `execute`, which normally clears Q for non-flag ops.
        self.regs.q = 0;
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

    /// Accept a non-maskable interrupt. Always taken; returns T-states of the NMI sequence.
    ///
    /// Clears IFF1 only (IFF2 preserved for `RETN`). Vector is `0x0066`.
    pub fn nmi<M: Memory>(&mut self, mem: &mut M) -> u32 {
        let t0 = self.t;
        if self.regs.halted {
            self.regs.halted = false;
            let op = mem.read(self.regs.pc, self.t).0;
            if op == 0x76 {
                self.regs.pc = self.regs.pc.wrapping_add(1);
            }
        }
        self.regs.iff1 = false;
        self.regs.q = 0;
        self.regs.inc_r();
        // Nominal uncontended NMI is 11T (5T ack + 6T push); `push` accounts for stack writes.
        self.push(mem, self.regs.pc);
        self.regs.pc = 0x0066;
        self.regs.memptr = 0x0066;
        self.add_t(5);
        (self.t.wrapping_sub(t0)) as u32
    }

    #[inline]
    pub(crate) fn add_t(&mut self, dt: u32) {
        self.t = self.t.wrapping_add(u64::from(dt));
    }

    /// IR bus address Fuse uses for internal `contend_read_no_mreq(IR, …)` cycles.
    #[inline]
    #[must_use]
    pub(crate) fn ir(&self) -> u16 {
        u16::from(self.regs.i) << 8 | u16::from(self.regs.r)
    }

    #[inline]
    fn fuse_push(&mut self, kind: FuseEventKind, addr: u16, value: Option<u8>) {
        if let Some(log) = self.fuse_log.as_mut() {
            log.push(FuseEvent {
                t: self.t,
                kind,
                addr,
                value,
            });
        }
    }

    /// Fuse-style no-MREQ contend cycles at `addr` (emit `MC` each T when logging).
    #[inline]
    pub(crate) fn contend_cycles(&mut self, addr: u16, n: u32) {
        if self.fuse_log.is_some() {
            for _ in 0..n {
                self.fuse_push(FuseEventKind::Mc, addr, None);
                self.t = self.t.wrapping_add(1);
            }
        } else {
            self.add_t(n);
        }
    }

    /// Fuse `contend_read(addr, time)`: MC, then a real memory access for wait
    /// (value discarded — no MR). Used when skipping an unread immediate
    /// (JR/DJNZ not taken).
    #[inline]
    pub(crate) fn contend_read_timing<M: Memory>(&mut self, mem: &mut M, addr: u16, time: u32) {
        self.fuse_push(FuseEventKind::Mc, addr, None);
        let (_v, wait) = mem.read(addr, self.t);
        self.add_t(time + wait);
    }

    /// Internal cycles that put IR on the bus (`contend_read_no_mreq(IR, n)`).
    #[inline]
    pub(crate) fn contend_ir_cycles(&mut self, n: u32) {
        let ir = self.ir();
        self.contend_cycles(ir, n);
    }

    #[inline]
    pub(crate) fn read_mem<M: Memory>(&mut self, mem: &mut M, addr: u16) -> u8 {
        self.fuse_push(FuseEventKind::Mc, addr, None);
        let (v, wait) = mem.read(addr, self.t);
        self.add_t(3 + wait);
        self.fuse_push(FuseEventKind::Mr, addr, Some(v));
        v
    }

    #[inline]
    pub(crate) fn write_mem<M: Memory>(&mut self, mem: &mut M, addr: u16, value: u8) {
        self.fuse_push(FuseEventKind::Mc, addr, None);
        let wait = mem.write(addr, value, self.t);
        self.add_t(3 + wait);
        self.fuse_push(FuseEventKind::Mw, addr, Some(value));
    }

    #[inline]
    pub(crate) fn fetch_opcode<M: Memory>(&mut self, mem: &mut M) -> u8 {
        let pc = self.regs.pc;
        self.fuse_push(FuseEventKind::Mc, pc, None);
        let (v, wait) = mem.read(pc, self.t);
        self.regs.pc = pc.wrapping_add(1);
        self.regs.inc_r();
        // Refresh at M1 T4 — 48K ULA snow when I=$40–$7F overlaps video fetch.
        let refresh_addr = u16::from(self.regs.i) << 8 | u16::from(self.regs.r & 0x7f);
        mem.m1_refresh(
            refresh_addr,
            self.t.wrapping_add(3).wrapping_add(u64::from(wait)),
            wait > 0,
        );
        self.add_t(4 + wait);
        self.fuse_push(FuseEventKind::Mr, pc, Some(v));
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
        if self.fuse_log.is_some() {
            // Fuse coretest `readport` timing (FlatMem wait is 0).
            self.fuse_port_preio(port);
            let (v, _wait) = io.in_port(port, self.t);
            self.fuse_push(FuseEventKind::Pr, port, Some(v));
            self.fuse_port_postio(port);
            self.regs.memptr = port.wrapping_add(1);
            v
        } else {
            let (v, wait) = io.in_port(port, self.t);
            self.add_t(4 + wait);
            self.regs.memptr = port.wrapping_add(1);
            v
        }
    }

    #[inline]
    pub(crate) fn out_port<I: Io>(&mut self, io: &mut I, port: u16, value: u8) {
        if self.fuse_log.is_some() {
            self.fuse_port_preio(port);
            let _wait = io.out_port(port, value, self.t);
            self.fuse_push(FuseEventKind::Pw, port, Some(value));
            self.fuse_port_postio(port);
            self.regs.memptr = (port & 0xff00) | (port.wrapping_add(1) & 0x00ff);
        } else {
            let wait = io.out_port(port, value, self.t);
            self.add_t(4 + wait);
            self.regs.memptr = (port & 0xff00) | (port.wrapping_add(1) & 0x00ff);
        }
    }

    /// Fuse `contend_port_preio`: PC if high byte in 0x40–0x7F, then +1T.
    #[inline]
    fn fuse_port_preio(&mut self, port: u16) {
        if port & 0xc000 == 0x4000 {
            self.fuse_push(FuseEventKind::Pc, port, None);
        }
        self.add_t(1);
    }

    /// Fuse `contend_port_postio` (ULA even-port / contended high-byte rules).
    #[inline]
    fn fuse_port_postio(&mut self, port: u16) {
        if port & 0x0001 != 0 {
            // Odd port
            if port & 0xc000 == 0x4000 {
                for _ in 0..3 {
                    self.fuse_push(FuseEventKind::Pc, port, None);
                    self.add_t(1);
                }
            } else {
                self.add_t(3);
            }
        } else {
            // Even port — always one late PC then +3T
            self.fuse_push(FuseEventKind::Pc, port, None);
            self.add_t(3);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::FlatMem;
    use crate::registers::flag;

    #[test]
    fn scf_ccf_use_q_for_undocumented_xy() {
        // After a flag-affecting op, Q == F, so SCF/CCF XY come from A only.
        let mut cpu = Cpu::new();
        let mut mem = FlatMem::new();
        mem.data[0] = 0xaf; // XOR A → F=0x44 (Z|PV), Q=F
        mem.data[1] = 0x37; // SCF
        mem.data[2] = 0x3f; // CCF
        cpu.regs.a = 0x28; // has Y|X
        cpu.regs.f = 0xff;
        cpu.regs.pc = 0;
        cpu.step(&mut mem);
        assert_eq!(cpu.regs.q, cpu.regs.f);
        cpu.regs.a = 0x00; // no XY in A
                           // Carry clear so SCF's set-C assertion is meaningful.
        cpu.regs.f = flag::S | flag::Z | flag::PV | flag::X | flag::Y | flag::H | flag::N;
        cpu.regs.q = cpu.regs.f;
        cpu.step(&mut mem); // SCF
                            // XY must be from A (0), not F|A, when Q == F.
        assert_eq!(cpu.regs.f & (flag::X | flag::Y), 0);
        assert_eq!(cpu.regs.f & flag::C, flag::C);
        assert_eq!(cpu.regs.f & (flag::H | flag::N), 0);

        // CCF with Q == F: XY from A only; carry toggles; H copies prior C.
        cpu.regs.a = 0x00;
        cpu.regs.f = flag::S | flag::Z | flag::PV | flag::X | flag::Y | flag::C;
        cpu.regs.q = cpu.regs.f;
        cpu.step(&mut mem); // CCF
        assert_eq!(cpu.regs.f & (flag::X | flag::Y), 0);
        assert_eq!(cpu.regs.f & flag::C, 0);
        assert_eq!(cpu.regs.f & flag::H, flag::H);
        assert_eq!(cpu.regs.f & flag::N, 0);

        // After a non-flag op, Q == 0, so SCF XY = (F|A) & XY.
        mem.data[3] = 0x00; // NOP clears Q
        mem.data[4] = 0x37; // SCF
        cpu.regs.pc = 3;
        cpu.regs.a = 0x00;
        cpu.regs.f = flag::X | flag::Y;
        cpu.regs.q = cpu.regs.f; // non-zero so NOP clear is observable
        cpu.step(&mut mem); // NOP
        assert_eq!(cpu.regs.q, 0);
        cpu.step(&mut mem); // SCF
        assert_eq!(cpu.regs.f & (flag::X | flag::Y), flag::X | flag::Y);
        assert_eq!(cpu.regs.f & flag::C, flag::C);

        // CCF with Q == 0: XY = (F|A) & XY; carry toggles.
        mem.data[6] = 0x00; // NOP
        mem.data[7] = 0x3f; // CCF
        cpu.regs.pc = 6;
        cpu.regs.a = 0x00;
        cpu.regs.f = flag::X | flag::Y | flag::C;
        cpu.regs.q = cpu.regs.f;
        cpu.step(&mut mem); // NOP
        assert_eq!(cpu.regs.q, 0);
        cpu.step(&mut mem); // CCF
        assert_eq!(cpu.regs.f & (flag::X | flag::Y), flag::X | flag::Y);
        assert_eq!(cpu.regs.f & flag::C, 0);
    }

    #[test]
    fn interrupt_clears_q_before_scf() {
        // IRQ acceptance bypasses execute(), so it must clear Q itself.
        let mut cpu = Cpu::new();
        let mut mem = FlatMem::new();
        mem.data[0x0038] = 0x37; // SCF at IM1 vector
        cpu.regs.sp = 0xfffd;
        cpu.regs.iff1 = true;
        cpu.regs.im = 1;
        cpu.regs.pc = 0x0100;
        cpu.regs.a = 0x00;
        cpu.regs.f = flag::X | flag::Y;
        cpu.regs.q = cpu.regs.f; // stale Q as if after a flag-affecting op
        let t = cpu.interrupt(&mut mem);
        assert_eq!(t, 13);
        assert_eq!(cpu.regs.q, 0);
        cpu.step(&mut mem); // SCF with Q==0 → XY from F|A
        assert_eq!(cpu.regs.f & (flag::X | flag::Y), flag::X | flag::Y);
        assert_eq!(cpu.regs.f & flag::C, flag::C);
    }

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
    fn nmi_vectors_to_0066_and_preserves_iff2() {
        let mut cpu = Cpu::new();
        let mut mem = FlatMem::new();
        cpu.regs.sp = 0xfffd;
        cpu.regs.pc = 0x1234;
        cpu.regs.iff1 = true;
        cpu.regs.iff2 = true;
        let t = cpu.nmi(&mut mem);
        assert_eq!(t, 11, "uncontended NMI is 11 T");
        assert_eq!(cpu.regs.pc, 0x0066);
        assert!(!cpu.regs.iff1);
        assert!(cpu.regs.iff2);
        let ret = u16::from(mem.data[cpu.regs.sp as usize])
            | (u16::from(mem.data[cpu.regs.sp.wrapping_add(1) as usize]) << 8);
        assert_eq!(ret, 0x1234);
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

    /// Skipped displacement probe must call Memory::read (for wait) but emit only MC.
    #[test]
    fn contend_read_timing_adds_wait_without_mr() {
        struct WaitMem {
            wait: u32,
            reads: u32,
        }
        impl crate::bus::Memory for WaitMem {
            fn read(&mut self, _addr: u16, _t: u64) -> (u8, u32) {
                self.reads += 1;
                (0xAB, self.wait)
            }
            fn write(&mut self, _addr: u16, _value: u8, _t: u64) -> u32 {
                0
            }
        }
        impl crate::bus::Io for WaitMem {
            fn in_port(&mut self, _port: u16, _t: u64) -> (u8, u32) {
                (0xff, 0)
            }
            fn out_port(&mut self, _port: u16, _value: u8, _t: u64) -> u32 {
                0
            }
        }

        let mut mem = WaitMem { wait: 6, reads: 0 };
        let mut cpu = Cpu::new();
        cpu.fuse_log = Some(Vec::new());
        cpu.contend_read_timing(&mut mem, 0x4000, 3);
        assert_eq!(mem.reads, 1, "must probe memory for wait");
        assert_eq!(cpu.t, 9, "base 3T + wait 6");
        let log = cpu.fuse_log.as_ref().unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].kind, FuseEventKind::Mc);
        assert_eq!(log[0].addr, 0x4000);
    }
}
