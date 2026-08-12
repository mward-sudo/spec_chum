//! Spec Chum ULA — frame timing, contention, floating bus, video render.

#![allow(clippy::pedantic)]

/// 48K PAL frame constants.
pub const T_LINE_48: u32 = 224;
pub const LINES_48: u32 = 312;
pub const FRAME_TSTATES_48: u32 = T_LINE_48 * LINES_48; // 69888
pub const INT_LENGTH_48: u32 = 32;
pub const PAPER_START_48: u32 = 14335;

/// 128K / grey +2 PAL frame constants.
pub const T_LINE_128: u32 = 228;
pub const LINES_128: u32 = 311;
pub const FRAME_TSTATES_128: u32 = T_LINE_128 * LINES_128; // 70908
pub const INT_LENGTH_128: u32 = 36;
pub const PAPER_START_128: u32 = 14361;

/// Contention pattern (48K/128K): delays for T-states within the 8-cycle window.
const CONTENTION: [u32; 8] = [6, 5, 4, 3, 2, 1, 0, 0];

#[inline]
fn contention_delay_params(frame_t: u32, paper_start: u32, t_line: u32) -> u32 {
    if frame_t < paper_start {
        return 0;
    }
    let t = frame_t - paper_start;
    let line = t / t_line;
    if line >= 192 {
        return 0;
    }
    let x = t % t_line;
    if x >= 128 {
        return 0;
    }
    CONTENTION[(x % 8) as usize]
}

/// Delay added when accessing contended memory at `frame_t` (48K timing).
#[must_use]
pub fn contention_delay(frame_t: u32) -> u32 {
    contention_delay_48(frame_t)
}

/// 48K contended-memory delay.
#[must_use]
pub fn contention_delay_48(frame_t: u32) -> u32 {
    contention_delay_params(frame_t, PAPER_START_48, T_LINE_48)
}

/// 128K / grey +2 contended-memory delay (228 T/line, later paper start).
#[must_use]
pub fn contention_delay_128(frame_t: u32) -> u32 {
    contention_delay_params(frame_t, PAPER_START_128, T_LINE_128)
}

#[inline]
fn floating_bus_params(frame_t: u32, screen: &[u8], paper_start: u32, t_line: u32) -> Option<u8> {
    if screen.len() < 6912 {
        return None;
    }
    if frame_t < paper_start {
        return None;
    }
    let t = frame_t - paper_start;
    let line = t / t_line;
    if line >= 192 {
        return None;
    }
    let x = t % t_line;
    if x >= 128 {
        return None;
    }
    let col = (x / 4) as usize;
    let row = line as usize;
    let phase = x % 8;
    let y = row;
    let third = y / 64;
    let yb = y % 8;
    let yo = (y / 8) % 8;
    let bitmap_off = (third * 2048) + (yo * 32) + (yb * 256) + col.min(31);
    let attr_off = 6144 + (row / 8) * 32 + col.min(31);
    match phase {
        0 | 1 => Some(screen[bitmap_off.min(6143)]),
        2 | 3 => Some(screen[attr_off.min(6911)]),
        _ => None,
    }
}

/// Floating bus byte during ULA fetch (48K), if any.
#[must_use]
pub fn floating_bus_byte(frame_t: u32, screen: &[u8]) -> Option<u8> {
    floating_bus_byte_48(frame_t, screen)
}

#[must_use]
pub fn floating_bus_byte_48(frame_t: u32, screen: &[u8]) -> Option<u8> {
    floating_bus_params(frame_t, screen, PAPER_START_48, T_LINE_48)
}

/// Floating bus byte during ULA fetch (128K / grey +2).
#[must_use]
pub fn floating_bus_byte_128(frame_t: u32, screen: &[u8]) -> Option<u8> {
    floating_bus_params(frame_t, screen, PAPER_START_128, T_LINE_128)
}

/// Spectrum RGB for ink/paper (bright).
#[must_use]
pub fn palette_rgb(color: u8, bright: bool) -> [u8; 3] {
    let level = if bright { 0xff } else { 0xd7 };
    let r = if color & 2 != 0 { level } else { 0 };
    let g = if color & 4 != 0 { level } else { 0 };
    let b = if color & 1 != 0 { level } else { 0 };
    if color == 0 && bright {
        [0x00, 0x00, 0x00]
    } else {
        [r, g, b]
    }
}

#[derive(Clone, Debug)]
pub struct Ula48 {
    /// Border changes: (frame_t, color 0–7)
    pub border_events: Vec<(u32, u8)>,
    pub border: u8,
    pub flash_phase: bool,
    pub frame: u64,
}

impl Default for Ula48 {
    fn default() -> Self {
        Self::new()
    }
}

impl Ula48 {
    #[must_use]
    pub fn new() -> Self {
        Self {
            border_events: Vec::new(),
            border: 0,
            flash_phase: false,
            frame: 0,
        }
    }

    pub fn set_border(&mut self, frame_t: u32, color: u8) {
        self.border = color & 7;
        self.border_events.push((frame_t, self.border));
    }

    pub fn end_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
        if self.frame.is_multiple_of(16) {
            self.flash_phase = !self.flash_phase;
        }
        self.border_events.clear();
        self.border_events.push((0, self.border));
    }

    /// Render 352×296 (with border) or paper-only 256×192 into RGBA8.
    pub fn render_rgba(&self, screen: &[u8], out: &mut [u8], with_border: bool) {
        let (w, h, ox, oy) = if with_border {
            (352usize, 296usize, 48usize, 48usize)
        } else {
            (256, 192, 0, 0)
        };
        assert!(out.len() >= w * h * 4);
        let border_rgb = palette_rgb(self.border, false);
        // fill border
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 4;
                out[i] = border_rgb[0];
                out[i + 1] = border_rgb[1];
                out[i + 2] = border_rgb[2];
                out[i + 3] = 255;
            }
        }
        if screen.len() < 6912 {
            return;
        }
        for py in 0..192usize {
            let third = py / 64;
            let yb = py % 8;
            let yo = (py / 8) % 8;
            for px in 0..256usize {
                let col = px / 8;
                let bit = 7 - (px % 8);
                let bitmap_off = (third * 2048) + (yo * 32) + (yb * 256) + col;
                let attr_off = 6144 + (py / 8) * 32 + col;
                let bits = screen[bitmap_off];
                let attr = screen[attr_off];
                let mut ink = attr & 7;
                let mut paper = (attr >> 3) & 7;
                let bright = attr & 0x40 != 0;
                if attr & 0x80 != 0 && self.flash_phase {
                    core::mem::swap(&mut ink, &mut paper);
                }
                let on = bits & (1 << bit) != 0;
                let rgb = palette_rgb(if on { ink } else { paper }, bright);
                let x = ox + px;
                let y = oy + py;
                let i = (y * w + x) * 4;
                out[i] = rgb[0];
                out[i + 1] = rgb[1];
                out[i + 2] = rgb[2];
                out[i + 3] = 255;
            }
        }
        // Apply border events coarsely by scanline for beam-ish accuracy
        if with_border && self.border_events.len() > 1 {
            for &(t, col) in &self.border_events {
                let rgb = palette_rgb(col, false);
                let line = (t / T_LINE_48) as usize;
                if line < h {
                    for x in 0..w {
                        let in_paper = line >= oy && line < oy + 192 && x >= ox && x < ox + 256;
                        if !in_paper {
                            let i = (line * w + x) * 4;
                            out[i] = rgb[0];
                            out[i + 1] = rgb[1];
                            out[i + 2] = rgb[2];
                        }
                    }
                }
            }
        }
    }
}

#[must_use]
pub fn int_active_48(frame_t: u32) -> bool {
    frame_t < INT_LENGTH_48
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_constants() {
        assert_eq!(FRAME_TSTATES_48, 69888);
        assert_eq!(FRAME_TSTATES_128, 70908);
    }

    #[test]
    fn contention_table() {
        assert_eq!(contention_delay(0), 0);
        let t = PAPER_START_48;
        assert_eq!(contention_delay(t), 6);
        assert_eq!(contention_delay(t + 1), 5);
        assert_eq!(contention_delay(t + 7), 0);
    }

    #[test]
    fn contention_table_128_uses_228_line() {
        assert_eq!(contention_delay_128(0), 0);
        assert_eq!(
            contention_delay_128(PAPER_START_48),
            0,
            "48K paper start is still idle on 128"
        );
        let t = PAPER_START_128;
        assert_eq!(contention_delay_128(t), 6);
        assert_eq!(contention_delay_128(t + 1), 5);
        // Past 128 T into the 228-T line → no contention
        assert_eq!(contention_delay_128(t + 128), 0);
        // Next display line starts 228 T later
        assert_eq!(contention_delay_128(t + T_LINE_128), 6);
    }

    #[test]
    fn floating_bus_128_active_in_paper() {
        let mut screen = vec![0u8; 6912];
        screen[0] = 0xA5;
        assert_eq!(floating_bus_byte_128(PAPER_START_128, &screen), Some(0xA5));
        assert_eq!(floating_bus_byte_128(0, &screen), None);
    }

    #[test]
    fn render_smoke() {
        let ula = Ula48::new();
        let screen = vec![0u8; 6912];
        let mut out = vec![0u8; 256 * 192 * 4];
        ula.render_rgba(&screen, &mut out, false);
        assert_eq!(out[3], 255);
    }
}
