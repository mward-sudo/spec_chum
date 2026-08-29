//! TZX tape image parsing and EAR pulse playback.
//!
//! Supported block IDs for playback: 0x10, 0x11, 0x12, 0x13, 0x14, 0x20.
//! Informational / skip blocks: 0x21, 0x22, 0x30, 0x32, 0x33, 0x35, 0x5A.

use std::path::Path;

use thiserror::Error;

use crate::{
    TapImage, TapeError, BIT0_T, BIT1_T, PAUSE_T, PILOT_DATA_PULSES, PILOT_HEADER_PULSES,
    PILOT_PULSE_T, SYNC1_T, SYNC2_T,
};

#[derive(Debug, Error)]
pub enum TzxError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Format(String),
}

impl From<TzxError> for TapeError {
    fn from(e: TzxError) -> Self {
        match e {
            TzxError::Io(io) => Self::Io(io),
            TzxError::Format(s) => Self::Format(s),
        }
    }
}

/// Parsed TZX as an EAR pulse schedule (duration T-states, level).
#[derive(Clone, Debug)]
pub struct TzxPlayer {
    pulses: Vec<(u32, bool)>,
    pulse_i: usize,
    remain: u32,
    level: bool,
    /// When false, [`Self::advance`] does not consume pulses.
    pub playing: bool,
    /// Index into logical playable blocks (for UI/tests).
    pub block: usize,
    block_starts: Vec<usize>,
}

impl TzxPlayer {
    pub fn load(path: &Path) -> Result<Self, TzxError> {
        let data = std::fs::read(path)?;
        Self::parse(&data)
    }

    pub fn parse(data: &[u8]) -> Result<Self, TzxError> {
        if data.len() < 10 || &data[0..7] != b"ZXTape!" || data[7] != 0x1a {
            return Err(TzxError::Format("missing TZX signature".into()));
        }
        let mut pulses = Vec::new();
        let mut block_starts = Vec::new();
        let mut level = false;
        let mut i = 10usize; // skip header (sig + ver)
        while i < data.len() {
            let id = data[i];
            i += 1;
            match id {
                0x10 => {
                    if i + 4 > data.len() {
                        return Err(TzxError::Format("truncated 0x10".into()));
                    }
                    let pause_ms = u16::from_le_bytes([data[i], data[i + 1]]);
                    let len = u16::from_le_bytes([data[i + 2], data[i + 3]]) as usize;
                    i += 4;
                    if i + len > data.len() {
                        return Err(TzxError::Format("0x10 data truncated".into()));
                    }
                    let block = &data[i..i + len];
                    i += len;
                    block_starts.push(pulses.len());
                    append_standard_block(&mut pulses, &mut level, block, pause_ms);
                }
                0x11 => {
                    // 18-byte turbo header (pilot…length); need indices i..i+17.
                    if i + 18 > data.len() {
                        return Err(TzxError::Format("truncated 0x11".into()));
                    }
                    let pilot = u16::from_le_bytes([data[i], data[i + 1]]);
                    let sync1 = u16::from_le_bytes([data[i + 2], data[i + 3]]);
                    let sync2 = u16::from_le_bytes([data[i + 4], data[i + 5]]);
                    let zero = u16::from_le_bytes([data[i + 6], data[i + 7]]);
                    let one = u16::from_le_bytes([data[i + 8], data[i + 9]]);
                    let pilot_pulses = u16::from_le_bytes([data[i + 10], data[i + 11]]);
                    let used_bits = data[i + 12];
                    let pause_ms = u16::from_le_bytes([data[i + 13], data[i + 14]]);
                    let len = u32::from(data[i + 15])
                        | (u32::from(data[i + 16]) << 8)
                        | (u32::from(data[i + 17]) << 16);
                    i += 18;
                    let len = len as usize;
                    if i + len > data.len() {
                        return Err(TzxError::Format("0x11 data truncated".into()));
                    }
                    let block = &data[i..i + len];
                    i += len;
                    block_starts.push(pulses.len());
                    append_turbo_block(
                        &mut pulses,
                        &mut level,
                        block,
                        pilot,
                        sync1,
                        sync2,
                        zero,
                        one,
                        pilot_pulses,
                        used_bits,
                        pause_ms,
                    );
                }
                0x12 => {
                    if i + 4 > data.len() {
                        return Err(TzxError::Format("truncated 0x12".into()));
                    }
                    let len = u16::from_le_bytes([data[i], data[i + 1]]);
                    let count = u16::from_le_bytes([data[i + 2], data[i + 3]]);
                    i += 4;
                    block_starts.push(pulses.len());
                    for _ in 0..count {
                        crate::push_pulse(&mut pulses, &mut level, u32::from(len));
                    }
                }
                0x13 => {
                    if i >= data.len() {
                        return Err(TzxError::Format("truncated 0x13".into()));
                    }
                    let n = data[i] as usize;
                    i += 1;
                    if i + n * 2 > data.len() {
                        return Err(TzxError::Format("0x13 pulses truncated".into()));
                    }
                    block_starts.push(pulses.len());
                    for _ in 0..n {
                        let len = u16::from_le_bytes([data[i], data[i + 1]]);
                        i += 2;
                        crate::push_pulse(&mut pulses, &mut level, u32::from(len));
                    }
                }
                0x14 => {
                    if i + 10 > data.len() {
                        return Err(TzxError::Format("truncated 0x14".into()));
                    }
                    let zero = u16::from_le_bytes([data[i], data[i + 1]]);
                    let one = u16::from_le_bytes([data[i + 2], data[i + 3]]);
                    let used_bits = data[i + 4];
                    let pause_ms = u16::from_le_bytes([data[i + 5], data[i + 6]]);
                    let len = u32::from(data[i + 7])
                        | (u32::from(data[i + 8]) << 8)
                        | (u32::from(data[i + 9]) << 16);
                    i += 10;
                    let len = len as usize;
                    if i + len > data.len() {
                        return Err(TzxError::Format("0x14 data truncated".into()));
                    }
                    let block = &data[i..i + len];
                    i += len;
                    block_starts.push(pulses.len());
                    append_pure_data(
                        &mut pulses,
                        &mut level,
                        block,
                        zero,
                        one,
                        used_bits,
                        pause_ms,
                    );
                }
                0x20 => {
                    if i + 2 > data.len() {
                        return Err(TzxError::Format("truncated 0x20".into()));
                    }
                    let pause_ms = u16::from_le_bytes([data[i], data[i + 1]]);
                    i += 2;
                    block_starts.push(pulses.len());
                    let t = ms_to_t(pause_ms);
                    if t > 0 {
                        pulses.push((t, false));
                        level = false;
                    }
                }
                0x21 => {
                    if i >= data.len() {
                        return Err(TzxError::Format("truncated 0x21".into()));
                    }
                    let n = data[i] as usize;
                    i += 1 + n;
                }
                0x22 => {}
                0x30 => {
                    if i >= data.len() {
                        return Err(TzxError::Format("truncated 0x30".into()));
                    }
                    let n = data[i] as usize;
                    i += 1 + n;
                }
                0x32 => {
                    if i + 2 > data.len() {
                        return Err(TzxError::Format("truncated 0x32".into()));
                    }
                    let n = u16::from_le_bytes([data[i], data[i + 1]]) as usize;
                    i += 2 + n;
                }
                0x33 => {
                    if i >= data.len() {
                        return Err(TzxError::Format("truncated 0x33".into()));
                    }
                    let n = data[i] as usize;
                    i += 1 + n * 3;
                }
                0x35 => {
                    if i + 16 > data.len() {
                        return Err(TzxError::Format("truncated 0x35".into()));
                    }
                    let n =
                        u32::from_le_bytes(data[i + 0x10..i + 0x14].try_into().unwrap_or([0; 4]))
                            as usize;
                    i += 0x14 + n;
                }
                0x5a => {
                    i += 9; // glue block
                }
                other => {
                    return Err(TzxError::Format(format!(
                        "unsupported TZX block ID 0x{other:02x} at offset {}",
                        i - 1
                    )));
                }
            }
        }
        let mut player = Self {
            pulses,
            pulse_i: 0,
            remain: 0,
            level: false,
            playing: true,
            block: 0,
            block_starts,
        };
        player.start();
        Ok(player)
    }

    pub fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
    }

    /// Rewind to the first pulse and pause.
    pub fn rewind(&mut self) {
        self.playing = false;
        self.block = 0;
        self.start();
    }

    fn start(&mut self) {
        if let Some(&(r, l)) = self.pulses.first() {
            self.remain = r;
            self.level = l;
            self.pulse_i = 0;
        } else {
            self.remain = 0;
            self.level = false;
            self.pulse_i = 0;
        }
        self.sync_block();
    }

    /// Keep `block` aligned with `pulse_i`, including zero-pulse logical blocks
    /// (pure tone with count 0, pause with duration 0) and trailing empty entries.
    fn sync_block(&mut self) {
        while self.block + 1 < self.block_starts.len()
            && self.block_starts[self.block + 1] <= self.pulse_i
        {
            self.block += 1;
        }
    }

    #[must_use]
    pub fn ear_level(&self) -> bool {
        self.level
    }

    #[must_use]
    pub fn scheduled_pulses(&self) -> usize {
        self.pulses.len()
    }

    #[must_use]
    pub fn pulse_index(&self) -> usize {
        self.pulse_i
    }

    /// True when every scheduled pulse has been consumed.
    #[must_use]
    pub fn finished(&self) -> bool {
        self.pulses.is_empty() || self.pulse_i >= self.pulses.len()
    }

    /// Pulse index within the active logical block (for UI progress).
    #[must_use]
    pub fn active_pulse_index(&self) -> usize {
        let start = self.block_starts.get(self.block).copied().unwrap_or(0);
        self.pulse_i.saturating_sub(start)
    }

    /// Pulse count for the active logical block (for UI progress).
    #[must_use]
    pub fn active_pulse_count(&self) -> usize {
        let start = self.block_starts.get(self.block).copied().unwrap_or(0);
        let end = self
            .block_starts
            .get(self.block + 1)
            .copied()
            .unwrap_or(self.pulses.len());
        end.saturating_sub(start)
    }

    #[must_use]
    pub fn block_count(&self) -> usize {
        self.block_starts.len()
    }

    /// Advance `dt` T-states; returns current EAR level.
    pub fn advance(&mut self, mut dt: u32) -> bool {
        if !self.playing {
            return self.level;
        }
        while dt > 0 {
            if self.pulses.is_empty() {
                self.sync_block();
                self.level = false;
                break;
            }
            if self.remain == 0 {
                self.pulse_i += 1;
                if self.pulse_i >= self.pulses.len() {
                    self.sync_block();
                    self.level = false;
                    break;
                }
                let (r, l) = self.pulses[self.pulse_i];
                self.remain = r;
                self.level = l;
                self.sync_block();
            }
            let step = dt.min(self.remain);
            self.remain -= step;
            dt -= step;
        }
        // #178: when the final pulse is consumed exactly (`remain == 0` while
        // still indexing the last pulse), promote to exhaustion immediately so
        // EAR turbo stops without requiring another `advance` call.
        if !self.pulses.is_empty()
            && self.remain == 0
            && self.pulse_i.saturating_add(1) >= self.pulses.len()
        {
            self.pulse_i = self.pulses.len();
            self.sync_block();
            // Idle EAR is low; do not leave a high final pulse latched after stop.
            self.level = false;
        }
        if self.finished() {
            self.playing = false;
        }
        self.level
    }

    /// True when every data-bearing block is standard-speed (0x10) — safe to flash-load via TAP.
    #[must_use]
    pub fn is_standard_speed_only(data: &[u8]) -> bool {
        if data.len() < 10 || &data[0..7] != b"ZXTape!" {
            return false;
        }
        let mut i = 10usize;
        let mut saw_data = false;
        while i < data.len() {
            let id = data[i];
            i += 1;
            match id {
                0x10 => {
                    saw_data = true;
                    if i + 4 > data.len() {
                        return false;
                    }
                    let len = u16::from_le_bytes([data[i + 2], data[i + 3]]) as usize;
                    i += 4 + len;
                }
                0x11..=0x14 => return false,
                0x20 => i += 2,
                0x21 => {
                    let n = data.get(i).copied().unwrap_or(0) as usize;
                    i += 1 + n;
                }
                0x22 => {}
                0x30 => {
                    let n = data.get(i).copied().unwrap_or(0) as usize;
                    i += 1 + n;
                }
                0x32 => {
                    if i + 2 > data.len() {
                        return false;
                    }
                    let n = u16::from_le_bytes([data[i], data[i + 1]]) as usize;
                    i += 2 + n;
                }
                0x33 => {
                    let n = data.get(i).copied().unwrap_or(0) as usize;
                    i += 1 + n * 3;
                }
                0x35 => {
                    if i + 0x14 > data.len() {
                        return false;
                    }
                    let n = u32::from_le_bytes([
                        data[i + 0x10],
                        data[i + 0x11],
                        data[i + 0x12],
                        data[i + 0x13],
                    ]) as usize;
                    i += 0x14 + n;
                }
                0x5a => i += 9,
                _ => return false,
            }
        }
        saw_data
    }

    /// Extract standard-speed (0x10) payloads as a TAP image when present.
    pub fn to_tap_image(data: &[u8]) -> Result<TapImage, TzxError> {
        if data.len() < 10 || &data[0..7] != b"ZXTape!" {
            return Err(TzxError::Format("missing TZX signature".into()));
        }
        let mut blocks = Vec::new();
        let mut pause_t = Vec::new();
        let mut i = 10usize;
        while i < data.len() {
            let id = data[i];
            i += 1;
            match id {
                0x10 => {
                    if i + 4 > data.len() {
                        break;
                    }
                    let pause_ms = u16::from_le_bytes([data[i], data[i + 1]]);
                    let len = u16::from_le_bytes([data[i + 2], data[i + 3]]) as usize;
                    i += 4;
                    if i + len > data.len() {
                        break;
                    }
                    blocks.push(data[i..i + len].to_vec());
                    pause_t.push(ms_to_t(pause_ms).max(1));
                    i += len;
                }
                0x11 => {
                    if i + 18 > data.len() {
                        break;
                    }
                    let len = (u32::from(data[i + 15])
                        | (u32::from(data[i + 16]) << 8)
                        | (u32::from(data[i + 17]) << 16)) as usize;
                    i += 18 + len;
                }
                0x12 => i += 4,
                0x13 => {
                    let n = data.get(i).copied().unwrap_or(0) as usize;
                    i += 1 + n * 2;
                }
                0x14 => {
                    if i + 10 > data.len() {
                        break;
                    }
                    let len = (u32::from(data[i + 7])
                        | (u32::from(data[i + 8]) << 8)
                        | (u32::from(data[i + 9]) << 16)) as usize;
                    i += 10 + len;
                }
                0x20 => i += 2,
                0x21 => {
                    let n = data.get(i).copied().unwrap_or(0) as usize;
                    i += 1 + n;
                }
                0x22 => {}
                0x30 => {
                    let n = data.get(i).copied().unwrap_or(0) as usize;
                    i += 1 + n;
                }
                0x32 => {
                    if i + 2 > data.len() {
                        break;
                    }
                    let n = u16::from_le_bytes([data[i], data[i + 1]]) as usize;
                    i += 2 + n;
                }
                0x33 => {
                    let n = data.get(i).copied().unwrap_or(0) as usize;
                    i += 1 + n * 3;
                }
                0x35 => {
                    if i + 0x14 > data.len() {
                        break;
                    }
                    let n = u32::from_le_bytes([
                        data[i + 0x10],
                        data[i + 0x11],
                        data[i + 0x12],
                        data[i + 0x13],
                    ]) as usize;
                    i += 0x14 + n;
                }
                0x5a => i += 9,
                _ => break,
            }
        }
        Ok(TapImage { blocks, pause_t })
    }

    /// Standard-speed TZX as a TAP deck, keeping per-block pause lengths.
    pub fn to_tap_player(data: &[u8]) -> Result<crate::TapPlayer, TzxError> {
        Ok(crate::TapPlayer::new(Self::to_tap_image(data)?))
    }
}

fn ms_to_t(ms: u16) -> u32 {
    // ~3.5 MHz → 3500 T per ms
    u32::from(ms).saturating_mul(3500)
}

fn append_standard_block(
    pulses: &mut Vec<(u32, bool)>,
    level: &mut bool,
    block: &[u8],
    pause_ms: u16,
) {
    let flag = block.first().copied().unwrap_or(0);
    let pilot_count = if flag == 0 {
        PILOT_HEADER_PULSES
    } else {
        PILOT_DATA_PULSES
    };
    *level = true;
    for _ in 0..pilot_count {
        crate::push_pulse(pulses, level, PILOT_PULSE_T);
    }
    crate::push_pulse(pulses, level, SYNC1_T);
    crate::push_pulse(pulses, level, SYNC2_T);
    for &byte in block {
        for bit in (0..8).rev() {
            let one = byte & (1 << bit) != 0;
            let len = if one { BIT1_T } else { BIT0_T };
            crate::push_pulse(pulses, level, len);
            crate::push_pulse(pulses, level, len);
        }
    }
    let pause = if pause_ms == 0 {
        PAUSE_T
    } else {
        ms_to_t(pause_ms)
    };
    crate::push_pulse(pulses, level, pause);
}

#[allow(clippy::too_many_arguments)]
fn append_turbo_block(
    pulses: &mut Vec<(u32, bool)>,
    level: &mut bool,
    block: &[u8],
    pilot: u16,
    sync1: u16,
    sync2: u16,
    zero: u16,
    one: u16,
    pilot_pulses: u16,
    used_bits: u8,
    pause_ms: u16,
) {
    *level = true;
    for _ in 0..pilot_pulses {
        crate::push_pulse(pulses, level, u32::from(pilot));
    }
    crate::push_pulse(pulses, level, u32::from(sync1));
    crate::push_pulse(pulses, level, u32::from(sync2));
    append_pure_data(pulses, level, block, zero, one, used_bits, pause_ms);
}

fn append_pure_data(
    pulses: &mut Vec<(u32, bool)>,
    level: &mut bool,
    block: &[u8],
    zero: u16,
    one: u16,
    used_bits: u8,
    pause_ms: u16,
) {
    let used_bits = if used_bits == 0 || used_bits > 8 {
        8
    } else {
        used_bits
    };
    for (bi, &byte) in block.iter().enumerate() {
        let bits = if bi + 1 == block.len() { used_bits } else { 8 };
        for bit in (0..bits).rev() {
            let is_one = byte & (1 << bit) != 0;
            let len = if is_one { one } else { zero };
            crate::push_pulse(pulses, level, u32::from(len));
            crate::push_pulse(pulses, level, u32::from(len));
        }
    }
    let pause = ms_to_t(pause_ms);
    if pause > 0 {
        crate::push_pulse(pulses, level, pause);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn minimal_tzx_standard(payload: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"ZXTape!");
        v.push(0x1a);
        v.push(1); // major
        v.push(20); // minor
        v.push(0x10);
        v.extend_from_slice(&1000u16.to_le_bytes()); // pause ms
        v.extend_from_slice(&(payload.len() as u16).to_le_bytes());
        v.extend_from_slice(payload);
        v.push(0x20);
        v.extend_from_slice(&0u16.to_le_bytes());
        v
    }

    #[test]
    fn parse_standard_block_schedules_pulses() {
        let data = minimal_tzx_standard(&[0x00, 0x41, 0x00]);
        let p = TzxPlayer::parse(&data).unwrap();
        assert!(p.scheduled_pulses() > 10);
        let mut p = p;
        let _ = p.advance(PILOT_PULSE_T);
        assert!(p.ear_level());
    }

    #[test]
    fn to_tap_extracts_id10() {
        let payload = vec![0xff, 1, 2, 3, 0];
        let data = minimal_tzx_standard(&payload);
        let tap = TzxPlayer::to_tap_image(&data).unwrap();
        assert_eq!(tap.blocks.len(), 1);
        assert_eq!(tap.blocks[0], payload);
        assert_eq!(tap.pause_t.len(), 1);
        assert_eq!(tap.pause_t[0], 1000 * 3500);
    }

    #[test]
    fn pure_tone_block() {
        let mut v = Vec::new();
        v.extend_from_slice(b"ZXTape!");
        v.extend_from_slice(&[0x1a, 1, 20]);
        v.push(0x12);
        v.extend_from_slice(&1000u16.to_le_bytes());
        v.extend_from_slice(&4u16.to_le_bytes());
        let p = TzxPlayer::parse(&v).unwrap();
        assert_eq!(p.scheduled_pulses(), 4);
    }

    #[test]
    fn advance_clears_playing_on_exact_final_pulse_boundary() {
        let mut v = Vec::new();
        v.extend_from_slice(b"ZXTape!");
        v.extend_from_slice(&[0x1a, 1, 20]);
        v.push(0x12); // Pure Tone
        v.extend_from_slice(&1000u16.to_le_bytes());
        v.extend_from_slice(&2u16.to_le_bytes());
        let mut p = TzxPlayer::parse(&v).unwrap();
        assert!(p.playing);
        assert_eq!(p.scheduled_pulses(), 2);
        let _ = p.advance(1000);
        assert!(
            p.playing,
            "exact end of first pulse must not clear playing yet"
        );
        let _ = p.advance(1000);
        assert!(
            !p.playing,
            "exact final pulse boundary must clear playing immediately"
        );
        assert!(p.finished());
        assert!(!p.ear_level(), "EAR must idle low after exact end");
    }

    #[test]
    fn advance_clears_high_ear_on_exact_final_pulse_end() {
        // Pure-tone pulses alternate from initial low; the 2nd pulse is high.
        // Exact end of that high pulse must not leave EAR latched high.
        let mut v = Vec::new();
        v.extend_from_slice(b"ZXTape!");
        v.extend_from_slice(&[0x1a, 1, 20]);
        v.push(0x12);
        v.extend_from_slice(&1000u16.to_le_bytes());
        v.extend_from_slice(&2u16.to_le_bytes());
        let mut p = TzxPlayer::parse(&v).unwrap();
        assert_eq!(p.scheduled_pulses(), 2);
        let _ = p.advance(1000);
        let mid = p.advance(500);
        assert!(mid && p.ear_level(), "second pure-tone pulse should be high");
        let level = p.advance(500);
        assert!(!p.playing);
        assert!(p.finished());
        assert!(!level, "exact end must return low EAR");
        assert!(!p.ear_level(), "exact end must clear latched high EAR");
    }

    #[test]
    fn standard_then_pure_tone_keeps_alternating_levels() {
        let mut v = Vec::new();
        v.extend_from_slice(b"ZXTape!");
        v.extend_from_slice(&[0x1a, 1, 20]);
        v.push(0x10);
        v.extend_from_slice(&0u16.to_le_bytes()); // pause ms (no trailing silence block)
        let payload = [0x00u8, 0x41, 0x00];
        v.extend_from_slice(&(payload.len() as u16).to_le_bytes());
        v.extend_from_slice(&payload);
        v.push(0x12);
        v.extend_from_slice(&500u16.to_le_bytes());
        v.extend_from_slice(&3u16.to_le_bytes());
        let p = TzxPlayer::parse(&v).unwrap();
        for w in p.pulses.windows(2) {
            assert_ne!(
                w[0].1, w[1].1,
                "adjacent TZX pulses must toggle across 0x10→0x12"
            );
        }
    }

    #[test]
    fn standard_then_pulse_sequence_keeps_alternating_levels() {
        let mut v = Vec::new();
        v.extend_from_slice(b"ZXTape!");
        v.extend_from_slice(&[0x1a, 1, 20]);
        v.push(0x10);
        v.extend_from_slice(&0u16.to_le_bytes());
        let payload = [0xffu8, 1, 2, 0];
        v.extend_from_slice(&(payload.len() as u16).to_le_bytes());
        v.extend_from_slice(&payload);
        v.push(0x13);
        v.push(2);
        v.extend_from_slice(&400u16.to_le_bytes());
        v.extend_from_slice(&500u16.to_le_bytes());
        let p = TzxPlayer::parse(&v).unwrap();
        for w in p.pulses.windows(2) {
            assert_ne!(w[0].1, w[1].1, "0x10→0x13 must keep EAR edges");
        }
    }

    #[test]
    fn standard_only_detects_boggit_style() {
        let data = minimal_tzx_standard(&[0x00, 0x41, 0x00]);
        assert!(TzxPlayer::is_standard_speed_only(&data));
        // Optional real-world fixture (never committed).
        let boggit = PathBuf::from("/Users/michael/Downloads/BoggitThe/The Boggit - Side 1.tzx");
        if let Ok(bytes) = std::fs::read(&boggit) {
            assert!(
                TzxPlayer::is_standard_speed_only(&bytes),
                "Boggit Side 1 should be standard-speed 0x10 blocks"
            );
            let tap = TzxPlayer::to_tap_image(&bytes).unwrap();
            assert!(tap.blocks.len() >= 4);
        }
    }

    #[test]
    fn paused_player_does_not_advance() {
        let data = minimal_tzx_standard(&[0xff, 1, 2, 3, 0]);
        let mut p = TzxPlayer::parse(&data).unwrap();
        p.set_playing(false);
        let before = p.scheduled_pulses();
        let _ = p.advance(1_000_000);
        assert_eq!(p.scheduled_pulses(), before);
        assert_eq!(p.block, 0);
    }

    #[test]
    fn active_pulse_counters_are_block_relative() {
        let mut v = Vec::new();
        v.extend_from_slice(b"ZXTape!");
        v.extend_from_slice(&[0x1a, 1, 20]);
        v.push(0x12);
        v.extend_from_slice(&1000u16.to_le_bytes());
        v.extend_from_slice(&4u16.to_le_bytes());
        v.push(0x12);
        v.extend_from_slice(&1000u16.to_le_bytes());
        v.extend_from_slice(&6u16.to_le_bytes());
        let mut p = TzxPlayer::parse(&v).unwrap();
        assert_eq!(p.block_count(), 2);
        assert_eq!(p.active_pulse_index(), 0);
        assert_eq!(p.active_pulse_count(), 4);
        assert_eq!(p.scheduled_pulses(), 10);
        // Step one pulse at a time so we land in block 1 without exhausting the
        // whole schedule (exact final-pulse promotion sets pulse_i == len).
        while p.block == 0 {
            let _ = p.advance(1000);
        }
        assert_eq!(p.block, 1);
        assert_eq!(p.active_pulse_count(), 6);
        assert!(p.active_pulse_index() < p.active_pulse_count());
        assert!(p.playing);
    }

    #[test]
    fn empty_tzx_reports_zero_blocks() {
        let mut v = Vec::new();
        v.extend_from_slice(b"ZXTape!");
        v.extend_from_slice(&[0x1a, 1, 20]);
        let p = TzxPlayer::parse(&v).unwrap();
        assert_eq!(p.block_count(), 0);
        assert_eq!(p.active_pulse_count(), 0);
        assert_eq!(p.scheduled_pulses(), 0);
    }

    /// Minimal ID 0x11 turbo block (custom pilot/sync/bit widths).
    fn minimal_tzx_turbo(payload: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"ZXTape!");
        v.extend_from_slice(&[0x1a, 1, 20]);
        v.push(0x11);
        v.extend_from_slice(&800u16.to_le_bytes()); // pilot
        v.extend_from_slice(&400u16.to_le_bytes()); // sync1
        v.extend_from_slice(&400u16.to_le_bytes()); // sync2
        v.extend_from_slice(&300u16.to_le_bytes()); // zero
        v.extend_from_slice(&600u16.to_le_bytes()); // one
        v.extend_from_slice(&10u16.to_le_bytes()); // pilot pulse count
        v.push(8); // used bits
        v.extend_from_slice(&100u16.to_le_bytes()); // pause ms
        let len = payload.len() as u32;
        v.push((len & 0xff) as u8);
        v.push(((len >> 8) & 0xff) as u8);
        v.push(((len >> 16) & 0xff) as u8);
        v.extend_from_slice(payload);
        v
    }

    #[test]
    fn turbo_block_schedules_custom_pulse_widths() {
        let payload = [0xffu8, 0x00]; // one byte of 1-bits, one of 0-bits
        let data = minimal_tzx_turbo(&payload);
        let p = TzxPlayer::parse(&data).unwrap();
        // 10 pilot + sync1 + sync2 + 8*2 bits * 2 bytes + pause = 45
        assert_eq!(p.scheduled_pulses(), 45);
        assert_eq!(p.pulses[0].0, 800);
        assert_eq!(p.pulses[9].0, 800);
        assert_eq!(p.pulses[10].0, 400); // sync1
        assert_eq!(p.pulses[11].0, 400); // sync2
        assert_eq!(p.pulses[12].0, 600); // first 1-bit half
        assert_eq!(p.pulses[12 + 16].0, 300);
        assert!(!TzxPlayer::is_standard_speed_only(&data));
    }

    #[test]
    fn to_tap_image_skips_turbo_blocks() {
        let data = minimal_tzx_turbo(&[0xff, 1, 2, 0]);
        let tap = TzxPlayer::to_tap_image(&data).unwrap();
        assert!(
            tap.blocks.is_empty(),
            "flash/TAP path must not extract ID 0x11 payloads"
        );
    }

    #[test]
    fn zero_pulse_blocks_sync_logical_index() {
        // 0x12 count=0, then a real tone, then 0x20 pause_ms=0 trailing.
        let mut v = Vec::new();
        v.extend_from_slice(b"ZXTape!");
        v.extend_from_slice(&[0x1a, 1, 20]);
        v.push(0x12);
        v.extend_from_slice(&1000u16.to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes());
        v.push(0x12);
        v.extend_from_slice(&1000u16.to_le_bytes());
        v.extend_from_slice(&3u16.to_le_bytes());
        v.push(0x20);
        v.extend_from_slice(&0u16.to_le_bytes());
        let mut p = TzxPlayer::parse(&v).unwrap();
        assert_eq!(p.block_count(), 3);
        // Leading zero-pulse tone shares pulse index 0 with the next block.
        assert_eq!(p.block, 1);
        assert_eq!(p.active_pulse_count(), 3);
        assert_eq!(p.active_pulse_index(), 0);
        while p.pulse_i < p.scheduled_pulses() {
            let _ = p.advance(10_000);
        }
        // Trailing pause-with-zero-duration is reachable at end-of-schedule.
        assert_eq!(p.block, 2);
        assert_eq!(p.active_pulse_count(), 0);
    }
}
