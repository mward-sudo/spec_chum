//! TZX tape image parsing and EAR pulse playback.
//!
//! Supported block IDs for playback: 0x10, 0x11, 0x12, 0x13, 0x14, 0x15,
//! 0x18, 0x19, 0x20, plus Loop Start/End (`0x24` / `0x25`), Jump/Call/Return/Select
//! (`0x23`, `0x26`–`0x28`), Stop-if-48K (`0x2A`, accepted as a no-op at expand —
//! model-aware stop is not applied during schedule build), and Set signal level
//! (`0x2B`). Select auto-picks the first entry for headless open.
//! Informational / skip blocks: 0x21, 0x22, 0x30, 0x32, 0x33, 0x35, 0x5A.

use std::io::Read;
use std::path::Path;

use flate2::read::ZlibDecoder;
use thiserror::Error;

use crate::{
    TapImage, TapeError, BIT0_T, BIT1_T, PAUSE_T, PILOT_DATA_PULSES, PILOT_HEADER_PULSES,
    PILOT_PULSE_T, SYNC1_T, SYNC2_T,
};

/// Cap on expanded EAR pulse count while parsing (incl. Loop Start/End replay).
/// Prevents pathological `0x24`/`0x25` nesting from unbounded `Vec` growth.
const MAX_SCHEDULED_PULSES: usize = 16_777_216;
/// Cap on logical block starts (`block_starts`) under the same expansion path.
const MAX_LOGICAL_BLOCKS: usize = 1_048_576;
/// Cap on Loop End → Loop Start replay jumps (empty nested loops add no pulses).
const MAX_LOOP_REPLAYS: usize = 1_048_576;
/// Cap on Jump / Call / Select control-flow transfers while expanding.
const MAX_CONTROL_TRANSFERS: usize = 1_048_576;

/// Open Call sequence (`0x26`) frame — nesting of Call blocks is forbidden by the TZX spec.
struct CallFrame {
    /// Block index of the Call sequence (relative offsets are from this block).
    call_bi: usize,
    /// Absolute block index to resume after all calls finish (`call_bi + 1`).
    return_bi: usize,
    /// Relative offsets from the Call block.
    offsets: Vec<i16>,
    /// Index of the *next* call to run after the current subroutine Returns.
    next_call: usize,
}

#[derive(Debug, Error)]
pub enum TzxError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Format(String),
}

fn ensure_pulse_budget(pulses: &[(u32, bool)]) -> Result<(), TzxError> {
    if pulses.len() > MAX_SCHEDULED_PULSES {
        return Err(TzxError::Format(format!(
            "TZX pulse schedule exceeded {MAX_SCHEDULED_PULSES} pulses (loop expansion?)"
        )));
    }
    Ok(())
}

fn ensure_pulse_room(pulses: &[(u32, bool)], additional: usize) -> Result<(), TzxError> {
    let Some(total) = pulses.len().checked_add(additional) else {
        return Err(TzxError::Format(
            "TZX pulse schedule size overflow (loop expansion?)".into(),
        ));
    };
    if total > MAX_SCHEDULED_PULSES {
        return Err(TzxError::Format(format!(
            "TZX pulse schedule exceeded {MAX_SCHEDULED_PULSES} pulses (loop expansion?)"
        )));
    }
    Ok(())
}

fn ensure_block_budget(block_starts: &[usize]) -> Result<(), TzxError> {
    if block_starts.len() >= MAX_LOGICAL_BLOCKS {
        return Err(TzxError::Format(format!(
            "TZX logical block count exceeded {MAX_LOGICAL_BLOCKS} (loop expansion?)"
        )));
    }
    Ok(())
}

/// Advance `i` past the body of a known TZX block (`id` already consumed).
///
/// Used to build the block-offset index for Jump/Call/Select relative addressing.
fn advance_past_tzx_block_body(data: &[u8], id: u8, mut i: usize) -> Result<usize, TzxError> {
    match id {
        0x10 => {
            if i + 4 > data.len() {
                return Err(TzxError::Format("truncated 0x10".into()));
            }
            let len = u16::from_le_bytes([data[i + 2], data[i + 3]]) as usize;
            i += 4 + len;
        }
        0x11 => {
            if i + 18 > data.len() {
                return Err(TzxError::Format("truncated 0x11".into()));
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
                return Err(TzxError::Format("truncated 0x14".into()));
            }
            let len = (u32::from(data[i + 7])
                | (u32::from(data[i + 8]) << 8)
                | (u32::from(data[i + 9]) << 16)) as usize;
            i += 10 + len;
        }
        0x15 => {
            if i + 8 > data.len() {
                return Err(TzxError::Format("truncated 0x15".into()));
            }
            let len = (u32::from(data[i + 5])
                | (u32::from(data[i + 6]) << 8)
                | (u32::from(data[i + 7]) << 16)) as usize;
            i += 8 + len;
        }
        0x18 | 0x19 => {
            if i + 4 > data.len() {
                return Err(TzxError::Format(format!("truncated 0x{id:02x}")));
            }
            let len = u32::from_le_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]) as usize;
            i += 4 + len;
        }
        0x20 => i += 2,
        0x21 => {
            let n = data.get(i).copied().unwrap_or(0) as usize;
            i += 1 + n;
        }
        0x22 | 0x25 | 0x27 => {}
        0x23 => i += 2,
        0x24 => i += 2,
        0x26 => {
            if i + 2 > data.len() {
                return Err(TzxError::Format("truncated 0x26".into()));
            }
            let n = u16::from_le_bytes([data[i], data[i + 1]]) as usize;
            i += 2 + n * 2;
        }
        0x28 => {
            if i + 2 > data.len() {
                return Err(TzxError::Format("truncated 0x28".into()));
            }
            let len = u16::from_le_bytes([data[i], data[i + 1]]) as usize;
            i += 2 + len;
        }
        0x2a => i += 4,
        0x2b => i += 5,
        0x30 => {
            let n = data.get(i).copied().unwrap_or(0) as usize;
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
            let n = data.get(i).copied().unwrap_or(0) as usize;
            i += 1 + n * 3;
        }
        0x35 => {
            if i + 0x14 > data.len() {
                return Err(TzxError::Format("truncated 0x35".into()));
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
        other => {
            return Err(TzxError::Format(format!(
                "unsupported TZX block ID 0x{other:02x} at offset {}",
                i.saturating_sub(1)
            )));
        }
    }
    if i > data.len() {
        return Err(TzxError::Format(format!(
            "truncated TZX block ID 0x{id:02x}"
        )));
    }
    Ok(i)
}

/// Byte offsets of each top-level block's ID byte (for relative Jump/Call/Select).
fn index_tzx_blocks(data: &[u8]) -> Result<Vec<usize>, TzxError> {
    let mut offsets = Vec::new();
    let mut i = 10usize;
    while i < data.len() {
        offsets.push(i);
        let id = data[i];
        i = advance_past_tzx_block_body(data, id, i + 1)?;
    }
    Ok(offsets)
}

fn resolve_relative_block(
    from_bi: usize,
    relative: i16,
    block_count: usize,
    kind: &str,
) -> Result<usize, TzxError> {
    if relative == 0 {
        return Err(TzxError::Format(format!(
            "TZX {kind} relative 0 is an infinite loop"
        )));
    }
    let target = i32::try_from(from_bi)
        .ok()
        .and_then(|b| b.checked_add(i32::from(relative)))
        .ok_or_else(|| TzxError::Format(format!("TZX {kind} target overflow")))?;
    // `block_count` means end-of-tape (Jump past the last block to skip a trailing
    // subroutine — common after Call/Return resume).
    if target < 0 || target as usize > block_count {
        return Err(TzxError::Format(format!(
            "TZX {kind} target out of range (from {from_bi} + {relative}, {block_count} blocks)"
        )));
    }
    Ok(target as usize)
}

fn bump_control_transfers(steps: &mut usize) -> Result<(), TzxError> {
    *steps = steps
        .checked_add(1)
        .ok_or_else(|| TzxError::Format("TZX control-flow transfer counter overflow".into()))?;
    if *steps > MAX_CONTROL_TRANSFERS {
        return Err(TzxError::Format(format!(
            "TZX control-flow transfer count exceeded {MAX_CONTROL_TRANSFERS}"
        )));
    }
    Ok(())
}

/// Pulse count for a trailing TZX pause (0 = ignored; else 1 pulse).
///
/// Fuse emits the whole pause as a single edge (`do_tail_pause`) then forces the
/// next block low (`END_OF_BLOCK_NEXT_LOW`).
fn estimate_tzx_pause_pulses(pause_t: u32) -> usize {
    usize::from(pause_t > 0)
}

/// Pulse count for a pure-data payload, matching [`append_pure_data`] emission.
fn estimate_pure_data_pulses(block_len: usize, used_bits: u8, pause_ms: u16) -> Option<usize> {
    let used_bits = if used_bits == 0 || used_bits > 8 {
        8
    } else {
        used_bits
    };
    let data = if block_len == 0 {
        0
    } else {
        let full_bytes = block_len.saturating_sub(1);
        let full = full_bytes.checked_mul(16)?;
        let last = usize::from(used_bits).checked_mul(2)?;
        full.checked_add(last)?
    };
    data.checked_add(estimate_tzx_pause_pulses(ms_to_t(pause_ms)))
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
        // Fuse starts tapes with force_low_level; non-zero pauses re-arm it.
        let mut force_low = true;
        // Relative Jump/Call/Select need a stable block index (#374).
        let block_offsets = index_tzx_blocks(data)?;
        let mut bi = 0usize;
        // Loop Start (0x24) / Loop End (0x25): replay the enclosed section N times.
        // `skip_depth` covers reps==0 (play zero times) and nested skips.
        // Stack entries are (restart_block_index, remaining_reps).
        let mut loop_stack: Vec<(usize, u16)> = Vec::new();
        let mut call_stack: Vec<CallFrame> = Vec::new();
        let mut skip_depth: usize = 0;
        let mut loop_replays: usize = 0;
        let mut control_transfers: usize = 0;
        // `i` is a per-arm decode cursor; after each arm we only need `next_bi`.
        // Clippy still flags trailing `i += …` in skip/info arms as unused_assignments.
        #[allow(unused_assignments)]
        while bi < block_offsets.len() {
            let id_off = block_offsets[bi];
            let id = data[id_off];
            let mut i = id_off + 1;
            let mut next_bi = bi + 1;
            let emit = skip_depth == 0;
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
                    if emit {
                        ensure_block_budget(&block_starts)?;
                        let flag = block.first().copied().unwrap_or(0);
                        let pilot_count = if flag == 0 {
                            PILOT_HEADER_PULSES
                        } else {
                            PILOT_DATA_PULSES
                        };
                        // Standard 0x10 always emits a trailing pause pulse (PAUSE_T if ms==0).
                        let data_pulses = block
                            .len()
                            .checked_mul(16)
                            .and_then(|n| n.checked_add(1))
                            .ok_or_else(|| {
                                TzxError::Format("TZX 0x10 pulse estimate overflow".into())
                            })?;
                        let additional = (pilot_count as usize)
                            .checked_add(2)
                            .and_then(|n| n.checked_add(data_pulses))
                            .ok_or_else(|| {
                                TzxError::Format("TZX 0x10 pulse estimate overflow".into())
                            })?;
                        ensure_pulse_room(&pulses, additional)?;
                        block_starts.push(pulses.len());
                        append_standard_block(
                            &mut pulses,
                            &mut level,
                            &mut force_low,
                            block,
                            pause_ms,
                        );
                        ensure_pulse_budget(&pulses)?;
                    }
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
                    if emit {
                        ensure_block_budget(&block_starts)?;
                        let data_pulses =
                            estimate_pure_data_pulses(block.len(), used_bits, pause_ms)
                                .ok_or_else(|| {
                                    TzxError::Format("TZX 0x11 pulse estimate overflow".into())
                                })?;
                        let additional = usize::from(pilot_pulses)
                            .checked_add(2)
                            .and_then(|n| n.checked_add(data_pulses))
                            .ok_or_else(|| {
                                TzxError::Format("TZX 0x11 pulse estimate overflow".into())
                            })?;
                        ensure_pulse_room(&pulses, additional)?;
                        block_starts.push(pulses.len());
                        append_turbo_block(
                            &mut pulses,
                            &mut level,
                            &mut force_low,
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
                        ensure_pulse_budget(&pulses)?;
                    }
                }
                0x12 => {
                    if i + 4 > data.len() {
                        return Err(TzxError::Format("truncated 0x12".into()));
                    }
                    let len = u16::from_le_bytes([data[i], data[i + 1]]);
                    let count = u16::from_le_bytes([data[i + 2], data[i + 3]]);
                    i += 4;
                    if emit {
                        ensure_block_budget(&block_starts)?;
                        ensure_pulse_room(&pulses, usize::from(count))?;
                        block_starts.push(pulses.len());
                        for _ in 0..count {
                            push_pulse_tzx(&mut pulses, &mut level, &mut force_low, u32::from(len));
                        }
                        ensure_pulse_budget(&pulses)?;
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
                    if emit {
                        ensure_block_budget(&block_starts)?;
                        ensure_pulse_room(&pulses, n)?;
                        block_starts.push(pulses.len());
                        for _ in 0..n {
                            let len = u16::from_le_bytes([data[i], data[i + 1]]);
                            i += 2;
                            push_pulse_tzx(&mut pulses, &mut level, &mut force_low, u32::from(len));
                        }
                        ensure_pulse_budget(&pulses)?;
                    } else {
                        i += n * 2;
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
                    if emit {
                        ensure_block_budget(&block_starts)?;
                        let data_pulses =
                            estimate_pure_data_pulses(block.len(), used_bits, pause_ms)
                                .ok_or_else(|| {
                                    TzxError::Format("TZX 0x14 pulse estimate overflow".into())
                                })?;
                        ensure_pulse_room(&pulses, data_pulses)?;
                        block_starts.push(pulses.len());
                        append_pure_data(
                            &mut pulses,
                            &mut level,
                            &mut force_low,
                            block,
                            zero,
                            one,
                            used_bits,
                            pause_ms,
                        );
                        ensure_pulse_budget(&pulses)?;
                    }
                }
                0x15 => {
                    // Direct recording — absolute EAR samples (bit 0=low, 1=high).
                    if i + 8 > data.len() {
                        return Err(TzxError::Format("truncated 0x15".into()));
                    }
                    let t_per_sample = u16::from_le_bytes([data[i], data[i + 1]]);
                    let pause_ms = u16::from_le_bytes([data[i + 2], data[i + 3]]);
                    let used_bits = data[i + 4];
                    let len = (u32::from(data[i + 5])
                        | (u32::from(data[i + 6]) << 8)
                        | (u32::from(data[i + 7]) << 16)) as usize;
                    i += 8;
                    if i + len > data.len() {
                        return Err(TzxError::Format("0x15 data truncated".into()));
                    }
                    let samples = &data[i..i + len];
                    i += len;
                    if emit {
                        ensure_block_budget(&block_starts)?;
                        let sample_pulses = estimate_direct_samples(samples.len(), used_bits)
                            .ok_or_else(|| {
                                TzxError::Format("TZX 0x15 pulse estimate overflow".into())
                            })?;
                        let pause_pulses = estimate_tzx_pause_pulses(ms_to_t(pause_ms));
                        let additional =
                            sample_pulses.checked_add(pause_pulses).ok_or_else(|| {
                                TzxError::Format("TZX 0x15 pulse estimate overflow".into())
                            })?;
                        ensure_pulse_room(&pulses, additional)?;
                        block_starts.push(pulses.len());
                        append_direct_recording(
                            &mut pulses,
                            &mut level,
                            &mut force_low,
                            samples,
                            u32::from(t_per_sample),
                            used_bits,
                            pause_ms,
                        )?;
                        ensure_pulse_budget(&pulses)?;
                    }
                }
                0x18 => {
                    // CSW recording (RLE / Z-RLE) — alternating absolute pulses.
                    if i + 4 > data.len() {
                        return Err(TzxError::Format("truncated 0x18".into()));
                    }
                    let body_len =
                        u32::from_le_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]])
                            as usize;
                    i += 4;
                    if body_len < 10 || i + body_len > data.len() {
                        return Err(TzxError::Format("truncated 0x18 body".into()));
                    }
                    let body = &data[i..i + body_len];
                    i += body_len;
                    if emit {
                        let pause_ms = u16::from_le_bytes([body[0], body[1]]);
                        let sample_rate = u32::from(body[2])
                            | (u32::from(body[3]) << 8)
                            | (u32::from(body[4]) << 16);
                        let compression = body[5];
                        let stored_pulses =
                            u32::from_le_bytes([body[6], body[7], body[8], body[9]]);
                        let csw_data = &body[10..];
                        ensure_block_budget(&block_starts)?;
                        let budget = (stored_pulses as usize)
                            .saturating_add(estimate_tzx_pause_pulses(ms_to_t(pause_ms)));
                        ensure_pulse_room(&pulses, budget)?;
                        block_starts.push(pulses.len());
                        append_csw_recording(
                            &mut pulses,
                            &mut level,
                            &mut force_low,
                            csw_data,
                            sample_rate,
                            compression,
                            stored_pulses,
                            pause_ms,
                        )?;
                        ensure_pulse_budget(&pulses)?;
                    }
                }
                0x19 => {
                    // Generalized data block — symbol alphabet + pilot/data streams.
                    if i + 4 > data.len() {
                        return Err(TzxError::Format("truncated 0x19".into()));
                    }
                    let body_len =
                        u32::from_le_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]])
                            as usize;
                    i += 4;
                    if body_len < 14 || i + body_len > data.len() {
                        return Err(TzxError::Format("truncated 0x19 body".into()));
                    }
                    let body = &data[i..i + body_len];
                    i += body_len;
                    if emit {
                        ensure_block_budget(&block_starts)?;
                        // Worst-case pulse count is hard to bound tightly; room-check
                        // incrementally inside the append helper.
                        block_starts.push(pulses.len());
                        append_generalized_data(&mut pulses, &mut level, &mut force_low, body)?;
                        ensure_pulse_budget(&pulses)?;
                    }
                }
                0x20 => {
                    if i + 2 > data.len() {
                        return Err(TzxError::Format("truncated 0x20".into()));
                    }
                    let pause_ms = u16::from_le_bytes([data[i], data[i + 1]]);
                    i += 2;
                    if emit {
                        ensure_block_budget(&block_starts)?;
                        let t = ms_to_t(pause_ms);
                        let pause_pulses = estimate_tzx_pause_pulses(t);
                        if pause_pulses > 0 {
                            ensure_pulse_room(&pulses, pause_pulses)?;
                        }
                        block_starts.push(pulses.len());
                        append_tzx_pause(&mut pulses, &mut level, &mut force_low, t);
                        ensure_pulse_budget(&pulses)?;
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
                0x23 => {
                    // Jump to block — relative signed word from this block's index.
                    if i + 2 > data.len() {
                        return Err(TzxError::Format("truncated 0x23".into()));
                    }
                    let relative = i16::from_le_bytes([data[i], data[i + 1]]);
                    if emit {
                        bump_control_transfers(&mut control_transfers)?;
                        next_bi = resolve_relative_block(
                            bi,
                            relative,
                            block_offsets.len(),
                            "Jump (0x23)",
                        )?;
                    }
                }
                0x24 => {
                    // Loop Start — repetitions N means play enclosed blocks N times.
                    if i + 2 > data.len() {
                        return Err(TzxError::Format("truncated 0x24".into()));
                    }
                    let reps = u16::from_le_bytes([data[i], data[i + 1]]);
                    if skip_depth > 0 {
                        skip_depth = skip_depth.checked_add(1).ok_or_else(|| {
                            TzxError::Format("TZX skip-depth overflow (nested Loop Start)".into())
                        })?;
                    } else if reps == 0 {
                        skip_depth = 1;
                    } else {
                        // Restart at the first enclosed block (bi + 1).
                        loop_stack.push((bi + 1, reps));
                    }
                }
                0x25 => {
                    // Loop End — jump back to matching Loop Start until N plays done.
                    if skip_depth > 0 {
                        skip_depth -= 1;
                    } else if let Some((restart_bi, remaining)) = loop_stack.last_mut() {
                        *remaining = remaining.saturating_sub(1);
                        if *remaining > 0 {
                            ensure_pulse_budget(&pulses)?;
                            loop_replays = loop_replays.checked_add(1).ok_or_else(|| {
                                TzxError::Format("TZX loop replay counter overflow".into())
                            })?;
                            if loop_replays > MAX_LOOP_REPLAYS {
                                return Err(TzxError::Format(format!(
                                    "TZX loop replay count exceeded {MAX_LOOP_REPLAYS}"
                                )));
                            }
                            next_bi = *restart_bi;
                        } else {
                            loop_stack.pop();
                        }
                    } else {
                        return Err(TzxError::Format(
                            "TZX Loop End (0x25) without matching Loop Start".into(),
                        ));
                    }
                }
                0x26 => {
                    // Call sequence — non-nestable; relatives from this Call block.
                    if i + 2 > data.len() {
                        return Err(TzxError::Format("truncated 0x26".into()));
                    }
                    let n = u16::from_le_bytes([data[i], data[i + 1]]) as usize;
                    i += 2;
                    if i + n * 2 > data.len() {
                        return Err(TzxError::Format("truncated 0x26 call list".into()));
                    }
                    let mut offsets = Vec::with_capacity(n);
                    for _ in 0..n {
                        offsets.push(i16::from_le_bytes([data[i], data[i + 1]]));
                        i += 2;
                    }
                    if !emit {
                        // Skipping (zero-rep loop body): ignore control flow.
                    } else if offsets.is_empty() {
                        // N=0 → fall through.
                    } else {
                        if !call_stack.is_empty() {
                            return Err(TzxError::Format(
                                "TZX nested Call sequence (0x26) is not allowed".into(),
                            ));
                        }
                        bump_control_transfers(&mut control_transfers)?;
                        let first = offsets[0];
                        call_stack.push(CallFrame {
                            call_bi: bi,
                            return_bi: bi + 1,
                            offsets,
                            next_call: 1,
                        });
                        next_bi =
                            resolve_relative_block(bi, first, block_offsets.len(), "Call (0x26)")?;
                    }
                }
                0x27 => {
                    // Return from sequence.
                    if emit {
                        let (call_bi, return_bi, next_relative) = {
                            let frame = call_stack.last().ok_or_else(|| {
                                TzxError::Format(
                                    "TZX Return (0x27) without matching Call sequence".into(),
                                )
                            })?;
                            (
                                frame.call_bi,
                                frame.return_bi,
                                frame.offsets.get(frame.next_call).copied(),
                            )
                        };
                        bump_control_transfers(&mut control_transfers)?;
                        if let Some(relative) = next_relative {
                            call_stack.last_mut().expect("call frame present").next_call += 1;
                            next_bi = resolve_relative_block(
                                call_bi,
                                relative,
                                block_offsets.len(),
                                "Call (0x26)",
                            )?;
                        } else {
                            call_stack.pop();
                            next_bi = return_bi;
                        }
                    }
                }
                0x28 => {
                    // Select block — headless open auto-picks the first selection (#374).
                    if i + 2 > data.len() {
                        return Err(TzxError::Format("truncated 0x28".into()));
                    }
                    let len = u16::from_le_bytes([data[i], data[i + 1]]) as usize;
                    i += 2;
                    if i + len > data.len() {
                        return Err(TzxError::Format("truncated 0x28 body".into()));
                    }
                    let body = &data[i..i + len];
                    if emit {
                        if body.is_empty() {
                            // No selections — fall through.
                        } else {
                            let n = body[0];
                            if n == 0 || body.len() < 3 {
                                // Empty menu — fall through.
                            } else {
                                let relative = i16::from_le_bytes([body[1], body[2]]);
                                bump_control_transfers(&mut control_transfers)?;
                                next_bi = resolve_relative_block(
                                    bi,
                                    relative,
                                    block_offsets.len(),
                                    "Select (0x28)",
                                )?;
                            }
                        }
                    }
                }
                0x2a => {
                    // Stop the tape if in 48K mode — accepted as a no-op while
                    // building the EAR schedule (no machine model at parse).
                    // Open/`has_tape` succeed; runtime model-aware stop can deepen later.
                    if i + 4 > data.len() {
                        return Err(TzxError::Format("truncated 0x2a".into()));
                    }
                    i += 4;
                }
                0x2b => {
                    // Set signal level — absolute EAR polarity for the next pulse.
                    if i + 5 > data.len() {
                        return Err(TzxError::Format("truncated 0x2b".into()));
                    }
                    let signal_high = data[i + 4] != 0;
                    i += 5;
                    if emit {
                        force_low = false;
                        level = signal_high;
                    }
                }
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
                        "unsupported TZX block ID 0x{other:02x} at offset {id_off}"
                    )));
                }
            }
            bi = next_bi;
        }
        if skip_depth != 0 || !loop_stack.is_empty() {
            return Err(TzxError::Format(
                "TZX Loop Start (0x24) without matching Loop End".into(),
            ));
        }
        if !call_stack.is_empty() {
            return Err(TzxError::Format(
                "TZX Call sequence (0x26) without matching Return (0x27)".into(),
            ));
        }
        let playing = !pulses.is_empty();
        let mut player = Self {
            pulses,
            pulse_i: 0,
            remain: 0,
            level: false,
            // Empty / pulse-free decks are already finished; don't report playing
            // until the first advance() clears the flag (#79 / CR outside-diff).
            playing,
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
                0x11..=0x15 | 0x18 | 0x19 => return false,
                0x20 => i += 2,
                0x21 => {
                    let n = data.get(i).copied().unwrap_or(0) as usize;
                    i += 1 + n;
                }
                0x22 => {}
                // Jump / Call / Select / Return are not flash/TAP-convertible: control
                // flow must expand into the EAR schedule (#374).
                0x23 | 0x26 | 0x27 | 0x28 => return false,
                // Loop markers are not flash/TAP-convertible: expanding them into
                // EAR pulses is required (reps>1), and TAP extraction would drop
                // repetitions. Keep off the standard-speed-only path (#372 CR).
                0x24 | 0x25 => return false,
                // Stop-if-48K / Set signal are EAR-schedule concerns, not TAP.
                0x2a => i += 4,
                0x2b => i += 5,
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
                0x15 => {
                    // Direct recording is not TAP-extractable — fail loudly (#374).
                    return Err(TzxError::Format(format!(
                        "unsupported TZX block ID 0x15 at offset {} (direct recording — use EAR)",
                        i - 1
                    )));
                }
                0x18 | 0x19 => {
                    return Err(TzxError::Format(format!(
                        "unsupported TZX block ID 0x{id:02x} at offset {} (pulse recording — use EAR)",
                        i - 1
                    )));
                }
                0x20 => i += 2,
                0x21 => {
                    let n = data.get(i).copied().unwrap_or(0) as usize;
                    i += 1 + n;
                }
                0x22 => {}
                0x24 => i += 2,
                0x25 => {}
                // Control-flow blocks are not linear TAP extractable — fail loudly
                // rather than walking past Jump/Call/Select in file order (#374).
                0x23 | 0x26 | 0x27 | 0x28 => {
                    return Err(TzxError::Format(format!(
                        "unsupported TZX block ID 0x{id:02x} at offset {} (control flow — use EAR)",
                        i - 1
                    )));
                }
                0x2a => i += 4,
                0x2b => i += 5,
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
                // Do not silently truncate: an unknown ID after a leading 0x10
                // used to drop trailing standard blocks (#374 / Arkanoid-class miss).
                other => {
                    return Err(TzxError::Format(format!(
                        "unsupported TZX block ID 0x{other:02x} at offset {}",
                        i - 1
                    )));
                }
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

/// Emit one pulse. When `force_low` is set (tape start / after a non-zero pause),
/// mimic Fuse `LEVEL_LOW`: the edge is absolute low, then the next edge is high.
///
/// Fuse sets `tape_microphone = 0` on that flag. Spec Chum stores the same sense
/// as Fuse's mic (OR'd into ULA bit 6); with the speaker bit clear, Fuse's
/// `r ^= mic` against `0xbf` also yields `ear_in` == mic.
///
/// Important: do **not** emit `*level` then pin low — when `*level` is already
/// false that schedules two lows and inverts the rest of the tape (#379).
fn push_pulse_tzx(
    pulses: &mut Vec<(u32, bool)>,
    level: &mut bool,
    force_low: &mut bool,
    duration: u32,
) {
    if *force_low {
        pulses.push((duration, false));
        // Next `push_pulse` emits high then flips — same as after a normal low.
        *level = true;
        *force_low = false;
    } else {
        crate::push_pulse(pulses, level, duration);
    }
}

/// Emit a TZX pause / data-block tail silence (Fuse `do_tail_pause`).
///
/// One edge for the full duration; the next playable block re-arms
/// `force_low` so its first edge gets Fuse `LEVEL_LOW` semantics.
fn append_tzx_pause(
    pulses: &mut Vec<(u32, bool)>,
    level: &mut bool,
    force_low: &mut bool,
    pause_t: u32,
) {
    if pause_t == 0 {
        return;
    }
    crate::push_pulse(pulses, level, pause_t);
    *force_low = true;
}

fn append_standard_block(
    pulses: &mut Vec<(u32, bool)>,
    level: &mut bool,
    force_low: &mut bool,
    block: &[u8],
    pause_ms: u16,
) {
    let flag = block.first().copied().unwrap_or(0);
    let pilot_count = if flag == 0 {
        PILOT_HEADER_PULSES
    } else {
        PILOT_DATA_PULSES
    };
    for _ in 0..pilot_count {
        push_pulse_tzx(pulses, level, force_low, PILOT_PULSE_T);
    }
    push_pulse_tzx(pulses, level, force_low, SYNC1_T);
    push_pulse_tzx(pulses, level, force_low, SYNC2_T);
    for &byte in block {
        for bit in (0..8).rev() {
            let one = byte & (1 << bit) != 0;
            let len = if one { BIT1_T } else { BIT0_T };
            push_pulse_tzx(pulses, level, force_low, len);
            push_pulse_tzx(pulses, level, force_low, len);
        }
    }
    let pause = if pause_ms == 0 {
        PAUSE_T
    } else {
        ms_to_t(pause_ms)
    };
    append_tzx_pause(pulses, level, force_low, pause);
}

// Pilot/sync/zero/one timings + pause are one TZX turbo block (#171).
#[allow(clippy::too_many_arguments)]
fn append_turbo_block(
    pulses: &mut Vec<(u32, bool)>,
    level: &mut bool,
    force_low: &mut bool,
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
    for _ in 0..pilot_pulses {
        push_pulse_tzx(pulses, level, force_low, u32::from(pilot));
    }
    push_pulse_tzx(pulses, level, force_low, u32::from(sync1));
    push_pulse_tzx(pulses, level, force_low, u32::from(sync2));
    append_pure_data(
        pulses, level, force_low, block, zero, one, used_bits, pause_ms,
    );
}

fn append_pure_data(
    pulses: &mut Vec<(u32, bool)>,
    level: &mut bool,
    force_low: &mut bool,
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
        // TZX: last-byte `used_bits` are the *most significant* bits
        // (e.g. used=6 → `xxxxxx00`). Emit MSB-first like a full byte.
        let bit_hi = 7u8;
        let bit_lo = bit_hi + 1 - bits;
        for bit in (bit_lo..=bit_hi).rev() {
            let is_one = byte & (1 << bit) != 0;
            let len = if is_one { one } else { zero };
            push_pulse_tzx(pulses, level, force_low, u32::from(len));
            push_pulse_tzx(pulses, level, force_low, u32::from(len));
        }
    }
    append_tzx_pause(pulses, level, force_low, ms_to_t(pause_ms));
}

/// Upper bound on Direct-recording sample pulses (one per bit before merge).
fn estimate_direct_samples(byte_len: usize, used_bits: u8) -> Option<usize> {
    if byte_len == 0 {
        return Some(0);
    }
    let used = if used_bits == 0 || used_bits > 8 {
        8
    } else {
        used_bits
    };
    let full = byte_len.saturating_sub(1).checked_mul(8)?;
    full.checked_add(usize::from(used))
}

/// Emit an absolute EAR level for `duration` T-states, merging with the prior
/// pulse when the level matches (Direct / CSW / GDB force-polarity paths).
fn push_absolute_level(
    pulses: &mut Vec<(u32, bool)>,
    level: &mut bool,
    force_low: &mut bool,
    ear_high: bool,
    duration: u32,
) -> Result<(), TzxError> {
    if duration == 0 {
        return Ok(());
    }
    *force_low = false;
    if let Some(last) = pulses.last_mut() {
        if last.1 == ear_high {
            last.0 = last.0.saturating_add(duration);
            *level = !ear_high;
            return Ok(());
        }
    }
    ensure_pulse_room(pulses, 1)?;
    pulses.push((duration, ear_high));
    *level = !ear_high;
    Ok(())
}

fn append_direct_recording(
    pulses: &mut Vec<(u32, bool)>,
    level: &mut bool,
    force_low: &mut bool,
    samples: &[u8],
    t_per_sample: u32,
    used_bits: u8,
    pause_ms: u16,
) -> Result<(), TzxError> {
    if samples.is_empty() || t_per_sample == 0 {
        append_tzx_pause(pulses, level, force_low, ms_to_t(pause_ms));
        return Ok(());
    }
    let used_bits = if used_bits == 0 || used_bits > 8 {
        8
    } else {
        used_bits
    };
    *force_low = false;
    for (bi, &byte) in samples.iter().enumerate() {
        let bits = if bi + 1 == samples.len() {
            used_bits
        } else {
            8
        };
        let bit_hi = 7u8;
        let bit_lo = bit_hi + 1 - bits;
        for bit in (bit_lo..=bit_hi).rev() {
            let ear_high = byte & (1 << bit) != 0;
            push_absolute_level(pulses, level, force_low, ear_high, t_per_sample)?;
        }
    }
    append_tzx_pause(pulses, level, force_low, ms_to_t(pause_ms));
    Ok(())
}

fn decode_csw_rle(data: &[u8]) -> Result<Vec<u32>, TzxError> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < data.len() {
        let b = data[i];
        i += 1;
        let samples = if b == 0 {
            if i + 4 > data.len() {
                return Err(TzxError::Format("truncated CSW RLE dword".into()));
            }
            let n = u32::from_le_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]);
            i += 4;
            n
        } else {
            u32::from(b)
        };
        out.push(samples);
    }
    Ok(out)
}

fn append_csw_recording(
    pulses: &mut Vec<(u32, bool)>,
    level: &mut bool,
    force_low: &mut bool,
    csw_data: &[u8],
    sample_rate: u32,
    compression: u8,
    stored_pulses: u32,
    pause_ms: u16,
) -> Result<(), TzxError> {
    if sample_rate == 0 {
        return Err(TzxError::Format("TZX CSW sample rate is 0".into()));
    }
    // T-states per CSW sample ≈ 3_500_000 / rate (Fuse / CLK convention).
    let t_per_sample = 3_500_000u32 / sample_rate;
    if t_per_sample == 0 {
        return Err(TzxError::Format("TZX CSW sample rate too high".into()));
    }
    let raw = match compression {
        0x01 => csw_data.to_vec(),
        0x02 => {
            // Bound inflate so a crafted stream cannot exhaust memory before
            // RLE/pulse-budget checks (#374 CR).
            const MAX_CSW_PLAIN: u64 = (MAX_SCHEDULED_PULSES as u64).saturating_mul(5);
            let mut zlib = ZlibDecoder::new(csw_data).take(MAX_CSW_PLAIN + 1);
            let mut plain = Vec::new();
            zlib.read_to_end(&mut plain)
                .map_err(|e| TzxError::Format(format!("TZX CSW Z-RLE inflate failed: {e}")))?;
            if plain.len() as u64 > MAX_CSW_PLAIN {
                return Err(TzxError::Format("TZX CSW Z-RLE stream too large".into()));
            }
            plain
        }
        other => {
            return Err(TzxError::Format(format!(
                "unsupported TZX CSW compression 0x{other:02x}"
            )));
        }
    };
    let sample_counts = decode_csw_rle(&raw)?;
    if stored_pulses != 0 && sample_counts.len() as u32 != stored_pulses {
        return Err(TzxError::Format(format!(
            "TZX CSW pulse count mismatch (got {}, header {stored_pulses})",
            sample_counts.len()
        )));
    }
    ensure_pulse_room(pulses, sample_counts.len())?;
    // CSW pulses alternate; first edge flips from the current level (CLK/Fuse).
    for &samples in &sample_counts {
        let duration = samples.saturating_mul(t_per_sample);
        push_pulse_tzx(pulses, level, force_low, duration);
    }
    append_tzx_pause(pulses, level, force_low, ms_to_t(pause_ms));
    Ok(())
}

fn alphabet_size(raw: u8, present: bool) -> usize {
    if !present {
        0
    } else if raw == 0 {
        256
    } else {
        usize::from(raw)
    }
}

fn bits_per_symbol(alphabet: usize) -> usize {
    if alphabet <= 1 {
        return 0;
    }
    let mut bits = 1usize;
    let mut base = 2usize;
    while base < alphabet {
        base = base.saturating_mul(2);
        bits += 1;
    }
    bits
}

struct SymDef {
    flags: u8,
    pulses: Vec<u16>,
}

fn read_symdefs(
    data: &[u8],
    mut off: usize,
    count: usize,
    max_pulses: usize,
) -> Result<(Vec<SymDef>, usize), TzxError> {
    let mut out = Vec::with_capacity(count);
    let row = 1usize
        .checked_add(
            max_pulses
                .checked_mul(2)
                .ok_or_else(|| TzxError::Format("TZX GDB symbol row overflow".into()))?,
        )
        .ok_or_else(|| TzxError::Format("TZX GDB symbol row overflow".into()))?;
    for _ in 0..count {
        if off + row > data.len() {
            return Err(TzxError::Format("truncated TZX GDB SYMDEF".into()));
        }
        let flags = data[off];
        off += 1;
        let mut pulses = Vec::with_capacity(max_pulses);
        for _ in 0..max_pulses {
            pulses.push(u16::from_le_bytes([data[off], data[off + 1]]));
            off += 2;
        }
        out.push(SymDef { flags, pulses });
    }
    Ok((out, off))
}

fn emit_gdb_symbol(
    pulses: &mut Vec<(u32, bool)>,
    level: &mut bool,
    force_low: &mut bool,
    symbol: &SymDef,
) -> Result<(), TzxError> {
    // TZX SYMDEF flags b0-b1: starting polarity.
    match symbol.flags & 0x03 {
        0x00 => {
            // Opposite to current — normal edge (default push_pulse_tzx).
        }
        0x01 => {
            // Same as current — no edge: emit first pulse at previous EAR level.
            *force_low = false;
            *level = !*level;
        }
        0x02 => {
            *force_low = false;
            *level = false; // force low
        }
        0x03 => {
            *force_low = false;
            *level = true; // force high
        }
        _ => unreachable!(),
    }
    for &len in &symbol.pulses {
        if len == 0 {
            break;
        }
        ensure_pulse_room(pulses, 1)?;
        push_pulse_tzx(pulses, level, force_low, u32::from(len));
    }
    Ok(())
}

fn append_generalized_data(
    pulses: &mut Vec<(u32, bool)>,
    level: &mut bool,
    force_low: &mut bool,
    body: &[u8],
) -> Result<(), TzxError> {
    // body starts at pause WORD (block length already consumed).
    if body.len() < 14 {
        return Err(TzxError::Format("truncated 0x19 header".into()));
    }
    let pause_ms = u16::from_le_bytes([body[0], body[1]]);
    let pilot_symbol_count = u32::from_le_bytes([body[2], body[3], body[4], body[5]]);
    let npp = body[6] as usize;
    let pilot_alphabet_raw = body[7];
    let data_symbol_count = u32::from_le_bytes([body[8], body[9], body[10], body[11]]);
    let npd = body[12] as usize;
    let data_alphabet_raw = body[13];
    let asp = alphabet_size(pilot_alphabet_raw, pilot_symbol_count > 0);
    let asd = alphabet_size(data_alphabet_raw, data_symbol_count > 0);
    let mut off = 14usize;

    let (pilot_syms, next) = if pilot_symbol_count > 0 {
        read_symdefs(body, off, asp, npp)?
    } else {
        (Vec::new(), off)
    };
    off = next;

    if pilot_symbol_count > 0 {
        let need = (pilot_symbol_count as usize)
            .checked_mul(3)
            .ok_or_else(|| TzxError::Format("TZX GDB pilot stream overflow".into()))?;
        if off + need > body.len() {
            return Err(TzxError::Format("truncated TZX GDB pilot stream".into()));
        }
        for _ in 0..pilot_symbol_count {
            let sym = body[off] as usize;
            let reps = u16::from_le_bytes([body[off + 1], body[off + 2]]);
            off += 3;
            if sym >= pilot_syms.len() {
                return Err(TzxError::Format(format!(
                    "TZX GDB pilot symbol {sym} out of range ({})",
                    pilot_syms.len()
                )));
            }
            for _ in 0..reps {
                emit_gdb_symbol(pulses, level, force_low, &pilot_syms[sym])?;
            }
        }
    }

    let (data_syms, next) = if data_symbol_count > 0 {
        read_symdefs(body, off, asd, npd)?
    } else {
        (Vec::new(), off)
    };
    off = next;

    if data_symbol_count > 0 {
        let nb = bits_per_symbol(asd);
        // ASD==1 → NB==0 is a degenerate alphabet; reject huge no-bit streams
        // that would otherwise spin without consuming input (#374 CR).
        if nb == 0 && data_symbol_count > 1 {
            return Err(TzxError::Format(
                "TZX GDB data stream with ASD<=1 and TOTD>1 is invalid".into(),
            ));
        }
        let stream = &body[off..];
        let mut bit_i = 0usize;
        for _ in 0..data_symbol_count {
            let mut symbol = 0usize;
            for _ in 0..nb {
                let byte_i = bit_i / 8;
                let bit = 7 - (bit_i % 8);
                if byte_i >= stream.len() {
                    return Err(TzxError::Format("truncated TZX GDB data stream".into()));
                }
                let bit_val = (stream[byte_i] >> bit) & 1;
                symbol = (symbol << 1) | usize::from(bit_val);
                bit_i += 1;
            }
            // ASD==1 → NB==0: always symbol 0 (no bits consumed).
            if symbol >= data_syms.len() {
                return Err(TzxError::Format(format!(
                    "TZX GDB data symbol {symbol} out of range ({})",
                    data_syms.len()
                )));
            }
            emit_gdb_symbol(pulses, level, force_low, &data_syms[symbol])?;
        }
    }

    append_tzx_pause(pulses, level, force_low, ms_to_t(pause_ms));
    Ok(())
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
        assert!(
            !p.ear_level(),
            "first pilot is low (Fuse LEVEL_LOW / classic start)"
        );
        let _ = p.advance(PILOT_PULSE_T);
        assert!(p.ear_level(), "second pilot toggles high");
        let _ = p.advance(PILOT_PULSE_T);
        assert!(!p.ear_level(), "third pilot pulse is low");
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

    /// TZX last-byte `used_bits` are MSBs (`xxxxxx00` for 6), not LSBs.
    /// Speedlock 0x14 markers (Arkanoid) use used=5/6 — LSB emission desyncs EAR load.
    #[test]
    fn pure_data_used_bits_are_most_significant() {
        let mut v = Vec::new();
        v.extend_from_slice(b"ZXTape!");
        v.extend_from_slice(&[0x1a, 1, 20]);
        v.push(0x14);
        v.extend_from_slice(&100u16.to_le_bytes()); // zero
        v.extend_from_slice(&200u16.to_le_bytes()); // one
        v.push(6); // used bits — top 6 of 0b1110_1000 → 1,1,1,0,1,0
        v.extend_from_slice(&0u16.to_le_bytes()); // pause
        v.extend_from_slice(&1u32.to_le_bytes()[..3]); // len = 1
        v.push(0b1110_1000);
        let p = TzxPlayer::parse(&v).unwrap();
        assert_eq!(p.scheduled_pulses(), 12, "6 bits × 2 edges");
        let expect = [
            200u32, 200, 200, 200, 200, 200, 100, 100, 200, 200, 100, 100,
        ];
        let got: Vec<u32> = p.pulses.iter().map(|(d, _)| *d).collect();
        assert_eq!(got, expect, "MSB-first bit order for used_bits=6");
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
        // LEVEL_LOW → low, toggle high, toggle low. Exact end of a high pulse
        // (2nd) must clear latched EAR; use 2 pulses ending on high.
        let mut v = Vec::new();
        v.extend_from_slice(b"ZXTape!");
        v.extend_from_slice(&[0x1a, 1, 20]);
        v.push(0x12);
        v.extend_from_slice(&1000u16.to_le_bytes());
        v.extend_from_slice(&2u16.to_le_bytes());
        let mut p = TzxPlayer::parse(&v).unwrap();
        assert_eq!(p.scheduled_pulses(), 2);
        let _ = p.advance(1000); // low
        let mid = p.advance(500);
        assert!(
            mid && p.ear_level(),
            "second pure-tone pulse should be high"
        );
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
        v.extend_from_slice(&0u16.to_le_bytes()); // pause ms → ROM default gap
        let payload = [0x00u8, 0x41, 0x00];
        v.extend_from_slice(&(payload.len() as u16).to_le_bytes());
        v.extend_from_slice(&payload);
        v.push(0x12);
        v.extend_from_slice(&500u16.to_le_bytes());
        v.extend_from_slice(&3u16.to_le_bytes());
        let p = TzxPlayer::parse(&v).unwrap();
        // Data/pilot pulses must edge; Fuse LEVEL_LOW may hold consecutive lows.
        for w in p.pulses.windows(2) {
            if w[0].1 == w[1].1 {
                assert!(
                    !w[0].1,
                    "only low (Fuse LEVEL_LOW) may repeat across 0x10 pause→0x12"
                );
            }
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
            if w[0].1 == w[1].1 {
                assert!(
                    !w[0].1,
                    "only low (Fuse LEVEL_LOW) may repeat across 0x10→0x13"
                );
            }
        }
    }

    /// Tape start: Fuse `LEVEL_LOW` with initial low ≡ classic first pilot low.
    #[test]
    fn standard_block_starts_low_after_tape_start() {
        let data = minimal_tzx_standard(&[0x00, 0x41, 0x00]);
        let p = TzxPlayer::parse(&data).unwrap();
        assert!(!p.pulses[0].1, "first pilot pulse must be low");
        assert!(p.pulses[1].1, "second pilot toggles high");
        assert!(!p.pulses[2].1, "third pilot pulse must be low");
    }

    /// Arkanoid-scale Pure Data pause must match Fuse: after an even number of
    /// data half-pulses the pause starts low, then the next tone starts low
    /// again via `LEVEL_LOW` (two consecutive lows).
    #[test]
    fn pure_data_pause_matches_fuse_polarity_before_next_tone() {
        let mut v = Vec::new();
        v.extend_from_slice(b"ZXTape!");
        v.extend_from_slice(&[0x1a, 1, 20]);
        // Eight bits → 16 half-pulses. Final high half leaves the pause low;
        // LEVEL_LOW then repeats low for the first Pure Tone pulse.
        v.push(0x14);
        v.extend_from_slice(&100u16.to_le_bytes());
        v.extend_from_slice(&200u16.to_le_bytes());
        v.push(8);
        v.extend_from_slice(&1750u16.to_le_bytes());
        v.extend_from_slice(&1u32.to_le_bytes()[..3]);
        v.push(0x01); // MSB=0 → first half-pulses are zeros (low if starting low)
        v.push(0x12);
        v.extend_from_slice(&2165u16.to_le_bytes());
        v.extend_from_slice(&4u16.to_le_bytes());
        let p = TzxPlayer::parse(&v).unwrap();
        // 8 bits × 2 + pause + 4 tone
        assert_eq!(p.scheduled_pulses(), 16 + 1 + 4);
        let pause = p.pulses[16];
        let tone0 = p.pulses[17];
        assert_eq!(pause.0, 1750 * 3500);
        assert!(
            !pause.1,
            "pause starts low after final data half-pulse high"
        );
        assert_eq!(tone0.0, 2165);
        assert!(!tone0.1, "next Pure Tone starts low (Fuse LEVEL_LOW)");
        assert_eq!(p.pulses[18].0, 2165);
        assert!(p.pulses[18].1, "second tone pulse toggles high");
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
        assert!(p.finished());
        assert!(!p.playing);
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

    /// Speedlock-style Loop Start/End: Pure Tone inside a loop of N=3.
    ///
    /// **Regression (#372):** before Loop support, `parse` returned
    /// `unsupported TZX block ID 0x24` — the Arkanoid / `SpecChumMac` Open failure.
    /// This synthetic fixture is the CI gate (no copyrighted game dump).
    #[test]
    fn loop_start_end_expands_pure_tone() {
        let mut v = Vec::new();
        v.extend_from_slice(b"ZXTape!");
        v.extend_from_slice(&[0x1a, 1, 20]);
        v.push(0x24); // Loop Start
        v.extend_from_slice(&3u16.to_le_bytes());
        v.push(0x12); // Pure Tone — 2 pulses
        v.extend_from_slice(&1000u16.to_le_bytes());
        v.extend_from_slice(&2u16.to_le_bytes());
        v.push(0x25); // Loop End
        let p = TzxPlayer::parse(&v).unwrap();
        assert_eq!(p.scheduled_pulses(), 6, "3×2 pulses from looped pure tone");
        assert_eq!(p.block_count(), 3);
    }

    /// Jump (`0x23`) relative +2 skips the next block (#374).
    #[test]
    fn jump_skips_next_block() {
        let mut v = tzx_header();
        v.push(0x23);
        v.extend_from_slice(&2i16.to_le_bytes());
        append_pure_tone(&mut v, 1000, 4);
        append_pure_tone(&mut v, 1000, 2);
        let p = TzxPlayer::parse(&v).unwrap();
        assert_eq!(
            p.scheduled_pulses(),
            2,
            "Jump +2 must skip the 4-pulse tone"
        );
    }

    /// Call (`0x26`) / Return (`0x27`): play subroutine then resume (#374).
    ///
    /// After Return, a Jump skips the in-file subroutine so linear fall-through
    /// does not re-enter it (common TZX layout).
    #[test]
    fn call_sequence_plays_subroutine_then_resumes() {
        let mut v = tzx_header();
        // 0: Call +3 → subroutine at block 3
        v.push(0x26);
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&3i16.to_le_bytes());
        // 1: resume tone (after Return)
        append_pure_tone(&mut v, 1000, 1);
        // 2: Jump +3 → past Return
        v.push(0x23);
        v.extend_from_slice(&3i16.to_le_bytes());
        // 3: subroutine body
        append_pure_tone(&mut v, 1000, 3);
        // 4: Return
        v.push(0x27);
        let p = TzxPlayer::parse(&v).unwrap();
        assert_eq!(
            p.scheduled_pulses(),
            4,
            "Call must play 3-pulse subroutine then 1-pulse resume"
        );
    }

    /// Select (`0x28`) auto-picks the first menu entry for headless open (#374).
    #[test]
    fn select_auto_picks_first_entry() {
        let mut v = tzx_header();
        v.push(0x28);
        let desc = b"part";
        let body_len = 1 + 2 + 1 + desc.len();
        v.extend_from_slice(&(body_len as u16).to_le_bytes());
        v.push(1);
        v.extend_from_slice(&2i16.to_le_bytes());
        v.push(desc.len() as u8);
        v.extend_from_slice(desc);
        append_pure_tone(&mut v, 1000, 5);
        append_pure_tone(&mut v, 1000, 2);
        let p = TzxPlayer::parse(&v).unwrap();
        assert_eq!(p.scheduled_pulses(), 2, "Select must jump to first entry");
    }

    #[test]
    fn jump_relative_zero_is_rejected() {
        let mut v = tzx_header();
        v.push(0x23);
        v.extend_from_slice(&0i16.to_le_bytes());
        let err = TzxPlayer::parse(&v).expect_err("rel 0 must Err");
        assert!(err.to_string().contains("infinite"), "got {err}");
    }

    #[test]
    fn loop_reps_zero_skips_body() {
        let mut v = Vec::new();
        v.extend_from_slice(b"ZXTape!");
        v.extend_from_slice(&[0x1a, 1, 20]);
        v.push(0x24);
        v.extend_from_slice(&0u16.to_le_bytes());
        v.push(0x12);
        v.extend_from_slice(&1000u16.to_le_bytes());
        v.extend_from_slice(&4u16.to_le_bytes());
        v.push(0x25);
        v.push(0x12);
        v.extend_from_slice(&500u16.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        let p = TzxPlayer::parse(&v).unwrap();
        assert_eq!(p.scheduled_pulses(), 1, "reps=0 must skip loop body");
    }

    #[test]
    fn loop_around_standard_blocks_not_flash_convertible() {
        // Loop of standard 0x10 must stay on the TZX pulse path — converting to
        // TAP would keep only one copy of the payload and drop repetitions.
        let mut v = Vec::new();
        v.extend_from_slice(b"ZXTape!");
        v.extend_from_slice(&[0x1a, 1, 20]);
        v.push(0x24);
        v.extend_from_slice(&3u16.to_le_bytes());
        v.push(0x10);
        v.extend_from_slice(&100u16.to_le_bytes());
        let payload = [0xffu8, 1, 2, 0];
        v.extend_from_slice(&(payload.len() as u16).to_le_bytes());
        v.extend_from_slice(&payload);
        v.push(0x25);
        assert!(
            !TzxPlayer::is_standard_speed_only(&v),
            "loop markers must disable flash/TAP-only conversion"
        );
        let p = TzxPlayer::parse(&v).unwrap();
        assert_eq!(
            p.block_count(),
            3,
            "parse must expand 0x10 three times via Loop Start/End"
        );
    }

    #[test]
    fn loop_expansion_respects_pulse_budget() {
        // Large body × large reps would otherwise OOM the pulse Vec.
        let mut v = Vec::new();
        v.extend_from_slice(b"ZXTape!");
        v.extend_from_slice(&[0x1a, 1, 20]);
        v.push(0x24);
        v.extend_from_slice(&u16::MAX.to_le_bytes());
        v.push(0x12);
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&4096u16.to_le_bytes());
        v.push(0x25);
        let err = TzxPlayer::parse(&v).expect_err("pathological loop must fail");
        assert!(
            err.to_string().contains("exceeded"),
            "expected pulse-budget Format error, got {err}"
        );
    }

    #[test]
    fn nested_empty_loops_hit_replay_budget() {
        // Empty nested loops add neither pulses nor logical blocks — need a
        // replay-transition cap so parse cannot spin for billions of jumps.
        let mut v = Vec::new();
        v.extend_from_slice(b"ZXTape!");
        v.extend_from_slice(&[0x1a, 1, 20]);
        v.push(0x24);
        v.extend_from_slice(&u16::MAX.to_le_bytes());
        v.push(0x24);
        v.extend_from_slice(&u16::MAX.to_le_bytes());
        v.push(0x25);
        v.push(0x25);
        let err = TzxPlayer::parse(&v).expect_err("empty nested loops must fail");
        assert!(
            err.to_string().contains("replay"),
            "expected loop-replay budget error, got {err}"
        );
    }

    #[test]
    fn to_tap_image_skips_loop_markers_and_keeps_id10() {
        // ID 0x10, then a loop of pure tone, then another ID 0x10.
        // Before #372, `_ => break` on 0x24 stopped the scan and dropped the
        // trailing standard block — that is the coverage gap this asserts.
        let mut v = Vec::new();
        v.extend_from_slice(b"ZXTape!");
        v.extend_from_slice(&[0x1a, 1, 20]);
        v.push(0x10);
        v.extend_from_slice(&100u16.to_le_bytes());
        let first = [0xffu8, 1, 2, 0];
        v.extend_from_slice(&(first.len() as u16).to_le_bytes());
        v.extend_from_slice(&first);
        v.push(0x24);
        v.extend_from_slice(&2u16.to_le_bytes());
        v.push(0x12);
        v.extend_from_slice(&100u16.to_le_bytes());
        v.extend_from_slice(&2u16.to_le_bytes());
        v.push(0x25);
        v.push(0x10);
        v.extend_from_slice(&50u16.to_le_bytes());
        let second = [0x00u8, 0x41, 0x00];
        v.extend_from_slice(&(second.len() as u16).to_le_bytes());
        v.extend_from_slice(&second);

        let tap = TzxPlayer::to_tap_image(&v).unwrap();
        assert_eq!(
            tap.blocks.len(),
            2,
            "loop markers must be skipped so trailing 0x10 is still extracted"
        );
        assert_eq!(tap.blocks[0], first);
        assert_eq!(tap.blocks[1], second);
        assert!(
            !TzxPlayer::is_standard_speed_only(&v),
            "0x12 inside the loop keeps this off the flash/TAP-only path"
        );
    }

    /// Optional local commercial Speedlock TZX (never commit the image).
    #[test]
    fn arkanoid_downloads_parses_when_present() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let path = PathBuf::from(home).join("Downloads/Arkanoid.tzx");
        if !path.is_file() {
            eprintln!("skip: ~/Downloads/Arkanoid.tzx not present");
            return;
        }
        let p = TzxPlayer::load(&path).expect("Arkanoid.tzx must parse (#372)");
        assert!(
            p.scheduled_pulses() > 10_000,
            "Speedlock loops should expand to a large pulse schedule, got {}",
            p.scheduled_pulses()
        );
        assert!(p.block_count() > 4);
        assert!(!TzxPlayer::is_standard_speed_only(
            &std::fs::read(&path).unwrap()
        ));
    }

    /// Fuse `LEVEL_LOW` after pauses (Arkanoid Speedlock gaps). Optional fixture.
    #[test]
    fn arkanoid_pause_polarity_matches_fuse_when_present() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let path = PathBuf::from(home).join("Downloads/Arkanoid.tzx");
        if !path.is_file() {
            eprintln!("skip: ~/Downloads/Arkanoid.tzx not present");
            return;
        }
        let p = TzxPlayer::load(&path).expect("Arkanoid.tzx must parse");
        // Tape start ≡ classic: low, high, low (LEVEL_LOW when already low).
        assert!(!p.pulses[0].1 && p.pulses[1].1 && !p.pulses[2].1);
        let p3984 = p
            .pulses
            .iter()
            .position(|(d, _)| *d == 3984 * 3500)
            .expect("3984ms Speedlock pause");
        let p1750 = p
            .pulses
            .iter()
            .position(|(d, _)| *d == 1750 * 3500)
            .expect("1750ms pause");
        // 3984ms pause ends high; LEVEL_LOW drops to low, then high (Fuse dump).
        assert!(p.pulses[p3984].1, "3984ms pause high (Fuse)");
        assert!(
            !p.pulses[p3984 + 1].1,
            "first Speedlock tone low (LEVEL_LOW)"
        );
        assert!(p.pulses[p3984 + 2].1, "next tone edge high");
        // 1750ms pause ends low; LEVEL_LOW keeps low (two lows), then high.
        assert!(!p.pulses[p1750].1, "1750ms pause low (Fuse)");
        assert!(!p.pulses[p1750 + 1].1, "LEVEL_LOW stays low");
        assert!(p.pulses[p1750 + 2].1, "next edge high");
    }

    // --- Media / TZX capability matrix (#374) ---------------------------------

    fn tzx_header() -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"ZXTape!");
        v.extend_from_slice(&[0x1a, 1, 20]);
        v
    }

    fn append_id10(v: &mut Vec<u8>, payload: &[u8], pause_ms: u16) {
        v.push(0x10);
        v.extend_from_slice(&pause_ms.to_le_bytes());
        v.extend_from_slice(&(payload.len() as u16).to_le_bytes());
        v.extend_from_slice(payload);
    }

    fn append_pure_tone(v: &mut Vec<u8>, len: u16, count: u16) {
        v.push(0x12);
        v.extend_from_slice(&len.to_le_bytes());
        v.extend_from_slice(&count.to_le_bytes());
    }

    /// Pulse-producing / skippable IDs currently accepted by [`TzxPlayer::parse`].
    ///
    /// Info-only blocks (`SkipOk`) parse successfully with an empty schedule; CI
    /// still covers them so a future regression that hard-errors on skip markers
    /// is caught. Pulse families require a non-empty schedule.
    #[test]
    fn tzx_block_matrix_supported_ids_parse() {
        #[derive(Clone, Copy)]
        enum Expect {
            Pulses,
            SkipOk,
        }
        struct Case {
            id: u8,
            name: &'static str,
            expect: Expect,
            build: fn() -> Vec<u8>,
        }

        let cases: &[Case] = &[
            Case {
                id: 0x10,
                name: "Standard Speed Data",
                expect: Expect::Pulses,
                build: || {
                    let mut v = tzx_header();
                    append_id10(&mut v, &[0x00, 0x41, 0x00], 100);
                    v
                },
            },
            Case {
                id: 0x11,
                name: "Turbo Speed Data",
                expect: Expect::Pulses,
                build: || {
                    let mut v = tzx_header();
                    v.push(0x11);
                    v.extend_from_slice(&800u16.to_le_bytes()); // pilot
                    v.extend_from_slice(&400u16.to_le_bytes()); // sync1
                    v.extend_from_slice(&400u16.to_le_bytes()); // sync2
                    v.extend_from_slice(&300u16.to_le_bytes()); // zero
                    v.extend_from_slice(&600u16.to_le_bytes()); // one
                    v.extend_from_slice(&10u16.to_le_bytes()); // pilot pulses
                    v.push(8); // used bits
                    v.extend_from_slice(&0u16.to_le_bytes()); // pause
                    let payload = [0xffu8, 0x00];
                    v.extend_from_slice(&(payload.len() as u32).to_le_bytes()[..3]);
                    v.extend_from_slice(&payload);
                    v
                },
            },
            Case {
                id: 0x12,
                name: "Pure Tone",
                expect: Expect::Pulses,
                build: || {
                    let mut v = tzx_header();
                    append_pure_tone(&mut v, 1000, 4);
                    v
                },
            },
            Case {
                id: 0x13,
                name: "Pulse Sequence",
                expect: Expect::Pulses,
                build: || {
                    let mut v = tzx_header();
                    v.push(0x13);
                    v.push(3); // n
                    v.extend_from_slice(&500u16.to_le_bytes());
                    v.extend_from_slice(&600u16.to_le_bytes());
                    v.extend_from_slice(&700u16.to_le_bytes());
                    v
                },
            },
            Case {
                id: 0x14,
                name: "Pure Data",
                expect: Expect::Pulses,
                build: || {
                    let mut v = tzx_header();
                    v.push(0x14);
                    v.extend_from_slice(&100u16.to_le_bytes()); // zero
                    v.extend_from_slice(&200u16.to_le_bytes()); // one
                    v.push(8);
                    v.extend_from_slice(&0u16.to_le_bytes());
                    let payload = [0xa5u8];
                    v.extend_from_slice(&(payload.len() as u32).to_le_bytes()[..3]);
                    v.extend_from_slice(&payload);
                    v
                },
            },
            Case {
                id: 0x15,
                name: "Direct recording",
                expect: Expect::Pulses,
                build: || {
                    let mut v = tzx_header();
                    v.push(0x15);
                    v.extend_from_slice(&158u16.to_le_bytes()); // ~22050 Hz
                    v.extend_from_slice(&0u16.to_le_bytes()); // pause
                    v.push(8); // used bits
                    let samples = [0b1010_1010u8];
                    v.extend_from_slice(&(samples.len() as u32).to_le_bytes()[..3]);
                    v.extend_from_slice(&samples);
                    v
                },
            },
            Case {
                id: 0x18,
                name: "CSW recording (RLE)",
                expect: Expect::Pulses,
                build: || {
                    let mut v = tzx_header();
                    v.push(0x18);
                    // body: pause + rate + compression + pulse count + RLE data
                    let csw = [10u8, 20, 30]; // three pulses
                    let body_len = 10 + csw.len();
                    v.extend_from_slice(&(body_len as u32).to_le_bytes());
                    v.extend_from_slice(&0u16.to_le_bytes()); // pause
                    v.extend_from_slice(&22_050u32.to_le_bytes()[..3]); // rate
                    v.push(0x01); // RLE
                    v.extend_from_slice(&(csw.len() as u32).to_le_bytes());
                    v.extend_from_slice(&csw);
                    v
                },
            },
            Case {
                id: 0x19,
                name: "Generalized data",
                expect: Expect::Pulses,
                build: || {
                    // Minimal GDB: no pilot, 2 data symbols (0/1), one data symbol.
                    // SYMDEF: flags=0 (edge), one pulse length each.
                    let mut v = tzx_header();
                    v.push(0x19);
                    let mut body = Vec::new();
                    body.extend_from_slice(&0u16.to_le_bytes()); // pause
                    body.extend_from_slice(&0u32.to_le_bytes()); // TOTP
                    body.push(0); // NPP
                    body.push(0); // ASP
                    body.extend_from_slice(&1u32.to_le_bytes()); // TOTD = 1
                    body.push(1); // NPD = 1 pulse max
                    body.push(2); // ASD = 2 symbols
                                  // Symbol 0: edge, pulse 500
                    body.push(0x00);
                    body.extend_from_slice(&500u16.to_le_bytes());
                    // Symbol 1: edge, pulse 1000
                    body.push(0x00);
                    body.extend_from_slice(&1000u16.to_le_bytes());
                    // Data stream: 1 bit → symbol 0 (MSB first → 0x00)
                    body.push(0x00);
                    v.extend_from_slice(&(body.len() as u32).to_le_bytes());
                    v.extend_from_slice(&body);
                    v
                },
            },
            Case {
                id: 0x20,
                name: "Pause",
                expect: Expect::Pulses,
                build: || {
                    let mut v = tzx_header();
                    v.push(0x20);
                    v.extend_from_slice(&10u16.to_le_bytes()); // non-zero → one silence pulse
                    v
                },
            },
            Case {
                id: 0x21,
                name: "Group Start",
                expect: Expect::SkipOk,
                build: || {
                    let mut v = tzx_header();
                    v.push(0x21);
                    let name = b"grp";
                    v.push(name.len() as u8);
                    v.extend_from_slice(name);
                    v
                },
            },
            Case {
                id: 0x22,
                name: "Group End",
                expect: Expect::SkipOk,
                build: || {
                    let mut v = tzx_header();
                    v.push(0x22);
                    v
                },
            },
            Case {
                id: 0x23,
                name: "Jump to block",
                expect: Expect::Pulses,
                build: || {
                    // Jump +2 skips a 4-pulse tone; land on a 2-pulse tone.
                    let mut v = tzx_header();
                    v.push(0x23);
                    v.extend_from_slice(&2i16.to_le_bytes());
                    append_pure_tone(&mut v, 1000, 4);
                    append_pure_tone(&mut v, 1000, 2);
                    v
                },
            },
            Case {
                id: 0x24,
                name: "Loop Start/End",
                expect: Expect::Pulses,
                build: || {
                    let mut v = tzx_header();
                    v.push(0x24);
                    v.extend_from_slice(&2u16.to_le_bytes());
                    append_pure_tone(&mut v, 1000, 2);
                    v.push(0x25);
                    v
                },
            },
            Case {
                id: 0x26,
                name: "Call sequence / Return",
                expect: Expect::Pulses,
                build: || {
                    // Call → sub → Return → resume tone → Jump past sub.
                    let mut v = tzx_header();
                    v.push(0x26);
                    v.extend_from_slice(&1u16.to_le_bytes());
                    v.extend_from_slice(&3i16.to_le_bytes());
                    append_pure_tone(&mut v, 1000, 1); // resume
                    v.push(0x23);
                    v.extend_from_slice(&3i16.to_le_bytes()); // skip sub
                    append_pure_tone(&mut v, 1000, 2); // subroutine
                    v.push(0x27);
                    v
                },
            },
            Case {
                id: 0x28,
                name: "Select block (first entry)",
                expect: Expect::Pulses,
                build: || {
                    // Select first option → jump +2 over a 4-pulse tone onto a 2-pulse tone.
                    let mut v = tzx_header();
                    v.push(0x28);
                    let desc = b"A";
                    // body: N + SELECT (rel WORD + L + text)
                    let body_len = 1 + 2 + 1 + desc.len();
                    v.extend_from_slice(&(body_len as u16).to_le_bytes());
                    v.push(1); // one selection
                    v.extend_from_slice(&2i16.to_le_bytes());
                    v.push(desc.len() as u8);
                    v.extend_from_slice(desc);
                    append_pure_tone(&mut v, 1000, 4);
                    append_pure_tone(&mut v, 1000, 2);
                    v
                },
            },
            Case {
                id: 0x2a,
                name: "Stop if 48K (accepted no-op)",
                expect: Expect::SkipOk,
                build: || {
                    let mut v = tzx_header();
                    v.push(0x2a);
                    v.extend_from_slice(&0u32.to_le_bytes());
                    v
                },
            },
            Case {
                id: 0x2b,
                name: "Set signal level",
                expect: Expect::SkipOk,
                build: || {
                    let mut v = tzx_header();
                    v.push(0x2b);
                    v.extend_from_slice(&1u32.to_le_bytes());
                    v.push(1); // high
                    v
                },
            },
            Case {
                id: 0x30,
                name: "Text description",
                expect: Expect::SkipOk,
                build: || {
                    let mut v = tzx_header();
                    v.push(0x30);
                    let text = b"hi";
                    v.push(text.len() as u8);
                    v.extend_from_slice(text);
                    v
                },
            },
            Case {
                id: 0x32,
                name: "Archive info",
                expect: Expect::SkipOk,
                build: || {
                    let mut v = tzx_header();
                    v.push(0x32);
                    let body = [0x00u8, 1, b'x']; // one text entry
                    v.extend_from_slice(&(body.len() as u16).to_le_bytes());
                    v.extend_from_slice(&body);
                    v
                },
            },
            Case {
                id: 0x33,
                name: "Hardware type",
                expect: Expect::SkipOk,
                build: || {
                    let mut v = tzx_header();
                    v.push(0x33);
                    v.push(1); // one entry
                    v.extend_from_slice(&[0x00, 0x00, 0x00]);
                    v
                },
            },
            Case {
                id: 0x35,
                name: "Custom info",
                expect: Expect::SkipOk,
                build: || {
                    let mut v = tzx_header();
                    v.push(0x35);
                    v.extend_from_slice(b"CUSTOMINFOBLOCK!"); // 16-byte id
                    v.extend_from_slice(&0u32.to_le_bytes()); // length
                    v
                },
            },
            Case {
                id: 0x5a,
                name: "Glue block",
                expect: Expect::SkipOk,
                build: || {
                    let mut v = tzx_header();
                    v.push(0x5a);
                    v.extend_from_slice(&[0; 9]);
                    v
                },
            },
        ];

        for case in cases {
            let data = (case.build)();
            let p = TzxPlayer::parse(&data).unwrap_or_else(|e| {
                panic!(
                    "supported TZX 0x{:02x} ({}) must parse: {e}",
                    case.id, case.name
                )
            });
            match case.expect {
                Expect::Pulses => assert!(
                    p.scheduled_pulses() > 0,
                    "0x{:02x} ({}) must schedule pulses, got 0",
                    case.id,
                    case.name
                ),
                Expect::SkipOk => assert_eq!(
                    p.scheduled_pulses(),
                    0,
                    "0x{:02x} ({}) is info/skip-only — expect empty schedule",
                    case.id,
                    case.name
                ),
            }
        }
    }

    /// Intentionally unsupported IDs must fail loudly with the hex ID in the
    /// message (host Open surfaces this via `sc_last_error`). Commercial-common
    /// Direct/CSW/GDB/0x2A/0x2B are supported (#374); keep the gate on reserved /
    /// deprecated IDs that remain hard errors.
    #[test]
    fn tzx_block_matrix_unsupported_ids_fail_loudly() {
        // Minimal byte after the ID is enough — parse rejects before reading a body.
        const UNSUPPORTED: &[u8] = &[
            0x16, // C64 ROM (deprecated)
            0x17, // C64 turbo (deprecated)
            0x34, // Emulation info (deprecated)
            0x40, // Snapshot block (deprecated)
        ];
        for &id in UNSUPPORTED {
            let mut v = tzx_header();
            v.push(id);
            // Padding so truncated-body paths are not what we exercise.
            v.extend_from_slice(&[0u8; 16]);
            let err = TzxPlayer::parse(&v).expect_err("unsupported ID must Err");
            let msg = err.to_string();
            assert!(
                msg.contains(&format!("0x{id:02x}")) || msg.contains(&format!("0x{id:02X}")),
                "error must name block ID 0x{id:02x}, got {msg}"
            );
            assert!(
                msg.to_ascii_lowercase().contains("unsupported"),
                "error must say unsupported, got {msg}"
            );
        }
    }

    /// `#374` scan harden: unknown IDs must not `_ => break` and drop a trailing
    /// standard `0x10` that a flash/TAP conversion would otherwise keep.
    #[test]
    fn to_tap_image_errors_on_unsupported_instead_of_truncating() {
        let mut v = tzx_header();
        append_id10(&mut v, &[0xff, 1, 2, 0], 100);
        // Jump is parseable for EAR but not linear TAP-extractable (#374).
        v.push(0x23);
        v.extend_from_slice(&1i16.to_le_bytes());
        append_id10(&mut v, &[0x00, 0x41, 0x00], 50);

        let err = TzxPlayer::to_tap_image(&v).expect_err("must not silently truncate");
        let msg = err.to_string();
        assert!(
            msg.contains("0x23"),
            "TAP scan must surface unsupported 0x23, got {msg}"
        );
    }

    /// Skip/info markers between two `0x10`s must not truncate the trailing block.
    #[test]
    fn to_tap_image_keeps_id10_across_skip_markers() {
        let mut v = tzx_header();
        let first = [0xffu8, 1, 2, 0];
        let second = [0x00u8, 0x41, 0x00];
        append_id10(&mut v, &first, 100);
        v.push(0x21);
        v.push(3);
        v.extend_from_slice(b"grp");
        v.push(0x30);
        v.push(2);
        v.extend_from_slice(b"hi");
        v.push(0x22);
        v.push(0x5a);
        v.extend_from_slice(&[0; 9]);
        append_id10(&mut v, &second, 50);

        let tap = TzxPlayer::to_tap_image(&v).unwrap();
        assert_eq!(tap.blocks.len(), 2);
        assert_eq!(tap.blocks[0], first);
        assert_eq!(tap.blocks[1], second);
    }
}
