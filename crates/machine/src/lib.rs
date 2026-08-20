//! Spec Chum machine — Spectrum models and frame runner.

#![allow(clippy::pedantic)]
#![allow(clippy::large_enum_variant)]

#[cfg(all(test, feature = "slow-tests"))]
mod z80test;

mod debugger;
mod inspect;

pub use debugger::{BreakReason, Debugger, Watch};
pub use inspect::{Inspect, Paging, TapeInspect};

use std::cell::Cell;

use bus::{Bus128, Bus48, BusPlus3, Kempston};
use formats::{apply_input_byte, DskImage, RzxRecording, Snapshot48};
pub use tape::LD_BYTES_TRAP_PC;
use tape::{
    evaluate_ld_bytes_trap, flash_load_block, is_ld_bytes_trap_pc, TapPlayer, TapeTrapResult,
    TzxPlayer,
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
        bc_: u16::from(r.b_) << 8 | u16::from(r.c_),
        de_: u16::from(r.d_) << 8 | u16::from(r.e_),
        hl_: u16::from(r.h_) << 8 | u16::from(r.l_),
        i: r.i,
        r: r.r,
        im: r.im,
        memptr: r.memptr,
        iff1: r.iff1,
        iff2: r.iff2,
        halted: r.halted,
    }
}

fn peek_opcode(read: impl Fn(u16) -> u8, pc: u16) -> ([u8; 4], u8) {
    let mut bytes = [0u8; 4];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = read(pc.wrapping_add(i as u16));
    }
    let len = z80::disasm_one(&bytes).len.clamp(1, 4);
    (bytes, len)
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
    /// Shortens TAP leader/pause; data pulse widths stay ROM-accurate so LD-BYTES can lock.
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
    pub(crate) watch: Option<debugger::WatchHook<'a>>,
}

impl Memory for MemIo48<'_> {
    fn read(&mut self, addr: u16, _t: u64) -> (u8, u32) {
        let wait = self.bus.contend_at(addr);
        if wait > 0 && trace::enabled(trace::Category::BUS) {
            emit_contend_sampled(addr, self.bus.frame_t, wait);
        }
        let v = self.bus.read(addr);
        if let Some(w) = self.watch.as_ref() {
            w.mem_access(addr, false, v);
        }
        (v, wait)
    }

    fn write(&mut self, addr: u16, value: u8, _t: u64) -> u32 {
        let wait = self.bus.contend_at(addr);
        if wait > 0 && trace::enabled(trace::Category::BUS) {
            emit_contend_sampled(addr, self.bus.frame_t, wait);
        }
        self.bus.write(addr, value);
        if let Some(w) = self.watch.as_ref() {
            w.mem_access(addr, true, value);
        }
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
        let v = self.bus.in_port(port);
        if let Some(w) = self.watch.as_ref() {
            w.port_access(port, false, v);
        }
        (v, wait)
    }

    fn out_port(&mut self, port: u16, value: u8, _t: u64) -> u32 {
        let wait = if port & 1 == 0 {
            self.bus.contend_at(0x4000)
        } else {
            0
        };
        self.bus.out_port(port, value);
        if let Some(w) = self.watch.as_ref() {
            w.port_access(port, true, value);
        }
        wait
    }
}

#[derive(Debug)]
pub struct MemIo128<'a> {
    pub bus: &'a mut Bus128,
    pub(crate) watch: Option<debugger::WatchHook<'a>>,
}

impl Memory for MemIo128<'_> {
    fn read(&mut self, addr: u16, _t: u64) -> (u8, u32) {
        let wait = self.bus.contend_at(addr);
        if wait > 0 && trace::enabled(trace::Category::BUS) {
            emit_contend_sampled(addr, self.bus.frame_t, wait);
        }
        let v = self.bus.read(addr);
        if let Some(w) = self.watch.as_ref() {
            w.mem_access(addr, false, v);
        }
        (v, wait)
    }

    fn write(&mut self, addr: u16, value: u8, _t: u64) -> u32 {
        let wait = self.bus.contend_at(addr);
        if wait > 0 && trace::enabled(trace::Category::BUS) {
            emit_contend_sampled(addr, self.bus.frame_t, wait);
        }
        self.bus.write(addr, value);
        if let Some(w) = self.watch.as_ref() {
            w.mem_access(addr, true, value);
        }
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
        let v = self.bus.in_port(port);
        if let Some(w) = self.watch.as_ref() {
            w.port_access(port, false, v);
        }
        (v, wait)
    }

    fn out_port(&mut self, port: u16, value: u8, _t: u64) -> u32 {
        let wait = if port & 1 == 0 {
            self.bus.contend_at(0x4000)
        } else {
            0
        };
        self.bus.out_port(port, value);
        if let Some(w) = self.watch.as_ref() {
            w.port_access(port, true, value);
        }
        wait
    }
}

#[derive(Debug)]
pub struct MemIoPlus3<'a> {
    pub bus: &'a mut BusPlus3,
    pub(crate) watch: Option<debugger::WatchHook<'a>>,
}

impl Memory for MemIoPlus3<'_> {
    fn read(&mut self, addr: u16, _t: u64) -> (u8, u32) {
        let wait = self.bus.contend_at(addr);
        if wait > 0 && trace::enabled(trace::Category::BUS) {
            emit_contend_sampled(addr, self.bus.frame_t, wait);
        }
        let v = self.bus.read(addr);
        if let Some(w) = self.watch.as_ref() {
            w.mem_access(addr, false, v);
        }
        (v, wait)
    }

    fn write(&mut self, addr: u16, value: u8, _t: u64) -> u32 {
        let wait = self.bus.contend_at(addr);
        if wait > 0 && trace::enabled(trace::Category::BUS) {
            emit_contend_sampled(addr, self.bus.frame_t, wait);
        }
        self.bus.write(addr, value);
        if let Some(w) = self.watch.as_ref() {
            w.mem_access(addr, true, value);
        }
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
        let v = self.bus.in_port(port);
        if let Some(w) = self.watch.as_ref() {
            w.port_access(port, false, v);
        }
        (v, wait)
    }

    fn out_port(&mut self, port: u16, value: u8, _t: u64) -> u32 {
        let wait = if port & 1 == 0 {
            self.bus.contend_at(0x4000)
        } else {
            0
        };
        self.bus.out_port(port, value);
        if let Some(w) = self.watch.as_ref() {
            w.port_access(port, true, value);
        }
        wait
    }
}

fn emit_contend_sampled(addr: u16, frame_t: u32, wait: u32) {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if n.is_multiple_of(64) {
        trace::emit(trace::EventKind::BusContend {
            addr,
            frame_t,
            wait,
        });
    }
}

fn mem_port_watch<'a>(
    debugger: &'a Debugger,
    hit: &'a Cell<Option<BreakReason>>,
) -> Option<debugger::WatchHook<'a>> {
    if debugger.mem_watches.is_empty() && debugger.port_watches.is_empty() {
        None
    } else {
        Some(debugger::WatchHook {
            mem: &debugger.mem_watches,
            port: &debugger.port_watches,
            hit,
        })
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
        debugger: Debugger,
    },
    Spec128 {
        cpu: Cpu,
        bus: Box<Bus128>,
        ula: Ula48,
        tape: Option<TapeDeck>,
        tape_opts: TapeLoadOptions,
        rzx: Option<RzxPlayer>,
        debugger: Debugger,
    },
    SpecPlus3 {
        cpu: Cpu,
        bus: Box<BusPlus3>,
        ula: Ula48,
        tape: Option<TapeDeck>,
        tape_opts: TapeLoadOptions,
        rzx: Option<RzxPlayer>,
        debugger: Debugger,
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
            debugger: Debugger::default(),
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
            debugger: Debugger::default(),
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
            debugger: Debugger::default(),
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
            Self::Spec48 {
                tape_opts, tape, ..
            }
            | Self::Spec128 {
                tape_opts, tape, ..
            }
            | Self::SpecPlus3 {
                tape_opts, tape, ..
            } => {
                *tape_opts = opts;
                if let Some(TapeDeck::Tap(p)) = tape.as_mut() {
                    p.set_speed(opts.speed);
                }
            }
        }
        trace::emit(trace::EventKind::MachineLoadMode {
            flash_load: opts.flash_load,
            speed: opts.speed as u8,
        });
    }

    pub fn insert_tape(&mut self, mut player: TapPlayer) {
        player.set_playing(false);
        player.set_speed(self.tape_load_options().speed);
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
        let cpu = self.cpu();
        if trace::enabled(trace::Category::MACHINE) {
            trace::emit(trace::EventKind::MachineSnapshot {
                pc: cpu.regs.pc,
                sp: cpu.regs.sp,
                border: snap.border,
            });
        }
    }

    /// Run one video frame; returns beeper edges and AY samples for the frame.
    pub fn run_frame(&mut self) -> FrameAudio {
        if self.debugger().paused {
            return FrameAudio::default();
        }
        self.apply_rzx_frame();
        match self {
            Self::Spec48 {
                cpu,
                bus,
                ula,
                tape,
                tape_opts,
                debugger,
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
                let mut broke_on_pc = false;
                while bus.frame_t < FRAME_TSTATES_48 {
                    if debugger.check_pc(cpu.regs.pc) {
                        broke_on_pc = true;
                        break;
                    }
                    if Self::hold_ld_bytes_until_play(cpu.regs.pc, tape, |a| bus.read(a)) {
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
                        let mut mio = MemIo48 {
                            bus: bus.as_mut(),
                            watch: None,
                        };
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
                                tape_opts.flash_load,
                            );
                            bus.frame_t = (bus.frame_t + irq_t) % FRAME_TSTATES_48;
                            last_t = cpu.t;
                            continue;
                        }
                    }
                    let pc = cpu.regs.pc;
                    let cpu_on = trace::enabled(trace::Category::CPU);
                    let pre = cpu_on.then(|| {
                        let (bytes, len) = peek_opcode(|a| bus.read(a), pc);
                        (bytes, len, reg_snap(cpu), cpu.regs.halted)
                    });
                    let hit = Cell::new(None);
                    {
                        let watch = mem_port_watch(debugger, &hit);
                        let mut mio = MemIo48 {
                            bus: bus.as_mut(),
                            watch,
                        };
                        cpu.step(&mut mio);
                    }
                    let dt = (cpu.t - last_t) as u32;
                    last_t = cpu.t;
                    if let Some((bytes, len, regs, was_halt)) = pre {
                        if was_halt {
                            trace::emit(trace::EventKind::CpuHalt { pc });
                        }
                        trace::emit(trace::EventKind::CpuStep {
                            pc,
                            bytes,
                            len,
                            dt: dt as u16,
                            regs,
                        });
                    }
                    if let Some(reason) = hit.get() {
                        debugger.apply_hit(reason);
                        broke_on_pc = true;
                    }
                    Self::advance_tape_ear(
                        tape,
                        &mut bus.ear,
                        bus.beeper,
                        &mut bus.beeper_edges,
                        bus.frame_t,
                        dt,
                        tape_opts.speed,
                        tape_opts.flash_load,
                    );
                    bus.frame_t += dt;
                    if broke_on_pc || bus.frame_t >= FRAME_TSTATES_48 {
                        break;
                    }
                }
                // Keep border_events for render; next run_frame begin_frame clears them.
                if !broke_on_pc {
                    bus.frame_t = 0;
                }
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
                debugger,
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
                let mut broke_on_pc = false;
                while bus.frame_t < FRAME_TSTATES_128 {
                    if debugger.check_pc(cpu.regs.pc) {
                        broke_on_pc = true;
                        break;
                    }
                    if Self::hold_ld_bytes_until_play(cpu.regs.pc, tape, |a| bus.read(a)) {
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
                        let mut mio = MemIo128 {
                            bus: bus.as_mut(),
                            watch: None,
                        };
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
                                tape_opts.flash_load,
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
                    let pc = cpu.regs.pc;
                    let cpu_on = trace::enabled(trace::Category::CPU);
                    let pre = cpu_on.then(|| {
                        let (bytes, len) = peek_opcode(|a| bus.read(a), pc);
                        (bytes, len, reg_snap(cpu), cpu.regs.halted)
                    });
                    let hit = Cell::new(None);
                    {
                        let watch = mem_port_watch(debugger, &hit);
                        let mut mio = MemIo128 {
                            bus: bus.as_mut(),
                            watch,
                        };
                        cpu.step(&mut mio);
                    }
                    let dt = (cpu.t - last_t) as u32;
                    last_t = cpu.t;
                    if let Some((bytes, len, regs, was_halt)) = pre {
                        if was_halt {
                            trace::emit(trace::EventKind::CpuHalt { pc });
                        }
                        trace::emit(trace::EventKind::CpuStep {
                            pc,
                            bytes,
                            len,
                            dt: dt as u16,
                            regs,
                        });
                    }
                    if let Some(reason) = hit.get() {
                        debugger.apply_hit(reason);
                        broke_on_pc = true;
                    }
                    Self::advance_tape_ear(
                        tape,
                        &mut bus.ear,
                        bus.beeper,
                        &mut bus.beeper_edges,
                        bus.frame_t,
                        dt,
                        tape_opts.speed,
                        tape_opts.flash_load,
                    );
                    bus.ay.advance(dt);
                    bus.frame_t += dt;
                    while ay_samples.len() < AY_SAMPLES
                        && f64::from(bus.frame_t.min(FRAME_TSTATES_128))
                            >= (ay_samples.len() as f64 + 1.0) * t_per_sample
                    {
                        ay_samples.push(bus.ay.sample_mono());
                    }
                    if broke_on_pc || bus.frame_t >= FRAME_TSTATES_128 {
                        break;
                    }
                }
                while ay_samples.len() < AY_SAMPLES {
                    ay_samples.push(bus.ay.sample_mono());
                }
                if !broke_on_pc {
                    bus.frame_t = 0;
                }
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
                debugger,
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
                let mut broke_on_pc = false;
                while bus.frame_t < FRAME_TSTATES_128 {
                    if debugger.check_pc(cpu.regs.pc) {
                        broke_on_pc = true;
                        break;
                    }
                    if Self::hold_ld_bytes_until_play(cpu.regs.pc, tape, |a| bus.read(a)) {
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
                        let mut mio = MemIoPlus3 {
                            bus: bus.as_mut(),
                            watch: None,
                        };
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
                                tape_opts.flash_load,
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
                    let pc = cpu.regs.pc;
                    let cpu_on = trace::enabled(trace::Category::CPU);
                    let pre = cpu_on.then(|| {
                        let (bytes, len) = peek_opcode(|a| bus.read(a), pc);
                        (bytes, len, reg_snap(cpu), cpu.regs.halted)
                    });
                    let hit = Cell::new(None);
                    {
                        let watch = mem_port_watch(debugger, &hit);
                        let mut mio = MemIoPlus3 {
                            bus: bus.as_mut(),
                            watch,
                        };
                        cpu.step(&mut mio);
                    }
                    let dt = (cpu.t - last_t) as u32;
                    last_t = cpu.t;
                    if let Some((bytes, len, regs, was_halt)) = pre {
                        if was_halt {
                            trace::emit(trace::EventKind::CpuHalt { pc });
                        }
                        trace::emit(trace::EventKind::CpuStep {
                            pc,
                            bytes,
                            len,
                            dt: dt as u16,
                            regs,
                        });
                    }
                    if let Some(reason) = hit.get() {
                        debugger.apply_hit(reason);
                        broke_on_pc = true;
                    }
                    Self::advance_tape_ear(
                        tape,
                        &mut bus.ear,
                        bus.beeper,
                        &mut bus.beeper_edges,
                        bus.frame_t,
                        dt,
                        tape_opts.speed,
                        tape_opts.flash_load,
                    );
                    bus.ay.advance(dt);
                    bus.frame_t += dt;
                    while ay_samples.len() < AY_SAMPLES
                        && f64::from(bus.frame_t.min(FRAME_TSTATES_128))
                            >= (ay_samples.len() as f64 + 1.0) * t_per_sample
                    {
                        ay_samples.push(bus.ay.sample_mono());
                    }
                    if broke_on_pc || bus.frame_t >= FRAME_TSTATES_128 {
                        break;
                    }
                }
                while ay_samples.len() < AY_SAMPLES {
                    ay_samples.push(bus.ay.sample_mono());
                }
                if !broke_on_pc {
                    bus.frame_t = 0;
                }
                FrameAudio {
                    beeper_edges: std::mem::take(&mut bus.beeper_edges),
                    ay_samples,
                }
            }
        }
    }

    /// Advance the tape EAR bitstream.
    ///
    /// Instant flash-load (`flash_load`) never plays TAP pulses: the LD-BYTES
    /// trap (ROM or relocated RAM clone) consumes blocks. Advancing EAR while
    /// BASIC/`USR` runs would skip later TAP blocks (The Boggit flag `0xC8`).
    /// Pure TZX pulse decks have no flash trap, so they still advance.
    /// Loaders that only poll EAR without an LD-BYTES-shaped trap need Instant off.
    fn advance_tape_ear(
        tape: &mut Option<TapeDeck>,
        ear: &mut bool,
        beeper: bool,
        edges: &mut Vec<(u32, bool)>,
        frame_t: u32,
        dt: u32,
        _speed: u32,
        flash_load: bool,
    ) {
        if dt == 0 {
            return;
        }
        if flash_load && matches!(tape.as_ref(), Some(TapeDeck::Tap(_))) {
            return;
        }
        let Some(t) = tape.as_mut() else {
            return;
        };
        // Motor off: do not drive EAR with a frozen pilot level (insert starts paused).
        if !t.playing() {
            return;
        }
        // TAP turbo shortens leader/pause when the block is queued; keep CPU:tape 1:1 here.
        let new_ear = t.advance(dt);
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
    fn hold_ld_bytes_until_play(
        pc: u16,
        tape: &Option<TapeDeck>,
        read: impl Fn(u16) -> u8,
    ) -> bool {
        let holding = tape.as_ref().is_some_and(|t| !t.playing()) && is_ld_bytes_trap_pc(pc, read);
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
        if !is_ld_bytes_trap_pc(cpu.regs.pc, |a| bus.read(a)) {
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
        if !is_ld_bytes_trap_pc(cpu.regs.pc, |a| bus.read(a)) {
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
        if !is_ld_bytes_trap_pc(cpu.regs.pc, |a| bus.read(a)) {
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
                debugger,
                ..
            } => {
                if debugger.check_pc(cpu.regs.pc) {
                    return;
                }
                if Self::hold_ld_bytes_until_play(cpu.regs.pc, tape, |a| bus.read(a)) {
                    const HOLD_T: u32 = 4;
                    Self::advance_tape_ear(
                        tape,
                        &mut bus.ear,
                        bus.beeper,
                        &mut bus.beeper_edges,
                        bus.frame_t,
                        HOLD_T,
                        tape_opts.speed,
                        tape_opts.flash_load,
                    );
                    bus.frame_t = (bus.frame_t + HOLD_T) % FRAME_TSTATES_48;
                    cpu.t = cpu.t.wrapping_add(u64::from(HOLD_T));
                    return;
                }
                if tape_opts.flash_load && Self::try_flash_load_48(cpu, bus, tape) {
                    return;
                }
                if int_active_48(bus.frame_t) {
                    let mut mio = MemIo48 {
                        bus: bus.as_mut(),
                        watch: None,
                    };
                    let irq_t = cpu.interrupt(&mut mio);
                    if irq_t > 0 {
                        if trace::enabled(trace::Category::CPU) {
                            trace::emit(trace::EventKind::CpuIrq {
                                pc: cpu.regs.pc,
                                im: cpu.regs.im,
                            });
                        }
                        if trace::enabled(trace::Category::ULA) {
                            trace::emit(trace::EventKind::UlaInt {
                                frame_t: bus.frame_t,
                            });
                        }
                        Self::advance_tape_ear(
                            tape,
                            &mut bus.ear,
                            bus.beeper,
                            &mut bus.beeper_edges,
                            bus.frame_t,
                            irq_t,
                            tape_opts.speed,
                            tape_opts.flash_load,
                        );
                        bus.frame_t = (bus.frame_t + irq_t) % FRAME_TSTATES_48;
                        return;
                    }
                }
                let pc = cpu.regs.pc;
                let was_halt = cpu.regs.halted;
                let cpu_on = trace::enabled(trace::Category::CPU);
                let pre = cpu_on.then(|| {
                    let (bytes, len) = peek_opcode(|a| bus.read(a), pc);
                    (bytes, len, reg_snap(cpu))
                });
                let hit = Cell::new(None);
                let last_t = cpu.t;
                {
                    let watch = mem_port_watch(debugger, &hit);
                    let mut mio = MemIo48 {
                        bus: bus.as_mut(),
                        watch,
                    };
                    cpu.step(&mut mio);
                }
                let dt = (cpu.t - last_t) as u32;
                if let Some((bytes, len, regs)) = pre {
                    if was_halt {
                        trace::emit(trace::EventKind::CpuHalt { pc });
                    }
                    trace::emit(trace::EventKind::CpuStep {
                        pc,
                        bytes,
                        len,
                        dt: dt as u16,
                        regs,
                    });
                }
                if let Some(reason) = hit.get() {
                    debugger.apply_hit(reason);
                }
                Self::advance_tape_ear(
                    tape,
                    &mut bus.ear,
                    bus.beeper,
                    &mut bus.beeper_edges,
                    bus.frame_t,
                    dt,
                    tape_opts.speed,
                    tape_opts.flash_load,
                );
                bus.frame_t = (bus.frame_t + dt) % FRAME_TSTATES_48;
            }
            Self::Spec128 {
                cpu,
                bus,
                tape,
                tape_opts,
                debugger,
                ..
            } => {
                if debugger.check_pc(cpu.regs.pc) {
                    return;
                }
                if Self::hold_ld_bytes_until_play(cpu.regs.pc, tape, |a| bus.read(a)) {
                    const HOLD_T: u32 = 4;
                    Self::advance_tape_ear(
                        tape,
                        &mut bus.ear,
                        bus.beeper,
                        &mut bus.beeper_edges,
                        bus.frame_t,
                        HOLD_T,
                        tape_opts.speed,
                        tape_opts.flash_load,
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
                    let mut mio = MemIo128 {
                        bus: bus.as_mut(),
                        watch: None,
                    };
                    let irq_t = cpu.interrupt(&mut mio);
                    if irq_t > 0 {
                        if trace::enabled(trace::Category::CPU) {
                            trace::emit(trace::EventKind::CpuIrq {
                                pc: cpu.regs.pc,
                                im: cpu.regs.im,
                            });
                        }
                        if trace::enabled(trace::Category::ULA) {
                            trace::emit(trace::EventKind::UlaInt {
                                frame_t: bus.frame_t,
                            });
                        }
                        Self::advance_tape_ear(
                            tape,
                            &mut bus.ear,
                            bus.beeper,
                            &mut bus.beeper_edges,
                            bus.frame_t,
                            irq_t,
                            tape_opts.speed,
                            tape_opts.flash_load,
                        );
                        bus.ay.advance(irq_t);
                        bus.frame_t = (bus.frame_t + irq_t) % FRAME_TSTATES_128;
                        return;
                    }
                }
                let pc = cpu.regs.pc;
                let was_halt = cpu.regs.halted;
                let cpu_on = trace::enabled(trace::Category::CPU);
                let pre = cpu_on.then(|| {
                    let (bytes, len) = peek_opcode(|a| bus.read(a), pc);
                    (bytes, len, reg_snap(cpu))
                });
                let hit = Cell::new(None);
                let last_t = cpu.t;
                {
                    let watch = mem_port_watch(debugger, &hit);
                    let mut mio = MemIo128 {
                        bus: bus.as_mut(),
                        watch,
                    };
                    cpu.step(&mut mio);
                }
                let dt = (cpu.t - last_t) as u32;
                if let Some((bytes, len, regs)) = pre {
                    if was_halt {
                        trace::emit(trace::EventKind::CpuHalt { pc });
                    }
                    trace::emit(trace::EventKind::CpuStep {
                        pc,
                        bytes,
                        len,
                        dt: dt as u16,
                        regs,
                    });
                }
                if let Some(reason) = hit.get() {
                    debugger.apply_hit(reason);
                }
                Self::advance_tape_ear(
                    tape,
                    &mut bus.ear,
                    bus.beeper,
                    &mut bus.beeper_edges,
                    bus.frame_t,
                    dt,
                    tape_opts.speed,
                    tape_opts.flash_load,
                );
                bus.ay.advance(dt);
                bus.frame_t = (bus.frame_t + dt) % FRAME_TSTATES_128;
            }
            Self::SpecPlus3 {
                cpu,
                bus,
                tape,
                tape_opts,
                debugger,
                ..
            } => {
                if debugger.check_pc(cpu.regs.pc) {
                    return;
                }
                if Self::hold_ld_bytes_until_play(cpu.regs.pc, tape, |a| bus.read(a)) {
                    const HOLD_T: u32 = 4;
                    Self::advance_tape_ear(
                        tape,
                        &mut bus.ear,
                        bus.beeper,
                        &mut bus.beeper_edges,
                        bus.frame_t,
                        HOLD_T,
                        tape_opts.speed,
                        tape_opts.flash_load,
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
                    let mut mio = MemIoPlus3 {
                        bus: bus.as_mut(),
                        watch: None,
                    };
                    let irq_t = cpu.interrupt(&mut mio);
                    if irq_t > 0 {
                        if trace::enabled(trace::Category::CPU) {
                            trace::emit(trace::EventKind::CpuIrq {
                                pc: cpu.regs.pc,
                                im: cpu.regs.im,
                            });
                        }
                        if trace::enabled(trace::Category::ULA) {
                            trace::emit(trace::EventKind::UlaInt {
                                frame_t: bus.frame_t,
                            });
                        }
                        Self::advance_tape_ear(
                            tape,
                            &mut bus.ear,
                            bus.beeper,
                            &mut bus.beeper_edges,
                            bus.frame_t,
                            irq_t,
                            tape_opts.speed,
                            tape_opts.flash_load,
                        );
                        bus.ay.advance(irq_t);
                        bus.frame_t = (bus.frame_t + irq_t) % FRAME_TSTATES_128;
                        return;
                    }
                }
                let pc = cpu.regs.pc;
                let was_halt = cpu.regs.halted;
                let cpu_on = trace::enabled(trace::Category::CPU);
                let pre = cpu_on.then(|| {
                    let (bytes, len) = peek_opcode(|a| bus.read(a), pc);
                    (bytes, len, reg_snap(cpu))
                });
                let hit = Cell::new(None);
                let last_t = cpu.t;
                {
                    let watch = mem_port_watch(debugger, &hit);
                    let mut mio = MemIoPlus3 {
                        bus: bus.as_mut(),
                        watch,
                    };
                    cpu.step(&mut mio);
                }
                let dt = (cpu.t - last_t) as u32;
                if let Some((bytes, len, regs)) = pre {
                    if was_halt {
                        trace::emit(trace::EventKind::CpuHalt { pc });
                    }
                    trace::emit(trace::EventKind::CpuStep {
                        pc,
                        bytes,
                        len,
                        dt: dt as u16,
                        regs,
                    });
                }
                if let Some(reason) = hit.get() {
                    debugger.apply_hit(reason);
                }
                Self::advance_tape_ear(
                    tape,
                    &mut bus.ear,
                    bus.beeper,
                    &mut bus.beeper_edges,
                    bus.frame_t,
                    dt,
                    tape_opts.speed,
                    tape_opts.flash_load,
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
                let mut mio = MemIo48 {
                    bus: bus.as_mut(),
                    watch: None,
                };
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
                    tape_opts.flash_load,
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
                let mut mio = MemIo128 {
                    bus: bus.as_mut(),
                    watch: None,
                };
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
                    tape_opts.flash_load,
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
                let mut mio = MemIoPlus3 {
                    bus: bus.as_mut(),
                    watch: None,
                };
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
                    tape_opts.flash_load,
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

    fn hold_keys(&mut self, keys: &[(usize, u8)], frames: u32) {
        for _ in 0..frames {
            let kb = self.keyboard_mut();
            kb.reset();
            for &(row, bit) in keys {
                kb.set_key(row, bit, true);
            }
            let _ = self.run_frame();
        }
    }

    /// Script `LOAD ""` [CODE] Enter for 48K keyword mode (ROM debounce included).
    pub fn type_load_quotes_48k(&mut self, with_code: bool) {
        const PRESS: u32 = 10;
        const GAP: u32 = 5;
        self.hold_keys(&[(6, 3)], PRESS);
        self.hold_keys(&[], GAP);
        self.hold_keys(&[(7, 1), (5, 0)], PRESS);
        self.hold_keys(&[], GAP);
        self.hold_keys(&[(7, 1), (5, 0)], PRESS);
        self.hold_keys(&[], GAP);
        if with_code {
            self.hold_keys(&[(0, 0), (7, 1)], PRESS);
            self.hold_keys(&[], GAP);
            self.hold_keys(&[(5, 2)], PRESS);
            self.hold_keys(&[], GAP);
        }
        self.hold_keys(&[(6, 0)], PRESS);
        self.hold_keys(&[], 15);
        self.keyboard_mut().reset();
    }

    fn wait_48_basic_prompt(&mut self, max_frames: u32) {
        let mut stable = 0u32;
        for _ in 0..max_frames {
            let pc = self.cpu().regs.pc;
            // 48K ROM MAIN-EXEC / WAIT-KEY after the copyright has finished.
            if (0x12A0..=0x1600).contains(&pc) {
                stable += 1;
                if stable >= 20 {
                    return;
                }
            } else {
                stable = 0;
            }
            let _ = self.run_frame();
        }
    }

    /// 128K menu: cursor-down to 48 BASIC, Enter, wait for the 48K prompt, then keywords.
    pub fn type_load_quotes_128k(&mut self, with_code: bool) {
        const PRESS: u32 = 10;
        const GAP: u32 = 5;
        // CAPS+6 = cursor down (Tape Loader → 128 BASIC → Calculator → 48 BASIC).
        for _ in 0..3 {
            self.hold_keys(&[(0, 0), (4, 4)], PRESS);
            self.hold_keys(&[], GAP);
        }
        self.hold_keys(&[(6, 0)], PRESS);
        self.hold_keys(&[], 10);
        self.wait_48_basic_prompt(500);
        self.type_load_quotes_48k(with_code);
    }

    /// +3 menu: cursor-down to 48 BASIC, Enter, then keyword `LOAD ""` [CODE].
    pub fn type_load_quotes_plus3(&mut self, with_code: bool) {
        const PRESS: u32 = 10;
        const GAP: u32 = 5;
        // CAPS+6 = cursor down (Loader → +3 BASIC → Calculator → 48 BASIC).
        for _ in 0..3 {
            self.hold_keys(&[(0, 0), (4, 4)], PRESS);
            self.hold_keys(&[], GAP);
        }
        self.hold_keys(&[(6, 0)], PRESS);
        self.hold_keys(&[], 10);
        self.wait_48_basic_prompt(500);
        self.type_load_quotes_48k(with_code);
    }

    /// Model-aware `LOAD ""` [CODE] (48K keyword / 128K / +3 48 BASIC).
    pub fn type_load_quotes(&mut self, with_code: bool) {
        match self.model() {
            Model::Spectrum48 => self.type_load_quotes_48k(with_code),
            Model::Spectrum128 => self.type_load_quotes_128k(with_code),
            Model::SpectrumPlus3 => self.type_load_quotes_plus3(with_code),
        }
    }

    #[must_use]
    pub fn debugger(&self) -> &Debugger {
        match self {
            Self::Spec48 { debugger, .. }
            | Self::Spec128 { debugger, .. }
            | Self::SpecPlus3 { debugger, .. } => debugger,
        }
    }

    pub fn debugger_mut(&mut self) -> &mut Debugger {
        match self {
            Self::Spec48 { debugger, .. }
            | Self::Spec128 { debugger, .. }
            | Self::SpecPlus3 { debugger, .. } => debugger,
        }
    }

    /// Run instructions until a breakpoint/watch, halt, or `max_insns`.
    pub fn run_until_break(&mut self, max_insns: u64) -> BreakReason {
        let pc = self.cpu().regs.pc;
        if self.debugger().paused {
            self.debugger_mut().continue_from_pc(pc);
        } else {
            self.debugger_mut().last_hit = BreakReason::None;
        }
        for _ in 0..max_insns {
            if self.debugger().paused {
                return self.debugger().last_hit;
            }
            if self.cpu().regs.halted && !self.cpu().regs.iff1 {
                self.debugger_mut().paused = true;
                self.debugger_mut().last_hit = BreakReason::Halt;
                return BreakReason::Halt;
            }
            self.step_once();
            let hit = self.debugger().last_hit;
            if hit.is_stop() {
                return hit;
            }
        }
        self.debugger_mut().last_hit = BreakReason::Budget;
        BreakReason::Budget
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
        m.set_tape_load_options(TapeLoadOptions {
            flash_load: false,
            speed: 1,
        });
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
            ..Default::default()
        };
        let mut m = Machine::new_48k(&rom).unwrap();
        m.set_tape_load_options(TapeLoadOptions {
            flash_load: false,
            speed: 1,
        });
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
        m.set_tape_load_options(TapeLoadOptions {
            flash_load: false,
            speed: 1,
        });
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
        m.set_tape_load_options(TapeLoadOptions {
            flash_load: false,
            speed: 1,
        });
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
    fn rom_ld_bytes_ear_loads_attr_mark_data_block() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/tape/attr_mark.tap");
        let img = TapImage::load(&path).expect("attr_mark");
        let data = img.blocks[1].clone();
        let mut m = Machine::new_48k(&rom).unwrap();
        m.set_tape_load_options(TapeLoadOptions {
            flash_load: false,
            speed: 1,
        });
        let mut player = TapPlayer::new(img);
        player.consume_block();
        m.insert_tape(player);
        m.set_tape_playing(true);

        let ret = 0x1234u16;
        m.cpu_mut().regs.sp = 0x5f00;
        m.write_mem(0x5f00, (ret & 0xff) as u8);
        m.write_mem(0x5f01, (ret >> 8) as u8);
        m.cpu_mut().regs.a = 0xff;
        m.cpu_mut().regs.f = flag::C;
        m.cpu_mut().regs.set_ix(0x8000);
        m.cpu_mut().regs.set_de((data.len() - 2) as u16);
        m.cpu_mut().regs.pc = 0x0556;
        if let Machine::Spec48 { bus, .. } = &mut m {
            bus.frame_t = INT_LENGTH_48;
        }
        let mut ok = false;
        for _ in 0..400 {
            let _ = m.run_frame();
            if attr_mark_code_ok(&m) {
                ok = true;
                break;
            }
        }
        if !ok {
            eprintln!(
                "EAR data fail PC={:04X} mem {:02X}{:02X}{:02X}{:02X}{:02X}{:02X} block={:?} IX={:04X} DE={:04X} F={:02X}",
                m.cpu().regs.pc,
                m.read_mem(0x8000),
                m.read_mem(0x8001),
                m.read_mem(0x8002),
                m.read_mem(0x8003),
                m.read_mem(0x8004),
                m.read_mem(0x8005),
                m.tape_block(),
                m.cpu().regs.ix(),
                m.cpu().regs.de(),
                m.cpu().regs.f,
            );
        }
        assert!(ok, "ROM LD-BYTES EAR path should load attr_mark CODE bytes");
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
        let sp = slow.tape_progress().unwrap();
        let fp = fast.tape_progress().unwrap();
        assert!(
            fp.pulse_count < sp.pulse_count,
            "10x should schedule a shorter leader (slow {} pulses, fast {})",
            sp.pulse_count,
            fp.pulse_count
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

    /// Optional local e2e: scripted `LOAD ""` + Play flash-loads Boggit PROGRAM header.
    /// Set `SPEC_CHUM_BOGGIT_TZX` (do not commit the TZX).
    #[test]
    fn boggit_load_quotes_flash_loads_header_when_present() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let Some(boggit) = std::env::var_os("SPEC_CHUM_BOGGIT_TZX").map(PathBuf::from) else {
            eprintln!("skip: set SPEC_CHUM_BOGGIT_TZX for Boggit LOAD \"\" e2e");
            return;
        };
        if !boggit.is_file() {
            eprintln!("skip: SPEC_CHUM_BOGGIT_TZX not a file ({boggit:?})");
            return;
        }
        let data = std::fs::read(&boggit).expect("read boggit");
        let img = tape::TzxPlayer::to_tap_image(&data).expect("to tap");
        let mut m = Machine::new_48k(&rom).unwrap();
        m.set_tape_load_options(TapeLoadOptions {
            flash_load: true,
            speed: 1,
        });
        m.insert_tape(TapPlayer::new(img));
        for _ in 0..200 {
            let _ = m.run_frame();
        }
        m.type_load_quotes_48k(false);
        assert_eq!(m.cpu().regs.pc, LD_BYTES_TRAP_PC);
        m.set_tape_playing(true);
        let mut progressed = false;
        for _ in 0..200 {
            let _ = m.run_frame();
            let block = m.tape_block();
            // Header (+ follow-on blocks) consumed, or PC left ROM into the loader.
            if block.is_some_and(|b| b >= 1) || m.cpu().regs.pc >= 0x4000 {
                progressed = true;
                break;
            }
        }
        assert!(
            progressed,
            "Boggit LOAD \"\" should flash-load past the first block (PC={:04X} block={:?})",
            m.cpu().regs.pc,
            m.tape_block()
        );
        eprintln!(
            "Boggit LOAD \"\" progressed: PC={:04X} block={:?}",
            m.cpu().regs.pc,
            m.tape_block()
        );
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

    /// Shared `LOAD "" CODE` harness for `attr_mark.tap`. Returns whether CODE
    /// bytes landed at 0x8000. Caller must hold `trace::test_lock()` when tracing.
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
        // attr_mark is a CODE block — plain LOAD "" only accepts PROGRAM headers.
        m.type_load_quotes_48k(true);
        m.set_tape_playing(true);
        let mut loaded = false;
        for _ in 0..200 {
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

    fn attr_mark_code_ok(m: &Machine) -> bool {
        m.read_mem(0x8000) == 0x21
            && m.read_mem(0x8001) == 0x00
            && m.read_mem(0x8002) == 0x58
            && m.read_mem(0x8003) == 0x36
            && m.read_mem(0x8004) == 0xd7
            && m.read_mem(0x8005) == 0xc9
    }

    fn run_attr_mark_typed(
        mut m: Machine,
        img: TapImage,
        flash_load: bool,
        speed: u32,
        warmup: u32,
        max_frames: u32,
    ) -> (Machine, bool) {
        m.set_tape_load_options(TapeLoadOptions { flash_load, speed });
        m.insert_tape(TapPlayer::new(img));
        for _ in 0..warmup {
            let _ = m.run_frame();
        }
        m.type_load_quotes(true);
        m.set_tape_playing(true);
        let mut loaded = false;
        for _ in 0..max_frames {
            let _ = m.run_frame();
            if attr_mark_code_ok(&m) {
                loaded = true;
                break;
            }
        }
        (m, loaded)
    }

    #[test]
    fn attr_mark_ear_load_quotes_code_succeeds_at_speed_10() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/tape/attr_mark.tap");
        let img = TapImage::load(&path).expect("attr_mark.tap");
        let m = Machine::new_48k(&rom).unwrap();
        // Speed 10 keeps ROM-accurate bit widths (leader/pause only); ~hundreds of frames.
        // Speed 10 floors the leader so LD-LEADER's 1045-edge wait still fits; pause is /10.
        let (_m, loaded) = run_attr_mark_typed(m, img, false, 10, 200, 2_000);
        assert!(
            loaded,
            "EAR LOAD \"\" CODE should poke CODE at 0x8000 (speed 10; budget 2000 frames)"
        );
    }

    #[test]
    fn attr_mark_type_load_128k_flash() {
        let Some(rom) = rom128() else {
            eprintln!("skip: roms/128/spec128uk.rom missing");
            return;
        };
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/tape/attr_mark.tap");
        let img = TapImage::load(&path).expect("attr_mark.tap");
        let m = Machine::new_128k(&rom).unwrap();
        let (_m, loaded) = run_attr_mark_typed(m, img, true, 1, 200, 400);
        assert!(
            loaded,
            "128K 48 BASIC LOAD \"\" CODE should flash-load attr_mark"
        );
    }

    #[test]
    fn attr_mark_type_load_plus3_flash() {
        let Some(rom) = rom_plus3() else {
            eprintln!("skip: plus3/plus2a ROM missing");
            return;
        };
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/tape/attr_mark.tap");
        let img = TapImage::load(&path).expect("attr_mark.tap");
        let m = Machine::new_plus3(&rom).unwrap();
        let (_m, loaded) = run_attr_mark_typed(m, img, true, 1, 250, 400);
        assert!(
            loaded,
            "+3 48 BASIC LOAD \"\" CODE should flash-load attr_mark"
        );
    }

    /// Deterministic tape repro harness (observability + success).
    ///
    /// Runs 48K `LOAD "" CODE` against `tests/fixtures/tape/attr_mark.tap` with
    /// the structured trace enabled. On load failure, dumps the ring to stderr.
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
        assert!(
            dump.contains("tape.flash.enter") && dump.contains("tape.flash.exit"),
            "expected flash-load enter/exit; dump head:\n{}",
            dump.chars().take(1200).collect::<String>()
        );
        assert!(
            loaded,
            "attr_mark LOAD \"\" CODE did not place CODE at 0x8000"
        );
        eprintln!("attr_mark LOAD \"\" CODE succeeded (CODE at 0x8000)");
    }

    /// Hard success gate for attr_mark `LOAD "" CODE`.
    #[test]
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

    /// `print_ok.tap` is a PROGRAM — plain `LOAD ""` must flash-load both blocks.
    #[test]
    fn print_ok_load_quotes_succeeds() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/tape/print_ok.tap");
        let img = TapImage::load(&path).expect("print_ok.tap");
        let mut m = Machine::new_48k(&rom).unwrap();
        m.set_tape_load_options(TapeLoadOptions {
            flash_load: true,
            speed: 1,
        });
        m.insert_tape(TapPlayer::new(img));
        for _ in 0..200 {
            let _ = m.run_frame();
        }
        m.type_load_quotes_48k(false);
        assert_eq!(
            m.cpu().regs.pc,
            LD_BYTES_TRAP_PC,
            "LOAD \"\" should reach LD-BYTES while paused"
        );
        m.set_tape_playing(true);
        let mut done = false;
        for _ in 0..200 {
            let _ = m.run_frame();
            // Both TAP blocks consumed and back in the editor / running.
            if m.tape_block() == Some(2) || m.tape_block().is_none() {
                // Program line 10 starts with length bytes; look for PRINT token 0xF5
                // or the "OK" string in the loaded BASIC area.
                let prog = u16::from_le_bytes([m.read_mem(0x5C53), m.read_mem(0x5C54)]);
                let eline = u16::from_le_bytes([m.read_mem(0x5C59), m.read_mem(0x5C5A)]);
                let mut found_ok = false;
                for a in prog..eline {
                    if m.read_mem(a) == b'O' && m.read_mem(a.wrapping_add(1)) == b'K' {
                        found_ok = true;
                        break;
                    }
                }
                if found_ok {
                    done = true;
                    break;
                }
            }
        }
        assert!(
            done,
            "print_ok LOAD \"\" should place BASIC containing OK (PC={:04X} block={:?})",
            m.cpu().regs.pc,
            m.tape_block()
        );
    }

    #[test]
    fn flash_load_skip_appears_in_trace_dump() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let img = TapImage {
            blocks: vec![vec![0xff, 0x11, 0xff ^ 0x11], vec![0x00, 0x22, 0x22]],
            ..Default::default()
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

    #[test]
    fn inspect_after_boot_steps() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let mut m = Machine::new_48k(&rom).unwrap();
        for _ in 0..8 {
            m.step_once();
        }
        let i = m.inspect();
        assert_eq!(i.model, Model::Spectrum48);
        assert!(i.cpu_t > 0);
        assert_eq!(i.frame_tstates, FRAME_TSTATES_48);
        let hex = m.hexdump(0x0000, 16);
        assert!(hex.contains("0000"));
        let d = m.disasm_window(0x0000, 4);
        assert!(d.contains("0000"));
        let json = i.to_json();
        assert!(json.contains("\"model\":\"48k\""));
    }

    #[test]
    fn inspect_128k_paging() {
        let Some(rom) = rom128() else {
            eprintln!("skip: roms/128/spec128uk.rom missing");
            return;
        };
        let m = Machine::new_128k(&rom).unwrap();
        let i = m.inspect();
        assert_eq!(i.model, Model::Spectrum128);
        assert!(i.paging.page_7ffd.is_some());
    }

    #[test]
    fn apply_sna48_sets_pc_ram_and_border() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let mut data = vec![0u8; 49179];
        data[26] = 5; // border
        data[23] = 0x00;
        data[24] = 0x40; // SP = 0x4000 → pop PC from RAM[0x4000]
        data[27] = 0x00;
        data[28] = 0x80; // PC = 0x8000
        data[27 + 0x4000] = 0xaa; // byte at 0x8000
        let snap = Snapshot48::parse_sna(&data).expect("synthetic SNA48");

        let _lock = trace::test_lock();
        let dump = trace::with_trace(trace::Category::MACHINE, || {
            let mut m = Machine::new_48k(&rom).unwrap();
            m.apply_snapshot48(&snap);
            let i = m.inspect();
            assert_eq!(i.regs.pc, 0x8000);
            assert_eq!(m.read_mem(0x8000), 0xaa);
            assert_eq!(i.border, 5);
            trace::dump_string()
        });
        assert!(
            dump.contains("machine.snapshot"),
            "expected machine.snapshot in dump:\n{dump}"
        );
    }

    #[test]
    fn until_pc_and_mem_watch() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let mut m = Machine::new_48k(&rom).unwrap();
        m.write_mem(0x8000, 0x18);
        m.write_mem(0x8001, 0xfe); // JR $
        m.cpu_mut().regs.pc = 0x8000;
        m.debugger_mut().add_pc_break(0x8000);
        let reason = m.run_until_break(8);
        assert_eq!(reason, BreakReason::Pc(0x8000));
        assert_eq!(m.cpu().regs.pc, 0x8000);

        let mut m = Machine::new_48k(&rom).unwrap();
        m.write_mem(0x8000, 0x00); // NOP
        m.write_mem(0x8001, 0x00);
        m.cpu_mut().regs.pc = 0x8000;
        if let Machine::Spec48 { bus, .. } = &mut m {
            bus.frame_t = INT_LENGTH_48;
        }
        assert_eq!(m.run_until_break(2), BreakReason::Budget);
        let pc_after = m.cpu().regs.pc;
        assert_eq!(m.run_until_break(8), BreakReason::Budget);
        assert_ne!(
            m.cpu().regs.pc,
            pc_after,
            "second budget run must not stall"
        );

        let mut m = Machine::new_48k(&rom).unwrap();
        m.write_mem(0x8000, 0x77); // LD (HL),A
        m.cpu_mut().regs.pc = 0x8000;
        m.cpu_mut().regs.set_hl(0x4000);
        m.cpu_mut().regs.a = 0xaa;
        if let Machine::Spec48 { bus, .. } = &mut m {
            bus.frame_t = INT_LENGTH_48; // avoid IRQ
        }
        m.debugger_mut().add_mem_watch(Watch {
            addr: 0x4000,
            read: false,
            write: true,
        });
        m.step_once();
        assert_eq!(m.read_mem(0x4000), 0xaa);
        assert!(matches!(
            m.debugger().last_hit,
            BreakReason::Mem {
                addr: 0x4000,
                write: true,
                value: 0xaa
            }
        ));

        let mut m = Machine::new_48k(&rom).unwrap();
        m.write_mem(0x8000, 0x77); // LD (HL),A
        m.cpu_mut().regs.pc = 0x8000;
        m.cpu_mut().regs.set_hl(0x4000);
        m.cpu_mut().regs.a = 0x55;
        m.debugger_mut().add_mem_watch(Watch {
            addr: 0x4000,
            read: false,
            write: true,
        });
        m.run_frame();
        assert_eq!(m.read_mem(0x4000), 0x55);
        assert!(m.debugger().paused);
        assert!(matches!(
            m.debugger().last_hit,
            BreakReason::Mem {
                addr: 0x4000,
                write: true,
                value: 0x55
            }
        ));
        assert!(m.frame_t() > 0, "mid-frame watch must keep raster time");
    }

    #[test]
    fn cpu_step_appears_in_trace() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let _lock = trace::test_lock();
        trace::clear();
        trace::enable(trace::Category::CPU);
        let mut m = Machine::new_48k(&rom).unwrap();
        if let Machine::Spec48 { bus, .. } = &mut m {
            bus.frame_t = INT_LENGTH_48;
        }
        m.step_once();
        let dump = trace::dump_string();
        assert!(dump.contains("cpu.step"), "dump=\n{dump}");
        let json = trace::dump_json();
        assert!(json.contains("cpu"));
        trace::disable();
        trace::clear();
    }

    fn custom_loader_tap() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/tape/custom_loader.tap")
    }

    fn custom_loader_ok(m: &Machine) -> bool {
        m.read_mem(0x9000) == 0xa5
    }

    fn run_typed_load(
        mut m: Machine,
        img: TapImage,
        flash_load: bool,
        speed: u32,
        with_code: bool,
        warmup: u32,
        max_frames: u32,
        done: impl Fn(&Machine) -> bool,
    ) -> (Machine, bool) {
        m.set_tape_load_options(TapeLoadOptions { flash_load, speed });
        m.insert_tape(TapPlayer::new(img));
        for _ in 0..warmup {
            let _ = m.run_frame();
        }
        m.type_load_quotes(with_code);
        m.set_tape_playing(true);
        let mut loaded = false;
        for _ in 0..max_frames {
            let _ = m.run_frame();
            if done(&m) {
                loaded = true;
                break;
            }
        }
        (m, loaded)
    }

    /// `attr_mark` CODE across models × Instant + EAR speeds (CLI `--speed` 1..=64).
    #[test]
    fn attr_mark_load_matrix_models_and_speeds() {
        let speeds = [1u32, 2, 5, 10, 20];
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/tape/attr_mark.tap");
        let img = TapImage::load(&path).expect("attr_mark");

        let mut cases: Vec<(Model, Vec<u8>, &str)> = Vec::new();
        if let Some(r) = rom48() {
            cases.push((Model::Spectrum48, r, "48k"));
        }
        if let Some(r) = rom128() {
            cases.push((Model::Spectrum128, r, "128k"));
        }
        if let Some(r) = rom_plus3() {
            cases.push((Model::SpectrumPlus3, r, "plus3"));
        }
        if cases.is_empty() {
            eprintln!("skip: no ROMs for attr_mark matrix");
            return;
        }

        let mut report = String::from("attr_mark matrix:\n");
        let mut failed = Vec::new();
        for (model, rom, label) in &cases {
            let warmup = if *model == Model::Spectrum48 {
                200
            } else {
                250
            };
            // Instant
            let (_m, ok) = run_typed_load(
                match model {
                    Model::Spectrum48 => Machine::new_48k(rom).unwrap(),
                    Model::Spectrum128 => Machine::new_128k(rom).unwrap(),
                    Model::SpectrumPlus3 => Machine::new_plus3(rom).unwrap(),
                },
                img.clone(),
                true,
                1,
                true,
                warmup,
                500,
                attr_mark_code_ok,
            );
            report.push_str(&format!(
                "  {label} instant: {}\n",
                if ok { "PASS" } else { "FAIL" }
            ));
            if !ok {
                failed.push(format!("{label}/instant"));
            }
            let full = std::env::var_os("SPEC_CHUM_FULL_TAPE_MATRIX").is_some();
            for speed in speeds {
                // EAR@1 is slow (~minutes of Spectrum time); default CI keeps it
                // on 48K only. Set SPEC_CHUM_FULL_TAPE_MATRIX=1 for 128K/+3 @1×.
                if speed == 1 && *model != Model::Spectrum48 && !full {
                    report.push_str(&format!(
                        "  {label} ear@{speed}: SKIP (set SPEC_CHUM_FULL_TAPE_MATRIX=1)\n"
                    ));
                    continue;
                }
                let max = match speed {
                    1 => 25_000,
                    2 => 15_000,
                    5 => 6_000,
                    10 => 3_000,
                    _ => 2_000,
                };
                let (_m, ok) = run_typed_load(
                    match model {
                        Model::Spectrum48 => Machine::new_48k(rom).unwrap(),
                        Model::Spectrum128 => Machine::new_128k(rom).unwrap(),
                        Model::SpectrumPlus3 => Machine::new_plus3(rom).unwrap(),
                    },
                    img.clone(),
                    false,
                    speed,
                    true,
                    warmup,
                    max,
                    attr_mark_code_ok,
                );
                report.push_str(&format!(
                    "  {label} ear@{speed}: {}\n",
                    if ok { "PASS" } else { "FAIL" }
                ));
                if !ok {
                    failed.push(format!("{label}/ear@{speed}"));
                }
            }
        }
        eprintln!("{report}");
        assert!(
            failed.is_empty(),
            "attr_mark failures: {}",
            failed.join(", ")
        );
    }

    /// Boggit-style PROGRAM + CODE + flag `0xC8` via RAM LD-BYTES clone.
    #[test]
    fn custom_loader_matrix_models_instant_and_ear() {
        let path = custom_loader_tap();
        let img = TapImage::load(&path).expect("custom_loader");
        assert_eq!(img.blocks.len(), 3);

        let mut cases: Vec<(Model, Vec<u8>, &str)> = Vec::new();
        if let Some(r) = rom48() {
            cases.push((Model::Spectrum48, r, "48k"));
        }
        if let Some(r) = rom128() {
            cases.push((Model::Spectrum128, r, "128k"));
        }
        if let Some(r) = rom_plus3() {
            cases.push((Model::SpectrumPlus3, r, "plus3"));
        }
        if cases.is_empty() {
            eprintln!("skip: no ROMs for custom_loader matrix");
            return;
        }

        let mut report = String::from("custom_loader matrix:\n");
        let mut failed = Vec::new();
        for (model, rom, label) in &cases {
            let warmup = if *model == Model::Spectrum48 {
                200
            } else {
                250
            };
            // Instant + EAR speeds matching CLI `--speed` presets.
            let full = std::env::var_os("SPEC_CHUM_FULL_TAPE_MATRIX").is_some();
            let mut modes: Vec<(bool, u32, &str, u32)> =
                vec![(true, 1, "instant", 800), (false, 2, "ear@2", 12_000)];
            for (speed, tag, max) in [
                (5u32, "ear@5", 6_000u32),
                (10, "ear@10", 4_000),
                (20, "ear@20", 2_500),
            ] {
                modes.push((false, speed, tag, max));
            }
            if *model == Model::Spectrum48 || full {
                modes.insert(1, (false, 1, "ear@1", 25_000));
            } else {
                report.push_str(&format!(
                    "  {label} ear@1: SKIP (set SPEC_CHUM_FULL_TAPE_MATRIX=1)\n"
                ));
            }
            for (flash, speed, tag, max) in modes {
                let mut m = match model {
                    Model::Spectrum48 => Machine::new_48k(rom).unwrap(),
                    Model::Spectrum128 => Machine::new_128k(rom).unwrap(),
                    Model::SpectrumPlus3 => Machine::new_plus3(rom).unwrap(),
                };
                m.set_tape_load_options(TapeLoadOptions {
                    flash_load: flash,
                    speed,
                });
                m.insert_tape(TapPlayer::new(img.clone()));
                for _ in 0..warmup {
                    let _ = m.run_frame();
                }
                m.type_load_quotes(true); // LOAD "" CODE
                m.set_tape_playing(true);
                let mut code_ready = false;
                for _ in 0..max {
                    let _ = m.run_frame();
                    // Wait until CODE data has finished (block ≥ 2), not merely the first byte.
                    if m.tape_block().is_some_and(|b| b >= 2)
                        && m.read_mem(0x8000) == 0xdd
                        && m.read_mem(0x800a) == 0xcd
                    {
                        code_ready = true;
                        break;
                    }
                }
                if code_ready {
                    // Instant must leave the C8 block queued (no EAR race). EAR may
                    // already be into the post-CODE pause / next pilot while BASIC
                    // returns, so rewind before USR for those cells only.
                    if flash {
                        assert_eq!(
                            m.tape_block(),
                            Some(2),
                            "{label} instant must not advance past CODE into C8 before USR"
                        );
                    } else if let Some(TapeDeck::Tap(p)) = match &mut m {
                        Machine::Spec48 { tape, .. }
                        | Machine::Spec128 { tape, .. }
                        | Machine::SpecPlus3 { tape, .. } => tape.as_mut(),
                    } {
                        p.rewind_to_block(2);
                        m.set_tape_playing(true);
                    }
                    // RANDOMIZE USR 32768 — enter the custom-flag loader.
                    let ret = 0x15e6u16;
                    m.cpu_mut().regs.sp = 0xfffd;
                    m.write_mem(0xfffd, (ret & 0xff) as u8);
                    m.write_mem(0xfffe, (ret >> 8) as u8);
                    m.cpu_mut().regs.pc = 0x8000;
                    for _ in 0..max {
                        let _ = m.run_frame();
                        if custom_loader_ok(&m) {
                            break;
                        }
                    }
                }
                let ok = custom_loader_ok(&m);
                report.push_str(&format!(
                    "  {label} {tag}: {} code_ready={code_ready} PC={:04X} block={:?} 8000={:02X} 9000={:02X}\n",
                    if ok { "PASS" } else { "FAIL" },
                    m.cpu().regs.pc,
                    m.tape_block(),
                    m.read_mem(0x8000),
                    m.read_mem(0x9000),
                ));
                if !ok {
                    failed.push(format!("{label}/{tag}"));
                }
            }
        }
        eprintln!("{report}");
        assert!(
            failed.is_empty(),
            "custom_loader failures: {}",
            failed.join(", ")
        );
    }

    /// Optional: Boggit Side 1 through custom `0xC8` loads (set `SPEC_CHUM_BOGGIT_TZX`).
    ///
    /// Default: Instant + EAR@2 on 48K/128K/+3 (complete to block 8 / `JP 5B00`).
    /// EAR@5+ shortens inter-block pauses below what Boggit's RAM loader needs —
    /// those cells are skipped unless `SPEC_CHUM_FULL_TAPE_MATRIX=1` (still may fail).
    /// `SPEC_CHUM_FULL_TAPE_MATRIX=1` also adds EAR@1.
    #[test]
    fn boggit_side1_matrix_when_present() {
        let Some(boggit) = std::env::var_os("SPEC_CHUM_BOGGIT_TZX").map(PathBuf::from) else {
            eprintln!("skip: set SPEC_CHUM_BOGGIT_TZX for full Boggit matrix");
            return;
        };
        if !boggit.is_file() {
            eprintln!("skip: SPEC_CHUM_BOGGIT_TZX not a file");
            return;
        }
        let data = std::fs::read(&boggit).expect("boggit");
        assert!(
            tape::TzxPlayer::is_standard_speed_only(&data),
            "SPEC_CHUM_BOGGIT_TZX must be standard-speed (0x10) only for TAP conversion"
        );
        let player = tape::TzxPlayer::to_tap_player(&data).expect("to tap");
        let img = player.image.clone();
        // PROG+CODE = blocks 0..3; first custom `0xC8` is block 4. Require that block
        // consumed (index ≥ 5) or game entry. Full Side-1 (block ≥ 8) is Instant-fast;
        // EAR@5+ still bit-accurate so huge C8s need minutes — not required for CI.
        let done =
            |m: &Machine| m.cpu().regs.pc == 0x5b00 || m.tape_block().is_some_and(|b| b >= 5);

        let full = std::env::var_os("SPEC_CHUM_FULL_TAPE_MATRIX").is_some();
        let mut report = String::from("boggit matrix:\n");
        let mut failed = Vec::new();
        for (rom, label, model) in [
            (rom48(), "48k", Model::Spectrum48),
            (rom128(), "128k", Model::Spectrum128),
            (rom_plus3(), "plus3", Model::SpectrumPlus3),
        ] {
            let Some(rom) = rom else {
                report.push_str(&format!("  {label}: SKIP (no ROM)\n"));
                continue;
            };
            let mut modes: Vec<(bool, u32, &str, u32)> =
                vec![(true, 1, "instant", 2_000), (false, 2, "ear@2", 20_000)];
            if full {
                modes.push((false, 1, "ear@1", 40_000));
                for (speed, tag, max) in [
                    (5u32, "ear@5", 10_000u32),
                    (10, "ear@10", 8_000),
                    (20, "ear@20", 5_000),
                ] {
                    modes.push((false, speed, tag, max));
                }
            } else {
                report.push_str(&format!(
                    "  {label} ear@1/@5/@10/@20: SKIP (set SPEC_CHUM_FULL_TAPE_MATRIX=1; ≥5x may fail)\n"
                ));
            }
            for (flash, speed, tag, max) in modes {
                let m = match model {
                    Model::Spectrum48 => Machine::new_48k(&rom).unwrap(),
                    Model::Spectrum128 => Machine::new_128k(&rom).unwrap(),
                    Model::SpectrumPlus3 => Machine::new_plus3(&rom).unwrap(),
                };
                let deck = TapPlayer::new(img.clone());
                let mut machine = m;
                machine.set_tape_load_options(TapeLoadOptions {
                    flash_load: flash,
                    speed,
                });
                machine.insert_tape(deck);
                for _ in 0..200 {
                    let _ = machine.run_frame();
                }
                machine.type_load_quotes(false);
                machine.set_tape_playing(true);
                let mut ok = false;
                for _ in 0..max {
                    let _ = machine.run_frame();
                    if done(&machine) {
                        ok = true;
                        break;
                    }
                }
                report.push_str(&format!(
                    "  {label} {tag}: {} (PC={:04X} block={:?})\n",
                    if ok { "PASS" } else { "FAIL" },
                    machine.cpu().regs.pc,
                    machine.tape_block()
                ));
                if !ok {
                    failed.push(format!("{label}/{tag}"));
                }
            }
        }
        eprintln!("{report}");
        assert!(failed.is_empty(), "boggit failures: {}", failed.join(", "));
    }
}
