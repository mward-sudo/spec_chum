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
pub use inspect::{BetaInspect, Inspect, Paging, TapeInspect};
pub use joystick::{apply_joystick, clear_joystick_matrix, JoystickMode, JoystickState};
pub use rom::{
    expected_main_rom_bytes, exrom_available, exrom_available_in, exrom_candidates,
    install_rom_slot, main_rom_available, main_rom_available_in, model_label, model_title,
    read_exrom, read_exrom_with_overrides, read_rom, read_rom_with_overrides, read_trdos_rom,
    read_trdos_rom_with_overrides, requires_exrom, requires_trdos_rom, requires_user_rom,
    resolve_exrom_path, resolve_exrom_path_in, resolve_exrom_path_in_with_overrides,
    resolve_rom_path, resolve_rom_path_in, resolve_rom_path_in_with_overrides,
    resolve_trdos_rom_path, resolve_trdos_rom_path_in, resolve_trdos_rom_path_in_with_overrides,
    resolve_trdos_rom_preferring_file_services, rom_available, rom_available_in,
    rom_available_in_with_overrides, rom_candidates, rom_path_status, rom_slot_descriptors,
    rom_slot_state, rom_slot_state_with_override, rom_slot_states, rom_slot_states_with_overrides,
    search_roots, trdos_rom_08d2_is_vg93_port_stub, trdos_rom_available, trdos_rom_available_in,
    trdos_rom_candidates, trdos_rom_fills_0800_hole, trdos_rom_has_native_file_services,
    unavailable_reason, writable_install_root, RomSlotDescriptor, RomSlotKind, RomSlotState,
    RomSlotStatus, ALL_MODELS, TRDOS_ROM_INSTALL_PATH,
};

use std::cell::Cell;

pub use bus::StereoMode as AyStereoMode;
use bus::{Bus128, Bus48, BusPlus3, Kempston, KempstonMouse};
use formats::{apply_input_byte, DskImage, RzxRecording, Snapshot128, Snapshot48};
pub use tape::LD_BYTES_TRAP_PC;
pub use tape::TIMEX_EXROM_LD_BYTES_PC;
use tape::{
    evaluate_ld_bytes_trap, flash_load_block, is_ld_bytes_trap_pc, TapPlayer, TapeTrapResult,
    TzxPlayer, LD_BYTES_PROLOGUE,
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

/// Errors constructing a [`Machine`] (ROM size / content).
#[derive(Debug, Error)]
pub enum MachineBuildError {
    #[error("invalid ROM: {0}")]
    InvalidRom(String),
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
    /// Timex TC2048 (#192 Phase 1): 48K-class + SCLD ports, distributable ROM.
    TimexTC2048,
    /// Timex TS2068 / TC2068 (#192 Phase 2a): home + EX-ROM, horizontal MMU, AY.
    TimexTS2068,
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

    /// 48K-class bus (16K / 48K / Timex).
    #[must_use]
    pub fn is_48k_class(self) -> bool {
        matches!(
            self,
            Self::Spectrum16K | Self::Spectrum48 | Self::TimexTC2048 | Self::TimexTS2068
        )
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
        let is_opcode = self.opcode_pc == Some(addr);
        if is_opcode {
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

    fn m1_refresh(&mut self, refresh_addr: u16, t: u64, m1_contended: bool) {
        let i = (refresh_addr >> 8) as u8;
        let r = (refresh_addr & 0x7f) as u8;
        let frame_t = self.ula_t(t);
        let screen = &self.bus.ram[..6912];
        let ovs = ula::snow_overrides(
            frame_t,
            r,
            m1_contended,
            ula::snow_possible(i),
            ula::SnowTiming::Class48,
            screen,
            None,
        );
        for o in ovs {
            self.bus.ula.record_snow(o.line, o.col, o.kind, o.byte);
        }
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
    /// Pentagon 128: 71680 T/frame, no memory or I/O contention (no ULA snow).
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
        let is_opcode = self.opcode_pc == Some(addr);
        if is_opcode {
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

    fn m1_refresh(&mut self, refresh_addr: u16, t: u64, m1_contended: bool) {
        // Pentagon and Amstrad +2A/+3 paths omit this hook — no original-ULA snow.
        if self.pentagon {
            return;
        }
        let i = (refresh_addr >> 8) as u8;
        let r = (refresh_addr & 0x7f) as u8;
        let frame_t = self.ula_t(t);
        let c000_contended = self.bus.c000_contended();
        if !ula::snow_possible_128(i, c000_contended) {
            return;
        }
        let screen_bank = if self.bus.page & 0x08 != 0 { 7 } else { 5 };
        let c000_bank = usize::from(self.bus.page & 7);
        let Some(i_bank) = ula::i_pointed_bank_128(i, c000_bank) else {
            return;
        };
        let src_bank = ula::snow_source_bank_128(i_bank, screen_bank);
        // Borrow banks by index (shared refs) — never allocate 6912 bytes in the M1 hot path.
        let corrupt_source = (src_bank != screen_bank).then(|| &self.bus.banks[src_bank][..6912]);
        let screen = &self.bus.banks[screen_bank][..6912];
        let ovs = ula::snow_overrides(
            frame_t,
            r,
            m1_contended,
            true,
            ula::SnowTiming::Class128,
            screen,
            corrupt_source,
        );
        for o in ovs {
            self.bus.ula.record_snow(o.line, o.col, o.kind, o.byte);
        }
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
/// Amstrad +2A/+3 memory+I/O — **no** original-ULA snow (`m1_refresh` stays default no-op).
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

    /// Timex TC2048: 48K hardware + SCLD ports (#192 Phase 1).
    pub fn new_timex_tc2048(rom: &[u8]) -> Result<Self, MachineBuildError> {
        let mut bus = Bus48::new();
        bus.timex = true;
        bus.load_rom(rom).map_err(MachineBuildError::InvalidRom)?;
        trace::emit(trace::EventKind::MachineModel { model: 7 });
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

    /// Timex TS2068 / TC2068: home ROM + EX-ROM, horizontal MMU, AY (#192 Phase 2a).
    pub fn new_timex_ts2068(home_rom: &[u8], exrom: &[u8]) -> Result<Self, MachineBuildError> {
        let mut bus = Bus48::new();
        bus.timex = true;
        bus.timex_2068 = true;
        bus.load_rom(home_rom)
            .map_err(MachineBuildError::InvalidRom)?;
        bus.load_timex_exrom(exrom)
            .map_err(MachineBuildError::InvalidRom)?;
        trace::emit(trace::EventKind::MachineModel { model: 8 });
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
        bus.frame_tstates = FRAME_TSTATES_PENTAGON;
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
                if bus.timex_2068 {
                    Model::TimexTS2068
                } else if bus.timex {
                    Model::TimexTC2048
                } else if bus.ram16k {
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
                if bus.timex {
                    bus.timex_scld.reset();
                }
                if bus.timex_2068 {
                    bus.ay.reset();
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

    /// Remove any inserted tape image (TAP/TZX deck).
    pub fn eject_tape(&mut self) {
        match self {
            Self::Spec48 { tape, .. }
            | Self::Spec128 { tape, .. }
            | Self::SpecPlus3 { tape, .. } => {
                *tape = None;
            }
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

    /// Set AY stereo pan mode (no-op on 48K / TC2048 without AY).
    pub fn set_ay_stereo_mode(&mut self, mode: bus::StereoMode) {
        match self {
            Self::Spec48 { bus, .. } if bus.timex_2068 => {
                bus.ay.stereo_mode = mode;
            }
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
            Self::Spec48 { bus, .. } if bus.timex_2068 => bus.ay.stereo_mode,
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

    /// Insert a Timex `.dck` dock cartridge (TS2068 / TC2068 only). Soft-resets the machine.
    pub fn insert_timex_dock(
        &mut self,
        image: &formats::DckImage,
    ) -> Result<(), bus::TimexDockError> {
        match self {
            Self::Spec48 { bus, .. } if bus.timex_2068 => {
                bus.insert_timex_dock(image)?;
            }
            _ => return Err(bus::TimexDockError::UnsupportedModel),
        }
        self.reset();
        Ok(())
    }

    /// Eject Timex dock cartridge and soft-reset the machine (keep Timex ROMs).
    pub fn eject_timex_dock(&mut self) -> Result<(), bus::TimexDockError> {
        match self {
            Self::Spec48 { bus, .. } if bus.timex_2068 => {
                bus.eject_timex_dock();
            }
            _ => return Err(bus::TimexDockError::UnsupportedModel),
        }
        self.reset();
        Ok(())
    }

    #[must_use]
    pub fn has_timex_dock(&self) -> bool {
        match self {
            Self::Spec48 { bus, .. } => bus.has_timex_dock(),
            _ => false,
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
                let has_ay = bus.timex_2068;
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
                let t_per_sample = f64::from(FRAME_TSTATES_48) / AY_SAMPLES as f64;
                let mut ay_samples = Vec::with_capacity(if has_ay { AY_SAMPLES } else { 0 });
                let mut ay_left = Vec::with_capacity(if has_ay { AY_SAMPLES } else { 0 });
                let mut ay_right = Vec::with_capacity(if has_ay { AY_SAMPLES } else { 0 });
                let mut ay_t = 0u32;
                let mut last_t = cpu.t;
                let mut broke_on_pc = false;
                let mut frame_done = false;
                while !frame_done && !broke_on_pc {
                    if debugger.check_pc(cpu.regs.pc) {
                        break;
                    }
                    Self::timex_redirect_spectrum_ld_bytes(cpu, bus);
                    if Self::hold_ld_bytes_until_play(cpu.regs.pc, tape, |a| bus.read(a)) {
                        const HOLD_T: u32 = 4;
                        cpu.t = cpu.t.wrapping_add(u64::from(HOLD_T));
                        last_t = cpu.t;
                        if has_ay {
                            ay_t = ay_t.saturating_add(HOLD_T);
                            bus.ay.advance(HOLD_T);
                            while ay_samples.len() < AY_SAMPLES
                                && f64::from(ay_t) >= (ay_samples.len() as f64 + 1.0) * t_per_sample
                            {
                                push_ay_frame_sample(
                                    &bus.ay,
                                    &mut ay_samples,
                                    &mut ay_left,
                                    &mut ay_right,
                                );
                            }
                        }
                        frame_done = advance_frame_t(&mut bus.frame_t, HOLD_T, FRAME_TSTATES_48);
                        continue;
                    }
                    if tape_opts.flash_load && Self::try_flash_load_48(cpu, bus, tape) {
                        continue;
                    }
                    if int_active_48(bus.frame_t) && !(bus.timex && bus.timex_scld.int_disabled()) {
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
                            if has_ay {
                                ay_t = ay_t.saturating_add(irq_t);
                                bus.ay.advance(irq_t);
                                while ay_samples.len() < AY_SAMPLES
                                    && f64::from(ay_t)
                                        >= (ay_samples.len() as f64 + 1.0) * t_per_sample
                                {
                                    push_ay_frame_sample(
                                        &bus.ay,
                                        &mut ay_samples,
                                        &mut ay_left,
                                        &mut ay_right,
                                    );
                                }
                            }
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
                    if has_ay {
                        ay_t = ay_t.saturating_add(dt);
                        bus.ay.advance(dt);
                        while ay_samples.len() < AY_SAMPLES
                            && f64::from(ay_t) >= (ay_samples.len() as f64 + 1.0) * t_per_sample
                        {
                            push_ay_frame_sample(
                                &bus.ay,
                                &mut ay_samples,
                                &mut ay_left,
                                &mut ay_right,
                            );
                        }
                    }
                    frame_done = advance_frame_t(&mut bus.frame_t, dt, FRAME_TSTATES_48);
                }
                if has_ay {
                    while ay_samples.len() < AY_SAMPLES {
                        push_ay_frame_sample(&bus.ay, &mut ay_samples, &mut ay_left, &mut ay_right);
                    }
                }
                // Keep border_events for render; next run_frame begin_frame clears them.
                FrameAudio {
                    beeper_edges: std::mem::take(&mut bus.beeper_edges),
                    ay_samples,
                    ay_left,
                    ay_right,
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
                bus.apply_pending_screen_switch();
                ula.border = bus.border;
                ula.display_screen_bank = bus.ula.display_screen_bank;
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
                bus.apply_pending_screen_switch();
                ula.border = bus.border;
                ula.display_screen_bank = bus.ula.display_screen_bank;
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

    /// Spectrum games often `CALL $0556` (48K LD-BYTES). Timex home ROM has different
    /// code there; the real loader lives at [`TIMEX_EXROM_LD_BYTES_PC`] in EX-ROM.
    ///
    /// When a RAM caller lands on `$0556` without the Spectrum prologue, page EX-ROM
    /// chunk 0' and continue at the Timex entry so EAR / Instant can still load.
    fn timex_redirect_spectrum_ld_bytes(cpu: &mut Cpu, bus: &mut Bus48) {
        const SPECTRUM_LD_BYTES: u16 = 0x0556;
        if !bus.timex_2068 || cpu.regs.pc != SPECTRUM_LD_BYTES {
            return;
        }
        if (0..4).all(|i| bus.read(SPECTRUM_LD_BYTES + i) == LD_BYTES_PROLOGUE[i as usize]) {
            return;
        }
        // Only rewrite CALLs from RAM — never Timex ROM fall-through at $0556.
        let sp = cpu.regs.sp;
        let ret = u16::from_le_bytes([bus.read(sp), bus.read(sp.wrapping_add(1))]);
        if ret < 0x4000 {
            return;
        }
        let ff = bus.timex_scld.port_ff() | 0x80;
        let f4 = bus.timex_scld.port_f4() | 0x01;
        bus.out_port(0x00FF, ff);
        bus.out_port(0x00F4, f4);
        cpu.regs.pc = TIMEX_EXROM_LD_BYTES_PC;
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
                Self::timex_redirect_spectrum_ld_bytes(cpu, bus);
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
                    if bus.timex_2068 {
                        bus.ay.advance(HOLD_T);
                    }
                    bus.frame_t = (bus.frame_t + HOLD_T) % FRAME_TSTATES_48;
                    cpu.t = cpu.t.wrapping_add(u64::from(HOLD_T));
                    return;
                }
                if tape_opts.flash_load && Self::try_flash_load_48(cpu, bus, tape) {
                    return;
                }
                if int_active_48(bus.frame_t) && !(bus.timex && bus.timex_scld.int_disabled()) {
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
                        if bus.timex_2068 {
                            bus.ay.advance(irq_t);
                        }
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
                if bus.timex_2068 {
                    bus.ay.advance(dt);
                }
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
                if bus.timex_2068 {
                    bus.ay.advance(dt);
                }
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
                if bus.timex {
                    let ff = bus.timex_scld.port_ff();
                    let screen = &bus.ram[..0x4000.min(bus.ram.len())];
                    if let Some(hires) = ula::TimexHiresMode::from_scrnmode(ff) {
                        bus.ula
                            .render_rgba_timex_hires(screen, out, with_border, hires, ff);
                    } else {
                        let mode = ula::TimexLoresMode::from_scrnmode(ff);
                        bus.ula
                            .render_rgba_timex_lores(screen, out, with_border, mode);
                    }
                } else {
                    bus.ula.render_rgba(bus.screen_bytes(), out, with_border);
                }
            }
            Self::Spec128 { bus, .. } => {
                bus.ula.render_rgba_timed_dual(
                    &bus.banks[5][..6912],
                    &bus.banks[7][..6912],
                    out,
                    with_border,
                    ula::PAPER_START_128,
                    ula::T_LINE_128,
                );
            }
            Self::SpecPlus3 { bus, .. } => {
                bus.ula.render_rgba_timed_dual(
                    &bus.banks[5][..6912],
                    &bus.banks[7][..6912],
                    out,
                    with_border,
                    ula::PAPER_START_128,
                    ula::T_LINE_128,
                );
            }
        }
    }

    /// Host RGBA size for the current SCLD mode (`with_border` selects border chrome).
    #[must_use]
    pub fn framebuffer_dims(&self, with_border: bool) -> (usize, usize) {
        let hires = self.framebuffer_hires();
        ula::framebuffer_dims(with_border, hires)
    }

    /// True when Timex SCLD is in a hi-res screen mode (512×192 paper).
    #[must_use]
    pub fn framebuffer_hires(&self) -> bool {
        matches!(self, Self::Spec48 { bus, .. } if bus.timex && bus.timex_scld.screen_mode().is_hires())
    }

    /// Timex SCLD `port_ff` low three bits (screen mode), when Timex hardware is active.
    #[must_use]
    pub fn timex_scld_mode(&self) -> Option<u8> {
        match self {
            Self::Spec48 { bus, .. } if bus.timex => Some(bus.timex_scld.port_ff() & 0x07),
            _ => None,
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
            Model::Spectrum16K | Model::Spectrum48 | Model::TimexTC2048 | Model::TimexTS2068 => {
                self.type_load_quotes_48k(with_code)
            }
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
            0xd3, 0x3f, // OUT (3Fh),A  track 0
            0x3e, 0x00, // LD A,0 — sector 0 (same size as old LD A,1 for jump targets)
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
        for _ in 0..100_000 {
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
        let Some(trdos) = trdos_rom_bytes() else {
            eprintln!("skip: roms/trdos.rom missing (optional #140 TR-DOS boot fixture)");
            return;
        };
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

    fn trdos_rom_bytes() -> Option<Vec<u8>> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let path = resolve_trdos_rom_preferring_file_services(
            std::slice::from_ref(&root),
            trdos_rom_candidates(Model::Pentagon128),
        )?;
        let data = std::fs::read(path).ok()?;
        (data.len() == bus::TRDOS_ROM_SIZE).then_some(data)
    }

    /// Hole-filled 5.04 (or any dump) for the harnessed `19ECh` stand-in path.
    /// Prefers `roms/pentagon/trdos.rom` so a complete `trdos-5.04t.rom` does not
    /// change the established RUN→boot fixture behaviour. Never returns a dump
    /// with native `08D2h`/`0D6Bh` services (those belong on the complete path).
    fn trdos_rom_bytes_harness() -> Option<Vec<u8>> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let preferred = [
            "roms/pentagon/trdos.rom",
            "roms/trdos/trdos.rom",
            "roms/trdos.rom",
        ];
        let mut fallback: Option<Vec<u8>> = None;
        for rel in preferred
            .iter()
            .copied()
            .chain(trdos_rom_candidates(Model::Pentagon128).iter().copied())
        {
            let p = root.join(rel);
            let Ok(data) = std::fs::read(&p) else {
                continue;
            };
            if data.len() != bus::TRDOS_ROM_SIZE {
                continue;
            }
            if trdos_rom_fills_0800_hole(&data) {
                continue;
            }
            // Prefer explicit hole-dump paths when present.
            if preferred.contains(&rel) {
                return Some(data);
            }
            if fallback.is_none() {
                fallback = Some(data);
            }
        }
        fallback
    }

    /// Complete dump only (fills the usual 5.04 `0800h` hole), if present.
    fn trdos_rom_bytes_complete() -> Option<Vec<u8>> {
        let data = trdos_rom_bytes()?;
        trdos_rom_fills_0800_hole(&data).then_some(data)
    }

    /// Synthetic TR-DOS ROM: read track 1 sector 1 (BASIC `boot`) into `8000h`.
    fn trdos_read_boot_basic_rom() -> [u8; bus::TRDOS_ROM_SIZE] {
        let mut rom = [0u8; bus::TRDOS_ROM_SIZE];
        let code: &[u8] = &[
            0x3e, 0x3c, // LD A,3Ch
            0xd3, 0xff, // OUT (FFh),A
            0x3e, 0x01, // LD A,1
            0xd3, 0x3f, // OUT (3Fh),A  track 1
            0x3e, 0x00, // LD A,0
            0xd3, 0x5f, // OUT (5Fh),A  sector 0
            0x3e, 0x80, // LD A,80h
            0xd3, 0x1f, // OUT (1Fh),A
            0x21, 0x00, 0x80, // LD HL,8000h
            0x01, 0x7f, 0x00, // LD BC,007Fh
            0xdb, 0xff, // IN A,(FFh)
            0xe6, 0xc0, // AND C0h
            0x28, 0xfa, // JR Z, wait
            0xfa, 0x23, 0x3d, // JP M, HALT (LD A,track is one byte longer than XOR A)
            0xed, 0xa2, // INI
            0x18, 0xf3, // JR wait
            0x76, // HALT
        ];
        rom[0x3d00..0x3d00 + code.len()].copy_from_slice(code);
        rom
    }

    #[test]
    fn beta_reads_synthetic_boot_basic_into_ram() {
        let mut m = Machine::new_48k(&[0u8; 16384]).unwrap();
        m.load_trdos_rom(&trdos_read_boot_basic_rom()).unwrap();
        m.insert_trd(formats::TrdImage::synthetic_trdos_boot_basic())
            .unwrap();
        m.cpu_mut().regs.pc = 0x3d00;
        m.cpu_mut().regs.sp = 0xfffd;
        for _ in 0..4000 {
            if m.cpu().regs.halted {
                break;
            }
            m.step_once();
        }
        assert!(m.cpu().regs.halted, "synthetic TR-DOS loop should HALT");
        assert_eq!(m.read_mem(0x8000), 0x00);
        assert_eq!(m.read_mem(0x8001), 0x0a);
        assert_eq!(m.read_mem(0x8002), 0x17);
        assert_eq!(m.read_mem(0x8003), 0x00);
        assert_eq!(m.read_mem(0x8004), 0xf4); // POKE
        assert_eq!(m.beta_mut().map(|b| b.sector_read_count), Some(1));
    }

    /// Synthetic TR-DOS ROM: WRITE TRACK one sector then read it back.
    fn trdos_write_track_rom() -> [u8; bus::TRDOS_ROM_SIZE] {
        let mut rom = [0u8; bus::TRDOS_ROM_SIZE];
        let code: &[u8] = &[
            0x3e, 0x3c, // LD A,3Ch
            0xd3, 0xff, // OUT (FFh),A
            0xaf, // XOR A
            0xd3, 0x3f, // OUT (3Fh),A  track 0
            0x3e, 0xf0, // LD A,F0h
            0xd3, 0x1f, // OUT (1Fh),A  WRITE TRACK
            0x3e, 0xfe, // ID: FE
            0xd3, 0x7f, // OUT (7Fh),A
            0xaf, // track 0
            0xd3, 0x7f, 0xaf, // side 0
            0xd3, 0x7f, 0x3e, 0x02, // sector 2
            0xd3, 0x7f, 0x3e, 0x01, // 256 bytes
            0xd3, 0x7f, 0x3e, 0xf7, // CRC
            0xd3, 0x7f, 0x3e, 0xfb, // data mark
            0xd3, 0x7f, 0x3e, 0xbe, // fill byte
            0x06, 0x00, // LD B,0  (256 bytes)
            0xd3, 0x7f, // loop: OUT (7Fh),A
            0x10, 0xfc, // DJNZ loop (-4 → 3D29h)
            0x3e, 0xf7, 0xd3, 0x7f, 0x3e, 0xd8, // Force interrupt
            0xd3, 0x1f, 0x3e, 0x02, // read sector ID 2 (VG93 sector register)
            0xd3, 0x5f, 0x3e, 0x80, 0xd3, 0x1f, 0x21, 0x00, 0x60, // HL=6000h
            0x01, 0x7f, 0x00, 0xdb, 0xff, 0xe6, 0xc0, 0x28, 0xfa, 0xfa, 0x50,
            0x3d, // JP M, HALT
            0xed, 0xa2, 0x18, 0xf3, 0x76,
        ];
        rom[0x3d00..0x3d00 + code.len()].copy_from_slice(code);
        rom
    }

    #[test]
    fn beta_write_track_via_synthetic_rom() {
        let mut m = Machine::new_48k(&[0u8; 16384]).unwrap();
        m.load_trdos_rom(&trdos_write_track_rom()).unwrap();
        m.insert_trd(synthetic_trd_with_marker(0, 0)).unwrap();
        m.cpu_mut().regs.pc = 0x3d00;
        m.cpu_mut().regs.sp = 0xfffd;
        for _ in 0..50_000 {
            if m.cpu().regs.halted {
                break;
            }
            m.step_once();
        }
        assert!(m.cpu().regs.halted);
        assert_eq!(m.read_mem(0x6000), 0xbe);
        assert_eq!(m.beta_mut().map(|b| b.write_track_count), Some(1));
    }

    fn rom_pentagon() -> Option<Vec<u8>> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        for rel in ["roms/pentagon/pentagon.rom", "roms/pentagon/128p.rom"] {
            if let Ok(data) = std::fs::read(root.join(rel)) {
                if data.len() == 32768 {
                    return Some(data);
                }
            }
        }
        None
    }

    fn init_trdos_usr_call_frame(m: &mut Machine) {
        m.cpu_mut().regs.sp = 0xfffe;
        m.cpu_mut().regs.set_hl(0);
    }

    fn enter_128k_basic_from_menu(m: &mut Machine) {
        const PRESS: u32 = 15;
        const GAP: u32 = 5;
        for _ in 0..250 {
            let _ = m.run_frame();
        }
        m.hold_keys(&[(0, 0), (4, 4)], PRESS);
        m.hold_keys(&[], GAP);
        m.hold_keys(&[(6, 0)], PRESS);
        m.hold_keys(&[], 30);
        for _ in 0..400 {
            let _ = m.run_frame();
        }
    }

    fn ensure_trdos_beta128_prog(m: &mut Machine) {
        let prog = u16::from(m.read_mem(0x5c4f)) | (u16::from(m.read_mem(0x5c50)) << 8);
        let chans = u16::from(m.read_mem(0x5c4d)) | (u16::from(m.read_mem(0x5c4e)) << 8);
        if chans == 0 {
            // Keep CHANS clear of `5D25h` sector buffer and PROG at `5E00h`.
            const CHANS: u16 = 0x5f00;
            m.write_mem(0x5c4d, (CHANS & 0xff) as u8);
            m.write_mem(0x5c4e, (CHANS >> 8) as u8);
        }
        // Beta128 `3D21h` requires `(PROG) >= 5D25h`, but `5D25h` is also the TR-DOS
        // 256-byte sector buffer (`1E4Bh` / `197Eh`). Park PROG above that window.
        if prog < 0x5e00 {
            const PROG: u16 = 0x5e00;
            m.write_mem(0x5c4f, (PROG & 0xff) as u8);
            m.write_mem(0x5c50, (PROG >> 8) as u8);
            m.write_mem(0x5c51, ((PROG + 1) & 0xff) as u8);
            m.write_mem(0x5c52, ((PROG + 1) >> 8) as u8);
            m.write_mem(PROG, 0x80);
        }
        // TR-DOS command parse (`3032h` / `02FCh`) reads Spectrum `(PROG)` at `5C59h`,
        // while Beta128 entry checks `5C4Fh`. Keep both pointers on the same line buffer.
        let prog = u16::from(m.read_mem(0x5c4f)) | (u16::from(m.read_mem(0x5c50)) << 8);
        m.write_mem(0x5c59, (prog & 0xff) as u8);
        m.write_mem(0x5c5a, (prog >> 8) as u8);
        // Find-boot (`195Ch`): `LD A,(5CF9); CP #FF; JP NZ,1E3Dh`. Non-`FF` skips the
        // catalog scan and enters load with `B=0` → `1E74h RET Z` (no Type-II). Init
        // copies `(5CF6)→(5CF9)`; `1812h` sets `#FF` on the named-RUN path we may miss.
        m.write_mem(0x5cf6, 0xff);
        m.write_mem(0x5cf9, 0xff);
        m.write_mem(0x5d17, 0xaa);
        m.write_mem(0x5d0f, 0x00);
        m.write_mem(0x5d16, 0x3c);
    }

    /// Map PC to a TR-DOS ROM offset. [`BetaDisk::read_rom`] only overlays
    /// `0000–3FFF`, so any higher PC is RAM and must not be treated as ROM.
    fn trdos_rom_pc(pc: u16) -> Option<u16> {
        (pc < 0x4000).then_some(pc)
    }

    /// `5CC2h` RST `#20` gate used by `2F72h`.
    ///
    /// Stock DOS init writes a lone `C9`. With that stub, `3D94h` `RST #20` / inline
    /// `0010h` falls into `JP 3D82h` and recurses (`CALL 3D94h` again). Skip only the
    /// `DE==0010h` service so `3D94h` can stay unpatched; other vectors (e.g. `1F54h`)
    /// still run. Bytes live in the DOS printer-buffer hole at `5CC2h` (11 bytes).
    fn install_trdos_rst20_5cc2_hook(m: &mut Machine) {
        // POP HL / CP L,#10 / JR NZ,do / OR H / RET Z / PUSH HL / RET
        const HOOK: [u8; 11] = [
            0xe1, 0x7d, 0xfe, 0x10, 0x20, 0x03, 0x7c, 0xb7, 0xc8, 0xe5, 0xc9,
        ];
        for (i, &b) in HOOK.iter().enumerate() {
            m.write_mem(0x5cc2 + i as u16, b);
        }
    }

    /// Harness entry for ROM-gated `RUN` → `boot` (#266 / #140).
    ///
    /// CAT / VG93-wait / PROG-wipe sites stay **stock** — see
    /// [`apply_trdos_run_native_abi`]. `3D94h` uses [`install_trdos_rst20_5cc2_hook`].
    /// Remaining gap: this 5.04 image has FF from `0800h`–`0E71h` (`08D2h` and
    /// `0D6Bh`). Native `012Ah` re-enters catalog before LINE-NEW; `19ECh`
    /// VG93+LINE-NEW handoff lives in [`apply_trdos_run_native_abi`].
    fn patch_trdos_run_harness_rom(_m: &mut Machine) {
        // No ROM writes — CAT/wait/`19ECh` reductions live in `apply_trdos_run_native_abi`.
    }

    /// Stock find-boot ABI so catalog ROM stays unpatched (#140 / #266).
    ///
    /// `195Ch` stores caller `DE` as catalog CHS (`1964h`) then `LD C,0` (`1968h`).
    /// The sibling entry `1946h` skips that and loads `C` from `(5CDB)`. Seed name
    /// `HL=5EE0h`, CHS `DE=0`, one catalog sector `B=1` at `195Ch`, and `C=16` at
    /// `196Ah` (after `LD C,0`) so the 16-byte dirent compare / `DJNZ` RET need no
    /// ROM writes.
    fn apply_trdos_find_boot_native_abi(m: &mut Machine) {
        const NAME: u16 = 0x5ee0;
        match m.cpu().regs.pc {
            0x195c => {
                m.cpu_mut().regs.set_hl(NAME);
                m.cpu_mut().regs.set_de(0);
                m.cpu_mut().regs.b = 1;
            }
            0x196a => {
                m.cpu_mut().regs.set_hl(NAME);
                m.cpu_mut().regs.set_de(0);
                m.cpu_mut().regs.set_bc(0x0110); // B=1, C=16
            }
            _ => {}
        }
    }

    /// True when the *currently loaded* TR-DOS image has a classic file-load at `08D2h`.
    ///
    /// Must not consult the preferred-on-disk resolver: harness tests load the hole
    /// dump even when `trdos-5.04t.rom` exists beside it. Alone Coder 5.04T fills
    /// `08D2h` with a VG93 port stub — that is **not** a file service.
    fn trdos_rom_has_native_file_services_paged(m: &mut Machine) -> bool {
        let was = m.beta_mut().map(|b| b.paged).unwrap_or(false);
        if let Some(beta) = m.beta_mut() {
            beta.page_trdos(true);
        }
        let mut img = [0u8; bus::TRDOS_ROM_SIZE];
        for (i, b) in img.iter_mut().enumerate() {
            *b = m.read_mem(i as u16);
        }
        let ok = trdos_rom_has_native_file_services(&img);
        if let Some(beta) = m.beta_mut() {
            beta.page_trdos(was);
        }
        ok
    }

    /// Register / PC ABI so RUN harness ROM stays stock (#140).
    ///
    /// Replaces former ROM RET/NOP/JR patches at `3D9Dh` / `02D4h` / `213Eh` /
    /// `2155h` plus the `3DFFh` A=1 delay seed, and the post-match `19ECh`
    /// VG93+LINE-NEW stand-in (this image’s `08D2h` is FF padding).
    fn apply_trdos_run_native_abi(m: &mut Machine) {
        apply_trdos_find_boot_native_abi(m);
        let pc = m.cpu().regs.pc;
        match pc {
            // Stock `3DFFh`: `LD C,#FF` / `DEC C` until Z / `DEC A` / JR NZ.
            // Callers use `A=5` (`02C3h`) or `A=#FF` × `B=3` (`3EA4h` motor spin).
            // `A=1` runs one inner 255-iter loop instead of a ROM RET patch.
            0x3dff => {
                m.cpu_mut().regs.a = 1;
            }
            // `3D9Ah` Type-I wait: stock `RST #20`→`1F54h` never unwinds with
            // instant completion — skip to `3DA5h` `POP HL` (was `JR 3DA5h` patch).
            0x3d9d => {
                m.cpu_mut().regs.pc = 0x3da5;
            }
            // Warm `02CBh` `CALL 1D83h` CAT blocks on a key (`161Dh`) — skip the CALL.
            0x02d4 => {
                m.cpu_mut().regs.pc = 0x02d7;
            }
            // 5.04T warm path: `0249h` is `CALL 3AE6h` (XOR A/OUT (9)/LD HL,5D17/RET)
            // where hole 5.04 inlines `LD HL,5D17`. Skip the CALL so SP/IFF stay aligned
            // with the harnessed hole path (native `08D2h` still runs at `19ECh`).
            0x0249
                if m.read_mem(0x0249) == 0xcd
                    && m.read_mem(0x024a) == 0xe6
                    && m.read_mem(0x024b) == 0x3a =>
            {
                m.cpu_mut().regs.set_hl(0x5d17);
                m.cpu_mut().regs.pc = 0x024c;
            }
            // `213Eh` `CALL Z,211Eh` wipes `(PROG)` when Z (`5D0F=0`); keep seeded
            // `RUN\\r` for `3032h` by skipping the call (was three NOPs).
            0x213e => {
                m.cpu_mut().regs.pc = 0x2141;
            }
            // `2155h` stock `JP 1D90h` (CAT) never returns to `02ECh` — RET to caller.
            0x2155 => {
                let sp = m.cpu().regs.sp;
                let ret =
                    u16::from(m.read_mem(sp)) | (u16::from(m.read_mem(sp.wrapping_add(1))) << 8);
                m.cpu_mut().regs.sp = sp.wrapping_add(2);
                m.cpu_mut().regs.pc = ret;
            }
            // 5.04T: `1FEBh` ends `JP 0897h` instead of hole `OUT (#FF),A / RET`.
            // `CALL 1FEBh` from `3E63h` must return so catalog `1E3Dh` can finish.
            0x1ff3
                if m.read_mem(0x1ff3) == 0xc3
                    && m.read_mem(0x1ff4) == 0x97
                    && m.read_mem(0x1ff5) == 0x08 =>
            {
                let sys = m.cpu().regs.a;
                if let Some(beta) = m.beta_mut() {
                    let _ = beta.out_port(0x00ff, sys);
                }
                let sp = m.cpu().regs.sp;
                let ret =
                    u16::from(m.read_mem(sp)) | (u16::from(m.read_mem(sp.wrapping_add(1))) << 8);
                m.cpu_mut().regs.sp = sp.wrapping_add(2);
                m.cpu_mut().regs.pc = ret;
            }
            // Stock `19ECh`: `RST #20` / `DW 08D2h`. On hole dumps, never enter
            // `08D2h` FF padding — FDC-load `boot` and enter Spectrum `LINE-NEW`.
            // Complete dumps (non-FF at `08D2h`) run the native service.
            0x19ec
                if !trdos_rom_has_native_file_services_paged(m)
                    && trdos_fdc_load_boot_into_prog(m) =>
            {
                m.cpu_mut().regs.pc = 0x1b76;
            }
            _ => {}
        }
    }

    /// Invoke TR-DOS `RUN` with no filename (loads `boot`).
    ///
    /// Warm entry `0239h`→`02E9h`→`3032h` reaches find-boot Type-II catalog reads.
    /// Post-match `19ECh` is stock `RST #20` / inline `08D2h`; this ROM's `08D2h` is
    /// FF padding, so [`apply_trdos_run_native_abi`] FDC-loads `boot` at the **call
    /// site**, unpages TR-DOS, and enters Spectrum `LINE-NEW` (`1B76h`) — never
    /// executes the hole.
    /// Name block lives at `5EE0h` so it does not overlap the `5D25h` sector buffer.
    fn invoke_trdos_run_boot(m: &mut Machine) -> bool {
        m.write_mem(0x5cb6, 0xf4);
        m.write_mem(0x5cb7, 0x0d);
        // RST #20 epilogue (`2F72h`) enters `5CC2h` before the inline service address.
        // Skip recursive `0010h`→`3D82h` so stock `3D94h` (`RST #20` / `0010h`) returns.
        install_trdos_rst20_5cc2_hook(m);
        m.write_mem(0x5d0f, 0);
        // Find-boot sentinel for `1921h` `CALL Z,195Ch`.
        m.write_mem(0x5d10, 0xff);
        // Seed Spectrum `(PROG)` at `5C59h`: ASCII `RUN` + CR so `3032h` tokenizes.
        let prog = u16::from(m.read_mem(0x5c59)) | (u16::from(m.read_mem(0x5c5a)) << 8);
        for (i, &b) in b"RUN\r\x80".iter().enumerate() {
            m.write_mem(prog.wrapping_add(i as u16), b);
        }
        // Re-assert find-boot catalog gate after `USR 15616` / warm path.
        m.write_mem(0x5cf6, 0xff);
        m.write_mem(0x5cf9, 0xff);
        // Sector count for find-boot outer `B` (`5CDC`) + `1946h` `C` from `(5CDB)`
        // (dirent length; `195Ch` still `LD C,0` and is fixed at `196Ah`).
        m.write_mem(0x5cdb, 0x10);
        m.write_mem(0x5cdc, 0x08);
        // Catalog start CHS (`5CD9` / `5CF4`); `195Ch` `LD (5CF4),DE` needs `DE=0`.
        m.write_mem(0x5cd9, 0x00);
        m.write_mem(0x5cda, 0x00);
        m.write_mem(0x5cf4, 0x00);
        m.write_mem(0x5cf5, 0x00);
        // Full 16-byte TR-DOS dirent for synthetic `boot` — **above** `5D25h` buffer.
        const NAME: u16 = 0x5ee0;
        let dirent: [u8; 16] = [
            b'b', b'o', b'o', b't', b' ', b' ', b' ', b' ', b'B', 28, 0, 27, 0, 1, 0, 1,
        ];
        for (i, &b) in dirent.iter().enumerate() {
            m.write_mem(NAME + i as u16, b);
        }
        m.write_mem(0x5cd7, (NAME & 0xff) as u8);
        m.write_mem(0x5cd8, (NAME >> 8) as u8);
        patch_trdos_run_harness_rom(m);
        let sp = m.cpu().regs.sp;
        m.write_mem(0x5c3d, (sp & 0xff) as u8);
        m.write_mem(0x5c3e, (sp >> 8) as u8);
        // `3D34h` `PUSH HL` after `3D21h` (`HL=5CC2h`), then `3D35h` → `0239h`.
        m.cpu_mut().regs.set_hl(0x5cc2);
        m.cpu_mut().regs.set_de(0);
        m.cpu_mut().regs.sp = sp.wrapping_sub(2);
        m.write_mem(sp.wrapping_sub(2), 0xc2);
        m.write_mem(sp.wrapping_sub(1), 0x5c);
        m.write_mem(0x5d17, 0xaa);
        m.cpu_mut().regs.pc = 0x0239;
        wait_for_trdos_boot_marker(m, 10_000)
    }

    fn manual_read_track1_sector1(m: &mut Machine) -> bool {
        let Some(beta) = m.beta_mut() else {
            return false;
        };
        beta.page_trdos(true);
        beta.out_port(0x00ff, 0x3c);
        beta.out_port(0x003f, 1);
        beta.out_port(0x005f, 1);
        beta.out_port(0x001f, 0x80);
        if beta.sector_read_count == 0 {
            return false;
        }
        let mut ok = true;
        for _ in 0..256 {
            let Some(st) = beta.in_port(0x001f) else {
                ok = false;
                break;
            };
            if st & 0x02 != 0 {
                let _ = beta.in_port(0x007f);
            }
            if st & 0x80 != 0 {
                break;
            }
        }
        ok
    }

    /// Enter TR-DOS command mode via `USR 15616` (`3D00h` → `3D31h`).
    ///
    /// Paging is asserted (it works today); reaching the `3D31h` command loop is
    /// returned rather than asserted because #140 RUN boot is still open.
    fn enter_trdos_command_mode(m: &mut Machine) -> bool {
        init_trdos_usr_call_frame(m);
        m.cpu_mut().regs.pc = 0x3d00;
        let mut saw_paged = false;
        let mut at_prompt = false;
        for _ in 0..20_000_000 {
            m.step_once();
            if m.beta_mut().is_some_and(|b| b.paged) {
                saw_paged = true;
            }
            if saw_paged && trdos_rom_pc(m.cpu().regs.pc) == Some(0x3D31) {
                at_prompt = true;
                break;
            }
        }
        assert!(
            saw_paged,
            "TR-DOS should page (PC={:#06x})",
            m.cpu().regs.pc
        );
        at_prompt
    }

    /// After find-boot matches `boot`, stock `19ECh` would `RST #20` into `08D2h`,
    /// which is FF padding on this ROM image. Load the file body through the real
    /// VG93 path into `(PROG)`, wire Spectrum sysvars / `NEWPPC` / FLAGS bit 7
    /// (running), unpage TR-DOS, page 48K BASIC ROM, and enter `LINE-NEW` (`1B76h`).
    ///
    /// Why not TR-DOS `012Ah` / native `08D2h` service (this image):
    /// - FF padding `0800h`–`0E71h` covers `08D2h` and `0D6Bh` (`012Ah` `CALL 1D97h`).
    /// - Entering `012Ah` from `19ECh` re-enters catalog (`30B2h`) / Type-I wait
    ///   (`3D9Ch`) before LINE-NEW; `RST #20`/`16B0h` is mid-`CALL 166Fh`.
    /// - Beta keeps the TR-DOS latch across RAM, so stock `5CC2h`→`1B76h` would still
    ///   fetch TR-DOS at `1B76h`.
    /// - 128/Pentagon ROM0 is the editor; `1B76h` LINE-NEW lives in ROM1 (`7FFDh` bit 4).
    fn trdos_fdc_load_boot_into_prog(m: &mut Machine) -> bool {
        let prog = u16::from(m.read_mem(0x5c59)) | (u16::from(m.read_mem(0x5c5a)) << 8);
        let start_sec = m.read_mem(0x5d25 + 14);
        let start_trk = m.read_mem(0x5d25 + 15);
        let file_type = m.read_mem(0x5d25 + 8);
        let len = u16::from(m.read_mem(0x5d25 + 9)) | (u16::from(m.read_mem(0x5d25 + 10)) << 8);
        if file_type != b'B' || start_trk == 0 || len == 0 || len > 256 {
            return false;
        }
        let mut buf = [0u8; 256];
        {
            let Some(beta) = m.beta_mut() else {
                return false;
            };
            beta.page_trdos(true);
            beta.out_port(0x00ff, 0x3c);
            beta.out_port(0x003f, start_trk);
            // TR-DOS dirent sector 0 → VG93 ID 1 (see `BetaDisk::sector_index`).
            beta.out_port(0x005f, start_sec.max(1));
            beta.out_port(0x001f, 0x80);
            if beta.sector_read_count == 0 {
                return false;
            }
            for b in &mut buf {
                let mut spins = 0u32;
                loop {
                    let st = beta.in_port(0x001f).unwrap_or(0);
                    if st & 0x02 != 0 {
                        break;
                    }
                    if st & 0x80 != 0 && st & 0x02 == 0 {
                        return false;
                    }
                    spins += 1;
                    if spins > 10_000 {
                        return false;
                    }
                }
                *b = beta.in_port(0x007f).unwrap_or(0);
            }
        }
        // Program + empty VARS only (`len`); autostart `AAh` trailer stays out of E_LINE.
        for (i, &b) in buf[..len as usize].iter().enumerate() {
            m.write_mem(prog.wrapping_add(i as u16), b);
        }
        let vars_off =
            u16::from(m.read_mem(0x5d25 + 11)) | (u16::from(m.read_mem(0x5d25 + 12)) << 8);
        let vars = prog.wrapping_add(vars_off);
        let e_line = vars.wrapping_add(1);
        let write_u16 = |m: &mut Machine, addr: u16, val: u16| {
            m.write_mem(addr, (val & 0xff) as u8);
            m.write_mem(addr.wrapping_add(1), (val >> 8) as u8);
        };
        // Standard Spectrum sysvars (TR-DOS harness aliases `5C4Fh`/`5C59h` differently).
        // Harness parked channel info at `5C4Dh` and PROG at `5C4Fh` — restore CHANS.
        let chans = u16::from(m.read_mem(0x5c4d)) | (u16::from(m.read_mem(0x5c4e)) << 8);
        write_u16(m, 0x5c4b, vars); // VARS
                                    // Prefer a live channel block left by 128 BASIC entry. The TR-DOS harness
                                    // parks a blank `5F00h` window and aliases CHANS at `5C4Dh` while overwriting
                                    // standard `5C4Fh` with PROG — recover the post-menu pointer when present.
        let standard_chans = u16::from(m.read_mem(0x5c4f)) | (u16::from(m.read_mem(0x5c50)) << 8);
        let chans_ptr = if (0x5b00..0x5e00).contains(&standard_chans) {
            standard_chans
        } else if (0x5b00..0x5e00).contains(&chans) {
            chans
        } else {
            // Minimal K/S/R/P channels (5 bytes each: OUT, IN, letter) as NEW installs.
            const CHANS: u16 = 0x5f00;
            let block: [u8; 21] = [
                0xf4, 0x09, 0xa8, 0x10, b'K', // PRINT-OUT / KEY-INPUT
                0xf4, 0x09, 0xc4, 0x15, b'S', // PRINT-OUT / KEY-INPUT
                0x81, 0x0f, 0xc4, 0x15, b'R', // ADD-CHAR / KEY-INPUT
                0xf4, 0x09, 0xc4, 0x15, b'P', // PRINT-OUT / KEY-INPUT
                0x80, // end marker
            ];
            for (i, &b) in block.iter().enumerate() {
                m.write_mem(CHANS.wrapping_add(i as u16), b);
            }
            CHANS
        };
        write_u16(m, 0x5c4f, chans_ptr); // CHANS
        write_u16(m, 0x5c51, chans_ptr); // CURCHL → first channel
                                         // Stream offsets from CHANS (NEW defaults).
        for (i, off) in [0x0001u16, 0x0006, 0x000b, 0x0001, 0x0001, 0x0006, 0x0010]
            .into_iter()
            .enumerate()
        {
            write_u16(m, 0x5c10 + (i as u16) * 2, off);
        }
        write_u16(m, 0x5c53, prog); // PROG
        write_u16(m, 0x5c59, e_line); // E_LINE
        write_u16(m, 0x5c61, e_line); // WORKSP
        write_u16(m, 0x5c63, e_line); // STKBOT
        write_u16(m, 0x5c65, e_line); // STKEND
                                      // Empty edit line terminator at E_LINE (required by many ROM walks).
        m.write_mem(e_line, 0x0d);
        m.write_mem(e_line.wrapping_add(1), 0x80);
        // Autostart LINE from TR-DOS trailer (`AAh`, line LE) or first program line.
        let newppc = if buf.get(len as usize) == Some(&0xaa) {
            u16::from(buf[len as usize + 1]) | (u16::from(buf[len as usize + 2]) << 8)
        } else {
            u16::from(buf[1]) | (u16::from(buf[0]) << 8)
        };
        write_u16(m, 0x5c42, newppc); // NEWPPC
        m.write_mem(0x5c44, 0); // NSPPC = first statement
                                // SYNTAX-Z (`1C11h`) is `BIT 7,(IY+1)`: Z set when bit 7 is
                                // clear, i.e. syntax-checking. TR-DOS/128 editor leftover FLAGS
                                // `1Dh` keeps DECIMAL inserting a second `0x0E` (`00 00 00 80 00`
                                // for 32768) in front of the stored `90…` float → Report C.
        m.write_mem(0x5c3b, m.read_mem(0x5c3b) | 0x80);
        // Leave DOS + select 48K BASIC ROM so `1B76h` is LINE-NEW.
        if let Some(beta) = m.beta_mut() {
            beta.page_trdos(false);
        }
        if let Machine::Spec128 { bus, .. } = m {
            let page = bus.page | 0x10; // bit 4 → ROM1 (48K BASIC)
            bus.out_7ffd(page);
            // Keep BANK_M (`5B5C`) in sync — 128 SWAP at `5B00h` XORs bit 4 from
            // this shadow copy. Desync (e.g. `07` while port is `17`) makes ROM1
            // `3B4Dh`→`0112h` run ROM1 message bytes instead of ROM0 Statement Return.
            m.write_mem(0x5b5c, page);
        }
        true
    }

    fn wait_for_trdos_boot_marker(m: &mut Machine, max_frames: u32) -> bool {
        let native = trdos_rom_has_native_file_services_paged(m);
        let mut saw_08d2 = false;
        // Prefer instruction steps so we cannot miss the one-instruction `19ECh`
        // call-site window inside a full frame (`apply_trdos_run_native_abi`).
        let max_steps = u64::from(max_frames).saturating_mul(70_000).min(3_000_000);
        for _ in 0..max_steps {
            if m.cpu().regs.pc == 0x08d2 {
                saw_08d2 = true;
            }
            apply_trdos_run_native_abi(m);
            m.step_once();
            if m.read_mem(0x8000) == 0xa5 {
                if native {
                    assert!(
                        saw_08d2,
                        "complete ROM: RUN boot must enter native 08D2h file-load service"
                    );
                } else {
                    assert!(
                        !saw_08d2,
                        "hole dump: RUN boot must not execute 08D2h FF padding (handoff at 19ECh)"
                    );
                }
                return true;
            }
        }
        false
    }

    /// Optional: real `roms/trdos.rom` + 128K main ROM. Skips when either is missing.
    #[test]
    fn trdos_rom_reads_boot_when_128k_chans_ok_and_fixture_present() {
        let Some(main) = rom_pentagon().or_else(rom128) else {
            eprintln!("skip: pentagon/128 main ROM missing");
            return;
        };
        let Some(trdos) = trdos_rom_bytes_harness() else {
            eprintln!("skip: roms/trdos.rom missing (optional #140 TR-DOS boot fixture)");
            return;
        };
        let mut m = Machine::new_pentagon128(&main, &trdos).unwrap();
        m.insert_trd(formats::TrdImage::synthetic_trdos_boot_basic())
            .unwrap();
        enter_128k_basic_from_menu(&mut m);
        ensure_trdos_beta128_prog(&mut m);
        let at_prompt = enter_trdos_command_mode(&mut m);
        assert!(
            manual_read_track1_sector1(&mut m),
            "FDC should read boot sector after TR-DOS entry (PC={:#06x}, at_prompt={at_prompt})",
            m.cpu().regs.pc
        );
    }

    /// Debug: dump LINE-NEW handoff sysvars + PC trail until RST `#08` / marker.
    #[test]
    #[ignore]
    fn debug_trdos_line_new_handoff() {
        let Some(main) = rom_pentagon().or_else(rom128) else {
            return;
        };
        let Some(trdos) = trdos_rom_bytes() else {
            return;
        };
        let mut m = Machine::new_pentagon128(&main, &trdos).unwrap();
        m.insert_trd(formats::TrdImage::synthetic_trdos_boot_basic())
            .unwrap();
        enter_128k_basic_from_menu(&mut m);
        ensure_trdos_beta128_prog(&mut m);
        assert!(enter_trdos_command_mode(&mut m));

        // Drive RUN until the harness loads + jumps to LINE-NEW, then stop stepping
        // the DOS path and inspect Spectrum BASIC state.
        m.write_mem(0x5cb6, 0xf4);
        m.write_mem(0x5cb7, 0x0d);
        install_trdos_rst20_5cc2_hook(&mut m);
        m.write_mem(0x5d0f, 0);
        m.write_mem(0x5d10, 0xff);
        let prog0 = u16::from(m.read_mem(0x5c59)) | (u16::from(m.read_mem(0x5c5a)) << 8);
        for (i, &b) in b"RUN\r\x80".iter().enumerate() {
            m.write_mem(prog0.wrapping_add(i as u16), b);
        }
        m.write_mem(0x5cf6, 0xff);
        m.write_mem(0x5cf9, 0xff);
        m.write_mem(0x5cdb, 0x10);
        m.write_mem(0x5cdc, 0x08);
        m.write_mem(0x5cd9, 0x00);
        m.write_mem(0x5cda, 0x00);
        m.write_mem(0x5cf4, 0x00);
        m.write_mem(0x5cf5, 0x00);
        const NAME: u16 = 0x5ee0;
        let dirent: [u8; 16] = [
            b'b', b'o', b'o', b't', b' ', b' ', b' ', b' ', b'B', 28, 0, 27, 0, 1, 0, 1,
        ];
        for (i, &b) in dirent.iter().enumerate() {
            m.write_mem(NAME + i as u16, b);
        }
        m.write_mem(0x5cd7, (NAME & 0xff) as u8);
        m.write_mem(0x5cd8, (NAME >> 8) as u8);
        patch_trdos_run_harness_rom(&mut m);
        let sp = m.cpu().regs.sp;
        m.write_mem(0x5c3d, (sp & 0xff) as u8);
        m.write_mem(0x5c3e, (sp >> 8) as u8);
        m.cpu_mut().regs.set_hl(0x5cc2);
        m.cpu_mut().regs.set_de(0);
        m.cpu_mut().regs.sp = sp.wrapping_sub(2);
        m.write_mem(sp.wrapping_sub(2), 0xc2);
        m.write_mem(sp.wrapping_sub(1), 0x5c);
        m.write_mem(0x5d17, 0xaa);
        m.cpu_mut().regs.pc = 0x0239;

        let mut loaded = false;
        for _ in 0..3_000_000u32 {
            assert_ne!(
                m.cpu().regs.pc,
                0x08d2,
                "debug path must not enter 08D2h FF padding"
            );
            let at_19ec = m.cpu().regs.pc == 0x19ec;
            apply_trdos_run_native_abi(&mut m);
            if at_19ec && m.cpu().regs.pc == 0x1b76 {
                loaded = true;
                break;
            }
            m.step_once();
        }
        assert!(loaded, "did not reach 19ECh FDC handoff");

        let rd16 = |m: &Machine, a: u16| -> u16 {
            u16::from(m.read_mem(a)) | (u16::from(m.read_mem(a.wrapping_add(1))) << 8)
        };
        let page = match &m {
            Machine::Spec128 { bus, .. } => bus.page,
            _ => 0,
        };
        let paged = m.beta_mut().is_some_and(|b| b.paged);
        eprintln!(
            "handoff PC={:#06x} SP={:#06x} IY={:#06x} page={page:#04x} trdos={paged}",
            m.cpu().regs.pc,
            m.cpu().regs.sp,
            m.cpu().regs.iy()
        );
        let stub: Vec<u8> = (0..32).map(|i| m.read_mem(0x5b00 + i)).collect();
        eprintln!("  5B00 stub={stub:02x?}");
        eprintln!(
            "  ERR_NR={:#04x} FLAGS={:#04x} FLAGS2={:#04x} ERR_SP={:#06x} RAMTOP={:#06x}",
            m.read_mem(0x5c3a),
            m.read_mem(0x5c3b),
            m.read_mem(0x5c3c),
            rd16(&m, 0x5c3d),
            rd16(&m, 0x5cb2)
        );
        eprintln!(
            "  NEWPPC={:#06x} NSPPC={:#04x} PPC={:#06x} SUBPPC={:#04x}",
            rd16(&m, 0x5c42),
            m.read_mem(0x5c44),
            rd16(&m, 0x5c45),
            m.read_mem(0x5c47)
        );
        eprintln!(
            "  VARS={:#06x} CHANS={:#06x} PROG={:#06x} E_LINE={:#06x} WORKSP={:#06x} STKEND={:#06x}",
            rd16(&m, 0x5c4b),
            rd16(&m, 0x5c4f),
            rd16(&m, 0x5c53),
            rd16(&m, 0x5c59),
            rd16(&m, 0x5c61),
            rd16(&m, 0x5c65)
        );
        let strms: Vec<u16> = (0..7).map(|i| rd16(&m, 0x5c10 + i * 2)).collect();
        eprintln!("  STRMS={strms:04x?}");
        let prog = rd16(&m, 0x5c53);
        let prog_bytes: Vec<u8> = (0..32).map(|i| m.read_mem(prog.wrapping_add(i))).collect();
        eprintln!("  PROG bytes={prog_bytes:02x?}");
        let chans = rd16(&m, 0x5c4f);
        let chans_bytes: Vec<u8> = (0..21).map(|i| m.read_mem(chans.wrapping_add(i))).collect();
        eprintln!("  CHANS bytes={chans_bytes:02x?}");

        let mut last = 0xffffu16;
        let mut trail: Vec<u16> = Vec::new();
        for step in 0..50_000u32 {
            let pc = m.cpu().regs.pc;
            if pc != last {
                trail.push(pc);
                if trail.len() <= 40 || matches!(pc, 0x0008 | 0x3b4d | 0x0112 | 0x1b76 | 0x1b7d) {
                    eprintln!(
                        "step={step} PC={pc:#06x} A={:#04x} HL={:#06x} ERR_NR={:#04x} FLAGS={:#04x}",
                        m.cpu().regs.a,
                        m.cpu().regs.hl(),
                        m.read_mem(0x5c3a),
                        m.read_mem(0x5c3b)
                    );
                }
                last = pc;
            }
            if pc == 0x0008 {
                let sp = m.cpu().regs.sp;
                let ret =
                    u16::from(m.read_mem(sp)) | (u16::from(m.read_mem(sp.wrapping_add(1))) << 8);
                let err_byte = m.read_mem(ret);
                let ch_add = rd16(&m, 0x5c5d);
                let prog_now = rd16(&m, 0x5c53);
                let around: Vec<u8> = (0..32)
                    .map(|i| m.read_mem(prog_now.wrapping_add(i)))
                    .collect();
                let ch_around: Vec<u8> = (0i16..8)
                    .map(|i| m.read_mem(ch_add.wrapping_add((i - 2) as u16)))
                    .collect();
                eprintln!(
                    "RST8 at step={step} err_byte={err_byte:#04x} CH_ADD={ch_add:#06x} STKEND={:#06x} PROG={prog_now:#06x}",
                    rd16(&m, 0x5c65)
                );
                eprintln!("  PROG now={around:02x?}");
                eprintln!("  near CH_ADD(-2..+5)={ch_around:02x?}");
                eprintln!("  trail={:04x?}", &trail[trail.len().saturating_sub(24)..]);
                break;
            }
            m.step_once();
            if m.read_mem(0x8000) == 0xa5 {
                eprintln!("MARKER at step={step}");
                break;
            }
        }
        eprintln!(
            "final PC={:#06x} 8000={:#04x} ERR_NR={:#04x} page trail_len={} last={:04x?}",
            m.cpu().regs.pc,
            m.read_mem(0x8000),
            m.read_mem(0x5c3a),
            trail.len(),
            &trail[trail.len().saturating_sub(20)..]
        );
    }

    /// Debug: short PC/FDC trace for #266 RUN path (ignore in CI).
    #[test]
    #[ignore]
    fn debug_trdos_run_pc_trace() {
        let Some(main) = rom_pentagon().or_else(rom128) else {
            return;
        };
        let Some(trdos) = trdos_rom_bytes() else {
            return;
        };
        let mut m = Machine::new_pentagon128(&main, &trdos).unwrap();
        m.insert_trd(formats::TrdImage::synthetic_trdos_boot_basic())
            .unwrap();
        enter_128k_basic_from_menu(&mut m);
        ensure_trdos_beta128_prog(&mut m);
        assert!(enter_trdos_command_mode(&mut m));
        // Same setup as invoke_trdos_run_boot without waiting.
        m.write_mem(0x5cb6, 0xf4);
        m.write_mem(0x5cb7, 0x0d);
        install_trdos_rst20_5cc2_hook(&mut m);
        m.write_mem(0x5d0f, 0);
        m.write_mem(0x5d10, 0xff);
        let prog = u16::from(m.read_mem(0x5c59)) | (u16::from(m.read_mem(0x5c5a)) << 8);
        for (i, &b) in b"RUN\r\x80".iter().enumerate() {
            m.write_mem(prog.wrapping_add(i as u16), b);
        }
        m.write_mem(0x5cf6, 0xff);
        m.write_mem(0x5cf9, 0xff);
        m.write_mem(0x5cdb, 0x10);
        m.write_mem(0x5cdc, 0x08);
        m.write_mem(0x5cd9, 0x00);
        m.write_mem(0x5cda, 0x00);
        m.write_mem(0x5cf4, 0x00);
        m.write_mem(0x5cf5, 0x00);
        const NAME: u16 = 0x5ee0;
        let dirent: [u8; 16] = [
            b'b', b'o', b'o', b't', b' ', b' ', b' ', b' ', b'B', 28, 0, 27, 0, 1, 0, 1,
        ];
        for (i, &b) in dirent.iter().enumerate() {
            m.write_mem(NAME + i as u16, b);
        }
        m.write_mem(0x5cd7, (NAME & 0xff) as u8);
        m.write_mem(0x5cd8, (NAME >> 8) as u8);
        patch_trdos_run_harness_rom(&mut m);
        let sp = m.cpu().regs.sp;
        m.write_mem(0x5c3d, (sp & 0xff) as u8);
        m.write_mem(0x5c3e, (sp >> 8) as u8);
        m.cpu_mut().regs.set_hl(0x5cc2);
        m.cpu_mut().regs.set_de(0);
        m.cpu_mut().regs.sp = sp.wrapping_sub(2);
        m.write_mem(sp.wrapping_sub(2), 0xc2);
        m.write_mem(sp.wrapping_sub(1), 0x5c);
        m.write_mem(0x5d17, 0xaa);
        m.cpu_mut().regs.pc = 0x0239;
        let mut last = 0xffffu16;
        let mut hits = 0u32;
        let mut escaped = 0u32;
        let mut last_ring_len = 0usize;
        let mut saw_c0 = false;
        let watch = [
            0x02e9u16, 0x02ec, 0x2135, 0x2155, 0x3032, 0x030a, 0x031a, 0x1d4d, 0x1d50, 0x1836,
            0x187a, 0x18a1, 0x1921, 0x195c, 0x197e, 0x1997, 0x199c, 0x19dd, 0x08d2, 0x03fa, 0x1e3d,
            0x1e40, 0x1e4d, 0x1e62, 0x1e74, 0x1e75, 0x1e83, 0x3e63, 0x012a, 0x3dc8, 0x3dfa, 0x3f0e,
            0x3f25, 0x07d6, 0x0787,
        ];
        for step in 0..400_000u32 {
            let pc = m.cpu().regs.pc;
            if step % 50_000 == 0 {
                eprintln!(
                    "tick step={step} PC={pc:#06x} 5CB6={:#04x} 5D16={:#04x}",
                    m.read_mem(0x5cb6),
                    m.read_mem(0x5d16)
                );
            }
            if let Some(b) = m.beta_mut() {
                let len = b.command_ring().len();
                if len != last_ring_len {
                    eprintln!(
                        "cmd step={step} PC={pc:#06x} ring={:02x?} track={} sys={:#04x} drive={}",
                        b.command_ring(),
                        b.track,
                        b.system,
                        b.system & 3,
                    );
                    if b.command_ring().last() == Some(&0xc0)
                        || b.command_ring().last().is_some_and(|c| c & 0xe0 == 0x80)
                    {
                        saw_c0 |= b.command_ring().last() == Some(&0xc0);
                        hits = 0;
                    }
                    last_ring_len = len;
                }
            }
            if pc != last {
                let breg = m.cpu().regs.b;
                let sp = m.cpu().regs.sp;
                let sectors = m.beta_mut().map(|x| x.sector_read_count).unwrap_or(0);
                let in_rom = pc < 0x4000;
                let dos_stub = pc == 0x5cc2 || (0x5c00..0x5e00).contains(&pc);
                let interesting = watch.contains(&pc);
                let limit = if saw_c0 { 250 } else { 80 };
                if interesting || (in_rom && hits < limit) {
                    eprintln!(
                        "step={step} PC={pc:#06x} B={breg:#04x} SP={sp:#06x} sectors={sectors}"
                    );
                    if matches!(pc, 0x1e3d | 0x1e40 | 0x1e62 | 0x1e74 | 0x195c | 0x1d4d) {
                        let dir: Vec<u8> = (0..16).map(|i| m.read_mem(0x5d25 + i)).collect();
                        eprintln!(
                            "  A={:#04x} HL={:#06x} 5CF9={:#04x} 5C59→{:#06x} 5D25={:02x?}",
                            m.cpu().regs.a,
                            m.cpu().regs.hl(),
                            m.read_mem(0x5cf9),
                            u16::from(m.read_mem(0x5c59)) | (u16::from(m.read_mem(0x5c5a)) << 8),
                            dir
                        );
                    }
                    if in_rom && !interesting {
                        hits += 1;
                    }
                } else if !in_rom && !dos_stub {
                    eprintln!("escaped PC={pc:#06x} at step={step} SP={sp:#06x}");
                    escaped += 1;
                    if escaped >= 3 {
                        break;
                    }
                }
                last = pc;
            }
            apply_trdos_run_native_abi(&mut m);
            m.step_once();
            if m.read_mem(0x8000) == 0xa5 {
                eprintln!("MARKER at step={step}");
                break;
            }
        }
        let pc = m.cpu().regs.pc;
        let marker = m.read_mem(0x8000);
        if let Some(b) = m.beta_mut() {
            eprintln!(
                "final PC={pc:#06x} ring={:02x?} track={} sys={:#04x} sectors={} mem8000={marker:#04x}",
                b.command_ring(),
                b.track,
                b.system,
                b.sector_read_count,
            );
        }
    }

    /// ROM-gated: stock `3D94h` (`RST #20` / inline `0010h`) returns via the `5CC2h`
    /// hook without RET-patching the ROM (#266).
    #[test]
    fn trdos_3d94_rst20_returns_without_rom_ret_patch_when_fixture_present() {
        let Some(main) = rom_pentagon().or_else(rom128) else {
            eprintln!("skip: pentagon/128 main ROM missing");
            return;
        };
        let Some(trdos) = trdos_rom_bytes() else {
            eprintln!("skip: roms/trdos.rom missing (optional #140 TR-DOS boot fixture)");
            return;
        };
        assert_eq!(
            trdos.get(0x3d94).copied(),
            Some(0xe7),
            "fixture ROM must keep stock RST #20 at 3D94h"
        );
        let mut m = Machine::new_pentagon128(&main, &trdos).unwrap();
        if let Some(beta) = m.beta_mut() {
            beta.page_trdos(true);
        }
        install_trdos_rst20_5cc2_hook(&mut m);
        assert!(
            m.beta_mut().is_some_and(|b| b.paged),
            "TR-DOS must be paged before 3D94h"
        );
        assert_eq!(
            m.read_mem(0x3d94),
            0xe7,
            "paged fetch at 3D94h should be RST #20"
        );
        let at20 = m.read_mem(0x0020);
        assert_eq!(
            at20, 0xc3,
            "paged fetch at 0020h should be TR-DOS JP 2F72h, got {at20:#04x}"
        );
        // RAM trampoline: `CALL 3D94h` / `HALT` — return addr sits below RST #20 traffic.
        const STUB: u16 = 0x8000;
        m.write_mem(STUB, 0xcd);
        m.write_mem(STUB + 1, 0x94);
        m.write_mem(STUB + 2, 0x3d);
        m.write_mem(STUB + 3, 0x76); // HALT
        m.cpu_mut().regs.sp = 0x6000;
        m.cpu_mut().regs.pc = STUB;
        let mut returned = false;
        for _ in 0..50_000 {
            m.step_once();
            if m.cpu().regs.pc == STUB + 3 {
                returned = true;
                break;
            }
        }
        let final_pc = m.cpu().regs.pc;
        let final_paged = m.beta_mut().is_some_and(|b| b.paged);
        assert!(
            returned,
            "3D94h RST #20 should return with 5CC2h hook (PC={final_pc:#06x}, paged={final_paged})"
        );
    }

    /// ROM-gated: find-boot catalog opcodes stay stock (ABI fixup, not ROM RET/NOP).
    #[test]
    fn trdos_find_boot_rom_unpatched_when_fixture_present() {
        let Some(main) = rom_pentagon().or_else(rom128) else {
            eprintln!("skip: pentagon/128 main ROM missing");
            return;
        };
        let Some(trdos) = trdos_rom_bytes() else {
            eprintln!("skip: roms/trdos.rom missing (optional #140 TR-DOS boot fixture)");
            return;
        };
        assert_eq!(
            trdos.get(0x1968).copied(),
            Some(0x0e),
            "stock LD C,0 at 1968h"
        );
        assert_eq!(
            trdos.get(0x1977).copied(),
            Some(0xed),
            "stock LD DE,(nn) at 1977h"
        );
        assert_eq!(
            trdos.get(0x1988).copied(),
            Some(0x2a),
            "stock LD HL,(nn) at 1988h"
        );
        assert_eq!(
            trdos.get(0x199a).copied(),
            Some(0x10),
            "stock DJNZ at 199Ah"
        );
        let mut m = Machine::new_pentagon128(&main, &trdos).unwrap();
        if let Some(beta) = m.beta_mut() {
            beta.page_trdos(true);
        }
        patch_trdos_run_harness_rom(&mut m);
        assert_eq!(
            m.read_mem(0x1968),
            0x0e,
            "harness must not patch 1968h LD C,0"
        );
        assert_eq!(
            m.read_mem(0x1977),
            0xed,
            "harness must not patch 1977h LD DE,(5CD9)"
        );
        assert_eq!(
            m.read_mem(0x1988),
            0x2a,
            "harness must not patch 1988h LD HL,(5CD7)"
        );
        assert_eq!(
            m.read_mem(0x199a),
            0x10,
            "harness must not RET-patch 199Ah DJNZ"
        );
    }

    /// ROM-gated: `3DFFh` delay loop stays stock (A=1 ABI, not ROM RET).
    #[test]
    fn trdos_3dff_delay_rom_unpatched_when_fixture_present() {
        let Some(main) = rom_pentagon().or_else(rom128) else {
            eprintln!("skip: pentagon/128 main ROM missing");
            return;
        };
        let Some(trdos) = trdos_rom_bytes() else {
            eprintln!("skip: roms/trdos.rom missing (optional #140 TR-DOS boot fixture)");
            return;
        };
        assert_eq!(
            trdos.get(0x3dff).copied(),
            Some(0x0e),
            "stock LD C,#FF at 3DFFh"
        );
        let mut m = Machine::new_pentagon128(&main, &trdos).unwrap();
        if let Some(beta) = m.beta_mut() {
            beta.page_trdos(true);
        }
        patch_trdos_run_harness_rom(&mut m);
        assert_eq!(
            m.read_mem(0x3dff),
            0x0e,
            "harness must not RET-patch 3DFFh delay"
        );
    }

    /// ROM-gated: CAT / VG93-wait / PROG-wipe sites stay stock (PC/RET ABI).
    #[test]
    fn trdos_cat_wait_rom_unpatched_when_fixture_present() {
        let Some(main) = rom_pentagon().or_else(rom128) else {
            eprintln!("skip: pentagon/128 main ROM missing");
            return;
        };
        let Some(trdos) = trdos_rom_bytes() else {
            eprintln!("skip: roms/trdos.rom missing (optional #140 TR-DOS boot fixture)");
            return;
        };
        assert_eq!(
            trdos.get(0x3d9d).copied(),
            Some(0xe7),
            "stock RST #20 at 3D9Dh"
        );
        assert_eq!(
            trdos.get(0x02d4).copied(),
            Some(0xcd),
            "stock CALL at 02D4h"
        );
        assert_eq!(
            trdos.get(0x213e).copied(),
            Some(0xcc),
            "stock CALL Z at 213Eh"
        );
        assert_eq!(trdos.get(0x2155).copied(), Some(0xc3), "stock JP at 2155h");
        let mut m = Machine::new_pentagon128(&main, &trdos).unwrap();
        if let Some(beta) = m.beta_mut() {
            beta.page_trdos(true);
        }
        patch_trdos_run_harness_rom(&mut m);
        assert_eq!(
            m.read_mem(0x3d9d),
            0xe7,
            "harness must not JR-patch 3D9Dh wait"
        );
        assert_eq!(
            m.read_mem(0x02d4),
            0xcd,
            "harness must not NOP 02D4h CAT CALL"
        );
        assert_eq!(
            m.read_mem(0x213e),
            0xcc,
            "harness must not NOP 213Eh CALL Z"
        );
        assert_eq!(
            m.read_mem(0x2155),
            0xc3,
            "harness must not RET-patch 2155h JP CAT"
        );
    }

    /// ROM-gated: `19ECh` stays stock `RST #20`/`08D2h`; padding is never patched.
    #[test]
    fn trdos_19ec_08d2_callsite_rom_unpatched_when_fixture_present() {
        let Some(main) = rom_pentagon().or_else(rom128) else {
            eprintln!("skip: pentagon/128 main ROM missing");
            return;
        };
        let Some(trdos) = trdos_rom_bytes_harness() else {
            eprintln!("skip: roms/trdos.rom missing (optional #140 TR-DOS boot fixture)");
            return;
        };
        assert_eq!(
            trdos.get(0x19ec).copied(),
            Some(0xe7),
            "stock RST #20 at 19ECh"
        );
        assert_eq!(
            trdos.get(0x19ed).copied(),
            Some(0xd2),
            "stock inline service lo at 19EDh"
        );
        assert_eq!(
            trdos.get(0x19ee).copied(),
            Some(0x08),
            "stock inline service hi → 08D2h"
        );
        let hole = !trdos_rom_fills_0800_hole(&trdos);
        if hole {
            assert_eq!(
                trdos.get(0x08d2).copied(),
                Some(0xff),
                "hole dump has FF padding at 08D2h"
            );
        }
        let mut m = Machine::new_pentagon128(&main, &trdos).unwrap();
        if let Some(beta) = m.beta_mut() {
            beta.page_trdos(true);
        }
        patch_trdos_run_harness_rom(&mut m);
        assert_eq!(
            m.read_mem(0x19ec),
            0xe7,
            "harness must not patch 19ECh RST #20"
        );
        assert_eq!(
            [m.read_mem(0x19ed), m.read_mem(0x19ee)],
            [0xd2, 0x08],
            "harness must not retarget 19ECh service word"
        );
        if hole {
            assert_eq!(
                m.read_mem(0x08d2),
                0xff,
                "harness must not write into 08D2h FF padding"
            );
        } else {
            assert_ne!(
                m.read_mem(0x08d2),
                0xff,
                "complete dump: harness must leave 08D2h service code intact"
            );
        }
    }

    /// ROM-gated: when a filled-hole dump is present, classify `08D2h`.
    ///
    /// Soft-pass on the usual hole-filled 5.04 image (documents the blocker).
    /// Alone Coder **5.04T** fills `0800h`+ with VG93 helpers — `08D2h` is a port
    /// stub, not classic file-load — so `19ECh` still uses the FDC stand-in.
    #[test]
    fn trdos_native_file_services_gate_when_fixture_present() {
        let Some(trdos) = trdos_rom_bytes() else {
            eprintln!("skip: roms/trdos.rom missing (optional #140 TR-DOS boot fixture)");
            return;
        };
        if !trdos_rom_fills_0800_hole(&trdos) {
            assert_eq!(
                trdos.get(0x08d2).copied(),
                Some(0xff),
                "hole dump: 08D2h stays FF"
            );
            assert_eq!(
                trdos.get(0x0d6b).copied(),
                Some(0xff),
                "hole dump: 0D6Bh stays FF"
            );
            eprintln!(
                "trdos native file-services gate: hole dump (0800h–0E71h FF) — \
                 place trdos-5.04t.rom under roms/pentagon/ (Refs #140)"
            );
            return;
        }
        if trdos_rom_08d2_is_vg93_port_stub(&trdos) {
            assert!(!trdos_rom_has_native_file_services(&trdos));
            eprintln!(
                "trdos native file-services gate: 5.04T VG93 port stub at 08D2h — \
                 RUN boot uses 19ECh FDC stand-in after catalog match (Refs #140)"
            );
            return;
        }
        assert!(trdos_rom_has_native_file_services(&trdos));
        assert_eq!(
            [trdos.get(0x19ec), trdos.get(0x19ed), trdos.get(0x19ee)],
            [Some(&0xe7), Some(&0xd2), Some(&0x08)],
            "classic complete dump should keep stock 19ECh → 08D2h"
        );
        eprintln!(
            "trdos native file-services gate: OPEN — classic file-load at 08D2h \
             (native RUN path eligible; Refs #140)"
        );
    }

    /// ROM-gated: native `012Ah` / `1D97h` stay stock; `0D6Bh` hole is not patched.
    #[test]
    fn trdos_012a_0d6b_service_rom_unpatched_when_fixture_present() {
        let Some(main) = rom_pentagon().or_else(rom128) else {
            eprintln!("skip: pentagon/128 main ROM missing");
            return;
        };
        let Some(trdos) = trdos_rom_bytes_harness() else {
            eprintln!("skip: roms/trdos.rom missing (optional #140 TR-DOS boot fixture)");
            return;
        };
        assert_eq!(
            trdos.get(0x012a).copied(),
            Some(0xcd),
            "stock CALL at 012Ah"
        );
        assert_eq!(
            [trdos.get(0x012b).copied(), trdos.get(0x012c).copied()],
            [Some(0xe5), Some(0x20)],
            "stock CALL 20E5h at 012Ah"
        );
        assert_eq!(
            [
                trdos.get(0x012d).copied(),
                trdos.get(0x012e).copied(),
                trdos.get(0x012f).copied()
            ],
            [Some(0xcd), Some(0x97), Some(0x1d)],
            "stock CALL 1D97h after 20E5h"
        );
        assert_eq!(
            [
                trdos.get(0x1d97).copied(),
                trdos.get(0x1d98).copied(),
                trdos.get(0x1d99).copied()
            ],
            [Some(0xe7), Some(0x6b), Some(0x0d)],
            "stock RST #20 / 0D6Bh at 1D97h"
        );
        assert_eq!(
            trdos.get(0x1d9a).copied(),
            Some(0xc9),
            "stock RET after 1D97h service word"
        );
        let hole = !trdos_rom_fills_0800_hole(&trdos);
        if hole {
            assert_eq!(
                trdos.get(0x0d6b).copied(),
                Some(0xff),
                "hole dump has FF padding at 0D6Bh (0800h–0E71h)"
            );
        } else {
            assert_ne!(
                trdos.get(0x0d6b).copied(),
                Some(0xff),
                "complete dump has code at 0D6Bh"
            );
        }
        assert_eq!(
            trdos.get(0x16b0).copied(),
            Some(0x16),
            "16B0h is the high byte of CALL 166Fh, not a RST #20 body"
        );
        let mut m = Machine::new_pentagon128(&main, &trdos).unwrap();
        if let Some(beta) = m.beta_mut() {
            beta.page_trdos(true);
        }
        patch_trdos_run_harness_rom(&mut m);
        assert_eq!(
            [m.read_mem(0x012a), m.read_mem(0x012d), m.read_mem(0x1d97)],
            [0xcd, 0xcd, 0xe7],
            "harness must not patch 012Ah / 1D97h"
        );
        if hole {
            assert_eq!(
                m.read_mem(0x0d6b),
                0xff,
                "harness must not write into 0D6Bh FF padding"
            );
        } else {
            assert_ne!(
                m.read_mem(0x0d6b),
                0xff,
                "complete dump: harness must leave 0D6Bh service code intact"
            );
        }
    }

    /// ROM-gated: TR-DOS `RUN` (no filename) loads synthetic `boot` → `POKE 32768,165`.
    #[test]
    fn trdos_rom_run_boot_basic_when_fixture_present() {
        let Some(main) = rom_pentagon().or_else(rom128) else {
            eprintln!("skip: pentagon/128 main ROM missing");
            return;
        };
        let Some(trdos) = trdos_rom_bytes_harness() else {
            eprintln!("skip: roms/trdos.rom missing (optional #140 TR-DOS boot fixture)");
            return;
        };
        let mut m = Machine::new_pentagon128(&main, &trdos).unwrap();
        m.insert_trd(formats::TrdImage::synthetic_trdos_boot_basic())
            .unwrap();
        enter_128k_basic_from_menu(&mut m);
        ensure_trdos_beta128_prog(&mut m);
        assert!(
            enter_trdos_command_mode(&mut m),
            "TR-DOS command prompt (3D31h) not reached (PC={:#06x})",
            m.cpu().regs.pc
        );
        let ok = invoke_trdos_run_boot(&mut m);
        let pc = m.cpu().regs.pc;
        let (sectors, track, ring) = m
            .beta_mut()
            .map(|b| (b.sector_read_count, b.track, b.command_ring().to_vec()))
            .unwrap_or((0, 0, Vec::new()));
        assert!(
            ok,
            "TR-DOS RUN boot should POKE 32768,165 (PC={pc:#06x}, sectors={sectors}, track={track}, ring={ring:02x?}, 8000={:#04x})",
            m.read_mem(0x8000)
        );
        assert_eq!(m.read_mem(0x8000), 0xa5, "boot BASIC POKE 32768,165");
    }

    /// 5.04T: `08D2h` is a VG93 port stub, so `19ECh` must still take the FDC stand-in.
    #[test]
    fn trdos_19ec_takes_fdc_standin_when_504t_port_stub_present() {
        let Some(main) = rom_pentagon().or_else(rom128) else {
            eprintln!("skip: pentagon/128 main ROM missing");
            return;
        };
        let Some(trdos) = trdos_rom_bytes_complete() else {
            eprintln!(
                "skip: complete TR-DOS dump missing — place trdos-5.04t.rom / \
                 trdos-complete.rom under roms/pentagon/ (Refs #140)"
            );
            return;
        };
        assert!(trdos_rom_fills_0800_hole(&trdos));
        if !trdos_rom_08d2_is_vg93_port_stub(&trdos) {
            eprintln!("skip: complete dump has classic 08D2h file-load (not 5.04T stub)");
            return;
        }
        assert!(!trdos_rom_has_native_file_services(&trdos));
        let mut m = Machine::new_pentagon128(&main, &trdos).unwrap();
        m.insert_trd(formats::TrdImage::synthetic_trdos_boot_basic())
            .unwrap();
        if let Some(beta) = m.beta_mut() {
            beta.page_trdos(true);
        }
        m.write_mem(0x5d25 + 8, b'B');
        m.write_mem(0x5d25 + 9, 28);
        m.write_mem(0x5d25 + 10, 0);
        m.write_mem(0x5d25 + 14, 1);
        m.write_mem(0x5d25 + 15, 1);
        m.write_mem(0x5c59, 0x00);
        m.write_mem(0x5c5a, 0x5e);
        m.cpu_mut().regs.pc = 0x19ec;
        apply_trdos_run_native_abi(&mut m);
        assert_eq!(
            m.cpu().regs.pc,
            0x1b76,
            "5.04T port-stub 08D2h: 19ECh must FDC-load and enter LINE-NEW"
        );
    }

    /// 5.04T RUN→boot: Type-II BUSY so catalog `1E3Dh` returns to `1981h`; `08D2h` is a
    /// VG93 port stub so `19ECh` uses the FDC/`LINE-NEW` stand-in (hard `0x8000==0xA5`).
    #[test]
    fn trdos_rom_run_boot_504t_catalog_match_when_complete_present() {
        let Some(main) = rom_pentagon().or_else(rom128) else {
            return;
        };
        let Some(trdos) = trdos_rom_bytes_complete() else {
            eprintln!(
                "skip: complete TR-DOS dump missing — place trdos-5.04t.rom / \
                 trdos-complete.rom under roms/pentagon/ (Refs #140)"
            );
            return;
        };
        if !trdos_rom_08d2_is_vg93_port_stub(&trdos) {
            eprintln!("skip: complete dump has classic 08D2h file-load (not 5.04T stub)");
            return;
        }
        let mut m = Machine::new_pentagon128(&main, &trdos).unwrap();
        m.insert_trd(formats::TrdImage::synthetic_trdos_boot_basic())
            .unwrap();
        enter_128k_basic_from_menu(&mut m);
        ensure_trdos_beta128_prog(&mut m);
        if let Some(beta) = m.beta_mut() {
            beta.page_trdos(true);
        }
        init_trdos_usr_call_frame(&mut m);
        assert!(
            invoke_trdos_run_boot(&mut m),
            "5.04T RUN boot should POKE 32768,165 after catalog match + 19ECh stand-in"
        );
        assert_eq!(m.read_mem(0x8000), 0xa5);
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

    /// Original Sinclair ULA snow: M1 refresh hook records overrides on 48K/128K paths.
    #[test]
    fn m1_refresh_records_snow_48k() {
        use z80::Memory;
        let mut bus = Bus48::new();
        bus.frame_t = ula::PAPER_START_48 + 3;
        bus.ram[0] = 0xAA;
        bus.ram[1] = 0x55;
        let mut mem = MemIo48 {
            bus: &mut bus,
            watch: None,
            t_step_start: 0,
            opcode_pc: None,
        };
        mem.m1_refresh(0x4001, 0, false);
        assert!(
            !bus.ula.snow_overrides().is_empty(),
            "48K-class ULA must record snow overrides"
        );
    }

    #[test]
    fn m1_refresh_skips_snow_when_m1_contended() {
        use z80::Memory;
        let mut bus = Bus48::new();
        bus.frame_t = ula::PAPER_START_48 + 3;
        bus.ram[0] = 0xAA;
        bus.ram[1] = 0x55;
        let mut mem = MemIo48 {
            bus: &mut bus,
            watch: None,
            t_step_start: 0,
            opcode_pc: None,
        };
        mem.m1_refresh(0x4001, 0, true);
        assert!(
            bus.ula.snow_overrides().is_empty(),
            "contended M1 must block snow"
        );
    }

    /// Each M1 refresh uses contention from that fetch, not a stale opcode_pc marker.
    #[test]
    fn m1_refresh_per_fetch_contention_not_stale() {
        use z80::Memory;
        let mut bus = Bus48::new();
        bus.frame_t = ula::PAPER_START_48 + 3;
        bus.ram[0] = 0xAA;
        bus.ram[1] = 0x55;
        {
            let mut mem = MemIo48 {
                bus: &mut bus,
                watch: None,
                t_step_start: 0,
                opcode_pc: Some(0x4000),
            };
            let (_, wait1) = mem.read(0x4000, 0);
            mem.m1_refresh(0x4001, 0, wait1 > 0);
        }
        assert!(
            bus.ula.snow_overrides().is_empty(),
            "contended first fetch must not snow"
        );
        {
            let mut mem = MemIo48 {
                bus: &mut bus,
                watch: None,
                t_step_start: 0,
                opcode_pc: None,
            };
            let (_, wait2) = mem.read(0x8000, 0);
            mem.m1_refresh(0x4001, 0, wait2 > 0);
        }
        assert!(
            !bus.ula.snow_overrides().is_empty(),
            "uncontended second M1 must snow even after contended first fetch"
        );
    }

    #[test]
    fn m1_refresh_records_snow_128k() {
        use z80::Memory;
        let mut bus = Bus128::new();
        bus.frame_t = ula::PAPER_START_128 + 3;
        bus.banks[5][0] = 0xAA;
        bus.banks[5][1] = 0x55;
        let mut mem = MemIo128 {
            bus: &mut bus,
            watch: None,
            t_step_start: 0,
            opcode_pc: None,
            pentagon: false,
        };
        mem.m1_refresh(0x4001, 0, false);
        assert!(
            !bus.ula.snow_overrides().is_empty(),
            "128K/grey+2 ULA must record snow overrides"
        );
    }

    /// Pentagon 128: no memory contention → no ULA snow hook effect.
    #[test]
    fn m1_refresh_pentagon128_skips_snow() {
        use z80::Memory;
        let mut bus = Bus128::new();
        bus.frame_t = ula::PAPER_START_128 + 3;
        bus.banks[5][0] = 0xAA;
        bus.banks[5][1] = 0x55;
        let mut mem = MemIo128 {
            bus: &mut bus,
            watch: None,
            t_step_start: 0,
            opcode_pc: None,
            pentagon: true,
        };
        mem.m1_refresh(0x4001, 0, false);
        assert!(
            bus.ula.snow_overrides().is_empty(),
            "Pentagon must not apply ULA snow"
        );
    }

    /// Amstrad +2A/+3: different ULA — default `m1_refresh` is a no-op.
    #[test]
    fn m1_refresh_plus3_skips_snow() {
        use z80::Memory;
        let mut bus = BusPlus3::new_with_disk(true);
        bus.frame_t = ula::PAPER_START_128 + 3;
        let mut mem = MemIoPlus3 {
            bus: &mut bus,
            watch: None,
            t_step_start: 0,
        };
        mem.m1_refresh(0x4001, 0, false);
        assert!(
            bus.ula.snow_overrides().is_empty(),
            "+3 Amstrad ULA must not apply snow"
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

    fn rom_timex_tc2048() -> Option<Vec<u8>> {
        let path = resolve_rom_path(Model::TimexTC2048)?;
        std::fs::read(path).ok()
    }

    fn rom_timex_ts2068() -> Option<(Vec<u8>, Vec<u8>)> {
        let home = resolve_rom_path(Model::TimexTS2068)?;
        let exrom = resolve_exrom_path(Model::TimexTS2068)?;
        Some((std::fs::read(home).ok()?, std::fs::read(exrom).ok()?))
    }

    #[test]
    fn timex_tc2048_boot_smoke() {
        let Some(path) = resolve_rom_path(Model::TimexTC2048) else {
            eprintln!("skip: roms/timex/tc2048.rom missing");
            return;
        };
        let rom = std::fs::read(path).expect("read timex rom");
        let mut m = Machine::new_timex_tc2048(&rom).unwrap();
        assert_eq!(m.model(), Model::TimexTC2048);
        for _ in 0..50 {
            let _ = m.run_frame();
        }
    }

    #[test]
    fn timex_scld_ext_colour_render_uses_alt_attrs() {
        // Screen-RAM / SCLD rendering only — no real Timex ROM required.
        let mut m = Machine::new_timex_tc2048(&[0; 16 * 1024]).unwrap();
        // Paint primary bitmap solid; primary 8×8 attr blue; alt 8×1 attr red.
        if let Machine::Spec48 { bus, .. } = &mut m {
            bus.write(0x4000, 0xFF);
            bus.write(0x5800, 0x01); // blue ink — must not win in ext colour
            bus.write(0x6000, 0x02); // red 8×1 attr (scrambled line 0)
            bus.out_port(0x00FF, 0x02); // EXTCOLOUR
        } else {
            panic!("expected Spec48");
        }
        let mut out = vec![0u8; 256 * 192 * 4];
        m.render_rgba(&mut out, false);
        let red = ula::palette_rgb(2, false);
        assert_eq!(&out[0..3], &red);
    }

    #[test]
    fn timex_scld_hires_render_interleaves_files() {
        let mut m = Machine::new_timex_tc2048(&[0; 16 * 1024]).unwrap();
        if let Machine::Spec48 { bus, .. } = &mut m {
            bus.write(0x4000, 0xFF); // primary solid
            bus.write(0x6000, 0x00); // alt empty
                                     // Mode 6 + white ink / black paper (bits 3–5 = 7).
            bus.out_port(0x00FF, 0x06 | (7 << 3));
        } else {
            panic!("expected Spec48");
        }
        assert_eq!(m.framebuffer_dims(false), (512, 192));
        let mut out = vec![0u8; 512 * 192 * 4];
        m.render_rgba(&mut out, false);
        let white = ula::palette_rgb(7, true);
        let black = ula::palette_rgb(0, true);
        assert_eq!(&out[0..3], &white);
        assert_eq!(&out[8 * 4..8 * 4 + 3], &black);
    }

    #[test]
    fn timex_ts2068_boot_smoke() {
        let Some((home, exrom)) = rom_timex_ts2068() else {
            eprintln!("skip: roms/timex/tc2068-*.rom missing");
            return;
        };
        let mut m = Machine::new_timex_ts2068(&home, &exrom).unwrap();
        assert_eq!(m.model(), Model::TimexTS2068);
        for _ in 0..50 {
            let _ = m.run_frame();
        }
        // Horizontal MMU: page EX-ROM over chunk 0.
        if let Machine::Spec48 { bus, .. } = &mut m {
            bus.out_port(0x00FF, 0x80);
            bus.out_port(0x00F4, 0x01);
            assert_eq!(bus.read(0x0000), exrom[0]);
        } else {
            panic!("expected Spec48 bus for TS2068");
        }
    }

    #[test]
    fn timex_ts2068_home_dck_replaces_rom_with_spectrum() {
        let Some((home, exrom)) = rom_timex_ts2068() else {
            eprintln!("skip: roms/timex/tc2068-*.rom missing");
            return;
        };
        let Some(spec) = resolve_rom_path(Model::Spectrum48).and_then(|p| std::fs::read(p).ok())
        else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let mut rom16 = [0u8; 16384];
        rom16.copy_from_slice(&spec[..16384]);
        let dck = formats::DckImage::spectrum_rom_home(&rom16);
        let mut m = Machine::new_timex_ts2068(&home, &exrom).unwrap();
        assert_ne!(
            m.read_mem(0x0556),
            rom16[0x0556],
            "precondition: Timex LD-BYTES site ≠ Spectrum"
        );
        m.insert_timex_dock(&dck).unwrap();
        assert!(m.has_timex_dock());
        assert_eq!(m.read_mem(0x0000), rom16[0]);
        assert_eq!(m.read_mem(0x0001), rom16[1]);
        // Spectrum LD-BYTES entry lives at $0556 in the home ROM overlay.
        assert_eq!(m.read_mem(0x0556), rom16[0x0556]);
        assert_eq!(m.read_mem(0x0557), rom16[0x0557]);
        m.eject_timex_dock().unwrap();
        assert!(!m.has_timex_dock());
        assert_eq!(m.read_mem(0x0556), home[0x0556]);
    }

    #[test]
    fn timex_ts2068_redirects_spectrum_ld_bytes_call_from_ram() {
        let Some((home, exrom)) = rom_timex_ts2068() else {
            eprintln!("skip: roms/timex/tc2068-*.rom missing");
            return;
        };
        let mut m = Machine::new_timex_ts2068(&home, &exrom).unwrap();
        // Simulate `CALL $0556` from RAM (Death Chase-style Spectrum loader).
        if let Machine::Spec48 { cpu, bus, .. } = &mut m {
            bus.write(0x8000, 0xC9); // RET landing pad for stack ret
            cpu.regs.sp = 0xFFFD;
            bus.write(0xFFFD, 0x00);
            bus.write(0xFFFE, 0x80); // ret → $8000
            cpu.regs.pc = 0x0556;
        } else {
            panic!("expected Spec48 bus for TS2068");
        }
        m.step_once();
        assert_eq!(
            m.cpu().regs.pc,
            TIMEX_EXROM_LD_BYTES_PC.wrapping_add(1),
            "after one opcode at Timex LD-BYTES entry"
        );
        if let Machine::Spec48 { bus, .. } = &m {
            assert!(bus.timex_scld.use_exrom());
            assert!(bus.timex_scld.chunk_paged(0));
            assert_eq!(
                bus.read(TIMEX_EXROM_LD_BYTES_PC),
                tape::LD_BYTES_PROLOGUE[0]
            );
        }
    }

    #[test]
    fn timex_ts2068_ay_advances_on_step_apis() {
        let Some((home, exrom)) = rom_timex_ts2068() else {
            eprintln!("skip: roms/timex/tc2068-*.rom missing");
            return;
        };
        let mut m = Machine::new_timex_ts2068(&home, &exrom).unwrap();
        // Short period tone A so sample_mono goes non-zero once AY advances.
        if let Machine::Spec48 { bus, .. } = &mut m {
            bus.out_port(0x00F5, 0);
            bus.out_port(0x00F6, 1); // period fine = 1
            bus.out_port(0x00F5, 1);
            bus.out_port(0x00F6, 0); // period coarse = 0
            bus.out_port(0x00F5, 7);
            bus.out_port(0x00F6, 0x3e); // enable tone A
            bus.out_port(0x00F5, 8);
            bus.out_port(0x00F6, 0x0f); // full volume A
        } else {
            panic!("expected Spec48 bus for TS2068");
        }
        let mut saw = false;
        for _ in 0..4_000 {
            m.step_once();
            if let Machine::Spec48 { bus, .. } = &m {
                if bus.ay.sample_mono() > 0.0 {
                    saw = true;
                    break;
                }
            }
        }
        assert!(saw, "step_once must advance Timex AY");
        m.run_tstates(2_000);
        m.step_cpu_only();
        let _ = m.run_frame();
        if let Machine::Spec48 { bus, .. } = &m {
            assert!(bus.ay.sample_mono().is_finite());
        }
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
        if let Some(r) = rom_timex_tc2048() {
            cases.push((Model::TimexTC2048, r, "timex"));
        }
        if let Some((home, _)) = rom_timex_ts2068() {
            cases.push((Model::TimexTS2068, home, "ts2068"));
        }
        if cases.is_empty() {
            eprintln!("skip: no ROMs for attr_mark matrix");
            return;
        }

        let mut report = String::from("attr_mark matrix:\n");
        let mut failed = Vec::new();
        for (model, rom, label) in &cases {
            let warmup = if matches!(
                model,
                Model::Spectrum48 | Model::TimexTC2048 | Model::TimexTS2068
            ) {
                200
            } else {
                250
            };
            // Instant / flash-load traps Spectrum LD-BYTES PCs; Timex BASIC differs.
            if matches!(model, Model::TimexTS2068) {
                report.push_str(&format!(
                    "  {label} instant: SKIP (Timex ROM — no Spectrum flash trap)\n"
                ));
            } else {
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
                        Model::TimexTC2048 => Machine::new_timex_tc2048(rom).unwrap(),
                        Model::TimexTS2068 => {
                            let ex = read_exrom(Model::TimexTS2068).expect("ts2068 exrom");
                            Machine::new_timex_ts2068(rom, &ex).unwrap()
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
            }
            let full = std::env::var_os("SPEC_CHUM_FULL_TAPE_MATRIX").is_some();
            for speed in speeds {
                // EAR@1 is slow (~minutes of Spectrum time); default CI keeps it
                // on 48K only. Set SPEC_CHUM_FULL_TAPE_MATRIX=1 for 128K/+3 @1×.
                if speed == 1
                    && !matches!(
                        model,
                        Model::Spectrum48 | Model::TimexTC2048 | Model::TimexTS2068
                    )
                    && !full
                {
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
                        Model::TimexTC2048 => Machine::new_timex_tc2048(rom).unwrap(),
                        Model::TimexTS2068 => {
                            let ex = read_exrom(Model::TimexTS2068).expect("ts2068 exrom");
                            Machine::new_timex_ts2068(rom, &ex).unwrap()
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
        if let Some(r) = rom_timex_tc2048() {
            cases.push((Model::TimexTC2048, r, "timex"));
        }
        // Timex TS2068: Timex BASIC is not Spectrum-compatible for this custom-loader
        // fixture (attr_mark EAR covers TS2068 tape via the shared match arms).
        if cases.is_empty() {
            eprintln!("skip: no ROMs for custom_loader matrix");
            return;
        }

        let mut report = String::from("custom_loader matrix:\n");
        let mut failed = Vec::new();
        for (model, rom, label) in &cases {
            let warmup = if matches!(
                model,
                Model::Spectrum48 | Model::TimexTC2048 | Model::TimexTS2068
            ) {
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
            if matches!(
                model,
                Model::Spectrum48 | Model::TimexTC2048 | Model::TimexTS2068
            ) || full
            {
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
                    Model::TimexTC2048 => Machine::new_timex_tc2048(rom).unwrap(),
                    Model::TimexTS2068 => {
                        let ex = read_exrom(Model::TimexTS2068).expect("ts2068 exrom");
                        Machine::new_timex_ts2068(rom, &ex).unwrap()
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
                    Model::TimexTC2048 => Machine::new_timex_tc2048(&rom).unwrap(),
                    Model::TimexTS2068 => {
                        let ex = read_exrom(Model::TimexTS2068).expect("ts2068 exrom");
                        Machine::new_timex_ts2068(&rom, &ex).unwrap()
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
