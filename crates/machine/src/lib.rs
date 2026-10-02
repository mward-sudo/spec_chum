//! Spec Chum machine — Spectrum models and frame runner.

#![allow(clippy::large_enum_variant)] // SpectrumModel enum variants differ widely in size by design.

#[cfg(all(test, feature = "slow-tests"))]
mod z80test;

#[cfg(all(test, feature = "system-tests"))]
mod system_tests;

mod construction;
mod debugger;
mod inspect;
mod joystick;
mod memory;
mod next;
mod rom;
mod tape_state;

pub use debugger::{BreakReason, Debugger, PcBreakpointHit, Watch};
pub use inspect::{BetaInspect, Inspect, Paging, TapeInspect};
pub use joystick::{apply_joystick, clear_joystick_matrix, JoystickMode, JoystickState};
pub use next::NextMachine;
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
    unavailable_reason, writable_install_root, RomReadError, RomSlotDescriptor, RomSlotKind,
    RomSlotState, RomSlotStatus, ALL_MODELS, TRDOS_ROM_INSTALL_PATH,
};
pub use tape_state::{TapeDeck, TapeLoadOptions, TapeProgress, INSTANT_EAR_FALLBACK_SPEED};

use std::cell::Cell;

pub use bus::StereoMode as AyStereoMode;
use bus::{Bus128, Bus48, BusPlus3, Kempston, KempstonMouse};
use formats::{apply_input_byte, DskImage, RzxRecording, Snapshot128, Snapshot48};
use memory::{mem_port_watch, MemIo128, MemIo48, MemIoPlus3};
pub use tape::LD_BYTES_TRAP_PC;
pub use tape::TIMEX_EXROM_LD_BYTES_PC;
use tape::{
    evaluate_ld_bytes_trap, flash_load_block, is_ld_bytes_trap_pc, ExperiencePendingFlash,
    TapPlayer, TapeTrapResult, TzxPlayer, EXPERIENCE_COSMETIC_STRIPES_PER_FRAME,
    EXPERIENCE_PILOT_DATA_PULSES, EXPERIENCE_PILOT_HEADER_PULSES, LD_BYTES_PROLOGUE,
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

/// Errors attaching Multiface 1 / 128 or loading its ROM.
#[derive(Debug, Error)]
pub enum MultifaceError {
    #[error(
        "Multiface is not supported on this model (Multiface 1: 48K-class; Multiface 128: 128K/+2; +2A/+3 unsupported)"
    )]
    UnsupportedModel,
    #[error(transparent)]
    Rom(#[from] bus::RomLoadError),
}

/// Errors attaching `DivMMC` or loading its EEPROM / SD images.
#[derive(Debug, Error)]
pub enum DivMmcError {
    #[error("DivMMC is not supported on Spectrum +2A/+3")]
    UnsupportedModel,
    /// Only slots `0` and `1` exist on the `DivMMC` CPLD (port `0xE7` CS bits).
    #[error("DivMMC SD slot {slot} is invalid (only 0 and 1)")]
    InvalidSdSlot { slot: u8 },
    #[error(transparent)]
    Rom(#[from] bus::RomLoadError),
}

/// Errors attaching Beta Disk / TR-DOS or loading its ROM.
#[derive(Debug, Error)]
pub enum BetaDiskError {
    #[error("Beta Disk is not supported on Spectrum +2A/+3")]
    UnsupportedModel,
    #[error(transparent)]
    Rom(#[from] bus::RomLoadError),
}

/// Errors inserting a +3 DSK image.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum InsertDiskError {
    #[error("+2A has no disk interface — use Spectrum +3 for DSK")]
    Plus2ANoDiskInterface,
    #[error("+3 disk requires SpectrumPlus3 model")]
    RequiresPlus3,
}

/// Errors constructing a [`Machine`] (ROM size / content).
#[derive(Debug, Error)]
pub enum MachineBuildError {
    #[error(transparent)]
    Rom(#[from] bus::RomLoadError),
    #[error(transparent)]
    RomRead(#[from] RomReadError),
    #[error("{0}")]
    Message(String),
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
use z80::{flag, Cpu};

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
    /// Amstrad +3 (same gate array with `µPD765` — menu Loader is +3DOS disk).
    SpectrumPlus3,
    /// Garry Lancaster +3e enhanced firmware on +3 hardware (#194).
    SpectrumPlus3e,
    /// Pentagon 128 clone (#188 Phase B / #193): user ROM + TR-DOS, distinct timing.
    Pentagon128,
    /// Scorpion ZS-256 clone (#193 Phase B2): 256K + `#1FFD`, user ROM + TR-DOS.
    ScorpionZs256,
    /// Timex TC2048 (#192 Phase 1): 48K-class + SCLD ports, distributable ROM.
    TimexTC2048,
    /// Timex TS2068 / TC2068 (#192 Phase 2a): home + EX-ROM, horizontal MMU, AY.
    TimexTS2068,
}

impl Model {
    /// +2A, +3, or +3e (shared Amstrad gate array).
    #[must_use]
    pub fn is_amstrad_plus(self) -> bool {
        matches!(
            self,
            Self::SpectrumPlus2A | Self::SpectrumPlus3 | Self::SpectrumPlus3e
        )
    }

    /// +3 or +3e (disk interface present).
    #[must_use]
    pub fn has_plus3_disk(self) -> bool {
        matches!(self, Self::SpectrumPlus3 | Self::SpectrumPlus3e)
    }

    /// 128K-class bus (Sinclair 128 / grey +2 / Pentagon / Scorpion banking).
    #[must_use]
    pub fn is_128k_class(self) -> bool {
        matches!(
            self,
            Self::Spectrum128 | Self::SpectrumPlus2 | Self::Pentagon128 | Self::ScorpionZs256
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
        /// Scorpion ZS-256: 256K + #1FFD; Pentagon frame timing (#193).
        scorpion: bool,
    },
    SpecPlus3 {
        cpu: Cpu,
        bus: Box<BusPlus3>,
        ula: Ula48,
        tape: Option<TapeDeck>,
        tape_opts: TapeLoadOptions,
        rzx: Option<RzxPlayer>,
        debugger: Debugger,
        /// +3e enhanced ROM set on otherwise identical +3 hardware (#194).
        plus3e: bool,
    },
}

#[derive(Clone, Debug, Default)]
pub struct FrameAudio {
    /// Beeper edges: (`frame_t`, level).
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

fn apply_rzx_input(byte: u8, keyboard_rows: &mut [u8; 8], kempston: &mut Kempston) {
    apply_input_byte(byte, keyboard_rows, |value| {
        for bit in 0..5 {
            kempston.set_bit(bit, value & (1 << bit) != 0);
        }
    });
}

impl Machine {
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
            Self::Spec128 { scorpion: true, .. } => Model::ScorpionZs256,
            Self::Spec128 { pentagon: true, .. } => Model::Pentagon128,
            Self::Spec128 {
                plus2_rom: true, ..
            } => Model::SpectrumPlus2,
            Self::Spec128 { .. } => Model::Spectrum128,
            Self::SpecPlus3 {
                bus, plus3e: true, ..
            } if bus.disk_interface => Model::SpectrumPlus3e,
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
        self.debugger_mut().reset_stop_state();
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
                bus.last_7ffd = 0;
                bus.beeper_edges.clear();
                bus.ay.reset();
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
                });
            }
        }
    }

    pub fn insert_disk(&mut self, image: DskImage) -> Result<(), InsertDiskError> {
        match self {
            Self::SpecPlus3 { bus, .. } if bus.disk_interface => {
                bus.fdc.insert(image);
                Ok(())
            }
            Self::SpecPlus3 { .. } => Err(InsertDiskError::Plus2ANoDiskInterface),
            _ => Err(InsertDiskError::RequiresPlus3),
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
                    apply_rzx_input(byte, &mut bus.keyboard.rows, &mut bus.kempston);
                }
                Self::Spec128 { bus, .. } => {
                    apply_rzx_input(byte, &mut bus.keyboard.rows, &mut bus.kempston);
                }
                Self::SpecPlus3 { bus, .. } => {
                    apply_rzx_input(byte, &mut bus.keyboard.rows, &mut bus.kempston);
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

    /// Attach Multiface with an 8 KiB ROM image.
    ///
    /// - **48K-class** → Multiface 1
    /// - **128K / grey +2** (and Pentagon on the same bus) → Multiface 128
    /// - **+2A / +3** → [`MultifaceError::UnsupportedModel`] (hardware incompatible)
    pub fn attach_multiface(&mut self, rom: &[u8]) -> Result<(), MultifaceError> {
        match self {
            Self::Spec48 { bus, .. } => Ok(bus.attach_multiface(rom)?),
            Self::Spec128 { bus, .. } => Ok(bus.attach_multiface(rom)?),
            Self::SpecPlus3 { .. } => Err(MultifaceError::UnsupportedModel),
        }
    }

    /// Press Multiface red button (if attached) and raise NMI.
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
            Self::Spec128 {
                cpu, bus, pentagon, ..
            } => {
                let mf = bus.multiface.as_mut()?;
                if !mf.press_button() {
                    return Some(0);
                }
                let t_step_start = cpu.t;
                let frame_len = bus.frame_tstates.max(1);
                let dt = {
                    let mut mio = MemIo128 {
                        bus: bus.as_mut(),
                        watch: None,
                        t_step_start,
                        opcode_pc: None,
                        pentagon: *pentagon,
                    };
                    cpu.nmi(&mut mio)
                };
                let _ = advance_frame_t(&mut bus.frame_t, dt, frame_len);
                if let Some(mf) = bus.multiface.as_mut() {
                    mf.page_on_nmi_vector();
                }
                Some(dt)
            }
            Self::SpecPlus3 { .. } => None,
        }
    }

    /// Attach `DivMMC` on 48K/128K (creates the peripheral if absent).
    pub fn attach_divmmc(&mut self) -> Result<&mut bus::DivMmc, DivMmcError> {
        match self {
            Self::Spec48 { bus, .. } => Ok(bus.attach_divmmc()),
            Self::Spec128 { bus, .. } => Ok(bus.attach_divmmc()),
            Self::SpecPlus3 { .. } => Err(DivMmcError::UnsupportedModel),
        }
    }

    /// Attach `DivMMC` and load an ESXDOS EEPROM image (8 KiB, or larger prefix).
    pub fn attach_divmmc_eeprom(&mut self, data: &[u8]) -> Result<(), DivMmcError> {
        let div = self.attach_divmmc()?;
        Ok(div.attach_eeprom(data)?)
    }

    /// Attach `DivMMC` (if needed) and load a flat SD/MMC sector image into slot 0.
    pub fn attach_divmmc_sd(&mut self, data: Vec<u8>) -> Result<(), DivMmcError> {
        self.attach_divmmc_sd_slot(0, data)
    }

    /// Attach `DivMMC` (if needed) and load a flat SD/MMC image into slot `0` or `1`.
    pub fn attach_divmmc_sd_slot(&mut self, slot: u8, data: Vec<u8>) -> Result<(), DivMmcError> {
        let div = self.attach_divmmc()?;
        div.attach_sd_slot(slot, data).map_err(|e| match e {
            bus::SdSlotError::InvalidSlot { slot, .. } => DivMmcError::InvalidSdSlot { slot },
        })
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
    pub fn attach_beta(&mut self) -> Result<&mut bus::BetaDisk, BetaDiskError> {
        match self {
            Self::Spec48 { bus, .. } => Ok(bus.attach_beta()),
            Self::Spec128 { bus, .. } => Ok(bus.attach_beta()),
            Self::SpecPlus3 { .. } => Err(BetaDiskError::UnsupportedModel),
        }
    }

    /// Insert a `.trd` image (attaches Beta if needed). 48K/128K only.
    pub fn insert_trd(&mut self, image: formats::TrdImage) -> Result<(), BetaDiskError> {
        self.attach_beta()?.insert(image);
        Ok(())
    }

    /// Load a 16 KiB TR-DOS ROM onto Beta (attaches the interface if needed).
    pub fn load_trdos_rom(&mut self, data: &[u8]) -> Result<(), BetaDiskError> {
        Ok(self.attach_beta()?.load_rom(data)?)
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
            Self::Spec128 { bus, .. } => bus.multiface.is_some(),
            Self::SpecPlus3 { .. } => false,
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

    fn ear_play_frame_reps(&self) -> u32 {
        let opts = self.tape_load_options();
        // Instant on a flashable deck pokes bytes at the LD-BYTES trap, so one
        // frame per call keeps traps snappy. A pulse-only deck has no trap to
        // hit: Instant then loads off EAR and must still be fast, so it uses
        // the maximum turbo rather than the Play speed control (#390).
        let speed = if opts.flash_load {
            if self.tape_supports_flash_load() {
                return 1;
            }
            INSTANT_EAR_FALLBACK_SPEED
        } else if opts.experience_load {
            // Hybrid flash + cosmetic stripes are paced one Spectrum frame per
            // host tick so abbreviated pilots take wall-clock time (#167).
            if self.tape_supports_flash_load() {
                return 1;
            }
            opts.speed.clamp(1, 64)
        } else {
            opts.speed.clamp(1, 64)
        };
        if speed <= 1 {
            return 1;
        }
        // Turbo while EAR is actively playing a non-finished deck (#178).
        if self.tape_playing() && !self.tape_finished() {
            return speed;
        }
        1
    }

    /// Spectrum frames actually run per [`Self::run_frame`] right now.
    ///
    /// Hosts must report this (not [`TapeLoadOptions::speed`]) so chrome can
    /// never claim 1× while the machine runs faster, or advertise turbo that no
    /// longer applies (#390).
    #[must_use]
    pub fn effective_speed_multiplier(&self) -> u32 {
        self.ear_play_frame_reps()
    }

    /// True when the inserted deck can serve LD-BYTES flash-load traps.
    ///
    /// Hosts use this so Instant chrome can say what will really happen: TAP
    /// decks (including standard-speed TZX converted on insert) flash; pulse
    /// TZX decks fall back to EAR at [`INSTANT_EAR_FALLBACK_SPEED`] (#390).
    #[must_use]
    pub fn tape_supports_flash_load(&self) -> bool {
        match self {
            Self::Spec48 { tape, .. }
            | Self::Spec128 { tape, .. }
            | Self::SpecPlus3 { tape, .. } => {
                tape.as_ref().is_some_and(TapeDeck::supports_flash_load)
            }
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

    /// Run one or more Spectrum frames. While EAR turbo applies, runs
    /// [`TapeLoadOptions::speed`] frames so wall-clock ≈ realtime / speed.
    /// Only the last inner frame's PCM/edges are returned (hosts should not try
    /// to play S seconds of audio in one tick).
    pub fn run_frame(&mut self) -> FrameAudio {
        let reps = self.ear_play_frame_reps();
        let mut audio = self.run_one_frame();
        for _ in 1..reps {
            if self.debugger().paused {
                break;
            }
            // Re-check each inner frame so pausing or the deck finishing
            // drops to 1× mid-burst (#178).
            if self.ear_play_frame_reps() <= 1 {
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
                if let Some(pending) = Self::tick_experience_cosmetic_paint(
                    tape,
                    &mut bus.border,
                    &mut bus.ula.border_events,
                    &mut bus.beeper_edges,
                    FRAME_TSTATES_48,
                ) {
                    Self::complete_experience_pending_flash(cpu, tape, &pending, |a, v| {
                        bus.write(a, v);
                    });
                }
                bus.ula.border = bus.border;
                ula.border = bus.border;
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
                    if Self::hold_ld_bytes_until_play(cpu.regs.pc, tape.as_ref(), |a| bus.read(a)) {
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
                    if tape_opts.ld_bytes_flash_trap()
                        && Self::try_flash_load_48(cpu, bus, tape, tape_opts.experience_load)
                    {
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
                                tape_opts.skip_tap_ear_advance(),
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
                        tape_opts.skip_tap_ear_advance(),
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
                let is_pentagon = *pentagon || bus.scorpion;
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
                if let Some(pending) = Self::tick_experience_cosmetic_paint(
                    tape,
                    &mut bus.border,
                    &mut bus.ula.border_events,
                    &mut bus.beeper_edges,
                    frame_len,
                ) {
                    Self::complete_experience_pending_flash(cpu, tape, &pending, |a, v| {
                        bus.write(a, v);
                    });
                }
                bus.ula.border = bus.border;
                ula.border = bus.border;
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
                    if Self::hold_ld_bytes_until_play(cpu.regs.pc, tape.as_ref(), |a| bus.read(a)) {
                        const HOLD_T: u32 = 4;
                        cpu.t = cpu.t.wrapping_add(u64::from(HOLD_T));
                        last_t = cpu.t;
                        ay_t = ay_t.saturating_add(HOLD_T);
                        frame_done = advance_frame_t(&mut bus.frame_t, HOLD_T, frame_len);
                        continue;
                    }
                    if tape_opts.ld_bytes_flash_trap()
                        && Self::try_flash_load_128(cpu, bus, tape, tape_opts.experience_load)
                    {
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
                                tape_opts.skip_tap_ear_advance(),
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
                        tape_opts.skip_tap_ear_advance(),
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
                if let Some(pending) = Self::tick_experience_cosmetic_paint(
                    tape,
                    &mut bus.border,
                    &mut bus.ula.border_events,
                    &mut bus.beeper_edges,
                    FRAME_TSTATES_128,
                ) {
                    Self::complete_experience_pending_flash(cpu, tape, &pending, |a, v| {
                        bus.write(a, v);
                    });
                    if !bus.disk_interface {
                        if let Some(TapeDeck::Tap(player)) = tape.as_ref() {
                            Self::plus2a_repair_menu_loader_stack_if_needed(bus, cpu, player);
                        }
                    }
                }
                bus.ula.border = bus.border;
                ula.border = bus.border;
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
                    if Self::hold_ld_bytes_until_play(cpu.regs.pc, tape.as_ref(), |a| bus.read(a)) {
                        const HOLD_T: u32 = 4;
                        cpu.t = cpu.t.wrapping_add(u64::from(HOLD_T));
                        last_t = cpu.t;
                        ay_t = ay_t.saturating_add(HOLD_T);
                        frame_done = advance_frame_t(&mut bus.frame_t, HOLD_T, FRAME_TSTATES_128);
                        continue;
                    }
                    if tape_opts.ld_bytes_flash_trap()
                        && Self::try_flash_load_plus3(cpu, bus, tape, tape_opts.experience_load)
                    {
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
                                tape_opts.skip_tap_ear_advance(),
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
                        tape_opts.skip_tap_ear_advance(),
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
    /// Instant flash-load and hybrid Experience never play TAP pulses: the
    /// LD-BYTES trap consumes blocks (Experience paints cosmetic pilots instead).
    /// Advancing EAR while BASIC/`USR` runs would skip later TAP blocks (The Boggit
    /// flag `0xC8`). Pure TZX pulse decks have no flash trap, so they still advance.
    fn advance_tape_ear(
        tape: &mut Option<TapeDeck>,
        ear: &mut bool,
        beeper: bool,
        edges: &mut Vec<(u32, bool)>,
        frame_t: u32,
        dt: u32,
        _speed: u32,
        skip_tap_ear: bool,
    ) {
        if dt == 0 {
            return;
        }
        if skip_tap_ear && matches!(tape.as_ref(), Some(TapeDeck::Tap(_))) {
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

    /// Tape inserted but paused at LD-BYTES, or hybrid Experience cosmetic hold:
    /// freeze PC so Play / stripe painting can finish before flash RET.
    #[must_use]
    fn hold_ld_bytes_until_play(
        pc: u16,
        tape: Option<&TapeDeck>,
        read: impl Fn(u16) -> u8,
    ) -> bool {
        let at_trap = is_ld_bytes_trap_pc(pc, read);
        let paused = tape.is_some_and(|t| !t.playing()) && at_trap;
        let cosmetic = at_trap
            && tape.is_some_and(|t| t.as_tap().is_some_and(|p| p.experience_pending.is_some()));
        let holding = paused || cosmetic;
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

    /// Paint one Spectrum frame of hybrid Experience cosmetic pilots (#167).
    ///
    /// When the abbreviated stripe budget is exhausted, returns the completed
    /// pending flash for the caller to poke/RET (avoids borrowing the bus for
    /// writes while border/beeper buffers are also borrowed).
    fn tick_experience_cosmetic_paint(
        tape: &mut Option<TapeDeck>,
        border: &mut u8,
        border_events: &mut Vec<(u32, u8)>,
        beeper_edges: &mut Vec<(u32, bool)>,
        frame_tstates: u32,
    ) -> Option<ExperiencePendingFlash> {
        let player = tape.as_mut()?.as_tap_mut()?;
        let pending = player.experience_pending.as_mut()?;
        let stripes = EXPERIENCE_COSMETIC_STRIPES_PER_FRAME.max(1);
        let step = (frame_tstates / stripes).max(1);
        for i in 0..stripes {
            let t = i.saturating_mul(step).min(frame_tstates.saturating_sub(1));
            // Classic loader feel: red ↔ cyan border stripes + tone edges.
            let color = if pending.level { 2 } else { 5 };
            *border = color;
            border_events.push((t, color));
            beeper_edges.push((t, pending.level));
            pending.level = !pending.level;
        }
        pending.pulses_left = pending.pulses_left.saturating_sub(stripes);
        if pending.pulses_left > 0 {
            return None;
        }
        player.experience_pending.take()
    }

    fn complete_experience_pending_flash(
        cpu: &mut Cpu,
        tape: &mut Option<TapeDeck>,
        pending: &ExperiencePendingFlash,
        mut write_mem: impl FnMut(u16, u8),
    ) {
        let block_after = tape
            .as_ref()
            .and_then(TapeDeck::as_tap)
            .map_or(0, |p| p.block as u32);
        let poke = if pending.success && pending.load {
            tape.as_ref()
                .and_then(TapeDeck::as_tap)
                .and_then(|p| p.image.blocks.get(pending.poke_block).cloned())
        } else {
            None
        };
        if pending.success {
            if let Some(block) = poke.as_ref() {
                flash_load_block(&mut write_mem, block, pending.dest);
            }
            if pending.load {
                cpu.regs.set_ix(pending.dest.wrapping_add(pending.len));
            }
            Self::ret_from_tape_trap(cpu, pending.ret_lo, pending.ret_hi, true);
            if trace::enabled(trace::Category::TAPE) {
                trace::emit(trace::EventKind::FlashLoadExit {
                    success: true,
                    bytes: pending.len,
                    block_after,
                    regs: reg_snap(cpu),
                });
            }
        } else {
            Self::ret_from_tape_trap(cpu, pending.ret_lo, pending.ret_hi, false);
            if trace::enabled(trace::Category::TAPE) {
                trace::emit(trace::EventKind::FlashLoadExit {
                    success: false,
                    bytes: 0,
                    block_after,
                    regs: reg_snap(cpu),
                });
            }
        }
    }

    fn try_flash_load_48(
        cpu: &mut Cpu,
        bus: &mut Bus48,
        tape: &mut Option<TapeDeck>,
        experience: bool,
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
        // Hybrid cosmetic already scheduled — hold path advances time until tick completes.
        if player.experience_pending.is_some() {
            return false;
        }
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
                if experience {
                    let pulses = if flag_expected == 0 {
                        EXPERIENCE_PILOT_HEADER_PULSES
                    } else {
                        EXPERIENCE_PILOT_DATA_PULSES
                    };
                    player.experience_pending = Some(ExperiencePendingFlash {
                        pulses_left: pulses,
                        level: true,
                        dest,
                        len: n,
                        success: true,
                        load,
                        ret_lo,
                        ret_hi,
                        poke_block: player.block.wrapping_sub(1),
                    });
                    return true;
                }
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

    fn try_flash_load_128(
        cpu: &mut Cpu,
        bus: &mut Bus128,
        tape: &mut Option<TapeDeck>,
        experience: bool,
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
        if player.experience_pending.is_some() {
            return false;
        }
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
                if experience {
                    let pulses = if flag_expected == 0 {
                        EXPERIENCE_PILOT_HEADER_PULSES
                    } else {
                        EXPERIENCE_PILOT_DATA_PULSES
                    };
                    player.experience_pending = Some(ExperiencePendingFlash {
                        pulses_left: pulses,
                        level: true,
                        dest,
                        len: n,
                        success: true,
                        load,
                        ret_lo,
                        ret_hi,
                        poke_block: player.block.wrapping_sub(1),
                    });
                    return true;
                }
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
        experience: bool,
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
        if player.experience_pending.is_some() {
            return false;
        }
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
                if experience {
                    let pulses = if flag_expected == 0 {
                        EXPERIENCE_PILOT_HEADER_PULSES
                    } else {
                        EXPERIENCE_PILOT_DATA_PULSES
                    };
                    player.experience_pending = Some(ExperiencePendingFlash {
                        pulses_left: pulses,
                        level: true,
                        dest,
                        len: n,
                        success: true,
                        load,
                        ret_lo,
                        ret_hi,
                        poke_block: player.block.wrapping_sub(1),
                    });
                    return true;
                }
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
                // Step paths have no per-frame cosmetic paint loop — finish any
                // deferred hybrid Experience flash immediately (#167 CodeRabbit).
                if let Some(pending) = tape
                    .as_mut()
                    .and_then(TapeDeck::as_tap_mut)
                    .and_then(|p| p.experience_pending.take())
                {
                    Self::complete_experience_pending_flash(cpu, tape, &pending, |a, v| {
                        bus.write(a, v);
                    });
                    return;
                }
                Self::timex_redirect_spectrum_ld_bytes(cpu, bus);
                if Self::hold_ld_bytes_until_play(cpu.regs.pc, tape.as_ref(), |a| bus.read(a)) {
                    const HOLD_T: u32 = 4;
                    Self::advance_tape_ear(
                        tape,
                        &mut bus.ear,
                        bus.beeper,
                        &mut bus.beeper_edges,
                        bus.frame_t,
                        HOLD_T,
                        tape_opts.speed,
                        tape_opts.skip_tap_ear_advance(),
                    );
                    if bus.timex_2068 {
                        bus.ay.advance(HOLD_T);
                    }
                    bus.frame_t = (bus.frame_t + HOLD_T) % FRAME_TSTATES_48;
                    cpu.t = cpu.t.wrapping_add(u64::from(HOLD_T));
                    return;
                }
                if tape_opts.ld_bytes_flash_trap()
                    && Self::try_flash_load_48(cpu, bus, tape, tape_opts.experience_load)
                {
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
                            tape_opts.skip_tap_ear_advance(),
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
                    tape_opts.skip_tap_ear_advance(),
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
                let is_pentagon = *pentagon || bus.scorpion;
                let frame_len = if is_pentagon {
                    FRAME_TSTATES_PENTAGON
                } else {
                    FRAME_TSTATES_128
                };
                if debugger.check_pc(cpu.regs.pc) {
                    return;
                }
                if let Some(pending) = tape
                    .as_mut()
                    .and_then(TapeDeck::as_tap_mut)
                    .and_then(|p| p.experience_pending.take())
                {
                    Self::complete_experience_pending_flash(cpu, tape, &pending, |a, v| {
                        bus.write(a, v);
                    });
                    return;
                }
                if Self::hold_ld_bytes_until_play(cpu.regs.pc, tape.as_ref(), |a| bus.read(a)) {
                    const HOLD_T: u32 = 4;
                    Self::advance_tape_ear(
                        tape,
                        &mut bus.ear,
                        bus.beeper,
                        &mut bus.beeper_edges,
                        bus.frame_t,
                        HOLD_T,
                        tape_opts.speed,
                        tape_opts.skip_tap_ear_advance(),
                    );
                    bus.ay.advance(HOLD_T);
                    bus.frame_t = (bus.frame_t + HOLD_T) % frame_len;
                    cpu.t = cpu.t.wrapping_add(u64::from(HOLD_T));
                    return;
                }
                if tape_opts.ld_bytes_flash_trap()
                    && Self::try_flash_load_128(cpu, bus, tape, tape_opts.experience_load)
                {
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
                            tape_opts.skip_tap_ear_advance(),
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
                    tape_opts.skip_tap_ear_advance(),
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
                if let Some(pending) = tape
                    .as_mut()
                    .and_then(TapeDeck::as_tap_mut)
                    .and_then(|p| p.experience_pending.take())
                {
                    Self::complete_experience_pending_flash(cpu, tape, &pending, |a, v| {
                        bus.write(a, v);
                    });
                    if !bus.disk_interface {
                        if let Some(TapeDeck::Tap(player)) = tape.as_ref() {
                            Self::plus2a_repair_menu_loader_stack_if_needed(bus, cpu, player);
                        }
                    }
                    return;
                }
                if !bus.disk_interface {
                    if let Some(TapeDeck::Tap(player)) = tape.as_ref() {
                        Self::plus2a_repair_menu_loader_stack_if_needed(bus, cpu, player);
                    }
                }
                if Self::hold_ld_bytes_until_play(cpu.regs.pc, tape.as_ref(), |a| bus.read(a)) {
                    const HOLD_T: u32 = 4;
                    Self::advance_tape_ear(
                        tape,
                        &mut bus.ear,
                        bus.beeper,
                        &mut bus.beeper_edges,
                        bus.frame_t,
                        HOLD_T,
                        tape_opts.speed,
                        tape_opts.skip_tap_ear_advance(),
                    );
                    bus.ay.advance(HOLD_T);
                    bus.frame_t = (bus.frame_t + HOLD_T) % FRAME_TSTATES_128;
                    cpu.t = cpu.t.wrapping_add(u64::from(HOLD_T));
                    return;
                }
                if tape_opts.ld_bytes_flash_trap()
                    && Self::try_flash_load_plus3(cpu, bus, tape, tape_opts.experience_load)
                {
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
                            tape_opts.skip_tap_ear_advance(),
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
                    tape_opts.skip_tap_ear_advance(),
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
                    tape_opts.skip_tap_ear_advance(),
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
                let is_pentagon = *pentagon || bus.scorpion;
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
                    tape_opts.skip_tap_ear_advance(),
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
                    tape_opts.skip_tap_ear_advance(),
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
                self.type_load_quotes_48k(with_code);
            }
            Model::Spectrum128 => self.type_load_quotes_128k(with_code),
            Model::SpectrumPlus2 => self.type_load_quotes_plus2(with_code),
            Model::SpectrumPlus2A => self.type_load_quotes_plus2a(with_code),
            Model::SpectrumPlus3 | Model::SpectrumPlus3e => self.type_load_quotes_plus3(with_code),
            Model::Pentagon128 | Model::ScorpionZs256 => self.type_load_quotes_128k(with_code),
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
#[path = "tests.rs"]
mod tests;
