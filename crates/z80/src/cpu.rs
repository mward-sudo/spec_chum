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
    /// Fuse `tests.expected` event labels — only needed by the `fuse` test harness.
    #[cfg(test)]
    #[must_use]
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

/// Instruction set selected for this CPU. The default preserves classic Z80
/// behavior, including the treatment of unsupported ED opcodes as 8 T NOPs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CpuProfile {
    #[default]
    Z80,
    /// Enable the Z80N instructions currently supported by this emulator.
    /// Other Z80N-only opcodes retain classic Z80 decoding, including the
    /// existing unknown-ED behavior where applicable.
    Z80N,
}

/// Z80 CPU with cycle-counted instruction execution.
#[derive(Clone, Debug, Default)]
pub struct Cpu {
    pub regs: Registers,
    pub(crate) profile: CpuProfile,
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

    /// Construct a CPU with an explicit instruction set.
    #[must_use]
    pub fn with_profile(profile: CpuProfile) -> Self {
        Self {
            profile,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn profile(&self) -> CpuProfile {
        self.profile
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
        let (v, wait) = mem.read_opcode(pc, self.t);
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

    struct NextBus {
        data: Box<[u8; 65536]>,
        reads: Vec<(u16, u64)>,
        writes: Vec<(u8, u8, u64)>,
    }

    impl NextBus {
        fn program(bytes: &[u8]) -> Self {
            let mut data = Box::new([0; 65536]);
            data[0x1200..0x1200 + bytes.len()].copy_from_slice(bytes);
            Self {
                data,
                reads: Vec::new(),
                writes: Vec::new(),
            }
        }
    }

    impl Memory for NextBus {
        fn read(&mut self, addr: u16, t: u64) -> (u8, u32) {
            self.reads.push((addr, t));
            (self.data[addr as usize], 0)
        }

        fn write(&mut self, addr: u16, value: u8, _t: u64) -> u32 {
            self.data[addr as usize] = value;
            0
        }
    }

    impl Io for NextBus {
        fn in_port(&mut self, _port: u16, _t: u64) -> (u8, u32) {
            (0xff, 0)
        }

        fn out_port(&mut self, _port: u16, _value: u8, _t: u64) -> u32 {
            0
        }

        fn nextreg_write(&mut self, register: u8, value: u8, t: u64) {
            self.writes.push((register, value, t));
        }
    }

    fn next_cpu() -> Cpu {
        let mut cpu = Cpu::with_profile(CpuProfile::Z80N);
        cpu.regs = Registers {
            a: 0x5a,
            f: 0xd7,
            b: 0x33,
            pc: 0x1200,
            r: 0x81,
            memptr: 0x9876,
            q: 0xac,
            ..Registers::default()
        };
        cpu
    }

    #[test]
    fn nextreg_immediate_writes_register_with_20_timing() {
        let mut cpu = next_cpu();
        let before = cpu.regs;
        let mut bus = NextBus::program(&[0xed, 0x91, 0x50, 0xa7]);

        assert_eq!(cpu.step(&mut bus), 20);
        assert_eq!(cpu.t, 20);
        assert_eq!(
            bus.reads,
            [(0x1200, 0), (0x1201, 4), (0x1202, 8), (0x1203, 11)]
        );
        assert_eq!(bus.writes, [(0x50, 0xa7, 20)]);
        assert_eq!(
            cpu.regs,
            Registers {
                pc: 0x1204,
                r: 0x83,
                q: 0,
                ..before
            }
        );
    }

    #[test]
    fn nextreg_accumulator_writes_a_with_17_timing() {
        let mut cpu = next_cpu();
        let before = cpu.regs;
        let mut bus = NextBus::program(&[0xed, 0x92, 0x50]);

        assert_eq!(cpu.step(&mut bus), 17);
        assert_eq!(cpu.t, 17);
        assert_eq!(bus.reads, [(0x1200, 0), (0x1201, 4), (0x1202, 8)]);
        assert_eq!(bus.writes, [(0x50, 0x5a, 17)]);
        assert_eq!(
            cpu.regs,
            Registers {
                pc: 0x1203,
                r: 0x83,
                q: 0,
                ..before
            }
        );
    }

    #[test]
    fn z80n_add_bc_a_is_eight_t_states_and_profile_gated() {
        let mut cpu = next_cpu();
        cpu.regs.set_bc(0xfff0);
        let flags = cpu.regs.f;
        let mut bus = NextBus::program(&[0xed, 0x33, 0x3e, 0x44]);

        assert_eq!(cpu.step(&mut bus), 8);
        assert_eq!(cpu.regs.bc(), 0x004a);
        assert_eq!(cpu.regs.f, flags & !flag::C);
        assert_eq!(cpu.regs.q, flags & !flag::C);
        assert_eq!(cpu.regs.pc, 0x1202);
        assert_eq!(cpu.step(&mut bus), 7, "following LD A,n must remain intact");
        assert_eq!(cpu.regs.a, 0x44);

        let mut classic = Cpu::new();
        classic.regs.a = 0x22;
        classic.regs.set_bc(0xc000);
        classic.regs.pc = 0x1200;
        let mut classic_bus = NextBus::program(&[0xed, 0x33, 0x3e, 0x55]);
        assert_eq!(classic.step(&mut classic_bus), 8);
        assert_eq!(classic.regs.bc(), 0xc000);
        assert_eq!(classic.regs.pc, 0x1202);
        assert_eq!(classic.step(&mut classic_bus), 7);
        assert_eq!(classic.regs.a, 0x55);
    }

    #[test]
    fn z80n_add_pair_immediates_preserve_flags_and_use_little_endian_operands() {
        for (opcode, initial, expected) in [
            (0x31, 0x1234, 0x128e),
            (0x32, 0x2345, 0x239f),
            (0x33, 0x3456, 0x34b0),
        ] {
            let mut cpu = next_cpu();
            cpu.regs.set_hl(0x1234);
            cpu.regs.set_de(0x2345);
            cpu.regs.set_bc(0x3456);
            let flags = cpu.regs.f;
            let pair = match opcode {
                0x31 => cpu.regs.hl(),
                0x32 => cpu.regs.de(),
                _ => cpu.regs.bc(),
            };
            assert_eq!(pair, initial);
            let mut bus = NextBus::program(&[0xed, opcode]);

            assert_eq!(cpu.step(&mut bus), 8);
            let result = match opcode {
                0x31 => cpu.regs.hl(),
                0x32 => cpu.regs.de(),
                _ => cpu.regs.bc(),
            };
            assert_eq!(result, initial + u16::from(cpu.regs.a));
            assert_eq!(result, expected);
            assert_eq!(cpu.regs.f, flags & !flag::C);
        }

        for (opcode, expected) in [(0x34, 0x1234), (0x35, 0x2234), (0x36, 0x3234)] {
            let mut cpu = next_cpu();
            cpu.regs.set_hl(0x1000);
            cpu.regs.set_de(0x2000);
            cpu.regs.set_bc(0x3000);
            let flags = cpu.regs.f;
            let mut bus = NextBus::program(&[0xed, opcode, 0x34, 0x02]);

            assert_eq!(cpu.step(&mut bus), 16);
            let result = match opcode {
                0x34 => cpu.regs.hl(),
                0x35 => cpu.regs.de(),
                _ => cpu.regs.bc(),
            };
            assert_eq!(result, expected);
            assert_eq!(cpu.regs.f, flags);
            assert_eq!(cpu.regs.pc, 0x1204);
        }
    }

    #[test]
    fn z80n_push_immediate_uses_big_endian_operand_and_is_profile_gated() {
        let mut cpu = next_cpu();
        cpu.regs.sp = 0x9000;
        let mut bus = NextBus::program(&[0xed, 0x8a, 0x00, 0x01, 0xc3, 0xa0, 0x1e]);

        assert_eq!(cpu.step(&mut bus), 23);
        assert_eq!(cpu.regs.sp, 0x8ffe);
        assert_eq!(bus.data[0x8ffe], 0x01);
        assert_eq!(bus.data[0x8fff], 0x00);
        assert_eq!(cpu.regs.pc, 0x1204);
        assert_eq!(cpu.step(&mut bus), 10, "following JP must remain aligned");
        assert_eq!(cpu.regs.pc, 0x1ea0);

        let mut classic = Cpu::new();
        classic.regs.pc = 0x1200;
        classic.regs.sp = 0x9000;
        let mut classic_bus = NextBus::program(&[0xed, 0x8a, 0x3e, 0x55]);
        assert_eq!(classic.step(&mut classic_bus), 8);
        assert_eq!(classic.regs.sp, 0x9000);
        assert_eq!(classic.regs.pc, 0x1202);
        assert_eq!(classic.step(&mut classic_bus), 7);
        assert_eq!(classic.regs.a, 0x55);
    }

    #[test]
    fn z80n_swapnib_bsra_and_mul_preserve_flags_and_are_profile_gated() {
        for (opcode, expected_a, expected_de) in [
            (0x23, 0xc3, 0x1234),
            (0x29, 0x5a, 0xff00),
            (0x30, 0x5a, 0x00fc),
        ] {
            let mut cpu = next_cpu();
            cpu.regs.set_de(0x1234);
            match opcode {
                0x23 => cpu.regs.a = 0x3c,
                0x29 => {
                    cpu.regs.set_de(0xf000);
                    cpu.regs.b = 4;
                }
                0x30 => {
                    cpu.regs.d = 12;
                    cpu.regs.e = 21;
                }
                _ => unreachable!(),
            }
            let flags = cpu.regs.f;
            let mut bus = NextBus::program(&[0xed, opcode, 0x3e, 0x44]);

            assert_eq!(cpu.step(&mut bus), 8);
            assert_eq!(cpu.regs.a, expected_a);
            assert_eq!(cpu.regs.de(), expected_de);
            assert_eq!(cpu.regs.f, flags);
            assert_eq!(cpu.regs.pc, 0x1202);
            assert_eq!(cpu.step(&mut bus), 7, "following LD A,n must remain intact");
            assert_eq!(cpu.regs.a, 0x44);
        }

        let mut classic = Cpu::new();
        classic.regs.a = 0x3c;
        classic.regs.set_de(0x1234);
        classic.regs.b = 4;
        classic.regs.pc = 0x1200;
        let mut bus = NextBus::program(&[0xed, 0x23, 0xed, 0x29, 0xed, 0x30, 0x3e, 0x55]);
        for index in 0..3 {
            assert_eq!(classic.step(&mut bus), 8);
            assert_eq!(classic.regs.pc, 0x1202 + index as u16 * 2);
        }
        assert_eq!(classic.regs.a, 0x3c);
        assert_eq!(classic.regs.de(), 0x1234);
        assert_eq!(classic.step(&mut bus), 7);
        assert_eq!(classic.regs.a, 0x55);
    }

    #[test]
    fn z80n_bsra_sign_extends_large_shift_counts() {
        for (shift, expected) in [(16, 0xffff), (31, 0xffff)] {
            let mut cpu = next_cpu();
            cpu.regs.set_de(0x8001);
            cpu.regs.b = shift;
            let mut bus = NextBus::program(&[0xed, 0x29]);

            assert_eq!(cpu.step(&mut bus), 8);
            assert_eq!(cpu.regs.de(), expected, "BSRA B={shift}");
        }
    }

    #[test]
    fn z80n_bsrl_is_logical_masks_shift_count_and_is_profile_gated() {
        for (value, shift, expected) in [
            (0x8001, 1, 0x4000),
            (0x8001, 0, 0x8001),
            (0x8001, 16, 0),
            (0x8001, 31, 0),
            (0x8001, 32, 0x8001),
        ] {
            let mut cpu = next_cpu();
            cpu.regs.set_de(value);
            cpu.regs.b = shift;
            cpu.regs.f = 0xd7;
            let mut bus = NextBus::program(&[0xed, 0x2a, 0x3e, 0x44]);

            assert_eq!(cpu.step(&mut bus), 8);
            assert_eq!(cpu.regs.de(), expected);
            assert_eq!(cpu.regs.f, 0xd7);
            assert_eq!(cpu.regs.pc, 0x1202);
            assert_eq!(cpu.step(&mut bus), 7, "following LD A,n must remain intact");
            assert_eq!(cpu.regs.a, 0x44);
        }

        let mut classic = Cpu::new();
        classic.regs.set_de(0x8001);
        classic.regs.b = 1;
        classic.regs.pc = 0x1200;
        let mut bus = NextBus::program(&[0xed, 0x2a, 0x3e, 0x55]);
        assert_eq!(classic.step(&mut bus), 8);
        assert_eq!(classic.regs.de(), 0x8001);
        assert_eq!(classic.regs.pc, 0x1202);
        assert_eq!(classic.step(&mut bus), 7);
        assert_eq!(classic.regs.a, 0x55);
    }

    #[test]
    fn z80n_ula_pixel_opcodes_calculate_screen_addresses_and_are_profile_gated() {
        for (y, x, expected) in [(0, 0, 0x4000), (191, 255, 0x57ff), (8, 16, 0x4022)] {
            let mut cpu = next_cpu();
            cpu.regs.d = y;
            cpu.regs.e = x;
            cpu.regs.f = 0xd7;
            let mut bus = NextBus::program(&[0xed, 0x94, 0x3e, 0x44]);

            assert_eq!(cpu.step(&mut bus), 8);
            assert_eq!(cpu.regs.hl(), expected);
            assert_eq!(cpu.regs.f, 0xd7);
            assert_eq!(cpu.step(&mut bus), 7);
            assert_eq!(cpu.regs.a, 0x44);
        }

        let mut cpu = next_cpu();
        cpu.regs.set_hl(0xffff);
        let mut bus = NextBus::program(&[0xed, 0x93]);
        assert_eq!(cpu.step(&mut bus), 8);
        assert_eq!(
            cpu.regs.hl(),
            0x001f,
            "PIXELDN must wrap at the u16 boundary"
        );

        let mut cpu = next_cpu();
        cpu.regs.set_hl(0x4700);
        cpu.regs.f = 0xa5;
        let mut bus = NextBus::program(&[0xed, 0x93, 0xed, 0x93, 0x3e, 0x44]);
        assert_eq!(cpu.step(&mut bus), 8);
        assert_eq!(cpu.regs.hl(), 0x4020);
        assert_eq!(cpu.regs.f, 0xa5);
        assert_eq!(cpu.step(&mut bus), 8);
        assert_eq!(cpu.regs.hl(), 0x4120);
        assert_eq!(cpu.step(&mut bus), 7);
        assert_eq!(cpu.regs.a, 0x44);

        let mut classic = Cpu::new();
        classic.regs.d = 191;
        classic.regs.e = 255;
        classic.regs.set_hl(0x1234);
        classic.regs.a = 0x55;
        classic.regs.pc = 0x1200;
        let mut bus = NextBus::program(&[0xed, 0x94, 0xed, 0x93, 0xed, 0x95, 0x3e, 0x66]);
        for expected_pc in [0x1202, 0x1204, 0x1206] {
            assert_eq!(classic.step(&mut bus), 8);
            assert_eq!(classic.regs.pc, expected_pc);
        }
        assert_eq!(classic.regs.hl(), 0x1234);
        assert_eq!(classic.regs.a, 0x55);
        assert_eq!(classic.step(&mut bus), 7);
        assert_eq!(classic.regs.a, 0x66);
    }

    #[test]
    fn indexed_displacement_loads_into_plain_h_and_l() {
        for prefix in [0xdd, 0xfd] {
            let mut cpu = Cpu::new();
            if prefix == 0xdd {
                cpu.regs.set_ix(0x2000);
                cpu.regs.set_iy(0x5678);
            } else {
                cpu.regs.set_ix(0x1234);
                cpu.regs.set_iy(0x2000);
            }
            cpu.regs.set_hl(0xabcd);
            cpu.regs.pc = 0x1200;
            let mut bus = NextBus::program(&[prefix, 0x66, 0x01, prefix, 0x6e, 0x02]);
            bus.data[0x2001] = 0x91;
            bus.data[0x2002] = 0x27;

            assert_eq!(cpu.step(&mut bus), 19);
            assert_eq!(cpu.regs.h, 0x91);
            assert_eq!(cpu.regs.ix(), if prefix == 0xdd { 0x2000 } else { 0x1234 });
            assert_eq!(cpu.regs.iy(), if prefix == 0xfd { 0x2000 } else { 0x5678 });
            assert_eq!(cpu.step(&mut bus), 19);
            assert_eq!(cpu.regs.l, 0x27);
            assert_eq!(cpu.regs.hl(), 0x9127);
        }
    }

    #[test]
    fn z80n_nextreg_accepts_ignored_index_prefixes() {
        for (bytes, expected_t, expected_len, expected_value) in [
            (&[0xdd, 0xed, 0x91, 0x50, 0xa7, 0x3e, 0x44][..], 24, 5, 0xa7),
            (&[0xfd, 0xed, 0x92, 0x50, 0x3e, 0x44][..], 21, 4, 0x5a),
        ] {
            let mut cpu = next_cpu();
            let mut bus = NextBus::program(bytes);

            assert_eq!(cpu.step(&mut bus), expected_t);
            assert_eq!(cpu.regs.pc, 0x1200 + expected_len);
            assert_eq!(bus.writes, [(0x50, expected_value, u64::from(expected_t))]);
            assert_eq!(cpu.step(&mut bus), 7, "following LD A,n must remain intact");
            assert_eq!(cpu.regs.a, 0x44);
        }
    }

    #[test]
    fn classic_ed_91_and_92_remain_unknown_eight_t_instructions() {
        assert_eq!(Cpu::new().profile(), CpuProfile::Z80);
        for opcode in [0x91, 0x92] {
            let mut cpu = Cpu::new();
            cpu.regs.pc = 0x1200;
            let mut bus = NextBus::program(&[0xed, opcode, 0x3e, 0x55]);

            assert_eq!(cpu.step(&mut bus), 8);
            assert_eq!(cpu.regs.pc, 0x1202);
            assert_eq!(bus.reads, [(0x1200, 0), (0x1201, 4)]);
            assert_eq!(bus.writes, []);
            assert_eq!(cpu.step(&mut bus), 7, "following LD A,n must remain intact");
            assert_eq!(cpu.regs.a, 0x55);
            assert_eq!(cpu.regs.pc, 0x1204);
        }
    }

    #[test]
    fn reset_keeps_the_instruction_profile() {
        let mut cpu = next_cpu();
        cpu.reset();
        assert_eq!(cpu.profile(), CpuProfile::Z80N);
    }

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

    /// Skipped displacement probe must call [`Memory::read`] (for wait) but emit only `MC`.
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
