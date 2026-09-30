//! TZX waveform scheduling and block pulse synthesis.

use std::io::Read;

use flate2::read::ZlibDecoder;

use super::{ensure_pulse_room, TzxError, MAX_SCHEDULED_PULSES};
use crate::{
    BIT0_T, BIT1_T, PAUSE_T, PILOT_DATA_PULSES, PILOT_HEADER_PULSES, PILOT_PULSE_T, SYNC1_T,
    SYNC2_T,
};

pub(super) fn ms_to_t(ms: u16) -> u32 {
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
pub(super) fn push_pulse_tzx(
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
pub(super) fn append_tzx_pause(
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

pub(super) fn append_standard_block(
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
pub(super) fn append_turbo_block(
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

pub(super) fn append_pure_data(
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
pub(super) fn estimate_direct_samples(byte_len: usize, used_bits: u8) -> Option<usize> {
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

pub(super) fn append_direct_recording(
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

pub(super) fn append_csw_recording(
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

pub(super) fn append_generalized_data(
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
