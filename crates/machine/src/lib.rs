//! Spec Chum machine — Spectrum models and frame runner.

#![allow(clippy::pedantic)]
#![allow(clippy::large_enum_variant)]

#[cfg(test)]
mod z80test;

use bus::{Bus128, Bus48};
use formats::Snapshot48;
use tape::TapPlayer;
use ula::{int_active_48, Ula48, FRAME_TSTATES_128, FRAME_TSTATES_48, INT_LENGTH_128};
use z80::{Cpu, Io, Memory};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Model {
    Spectrum48,
    Spectrum128,
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
        (self.bus.in_port(port), 0)
    }

    fn out_port(&mut self, port: u16, value: u8, _t: u64) -> u32 {
        self.bus.out_port(port, value);
        0
    }
}

#[derive(Clone, Debug)]
pub enum Machine {
    Spec48 {
        cpu: Cpu,
        bus: Bus48,
        ula: Ula48,
        tape: Option<TapPlayer>,
    },
    Spec128 {
        cpu: Cpu,
        bus: Bus128,
        ula: Ula48,
        tape: Option<TapPlayer>,
    },
}

impl Machine {
    pub fn new_48k(rom: &[u8]) -> Result<Self, String> {
        let mut bus = Bus48::new();
        bus.load_rom(rom)?;
        Ok(Self::Spec48 {
            cpu: Cpu::new(),
            bus,
            ula: Ula48::new(),
            tape: None,
        })
    }

    pub fn new_128k(rom: &[u8]) -> Result<Self, String> {
        let mut bus = Bus128::new();
        bus.load_rom128(rom)?;
        Ok(Self::Spec128 {
            cpu: Cpu::new(),
            bus,
            ula: Ula48::new(),
            tape: None,
        })
    }

    #[must_use]
    pub fn model(&self) -> Model {
        match self {
            Self::Spec48 { .. } => Model::Spectrum48,
            Self::Spec128 { .. } => Model::Spectrum128,
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
                *ula = Ula48::new();
                *tape = None;
            }
        }
    }

    pub fn insert_tape(&mut self, player: TapPlayer) {
        match self {
            Self::Spec48 { tape, .. } | Self::Spec128 { tape, .. } => *tape = Some(player),
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
            ula.border = snap.border;
        }
    }

    /// Run one video frame; returns beeper edges for the frame.
    pub fn run_frame(&mut self) -> Vec<(u32, bool)> {
        match self {
            Self::Spec48 {
                cpu,
                bus,
                ula,
                tape,
            } => {
                bus.beeper_edges.clear();
                bus.frame_t = 0;
                let mut last_t = cpu.t;
                while bus.frame_t < FRAME_TSTATES_48 {
                    if let Some(t) = tape.as_mut() {
                        bus.ear = t.advance(1);
                    }
                    if int_active_48(bus.frame_t) {
                        let mut mio = MemIo48 { bus };
                        let irq_t = cpu.interrupt(&mut mio);
                        if irq_t > 0 {
                            bus.frame_t = (bus.frame_t + irq_t) % FRAME_TSTATES_48;
                            last_t = cpu.t;
                            continue;
                        }
                    }
                    let mut mio = MemIo48 { bus };
                    cpu.step(&mut mio);
                    let dt = (cpu.t - last_t) as u32;
                    last_t = cpu.t;
                    bus.frame_t += dt;
                    if bus.frame_t >= FRAME_TSTATES_48 {
                        break;
                    }
                }
                ula.border = bus.border;
                ula.end_frame();
                bus.frame_t = 0;
                std::mem::take(&mut bus.beeper_edges)
            }
            Self::Spec128 {
                cpu,
                bus,
                ula,
                tape,
            } => {
                bus.beeper_edges.clear();
                bus.frame_t = 0;
                let mut last_t = cpu.t;
                while bus.frame_t < FRAME_TSTATES_128 {
                    if let Some(t) = tape.as_mut() {
                        bus.ear = t.advance(1);
                    }
                    if bus.frame_t < INT_LENGTH_128 {
                        let mut mio = MemIo128 { bus };
                        let irq_t = cpu.interrupt(&mut mio);
                        if irq_t > 0 {
                            bus.frame_t = (bus.frame_t + irq_t) % FRAME_TSTATES_128;
                            last_t = cpu.t;
                            continue;
                        }
                    }
                    let mut mio = MemIo128 { bus };
                    cpu.step(&mut mio);
                    let dt = (cpu.t - last_t) as u32;
                    last_t = cpu.t;
                    bus.frame_t += dt;
                    if bus.frame_t >= FRAME_TSTATES_128 {
                        break;
                    }
                }
                ula.border = bus.border;
                ula.end_frame();
                bus.frame_t = 0;
                std::mem::take(&mut bus.beeper_edges)
            }
        }
    }

    pub fn render_rgba(&self, out: &mut [u8], with_border: bool) {
        match self {
            Self::Spec48 { bus, ula, .. } => ula.render_rgba(bus.screen_bytes(), out, with_border),
            Self::Spec128 { bus, ula, .. } => ula.render_rgba(bus.screen_bytes(), out, with_border),
        }
    }

    pub fn keyboard_mut(&mut self) -> &mut bus::Keyboard {
        match self {
            Self::Spec48 { bus, .. } => &mut bus.keyboard,
            Self::Spec128 { bus, .. } => &mut bus.keyboard,
        }
    }

    pub fn set_ear(&mut self, level: bool) {
        match self {
            Self::Spec48 { bus, .. } => bus.ear = level,
            Self::Spec128 { bus, .. } => bus.ear = level,
        }
    }

    #[must_use]
    pub fn cpu(&self) -> &Cpu {
        match self {
            Self::Spec48 { cpu, .. } | Self::Spec128 { cpu, .. } => cpu,
        }
    }

    pub fn cpu_mut(&mut self) -> &mut Cpu {
        match self {
            Self::Spec48 { cpu, .. } | Self::Spec128 { cpu, .. } => cpu,
        }
    }

    pub fn write_mem(&mut self, addr: u16, value: u8) {
        match self {
            Self::Spec48 { bus, .. } => bus.write(addr, value),
            Self::Spec128 { bus, .. } => bus.write(addr, value),
        }
    }

    #[must_use]
    pub fn read_mem(&self, addr: u16) -> u8 {
        match self {
            Self::Spec48 { bus, .. } => bus.read(addr),
            Self::Spec128 { bus, .. } => bus.read(addr),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn rom48() -> Option<Vec<u8>> {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/spec48.rom");
        std::fs::read(p).ok()
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
}
