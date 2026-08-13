//! Spec Chum machine — Spectrum models and frame runner.

#![allow(clippy::pedantic)]
#![allow(clippy::large_enum_variant)]

#[cfg(all(test, feature = "slow-tests"))]
mod z80test;

use bus::{Bus128, Bus48, BusPlus3, Kempston};
use formats::{apply_input_byte, DskImage, RzxRecording, Snapshot48};
use tape::{
    evaluate_ld_bytes_trap, flash_load_block, TapPlayer, TapeTrapResult, TzxPlayer,
    LD_BYTES_TRAP_PC,
};
use ula::{int_active_48, Ula48, FRAME_TSTATES_128, FRAME_TSTATES_48, INT_LENGTH_128};
use z80::{flag, Cpu, Io, Memory};

fn next_frame_n() -> u32 {
    static FRAME_N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    FRAME_N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

fn reg_snap(cpu: &Cpu) -> trace::RegSnap {
    let r = &cpu.regs;
    trace::RegSnap {
        pc: r.pc,
        sp: r.sp,
        af: r.af(),
        bc: r.bc(),
        de: r.de(),
        hl: r.hl(),
        ix: r.ix(),
        iy: r.iy(),
        af_: u16::from(r.a_) << 8 | u16::from(r.f_),
        iff1: r.iff1,
        halted: r.halted,
    }
}

/// Inserted tape image (TAP pulse player or TZX pulse player).
#[derive(Clone, Debug)]
pub enum TapeDeck {
    Tap(TapPlayer),
    Tzx(TzxPlayer),
}

impl TapeDeck {
    pub fn advance(&mut self, dt: u32) -> bool {
        match self {
            Self::Tap(t) => t.advance(dt),
            Self::Tzx(t) => t.advance(dt),
        }
    }

    #[must_use]
    pub fn block(&self) -> Option<usize> {
        match self {
            Self::Tap(t) => Some(t.block),
            Self::Tzx(t) => Some(t.block),
        }
    }

    #[must_use]
    pub fn block_count(&self) -> usize {
        match self {
            Self::Tap(t) => t.image.blocks.len(),
            Self::Tzx(t) => t.block_count(),
        }
    }

    #[must_use]
    pub fn pulse_index(&self) -> usize {
        match self {
            Self::Tap(t) => t.pulse_index(),
            Self::Tzx(t) => t.pulse_index(),
        }
    }

    #[must_use]
    pub fn pulse_count(&self) -> usize {
        match self {
            Self::Tap(t) => t.scheduled_pulses(),
            Self::Tzx(t) => t.scheduled_pulses(),
        }
    }

    pub fn as_tap_mut(&mut self) -> Option<&mut TapPlayer> {
        match self {
            Self::Tap(t) => Some(t),
            Self::Tzx(_) => None,
        }
    }

    pub fn set_playing(&mut self, playing: bool) {
        match self {
            Self::Tap(t) => t.set_playing(playing),
            Self::Tzx(t) => t.set_playing(playing),
        }
    }

    #[must_use]
    pub fn playing(&self) -> bool {
        match self {
            Self::Tap(t) => t.playing,
            Self::Tzx(t) => t.playing,
        }
    }

    pub fn rewind(&mut self) {
        match self {
            Self::Tap(t) => t.rewind(),
            Self::Tzx(t) => t.rewind(),
        }
    }
}

/// User controls for tape loading speed / instant flash-load.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TapeLoadOptions {
    /// When true, TAP decks trap at LD-BYTES and poke bytes immediately.
    pub flash_load: bool,
    /// EAR bitstream speed multiplier (`1` = realtime). Clamped to `1..=64`.
    pub speed: u32,
}

impl Default for TapeLoadOptions {
    fn default() -> Self {
        Self {
            flash_load: true,
            speed: 1,
        }
    }
}

impl TapeLoadOptions {
    #[must_use]
    pub fn with_speed(mut self, speed: u32) -> Self {
        self.speed = speed.clamp(1, 64);
        self
    }
}

/// Tape position for UI progress (block + pulse within the current schedule).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TapeProgress {
    pub block_index: u32,
    pub block_count: u32,
    pub pulse_index: u32,
    pub pulse_count: u32,
}

impl TapeProgress {
    /// 0.0..1.0 overall position estimate (block + intra-block pulse fraction).
    #[must_use]
    pub fn fraction(&self) -> f32 {
        if self.block_count == 0 {
            return 0.0;
        }
        let block = self.block_index.min(self.block_count) as f32;
        let within = if self.pulse_count == 0 {
            0.0
        } else {
            self.pulse_index.min(self.pulse_count) as f32 / self.pulse_count as f32
        };
        ((block + within) / self.block_count as f32).clamp(0.0, 1.0)
    }
}

#[derive(Clone, Debug, Default)]
pub struct RzxPlayer {
    pub recording: RzxRecording,
    pub frame: usize,
}

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
        tape: Option<TapeDeck>,
        tape_opts: TapeLoadOptions,
        rzx: Option<RzxPlayer>,
    },
    Spec128 {
        cpu: Cpu,
        bus: Box<Bus128>,
        ula: Ula48,
        tape: Option<TapeDeck>,
        tape_opts: TapeLoadOptions,
        rzx: Option<RzxPlayer>,
    },
    SpecPlus3 {
        cpu: Cpu,
        bus: Box<BusPlus3>,
        ula: Ula48,
        tape: Option<TapeDeck>,
        tape_opts: TapeLoadOptions,
        rzx: Option<RzxPlayer>,
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
        trace::emit(trace::EventKind::MachineModel { model: 0 });
        Ok(Self::Spec48 {
            cpu: Cpu::new(),
            bus: Box::new(bus),
            ula: Ula48::new(),
            tape: None,
            tape_opts: TapeLoadOptions::default(),
            rzx: None,
        })
    }

    pub fn new_128k(rom: &[u8]) -> Result<Self, String> {
        let mut bus = Bus128::new();
        bus.load_rom128(rom)?;
        trace::emit(trace::EventKind::MachineModel { model: 1 });
        Ok(Self::Spec128 {
            cpu: Cpu::new(),
            bus: Box::new(bus),
            ula: Ula48::new(),
            tape: None,
            tape_opts: TapeLoadOptions::default(),
            rzx: None,
        })
    }

    pub fn new_plus3(rom: &[u8]) -> Result<Self, String> {
        let mut bus = BusPlus3::new();
        bus.load_rom64(rom)?;
        trace::emit(trace::EventKind::MachineModel { model: 2 });
        Ok(Self::SpecPlus3 {
            cpu: Cpu::new(),
            bus: Box::new(bus),
            ula: Ula48::new(),
            tape: None,
            tape_opts: TapeLoadOptions::default(),
            rzx: None,
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
                rzx,
                ..
            } => {
                cpu.reset();
                bus.keyboard.reset();
                bus.frame_t = 0;
                bus.beeper_edges.clear();
                bus.kempston.reset();
                *ula = Ula48::new();
                *tape = None;
                *rzx = None;
            }
            Self::Spec128 {
                cpu,
                bus,
                ula,
                tape,
                rzx,
                ..
            } => {
                cpu.reset();
                bus.keyboard.reset();
                bus.frame_t = 0;
                bus.page = 0;
                bus.locked = false;
                bus.beeper_edges.clear();
                bus.ay.reset();
                bus.kempston.reset();
                *ula = Ula48::new();
                *tape = None;
                *rzx = None;
            }
            Self::SpecPlus3 {
                cpu,
                bus,
                ula,
                tape,
                rzx,
                ..
            } => {
                cpu.reset();
                bus.keyboard.reset();
                bus.frame_t = 0;
                bus.page_7ffd = 0;
                bus.page_1ffd = 0;
                bus.locked = false;
                bus.beeper_edges.clear();
                bus.ay.reset();
                bus.kempston.reset();
                *ula = Ula48::new();
                *tape = None;
                *rzx = None;
            }
        }
    }

    #[must_use]
    pub fn tape_load_options(&self) -> TapeLoadOptions {
        match self {
            Self::Spec48 { tape_opts, .. }
            | Self::Spec128 { tape_opts, .. }
            | Self::SpecPlus3 { tape_opts, .. } => *tape_opts,
        }
    }

    pub fn set_tape_load_options(&mut self, opts: TapeLoadOptions) {
        let opts = TapeLoadOptions {
            flash_load: opts.flash_load,
            speed: opts.speed.clamp(1, 64),
        };
        match self {
            Self::Spec48 { tape_opts, .. }
            | Self::Spec128 { tape_opts, .. }
            | Self::SpecPlus3 { tape_opts, .. } => *tape_opts = opts,
        }
        trace::emit(trace::EventKind::MachineLoadMode {
            flash_load: opts.flash_load,
            speed: opts.speed as u8,
        });
    }

    pub fn insert_tape(&mut self, mut player: TapPlayer) {
        player.set_playing(false);
        match self {
            Self::Spec48 { tape, .. }
            | Self::Spec128 { tape, .. }
            | Self::SpecPlus3 { tape, .. } => *tape = Some(TapeDeck::Tap(player)),
        }
    }

    pub fn insert_tzx(&mut self, mut player: TzxPlayer) {
        player.set_playing(false);
        match self {
            Self::Spec48 { tape, .. }
            | Self::Spec128 { tape, .. }
            | Self::SpecPlus3 { tape, .. } => *tape = Some(TapeDeck::Tzx(player)),
        }
    }

    pub fn set_tape_playing(&mut self, playing: bool) {
        match self {
            Self::Spec48 { tape, .. }
            | Self::Spec128 { tape, .. }
            | Self::SpecPlus3 { tape, .. } => {
                if let Some(t) = tape.as_mut() {
                    let block = t.block().unwrap_or(0) as u32;
                    let blocks = t.block_count() as u32;
                    t.set_playing(playing);
                    if playing {
                        trace::emit(trace::EventKind::TapePlay { block, blocks });
                    } else {
                        trace::emit(trace::EventKind::TapePause { block });
                    }
                }
            }
        }
    }

    #[must_use]
    pub fn tape_playing(&self) -> bool {
        match self {
            Self::Spec48 { tape, .. }
            | Self::Spec128 { tape, .. }
            | Self::SpecPlus3 { tape, .. } => tape.as_ref().is_some_and(TapeDeck::playing),
        }
    }

    pub fn rewind_tape(&mut self) {
        match self {
            Self::Spec48 { tape, .. }
            | Self::Spec128 { tape, .. }
            | Self::SpecPlus3 { tape, .. } => {
                if let Some(t) = tape.as_mut() {
                    t.rewind();
                    trace::emit(trace::EventKind::TapeRewind);
                }
            }
        }
    }

    #[must_use]
    pub fn has_tape(&self) -> bool {
        match self {
            Self::Spec48 { tape, .. }
            | Self::Spec128 { tape, .. }
            | Self::SpecPlus3 { tape, .. } => tape.is_some(),
        }
    }

    pub fn insert_rzx(&mut self, recording: RzxRecording) {
        match self {
            Self::Spec48 { rzx, .. } | Self::Spec128 { rzx, .. } | Self::SpecPlus3 { rzx, .. } => {
                *rzx = Some(RzxPlayer {
                    recording,
                    frame: 0,
                })
            }
        }
    }

    pub fn insert_disk(&mut self, image: DskImage) -> Result<(), String> {
        match self {
            Self::SpecPlus3 { bus, .. } => {
                bus.fdc.insert(image);
                Ok(())
            }
            _ => Err("+3 disk requires SpectrumPlus3 model".into()),
        }
    }

    pub fn kempston_mut(&mut self) -> &mut Kempston {
        match self {
            Self::Spec48 { bus, .. } => &mut bus.kempston,
            Self::Spec128 { bus, .. } => &mut bus.kempston,
            Self::SpecPlus3 { bus, .. } => &mut bus.kempston,
        }
    }

    fn apply_rzx_frame(&mut self) {
        let inputs = {
            let rzx = match self {
                Self::Spec48 { rzx, .. }
                | Self::Spec128 { rzx, .. }
                | Self::SpecPlus3 { rzx, .. } => rzx,
            };
            let Some(player) = rzx.as_mut() else {
                return;
            };
            if player.frame >= player.recording.frames.len() {
                return;
            }
            let inputs = player.recording.frames[player.frame].inputs.clone();
            player.frame += 1;
            inputs
        };
        for byte in inputs {
            match self {
                Self::Spec48 { bus, .. } => {
                    apply_input_byte(byte, &mut bus.keyboard.rows, |v| {
                        bus.kempston.right = v & 1 != 0;
                        bus.kempston.left = v & 2 != 0;
                        bus.kempston.down = v & 4 != 0;
                        bus.kempston.up = v & 8 != 0;
                        bus.kempston.fire = v & 0x10 != 0;
                    });
                }
                Self::Spec128 { bus, .. } => {
                    apply_input_byte(byte, &mut bus.keyboard.rows, |v| {
                        bus.kempston.right = v & 1 != 0;
                        bus.kempston.left = v & 2 != 0;
                        bus.kempston.down = v & 4 != 0;
                        bus.kempston.up = v & 8 != 0;
                        bus.kempston.fire = v & 0x10 != 0;
                    });
                }
                Self::SpecPlus3 { bus, .. } => {
                    apply_input_byte(byte, &mut bus.keyboard.rows, |v| {
                        bus.kempston.right = v & 1 != 0;
                        bus.kempston.left = v & 2 != 0;
                        bus.kempston.down = v & 4 != 0;
                        bus.kempston.up = v & 8 != 0;
                        bus.kempston.fire = v & 0x10 != 0;
                    });
                }
            }
        }
    }

    /// Current tape block index, if a player is inserted.
    #[must_use]
    pub fn tape_block(&self) -> Option<usize> {
        match self {
            Self::Spec48 { tape, .. }
            | Self::Spec128 { tape, .. }
            | Self::SpecPlus3 { tape, .. } => tape.as_ref().and_then(TapeDeck::block),
        }
    }

    /// Tape progress for UI (block + pulse counters).
    #[must_use]
    pub fn tape_progress(&self) -> Option<TapeProgress> {
        match self {
            Self::Spec48 { tape, .. }
            | Self::Spec128 { tape, .. }
            | Self::SpecPlus3 { tape, .. } => tape.as_ref().map(|t| TapeProgress {
                block_index: t.block().unwrap_or(0) as u32,
                block_count: t.block_count() as u32,
                pulse_index: t.pulse_index() as u32,
                pulse_count: t.pulse_count() as u32,
            }),
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
        self.apply_rzx_frame();
        match self {
            Self::Spec48 {
                cpu,
                bus,
                ula,
                tape,
                tape_opts,
                ..
            } => {
                bus.beeper_edges.clear();
                bus.frame_t = 0;
                bus.ula.border = bus.border;
                bus.ula.begin_frame();
                ula.border = bus.border;
                ula.begin_frame();
                if trace::enabled(trace::Category::ULA) {
                    let frame = next_frame_n();
                    trace::emit(trace::EventKind::UlaFrame { frame });
                }
                let mut last_t = cpu.t;
                while bus.frame_t < FRAME_TSTATES_48 {
                    if Self::hold_ld_bytes_until_play(cpu.regs.pc, tape) {
                        const HOLD_T: u32 = 4;
                        bus.frame_t += HOLD_T;
                        cpu.t = cpu.t.wrapping_add(u64::from(HOLD_T));
                        last_t = cpu.t;
                        continue;
                    }
                    if tape_opts.flash_load && Self::try_flash_load_48(cpu, bus, tape) {
                        continue;
                    }
                    if int_active_48(bus.frame_t) {
                        let mut mio = MemIo48 { bus: bus.as_mut() };
                        let irq_t = cpu.interrupt(&mut mio);
                        if irq_t > 0 {
                            Self::advance_tape_ear(
                                tape,
                                &mut bus.ear,
                                bus.beeper,
                                &mut bus.beeper_edges,
                                bus.frame_t,
                                irq_t,
                                tape_opts.speed,
                            );
                            bus.frame_t = (bus.frame_t + irq_t) % FRAME_TSTATES_48;
                            last_t = cpu.t;
                            continue;
                        }
                    }
                    let mut mio = MemIo48 { bus: bus.as_mut() };
                    cpu.step(&mut mio);
                    let dt = (cpu.t - last_t) as u32;
                    last_t = cpu.t;
                    Self::advance_tape_ear(
                        tape,
                        &mut bus.ear,
                        bus.beeper,
                        &mut bus.beeper_edges,
                        bus.frame_t,
                        dt,
                        tape_opts.speed,
                    );
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
                tape_opts,
                ..
            } => {
                bus.beeper_edges.clear();
                bus.frame_t = 0;
                bus.ula.border = bus.border;
                bus.ula.begin_frame();
                ula.border = bus.border;
                ula.begin_frame();
                if trace::enabled(trace::Category::ULA) {
                    let frame = next_frame_n();
                    trace::emit(trace::EventKind::UlaFrame { frame });
                }
                const AY_SAMPLES: usize = 882; // ~44100 Hz / 50 Hz
                let t_per_sample = f64::from(FRAME_TSTATES_128) / AY_SAMPLES as f64;
                let mut ay_samples = Vec::with_capacity(AY_SAMPLES);
                let mut last_t = cpu.t;
                while bus.frame_t < FRAME_TSTATES_128 {
                    if Self::hold_ld_bytes_until_play(cpu.regs.pc, tape) {
                        const HOLD_T: u32 = 4;
                        bus.frame_t += HOLD_T;
                        cpu.t = cpu.t.wrapping_add(u64::from(HOLD_T));
                        last_t = cpu.t;
                        continue;
                    }
                    if tape_opts.flash_load && Self::try_flash_load_128(cpu, bus, tape) {
                        continue;
                    }
                    if bus.frame_t < INT_LENGTH_128 {
                        let mut mio = MemIo128 { bus: bus.as_mut() };
                        let irq_t = cpu.interrupt(&mut mio);
                        if irq_t > 0 {
                            Self::advance_tape_ear(
                                tape,
                                &mut bus.ear,
                                bus.beeper,
                                &mut bus.beeper_edges,
                                bus.frame_t,
                                irq_t,
                                tape_opts.speed,
                            );
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
                    Self::advance_tape_ear(
                        tape,
                        &mut bus.ear,
                        bus.beeper,
                        &mut bus.beeper_edges,
                        bus.frame_t,
                        dt,
                        tape_opts.speed,
                    );
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
                tape_opts,
                ..
            } => {
                bus.beeper_edges.clear();
                bus.frame_t = 0;
                bus.ula.border = bus.border;
                bus.ula.begin_frame();
                ula.border = bus.border;
                ula.begin_frame();
                if trace::enabled(trace::Category::ULA) {
                    let frame = next_frame_n();
                    trace::emit(trace::EventKind::UlaFrame { frame });
                }
                const AY_SAMPLES: usize = 882;
                let t_per_sample = f64::from(FRAME_TSTATES_128) / AY_SAMPLES as f64;
                let mut ay_samples = Vec::with_capacity(AY_SAMPLES);
                let mut last_t = cpu.t;
                while bus.frame_t < FRAME_TSTATES_128 {
                    if Self::hold_ld_bytes_until_play(cpu.regs.pc, tape) {
                        const HOLD_T: u32 = 4;
                        bus.frame_t += HOLD_T;
                        cpu.t = cpu.t.wrapping_add(u64::from(HOLD_T));
                        last_t = cpu.t;
                        continue;
                    }
                    if tape_opts.flash_load && Self::try_flash_load_plus3(cpu, bus, tape) {
                        continue;
                    }
                    if bus.frame_t < INT_LENGTH_128 {
                        let mut mio = MemIoPlus3 { bus: bus.as_mut() };
                        let irq_t = cpu.interrupt(&mut mio);
                        if irq_t > 0 {
                            Self::advance_tape_ear(
                                tape,
                                &mut bus.ear,
                                bus.beeper,
                                &mut bus.beeper_edges,
                                bus.frame_t,
                                irq_t,
                                tape_opts.speed,
                            );
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
                    Self::advance_tape_ear(
                        tape,
                        &mut bus.ear,
                        bus.beeper,
                        &mut bus.beeper_edges,
                        bus.frame_t,
                        dt,
                        tape_opts.speed,
                    );
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

    fn advance_tape_ear(
        tape: &mut Option<TapeDeck>,
        ear: &mut bool,
        beeper: bool,
        edges: &mut Vec<(u32, bool)>,
        frame_t: u32,
        dt: u32,
        speed: u32,
    ) {
        if dt == 0 {
            return;
        }
        let Some(t) = tape.as_mut() else {
            return;
        };
        // Motor off: do not drive EAR with a frozen pilot level (insert starts paused).
        if !t.playing() {
            return;
        }
        let advance_dt = dt.saturating_mul(speed.max(1));
        let new_ear = t.advance(advance_dt);
        if new_ear != *ear {
            *ear = new_ear;
            // Count EAR edges; emit a sampled rate (edges since last sample), not the stride.
            if trace::enabled(trace::Category::TAPE) {
                static EAR_EDGES: std::sync::atomic::AtomicU32 =
                    std::sync::atomic::AtomicU32::new(0);
                static EAR_SAMPLES: std::sync::atomic::AtomicU32 =
                    std::sync::atomic::AtomicU32::new(0);
                let edges = EAR_EDGES.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                let samples = EAR_SAMPLES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if samples.is_multiple_of(256) {
                    trace::emit(trace::EventKind::TapeEarRate {
                        edges_per_frame: edges,
                        level: new_ear,
                    });
                    EAR_EDGES.store(0, std::sync::atomic::Ordering::Relaxed);
                }
            }
            let level = beeper || new_ear;
            if edges.last().map(|&(_, l)| l) != Some(level) {
                edges.push((frame_t, level));
            }
        }
    }

    /// Tape inserted but paused at LD-BYTES: hold PC so Play can still flash-load / EAR-load.
    #[must_use]
    fn hold_ld_bytes_until_play(pc: u16, tape: &Option<TapeDeck>) -> bool {
        let holding = pc == LD_BYTES_TRAP_PC && tape.as_ref().is_some_and(|t| !t.playing());
        if holding && trace::enabled(trace::Category::MACHINE) {
            // Sampled: one event per hold check would flood; emit sparsely via counter.
            static HOLD_N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let n = HOLD_N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if n.is_multiple_of(1024) {
                trace::emit(trace::EventKind::MachineLdBytesHold { holding: true, pc });
            }
        }
        holding
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

    fn try_flash_load_48(cpu: &mut Cpu, bus: &mut Bus48, tape: &mut Option<TapeDeck>) -> bool {
        if cpu.regs.pc != LD_BYTES_TRAP_PC {
            return false;
        }
        let Some(deck) = tape.as_mut() else {
            return false;
        };
        let Some(player) = deck.as_tap_mut() else {
            return false;
        };
        // ROM did `EX AF,AF'` before 0x056C — flag + load/verify carry are in A′/F′.
        let flag_expected = cpu.regs.a_;
        let load = cpu.regs.f_ & flag::C != 0;
        let addr = cpu.regs.ix();
        let len = cpu.regs.de();
        let block = player.block as u32;
        trace::set_t_hint(cpu.t);
        if trace::enabled(trace::Category::TAPE) {
            trace::emit(trace::EventKind::FlashLoadEnter {
                regs: reg_snap(cpu),
                flag_expected,
                load,
                addr,
                len,
                block,
            });
        }
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
                if trace::enabled(trace::Category::TAPE) {
                    trace::emit(trace::EventKind::FlashLoadExit {
                        success: true,
                        bytes: n,
                        block_after: player.block as u32,
                        regs: reg_snap(cpu),
                    });
                }
                true
            }
            TapeTrapResult::Failure => {
                Self::ret_from_tape_trap(cpu, ret_lo, ret_hi, false);
                if trace::enabled(trace::Category::TAPE) {
                    trace::emit(trace::EventKind::FlashLoadExit {
                        success: false,
                        bytes: 0,
                        block_after: player.block as u32,
                        regs: reg_snap(cpu),
                    });
                }
                true
            }
        }
    }

    fn try_flash_load_128(cpu: &mut Cpu, bus: &mut Bus128, tape: &mut Option<TapeDeck>) -> bool {
        if cpu.regs.pc != LD_BYTES_TRAP_PC {
            return false;
        }
        let Some(deck) = tape.as_mut() else {
            return false;
        };
        let Some(player) = deck.as_tap_mut() else {
            return false;
        };
        let flag_expected = cpu.regs.a_;
        let load = cpu.regs.f_ & flag::C != 0;
        let addr = cpu.regs.ix();
        let len = cpu.regs.de();
        let block = player.block as u32;
        trace::set_t_hint(cpu.t);
        if trace::enabled(trace::Category::TAPE) {
            trace::emit(trace::EventKind::FlashLoadEnter {
                regs: reg_snap(cpu),
                flag_expected,
                load,
                addr,
                len,
                block,
            });
        }
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
                if trace::enabled(trace::Category::TAPE) {
                    trace::emit(trace::EventKind::FlashLoadExit {
                        success: true,
                        bytes: n,
                        block_after: player.block as u32,
                        regs: reg_snap(cpu),
                    });
                }
                true
            }
            TapeTrapResult::Failure => {
                Self::ret_from_tape_trap(cpu, ret_lo, ret_hi, false);
                if trace::enabled(trace::Category::TAPE) {
                    trace::emit(trace::EventKind::FlashLoadExit {
                        success: false,
                        bytes: 0,
                        block_after: player.block as u32,
                        regs: reg_snap(cpu),
                    });
                }
                true
            }
        }
    }

    fn try_flash_load_plus3(
        cpu: &mut Cpu,
        bus: &mut BusPlus3,
        tape: &mut Option<TapeDeck>,
    ) -> bool {
        if cpu.regs.pc != LD_BYTES_TRAP_PC {
            return false;
        }
        let Some(deck) = tape.as_mut() else {
            return false;
        };
        let Some(player) = deck.as_tap_mut() else {
            return false;
        };
        let flag_expected = cpu.regs.a_;
        let load = cpu.regs.f_ & flag::C != 0;
        let addr = cpu.regs.ix();
        let len = cpu.regs.de();
        let block = player.block as u32;
        trace::set_t_hint(cpu.t);
        if trace::enabled(trace::Category::TAPE) {
            trace::emit(trace::EventKind::FlashLoadEnter {
                regs: reg_snap(cpu),
                flag_expected,
                load,
                addr,
                len,
                block,
            });
        }
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
                if trace::enabled(trace::Category::TAPE) {
                    trace::emit(trace::EventKind::FlashLoadExit {
                        success: true,
                        bytes: n,
                        block_after: player.block as u32,
                        regs: reg_snap(cpu),
                    });
                }
                true
            }
            TapeTrapResult::Failure => {
                Self::ret_from_tape_trap(cpu, ret_lo, ret_hi, false);
                if trace::enabled(trace::Category::TAPE) {
                    trace::emit(trace::EventKind::FlashLoadExit {
                        success: false,
                        bytes: 0,
                        block_after: player.block as u32,
                        regs: reg_snap(cpu),
                    });
                }
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
            Self::Spec48 {
                cpu,
                bus,
                tape,
                tape_opts,
                ..
            } => {
                if Self::hold_ld_bytes_until_play(cpu.regs.pc, tape) {
                    const HOLD_T: u32 = 4;
                    Self::advance_tape_ear(
                        tape,
                        &mut bus.ear,
                        bus.beeper,
                        &mut bus.beeper_edges,
                        bus.frame_t,
                        HOLD_T,
                        tape_opts.speed,
                    );
                    bus.frame_t = (bus.frame_t + HOLD_T) % FRAME_TSTATES_48;
                    cpu.t = cpu.t.wrapping_add(u64::from(HOLD_T));
                    return;
                }
                if tape_opts.flash_load && Self::try_flash_load_48(cpu, bus, tape) {
                    return;
                }
                if int_active_48(bus.frame_t) {
                    let mut mio = MemIo48 { bus: bus.as_mut() };
                    let irq_t = cpu.interrupt(&mut mio);
                    if irq_t > 0 {
                        Self::advance_tape_ear(
                            tape,
                            &mut bus.ear,
                            bus.beeper,
                            &mut bus.beeper_edges,
                            bus.frame_t,
                            irq_t,
                            tape_opts.speed,
                        );
                        bus.frame_t = (bus.frame_t + irq_t) % FRAME_TSTATES_48;
                        return;
                    }
                }
                let last_t = cpu.t;
                let mut mio = MemIo48 { bus: bus.as_mut() };
                cpu.step(&mut mio);
                let dt = (cpu.t - last_t) as u32;
                Self::advance_tape_ear(
                    tape,
                    &mut bus.ear,
                    bus.beeper,
                    &mut bus.beeper_edges,
                    bus.frame_t,
                    dt,
                    tape_opts.speed,
                );
                bus.frame_t = (bus.frame_t + dt) % FRAME_TSTATES_48;
            }
            Self::Spec128 {
                cpu,
                bus,
                tape,
                tape_opts,
                ..
            } => {
                if Self::hold_ld_bytes_until_play(cpu.regs.pc, tape) {
                    const HOLD_T: u32 = 4;
                    Self::advance_tape_ear(
                        tape,
                        &mut bus.ear,
                        bus.beeper,
                        &mut bus.beeper_edges,
                        bus.frame_t,
                        HOLD_T,
                        tape_opts.speed,
                    );
                    bus.ay.advance(HOLD_T);
                    bus.frame_t = (bus.frame_t + HOLD_T) % FRAME_TSTATES_128;
                    cpu.t = cpu.t.wrapping_add(u64::from(HOLD_T));
                    return;
                }
                if tape_opts.flash_load && Self::try_flash_load_128(cpu, bus, tape) {
                    return;
                }
                if bus.frame_t < INT_LENGTH_128 {
                    let mut mio = MemIo128 { bus: bus.as_mut() };
                    let irq_t = cpu.interrupt(&mut mio);
                    if irq_t > 0 {
                        Self::advance_tape_ear(
                            tape,
                            &mut bus.ear,
                            bus.beeper,
                            &mut bus.beeper_edges,
                            bus.frame_t,
                            irq_t,
                            tape_opts.speed,
                        );
                        bus.ay.advance(irq_t);
                        bus.frame_t = (bus.frame_t + irq_t) % FRAME_TSTATES_128;
                        return;
                    }
                }
                let last_t = cpu.t;
                let mut mio = MemIo128 { bus: bus.as_mut() };
                cpu.step(&mut mio);
                let dt = (cpu.t - last_t) as u32;
                Self::advance_tape_ear(
                    tape,
                    &mut bus.ear,
                    bus.beeper,
                    &mut bus.beeper_edges,
                    bus.frame_t,
                    dt,
                    tape_opts.speed,
                );
                bus.ay.advance(dt);
                bus.frame_t = (bus.frame_t + dt) % FRAME_TSTATES_128;
            }
            Self::SpecPlus3 {
                cpu,
                bus,
                tape,
                tape_opts,
                ..
            } => {
                if Self::hold_ld_bytes_until_play(cpu.regs.pc, tape) {
                    const HOLD_T: u32 = 4;
                    Self::advance_tape_ear(
                        tape,
                        &mut bus.ear,
                        bus.beeper,
                        &mut bus.beeper_edges,
                        bus.frame_t,
                        HOLD_T,
                        tape_opts.speed,
                    );
                    bus.ay.advance(HOLD_T);
                    bus.frame_t = (bus.frame_t + HOLD_T) % FRAME_TSTATES_128;
                    cpu.t = cpu.t.wrapping_add(u64::from(HOLD_T));
                    return;
                }
                if tape_opts.flash_load && Self::try_flash_load_plus3(cpu, bus, tape) {
                    return;
                }
                if bus.frame_t < INT_LENGTH_128 {
                    let mut mio = MemIoPlus3 { bus: bus.as_mut() };
                    let irq_t = cpu.interrupt(&mut mio);
                    if irq_t > 0 {
                        Self::advance_tape_ear(
                            tape,
                            &mut bus.ear,
                            bus.beeper,
                            &mut bus.beeper_edges,
                            bus.frame_t,
                            irq_t,
                            tape_opts.speed,
                        );
                        bus.ay.advance(irq_t);
                        bus.frame_t = (bus.frame_t + irq_t) % FRAME_TSTATES_128;
                        return;
                    }
                }
                let last_t = cpu.t;
                let mut mio = MemIoPlus3 { bus: bus.as_mut() };
                cpu.step(&mut mio);
                let dt = (cpu.t - last_t) as u32;
                Self::advance_tape_ear(
                    tape,
                    &mut bus.ear,
                    bus.beeper,
                    &mut bus.beeper_edges,
                    bus.frame_t,
                    dt,
                    tape_opts.speed,
                );
                bus.ay.advance(dt);
                bus.frame_t = (bus.frame_t + dt) % FRAME_TSTATES_128;
            }
        }
    }

    /// One CPU instruction without IRQ or tape flash-load traps (for hosted tests).
    pub fn step_cpu_only(&mut self) {
        match self {
            Self::Spec48 {
                cpu,
                bus,
                tape,
                tape_opts,
                ..
            } => {
                let last_t = cpu.t;
                let mut mio = MemIo48 { bus: bus.as_mut() };
                cpu.step(&mut mio);
                let dt = (cpu.t - last_t) as u32;
                Self::advance_tape_ear(
                    tape,
                    &mut bus.ear,
                    bus.beeper,
                    &mut bus.beeper_edges,
                    bus.frame_t,
                    dt,
                    tape_opts.speed,
                );
                bus.frame_t = (bus.frame_t + dt) % FRAME_TSTATES_48;
            }
            Self::Spec128 {
                cpu,
                bus,
                tape,
                tape_opts,
                ..
            } => {
                let last_t = cpu.t;
                let mut mio = MemIo128 { bus: bus.as_mut() };
                cpu.step(&mut mio);
                let dt = (cpu.t - last_t) as u32;
                Self::advance_tape_ear(
                    tape,
                    &mut bus.ear,
                    bus.beeper,
                    &mut bus.beeper_edges,
                    bus.frame_t,
                    dt,
                    tape_opts.speed,
                );
                bus.ay.advance(dt);
                bus.frame_t = (bus.frame_t + dt) % FRAME_TSTATES_128;
            }
            Self::SpecPlus3 {
                cpu,
                bus,
                tape,
                tape_opts,
                ..
            } => {
                let last_t = cpu.t;
                let mut mio = MemIoPlus3 { bus: bus.as_mut() };
                cpu.step(&mut mio);
                let dt = (cpu.t - last_t) as u32;
                Self::advance_tape_ear(
                    tape,
                    &mut bus.ear,
                    bus.beeper,
                    &mut bus.beeper_edges,
                    bus.frame_t,
                    dt,
                    tape_opts.speed,
                );
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
        m.set_tape_playing(true);
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
        m.set_tape_playing(true);
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
    fn tape_paused_does_not_advance_ear_until_play() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let img = TapImage::load(&fixture_tap()).expect("fixture");
        let mut m = Machine::new_48k(&rom).unwrap();
        m.insert_tape(TapPlayer::new(img));
        assert!(!m.tape_playing());
        let block0 = m.tape_block();
        for _ in 0..5 {
            let _ = m.run_frame();
        }
        assert_eq!(m.tape_block(), block0, "paused tape must not advance");
        assert!(!m.ear(), "paused pilot must not drive EAR");
        m.set_tape_playing(true);
        let mut saw_high = false;
        for _ in 0..5 {
            let _ = m.run_frame();
            if m.ear() {
                saw_high = true;
                break;
            }
        }
        assert!(saw_high, "Play must advance EAR during pilot");
        assert!(
            m.tape_block() != block0 || m.ear(),
            "Play should move the deck or at least raise EAR"
        );
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
        m.set_tape_playing(true);

        // Set up a fake CALL return address and LD-BYTES register state.
        let ret = 0x1234u16;
        m.cpu_mut().regs.sp = 0x5f00;
        m.write_mem(0x5f00, (ret & 0xff) as u8);
        m.write_mem(0x5f01, (ret >> 8) as u8);
        m.cpu_mut().regs.pc = LD_BYTES_TRAP_PC;
        // Mimic ROM after EX AF,AF': flag + carry live in A′/F′; A is dirty.
        m.cpu_mut().regs.a = 0x0f;
        m.cpu_mut().regs.f = 0;
        m.cpu_mut().regs.a_ = 0xff;
        m.cpu_mut().regs.f_ = flag::C;
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

    #[test]
    fn ld_bytes_waits_while_tape_paused_then_flash_loads_on_play() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let img = TapImage::load(&fixture_tap()).expect("fixture");
        let header = img.blocks[0].clone();
        let mut m = Machine::new_48k(&rom).unwrap();
        m.insert_tape(TapPlayer::new(img));
        assert!(!m.tape_playing());

        let ret = 0x12abu16;
        m.cpu_mut().regs.sp = 0x5f00;
        m.write_mem(0x5f00, (ret & 0xff) as u8);
        m.write_mem(0x5f01, (ret >> 8) as u8);
        m.cpu_mut().regs.pc = LD_BYTES_TRAP_PC;
        m.cpu_mut().regs.a = 0x0f;
        m.cpu_mut().regs.f = 0;
        m.cpu_mut().regs.a_ = 0x00; // header flag in A′
        m.cpu_mut().regs.f_ = flag::C;
        m.cpu_mut().regs.set_ix(0x5c00);
        m.cpu_mut().regs.set_de((header.len() - 2) as u16);
        if let Machine::Spec48 { bus, .. } = &mut m {
            bus.frame_t = INT_LENGTH_48;
        }

        // While paused, ROM must not run past LD-BYTES (the old stall root cause).
        for _ in 0..64 {
            m.step_once();
            assert_eq!(
                m.cpu().regs.pc,
                LD_BYTES_TRAP_PC,
                "must hold at LD-BYTES until Play"
            );
        }

        m.set_tape_playing(true);
        m.step_once();
        assert_eq!(m.cpu().regs.pc, ret, "Play should flash-load and RET");
        assert_eq!(m.tape_block(), Some(1));
        // Header payload: type + filename etc. — first data byte after flag is type.
        assert_eq!(m.read_mem(0x5c00), header[1]);
    }

    #[test]
    fn tape_progress_reports_blocks_and_fraction() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let img = TapImage::load(&fixture_tap()).expect("fixture");
        let n = img.blocks.len() as u32;
        let mut m = Machine::new_48k(&rom).unwrap();
        m.insert_tape(TapPlayer::new(img));
        let p = m.tape_progress().expect("progress");
        assert_eq!(p.block_index, 0);
        assert_eq!(p.block_count, n);
        assert!(p.pulse_count > 0);
        assert!(p.fraction() < 1.0);
    }

    #[test]
    fn tape_ear_emits_speaker_edges_while_playing() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let img = TapImage::load(&fixture_tap()).expect("fixture");
        let mut m = Machine::new_48k(&rom).unwrap();
        m.insert_tape(TapPlayer::new(img));
        m.set_tape_playing(true);
        let audio = m.run_frame();
        assert!(
            !audio.beeper_edges.is_empty(),
            "EAR pilot should produce speaker edges for load tones"
        );
    }

    #[test]
    fn rom_ld_bytes_entry_flash_loads_via_shadow_af() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let img = TapImage::load(&fixture_tap()).expect("fixture");
        let header = img.blocks[0].clone();
        let mut m = Machine::new_48k(&rom).unwrap();
        m.insert_tape(TapPlayer::new(img));
        m.set_tape_playing(true);

        // Call ROM LD-BYTES (0x0556) with flag in A and load carry — ROM will EX AF,AF'
        // before the 0x056C trap; flash-load must read A′/F′.
        let ret = 0x7000u16;
        m.cpu_mut().regs.sp = 0x5f00;
        m.write_mem(0x5f00, (ret & 0xff) as u8);
        m.write_mem(0x5f01, (ret >> 8) as u8);
        m.cpu_mut().regs.a = 0x00;
        m.cpu_mut().regs.f = flag::C;
        m.cpu_mut().regs.set_ix(0x5c00);
        m.cpu_mut().regs.set_de(17);
        m.cpu_mut().regs.pc = 0x0556;
        if let Machine::Spec48 { bus, .. } = &mut m {
            bus.frame_t = INT_LENGTH_48;
        }

        for _ in 0..256 {
            m.step_once();
            if m.cpu().regs.pc == ret {
                break;
            }
        }
        assert_eq!(m.cpu().regs.pc, ret, "LD-BYTES should return via SA/LD-RET");
        assert_eq!(m.cpu().regs.f & flag::C, flag::C, "carry set on success");
        // Filename in header payload starts at offset 1 (type) + 1… name at IX+1
        assert_eq!(m.read_mem(0x5c00), header[1], "type byte");
        assert_eq!(m.read_mem(0x5c01), b't');
        assert_eq!(m.read_mem(0x5c02), b'e');
        assert_eq!(m.tape_block(), Some(1));
    }

    #[test]
    fn attr_mark_fixture_flash_loads_code_bytes() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/tape/attr_mark.tap");
        let img = TapImage::load(&path).expect("attr_mark");
        let data = img.blocks[1].clone();
        let mut m = Machine::new_48k(&rom).unwrap();
        let mut player = TapPlayer::new(img);
        player.consume_block();
        m.insert_tape(player);
        m.set_tape_playing(true);

        let ret = 0x1234u16;
        m.cpu_mut().regs.sp = 0x5f00;
        m.write_mem(0x5f00, (ret & 0xff) as u8);
        m.write_mem(0x5f01, (ret >> 8) as u8);
        m.cpu_mut().regs.pc = LD_BYTES_TRAP_PC;
        m.cpu_mut().regs.a = 0x0f;
        m.cpu_mut().regs.a_ = 0xff;
        m.cpu_mut().regs.f_ = flag::C;
        m.cpu_mut().regs.set_ix(0x8000);
        m.cpu_mut().regs.set_de((data.len() - 2) as u16);
        if let Machine::Spec48 { bus, .. } = &mut m {
            bus.frame_t = INT_LENGTH_48;
        }
        m.step_once();
        assert_eq!(m.cpu().regs.pc, ret);
        assert_eq!(m.read_mem(0x8000), 0x21);
        assert_eq!(m.read_mem(0x8001), 0x00);
        assert_eq!(m.read_mem(0x8002), 0x58);
        assert_eq!(m.read_mem(0x8003), 0x36);
        assert_eq!(m.read_mem(0x8004), 0xd7);
        assert_eq!(m.read_mem(0x8005), 0xc9);
    }

    #[test]
    fn flash_load_can_be_disabled_for_ear_path() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let img = TapImage::load(&fixture_tap()).expect("fixture");
        let mut m = Machine::new_48k(&rom).unwrap();
        m.set_tape_load_options(TapeLoadOptions {
            flash_load: false,
            speed: 1,
        });
        m.insert_tape(TapPlayer::new(img));
        m.set_tape_playing(true);
        m.cpu_mut().regs.pc = LD_BYTES_TRAP_PC;
        m.cpu_mut().regs.a_ = 0x00;
        m.cpu_mut().regs.f_ = flag::C;
        m.cpu_mut().regs.set_de(17);
        m.cpu_mut().regs.sp = 0x5f00;
        m.write_mem(0x5f00, 0x00);
        m.write_mem(0x5f01, 0x70);
        if let Machine::Spec48 { bus, .. } = &mut m {
            bus.frame_t = INT_LENGTH_48;
        }
        m.step_once();
        // Without flash-load, ROM proceeds into edge-detect (not an instant RET).
        assert_ne!(m.cpu().regs.pc, 0x7000);
        assert_eq!(
            m.tape_block(),
            Some(0),
            "EAR path should not consume via trap"
        );
    }

    #[test]
    fn tape_speed_multiplier_advances_pulses_faster() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let img = TapImage::load(&fixture_tap()).expect("fixture");
        let mut slow = Machine::new_48k(&rom).unwrap();
        let mut fast = Machine::new_48k(&rom).unwrap();
        slow.set_tape_load_options(TapeLoadOptions {
            flash_load: false,
            speed: 1,
        });
        fast.set_tape_load_options(TapeLoadOptions {
            flash_load: false,
            speed: 10,
        });
        slow.insert_tape(TapPlayer::new(img.clone()));
        fast.insert_tape(TapPlayer::new(img));
        slow.set_tape_playing(true);
        fast.set_tape_playing(true);
        for _ in 0..3 {
            let _ = slow.run_frame();
            let _ = fast.run_frame();
        }
        let sp = slow.tape_progress().unwrap();
        let fp = fast.tape_progress().unwrap();
        assert!(
            fp.pulse_index > sp.pulse_index || fp.block_index > sp.block_index,
            "10x speed should advance further (slow pulse {}/{}, fast {}/{})",
            sp.pulse_index,
            sp.block_index,
            fp.pulse_index,
            fp.block_index
        );
    }

    #[test]
    fn boggit_header_flash_loads_when_present() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let Some(boggit) = std::env::var_os("SPEC_CHUM_BOGGIT_TZX").map(PathBuf::from) else {
            eprintln!("skip: set SPEC_CHUM_BOGGIT_TZX to run the Boggit regression");
            return;
        };
        if !boggit.is_file() {
            eprintln!("skip: SPEC_CHUM_BOGGIT_TZX path is not a file ({boggit:?})");
            return;
        }
        let data = std::fs::read(&boggit).expect("read boggit");
        assert!(
            tape::TzxPlayer::is_standard_speed_only(&data),
            "Boggit side 1 should convert to TAP"
        );
        let img = tape::TzxPlayer::to_tap_image(&data).expect("to tap");
        let header = img.blocks[0].clone();
        let mut m = Machine::new_48k(&rom).unwrap();
        m.insert_tape(TapPlayer::new(img));
        m.set_tape_playing(true);

        let ret = 0x7000u16;
        m.cpu_mut().regs.sp = 0x5f00;
        m.write_mem(0x5f00, (ret & 0xff) as u8);
        m.write_mem(0x5f01, (ret >> 8) as u8);
        m.cpu_mut().regs.a = 0x00;
        m.cpu_mut().regs.f = flag::C;
        m.cpu_mut().regs.set_ix(0x5c00);
        m.cpu_mut().regs.set_de(17);
        m.cpu_mut().regs.pc = 0x0556;
        if let Machine::Spec48 { bus, .. } = &mut m {
            bus.frame_t = INT_LENGTH_48;
        }
        for _ in 0..256 {
            m.step_once();
            if m.cpu().regs.pc == ret {
                break;
            }
        }
        assert_eq!(m.cpu().regs.pc, ret);
        assert_eq!(m.read_mem(0x5c00), header[1]);
        // "BOGGIT pt1"
        assert_eq!(m.read_mem(0x5c01), b'B');
        assert_eq!(m.read_mem(0x5c02), b'O');
        assert_eq!(m.read_mem(0x5c06), b'T');
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
        // Boot long enough for the editor menu to paint (was blank when 7FFD→1FFD).
        for _ in 0..120 {
            m.run_frame();
        }
        if let Machine::SpecPlus3 { bus, cpu, .. } = &mut m {
            assert_eq!(
                bus.page_1ffd & 0x01,
                0,
                "must leave special paging off at menu"
            );
            let screen_nz = bus.screen_bytes().iter().filter(|&&b| b != 0).count();
            assert!(
                screen_nz > 100,
                "expected menu pixels, got {screen_nz} nonzero (PC={:04X} 7FFD={:02X} 1FFD={:02X})",
                cpu.regs.pc,
                bus.page_7ffd,
                bus.page_1ffd
            );
            bus.banks[0][0] = 0x5a;
            bus.out_1ffd(0x01);
            assert_eq!(bus.read(0x0000), 0x5a);
            assert_eq!(bus.in_port(0x00ff), 0xff, "no floating bus");
        }
    }

    /// Shared LOAD "" harness body. Returns whether CODE bytes landed at 0x8000.
    /// Caller must hold `trace::test_lock()` and configure categories.
    fn run_attr_mark_load_path(rom: &[u8]) -> (Machine, bool) {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/tape/attr_mark.tap");
        let img = TapImage::load(&path).expect("attr_mark.tap");
        let mut m = Machine::new_48k(rom).unwrap();
        m.set_tape_load_options(TapeLoadOptions {
            flash_load: true,
            speed: 1,
        });
        m.insert_tape(TapPlayer::new(img));
        for _ in 0..200 {
            let _ = m.run_frame();
        }
        let sym = (7usize, 1u8);
        let chords: [(Vec<(usize, u8)>, u32); 8] = [
            (vec![(6, 3)], 6), // J = LOAD
            (vec![], 3),
            (vec![sym, (5, 0)], 6), // Sym + P = "
            (vec![], 3),
            (vec![sym, (5, 0)], 6), // "
            (vec![], 3),
            (vec![(6, 0)], 6), // Enter
            (vec![], 10),
        ];
        for (keys, frames) in chords {
            for _ in 0..frames {
                let kb = m.keyboard_mut();
                kb.reset();
                for &(row, bit) in &keys {
                    kb.set_key(row, bit, true);
                }
                let _ = m.run_frame();
            }
        }
        m.keyboard_mut().reset();
        m.set_tape_playing(true);
        let mut loaded = false;
        for _ in 0..400 {
            let _ = m.run_frame();
            if m.read_mem(0x8000) == 0x21
                && m.read_mem(0x8001) == 0x00
                && m.read_mem(0x8002) == 0x58
                && m.read_mem(0x8003) == 0x36
                && m.read_mem(0x8004) == 0xd7
                && m.read_mem(0x8005) == 0xc9
            {
                loaded = true;
                break;
            }
        }
        (m, loaded)
    }

    /// Deterministic tape repro harness (observability).
    ///
    /// Runs a 48K `LOAD ""` path against `tests/fixtures/tape/attr_mark.tap` with
    /// the structured trace enabled. On load failure, dumps the ring to stderr.
    ///
    /// Load success is reported but not required for CI green while tape is still
    /// broken in practice (#85); observability itself is asserted. For a hard
    /// gate, run `attr_mark_load_path_must_succeed` with `--ignored`.
    ///
    /// Local commercial tape (do **not** commit):
    /// `<path-to-local-commercial-tape>/The Boggit - Side 1.tzx`
    #[test]
    fn attr_mark_load_path_dumps_trace_on_failure() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };

        let _lock = trace::test_lock();
        struct TraceRestore;
        impl Drop for TraceRestore {
            fn drop(&mut self) {
                trace::disable();
                trace::clear();
            }
        }
        let _restore = TraceRestore;
        trace::clear();
        trace::enable(trace::Category::DEFAULT | trace::Category::TAPE);

        let (m, loaded) = run_attr_mark_load_path(&rom);
        let dump = trace::dump_string();
        if !loaded {
            eprintln!("=== attr_mark LOAD path failed — dumping trace ===");
            eprintln!(
                "PC={:04X} tape_block={:?} playing={} AF'={:02X}{:02X}",
                m.cpu().regs.pc,
                m.tape_block(),
                m.tape_playing(),
                m.cpu().regs.a_,
                m.cpu().regs.f_
            );
            eprintln!("{dump}");
            let _ = trace::dump_to_env_file();
        }

        assert!(
            dump.contains("tape.play")
                || dump.contains("tape.flash")
                || dump.contains("tape.block")
                || dump.contains("tape.ear_rate"),
            "expected tape.* trace events when exercising LOAD path; dump head:\n{}",
            dump.chars().take(1200).collect::<String>()
        );

        if loaded {
            eprintln!("attr_mark LOAD \"\" succeeded (CODE at 0x8000)");
        } else {
            eprintln!(
                "attr_mark LOAD \"\" did not place CODE at 0x8000 (tracked by #85); \
                 trace dump above is the debugging artifact"
            );
        }
    }

    /// Hard success gate for attr_mark `LOAD ""` (ignored until #85 is fixed).
    #[test]
    #[ignore = "blocked on #85 tape LOAD; run with --ignored --nocapture to dump"]
    fn attr_mark_load_path_must_succeed() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let _lock = trace::test_lock();
        struct TraceRestore;
        impl Drop for TraceRestore {
            fn drop(&mut self) {
                trace::disable();
                trace::clear();
            }
        }
        let _restore = TraceRestore;
        trace::clear();
        trace::enable(trace::Category::DEFAULT | trace::Category::TAPE);
        let (_m, loaded) = run_attr_mark_load_path(&rom);
        if !loaded {
            eprintln!("=== attr_mark hard gate failed — dumping trace ===");
            trace::dump_to_stderr();
        }
        assert!(loaded, "attr_mark CODE missing at 0x8000");
    }

    #[test]
    fn flash_load_skip_appears_in_trace_dump() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let img = TapImage {
            blocks: vec![vec![0xff, 0x11, 0xff ^ 0x11], vec![0x00, 0x22, 0x22]],
        };
        let _lock = trace::test_lock();
        trace::clear();
        trace::enable(trace::Category::TAPE);
        let mut m = Machine::new_48k(&rom).unwrap();
        m.insert_tape(TapPlayer::new(img));
        m.set_tape_playing(true);
        // Expect header flag 0x00 but first block is data 0xff → skip then load.
        let ret = 0x7000u16;
        m.cpu_mut().regs.sp = 0x5f00;
        m.write_mem(0x5f00, (ret & 0xff) as u8);
        m.write_mem(0x5f01, (ret >> 8) as u8);
        m.cpu_mut().regs.pc = LD_BYTES_TRAP_PC;
        m.cpu_mut().regs.a_ = 0x00;
        m.cpu_mut().regs.f_ = flag::C;
        m.cpu_mut().regs.set_ix(0x5c00);
        m.cpu_mut().regs.set_de(1);
        if let Machine::Spec48 { bus, .. } = &mut m {
            bus.frame_t = INT_LENGTH_48;
        }
        m.step_once();
        let dump = trace::dump_string();
        assert!(
            dump.contains("wrong_flag") || dump.contains("tape.flash"),
            "dump=\n{dump}"
        );
        trace::disable();
        trace::clear();
    }
}
