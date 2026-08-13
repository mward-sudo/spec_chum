//! Spec Chum tape — TAP/TZX loading via EAR bitstream.

#![allow(clippy::pedantic)]

mod tzx;

pub use tzx::{TzxError, TzxPlayer};

use std::path::Path;

use thiserror::Error;

#[derive(Clone, Debug)]
pub struct TapImage {
    pub blocks: Vec<Vec<u8>>,
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
        Ok(Self { blocks })
    }
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
        };
        p.queue_block(0);
        p
    }

    pub fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
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
        // Pilot: `N` alternating pulses of 2168 T (not N pairs).
        let flag = block.first().copied().unwrap_or(0);
        let pilot_count = if flag == 0 {
            PILOT_HEADER_PULSES
        } else {
            PILOT_DATA_PULSES
        };
        let mut level = true;
        for _ in 0..pilot_count {
            self.pulses.push((PILOT_PULSE_T, level));
            level = !level;
        }
        // Sync pulses
        self.pulses.push((SYNC1_T, true));
        self.pulses.push((SYNC2_T, false));
        // Data bits (MSB first): each bit is two equal edges
        for &byte in block {
            for bit in (0..8).rev() {
                let one = byte & (1 << bit) != 0;
                let len = if one { BIT1_T } else { BIT0_T };
                self.pulses.push((len, true));
                self.pulses.push((len, false));
            }
        }
        // Pause (silence)
        self.pulses.push((PAUSE_T, false));
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
        if trace::enabled(trace::Category::TAPE) {
            trace::emit(trace::EventKind::FlashLoadSkip {
                reason: trace::FlashSkipReason::Paused,
                block: player.block as u32,
                flag_got: 0,
                flag_want: flag_expected,
                block_len: 0,
                want_len: len,
            });
        }
        return TapeTrapResult::Ignored;
    }
<<<<<<< HEAD
    // `load` is kept for a ROM-compatible signature; the machine layer applies
    // load-vs-verify (poke memory or not) after Success.
    let _ = load;
    let start_block = player.block;
=======
    let _ = load;
>>>>>>> 92c3806 (Add structured emulator debug tracing and dump harness.)
    loop {
        let block_i = player.block as u32;
        let Some(block) = player.current_block_bytes() else {
<<<<<<< HEAD
            // No matching flag left: restore so a retry does not need a manual rewind.
            player.rewind_to_block(start_block);
=======
            trace::emit(trace::EventKind::FlashLoadSkip {
                reason: trace::FlashSkipReason::NoBlock,
                block: block_i,
                flag_got: 0,
                flag_want: flag_expected,
                block_len: 0,
                want_len: len,
            });
>>>>>>> 92c3806 (Add structured emulator debug tracing and dump harness.)
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
<<<<<<< HEAD
=======
        trace::emit(trace::EventKind::TapeBlock {
            index: block_i,
            flag: flag_got,
            len,
        });
>>>>>>> 92c3806 (Add structured emulator debug tracing and dump harness.)
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
        };
        let p = TapPlayer::new(img);
        let expected = PILOT_DATA_PULSES as usize + 2 + (2 * 8 * 2) + 1;
        assert_eq!(p.scheduled_pulses(), expected);
    }

    #[test]
    fn ear_toggles_during_pilot() {
        let img = TapImage {
            blocks: vec![vec![0x00, 0x00]],
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
    fn advances_to_next_block_after_pause() {
        let img = TapImage {
            blocks: vec![vec![0x00], vec![0xff, 0x00]],
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
}
