//! Spec Chum machine — Spectrum models and frame runner.

#![allow(clippy::pedantic)]
#![allow(clippy::large_enum_variant)]

#[cfg(all(test, feature = "slow-tests"))]
mod z80test;

#[cfg(all(test, feature = "system-tests"))]
mod system_tests;

mod debugger;
mod inspect;
mod joystick;
mod rom;

pub use debugger::{BreakReason, Debugger, Watch};
pub use inspect::{Inspect, Paging, TapeInspect};
pub use joystick::{apply_joystick, clear_joystick_matrix, JoystickMode, JoystickState};
pub use rom::{
    expected_main_rom_bytes, install_rom_slot, main_rom_available, main_rom_available_in,
    model_label, model_title, read_rom, read_rom_with_overrides, read_trdos_rom,
    read_trdos_rom_with_overrides, requires_trdos_rom, requires_user_rom, resolve_rom_path,
    resolve_rom_path_in, resolve_rom_path_in_with_overrides, resolve_trdos_rom_path,
    resolve_trdos_rom_path_in, resolve_trdos_rom_path_in_with_overrides, rom_available,
    rom_available_in, rom_available_in_with_overrides, rom_candidates, rom_path_status,
    rom_slot_descriptors, rom_slot_state, rom_slot_state_with_override, rom_slot_states,
    rom_slot_states_with_overrides, search_roots, trdos_rom_available, trdos_rom_available_in,
    unavailable_reason, writable_install_root, RomSlotDescriptor, RomSlotKind, RomSlotState,
    RomSlotStatus, ALL_MODELS,
};

use std::cell::Cell;

pub use bus::StereoMode as AyStereoMode;
use bus::{Bus128, Bus48, BusPlus3, Kempston, KempstonMouse};
use formats::{apply_input_byte, DskImage, RzxRecording, Snapshot128, Snapshot48};
pub use tape::LD_BYTES_TRAP_PC;
use tape::{
    evaluate_ld_bytes_trap, flash_load_block, is_ld_bytes_trap_pc, TapPlayer, TapeTrapResult,
    TzxPlayer,
};
use thiserror::Error;
use ula::{
    int_active_48, int_active_pentagon, Ula48, FRAME_TSTATES_128, FRAME_TSTATES_48,
    FRAME_TSTATES_PENTAGON, INT_LENGTH_128,
};

/// Errors attaching Interface 1 or loading its shadow ROM.
#[derive(Debug, Error)]
pub enum Interface1Error {
    #[error("Interface 1 is not supported on Spectrum +2A/+3")]
    UnsupportedModel,
    #[error(transparent)]
    Rom(#[from] bus::Interface1RomError),
}

/// Advance `frame_t` by `dt` and report whether a display frame boundary was crossed.
///
/// Carrying the remainder (instead of resetting to 0) keeps IRQ-to-IRQ spacing at
/// `FRAME_TSTATES_*` on average — required by Minfo / Timing Test.
#[inline]
fn advance_frame_t(frame_t: &mut u32, dt: u32, frame_len: u32) -> bool {
    *frame_t += dt;
    if *frame_t >= frame_len {
        *frame_t -= frame_len;
        true
    } else {
        false
    }
}
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
            // TZX: progress is within the active block, not the whole schedule.
            Self::Tzx(t) => t.active_pulse_index(),
        }
    }

    #[must_use]
    pub fn pulse_count(&self) -> usize {
        match self {
            Self::Tap(t) => t.scheduled_pulses(),
            Self::Tzx(t) => t.active_pulse_count(),
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

    /// True when the deck has no remaining bitstream / TAP blocks to play.
    #[must_use]
    pub fn finished(&self) -> bool {
        match self {
            Self::Tap(t) => t.finished(),
            Self::Tzx(t) => t.finished(),
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
    ///
    /// While a tape is **playing** on the EAR path (flash-load off), each
    /// [`Machine::run_frame`] executes this many Spectrum frames so wall-clock
    /// load time ≈ realtime / speed. Pulse widths stay ROM-accurate (CPU↔tape
    /// 1:1); Instant/flash-load is unchanged (single frame per call).
    pub speed: u32,
    /// ~20s-class load: abbreviated inter-block pauses on the EAR path at
    /// [`tape::EXPERIENCE_EAR_SPEED`] (issue #82). Mutually exclusive with
    /// [`Self::flash_load`].
    pub experience_load: bool,
}

impl Default for TapeLoadOptions {
    fn default() -> Self {
        // EAR path by default; UI Instant actions enable flash-load ephemerally.
        Self {
            flash_load: false,
            speed: 1,
            experience_load: false,
        }
    }
}

impl TapeLoadOptions {
    #[must_use]
    pub fn with_speed(mut self, speed: u32) -> Self {
        self.speed = speed.clamp(1, 64);
        self.experience_load = false;
        self
    }

    #[must_use]
    pub fn experience() -> Self {
        Self {
            flash_load: false,
            speed: tape::EXPERIENCE_EAR_SPEED,
            experience_load: true,
        }
    }

    fn normalized(mut self) -> Self {
        if self.experience_load {
            self.flash_load = false;
            self.speed = tape::EXPERIENCE_EAR_SPEED;
        } else if self.flash_load {
            self.experience_load = false;
        }
        self.speed = self.speed.clamp(1, 64);
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
            // Consumed trailing zero-pulse block (e.g. 0x20 pause_ms=0): treat as done.
            if self.block_index.saturating_add(1) >= self.block_count {
                1.0
            } else {
                0.0
            }
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
    /// 16 KiB RAM, same ROM and 48K ULA timing as 48K (#188).
    Spectrum16K,
    Spectrum48,
    Spectrum128,
    /// Amstrad grey +2 (128K hardware, +2 ROM / menu).
    SpectrumPlus2,
    /// Amstrad +2A (gate array `1FFD`, no disk interface — menu Loader is tape).
    SpectrumPlus2A,
    /// Amstrad +3 (same gate array with µPD765 — menu Loader is +3DOS disk).
    SpectrumPlus3,
    /// Pentagon 128 clone (#188 Phase B / #193): user ROM + TR-DOS, distinct timing.
    Pentagon128,
}

impl Model {
    /// +2A or +3 (shared Amstrad gate array).
    #[must_use]
    pub fn is_amstrad_plus(self) -> bool {
        matches!(self, Self::SpectrumPlus2A | Self::SpectrumPlus3)
    }

    /// 128K-class bus (Sinclair 128 / grey +2 / Pentagon banking).
    #[must_use]
    pub fn is_128k_class(self) -> bool {
        matches!(
            self,
            Self::Spectrum128 | Self::SpectrumPlus2 | Self::Pentagon128
        )
    }

    /// 48K-class bus (16K / 48K).
    #[must_use]
    pub fn is_48k_class(self) -> bool {
        matches!(self, Self::Spectrum16K | Self::Spectrum48)
    }
}

/// Memory+Io adapter for 48K.
#[derive(Debug)]
pub struct MemIo48<'a> {
    pub bus: &'a mut Bus48,
    pub(crate) watch: Option<debugger::WatchHook<'a>>,
    /// `cpu.t` at the start of the current instruction (for mid-instruction ULA time).
    pub(crate) t_step_start: u64,
    /// When set, the first `read` at this PC runs IF1 pre/post opcode-fetch paging.
    pub(crate) opcode_pc: Option<u16>,
}

impl MemIo48<'_> {
    #[inline]
    fn ula_t(&self, t: u64) -> u32 {
        let dt = t.wrapping_sub(self.t_step_start) as u32;
        (self.bus.frame_t.wrapping_add(dt)) % FRAME_TSTATES_48
    }
}

impl Memory for MemIo48<'_> {
    fn read(&mut self, addr: u16, t: u64) -> (u8, u32) {
        let mut unpage_after = false;
        if self.opcode_pc == Some(addr) {
            if let Some(if1) = self.bus.interface1.as_mut() {
                if1.pre_opcode_fetch(addr);
                unpage_after = addr == 0x0700;
            }
            self.opcode_pc = None;
        }
        let wait = if Bus48::is_contended(addr) {
            ula::contention_delay_48(self.ula_t(t))
        } else {
            0
        };
        if wait > 0 && trace::enabled(trace::Category::BUS) {
            emit_contend_sampled(addr, self.ula_t(t), wait);
        }
        let v = self.bus.read(addr);
        if unpage_after {
            if let Some(if1) = self.bus.interface1.as_mut() {
                if1.post_opcode_fetch(0x0700);
            }
        }
        if let Some(w) = self.watch.as_ref() {
            w.mem_access(addr, false, v);
        }
        (v, wait)
    }

    fn write(&mut self, addr: u16, value: u8, t: u64) -> u32 {
        let wait = if Bus48::is_contended(addr) {
            ula::contention_delay_48(self.ula_t(t))
        } else {
            0
        };
        if wait > 0 && trace::enabled(trace::Category::BUS) {
            emit_contend_sampled(addr, self.ula_t(t), wait);
        }
        self.bus.write(addr, value);
        if let Some(w) = self.watch.as_ref() {
            w.mem_access(addr, true, value);
        }
        wait
    }
}

impl Io for MemIo48<'_> {
    fn in_port(&mut self, port: u16, t: u64) -> (u8, u32) {
        let ft = self.ula_t(t);
        let wait = ula::io_contention_extra_48(ft, port);
        // Z80 latches the data bus on the last T of the I/O cycle. Odd ports are
        // `N:4` (no I/O wait); even ports add FAQ waits before that last T.
        let sample = ft.wrapping_add(3).wrapping_add(wait) % FRAME_TSTATES_48;
        let saved = self.bus.frame_t;
        self.bus.frame_t = sample;
        let v = self.bus.in_port(port);
        self.bus.frame_t = saved;
        if let Some(w) = self.watch.as_ref() {
            w.port_access(port, false, v);
        }
        (v, wait)
    }

    fn out_port(&mut self, port: u16, value: u8, t: u64) -> u32 {
        let ft = self.ula_t(t);
        let wait = ula::io_contention_extra_48(ft, port);
        let saved = self.bus.frame_t;
        self.bus.frame_t = ft;
        self.bus.out_port(port, value);
        self.bus.frame_t = saved;
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
    pub(crate) t_step_start: u64,
    /// When set, the first `read` at this PC runs IF1 pre/post opcode-fetch paging.
    pub(crate) opcode_pc: Option<u16>,
    /// Pentagon 128: 71680 T/frame, no memory or I/O contention.
    pub(crate) pentagon: bool,
}

impl MemIo128<'_> {
    #[inline]
    fn frame_len(&self) -> u32 {
        if self.pentagon {
            FRAME_TSTATES_PENTAGON
        } else {
            FRAME_TSTATES_128
        }
    }

    #[inline]
    fn ula_t(&self, t: u64) -> u32 {
        let dt = t.wrapping_sub(self.t_step_start) as u32;
        (self.bus.frame_t.wrapping_add(dt)) % self.frame_len()
    }
}

impl Memory for MemIo128<'_> {
    fn read(&mut self, addr: u16, t: u64) -> (u8, u32) {
        let mut unpage_after = false;
        if self.opcode_pc == Some(addr) {
            if let Some(if1) = self.bus.interface1.as_mut() {
                if1.pre_opcode_fetch(addr);
                unpage_after = addr == 0x0700;
            }
            self.opcode_pc = None;
        }
        let ft = self.ula_t(t);
        let saved = self.bus.frame_t;
        self.bus.frame_t = ft;
        let wait = if self.pentagon {
            0
        } else {
            self.bus.contend_at(addr)
        };
        self.bus.frame_t = saved;
        if wait > 0 && trace::enabled(trace::Category::BUS) {
            emit_contend_sampled(addr, ft, wait);
        }
        let v = self.bus.read(addr);
        if unpage_after {
            if let Some(if1) = self.bus.interface1.as_mut() {
                if1.post_opcode_fetch(0x0700);
            }
        }
        if let Some(w) = self.watch.as_ref() {
            w.mem_access(addr, false, v);
        }
        (v, wait)
    }

    fn write(&mut self, addr: u16, value: u8, t: u64) -> u32 {
        let ft = self.ula_t(t);
        let saved = self.bus.frame_t;
        self.bus.frame_t = ft;
        let wait = if self.pentagon {
            0
        } else {
            self.bus.contend_at(addr)
        };
        self.bus.frame_t = saved;
        if wait > 0 && trace::enabled(trace::Category::BUS) {
            emit_contend_sampled(addr, ft, wait);
        }
        self.bus.write(addr, value);
        if let Some(w) = self.watch.as_ref() {
            w.mem_access(addr, true, value);
        }
        wait
    }
}

impl Io for MemIo128<'_> {
    fn in_port(&mut self, port: u16, t: u64) -> (u8, u32) {
        let ft = self.ula_t(t);
        let wait = if self.pentagon {
            0
        } else {
            ula::io_contention_extra_128(ft, port, self.bus.c000_contended())
        };
        let sample = ft.wrapping_add(3).wrapping_add(wait) % self.frame_len();
        let saved = self.bus.frame_t;
        self.bus.frame_t = sample;
        let v = self.bus.in_port(port);
        self.bus.frame_t = saved;
        if let Some(w) = self.watch.as_ref() {
            w.port_access(port, false, v);
        }
        (v, wait)
    }

    fn out_port(&mut self, port: u16, value: u8, t: u64) -> u32 {
        let ft = self.ula_t(t);
        let wait = if self.pentagon {
            0
        } else {
            ula::io_contention_extra_128(ft, port, self.bus.c000_contended())
        };
        let saved = self.bus.frame_t;
        self.bus.frame_t = ft;
        self.bus.out_port(port, value);
        self.bus.frame_t = saved;
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
    pub(crate) t_step_start: u64,
}

impl MemIoPlus3<'_> {
    #[inline]
    fn ula_t(&self, t: u64) -> u32 {
        let dt = t.wrapping_sub(self.t_step_start) as u32;
        (self.bus.frame_t.wrapping_add(dt)) % FRAME_TSTATES_128
    }
}

impl Memory for MemIoPlus3<'_> {
    fn read(&mut self, addr: u16, t: u64) -> (u8, u32) {
        let ft = self.ula_t(t);
        let saved = self.bus.frame_t;
        self.bus.frame_t = ft;
        let wait = self.bus.contend_at(addr);
        self.bus.frame_t = saved;
        if wait > 0 && trace::enabled(trace::Category::BUS) {
            emit_contend_sampled(addr, ft, wait);
        }
        let v = self.bus.read(addr);
        if let Some(w) = self.watch.as_ref() {
            w.mem_access(addr, false, v);
        }
        (v, wait)
    }

    fn write(&mut self, addr: u16, value: u8, t: u64) -> u32 {
        let ft = self.ula_t(t);
        let saved = self.bus.frame_t;
        self.bus.frame_t = ft;
        let wait = self.bus.contend_at(addr);
        self.bus.frame_t = saved;
        if wait > 0 && trace::enabled(trace::Category::BUS) {
            emit_contend_sampled(addr, ft, wait);
        }
        self.bus.write(addr, value);
        if let Some(w) = self.watch.as_ref() {
            w.mem_access(addr, true, value);
        }
        wait
    }
}

impl Io for MemIoPlus3<'_> {
    fn in_port(&mut self, port: u16, t: u64) -> (u8, u32) {
        // +2A/+3 gate array: no Sinclair-style ULA I/O contention. FDC ports
        // `2FFD`/`3FFD` are on the gate array as well (wait=0). Confirmed for
        // +3DOS; no accuracy follow-up from #141.
        let _ = (port, t);
        let v = self.bus.in_port(port);
        if let Some(w) = self.watch.as_ref() {
            w.port_access(port, false, v);
        }
        (v, 0)
    }

    fn out_port(&mut self, port: u16, value: u8, t: u64) -> u32 {
        let _ = t;
        self.bus.out_port(port, value);
        if let Some(w) = self.watch.as_ref() {
            w.port_access(port, true, value);
        }
        0
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
        /// Grey +2 uses the same 128K core with a distinct ROM / menu.
        plus2_rom: bool,
        /// Pentagon 128 clone timing / no contention (#188 Phase B).
        pentagon: bool,
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
    /// Left AY channel (same length as `ay_samples`; empty on 48K).
    pub ay_left: Vec<f32>,
    /// Right AY channel (same length as `ay_samples`; empty on 48K).
    pub ay_right: Vec<f32>,
}

fn push_ay_frame_sample(
    ay: &bus::Ay8912,
    mono: &mut Vec<f32>,
    left: &mut Vec<f32>,
    right: &mut Vec<f32>,
) {
    let (l, r) = ay.sample_stereo();
    left.push(l);
    right.push(r);
    mono.push(ay.sample_mono());
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

    /// Spectrum 16K: 48K ULA / bus timing, 16 KiB RAM only (#188).
    pub fn new_16k(rom: &[u8]) -> Result<Self, String> {
        let mut bus = Bus48::new();
        bus.ram16k = true;
        bus.load_rom(rom)?;
        trace::emit(trace::EventKind::MachineModel { model: 5 });
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
            plus2_rom: false,
            pentagon: false,
        })
    }

    /// Amstrad grey +2: 128K hardware with `roms/plus2/` ROM (#188).
    pub fn new_plus2(rom: &[u8]) -> Result<Self, String> {
        let mut bus = Bus128::new();
        bus.load_rom128(rom)?;
        trace::emit(trace::EventKind::MachineModel { model: 4 });
        Ok(Self::Spec128 {
            cpu: Cpu::new(),
            bus: Box::new(bus),
            ula: Ula48::new(),
            tape: None,
            tape_opts: TapeLoadOptions::default(),
            rzx: None,
            debugger: Debugger::default(),
            plus2_rom: true,
            pentagon: false,
        })
    }

    /// Pentagon 128: 128K banking, user main ROM + TR-DOS (#188 Phase B / #193).
    pub fn new_pentagon128(main_rom: &[u8], trdos_rom: &[u8]) -> Result<Self, String> {
        let mut bus = Bus128::new();
        bus.load_rom128(main_rom)?;
        trace::emit(trace::EventKind::MachineModel { model: 6 });
        let mut m = Self::Spec128 {
            cpu: Cpu::new(),
            bus: Box::new(bus),
            ula: Ula48::new(),
            tape: None,
            tape_opts: TapeLoadOptions::default(),
            rzx: None,
            debugger: Debugger::default(),
            plus2_rom: false,
            pentagon: true,
        };
        m.attach_beta()?.load_rom(trdos_rom)?;
        Ok(m)
    }

    pub fn new_plus3(rom: &[u8]) -> Result<Self, String> {
        Self::new_amstrad_plus(rom, true)
    }

    /// Spectrum +2A: same gate array as +3 but FDC ports float (`disk_interface = false`).
    pub fn new_plus2a(rom: &[u8]) -> Result<Self, String> {
        Self::new_amstrad_plus(rom, false)
    }

    fn new_amstrad_plus(rom: &[u8], disk_interface: bool) -> Result<Self, String> {
        let mut bus = BusPlus3::new_with_disk(disk_interface);
        bus.load_rom64(rom)?;
        let model_id = if disk_interface { 2 } else { 3 };
        trace::emit(trace::EventKind::MachineModel { model: model_id });
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
            Self::Spec48 { bus, .. } => {
                if bus.ram16k {
                    Model::Spectrum16K
                } else {
                    Model::Spectrum48
                }
            }
            Self::Spec128 { pentagon: true, .. } => Model::Pentagon128,
            Self::Spec128 {
                plus2_rom: true, ..
            } => Model::SpectrumPlus2,
            Self::Spec128 { .. } => Model::Spectrum128,
            Self::SpecPlus3 { bus, .. } => {
                if bus.disk_interface {
                    Model::SpectrumPlus3
                } else {
                    Model::SpectrumPlus2A
                }
            }
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
                bus.mouse.reset();
                if let Some(mf) = bus.multiface.as_mut() {
                    mf.reset();
                }
                if let Some(if1) = bus.interface1.as_mut() {
                    if1.page_rom(false);
                }
                if let Some(beta) = bus.beta.as_mut() {
                    beta.page_trdos(false);
                }
                if let Some(div) = bus.divmmc.as_mut() {
                    div.reset_soft();
                }
                *ula = Ula48::new();
                // Keep inserted tape/disk media across reset; pause the deck at its
                // current position. RZX input playback is cleared (machine state diverges).
                if let Some(t) = tape.as_mut() {
                    t.set_playing(false);
                }
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
                bus.mouse.reset();
                if let Some(if1) = bus.interface1.as_mut() {
                    if1.page_rom(false);
                }
                if let Some(beta) = bus.beta.as_mut() {
                    beta.page_trdos(false);
                }
                if let Some(div) = bus.divmmc.as_mut() {
                    div.reset_soft();
                }
                *ula = Ula48::new();
                if let Some(t) = tape.as_mut() {
                    t.set_playing(false);
                }
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
                bus.mouse.reset();
                *ula = Ula48::new();
                // +3 DSK stays in `bus.fdc.image`; reset µPD765 command state.
                bus.fdc.reset_controller();
                bus.fdc.set_motor(false);
                if let Some(t) = tape.as_mut() {
                    t.set_playing(false);
                }
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
        let opts = opts.normalized();
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
                    p.set_experience(opts.experience_load);
                }
            }
        }
        trace::emit(trace::EventKind::MachineLoadMode {
            flash_load: opts.flash_load,
            speed: opts.speed as u8,
            experience_load: opts.experience_load,
        });
    }

    pub fn insert_tape(&mut self, mut player: TapPlayer) {
        player.set_playing(false);
        let opts = self.tape_load_options();
        player.set_speed(opts.speed);
        player.set_experience(opts.experience_load);
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
            Self::SpecPlus3 { bus, .. } if bus.disk_interface => {
                bus.fdc.insert(image);
                Ok(())
            }
            Self::SpecPlus3 { .. } => {
                Err("+2A has no disk interface — use Spectrum +3 for DSK".into())
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

    pub fn mouse_mut(&mut self) -> &mut KempstonMouse {
        match self {
            Self::Spec48 { bus, .. } => &mut bus.mouse,
            Self::Spec128 { bus, .. } => &mut bus.mouse,
            Self::SpecPlus3 { bus, .. } => &mut bus.mouse,
        }
    }

    /// Set AY stereo pan mode (no-op on 48K).
    pub fn set_ay_stereo_mode(&mut self, mode: bus::StereoMode) {
        match self {
            Self::Spec48 { .. } => {}
            Self::Spec128 { bus, .. } => {
                bus.ay.stereo_mode = mode;
            }
            Self::SpecPlus3 { bus, .. } => {
                bus.ay.stereo_mode = mode;
            }
        }
    }

    #[must_use]
    pub fn ay_stereo_mode(&self) -> bus::StereoMode {
        match self {
            Self::Spec48 { .. } => bus::StereoMode::Mono,
            Self::Spec128 { bus, .. } => bus.ay.stereo_mode,
            Self::SpecPlus3 { bus, .. } => bus.ay.stereo_mode,
        }
    }

    /// Apply a host joystick under `mode` (clears prior joystick matrix/Kempston first).
    pub fn apply_joystick_state(&mut self, mode: JoystickMode, state: JoystickState) {
        let (k, kb) = match self {
            Self::Spec48 { bus, .. } => (&mut bus.kempston, &mut bus.keyboard),
            Self::Spec128 { bus, .. } => (&mut bus.kempston, &mut bus.keyboard),
            Self::SpecPlus3 { bus, .. } => (&mut bus.kempston, &mut bus.keyboard),
        };
        apply_joystick(mode, state, k, kb);
    }

    /// Clear Kempston and all matrix keys used by joystick modes.
    pub fn clear_joystick_state(&mut self) {
        let (k, kb) = match self {
            Self::Spec48 { bus, .. } => (&mut bus.kempston, &mut bus.keyboard),
            Self::Spec128 { bus, .. } => (&mut bus.kempston, &mut bus.keyboard),
            Self::SpecPlus3 { bus, .. } => (&mut bus.kempston, &mut bus.keyboard),
        };
        k.reset();
        clear_joystick_matrix(kb);
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
        match self {
            Self::Spec48 { bus, ula, .. } => {
                bus.border = snap.border;
                bus.ula.border = snap.border;
                ula.border = snap.border;
            }
            Self::Spec128 { bus, ula, .. } => {
                bus.border = snap.border;
                bus.ula.border = snap.border;
                ula.border = snap.border;
            }
            Self::SpecPlus3 { bus, ula, .. } => {
                bus.border = snap.border;
                bus.ula.border = snap.border;
                ula.border = snap.border;
            }
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

    /// Attach Multiface 1 with an 8 KiB ROM image (48K only).
    pub fn attach_multiface(&mut self, rom: &[u8]) -> Result<(), String> {
        match self {
            Self::Spec48 { bus, .. } => bus.attach_multiface(rom),
            _ => Err("Multiface 1 is only supported on Spectrum 48K".into()),
        }
    }

    /// Press Multiface 1 red button (if attached) and raise NMI.
    ///
    /// Asserts NMI pending, runs the Z80 NMI sequence to `0x0066`, then pages MF
    /// ROM/RAM over `0000–3FFF` (vector-fetch latch). Returns NMI T-states, or
    /// `None` if MF is absent. A second press while NMI is still pending is ignored
    /// (returns `Some(0)`).
    pub fn multiface_nmi(&mut self) -> Option<u32> {
        match self {
            Self::Spec48 { cpu, bus, .. } => {
                let mf = bus.multiface.as_mut()?;
                if !mf.press_button() {
                    return Some(0);
                }
                let t_step_start = cpu.t;
                let dt = {
                    let mut mio = MemIo48 {
                        bus: bus.as_mut(),
                        watch: None,
                        t_step_start,
                        opcode_pc: None,
                    };
                    cpu.nmi(&mut mio)
                };
                bus.advance_frame_t(dt);
                if let Some(mf) = bus.multiface.as_mut() {
                    mf.page_on_nmi_vector();
                }
                Some(dt)
            }
            _ => None,
        }
    }

    /// Attach DivMMC on 48K/128K (creates the peripheral if absent).
    pub fn attach_divmmc(&mut self) -> Result<&mut bus::DivMmc, String> {
        match self {
            Self::Spec48 { bus, .. } => Ok(bus.attach_divmmc()),
            Self::Spec128 { bus, .. } => Ok(bus.attach_divmmc()),
            Self::SpecPlus3 { .. } => Err("DivMMC is not supported on Spectrum +2A/+3".into()),
        }
    }

    /// Attach DivMMC and load an ESXDOS EEPROM image (8 KiB, or larger prefix).
    pub fn attach_divmmc_eeprom(&mut self, data: &[u8]) -> Result<(), String> {
        let div = self.attach_divmmc()?;
        div.attach_eeprom(data)
    }

    pub fn divmmc_mut(&mut self) -> Option<&mut bus::DivMmc> {
        match self {
            Self::Spec48 { bus, .. } => bus.divmmc.as_mut(),
            Self::Spec128 { bus, .. } => bus.divmmc.as_mut(),
            Self::SpecPlus3 { .. } => None,
        }
    }

    #[must_use]
    pub fn has_divmmc(&self) -> bool {
        match self {
            Self::Spec48 { bus, .. } => bus.divmmc.is_some(),
            Self::Spec128 { bus, .. } => bus.divmmc.is_some(),
            Self::SpecPlus3 { .. } => false,
        }
    }

    #[must_use]
    pub fn has_divmmc_eeprom(&self) -> bool {
        match self {
            Self::Spec48 { bus, .. } => bus.divmmc.as_ref().is_some_and(|d| d.eeprom_loaded),
            Self::Spec128 { bus, .. } => bus.divmmc.as_ref().is_some_and(|d| d.eeprom_loaded),
            Self::SpecPlus3 { .. } => false,
        }
    }

    /// Attach Interface 1 on 48K/128K.
    pub fn attach_interface1(&mut self) -> Result<&mut bus::Interface1, Interface1Error> {
        match self {
            Self::Spec48 { bus, .. } => Ok(bus.attach_interface1()),
            Self::Spec128 { bus, .. } => Ok(bus.attach_interface1()),
            Self::SpecPlus3 { .. } => Err(Interface1Error::UnsupportedModel),
        }
    }

    pub fn interface1_mut(&mut self) -> Option<&mut bus::Interface1> {
        match self {
            Self::Spec48 { bus, .. } => bus.interface1.as_mut(),
            Self::Spec128 { bus, .. } => bus.interface1.as_mut(),
            Self::SpecPlus3 { .. } => None,
        }
    }

    #[must_use]
    pub fn has_interface1(&self) -> bool {
        match self {
            Self::Spec48 { bus, .. } => bus.interface1.is_some(),
            Self::Spec128 { bus, .. } => bus.interface1.is_some(),
            Self::SpecPlus3 { .. } => false,
        }
    }

    /// Load an 8 KiB Interface 1 ROM into the attached peripheral (creates IF1 if needed).
    pub fn load_interface1_rom(&mut self, data: &[u8]) -> Result<(), Interface1Error> {
        let if1 = self.attach_interface1()?;
        if1.load_rom(data)?;
        Ok(())
    }

    /// True when IF1 is attached and an 8K ROM image has been loaded.
    #[must_use]
    pub fn interface1_rom_loaded(&self) -> bool {
        match self {
            Self::Spec48 { bus, .. } => bus.interface1.as_ref().is_some_and(|i| i.rom_loaded),
            Self::Spec128 { bus, .. } => bus.interface1.as_ref().is_some_and(|i| i.rom_loaded),
            Self::SpecPlus3 { .. } => false,
        }
    }

    /// Attach Beta Disk / TR-DOS on 48K/128K.
    pub fn attach_beta(&mut self) -> Result<&mut bus::BetaDisk, String> {
        match self {
            Self::Spec48 { bus, .. } => Ok(bus.attach_beta()),
            Self::Spec128 { bus, .. } => Ok(bus.attach_beta()),
            Self::SpecPlus3 { .. } => Err("Beta Disk is not supported on Spectrum +2A/+3".into()),
        }
    }

    /// Insert a `.trd` image (attaches Beta if needed). 48K/128K only.
    pub fn insert_trd(&mut self, image: formats::TrdImage) -> Result<(), String> {
        self.attach_beta()?.insert(image);
        Ok(())
    }

    /// Load a 16 KiB TR-DOS ROM onto Beta (attaches the interface if needed).
    pub fn load_trdos_rom(&mut self, data: &[u8]) -> Result<(), String> {
        self.attach_beta()?.load_rom(data)
    }

    pub fn beta_mut(&mut self) -> Option<&mut bus::BetaDisk> {
        match self {
            Self::Spec48 { bus, .. } => bus.beta.as_mut(),
            Self::Spec128 { bus, .. } => bus.beta.as_mut(),
            Self::SpecPlus3 { .. } => None,
        }
    }

    #[must_use]
    pub fn has_beta(&self) -> bool {
        match self {
            Self::Spec48 { bus, .. } => bus.beta.is_some(),
            Self::Spec128 { bus, .. } => bus.beta.is_some(),
            Self::SpecPlus3 { .. } => false,
        }
    }

    #[must_use]
    pub fn has_multiface(&self) -> bool {
        match self {
            Self::Spec48 { bus, .. } => bus.multiface.is_some(),
            _ => false,
        }
    }

    /// Apply a 128K / +2A/+3 banked snapshot (SNA128 or Z80 v2/v3).
    ///
    /// On `Spec48`, maps banks 5/2/`page_7ffd&7` into the 48K address space.
    /// On `SpecPlus3`, also restores `0x1FFD` when present in the snapshot.
    pub fn apply_snapshot128(&mut self, snap: &Snapshot128) {
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
        match self {
            Self::Spec48 { bus, ula, .. } => {
                let paged = usize::from(snap.page_7ffd & 7);
                bus.ram[..16384].copy_from_slice(&snap.banks[5]);
                bus.ram[16384..32768].copy_from_slice(&snap.banks[2]);
                bus.ram[32768..49152].copy_from_slice(&snap.banks[paged]);
                bus.border = snap.border;
                bus.ula.border = snap.border;
                ula.border = snap.border;
            }
            Self::Spec128 { bus, ula, .. } => {
                for (i, bank) in snap.banks.iter().enumerate() {
                    bus.banks[i].copy_from_slice(bank);
                }
                bus.locked = false;
                bus.out_7ffd(snap.page_7ffd);
                bus.border = snap.border;
                bus.ula.border = snap.border;
                ula.border = snap.border;
            }
            Self::SpecPlus3 { bus, ula, .. } => {
                for (i, bank) in snap.banks.iter().enumerate() {
                    bus.banks[i].copy_from_slice(bank);
                }
                bus.locked = false;
                // Apply 1FFD before 7FFD so a paging-lock bit cannot block it.
                if let Some(p) = snap.page_1ffd {
                    bus.out_1ffd(p);
                } else {
                    bus.out_1ffd(0);
                }
                bus.out_7ffd(snap.page_7ffd);
                bus.border = snap.border;
                bus.ula.border = snap.border;
                ula.border = snap.border;
            }
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
    /// Spectrum frames to run per [`Self::run_frame`] while EAR tape is playing.
    ///
    /// Speed multiplies wall-clock progress with CPU↔tape still 1:1 (ROM LD-BYTES
    /// stays locked). Flash-load / Instant keeps a single frame so traps stay snappy.
    /// At `speed == 1` this is always 1 — hardware-accurate path unchanged.
    #[must_use]
    fn ear_play_frame_reps(&self) -> u32 {
        let opts = self.tape_load_options();
        // Turbo only while EAR is actively playing a non-finished deck (#178).
        if opts.flash_load || !self.tape_playing() || self.tape_finished() {
            1
        } else {
            opts.speed.clamp(1, 64)
        }
    }

    /// True when an inserted deck has exhausted its bitstream / blocks.
    #[must_use]
    pub fn tape_finished(&self) -> bool {
        match self {
            Self::Spec48 { tape, .. }
            | Self::Spec128 { tape, .. }
            | Self::SpecPlus3 { tape, .. } => tape.as_ref().is_some_and(TapeDeck::finished),
        }
    }

    /// Run one or more Spectrum frames. While an EAR deck is playing, runs
    /// [`TapeLoadOptions::speed`] frames so wall-clock ≈ realtime / speed.
    /// Only the last inner frame's PCM/edges are returned (hosts should not try
    /// to play S seconds of audio in one tick).
    pub fn run_frame(&mut self) -> FrameAudio {
        let reps = self.ear_play_frame_reps();
        let mut audio = self.run_one_frame();
        for _ in 1..reps {
            if self.debugger().paused || !self.tape_playing() {
                break;
            }
            audio = self.run_one_frame();
        }
        audio
    }

    fn run_one_frame(&mut self) -> FrameAudio {
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
                // Keep any overshoot remainder from the previous frame (do not zero).
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
                let mut frame_done = false;
                while !frame_done && !broke_on_pc {
                    if debugger.check_pc(cpu.regs.pc) {
                        break;
                    }
                    if Self::hold_ld_bytes_until_play(cpu.regs.pc, tape, |a| bus.read(a)) {
                        const HOLD_T: u32 = 4;
                        cpu.t = cpu.t.wrapping_add(u64::from(HOLD_T));
                        last_t = cpu.t;
                        frame_done = advance_frame_t(&mut bus.frame_t, HOLD_T, FRAME_TSTATES_48);
                        continue;
                    }
                    if tape_opts.flash_load && Self::try_flash_load_48(cpu, bus, tape) {
                        continue;
                    }
                    if int_active_48(bus.frame_t) {
                        let mut mio = MemIo48 {
                            bus: bus.as_mut(),
                            watch: None,
                            t_step_start: cpu.t,
                            opcode_pc: None,
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
                            // INT only near t=0; wrap is vanishingly rare but keep carry semantics.
                            frame_done = advance_frame_t(&mut bus.frame_t, irq_t, FRAME_TSTATES_48);
                            last_t = cpu.t;
                            continue;
                        }
                    }
                    let pc = cpu.regs.pc;
                    bus.notify_divmmc_m1(pc);
                    bus.notify_beta_m1(pc);
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
                            t_step_start: cpu.t,
                            opcode_pc: Some(pc),
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
                    frame_done = advance_frame_t(&mut bus.frame_t, dt, FRAME_TSTATES_48);
                }
                // Keep border_events for render; next run_frame begin_frame clears them.
                FrameAudio {
                    beeper_edges: std::mem::take(&mut bus.beeper_edges),
                    ay_samples: Vec::new(),
                    ay_left: Vec::new(),
                    ay_right: Vec::new(),
                }
            }
            Self::Spec128 {
                cpu,
                bus,
                ula,
                tape,
                tape_opts,
                debugger,
                pentagon,
                ..
            } => {
                let is_pentagon = *pentagon;
                let frame_len = if is_pentagon {
                    FRAME_TSTATES_PENTAGON
                } else {
                    FRAME_TSTATES_128
                };
                bus.beeper_edges.clear();
                // Keep any overshoot remainder from the previous frame (do not zero).
                bus.ula.border = bus.border;
                bus.ula.begin_frame();
                ula.border = bus.border;
                ula.begin_frame();
                if trace::enabled(trace::Category::ULA) {
                    let frame = next_frame_n();
                    trace::emit(trace::EventKind::UlaFrame { frame });
                }
                const AY_SAMPLES: usize = 882; // ~44100 Hz / 50 Hz
                let t_per_sample = f64::from(frame_len) / AY_SAMPLES as f64;
                let mut ay_samples = Vec::with_capacity(AY_SAMPLES);
                let mut ay_left = Vec::with_capacity(AY_SAMPLES);
                let mut ay_right = Vec::with_capacity(AY_SAMPLES);
                let mut ay_t = 0u32;
                let mut last_t = cpu.t;
                let mut broke_on_pc = false;
                let mut frame_done = false;
                while !frame_done && !broke_on_pc {
                    if debugger.check_pc(cpu.regs.pc) {
                        break;
                    }
                    if Self::hold_ld_bytes_until_play(cpu.regs.pc, tape, |a| bus.read(a)) {
                        const HOLD_T: u32 = 4;
                        cpu.t = cpu.t.wrapping_add(u64::from(HOLD_T));
                        last_t = cpu.t;
                        ay_t = ay_t.saturating_add(HOLD_T);
                        frame_done = advance_frame_t(&mut bus.frame_t, HOLD_T, frame_len);
                        continue;
                    }
                    if tape_opts.flash_load && Self::try_flash_load_128(cpu, bus, tape) {
                        continue;
                    }
                    let int_window = if is_pentagon {
                        int_active_pentagon(bus.frame_t)
                    } else {
                        bus.frame_t < INT_LENGTH_128
                    };
                    if int_window {
                        let mut mio = MemIo128 {
                            bus: bus.as_mut(),
                            watch: None,
                            t_step_start: cpu.t,
                            opcode_pc: None,
                            pentagon: is_pentagon,
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
                            ay_t = ay_t.saturating_add(irq_t);
                            frame_done = advance_frame_t(&mut bus.frame_t, irq_t, frame_len);
                            while ay_samples.len() < AY_SAMPLES
                                && f64::from(ay_t.min(frame_len))
                                    >= (ay_samples.len() as f64 + 1.0) * t_per_sample
                            {
                                push_ay_frame_sample(
                                    &bus.ay,
                                    &mut ay_samples,
                                    &mut ay_left,
                                    &mut ay_right,
                                );
                            }
                            last_t = cpu.t;
                            continue;
                        }
                    }
                    let pc = cpu.regs.pc;
                    bus.notify_divmmc_m1(pc);
                    bus.notify_beta_m1(pc);
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
                            t_step_start: cpu.t,
                            opcode_pc: Some(pc),
                            pentagon: is_pentagon,
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
                    ay_t = ay_t.saturating_add(dt);
                    frame_done = advance_frame_t(&mut bus.frame_t, dt, frame_len);
                    while ay_samples.len() < AY_SAMPLES
                        && f64::from(ay_t.min(frame_len))
                            >= (ay_samples.len() as f64 + 1.0) * t_per_sample
                    {
                        push_ay_frame_sample(&bus.ay, &mut ay_samples, &mut ay_left, &mut ay_right);
                    }
                }
                while ay_samples.len() < AY_SAMPLES {
                    push_ay_frame_sample(&bus.ay, &mut ay_samples, &mut ay_left, &mut ay_right);
                }
                FrameAudio {
                    beeper_edges: std::mem::take(&mut bus.beeper_edges),
                    ay_samples,
                    ay_left,
                    ay_right,
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
                // Keep any overshoot remainder from the previous frame (do not zero).
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
                let mut ay_left = Vec::with_capacity(AY_SAMPLES);
                let mut ay_right = Vec::with_capacity(AY_SAMPLES);
                let mut ay_t = 0u32;
                let mut last_t = cpu.t;
                let mut broke_on_pc = false;
                let mut frame_done = false;
                while !frame_done && !broke_on_pc {
                    if debugger.check_pc(cpu.regs.pc) {
                        break;
                    }
                    if Self::hold_ld_bytes_until_play(cpu.regs.pc, tape, |a| bus.read(a)) {
                        const HOLD_T: u32 = 4;
                        cpu.t = cpu.t.wrapping_add(u64::from(HOLD_T));
                        last_t = cpu.t;
                        ay_t = ay_t.saturating_add(HOLD_T);
                        frame_done = advance_frame_t(&mut bus.frame_t, HOLD_T, FRAME_TSTATES_128);
                        continue;
                    }
                    if tape_opts.flash_load && Self::try_flash_load_plus3(cpu, bus, tape) {
                        continue;
                    }
                    if bus.frame_t < INT_LENGTH_128 {
                        let mut mio = MemIoPlus3 {
                            bus: bus.as_mut(),
                            watch: None,
                            t_step_start: cpu.t,
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
                            ay_t = ay_t.saturating_add(irq_t);
                            frame_done =
                                advance_frame_t(&mut bus.frame_t, irq_t, FRAME_TSTATES_128);
                            while ay_samples.len() < AY_SAMPLES
                                && f64::from(ay_t.min(FRAME_TSTATES_128))
                                    >= (ay_samples.len() as f64 + 1.0) * t_per_sample
                            {
                                push_ay_frame_sample(
                                    &bus.ay,
                                    &mut ay_samples,
                                    &mut ay_left,
                                    &mut ay_right,
                                );
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
                            t_step_start: cpu.t,
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
                    ay_t = ay_t.saturating_add(dt);
                    frame_done = advance_frame_t(&mut bus.frame_t, dt, FRAME_TSTATES_128);
                    while ay_samples.len() < AY_SAMPLES
                        && f64::from(ay_t.min(FRAME_TSTATES_128))
                            >= (ay_samples.len() as f64 + 1.0) * t_per_sample
                    {
                        push_ay_frame_sample(&bus.ay, &mut ay_samples, &mut ay_left, &mut ay_right);
                    }
                }
                while ay_samples.len() < AY_SAMPLES {
                    push_ay_frame_sample(&bus.ay, &mut ay_samples, &mut ay_left, &mut ay_right);
                }
                if !bus.disk_interface {
                    if let Some(TapeDeck::Tap(player)) = tape.as_ref() {
                        Self::plus2a_repair_menu_loader_stack_if_needed(bus, cpu, player);
                    }
                }
                FrameAudio {
                    beeper_edges: std::mem::take(&mut bus.beeper_edges),
                    ay_samples,
                    ay_left,
                    ay_right,
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
        // CPU↔tape 1:1 with ROM-accurate pulse widths. Wall-clock turbo is
        // [`Machine::ear_play_frame_reps`] (multiple Spectrum frames per host tick).
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
                if !bus.disk_interface {
                    Self::plus2a_repair_menu_loader_stack_if_needed(bus, cpu, player);
                }
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

    /// #179: +2A menu Loader + CLEAR 32767 games leave `0x0038` where the
    /// 48K FP calculator expects the `0xFFFF` end-marker (and editor return
    /// addresses instead of MAIN). Detect the verified Loader stack state and
    /// rewrite the post-CLEAR stack words to the 48 BASIC equivalents so
    /// Instant/EAR auto-run works. Requires SP in the CLEAR-32767 window, PC
    /// still in ROM, RAMTOP=`0x7FFF`, marker `0x0038` at `$7FEC`, and `$7FFC`
    /// not already the repaired MAIN return (`0x1303`).
    fn plus2a_repair_menu_loader_stack_if_needed(
        bus: &mut BusPlus3,
        cpu: &Cpu,
        _player: &TapPlayer,
    ) {
        // Instant flash often exits with SP=$7FDC; EAR LD-BYTES sits ~$7FE4..$7FE8.
        // Editor / +3DOS stacks live up near `$FFxx` — never rewrite those.
        let sp = cpu.regs.sp;
        if !(0x7FD0..=0x7FF0).contains(&sp) {
            return;
        }
        if cpu.regs.pc >= 0x4000 {
            return;
        }
        // CLEAR 32767 → RAMTOP=$7FFF. Skip pre-CLEAR Instant traps (RAMTOP still low).
        let ramtop = u16::from_le_bytes([bus.read(0x5CB2), bus.read(0x5CB3)]);
        if ramtop != 0x7FFF {
            return;
        }
        let marker = u16::from_le_bytes([bus.read(0x7FEC), bus.read(0x7FED)]);
        if marker != 0x0038 {
            return;
        }
        // Already repaired / 48 BASIC Instant leaves MAIN return at $7FFC.
        let ret_chain = u16::from_le_bytes([bus.read(0x7FFC), bus.read(0x7FFD)]);
        if ret_chain == 0x1303 {
            return;
        }
        bus.write(0x7FEC, 0xFF);
        bus.write(0x7FED, 0xFF);
        // Snapshot from a working 48 BASIC Instant load of Deathchase at the
        // same CLEAR 32767 / USR / LD-BYTES depth (SP=7FE6). Absolute addresses
        // are stable for RAMTOP=7FFF PROGRAM+CODE loaders.
        const WORDS: &[(u16, u16)] = &[
            (0x7FCC, 0x02DB),
            (0x7FCE, 0x3873),
            (0x7FD0, 0x5DB8),
            (0x7FD2, 0x004D),
            (0x7FD4, 0x52C7),
            (0x7FD6, 0x0039),
            (0x7FD8, 0x52C6),
            (0x7FDA, 0x020C),
            (0x7FDC, 0x0E5C),
            (0x7FF2, 0x0009),
            (0x7FF6, 0x1C10),
            (0x7FF8, 0x1B52),
            (0x7FFA, 0x1B76),
            (0x7FFC, 0x1303),
        ];
        for &(addr, val) in WORDS {
            bus.write(addr, (val & 0xFF) as u8);
            bus.write(addr.wrapping_add(1), (val >> 8) as u8);
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
                        t_step_start: cpu.t,
                        opcode_pc: None,
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
                bus.notify_divmmc_m1(pc);
                bus.notify_beta_m1(pc);
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
                        t_step_start: cpu.t,
                        opcode_pc: Some(pc),
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
                pentagon,
                ..
            } => {
                let is_pentagon = *pentagon;
                let frame_len = if is_pentagon {
                    FRAME_TSTATES_PENTAGON
                } else {
                    FRAME_TSTATES_128
                };
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
                    bus.frame_t = (bus.frame_t + HOLD_T) % frame_len;
                    cpu.t = cpu.t.wrapping_add(u64::from(HOLD_T));
                    return;
                }
                if tape_opts.flash_load && Self::try_flash_load_128(cpu, bus, tape) {
                    return;
                }
                let int_window = if is_pentagon {
                    int_active_pentagon(bus.frame_t)
                } else {
                    bus.frame_t < INT_LENGTH_128
                };
                if int_window {
                    let mut mio = MemIo128 {
                        bus: bus.as_mut(),
                        watch: None,
                        t_step_start: cpu.t,
                        opcode_pc: None,
                        pentagon: is_pentagon,
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
                        bus.frame_t = (bus.frame_t + irq_t) % frame_len;
                        return;
                    }
                }
                let pc = cpu.regs.pc;
                bus.notify_divmmc_m1(pc);
                bus.notify_beta_m1(pc);
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
                        t_step_start: cpu.t,
                        opcode_pc: Some(pc),
                        pentagon: is_pentagon,
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
                bus.frame_t = (bus.frame_t + dt) % frame_len;
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
                if !bus.disk_interface {
                    if let Some(TapeDeck::Tap(player)) = tape.as_ref() {
                        Self::plus2a_repair_menu_loader_stack_if_needed(bus, cpu, player);
                    }
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
                        t_step_start: cpu.t,
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
                        t_step_start: cpu.t,
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
                bus.notify_divmmc_m1(cpu.regs.pc);
                bus.notify_beta_m1(cpu.regs.pc);
                let last_t = cpu.t;
                let mut mio = MemIo48 {
                    bus: bus.as_mut(),
                    watch: None,
                    t_step_start: cpu.t,
                    opcode_pc: Some(cpu.regs.pc),
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
                pentagon,
                ..
            } => {
                let is_pentagon = *pentagon;
                let frame_len = if is_pentagon {
                    FRAME_TSTATES_PENTAGON
                } else {
                    FRAME_TSTATES_128
                };
                bus.notify_divmmc_m1(cpu.regs.pc);
                bus.notify_beta_m1(cpu.regs.pc);
                let last_t = cpu.t;
                let mut mio = MemIo128 {
                    bus: bus.as_mut(),
                    watch: None,
                    t_step_start: cpu.t,
                    opcode_pc: Some(cpu.regs.pc),
                    pentagon: is_pentagon,
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
                bus.frame_t = (bus.frame_t + dt) % frame_len;
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
                    t_step_start: cpu.t,
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
            Self::Spec48 { bus, .. } => {
                Self::set_ear_mixed(
                    &mut bus.ear,
                    bus.beeper,
                    bus.frame_t,
                    &mut bus.beeper_edges,
                    level,
                );
            }
            Self::Spec128 { bus, .. } => {
                Self::set_ear_mixed(
                    &mut bus.ear,
                    bus.beeper,
                    bus.frame_t,
                    &mut bus.beeper_edges,
                    level,
                );
            }
            Self::SpecPlus3 { bus, .. } => {
                Self::set_ear_mixed(
                    &mut bus.ear,
                    bus.beeper,
                    bus.frame_t,
                    &mut bus.beeper_edges,
                    level,
                );
            }
        }
    }

    fn set_ear_mixed(
        ear: &mut bool,
        beeper: bool,
        frame_t: u32,
        edges: &mut Vec<(u32, bool)>,
        level: bool,
    ) {
        if *ear == level {
            return;
        }
        *ear = level;
        let mixed = level || beeper;
        if edges.last().map(|&(_, l)| l) != Some(mixed) {
            edges.push((frame_t, mixed));
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
    ///
    /// Do **not** use menu **Loader** here — that is +3DOS disk.
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

    /// +2A menu: **Loader** is tape (no disk interface). Enter alone for PROGRAM;
    /// `LOAD "" CODE` still goes via 48 BASIC.
    pub fn type_load_quotes_plus2a(&mut self, with_code: bool) {
        if with_code {
            self.type_load_quotes_plus3(true);
            return;
        }
        const PRESS: u32 = 10;
        self.hold_keys(&[(6, 0)], PRESS);
        self.hold_keys(&[], 10);
    }

    /// Grey +2 menu matches 128K (Calculator → 48 BASIC path).
    pub fn type_load_quotes_plus2(&mut self, with_code: bool) {
        self.type_load_quotes_128k(with_code);
    }

    /// Model-aware `LOAD ""` [CODE] (48K keyword / 128K / +2 / +2A Loader / +3 48 BASIC).
    pub fn type_load_quotes(&mut self, with_code: bool) {
        match self.model() {
            Model::Spectrum16K | Model::Spectrum48 => self.type_load_quotes_48k(with_code),
            Model::Spectrum128 => self.type_load_quotes_128k(with_code),
            Model::SpectrumPlus2 => self.type_load_quotes_plus2(with_code),
            Model::SpectrumPlus2A => self.type_load_quotes_plus2a(with_code),
            Model::SpectrumPlus3 => self.type_load_quotes_plus3(with_code),
            Model::Pentagon128 => self.type_load_quotes_128k(with_code),
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
    use tape::{tap_checksum, TapImage};
    use ula::{FRAME_TSTATES_48, INT_LENGTH_48};

    fn rom48() -> Option<Vec<u8>> {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/spec48.rom");
        std::fs::read(p).ok()
    }

    fn fixture_tap() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/tape/minimal_code.tap")
    }

    #[test]
    fn multiface_in_pages_out_and_back_for_return() {
        let mut rom = [0u8; bus::MULTIFACE1_SIZE];
        rom[0x66] = 0x76; // HALT at NMI vector
        let mut m = Machine::new_48k(&[0u8; 16384]).unwrap();
        m.attach_multiface(&rom).unwrap();
        m.cpu_mut().regs.sp = 0xfffd;
        let _ = m.multiface_nmi().expect("MF attached");
        match &m {
            Machine::Spec48 { bus, .. } => {
                assert!(bus.multiface.as_ref().unwrap().paged);
            }
            _ => unreachable!(),
        }
        // Toolkit-style page out / page in without another button press.
        match &mut m {
            Machine::Spec48 { bus, .. } => {
                let _ = bus.in_port(0x001f);
                assert!(!bus.multiface.as_ref().unwrap().paged);
                let _ = bus.in_port(0x009f);
                assert!(bus.multiface.as_ref().unwrap().paged);
                assert_eq!(bus.read(0x0066), 0x76);
            }
            _ => unreachable!(),
        }
    }

    /// Synthetic MF ROM: at NMI vector, `LD A,42h / LD (2000h),A / HALT` — flag in MF RAM.
    #[test]
    fn multiface_nmi_executes_attached_rom() {
        let mut rom = [0u8; bus::MULTIFACE1_SIZE];
        // 0066: 3E 42       LD A,42h
        // 0068: 32 00 20    LD (2000h),A
        // 006B: 76          HALT
        rom[0x66] = 0x3e;
        rom[0x67] = 0x42;
        rom[0x68] = 0x32;
        rom[0x69] = 0x00;
        rom[0x6a] = 0x20;
        rom[0x6b] = 0x76;

        let mut m = Machine::new_48k(&[0u8; 16384]).unwrap();
        m.attach_multiface(&rom).unwrap();
        m.cpu_mut().regs.sp = 0xfffd;
        m.cpu_mut().regs.pc = 0x8000;

        let t = m.multiface_nmi().expect("MF attached");
        assert_eq!(t, 11);
        assert_eq!(m.cpu().regs.pc, 0x0066);
        match &m {
            Machine::Spec48 { bus, .. } => {
                assert!(bus.multiface.as_ref().unwrap().paged);
            }
            _ => unreachable!(),
        }

        // Run until HALT stores the flag.
        for _ in 0..8 {
            if m.cpu().regs.halted {
                break;
            }
            m.step_once();
        }
        assert!(m.cpu().regs.halted);
        assert_eq!(
            m.read_mem(0x2000),
            0x42,
            "NMI handler should have written flag to MF RAM"
        );
    }

    fn synthetic_trd_with_marker(b0: u8, b1: u8) -> formats::TrdImage {
        let mut raw = vec![0u8; formats::TRD_SECTOR_SIZE * formats::TRD_SECTORS_PER_TRACK];
        raw[0] = b0;
        raw[1] = b1;
        formats::TrdImage::parse(&raw).unwrap()
    }

    /// TR-DOS-style `IN A,(#FF)` / `INI` loop at `USR 15616` (`0x3D00`).
    fn trdos_read_sector_rom() -> [u8; bus::TRDOS_ROM_SIZE] {
        let mut rom = [0u8; bus::TRDOS_ROM_SIZE];
        let code: &[u8] = &[
            0x3e, 0x3c, // LD A,3Ch
            0xd3, 0xff, // OUT (FFh),A
            0xaf, // XOR A
            0xd3, 0x3f, // OUT (3Fh),A
            0x3e, 0x01, // LD A,1
            0xd3, 0x5f, // OUT (5Fh),A
            0x3e, 0x80, // LD A,80h
            0xd3, 0x1f, // OUT (1Fh),A
            0x21, 0x00, 0x40, // LD HL,4000h
            0x01, 0x7f, 0x00, // LD BC,007Fh
            0xdb, 0xff, // IN A,(FFh)
            0xe6, 0xc0, // AND C0h
            0x28, 0xfa, // JR Z, wait
            0xfa, 0x22, 0x3d, // JP M, done
            0xed, 0xa2, // INI
            0x18, 0xf3, // JR wait
            0x76, // HALT
        ];
        rom[0x3d00..0x3d00 + code.len()].copy_from_slice(code);
        rom
    }

    #[test]
    fn beta_trdos_rom_loop_reads_trd_sector_into_ram() {
        let mut m = Machine::new_48k(&[0u8; 16384]).unwrap();
        m.load_trdos_rom(&trdos_read_sector_rom()).unwrap();
        m.insert_trd(synthetic_trd_with_marker(0x12, 0x34)).unwrap();
        m.cpu_mut().regs.pc = 0x3d00;
        m.cpu_mut().regs.sp = 0xfffd;
        for _ in 0..4000 {
            if m.cpu().regs.halted {
                break;
            }
            m.step_once();
        }
        assert!(m.cpu().regs.halted, "synthetic TR-DOS loop should HALT");
        assert_eq!(m.read_mem(0x4000), 0x12);
        assert_eq!(m.read_mem(0x4001), 0x34);
        assert!(m.has_beta());
    }

    /// Optional: real `roms/trdos.rom` + 48K ROM. Skips cleanly when either is missing.
    #[test]
    fn trdos_rom_usr_15616_pages_when_fixture_present() {
        let Some(spec) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/trdos.rom");
        let Ok(trdos) = std::fs::read(&path) else {
            eprintln!("skip: roms/trdos.rom missing (optional #140 TR-DOS boot fixture)");
            return;
        };
        if trdos.len() != bus::TRDOS_ROM_SIZE {
            eprintln!(
                "skip: roms/trdos.rom is {} bytes, expected {}",
                trdos.len(),
                bus::TRDOS_ROM_SIZE
            );
            return;
        }
        let mut m = Machine::new_48k(&spec).unwrap();
        m.load_trdos_rom(&trdos).unwrap();
        m.insert_trd(synthetic_trd_with_marker(0, 0)).unwrap();
        m.cpu_mut().regs.pc = 0x3d00;
        m.cpu_mut().regs.sp = 0xfffd;
        let mut saw_paged = false;
        for _ in 0..50_000 {
            m.step_once();
            if let Machine::Spec48 { bus, .. } = &m {
                if bus.beta.as_ref().is_some_and(|b| b.paged) {
                    saw_paged = true;
                    break;
                }
            }
        }
        assert!(
            saw_paged,
            "fetch at 0x3D00 should page TR-DOS ROM (USR 15616)"
        );
    }

    /// Mid-instruction ULA time: `frame_t` at insn start + `(cpu.t - t_step_start)`.

    #[test]
    fn interface1_opcode_fetch_pages_shadow_rom() {
        let rom = [0u8; 16384];
        let mut m = Machine::new_48k(&rom).unwrap();
        let if1 = m.attach_interface1().unwrap();
        let mut if1_rom = [0u8; bus::IF1_ROM_SIZE];
        if1_rom[0x0008] = 0x00; // NOP
        if1_rom[0x0700] = 0x00; // NOP — post-fetch unpages
        if1.load_rom(&if1_rom).unwrap();
        m.cpu_mut().regs.pc = 0x0008;
        m.step_cpu_only();
        assert!(
            m.interface1_mut().unwrap().rom_paged,
            "IF1 should stay paged after fetch at 0x0008 (until 0x0700)"
        );
        m.cpu_mut().regs.pc = 0x0700;
        m.interface1_mut().unwrap().page_rom(true);
        m.step_cpu_only();
        assert!(
            !m.interface1_mut().unwrap().rom_paged,
            "IF1 should unpage after opcode fetch at 0x0700"
        );
    }

    #[test]
    fn interface1_rom_load_skips_cleanly_when_missing() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/if1.rom");
        if !path.is_file() {
            eprintln!("skip: roms/if1.rom missing");
            return;
        }
        let rom = [0u8; 16384];
        let mut m = Machine::new_48k(&rom).unwrap();
        let data = std::fs::read(&path).unwrap();
        m.load_interface1_rom(&data).unwrap();
        assert!(m.interface1_rom_loaded());
    }

    #[test]
    fn memio_mid_instruction_contention_table() {
        // Contended screen address; FAQ Contended I/O ports.
        const ADDR: u16 = 0x4000;
        const FE: u16 = 0x00FE;
        const FF: u16 = 0x00FF;
        const HI_FE: u16 = 0x40FE;
        const HI_FF: u16 = 0x40FF;
        const C0_FE: u16 = 0xC0FE;
        // (frame_t, dt, mem_wait, io_FE, io_40FE, io_40FF)
        const ROWS_48: &[(u32, u64, u32, u32, u32, u32)] = &[
            (ula::PAPER_START_48, 0, 6, 5, 6, 12),
            (ula::PAPER_START_48, 1, 5, 4, 5, 11),
            (ula::PAPER_START_48, 2, 4, 3, 4, 10),
            (ula::PAPER_START_48, 3, 3, 2, 3, 9),
            (ula::PAPER_START_48, 4, 2, 1, 2, 8),
            (ula::PAPER_START_48, 5, 1, 0, 1, 7),
            (ula::PAPER_START_48, 6, 0, 0, 0, 6),
            (ula::PAPER_START_48, 7, 0, 6, 6, 12),
        ];
        const ROWS_128: &[(u32, u64, u32, u32)] = &[
            (ula::PAPER_START_128, 0, 6, 5),
            (ula::PAPER_START_128, 1, 5, 4),
            (ula::PAPER_START_128, 2, 4, 3),
            (ula::PAPER_START_128, 3, 3, 2),
            (ula::PAPER_START_128, 4, 2, 1),
            (ula::PAPER_START_128, 5, 1, 0),
            (ula::PAPER_START_128, 6, 0, 0),
            (ula::PAPER_START_128, 7, 0, 6),
        ];
        // C:1,C:3 totals when high byte contends at paper start + dt.
        const C0_CONTENDED: [u32; 8] = [6, 5, 4, 3, 2, 1, 0, 6];

        for &(frame_t, dt, expect, io_fe, io_hife, io_hiff) in ROWS_48 {
            let mut bus = Bus48::new();
            bus.frame_t = frame_t;
            let mut mem = MemIo48 {
                bus: &mut bus,
                watch: None,
                t_step_start: 100,
                opcode_pc: None,
            };
            let t = 100 + dt;
            assert_eq!(
                mem.read(ADDR, t).1,
                expect,
                "48 mem R frame={frame_t} dt={dt}"
            );
            assert_eq!(
                mem.write(ADDR, 0, t),
                expect,
                "48 mem W frame={frame_t} dt={dt}"
            );
            assert_eq!(
                mem.in_port(FE, t).1,
                io_fe,
                "48 IN FE frame={frame_t} dt={dt}"
            );
            assert_eq!(mem.in_port(FF, t).1, 0, "48 IN FF never contends");
            assert_eq!(
                mem.in_port(HI_FE, t).1,
                io_hife,
                "48 IN 40FE frame={frame_t} dt={dt}"
            );
            assert_eq!(
                mem.in_port(HI_FF, t).1,
                io_hiff,
                "48 IN 40FF frame={frame_t} dt={dt}"
            );
            // Uncontended high RAM
            assert_eq!(mem.read(0x8000, t).1, 0);
        }

        for &(frame_t, dt, expect, io_fe) in ROWS_128 {
            let mut bus = Bus128::new();
            bus.frame_t = frame_t;
            // Default page = bank 0 at C000 (uncontended).
            let mut mem = MemIo128 {
                bus: &mut bus,
                watch: None,
                t_step_start: 100,
                opcode_pc: None,
                pentagon: false,
            };
            let t = 100 + dt;
            assert_eq!(
                mem.read(ADDR, t).1,
                expect,
                "128 mem R frame={frame_t} dt={dt}"
            );
            assert_eq!(
                mem.in_port(FE, t).1,
                io_fe,
                "128 IN FE frame={frame_t} dt={dt}"
            );
            // High 0xC0 with uncontended bank 0: same as FE (N:1,C:3).
            assert_eq!(
                mem.in_port(C0_FE, t).1,
                io_fe,
                "128 IN C0FE bank0 frame={frame_t} dt={dt}"
            );

            // Page contended bank 1 at C000 → C0FE uses C:1,C:3 (same totals as 40FE).
            mem.bus.out_7ffd(1);
            assert_eq!(
                mem.in_port(C0_FE, t).1,
                C0_CONTENDED[dt as usize],
                "128 IN C0FE bank1 frame={frame_t} dt={dt}"
            );
            // Reset page for next iteration clarity
            mem.bus.page = 0;
            mem.bus.locked = false;

            let mut bus3 = BusPlus3::new();
            bus3.frame_t = frame_t;
            let mut mem3 = MemIoPlus3 {
                bus: &mut bus3,
                watch: None,
                t_step_start: 100,
            };
            assert_eq!(
                mem3.read(ADDR, t).1,
                expect,
                "+3 mem R frame={frame_t} dt={dt}"
            );
            assert_eq!(mem3.in_port(FE, t).1, 0, "+3 I/O never contends");
            assert_eq!(mem3.in_port(HI_FF, t).1, 0, "+3 I/O never contends");
            assert_eq!(mem3.in_port(0x2ffd, t).1, 0, "+3 FDC status uncontended");
            assert_eq!(mem3.in_port(0x3ffd, t).1, 0, "+3 FDC data uncontended");
        }

        // Access after the instruction has already burned into the contended window.
        let mut bus = Bus48::new();
        bus.frame_t = ula::PAPER_START_48.wrapping_sub(3);
        let mut mem = MemIo48 {
            bus: &mut bus,
            watch: None,
            t_step_start: 50,
            opcode_pc: None,
        };
        assert_eq!(
            mem.read(ADDR, 53).1,
            6,
            "ULA time = frame_t+3 lands on first contended cycle"
        );
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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
        m.set_tape_load_options(TapeLoadOptions {
            flash_load: true,
            speed: 1,
            ..Default::default()
        });
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
        m.set_tape_load_options(TapeLoadOptions {
            flash_load: true,
            speed: 1,
            ..Default::default()
        });
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
    fn tape_progress_trailing_empty_block_reports_complete() {
        // Mirrors TZX ending in a consumed 0x20 pause_ms=0 (zero pulses on final block).
        let p = TapeProgress {
            block_index: 2,
            block_count: 3,
            pulse_index: 0,
            pulse_count: 0,
        };
        assert_eq!(p.fraction(), 1.0);
        // Non-final empty block stays at the block boundary (not complete).
        let mid = TapeProgress {
            block_index: 1,
            block_count: 3,
            pulse_index: 0,
            pulse_count: 0,
        };
        assert!((mid.fraction() - 1.0 / 3.0).abs() < f32::EPSILON);
        // u32::MAX + 1 must not panic in debug; treat as past the end → complete.
        let max = TapeProgress {
            block_index: u32::MAX,
            block_count: 1,
            pulse_index: 0,
            pulse_count: 0,
        };
        assert_eq!(max.fraction(), 1.0);
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
            ..Default::default()
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
        m.set_tape_load_options(TapeLoadOptions {
            flash_load: true,
            speed: 1,
            ..Default::default()
        });
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
        m.set_tape_load_options(TapeLoadOptions {
            flash_load: true,
            speed: 1,
            ..Default::default()
        });
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
            ..Default::default()
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
            ..Default::default()
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
    fn ear_turbo_returns_to_1x_after_tape_finishes() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        // Tiny TAP so EAR turbo finishes quickly.
        let img = TapImage {
            blocks: vec![vec![0xff, 0x00, 0xff]],
            ..Default::default()
        };
        let mut m = Machine::new_48k(&rom).unwrap();
        m.set_tape_load_options(TapeLoadOptions {
            flash_load: false,
            speed: 20,
            ..Default::default()
        });
        m.insert_tape(TapPlayer::new(img));
        m.set_tape_playing(true);
        assert!(m.tape_playing());
        // Mid-load: turbo must run multiple Spectrum frames per host tick.
        let t0 = m.cpu().t;
        let _ = m.run_frame();
        let mid_dt = m.cpu().t.saturating_sub(t0);
        assert!(
            mid_dt > 20_000,
            "expected turbo mid-load T-state advance, got {mid_dt}"
        );
        for _ in 0..20_000 {
            let _ = m.run_frame();
            if m.tape_finished() || !m.tape_playing() {
                break;
            }
        }
        assert!(
            m.tape_finished() || !m.tape_playing(),
            "deck should finish/pause after EAR exhausts"
        );
        assert!(
            !m.tape_playing(),
            "playing must clear when finished so turbo stops (#178)"
        );
        let t1 = m.cpu().t;
        let _ = m.run_frame();
        let post_dt = m.cpu().t.saturating_sub(t1);
        // One Spectrum frame ≈ 69888 T-states (48K); allow slack, but not N× turbo.
        assert!(
            post_dt < 150_000,
            "after tape end, run_frame should be ~1× (got {post_dt} T-states)"
        );
    }

    #[test]
    fn plus2a_stack_repair_ignores_coincidental_0038_marker() {
        let player = TapPlayer::new(TapImage::default());
        let plant = |bus: &mut BusPlus3| {
            bus.write(0x5CB2, 0xFF);
            bus.write(0x5CB3, 0x7F); // RAMTOP = 0x7FFF
            bus.write(0x7FEC, 0x38);
            bus.write(0x7FED, 0x00); // coincidental 0x0038 marker
            bus.write(0x7FFC, 0xAA);
            bus.write(0x7FFD, 0xBB); // sentinel 0xBBAA
        };
        let assert_untouched = |bus: &BusPlus3| {
            assert_eq!(
                u16::from_le_bytes([bus.read(0x7FEC), bus.read(0x7FED)]),
                0x0038
            );
            assert_eq!(
                u16::from_le_bytes([bus.read(0x7FFC), bus.read(0x7FFD)]),
                0xBBAA
            );
        };

        // SP outside CLEAR-32767 loader window — must not rewrite high RAM.
        {
            let mut bus = BusPlus3::new_with_disk(false);
            plant(&mut bus);
            let mut cpu = Cpu::new();
            cpu.regs.sp = 0xFF50;
            cpu.regs.pc = 0x15E8;
            Machine::plus2a_repair_menu_loader_stack_if_needed(&mut bus, &cpu, &player);
            assert_untouched(&bus);
        }

        // SP looks like Loader depth but PC already left ROM — must not rewrite.
        {
            let mut bus = BusPlus3::new_with_disk(false);
            plant(&mut bus);
            let mut cpu = Cpu::new();
            cpu.regs.sp = 0x7FE6;
            cpu.regs.pc = 0x8000;
            Machine::plus2a_repair_menu_loader_stack_if_needed(&mut bus, &cpu, &player);
            assert_untouched(&bus);
        }
    }

    #[test]
    fn ear_speed_finishes_block_in_fewer_run_frames() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let img = TapImage {
            blocks: vec![vec![0x00, 0x00]],
            ..Default::default()
        };
        let frames_until_block1 = |speed: u32| -> u32 {
            let mut m = Machine::new_48k(&rom).unwrap();
            m.set_tape_load_options(TapeLoadOptions {
                flash_load: false,
                speed,
                ..Default::default()
            });
            m.insert_tape(TapPlayer::new(img.clone()));
            m.set_tape_playing(true);
            for n in 1..=50_000u32 {
                let _ = m.run_frame();
                if m.tape_block() == Some(1) {
                    return n;
                }
            }
            panic!("speed {speed}: did not reach block 1");
        };
        let slow = frames_until_block1(1);
        let fast = frames_until_block1(10);
        assert!(
            fast * 7 < slow,
            "EAR speed 10 should finish in ~1/10 host run_frames (1x={slow}, 10x={fast})"
        );
    }

    #[test]
    fn reset_keeps_tape_inserted_and_paused_at_position() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let img = TapImage::load(&fixture_tap()).expect("fixture");
        let mut m = Machine::new_48k(&rom).unwrap();
        m.set_tape_load_options(TapeLoadOptions {
            flash_load: false,
            speed: 1,
            ..Default::default()
        });
        m.insert_tape(TapPlayer::new(img));
        m.set_tape_playing(true);
        for _ in 0..3 {
            let _ = m.run_frame();
        }
        let block_before = m.tape_block();
        let pulse_before = m.tape_progress().map(|p| p.pulse_index);
        assert!(m.has_tape());
        assert!(m.tape_playing());
        m.reset();
        assert!(m.has_tape(), "reset must not eject the tape");
        assert!(!m.tape_playing(), "reset should pause the deck");
        assert_eq!(
            m.tape_block(),
            block_before,
            "reset should keep tape position"
        );
        assert_eq!(
            m.tape_progress().map(|p| p.pulse_index),
            pulse_before,
            "reset should keep pulse position"
        );
    }

    #[test]
    fn reset_keeps_plus3_disk_inserted() {
        let Some(rom) = rom_plus3_only().or_else(rom_plus2a_only) else {
            eprintln!("skip: plus3 ROM missing");
            return;
        };
        let mut m = Machine::new_plus3(&rom).unwrap();
        let data = {
            let mut data = vec![0u8; 0x100];
            data[0..8].copy_from_slice(b"MV - CPC");
            data[0x30] = 1;
            data[0x31] = 1;
            let track_size: u16 = 0x100;
            data[0x32..0x34].copy_from_slice(&track_size.to_le_bytes());
            let mut track = vec![0u8; track_size as usize];
            track[0..12].copy_from_slice(b"Track-Info\r\n");
            data.extend_from_slice(&track);
            data
        };
        let img = formats::DskImage::parse(&data).expect("minimal dsk");
        m.insert_disk(img).expect("insert");
        {
            let Machine::SpecPlus3 { bus, .. } = &m else {
                panic!("expected SpecPlus3");
            };
            assert!(bus.fdc.image.is_some());
        }
        m.reset();
        let Machine::SpecPlus3 { bus, .. } = &m else {
            panic!("expected SpecPlus3");
        };
        assert!(
            bus.fdc.image.is_some(),
            "reset must not eject the +3 disk image"
        );
    }

    #[test]
    fn rom_ld_bytes_ear_survives_mid_load_speed_change() {
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
            speed: 5,
            ..Default::default()
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
        // Into the leader/data, then raise EAR turbo — must not restart the block.
        for _ in 0..40 {
            let _ = m.run_frame();
        }
        m.set_tape_load_options(TapeLoadOptions {
            flash_load: false,
            speed: 15,
            ..Default::default()
        });
        let mut ok = false;
        for _ in 0..400 {
            let _ = m.run_frame();
            if attr_mark_code_ok(&m) {
                ok = true;
                break;
            }
        }
        assert!(
            ok,
            "ROM LD-BYTES EAR path should complete after mid-load speed change (PC={:04X} block={:?})",
            m.cpu().regs.pc,
            m.tape_block()
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
        m.set_tape_load_options(TapeLoadOptions {
            flash_load: true,
            speed: 1,
            ..Default::default()
        });
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
            ..Default::default()
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

    fn rom_plus2a_only() -> Option<Vec<u8>> {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/plus2a/plus2a.rom");
        std::fs::read(p).ok()
    }

    fn rom_plus3_only() -> Option<Vec<u8>> {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/plus3/plus3.rom");
        std::fs::read(p).ok()
    }

    fn rom_plus2() -> Option<Vec<u8>> {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/plus2/plus2uk.rom");
        std::fs::read(p).ok()
    }

    #[test]
    fn model_16k_limits_ram_to_16k() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let mut m = Machine::new_16k(&rom).unwrap();
        assert_eq!(m.model(), Model::Spectrum16K);
        m.write_mem(0x4000, 0xAB);
        m.write_mem(0x8000, 0xCD);
        assert_eq!(m.read_mem(0x4000), 0xAB);
        assert_eq!(m.read_mem(0x8000), 0xFF);
    }

    #[test]
    fn model_plus2_tags_grey_plus2() {
        let Some(rom) = rom_plus2() else {
            eprintln!("skip: roms/plus2/plus2uk.rom missing");
            return;
        };
        let m = Machine::new_plus2(&rom).unwrap();
        assert_eq!(m.model(), Model::SpectrumPlus2);
    }

    #[test]
    fn plus2a_model_has_no_disk_and_rejects_dsk() {
        let Some(rom) = rom_plus2a_only().or_else(rom_plus3_only) else {
            eprintln!("skip: plus2a/plus3 ROM missing");
            return;
        };
        let mut m = Machine::new_plus2a(&rom).unwrap();
        assert_eq!(m.model(), Model::SpectrumPlus2A);
        {
            let Machine::SpecPlus3 { bus, .. } = &mut m else {
                panic!("expected SpecPlus3");
            };
            assert!(!bus.disk_interface);
            assert_eq!(bus.in_port(0x2ffd), 0xff);
        }
        let data = {
            let mut data = vec![0u8; 0x100];
            data[0..8].copy_from_slice(b"MV - CPC");
            data[0x30] = 1;
            data[0x31] = 1;
            let track_size: u16 = 0x100;
            data[0x32..0x34].copy_from_slice(&track_size.to_le_bytes());
            let mut track = vec![0u8; track_size as usize];
            track[0..12].copy_from_slice(b"Track-Info\r\n");
            data.extend_from_slice(&track);
            data
        };
        let img = formats::DskImage::parse(&data).expect("minimal dsk");
        let err = m.insert_disk(img).unwrap_err();
        assert!(
            err.contains("+2A") || err.contains("disk"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn plus3_model_keeps_disk_interface() {
        let Some(rom) = rom_plus3_only().or_else(rom_plus2a_only) else {
            eprintln!("skip: plus3 ROM missing");
            return;
        };
        let mut m = Machine::new_plus3(&rom).unwrap();
        assert_eq!(m.model(), Model::SpectrumPlus3);
        let Machine::SpecPlus3 { bus, .. } = &mut m else {
            panic!("expected SpecPlus3");
        };
        assert!(bus.disk_interface);
        assert_eq!(bus.in_port(0x2ffd) & 0x80, 0x80);
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

    /// ROM-gated: Loader (menu Enter) must talk to the µPD765 on a synthetic
    /// +3DOS DATA disk. Skips if `roms/plus3/plus3.rom` is missing — do not
    /// fall back to +2A ROM.
    #[test]
    fn plus3_loader_talks_to_fdc_on_data_disk() {
        let Some(rom) = rom_plus3_only() else {
            eprintln!("skip: roms/plus3/plus3.rom missing — run ./scripts/fetch_roms.sh");
            return;
        };
        let mut m = Machine::new_plus3(&rom).unwrap();
        m.insert_disk(formats::DskImage::synthetic_plus3_data())
            .expect("insert");
        for _ in 0..120 {
            let _ = m.run_frame();
        }
        // Loader is the first menu item; Enter = row 6 bit 0.
        m.hold_keys(&[(6, 0)], 10);
        m.hold_keys(&[], 5);
        m.keyboard_mut().reset();
        for _ in 0..500 {
            let _ = m.run_frame();
        }
        let Machine::SpecPlus3 { bus, cpu, .. } = &m else {
            panic!("expected SpecPlus3");
        };
        assert!(
            bus.fdc.read_count > 0,
            "Loader/+3DOS should READ from the FDC (seek={} read={} write={} PC={:04X} 1FFD={:02X} PCN={})",
            bus.fdc.seek_count,
            bus.fdc.read_count,
            bus.fdc.write_count,
            cpu.regs.pc,
            bus.page_1ffd,
            bus.fdc.pcn(0)
        );
        assert!(
            bus.fdc.seek_count > 0,
            "expected SEEK or RECALIBRATE before/during disk boot (seek=0 read={})",
            bus.fdc.read_count
        );
    }

    /// ROM-gated: menu Loader + `DOS_BOOT` (checksum 3) must run the titled
    /// bootstrap. Commercial +3 disks and DSKTOOL use this path; Fuse's phantom
    /// typist only presses Enter — the ROM does the rest.
    #[test]
    fn plus3_loader_dos_boot_runs_titled_marker() {
        let Some(rom) = rom_plus3_only() else {
            eprintln!("skip: roms/plus3/plus3.rom missing — run ./scripts/fetch_roms.sh");
            return;
        };
        let mut m = Machine::new_plus3(&rom).unwrap();
        m.insert_disk(formats::DskImage::synthetic_plus3_boot_marker())
            .expect("insert");
        for _ in 0..120 {
            let _ = m.run_frame();
        }
        m.hold_keys(&[(6, 0)], 10);
        m.hold_keys(&[], 5);
        m.keyboard_mut().reset();
        let mut ok = false;
        for _ in 0..800 {
            let _ = m.run_frame();
            if m.inspect().border == 2 || m.read_mem(0xFE20) == 0xA5 {
                ok = true;
                break;
            }
        }
        assert!(
            ok,
            "Loader DOS_BOOT should set border 2 or poke FE20 (PC={:04X} border={} FE20={:02X} special={})",
            m.cpu().regs.pc,
            m.inspect().border,
            m.read_mem(0xFE20),
            m.inspect().paging.special
        );
    }

    /// ROM-gated: non-bootable titled disk → Loader `LOAD "DISK"` → BASIC RUN.
    #[test]
    fn plus3_loader_load_disk_runs_basic_marker() {
        let Some(rom) = rom_plus3_only() else {
            eprintln!("skip: roms/plus3/plus3.rom missing — run ./scripts/fetch_roms.sh");
            return;
        };
        let mut m = Machine::new_plus3(&rom).unwrap();
        m.insert_disk(formats::DskImage::synthetic_plus3_disk_basic())
            .expect("insert");
        for _ in 0..120 {
            let _ = m.run_frame();
        }
        m.hold_keys(&[(6, 0)], 10);
        m.hold_keys(&[], 5);
        m.keyboard_mut().reset();
        let mut ok = false;
        for _ in 0..2_500 {
            let _ = m.run_frame();
            if m.read_mem(0x8000) == 0xA5 {
                ok = true;
                break;
            }
        }
        let pc = m.cpu().regs.pc;
        let poke = m.read_mem(0x8000);
        let (read, seek) = match &m {
            Machine::SpecPlus3 { bus, .. } => (bus.fdc.read_count, bus.fdc.seek_count),
            _ => panic!("expected SpecPlus3"),
        };
        assert!(
            ok,
            "Loader LOAD \"DISK\" should RUN BASIC poke at 8000 (PC={pc:04X} 8000={poke:02X} read={read} seek={seek})"
        );
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
            ..Default::default()
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
        m.set_tape_load_options(TapeLoadOptions {
            flash_load,
            speed,
            ..Default::default()
        });
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
    fn attr_mark_experience_load_succeeds() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/tape/attr_mark.tap");
        let img = TapImage::load(&path).expect("attr_mark.tap");
        let mut m = Machine::new_48k(&rom).unwrap();
        m.set_tape_load_options(TapeLoadOptions::experience());
        m.insert_tape(TapPlayer::new(img));
        for _ in 0..200 {
            let _ = m.run_frame();
        }
        m.type_load_quotes(true);
        m.set_tape_playing(true);
        let mut loaded = false;
        for _ in 0..800 {
            let _ = m.run_frame();
            if attr_mark_code_ok(&m) {
                loaded = true;
                break;
            }
        }
        assert!(
            loaded,
            "experience LOAD \"\" CODE should poke attr_mark bytes at 0x8000"
        );
        let opts = m.tape_load_options();
        assert!(opts.experience_load);
        assert_eq!(opts.speed, tape::EXPERIENCE_EAR_SPEED);
    }

    #[test]
    fn ld_bytes_waits_while_tape_paused_in_experience_mode() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let img = TapImage::load(&fixture_tap()).expect("fixture");
        let header_len = img.blocks[0].len();
        let mut m = Machine::new_48k(&rom).unwrap();
        m.set_tape_load_options(TapeLoadOptions::experience());
        m.insert_tape(TapPlayer::new(img));
        let ret = 0x12abu16;
        m.cpu_mut().regs.sp = 0x5f00;
        m.write_mem(0x5f00, (ret & 0xff) as u8);
        m.write_mem(0x5f01, (ret >> 8) as u8);
        m.cpu_mut().regs.pc = LD_BYTES_TRAP_PC;
        m.cpu_mut().regs.a_ = 0x00;
        m.cpu_mut().regs.f_ = flag::C;
        m.cpu_mut().regs.set_ix(0x5c00);
        m.cpu_mut().regs.set_de((header_len - 2) as u16);
        if let Machine::Spec48 { bus, .. } = &mut m {
            bus.frame_t = INT_LENGTH_48;
        }
        for _ in 0..64 {
            m.step_once();
            assert_eq!(
                m.cpu().regs.pc,
                LD_BYTES_TRAP_PC,
                "experience must hold at LD-BYTES until Play"
            );
        }
    }

    #[test]
    fn experience_multi_block_load_within_wall_clock_budget() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let payload_len = 1024usize;
        let mut blocks = Vec::new();
        for i in 0..30u8 {
            let mut block = vec![0u8; payload_len + 2];
            block[0] = 0xff;
            block[1..=payload_len].fill(i);
            block[payload_len + 1] = tap_checksum(&block[..=payload_len]);
            blocks.push(block);
        }
        let img = TapImage {
            blocks,
            ..Default::default()
        };
        let mut m = Machine::new_48k(&rom).unwrap();
        m.set_tape_load_options(TapeLoadOptions::experience());
        m.insert_tape(TapPlayer::new(img));
        m.set_tape_playing(true);
        let max_host_frames = 25 * 50;
        for _n in 0..max_host_frames {
            let _ = m.run_frame();
            if m.tape_block().is_none_or(|b| b >= 30) {
                return;
            }
        }
        panic!("experience load did not finish within {max_host_frames} host frames");
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
        // Speed N runs N Spectrum frames per run_frame while EAR playing.
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
            ..Default::default()
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
        m.set_tape_load_options(TapeLoadOptions {
            flash_load: true,
            speed: 1,
            ..Default::default()
        });
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

    /// Minimal uncompressed Z80 v1 for apply_snapshot48 golden.
    fn synthetic_z80_v1_for_machine() -> Vec<u8> {
        let mut data = vec![0u8; 30 + 49152];
        data[0] = 0x11; // A
        data[1] = 0x22; // F
        data[6] = 0x00;
        data[7] = 0x81; // PC = 0x8100
        data[8] = 0x00;
        data[9] = 0x70; // SP = 0x7000
        data[12] = (6 << 1) & 0x0e; // border 6, uncompressed
        data[30] = 0xbe; // RAM @ Spectrum 0x4000
        data[31] = 0xef;
        data[30 + 0x1000] = 0x42; // Spectrum 0x5000
        data
    }

    #[test]
    fn apply_z80_snapshot48_sets_pc_ram_and_border() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let snap = Snapshot48::parse_z80(&synthetic_z80_v1_for_machine()).expect("z80 v1");
        let mut m = Machine::new_48k(&rom).unwrap();
        m.apply_snapshot48(&snap);
        let i = m.inspect();
        assert_eq!(i.regs.pc, 0x8100);
        assert_eq!(i.regs.sp, 0x7000);
        assert_eq!(i.border, 6);
        assert_eq!(m.read_mem(0x4000), 0xbe);
        assert_eq!(m.read_mem(0x4001), 0xef);
        assert_eq!(m.read_mem(0x5000), 0x42);
    }

    fn synthetic_z80_v2_128_for_machine() -> Vec<u8> {
        let mut data = vec![0u8; 55];
        data[0] = 0x11;
        data[1] = 0x22;
        data[6] = 0;
        data[7] = 0;
        data[8] = 0x00;
        data[9] = 0x70; // SP
        data[12] = (3 << 1) & 0x0e; // border 3
        data[30] = 23;
        data[31] = 0;
        data[32] = 0x00;
        data[33] = 0x90; // PC = 0x9000
        data[34] = 3; // v2 128K
        data[35] = 0x04; // 7FFD: bank 4 at C000
        for page in 3u8..=10 {
            let bank = page - 3;
            data.extend_from_slice(&0xffffu16.to_le_bytes());
            data.push(page);
            let mut page_ram = vec![0u8; 16384];
            page_ram[0] = 0xf0 | bank;
            page_ram[1] = bank;
            data.extend_from_slice(&page_ram);
        }
        data
    }

    fn synthetic_z80_v3_plus3_for_machine() -> Vec<u8> {
        let mut data = vec![0u8; 87];
        data[6] = 0;
        data[7] = 0;
        data[8] = 0xfe;
        data[9] = 0xff;
        data[12] = 7 << 1; // bits 1–3: last OUT to 7FFD bank select (bank 7)
        data[30] = 55;
        data[31] = 0;
        data[32] = 0x00;
        data[33] = 0xc0; // PC in paged bank window
        data[34] = 7; // +3
        data[35] = 0x01; // bank 1 at C000
        data[86] = 0x04; // 1FFD ROM high
        for page in 3u8..=10 {
            let bank = page - 3;
            data.extend_from_slice(&0xffffu16.to_le_bytes());
            data.push(page);
            let mut page_ram = vec![0u8; 16384];
            page_ram[0] = 0xa0 | bank;
            data.extend_from_slice(&page_ram);
        }
        data
    }

    #[test]
    fn apply_snapshot128_z80_pages_and_7ffd() {
        let Some(rom) = rom128() else {
            eprintln!("skip: roms/128/spec128uk.rom missing");
            return;
        };
        let snap = Snapshot128::parse_z80(&synthetic_z80_v2_128_for_machine()).expect("z80 128");
        let mut m = Machine::new_128k(&rom).unwrap();
        m.apply_snapshot128(&snap);
        let i = m.inspect();
        assert_eq!(i.regs.pc, 0x9000);
        assert_eq!(i.border, 3);
        assert_eq!(i.paging.page_7ffd, Some(0x04));
        assert_eq!(m.read_mem(0x4000), 0xf5); // bank 5
        assert_eq!(m.read_mem(0x8000), 0xf2); // bank 2
        assert_eq!(m.read_mem(0xc000), 0xf4); // bank 4 via 7FFD
        if let Machine::Spec128 { bus, .. } = &m {
            assert_eq!(bus.banks[0][0], 0xf0);
            assert_eq!(bus.banks[7][0], 0xf7);
            assert_eq!(bus.page, 0x04);
        } else {
            panic!("expected Spec128");
        }
    }

    #[test]
    fn apply_snapshot128_plus3_applies_1ffd() {
        let Some(rom) = rom_plus3() else {
            eprintln!("skip: plus3/plus2a ROM missing");
            return;
        };
        let snap = Snapshot128::parse_z80(&synthetic_z80_v3_plus3_for_machine()).expect("z80 +3");
        let mut m = Machine::new_plus3(&rom).unwrap();
        m.apply_snapshot128(&snap);
        let i = m.inspect();
        assert_eq!(i.regs.pc, 0xc000);
        assert_eq!(i.paging.page_7ffd, Some(0x01));
        assert_eq!(i.paging.page_1ffd, Some(0x04));
        assert_eq!(m.read_mem(0xc000), 0xa1); // bank 1
        if let Machine::SpecPlus3 { bus, .. } = &m {
            assert_eq!(bus.page_1ffd, 0x04);
            assert_eq!(bus.banks[6][0], 0xa6);
        } else {
            panic!("expected SpecPlus3");
        }
    }

    /// Uncompressed RZX input block (same layout as formats::rzx tests).
    fn minimal_rzx(frames: &[(u16, &[u8])]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"RZX!");
        v.extend_from_slice(&[0x00, 0x0d]);
        v.extend_from_slice(&[0, 0, 0, 0]);
        let mut body = Vec::new();
        body.extend_from_slice(&[0, 0, 0, 0]);
        body.push(0); // uncompressed
        for &(fetch, inputs) in frames {
            body.extend_from_slice(&fetch.to_le_bytes());
            body.extend_from_slice(&(inputs.len() as u16).to_le_bytes());
            body.extend_from_slice(inputs);
        }
        let block_len = (5 + body.len()) as u32;
        v.push(0x80);
        v.extend_from_slice(&block_len.to_le_bytes());
        v.extend_from_slice(&body);
        v
    }

    #[test]
    fn rzx_replay_applies_keyboard_and_kempston() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        // Frame 0: row 1 keys=0x01 (byte 0x21). Frame 1: Kempston 0x15 (byte 0x95).
        // Frame 2: row 0 keys=0x10 (byte 0x10).
        let data = minimal_rzx(&[(100, &[0x21]), (100, &[0x95]), (100, &[0x10])]);
        let rec = RzxRecording::parse(&data).expect("rzx");
        let mut m = Machine::new_48k(&rom).unwrap();
        m.insert_rzx(rec);

        m.run_frame();
        assert_eq!(m.keyboard_mut().rows[1], 0x01);

        m.run_frame();
        assert_eq!(m.kempston_mut().read(), 0x15);
        if let Machine::Spec48 { bus, .. } = &mut m {
            assert_eq!(bus.in_port(0x001f), 0x15);
        }

        m.run_frame();
        assert_eq!(m.keyboard_mut().rows[0], 0x10);
    }

    fn minimal_tzx_turbo_machine(payload: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"ZXTape!");
        v.extend_from_slice(&[0x1a, 1, 20]);
        v.push(0x11);
        v.extend_from_slice(&800u16.to_le_bytes());
        v.extend_from_slice(&400u16.to_le_bytes());
        v.extend_from_slice(&400u16.to_le_bytes());
        v.extend_from_slice(&300u16.to_le_bytes());
        v.extend_from_slice(&600u16.to_le_bytes());
        v.extend_from_slice(&20u16.to_le_bytes());
        v.push(8);
        v.extend_from_slice(&50u16.to_le_bytes());
        let len = payload.len() as u32;
        v.push((len & 0xff) as u8);
        v.push(((len >> 8) & 0xff) as u8);
        v.push(((len >> 16) & 0xff) as u8);
        v.extend_from_slice(payload);
        v
    }

    #[test]
    fn turbo_tzx_ear_advances_without_flash_path() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let data = minimal_tzx_turbo_machine(&[0xff, 0x00, 0xaa]);
        assert!(
            !tape::TzxPlayer::is_standard_speed_only(&data),
            "turbo must not be treated as standard-speed TAP"
        );
        let tap = tape::TzxPlayer::to_tap_image(&data).expect("to_tap");
        assert!(
            tap.blocks.is_empty(),
            "flash/TAP extraction must skip ID 0x11"
        );

        let player = TzxPlayer::parse(&data).expect("tzx");
        assert!(player.scheduled_pulses() > 20);

        let mut m = Machine::new_48k(&rom).unwrap();
        m.set_tape_load_options(TapeLoadOptions {
            flash_load: false,
            speed: 1,
            ..Default::default()
        });
        m.insert_tzx(player);
        m.set_tape_playing(true);
        assert!(!m.ear(), "EAR idle before pulse advance");

        let mut saw_high = false;
        let mut saw_progress = false;
        let mut last_pulse = 0u32;
        for _ in 0..8 {
            m.run_frame();
            if m.ear() {
                saw_high = true;
            }
            if let Some(p) = m.tape_progress() {
                if p.pulse_index > last_pulse {
                    saw_progress = true;
                    last_pulse = p.pulse_index;
                }
            }
        }
        assert!(saw_high, "turbo pilot must drive EAR high");
        assert!(saw_progress, "pulse index must advance under EAR path");
        assert!(
            m.tape_progress().map(|p| p.pulse_count).unwrap_or(0) > 0,
            "turbo deck reports scheduled pulses"
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
        m.set_tape_load_options(TapeLoadOptions {
            flash_load,
            speed,
            ..Default::default()
        });
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
                    Model::Spectrum16K => Machine::new_16k(rom).unwrap(),
                    Model::Spectrum48 => Machine::new_48k(rom).unwrap(),
                    Model::Spectrum128 => Machine::new_128k(rom).unwrap(),
                    Model::SpectrumPlus2 => Machine::new_plus2(rom).unwrap(),
                    Model::SpectrumPlus2A => Machine::new_plus2a(rom).unwrap(),
                    Model::SpectrumPlus3 => Machine::new_plus3(rom).unwrap(),
                    Model::Pentagon128 => {
                        let trdos = read_trdos_rom(Model::Pentagon128).expect("pentagon trdos");
                        Machine::new_pentagon128(rom, &trdos).unwrap()
                    }
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
                        Model::Spectrum16K => Machine::new_16k(rom).unwrap(),
                        Model::Spectrum48 => Machine::new_48k(rom).unwrap(),
                        Model::Spectrum128 => Machine::new_128k(rom).unwrap(),
                        Model::SpectrumPlus2 => Machine::new_plus2(rom).unwrap(),
                        Model::SpectrumPlus2A => Machine::new_plus2a(rom).unwrap(),
                        Model::SpectrumPlus3 => Machine::new_plus3(rom).unwrap(),
                        Model::Pentagon128 => {
                            let trdos = read_trdos_rom(Model::Pentagon128).expect("pentagon trdos");
                            Machine::new_pentagon128(rom, &trdos).unwrap()
                        }
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
                    Model::Spectrum16K => Machine::new_16k(rom).unwrap(),
                    Model::Spectrum48 => Machine::new_48k(rom).unwrap(),
                    Model::Spectrum128 => Machine::new_128k(rom).unwrap(),
                    Model::SpectrumPlus2 => Machine::new_plus2(rom).unwrap(),
                    Model::SpectrumPlus2A => Machine::new_plus2a(rom).unwrap(),
                    Model::SpectrumPlus3 => Machine::new_plus3(rom).unwrap(),
                    Model::Pentagon128 => {
                        let trdos = read_trdos_rom(Model::Pentagon128).expect("pentagon trdos");
                        Machine::new_pentagon128(rom, &trdos).unwrap()
                    }
                };
                m.set_tape_load_options(TapeLoadOptions {
                    flash_load: flash,
                    speed,
                    ..Default::default()
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
                    } else {
                        // Pause while rewinding/arming USR so multi-frame turbo cannot
                        // burn C8 pilot before the RAM loader starts edge-detect.
                        m.set_tape_playing(false);
                        if let Some(TapeDeck::Tap(p)) = match &mut m {
                            Machine::Spec48 { tape, .. }
                            | Machine::Spec128 { tape, .. }
                            | Machine::SpecPlus3 { tape, .. } => tape.as_mut(),
                        } {
                            p.rewind_to_block(2);
                        }
                    }
                    // RANDOMIZE USR 32768 — enter the custom-flag loader.
                    let ret = 0x15e6u16;
                    m.cpu_mut().regs.sp = 0xfffd;
                    m.write_mem(0xfffd, (ret & 0xff) as u8);
                    m.write_mem(0xfffe, (ret >> 8) as u8);
                    m.cpu_mut().regs.halted = false;
                    m.cpu_mut().regs.pc = 0x8000;
                    // Fake USR return (0x15E6) is not a real BASIC continuation: the
                    // C8 byte at 0x9000 is visible for only ~15 frames then cleared.
                    // Poll at 1× so warp-S host ticks cannot skip that window; the
                    // earlier LOAD "" CODE phase still used full EAR warp.
                    if !flash {
                        m.set_tape_load_options(TapeLoadOptions {
                            flash_load: false,
                            speed: 1,
                            ..Default::default()
                        });
                    }
                    m.set_tape_playing(true);
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
        // PROG+CODE = blocks 0..3; four custom `0xC8` blocks are 4..7 → success at ≥8.
        let done =
            |m: &Machine| m.cpu().regs.pc == 0x5b00 || m.tape_block().is_some_and(|b| b >= 8);

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
                    Model::Spectrum16K => Machine::new_16k(&rom).unwrap(),
                    Model::Spectrum48 => Machine::new_48k(&rom).unwrap(),
                    Model::Spectrum128 => Machine::new_128k(&rom).unwrap(),
                    Model::SpectrumPlus2 => Machine::new_plus2(&rom).unwrap(),
                    Model::SpectrumPlus2A => Machine::new_plus2a(&rom).unwrap(),
                    Model::SpectrumPlus3 => Machine::new_plus3(&rom).unwrap(),
                    Model::Pentagon128 => {
                        let trdos = read_trdos_rom(Model::Pentagon128).expect("pentagon trdos");
                        Machine::new_pentagon128(&rom, &trdos).unwrap()
                    }
                };
                let deck = TapPlayer::new(img.clone());
                let mut machine = m;
                machine.set_tape_load_options(TapeLoadOptions {
                    flash_load: flash,
                    speed,
                    ..Default::default()
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
