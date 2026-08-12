//! Spec Chum machine — Spectrum models and frame runner.

#![allow(clippy::pedantic)]
#![allow(clippy::large_enum_variant)]

#[cfg(all(test, feature = "slow-tests"))]
mod z80test;

use bus::{Bus128, Bus48, BusPlus3};
use formats::Snapshot48;
use tape::{evaluate_ld_bytes_trap, flash_load_block, TapPlayer, TapeTrapResult, LD_BYTES_TRAP_PC};
use ula::{int_active_48, Ula48, FRAME_TSTATES_128, FRAME_TSTATES_48, INT_LENGTH_128};
use z80::{flag, Cpu, Io, Memory};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Model {
    Spectrum48,
    Spectrum128,
    /// Amstrad +2A / +3 gate array (port `1FFD`, no floating bus).
    SpectrumPlus3,
}

/// Memory+Io adapter for 48K.
#[derive(Debug)]
pub struct MemIo48<'a> {
    pub bus: &'a mut Bus48,
}

impl Memory for MemIo48<'_> {
    fn read(&mut self, addr: u16, _t: u64) -> (u8, u32) {
        let wait = self.bus.contend_at(addr);
        (self.bus.read(addr), wait)
    }

    fn write(&mut self, addr: u16, value: u8, _t: u64) -> u32 {
        let wait = self.bus.contend_at(addr);
        self.bus.write(addr, value);
        wait
    }
}

impl Io for MemIo48<'_> {
    fn in_port(&mut self, port: u16, _t: u64) -> (u8, u32) {
        let wait = if port & 1 == 0 {
            self.bus.contend_at(0x4000)
        } else {
            0
        };
        (self.bus.in_port(port), wait)
    }

    fn out_port(&mut self, port: u16, value: u8, _t: u64) -> u32 {
        let wait = if port & 1 == 0 {
            self.bus.contend_at(0x4000)
        } else {
            0
        };
        self.bus.out_port(port, value);
        wait
    }
}

#[derive(Debug)]
pub struct MemIo128<'a> {
    pub bus: &'a mut Bus128,
}

impl Memory for MemIo128<'_> {
    fn read(&mut self, addr: u16, _t: u64) -> (u8, u32) {
        let wait = self.bus.contend_at(addr);
        (self.bus.read(addr), wait)
    }

    fn write(&mut self, addr: u16, value: u8, _t: u64) -> u32 {
        let wait = self.bus.contend_at(addr);
        self.bus.write(addr, value);
        wait
    }
}

impl Io for MemIo128<'_> {
    fn in_port(&mut self, port: u16, _t: u64) -> (u8, u32) {
        let wait = if port & 1 == 0 {
            self.bus.contend_at(0x4000)
        } else {
            0
        };
        (self.bus.in_port(port), wait)
    }

    fn out_port(&mut self, port: u16, value: u8, _t: u64) -> u32 {
        let wait = if port & 1 == 0 {
            self.bus.contend_at(0x4000)
        } else {
            0
        };
        self.bus.out_port(port, value);
        wait
    }
}

#[derive(Debug)]
pub struct MemIoPlus3<'a> {
    pub bus: &'a mut BusPlus3,
}

impl Memory for MemIoPlus3<'_> {
    fn read(&mut self, addr: u16, _t: u64) -> (u8, u32) {
        let wait = self.bus.contend_at(addr);
        (self.bus.read(addr), wait)
    }

    fn write(&mut self, addr: u16, value: u8, _t: u64) -> u32 {
        let wait = self.bus.contend_at(addr);
        self.bus.write(addr, value);
        wait
    }
}

impl Io for MemIoPlus3<'_> {
    fn in_port(&mut self, port: u16, _t: u64) -> (u8, u32) {
        let wait = if port & 1 == 0 {
            self.bus.contend_at(0x4000)
        } else {
            0
        };
        (self.bus.in_port(port), wait)
    }

    fn out_port(&mut self, port: u16, value: u8, _t: u64) -> u32 {
        let wait = if port & 1 == 0 {
            self.bus.contend_at(0x4000)
        } else {
            0
        };
        self.bus.out_port(port, value);
        wait
    }
}

#[derive(Clone, Debug)]
pub enum Machine {
    Spec48 {
        cpu: Cpu,
        bus: Box<Bus48>,
        ula: Ula48,
        tape: Option<TapPlayer>,
    },
    Spec128 {
        cpu: Cpu,
        bus: Box<Bus128>,
        ula: Ula48,
        tape: Option<TapPlayer>,
    },
    SpecPlus3 {
        cpu: Cpu,
        bus: Box<BusPlus3>,
        ula: Ula48,
        tape: Option<TapPlayer>,
    },
}

#[derive(Clone, Debug, Default)]
pub struct FrameAudio {
    /// Beeper edges: (frame_t, level).
    pub beeper_edges: Vec<(u32, bool)>,
    /// Mono AY samples for this frame (empty on 48K). Amplitude roughly 0..1.
    pub ay_samples: Vec<f32>,
}

impl Machine {
    pub fn new_48k(rom: &[u8]) -> Result<Self, String> {
        let mut bus = Bus48::new();
        bus.load_rom(rom)?;
        Ok(Self::Spec48 {
            cpu: Cpu::new(),
            bus: Box::new(bus),
            ula: Ula48::new(),
            tape: None,
        })
    }

    pub fn new_128k(rom: &[u8]) -> Result<Self, String> {
        let mut bus = Bus128::new();
        bus.load_rom128(rom)?;
        Ok(Self::Spec128 {
            cpu: Cpu::new(),
            bus: Box::new(bus),
            ula: Ula48::new(),
            tape: None,
        })
    }

    pub fn new_plus3(rom: &[u8]) -> Result<Self, String> {
        let mut bus = BusPlus3::new();
        bus.load_rom64(rom)?;
        Ok(Self::SpecPlus3 {
            cpu: Cpu::new(),
            bus: Box::new(bus),
            ula: Ula48::new(),
            tape: None,
        })
    }

    #[must_use]
    pub fn model(&self) -> Model {
        match self {
            Self::Spec48 { .. } => Model::Spectrum48,
            Self::Spec128 { .. } => Model::Spectrum128,
            Self::SpecPlus3 { .. } => Model::SpectrumPlus3,
        }
    }

    pub fn reset(&mut self) {
        match self {
            Self::Spec48 {
                cpu,
                bus,
                ula,
                tape,
            } => {
                cpu.reset();
                bus.keyboard.reset();
                bus.frame_t = 0;
                bus.beeper_edges.clear();
                *ula = Ula48::new();
                *tape = None;
            }
            Self::Spec128 {
                cpu,
                bus,
                ula,
                tape,
            } => {
                cpu.reset();
                bus.keyboard.reset();
                bus.frame_t = 0;
                bus.page = 0;
                bus.locked = false;
                bus.beeper_edges.clear();
                bus.ay.reset();
                *ula = Ula48::new();
                *tape = None;
            }
            Self::SpecPlus3 {
                cpu,
                bus,
                ula,
                tape,
            } => {
                cpu.reset();
                bus.keyboard.reset();
                bus.frame_t = 0;
                bus.page_7ffd = 0;
                bus.page_1ffd = 0;
                bus.locked = false;
                bus.beeper_edges.clear();
                bus.ay.reset();
                *ula = Ula48::new();
                *tape = None;
            }
        }
    }

    pub fn insert_tape(&mut self, player: TapPlayer) {
        match self {
            Self::Spec48 { tape, .. }
            | Self::Spec128 { tape, .. }
            | Self::SpecPlus3 { tape, .. } => *tape = Some(player),
        }
    }

    /// Current tape block index, if a player is inserted.
    #[must_use]
    pub fn tape_block(&self) -> Option<usize> {
        match self {
            Self::Spec48 { tape, .. }
            | Self::Spec128 { tape, .. }
            | Self::SpecPlus3 { tape, .. } => tape.as_ref().map(|t| t.block),
        }
    }

    #[must_use]
    pub fn ear(&self) -> bool {
        match self {
            Self::Spec48 { bus, .. } => bus.ear,
            Self::Spec128 { bus, .. } => bus.ear,
            Self::SpecPlus3 { bus, .. } => bus.ear,
        }
    }

    pub fn apply_snapshot48(&mut self, snap: &Snapshot48) {
        {
            let cpu = self.cpu_mut();
            cpu.regs.set_af(snap.af);
            cpu.regs.set_bc(snap.bc);
            cpu.regs.set_de(snap.de);
            cpu.regs.set_hl(snap.hl);
            cpu.regs.set_ix(snap.ix);
            cpu.regs.set_iy(snap.iy);
            cpu.regs.sp = snap.sp;
            cpu.regs.pc = snap.pc;
            cpu.regs.i = snap.i;
            cpu.regs.r = snap.r;
            cpu.regs.im = snap.im;
            cpu.regs.iff1 = snap.iff2;
            cpu.regs.iff2 = snap.iff2;
            cpu.regs.a_ = (snap.af_ >> 8) as u8;
            cpu.regs.f_ = snap.af_ as u8;
            cpu.regs.b_ = (snap.bc_ >> 8) as u8;
            cpu.regs.c_ = snap.bc_ as u8;
            cpu.regs.d_ = (snap.de_ >> 8) as u8;
            cpu.regs.e_ = snap.de_ as u8;
            cpu.regs.h_ = (snap.hl_ >> 8) as u8;
            cpu.regs.l_ = snap.hl_ as u8;
        }
        for (i, b) in snap.ram.iter().enumerate() {
            self.write_mem(0x4000 + i as u16, *b);
        }
        if let Self::Spec48 { bus, ula, .. } = self {
            bus.border = snap.border;
            bus.ula.border = snap.border;
            ula.border = snap.border;
        }
    }

    /// Run one video frame; returns beeper edges and AY samples for the frame.
    pub fn run_frame(&mut self) -> FrameAudio {
        match self {
            Self::Spec48 {
                cpu,
                bus,
                ula,
                tape,
            } => {
                bus.beeper_edges.clear();
                bus.frame_t = 0;
                bus.ula.border = bus.border;
                bus.ula.begin_frame();
                ula.border = bus.border;
                ula.begin_frame();
                let mut last_t = cpu.t;
                while bus.frame_t < FRAME_TSTATES_48 {
                    if Self::try_flash_load_48(cpu, bus, tape) {
                        continue;
                    }
                    if int_active_48(bus.frame_t) {
                        let mut mio = MemIo48 { bus: bus.as_mut() };
                        let irq_t = cpu.interrupt(&mut mio);
                        if irq_t > 0 {
                            Self::advance_tape_ear(tape, &mut bus.ear, irq_t);
                            bus.frame_t = (bus.frame_t + irq_t) % FRAME_TSTATES_48;
                            last_t = cpu.t;
                            continue;
                        }
                    }
                    let mut mio = MemIo48 { bus: bus.as_mut() };
                    cpu.step(&mut mio);
                    let dt = (cpu.t - last_t) as u32;
                    last_t = cpu.t;
                    Self::advance_tape_ear(tape, &mut bus.ear, dt);
                    bus.frame_t += dt;
                    if bus.frame_t >= FRAME_TSTATES_48 {
                        break;
                    }
                }
                // Keep border_events for render; next run_frame begin_frame clears them.
                bus.frame_t = 0;
                FrameAudio {
                    beeper_edges: std::mem::take(&mut bus.beeper_edges),
                    ay_samples: Vec::new(),
                }
            }
            Self::Spec128 {
                cpu,
                bus,
                ula,
                tape,
            } => {
                bus.beeper_edges.clear();
                bus.frame_t = 0;
                bus.ula.border = bus.border;
                bus.ula.begin_frame();
                ula.border = bus.border;
                ula.begin_frame();
                const AY_SAMPLES: usize = 882; // ~44100 Hz / 50 Hz
                let t_per_sample = f64::from(FRAME_TSTATES_128) / AY_SAMPLES as f64;
                let mut ay_samples = Vec::with_capacity(AY_SAMPLES);
                let mut last_t = cpu.t;
                while bus.frame_t < FRAME_TSTATES_128 {
                    if Self::try_flash_load_128(cpu, bus, tape) {
                        continue;
                    }
                    if bus.frame_t < INT_LENGTH_128 {
                        let mut mio = MemIo128 { bus: bus.as_mut() };
                        let irq_t = cpu.interrupt(&mut mio);
                        if irq_t > 0 {
                            Self::advance_tape_ear(tape, &mut bus.ear, irq_t);
                            bus.ay.advance(irq_t);
                            bus.frame_t = (bus.frame_t + irq_t) % FRAME_TSTATES_128;
                            while ay_samples.len() < AY_SAMPLES
                                && f64::from(bus.frame_t)
                                    >= (ay_samples.len() as f64 + 1.0) * t_per_sample
                            {
                                ay_samples.push(bus.ay.sample_mono());
                            }
                            last_t = cpu.t;
                            continue;
                        }
                    }
                    let mut mio = MemIo128 { bus: bus.as_mut() };
                    cpu.step(&mut mio);
                    let dt = (cpu.t - last_t) as u32;
                    last_t = cpu.t;
                    Self::advance_tape_ear(tape, &mut bus.ear, dt);
                    bus.ay.advance(dt);
                    bus.frame_t += dt;
                    while ay_samples.len() < AY_SAMPLES
                        && f64::from(bus.frame_t.min(FRAME_TSTATES_128))
                            >= (ay_samples.len() as f64 + 1.0) * t_per_sample
                    {
                        ay_samples.push(bus.ay.sample_mono());
                    }
                    if bus.frame_t >= FRAME_TSTATES_128 {
                        break;
                    }
                }
                while ay_samples.len() < AY_SAMPLES {
                    ay_samples.push(bus.ay.sample_mono());
                }
                bus.frame_t = 0;
                FrameAudio {
                    beeper_edges: std::mem::take(&mut bus.beeper_edges),
                    ay_samples,
                }
            }
            Self::SpecPlus3 {
                cpu,
                bus,
                ula,
                tape,
            } => {
                bus.beeper_edges.clear();
                bus.frame_t = 0;
                bus.ula.border = bus.border;
                bus.ula.begin_frame();
                ula.border = bus.border;
                ula.begin_frame();
                const AY_SAMPLES: usize = 882;
                let t_per_sample = f64::from(FRAME_TSTATES_128) / AY_SAMPLES as f64;
                let mut ay_samples = Vec::with_capacity(AY_SAMPLES);
                let mut last_t = cpu.t;
                while bus.frame_t < FRAME_TSTATES_128 {
                    if Self::try_flash_load_plus3(cpu, bus, tape) {
                        continue;
                    }
                    if bus.frame_t < INT_LENGTH_128 {
                        let mut mio = MemIoPlus3 { bus: bus.as_mut() };
                        let irq_t = cpu.interrupt(&mut mio);
                        if irq_t > 0 {
                            Self::advance_tape_ear(tape, &mut bus.ear, irq_t);
                            bus.ay.advance(irq_t);
                            bus.frame_t = (bus.frame_t + irq_t) % FRAME_TSTATES_128;
                            while ay_samples.len() < AY_SAMPLES
                                && f64::from(bus.frame_t)
                                    >= (ay_samples.len() as f64 + 1.0) * t_per_sample
                            {
                                ay_samples.push(bus.ay.sample_mono());
                            }
                            last_t = cpu.t;
                            continue;
                        }
                    }
                    let mut mio = MemIoPlus3 { bus: bus.as_mut() };
                    cpu.step(&mut mio);
                    let dt = (cpu.t - last_t) as u32;
                    last_t = cpu.t;
                    Self::advance_tape_ear(tape, &mut bus.ear, dt);
                    bus.ay.advance(dt);
                    bus.frame_t += dt;
                    while ay_samples.len() < AY_SAMPLES
                        && f64::from(bus.frame_t.min(FRAME_TSTATES_128))
                            >= (ay_samples.len() as f64 + 1.0) * t_per_sample
                    {
                        ay_samples.push(bus.ay.sample_mono());
                    }
                    if bus.frame_t >= FRAME_TSTATES_128 {
                        break;
                    }
                }
                while ay_samples.len() < AY_SAMPLES {
                    ay_samples.push(bus.ay.sample_mono());
                }
                bus.frame_t = 0;
                FrameAudio {
                    beeper_edges: std::mem::take(&mut bus.beeper_edges),
                    ay_samples,
                }
            }
        }
    }

    fn advance_tape_ear(tape: &mut Option<TapPlayer>, ear: &mut bool, dt: u32) {
        if dt == 0 {
            return;
        }
        if let Some(t) = tape.as_mut() {
            *ear = t.advance(dt);
        }
    }

    fn ret_from_tape_trap(cpu: &mut Cpu, lo: u8, hi: u8, success: bool) {
        if success {
            cpu.regs.f |= flag::C;
            cpu.regs.set_de(0);
        } else {
            cpu.regs.f &= !flag::C;
        }
        cpu.regs.sp = cpu.regs.sp.wrapping_add(2);
        cpu.regs.pc = u16::from_le_bytes([lo, hi]);
    }

    fn try_flash_load_48(cpu: &mut Cpu, bus: &mut Bus48, tape: &mut Option<TapPlayer>) -> bool {
        if cpu.regs.pc != LD_BYTES_TRAP_PC {
            return false;
        }
        let Some(player) = tape.as_mut() else {
            return false;
        };
        let flag_expected = cpu.regs.a;
        let load = cpu.regs.f & flag::C != 0;
        let addr = cpu.regs.ix();
        let len = cpu.regs.de();
        let sp = cpu.regs.sp;
        let ret_lo = bus.read(sp);
        let ret_hi = bus.read(sp.wrapping_add(1));
        let result = evaluate_ld_bytes_trap(cpu.regs.pc, flag_expected, load, addr, len, player);
        match result {
            TapeTrapResult::Ignored => false,
            TapeTrapResult::Success { addr: dest, len: n } => {
                if load {
                    if let Some(block) = player.image.blocks.get(player.block.wrapping_sub(1)) {
                        flash_load_block(&mut |a, v| bus.write(a, v), block, dest);
                    }
                }
                cpu.regs.set_ix(dest.wrapping_add(n));
                Self::ret_from_tape_trap(cpu, ret_lo, ret_hi, true);
                true
            }
            TapeTrapResult::Failure => {
                Self::ret_from_tape_trap(cpu, ret_lo, ret_hi, false);
                true
            }
        }
    }

    fn try_flash_load_128(cpu: &mut Cpu, bus: &mut Bus128, tape: &mut Option<TapPlayer>) -> bool {
        if cpu.regs.pc != LD_BYTES_TRAP_PC {
            return false;
        }
        let Some(player) = tape.as_mut() else {
            return false;
        };
        let flag_expected = cpu.regs.a;
        let load = cpu.regs.f & flag::C != 0;
        let addr = cpu.regs.ix();
        let len = cpu.regs.de();
        let sp = cpu.regs.sp;
        let ret_lo = bus.read(sp);
        let ret_hi = bus.read(sp.wrapping_add(1));
        let result = evaluate_ld_bytes_trap(cpu.regs.pc, flag_expected, load, addr, len, player);
        match result {
            TapeTrapResult::Ignored => false,
            TapeTrapResult::Success { addr: dest, len: n } => {
                if load {
                    if let Some(block) = player.image.blocks.get(player.block.wrapping_sub(1)) {
                        flash_load_block(&mut |a, v| bus.write(a, v), block, dest);
                    }
                }
                cpu.regs.set_ix(dest.wrapping_add(n));
                Self::ret_from_tape_trap(cpu, ret_lo, ret_hi, true);
                true
            }
            TapeTrapResult::Failure => {
                Self::ret_from_tape_trap(cpu, ret_lo, ret_hi, false);
                true
            }
        }
    }

    fn try_flash_load_plus3(
        cpu: &mut Cpu,
        bus: &mut BusPlus3,
        tape: &mut Option<TapPlayer>,
    ) -> bool {
        if cpu.regs.pc != LD_BYTES_TRAP_PC {
            return false;
        }
        let Some(player) = tape.as_mut() else {
            return false;
        };
        let flag_expected = cpu.regs.a;
        let load = cpu.regs.f & flag::C != 0;
        let addr = cpu.regs.ix();
        let len = cpu.regs.de();
        let sp = cpu.regs.sp;
        let ret_lo = bus.read(sp);
        let ret_hi = bus.read(sp.wrapping_add(1));
        let result = evaluate_ld_bytes_trap(cpu.regs.pc, flag_expected, load, addr, len, player);
        match result {
            TapeTrapResult::Ignored => false,
            TapeTrapResult::Success { addr: dest, len: n } => {
                if load {
                    if let Some(block) = player.image.blocks.get(player.block.wrapping_sub(1)) {
                        flash_load_block(&mut |a, v| bus.write(a, v), block, dest);
                    }
                }
                cpu.regs.set_ix(dest.wrapping_add(n));
                Self::ret_from_tape_trap(cpu, ret_lo, ret_hi, true);
                true
            }
            TapeTrapResult::Failure => {
                Self::ret_from_tape_trap(cpu, ret_lo, ret_hi, false);
                true
            }
        }
    }

    /// Execute until at least `min_t` T-states elapse (tape + IRQs included).
    pub fn run_tstates(&mut self, min_t: u32) {
        let mut left = min_t;
        while left > 0 {
            let before = self.cpu().t;
            self.step_once();
            let dt = (self.cpu().t - before) as u32;
            if dt == 0 {
                // Halted or trap with no T — still count a minimum slice.
                left = left.saturating_sub(1);
            } else {
                left = left.saturating_sub(dt);
            }
        }
    }

    /// One machine step: flash-load trap, optional IRQ, or one CPU instruction.
    pub fn step_once(&mut self) {
        match self {
            Self::Spec48 { cpu, bus, tape, .. } => {
                if Self::try_flash_load_48(cpu, bus, tape) {
                    return;
                }
                if int_active_48(bus.frame_t) {
                    let mut mio = MemIo48 { bus: bus.as_mut() };
                    let irq_t = cpu.interrupt(&mut mio);
                    if irq_t > 0 {
                        Self::advance_tape_ear(tape, &mut bus.ear, irq_t);
                        bus.frame_t = (bus.frame_t + irq_t) % FRAME_TSTATES_48;
                        return;
                    }
                }
                let last_t = cpu.t;
                let mut mio = MemIo48 { bus: bus.as_mut() };
                cpu.step(&mut mio);
                let dt = (cpu.t - last_t) as u32;
                Self::advance_tape_ear(tape, &mut bus.ear, dt);
                bus.frame_t = (bus.frame_t + dt) % FRAME_TSTATES_48;
            }
            Self::Spec128 { cpu, bus, tape, .. } => {
                if Self::try_flash_load_128(cpu, bus, tape) {
                    return;
                }
                if bus.frame_t < INT_LENGTH_128 {
                    let mut mio = MemIo128 { bus: bus.as_mut() };
                    let irq_t = cpu.interrupt(&mut mio);
                    if irq_t > 0 {
                        Self::advance_tape_ear(tape, &mut bus.ear, irq_t);
                        bus.ay.advance(irq_t);
                        bus.frame_t = (bus.frame_t + irq_t) % FRAME_TSTATES_128;
                        return;
                    }
                }
                let last_t = cpu.t;
                let mut mio = MemIo128 { bus: bus.as_mut() };
                cpu.step(&mut mio);
                let dt = (cpu.t - last_t) as u32;
                Self::advance_tape_ear(tape, &mut bus.ear, dt);
                bus.ay.advance(dt);
                bus.frame_t = (bus.frame_t + dt) % FRAME_TSTATES_128;
            }
            Self::SpecPlus3 { cpu, bus, tape, .. } => {
                if Self::try_flash_load_plus3(cpu, bus, tape) {
                    return;
                }
                if bus.frame_t < INT_LENGTH_128 {
                    let mut mio = MemIoPlus3 { bus: bus.as_mut() };
                    let irq_t = cpu.interrupt(&mut mio);
                    if irq_t > 0 {
                        Self::advance_tape_ear(tape, &mut bus.ear, irq_t);
                        bus.ay.advance(irq_t);
                        bus.frame_t = (bus.frame_t + irq_t) % FRAME_TSTATES_128;
                        return;
                    }
                }
                let last_t = cpu.t;
                let mut mio = MemIoPlus3 { bus: bus.as_mut() };
                cpu.step(&mut mio);
                let dt = (cpu.t - last_t) as u32;
                Self::advance_tape_ear(tape, &mut bus.ear, dt);
                bus.ay.advance(dt);
                bus.frame_t = (bus.frame_t + dt) % FRAME_TSTATES_128;
            }
        }
    }

    /// One CPU instruction without IRQ or tape flash-load traps (for hosted tests).
    pub fn step_cpu_only(&mut self) {
        match self {
            Self::Spec48 { cpu, bus, tape, .. } => {
                let last_t = cpu.t;
                let mut mio = MemIo48 { bus: bus.as_mut() };
                cpu.step(&mut mio);
                let dt = (cpu.t - last_t) as u32;
                Self::advance_tape_ear(tape, &mut bus.ear, dt);
                bus.frame_t = (bus.frame_t + dt) % FRAME_TSTATES_48;
            }
            Self::Spec128 { cpu, bus, tape, .. } => {
                let last_t = cpu.t;
                let mut mio = MemIo128 { bus: bus.as_mut() };
                cpu.step(&mut mio);
                let dt = (cpu.t - last_t) as u32;
                Self::advance_tape_ear(tape, &mut bus.ear, dt);
                bus.ay.advance(dt);
                bus.frame_t = (bus.frame_t + dt) % FRAME_TSTATES_128;
            }
            Self::SpecPlus3 { cpu, bus, tape, .. } => {
                let last_t = cpu.t;
                let mut mio = MemIoPlus3 { bus: bus.as_mut() };
                cpu.step(&mut mio);
                let dt = (cpu.t - last_t) as u32;
                Self::advance_tape_ear(tape, &mut bus.ear, dt);
                bus.ay.advance(dt);
                bus.frame_t = (bus.frame_t + dt) % FRAME_TSTATES_128;
            }
        }
    }

    /// Simulate `RET` (pop PC from stack).
    pub fn ret(&mut self) {
        let sp = self.cpu().regs.sp;
        let lo = self.read_mem(sp);
        let hi = self.read_mem(sp.wrapping_add(1));
        self.cpu_mut().regs.sp = sp.wrapping_add(2);
        self.cpu_mut().regs.pc = u16::from_le_bytes([lo, hi]);
    }

    /// Push `addr` onto the stack (as a CALL would).
    pub fn push_word(&mut self, addr: u16) {
        let sp = self.cpu().regs.sp.wrapping_sub(2);
        self.cpu_mut().regs.sp = sp;
        self.write_mem(sp, (addr & 0xff) as u8);
        self.write_mem(sp.wrapping_add(1), (addr >> 8) as u8);
    }

    pub fn render_rgba(&self, out: &mut [u8], with_border: bool) {
        match self {
            Self::Spec48 { bus, .. } => {
                bus.ula.render_rgba(bus.screen_bytes(), out, with_border);
            }
            Self::Spec128 { bus, .. } => {
                bus.ula.render_rgba_timed(
                    bus.screen_bytes(),
                    out,
                    with_border,
                    ula::PAPER_START_128,
                    ula::T_LINE_128,
                );
            }
            Self::SpecPlus3 { bus, .. } => {
                bus.ula.render_rgba_timed(
                    bus.screen_bytes(),
                    out,
                    with_border,
                    ula::PAPER_START_128,
                    ula::T_LINE_128,
                );
            }
        }
    }

    pub fn keyboard_mut(&mut self) -> &mut bus::Keyboard {
        match self {
            Self::Spec48 { bus, .. } => &mut bus.keyboard,
            Self::Spec128 { bus, .. } => &mut bus.keyboard,
            Self::SpecPlus3 { bus, .. } => &mut bus.keyboard,
        }
    }

    pub fn set_ear(&mut self, level: bool) {
        match self {
            Self::Spec48 { bus, .. } => bus.ear = level,
            Self::Spec128 { bus, .. } => bus.ear = level,
            Self::SpecPlus3 { bus, .. } => bus.ear = level,
        }
    }

    #[must_use]
    pub fn cpu(&self) -> &Cpu {
        match self {
            Self::Spec48 { cpu, .. } | Self::Spec128 { cpu, .. } | Self::SpecPlus3 { cpu, .. } => {
                cpu
            }
        }
    }

    pub fn cpu_mut(&mut self) -> &mut Cpu {
        match self {
            Self::Spec48 { cpu, .. } | Self::Spec128 { cpu, .. } | Self::SpecPlus3 { cpu, .. } => {
                cpu
            }
        }
    }

    pub fn write_mem(&mut self, addr: u16, value: u8) {
        match self {
            Self::Spec48 { bus, .. } => bus.write(addr, value),
            Self::Spec128 { bus, .. } => bus.write(addr, value),
            Self::SpecPlus3 { bus, .. } => bus.write(addr, value),
        }
    }

    #[must_use]
    pub fn read_mem(&self, addr: u16) -> u8 {
        match self {
            Self::Spec48 { bus, .. } => bus.read(addr),
            Self::Spec128 { bus, .. } => bus.read(addr),
            Self::SpecPlus3 { bus, .. } => bus.read(addr),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tape::TapImage;
    use ula::{FRAME_TSTATES_48, INT_LENGTH_48};

    fn rom48() -> Option<Vec<u8>> {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/spec48.rom");
        std::fs::read(p).ok()
    }

    fn fixture_tap() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/tape/minimal_code.tap")
    }

    #[test]
    fn boot_frames_advance() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let mut m = Machine::new_48k(&rom).unwrap();
        for _ in 0..10 {
            m.run_frame();
        }
        assert!(m.cpu().t > 0);
    }

    #[test]
    fn tape_ear_toggles_when_player_inserted() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let img = TapImage::load(&fixture_tap()).expect("fixture");
        let mut m = Machine::new_48k(&rom).unwrap();
        assert!(!m.ear(), "EAR idle without tape");
        m.insert_tape(TapPlayer::new(img));
        // Pilot starts high; first instruction group must raise EAR.
        let _ = m.run_frame();
        // Parallel probe advanced by one frame of T-states should share block index.
        let mut probe = TapPlayer::new(TapImage::load(&fixture_tap()).expect("fixture"));
        probe.advance(FRAME_TSTATES_48);
        assert_eq!(m.tape_block(), Some(probe.block));
        // EAR must leave the idle-low power-on default during pilot.
        let mut saw_high = m.ear();
        for _ in 0..5 {
            let _ = m.run_frame();
            if m.ear() {
                saw_high = true;
                break;
            }
        }
        assert!(saw_high, "pilot tone must drive EAR high");
    }

    #[test]
    fn tape_tracks_frame_tstates() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let img = TapImage {
            blocks: vec![vec![0x00]],
        };
        let mut m = Machine::new_48k(&rom).unwrap();
        m.insert_tape(TapPlayer::new(img.clone()));
        let mut probe = TapPlayer::new(img);
        for _ in 0..10 {
            m.run_frame();
            probe.advance(FRAME_TSTATES_48);
            assert_eq!(
                m.tape_block(),
                Some(probe.block),
                "tape must advance ~FRAME_TSTATES per frame (not 1 T/instruction)"
            );
        }
    }

    #[test]
    fn flash_load_trap_loads_data_block() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let img = TapImage::load(&fixture_tap()).expect("fixture");
        let data = img.blocks[1].clone();
        let mut m = Machine::new_48k(&rom).unwrap();
        // Skip header block so trap sees the data block.
        let mut player = TapPlayer::new(img);
        player.consume_block();
        m.insert_tape(player);

        // Set up a fake CALL return address and LD-BYTES register state.
        let ret = 0x1234u16;
        m.cpu_mut().regs.sp = 0x5f00;
        m.write_mem(0x5f00, (ret & 0xff) as u8);
        m.write_mem(0x5f01, (ret >> 8) as u8);
        m.cpu_mut().regs.pc = LD_BYTES_TRAP_PC;
        m.cpu_mut().regs.a = 0xff;
        m.cpu_mut().regs.f |= flag::C;
        m.cpu_mut().regs.set_ix(0x8000);
        m.cpu_mut().regs.set_de((data.len() - 2) as u16);

        // Avoid IRQ at frame_t=0 stealing control before the trap runs.
        if let Machine::Spec48 { bus, .. } = &mut m {
            bus.frame_t = INT_LENGTH_48;
        }
        m.step_once();

        assert_eq!(m.cpu().regs.pc, ret, "trap should RET");
        assert_eq!(m.read_mem(0x8000), 0x21);
        assert_eq!(m.read_mem(0x8001), 0x00);
        assert_eq!(m.read_mem(0x8002), 0x40);
        assert_eq!(m.read_mem(0x8003), 0x36);
        assert_eq!(m.read_mem(0x8004), 0x42);
        assert_eq!(m.read_mem(0x8005), 0xc9);
        assert_eq!(m.tape_block(), Some(2));
    }

    fn rom128() -> Option<Vec<u8>> {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/128/spec128uk.rom");
        std::fs::read(p).ok()
    }

    #[test]
    fn ay_frame_audio_nonzero_when_tone_programmed() {
        let Some(rom) = rom128() else {
            eprintln!("skip: roms/128/spec128uk.rom missing");
            return;
        };
        let mut m = Machine::new_128k(&rom).unwrap();
        // Program AY tone A via ports
        if let Machine::Spec128 { bus, .. } = &mut m {
            bus.out_port(0xfffd, 0); // select R0
            bus.out_port(0xbffd, 16); // fine
            bus.out_port(0xfffd, 1);
            bus.out_port(0xbffd, 0); // coarse
            bus.out_port(0xfffd, 8);
            bus.out_port(0xbffd, 0x0f); // volume
            bus.out_port(0xfffd, 7);
            bus.out_port(0xbffd, 0x38); // tone A only
        }
        let audio = m.run_frame();
        assert!(!audio.ay_samples.is_empty());
        let energy: f32 = audio.ay_samples.iter().map(|s| s * s).sum();
        assert!(
            energy > 0.01,
            "AY tone should produce frame audio energy, got {energy}"
        );
    }

    fn rom_plus3() -> Option<Vec<u8>> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms");
        for rel in ["plus3/plus3.rom", "plus2a/plus2a.rom"] {
            if let Ok(data) = std::fs::read(root.join(rel)) {
                return Some(data);
            }
        }
        None
    }

    #[test]
    fn plus3_boots_and_1ffd_special_maps() {
        let Some(rom) = rom_plus3() else {
            eprintln!("skip: plus3/plus2a ROM missing — run ./scripts/fetch_roms.sh");
            return;
        };
        let mut m = Machine::new_plus3(&rom).unwrap();
        assert_eq!(m.model(), Model::SpectrumPlus3);
        m.run_frame();
        if let Machine::SpecPlus3 { bus, .. } = &mut m {
            bus.banks[0][0] = 0x5a;
            bus.out_1ffd(0x01);
            assert_eq!(bus.read(0x0000), 0x5a);
            assert_eq!(bus.in_port(0x00ff), 0xff, "no floating bus");
        }
    }
}
