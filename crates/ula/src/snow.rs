//! 48K/16K ULA snow effect — original Sinclair hardware bug when `I` is `$40–$7F`.
//!
//! When an M1 refresh cycle overlaps ULA video fetch, the low byte of the display
//! read address is replaced with register R (snow), or the previous column is
//! duplicated (double effect). See Weiv / MAME `spectrum_refresh_w`.

use crate::{PAPER_START_48, T_LINE_48};

/// True when the I register points at contended RAM (snow prerequisite).
#[must_use]
pub fn snow_possible(i: u8) -> bool {
    i & 0xC0 == 0x40
}

/// Snow / double pattern within the 8T ULA fetch window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnowPattern {
    /// Low address byte replaced with R — ULA reads `(hi | r)` from RAM.
    Corrupt,
    /// Second column pair duplicates the previous column (double effect).
    Double,
}

/// Active ULA display fetch during the paper area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UlaFetch {
    pub line: u32,
    pub col: usize,
    pub phase: u8,
    pub bitmap_off: usize,
    pub attr_off: usize,
    /// Low byte of the intended Spectrum screen address for this fetch.
    pub addr_lo: u8,
}

/// Locate the ULA display fetch at `frame_t` (48K PAL timing).
#[must_use]
pub fn ula_fetch_at(frame_t: u32) -> Option<UlaFetch> {
    ula_fetch_at_timed(frame_t, PAPER_START_48, T_LINE_48)
}

/// Locate ULA fetch with explicit line timing (48K only — snow is not modelled on 128K+).
#[must_use]
pub fn ula_fetch_at_timed(frame_t: u32, paper_start: u32, t_line: u32) -> Option<UlaFetch> {
    if frame_t < paper_start {
        return None;
    }
    let t = frame_t - paper_start;
    let line = t / t_line;
    if line >= 192 {
        return None;
    }
    let x = t % t_line;
    // First display fetch is 3T after paper/contention starts.
    if !(3..131).contains(&x) {
        return None;
    }
    let xf = x - 3;
    let phase = (xf % 8) as u8;
    let col = ((xf / 8) * 2 + u32::from(phase / 2)) as usize;
    if col > 31 {
        return None;
    }
    let row = line as usize;
    let third = row / 64;
    let yb = row % 8;
    let yo = (row / 8) % 8;
    let bitmap_off = third * 2048 + yo * 32 + yb * 256 + col;
    let attr_off = 6144 + (row / 8) * 32 + col;
    // MAME `addr_lo = ((y & 0x18) << 2) | (x >> 3)` with x = col * 8 px.
    let addr_lo = ((row as u8) & 0x18) << 2 | col as u8;
    Some(UlaFetch {
        line,
        col,
        phase,
        bitmap_off,
        attr_off,
        addr_lo,
    })
}

/// Snow pattern at ULA fetch phase (Weiv / MAME: T3 and T5 of the 8T window).
#[must_use]
pub fn pattern_at_phase(phase: u8) -> Option<SnowPattern> {
    match phase {
        0 | 1 => Some(SnowPattern::Corrupt),
        4 | 5 => Some(SnowPattern::Double),
        _ => None,
    }
}

/// Screen-RAM offsets corrupted by snow at M1 refresh T4.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnowOverride {
    pub offset: usize,
    pub byte: u8,
}

/// Compute snow overrides for one M1 refresh at `frame_t` (48K).
///
/// `screen` is the 6912-byte display file (`0x4000` relative). `m1_contended`
/// must be true when the opcode fetch at T1 was contended (snow requires an
/// uncontended M1 overlapping ULA fetch).
#[must_use]
pub fn snow_overrides(
    frame_t: u32,
    i: u8,
    r: u8,
    m1_contended: bool,
    screen: &[u8],
) -> Vec<SnowOverride> {
    if m1_contended || !snow_possible(i) || screen.len() < 6912 {
        return Vec::new();
    }
    let Some(fetch) = ula_fetch_at(frame_t) else {
        return Vec::new();
    };
    let Some(pattern) = pattern_at_phase(fetch.phase) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(2);
    match pattern {
        SnowPattern::Corrupt => {
            let r_lo = r & 0x7f;
            if r_lo == fetch.addr_lo {
                return out;
            }
            let bm_hi = fetch.bitmap_off & !0xff;
            let at_hi = fetch.attr_off & !0xff;
            let corrupt_bm = bm_hi | usize::from(r_lo);
            let corrupt_at = at_hi | usize::from(r_lo);
            if corrupt_bm >= 6912 || corrupt_at >= 6912 {
                return out;
            }
            let bm_byte = screen[corrupt_bm];
            let at_byte = screen[corrupt_at];
            if bm_byte != screen[fetch.bitmap_off] {
                out.push(SnowOverride {
                    offset: fetch.bitmap_off,
                    byte: bm_byte,
                });
            }
            if at_byte != screen[fetch.attr_off] {
                out.push(SnowOverride {
                    offset: fetch.attr_off,
                    byte: at_byte,
                });
            }
        }
        SnowPattern::Double => {
            if fetch.col == 0 {
                return out;
            }
            let prev_bm = fetch.bitmap_off - 1;
            let prev_at = fetch.attr_off - 1;
            let bm_byte = screen[prev_bm];
            let at_byte = screen[prev_at];
            if bm_byte != screen[fetch.bitmap_off] {
                out.push(SnowOverride {
                    offset: fetch.bitmap_off,
                    byte: bm_byte,
                });
            }
            if at_byte != screen[fetch.attr_off] {
                out.push(SnowOverride {
                    offset: fetch.attr_off,
                    byte: at_byte,
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snow_possible_only_i_40_7f() {
        assert!(!snow_possible(0x3f));
        assert!(snow_possible(0x40));
        assert!(snow_possible(0x7f));
        assert!(!snow_possible(0x80));
    }

    #[test]
    fn corrupt_uses_refresh_low_byte_not_display() {
        let mut screen = vec![0u8; 6912];
        // Row 0 col 0: intended bitmap 0xAA; byte at (hi|r) = 0x55 when r=1.
        screen[0] = 0xAA;
        screen[1] = 0x55;
        let t = PAPER_START_48 + 3; // phase 0, first bm fetch
        let ovs = snow_overrides(t, 0x40, 1, false, &screen);
        assert_eq!(ovs.len(), 1);
        assert_eq!(ovs[0].offset, 0);
        assert_eq!(ovs[0].byte, 0x55);
        assert_ne!(ovs[0].byte, screen[0]);
    }

    #[test]
    fn corrupt_skipped_when_r_matches_addr_lo() {
        let screen = vec![0u8; 6912];
        let t = PAPER_START_48 + 3;
        // Row 0 col 0 → addr_lo = 0.
        let ovs = snow_overrides(t, 0x40, 0, false, &screen);
        assert!(ovs.is_empty());
    }

    #[test]
    fn double_duplicates_previous_column() {
        let mut screen = vec![0u8; 6912];
        screen[1] = 0x11;
        screen[2] = 0x22;
        // phase 4 → col 2 in 8T window: t = paper_start + 3 + 4
        let t = PAPER_START_48 + 7;
        let ovs = snow_overrides(t, 0x40, 0, false, &screen);
        assert!(ovs.iter().any(|o| o.offset == 2 && o.byte == 0x11));
    }

    #[test]
    fn no_snow_when_m1_contended() {
        let mut screen = vec![0u8; 6912];
        screen[0] = 0xAA;
        screen[1] = 0x55;
        let t = PAPER_START_48 + 3;
        assert!(snow_overrides(t, 0x40, 1, true, &screen).is_empty());
    }
}
