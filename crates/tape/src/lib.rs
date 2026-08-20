//! Spec Chum tape — TAP/TZX loading via EAR bitstream.

#![allow(clippy::pedantic)]

mod tzx;

pub use tzx::{TzxError, TzxPlayer};

use std::path::Path;

use thiserror::Error;

#[derive(Clone, Debug, Default)]
pub struct TapImage {
    pub blocks: Vec<Vec<u8>>,
    /// Pause after each block in T-states. Empty or `0` → [`PAUSE_T`] (TAP default).
    /// TZX `0x10` conversions store the real inter-block gap here.
    pub pause_t: Vec<u32>,
}

#[derive(Debug, Error)]
pub enum TapeError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Format(String),
}

impl TapImage {
    pub fn load(path: &Path) -> Result<Self, TapeError> {
        let data = std::fs::read(path)?;
        Self::parse(&data)
    }

    pub fn parse(mut data: &[u8]) -> Result<Self, TapeError> {
        let mut blocks = Vec::new();
        while !data.is_empty() {
            if data.len() < 2 {
                return Err(TapeError::Format("truncated TAP length".into()));
            }
            let len = u16::from_le_bytes([data[0], data[1]]) as usize;
            data = &data[2..];
            if data.len() < len {
                return Err(TapeError::Format("truncated TAP block".into()));
            }
            blocks.push(data[..len].to_vec());
            data = &data[len..];
        }
        Ok(Self {
            blocks,
            pause_t: Vec::new(),
        })
    }
}

/// `INC D / EX AF,AF' / DEC D / DI` at the ROM (and Boggit RAM-clone) LD-BYTES entry.
pub const LD_BYTES_PROLOGUE: [u8; 4] = [0x14, 0x08, 0x15, 0xF3];
/// Bytes from LD-BYTES entry (`0x0556`) to the first `CALL LD-EDGE-1` (`0x056C`).
pub const LD_BYTES_EDGE_CALL_OFF: u16 = 0x16;

/// True when `pc` is ROM `LD-BYTES` edge-detect or a RAM copy of the same routine.
///
/// The 128K editor ROM maps different bytes at `0x056C`; we require the `0x0556`
/// prologue so flash-load does not fire in ROM 0. Relocated loaders (The Boggit
/// at `0x8097`) match the prologue `LD_BYTES_EDGE_CALL_OFF` bytes before `pc`.
#[must_use]
pub fn is_ld_bytes_trap_pc(pc: u16, mut read: impl FnMut(u16) -> u8) -> bool {
    if pc == LD_BYTES_TRAP_PC {
        return (0..4).all(|i| read(0x0556 + i) == LD_BYTES_PROLOGUE[i as usize]);
    }
    if pc < 0x4000 || read(pc) != 0xCD {
        return false;
    }
    let start = pc.wrapping_sub(LD_BYTES_EDGE_CALL_OFF);
    if start < 0x4000 {
        return false;
    }
    (0..4).all(|i| read(start.wrapping_add(i)) == LD_BYTES_PROLOGUE[i as usize])
}

/// Spectrum ROM LD-BYTES entry used for flash-load traps.
pub const LD_BYTES_TRAP_PC: u16 = 0x056C;

/// Pilot / sync / data pulse timings (T-states), matching the 48K ROM loader.
pub const PILOT_PULSE_T: u32 = 2168;
pub const PILOT_HEADER_PULSES: u32 = 8063;
pub const PILOT_DATA_PULSES: u32 = 3223;
pub const SYNC1_T: u32 = 667;
pub const SYNC2_T: u32 = 735;
pub const BIT0_T: u32 = 855;
pub const BIT1_T: u32 = 1710;
/// Inter-block pause (~1s at 3.5 MHz).
pub const PAUSE_T: u32 = 3_500_000;
/// Fewest leader pulses still accepted by the 48K ROM LD-LEADER loop.
const MIN_PILOT_PULSES: u32 = 2_560;

/// Append one edge-to-edge pulse and toggle the EAR level.
pub(crate) fn push_pulse(pulses: &mut Vec<(u32, bool)>, level: &mut bool, duration: u32) {
    pulses.push((duration, *level));
    *level = !*level;
}

/// Pilot count for a TAP flag byte, optionally shortened for turbo (`speed` ≥ 2).
#[must_use]
pub fn pilot_pulses_for_speed(flag: u8, speed: u32) -> u32 {
    let full = if flag == 0 {
        PILOT_HEADER_PULSES
    } else {
        PILOT_DATA_PULSES
    };
    let speed = speed.max(1);
    (full / speed).max(MIN_PILOT_PULSES).min(full)
}

/// Inter-block pause T-states at the given turbo multiplier.
#[must_use]
pub fn pause_t_for_speed(speed: u32) -> u32 {
    let speed = speed.max(1);
    (PAUSE_T / speed).max(3_500)
}

/// Generates EAR levels for a TAP block (ROM timing).
#[derive(Clone, Debug)]
pub struct TapPlayer {
    pub image: TapImage,
    /// Index of the block currently playing (or next to play when idle between blocks).
    pub block: usize,
    /// When false, [`Self::advance`] does not consume pulses (deck paused).
    pub playing: bool,
    /// Pulse schedule: (duration T-states, EAR level while this pulse is active).
    pulses: Vec<(u32, bool)>,
    pulse_i: usize,
    remain: u32,
    level: bool,
    /// Turbo: fewer leader pulses and a shorter pause; bit/sync widths stay ROM-accurate.
    speed: u32,
    /// Optional per-block pause override (TZX 0x10). Empty → [`pause_t_for_speed`].
    block_pause_t: Vec<u32>,
}

impl TapPlayer {
    #[must_use]
    pub fn new(image: TapImage) -> Self {
        let mut p = Self {
            image,
            block: 0,
            playing: true,
            pulses: Vec::new(),
            pulse_i: 0,
            remain: 0,
            level: false,
            speed: 1,
            block_pause_t: Vec::new(),
        };
        p.block_pause_t = p.image.pause_t.clone();
        p.queue_block(0);
        p
    }

    /// TZX-derived pauses (T-states) aligned with [`TapImage::blocks`].
    pub fn set_block_pauses(&mut self, pause_t: Vec<u32>) {
        self.block_pause_t = pause_t;
        self.queue_block(self.block);
    }

    pub fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
    }

    /// EAR turbo (`1` = realtime). Rebuilds the current block's pulse schedule.
    pub fn set_speed(&mut self, speed: u32) {
        self.speed = speed.clamp(1, 64);
        self.queue_block(self.block);
    }

    #[must_use]
    pub fn speed(&self) -> u32 {
        self.speed
    }

    /// Rewind to the first block and pause.
    pub fn rewind(&mut self) {
        self.block = 0;
        self.playing = false;
        self.queue_block(0);
    }

    /// Number of pulses currently scheduled (including pause). Useful for tests.
    #[must_use]
    pub fn scheduled_pulses(&self) -> usize {
        self.pulses.len()
    }

    /// Index of the active pulse within [`Self::scheduled_pulses`] (0 if idle).
    #[must_use]
    pub fn pulse_index(&self) -> usize {
        self.pulse_i
    }

    #[must_use]
    pub fn ear_level(&self) -> bool {
        self.level
    }

    #[must_use]
    pub fn finished(&self) -> bool {
        self.block >= self.image.blocks.len() && self.pulses.is_empty()
    }

    /// Skip the EAR bitstream for the current block (used by flash-load traps).
    pub fn consume_block(&mut self) {
        if self.block < self.image.blocks.len() {
            self.block += 1;
        }
        self.queue_block(self.block);
    }

    /// Restore the deck to `idx` without changing play/pause (flash-load search rollback).
    pub fn rewind_to_block(&mut self, idx: usize) {
        self.block = idx.min(self.image.blocks.len());
        self.queue_block(self.block);
    }

    #[must_use]
    pub fn current_block_bytes(&self) -> Option<&[u8]> {
        self.image.blocks.get(self.block).map(Vec::as_slice)
    }

    fn queue_block(&mut self, idx: usize) {
        self.pulses.clear();
        self.pulse_i = 0;
        self.remain = 0;
        let Some(block) = self.image.blocks.get(idx) else {
            self.level = false;
            return;
        };
        // Pilot: `N` alternating edges of 2168 T. Sync/data must continue toggling —
        // a hardcoded SYNC1 high after an odd leader merges the last pilot with sync
        // and the 48K ROM never locks onto the block.
        let flag = block.first().copied().unwrap_or(0);
        let mut level = true;
        for _ in 0..pilot_pulses_for_speed(flag, self.speed) {
            push_pulse(&mut self.pulses, &mut level, PILOT_PULSE_T);
        }
        push_pulse(&mut self.pulses, &mut level, SYNC1_T);
        push_pulse(&mut self.pulses, &mut level, SYNC2_T);
        for &byte in block {
            for bit in (0..8).rev() {
                let one = byte & (1 << bit) != 0;
                let len = if one { BIT1_T } else { BIT0_T };
                push_pulse(&mut self.pulses, &mut level, len);
                push_pulse(&mut self.pulses, &mut level, len);
            }
        }
        let pause = self
            .block_pause_t
            .get(idx)
            .copied()
            .filter(|&t| t > 0)
            .map(|t| (t / self.speed.max(1)).max(3_500))
            .unwrap_or_else(|| pause_t_for_speed(self.speed));
        push_pulse(&mut self.pulses, &mut level, pause);
        if let Some(&(r, l)) = self.pulses.first() {
            self.remain = r;
            self.level = l;
            self.pulse_i = 0;
        }
    }

    /// Advance `dt` T-states; returns current EAR level.
    pub fn advance(&mut self, mut dt: u32) -> bool {
        if !self.playing {
            return self.level;
        }
        while dt > 0 {
            if self.pulses.is_empty() {
                self.level = false;
                break;
            }
            if self.remain == 0 {
                self.pulse_i += 1;
                if self.pulse_i >= self.pulses.len() {
                    self.block += 1;
                    self.queue_block(self.block);
                    continue;
                }
                let (r, l) = self.pulses[self.pulse_i];
                self.remain = r;
                self.level = l;
            }
            let step = dt.min(self.remain);
            self.remain -= step;
            dt -= step;
        }
        // If we landed exactly on a pulse boundary at end-of-block, promote.
        if self.remain == 0 && !self.pulses.is_empty() && self.pulse_i + 1 >= self.pulses.len() {
            self.block += 1;
            self.queue_block(self.block);
        }
        self.level
    }
}

/// XOR checksum used by Spectrum TAP blocks (all bytes including flag, excluding the checksum byte itself).
#[must_use]
pub fn tap_checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |acc, b| acc ^ b)
}

/// Flash-load: poke TAP block payload (excluding flag + checksum) into memory.
pub fn flash_load_block(machine_ram_write: &mut dyn FnMut(u16, u8), block: &[u8], addr: u16) {
    if block.len() < 2 {
        return;
    }
    // Skip flag (first) and checksum (last)
    for (i, b) in block[1..block.len() - 1].iter().enumerate() {
        machine_ram_write(addr.wrapping_add(i as u16), *b);
    }
}

/// Result of attempting an LD-BYTES flash-load trap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TapeTrapResult {
    /// No trap (PC mismatch or no tape).
    Ignored,
    /// Block loaded or verified; caller should simulate RET with carry set.
    Success { addr: u16, len: u16 },
    /// Missing/mismatched block; caller should simulate RET with carry clear.
    Failure,
}

/// Interpret CPU state at [`LD_BYTES_TRAP_PC`] and apply the next TAP block if present.
///
/// The trap is ignored while the deck is paused (`playing == false`) so Tape → Play is required
/// before flash-load or EAR bitstream progress.
///
/// Blocks whose flag byte does not match `flag_expected` are skipped (ROM keeps searching).
///
/// On `Success` / `Failure`, the player has already consumed (or not) the block as appropriate;
/// the CPU must still perform a ROM-compatible return (RET) and flag update.
///
/// **Register note:** the 48K ROM executes `EX AF,AF'` before [`LD_BYTES_TRAP_PC`], so callers
/// must pass the expected flag / load-vs-verify carry from **A′ / F′**, not A / F.
///
/// When the `tape` trace category is enabled, skip/fail reasons are recorded for debugging.
#[must_use]
pub fn evaluate_ld_bytes_trap(
    pc: u16,
    flag_expected: u8,
    load: bool,
    addr: u16,
    len: u16,
    player: &mut TapPlayer,
) -> TapeTrapResult {
    if pc != LD_BYTES_TRAP_PC {
        return TapeTrapResult::Ignored;
    }
    if !player.playing {
        trace::emit(trace::EventKind::FlashLoadSkip {
            reason: trace::FlashSkipReason::Paused,
            block: player.block as u32,
            flag_got: 0,
            flag_want: flag_expected,
            block_len: 0,
            want_len: len,
        });
        return TapeTrapResult::Ignored;
    }
    // `load` is kept for a ROM-compatible signature; the machine layer applies
    // load-vs-verify (poke memory or not) after Success.
    let _ = load;
    let start_block = player.block;
    loop {
        let block_i = player.block as u32;
        let Some(block) = player.current_block_bytes() else {
            // No matching flag left: restore so a retry does not need a manual rewind.
            player.rewind_to_block(start_block);
            trace::emit(trace::EventKind::FlashLoadSkip {
                reason: trace::FlashSkipReason::NoBlock,
                block: block_i,
                flag_got: 0,
                flag_want: flag_expected,
                block_len: 0,
                want_len: len,
            });
            return TapeTrapResult::Failure;
        };
        if block.is_empty() {
            trace::emit(trace::EventKind::FlashLoadSkip {
                reason: trace::FlashSkipReason::EmptyBlock,
                block: block_i,
                flag_got: 0,
                flag_want: flag_expected,
                block_len: 0,
                want_len: len,
            });
            player.consume_block();
            continue;
        }
        let flag_got = block[0];
        let block_len = block.len() as u16;
        if flag_got != flag_expected {
            // Wrong flag: skip and keep searching (authentic LD-BYTES behaviour).
            trace::emit(trace::EventKind::FlashLoadSkip {
                reason: trace::FlashSkipReason::WrongFlag,
                block: block_i,
                flag_got,
                flag_want: flag_expected,
                block_len,
                want_len: len,
            });
            player.consume_block();
            continue;
        }
        // Block is flag + `len` data bytes + checksum
        if block.len() != usize::from(len) + 2 {
            trace::emit(trace::EventKind::FlashLoadSkip {
                reason: trace::FlashSkipReason::LengthMismatch,
                block: block_i,
                flag_got,
                flag_want: flag_expected,
                block_len,
                want_len: len,
            });
            return TapeTrapResult::Failure;
        }
        let checksum = block[block.len() - 1];
        if tap_checksum(&block[..block.len() - 1]) != checksum {
            trace::emit(trace::EventKind::FlashLoadSkip {
                reason: trace::FlashSkipReason::ChecksumFail,
                block: block_i,
                flag_got,
                flag_want: flag_expected,
                block_len,
                want_len: len,
            });
            return TapeTrapResult::Failure;
        }
        trace::emit(trace::EventKind::TapeBlock {
            index: block_i,
            flag: flag_got,
            len,
        });
        player.consume_block();
        return TapeTrapResult::Success { addr, len };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture_tap() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/tape/minimal_code.tap")
    }

    #[test]
    fn parse_empty() {
        let t = TapImage::parse(&[]).unwrap();
        assert!(t.blocks.is_empty());
    }

    #[test]
    fn parse_one_block() {
        let raw = vec![0x03, 0x00, 0x00, 0x11, 0x22];
        let t = TapImage::parse(&raw).unwrap();
        assert_eq!(t.blocks.len(), 1);
        assert_eq!(t.blocks[0], vec![0x00, 0x11, 0x22]);
    }

    #[test]
    fn header_pilot_pulse_count_matches_rom() {
        let img = TapImage {
            blocks: vec![vec![0x00, 0x11, 0x22]],
            ..Default::default()
        };
        let p = TapPlayer::new(img);
        // pilot + sync(2) + 3 bytes * 8 bits * 2 edges + pause
        let expected = PILOT_HEADER_PULSES as usize + 2 + (3 * 8 * 2) + 1;
        assert_eq!(p.scheduled_pulses(), expected);
    }

    #[test]
    fn data_pilot_pulse_count_matches_rom() {
        let img = TapImage {
            blocks: vec![vec![0xff, 0x00]],
            ..Default::default()
        };
        let p = TapPlayer::new(img);
        let expected = PILOT_DATA_PULSES as usize + 2 + (2 * 8 * 2) + 1;
        assert_eq!(p.scheduled_pulses(), expected);
    }

    #[test]
    fn ear_toggles_during_pilot() {
        let img = TapImage {
            blocks: vec![vec![0x00, 0x00]],
            ..Default::default()
        };
        let mut p = TapPlayer::new(img);
        let first = p.ear_level();
        assert!(first, "pilot starts high");
        // Mid-first-pulse: still high
        assert!(p.advance(PILOT_PULSE_T / 2));
        // Cross into second pilot pulse: low
        assert!(!p.advance(PILOT_PULSE_T));
        // Third pulse: high again
        assert!(p.advance(PILOT_PULSE_T));
    }

    #[test]
    fn tap_pulses_always_alternate_including_sync() {
        let img = TapImage {
            blocks: vec![vec![0x00, 0x11, 0x22]],
            ..Default::default()
        };
        let p = TapPlayer::new(img);
        let pulses = p.pulses.clone();
        assert!(pulses.len() > PILOT_HEADER_PULSES as usize + 2);
        for w in pulses.windows(2) {
            assert_ne!(
                w[0].1, w[1].1,
                "adjacent EAR pulses must toggle (merged sync/pilot breaks ROM LD-BYTES)"
            );
        }
        let last_pilot = pulses[PILOT_HEADER_PULSES as usize - 1];
        let sync1 = pulses[PILOT_HEADER_PULSES as usize];
        assert_eq!(sync1.0, SYNC1_T);
        assert_ne!(last_pilot.1, sync1.1);
    }

    #[test]
    fn turbo_speed_shortens_leader_not_bit_widths() {
        let img = TapImage {
            blocks: vec![vec![0x00, 0x00]],
            ..Default::default()
        };
        let mut p = TapPlayer::new(img);
        let slow_n = p.scheduled_pulses();
        p.set_speed(10);
        let fast_n = p.scheduled_pulses();
        assert!(
            fast_n < slow_n,
            "10x should drop leader pulses ({slow_n} vs {fast_n})"
        );
        assert_eq!(
            pilot_pulses_for_speed(0, 10),
            (PILOT_HEADER_PULSES / 10).max(2_560)
        );
        assert_eq!(p.pulses[0].0, PILOT_PULSE_T);
    }

    #[test]
    fn advances_to_next_block_after_pause() {
        let img = TapImage {
            blocks: vec![vec![0x00], vec![0xff, 0x00]],
            ..Default::default()
        };
        let mut p = TapPlayer::new(img);
        assert_eq!(p.block, 0);
        // Pilot + sync + 1 byte*16 edges + pause
        let t = PILOT_HEADER_PULSES * PILOT_PULSE_T
            + SYNC1_T
            + SYNC2_T
            + 16 * BIT0_T // flag 0x00 → all zero bits
            + PAUSE_T;
        p.advance(t);
        assert_eq!(p.block, 1);
        assert_eq!(
            p.scheduled_pulses(),
            PILOT_DATA_PULSES as usize + 2 + (2 * 8 * 2) + 1
        );
    }

    #[test]
    fn load_minimal_fixture() {
        let path = fixture_tap();
        let img = TapImage::load(&path).expect("fixture TAP");
        assert_eq!(img.blocks.len(), 2);
        assert_eq!(img.blocks[0][0], 0x00);
        assert_eq!(img.blocks[1][0], 0xff);
        assert_eq!(
            tap_checksum(&img.blocks[0][..img.blocks[0].len() - 1]),
            *img.blocks[0].last().unwrap()
        );
    }

    #[test]
    fn flash_load_skips_flag_and_checksum() {
        let block = vec![0xff, 0x11, 0x22, 0x33, 0xff ^ 0x11 ^ 0x22 ^ 0x33];
        let mut mem = [0u8; 8];
        flash_load_block(
            &mut |addr, v| {
                mem[addr as usize] = v;
            },
            &block,
            0,
        );
        assert_eq!(&mem[..3], &[0x11, 0x22, 0x33]);
    }

    #[test]
    fn ld_bytes_trap_consumes_matching_block() {
        let img = TapImage {
            blocks: vec![vec![0xff, 0x42, 0xff ^ 0x42]],
            ..Default::default()
        };
        let mut p = TapPlayer::new(img);
        let r = evaluate_ld_bytes_trap(LD_BYTES_TRAP_PC, 0xff, true, 0x8000, 1, &mut p);
        assert_eq!(
            r,
            TapeTrapResult::Success {
                addr: 0x8000,
                len: 1
            }
        );
        assert_eq!(p.block, 1);
        assert!(p.finished() || p.current_block_bytes().is_none());
    }

    #[test]
    fn ld_bytes_trap_skips_wrong_flag_then_loads() {
        let data = vec![0xff, 0x42, 0xff ^ 0x42];
        let header = {
            let mut h = vec![0x00, 0x11];
            h.push(tap_checksum(&h));
            h
        };
        let img = TapImage {
            blocks: vec![data, header.clone()],
            ..Default::default()
        };
        let mut p = TapPlayer::new(img);
        // Expecting header flag 0 — first block is data, should be skipped.
        let r = evaluate_ld_bytes_trap(LD_BYTES_TRAP_PC, 0x00, true, 0x5c00, 1, &mut p);
        assert_eq!(
            r,
            TapeTrapResult::Success {
                addr: 0x5c00,
                len: 1
            }
        );
        assert_eq!(p.block, 2);
    }

    #[test]
    fn ld_bytes_trap_restores_position_when_flag_not_found() {
        let img = TapImage {
            blocks: vec![vec![0xff, 0x42, 0xff ^ 0x42]],
            ..Default::default()
        };
        let mut p = TapPlayer::new(img);
        let r = evaluate_ld_bytes_trap(LD_BYTES_TRAP_PC, 0x00, true, 0x5c00, 1, &mut p);
        assert_eq!(r, TapeTrapResult::Failure);
        assert_eq!(p.block, 0, "should not drain the deck when no flag matches");
    }

    #[test]
    fn ld_bytes_trap_ignored_while_paused() {
        let img = TapImage {
            blocks: vec![vec![0xff, 0x42, 0xff ^ 0x42]],
            ..Default::default()
        };
        let mut p = TapPlayer::new(img);
        p.set_playing(false);
        let r = evaluate_ld_bytes_trap(LD_BYTES_TRAP_PC, 0xff, true, 0x8000, 1, &mut p);
        assert_eq!(r, TapeTrapResult::Ignored);
        assert_eq!(p.block, 0);
        p.set_playing(true);
        let r = evaluate_ld_bytes_trap(LD_BYTES_TRAP_PC, 0xff, true, 0x8000, 1, &mut p);
        assert_eq!(
            r,
            TapeTrapResult::Success {
                addr: 0x8000,
                len: 1
            }
        );
    }

    #[test]
    fn advance_does_not_consume_while_paused() {
        let img = TapImage {
            blocks: vec![vec![0x00, 0x00]],
            ..Default::default()
        };
        let mut p = TapPlayer::new(img);
        p.set_playing(false);
        let level = p.ear_level();
        assert_eq!(p.advance(PILOT_PULSE_T * 10), level);
        assert_eq!(p.block, 0);
        p.set_playing(true);
        assert!(p.advance(PILOT_PULSE_T / 2), "still in first pilot pulse");
        assert!(!p.advance(PILOT_PULSE_T), "cross into second pilot pulse");
    }

    #[test]
    fn ld_bytes_trap_pc_requires_rom_prologue() {
        let mut rom = [0u8; 0x600];
        rom[0x0556] = 0x14;
        rom[0x0557] = 0x08;
        rom[0x0558] = 0x15;
        rom[0x0559] = 0xF3;
        assert!(is_ld_bytes_trap_pc(LD_BYTES_TRAP_PC, |a| rom
            .get(a as usize)
            .copied()
            .unwrap_or(0)));
        rom[0x0556] = 0x20; // 128K editor ROM 0
        assert!(!is_ld_bytes_trap_pc(LD_BYTES_TRAP_PC, |a| rom
            .get(a as usize)
            .copied()
            .unwrap_or(0)));
    }

    #[test]
    fn ld_bytes_trap_pc_matches_relocated_boggit_style_loader() {
        let mut mem = [0u8; 0x200];
        // Fake RAM clone at 0x8097 mapped into this slice at 0.
        let start = 0u16;
        mem[start as usize] = 0x14;
        mem[1] = 0x08;
        mem[2] = 0x15;
        mem[3] = 0xF3;
        let call_pc = start + LD_BYTES_EDGE_CALL_OFF;
        mem[call_pc as usize] = 0xCD;
        let read = |a: u16| {
            if a >= 0x8097 {
                mem.get((a - 0x8097) as usize).copied().unwrap_or(0)
            } else {
                0
            }
        };
        assert!(is_ld_bytes_trap_pc(0x8097 + LD_BYTES_EDGE_CALL_OFF, read));
        assert!(!is_ld_bytes_trap_pc(0x8097, read));
    }
}
