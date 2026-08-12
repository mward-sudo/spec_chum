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

    /// Begin a new frame: advance flash phase and reset border event log.
    ///
    /// Call at the **start** of frame emulation so the previous frame’s events
    /// remain available for `render_rgba` until the next frame begins.
    pub fn begin_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
        if self.frame.is_multiple_of(16) {
            self.flash_phase = !self.flash_phase;
        }
        self.border_events.clear();
        self.border_events.push((0, self.border));
    }

    /// Deprecated alias — prefer `begin_frame` at frame start.
    pub fn end_frame(&mut self) {
        self.begin_frame();
    }

    /// Border colour active at frame T-state `t`.
    #[must_use]
    pub fn border_at(&self, t: u32) -> u8 {
        let mut col = self.border_events.first().map_or(self.border, |&(_, c)| c);
        for &(et, c) in &self.border_events {
            if et <= t {
                col = c;
            }
        }
        col
    }

    /// Frame T-state when framebuffer pixel `(x, y)` is painted (with-border mode).
    ///
    /// Uses 2 pixels/T horizontally. Paper origin aligns with `paper_start` at
    /// `(ox, oy)` so mid-line border changes match ULA beam timing.
    #[must_use]
    pub fn pixel_tstate(
        x: usize,
        y: usize,
        _ox: usize,
        oy: usize,
        paper_start: u32,
        t_line: u32,
    ) -> u32 {
        let line_base = if y < oy {
            paper_start.saturating_sub(((oy - y) as u32).saturating_mul(t_line))
        } else {
            paper_start.saturating_add(((y - oy) as u32).saturating_mul(t_line))
        };
        line_base.saturating_add((x as u32) / 2)
    }

    /// Render 352×296 (with border) or paper-only 256×192 into RGBA8 (48K timing).
    pub fn render_rgba(&self, screen: &[u8], out: &mut [u8], with_border: bool) {
        self.render_rgba_timed(screen, out, with_border, PAPER_START_48, T_LINE_48);
    }

    /// Render with explicit line timing (48K: 224, 128K/+2: 228).
    pub fn render_rgba_timed(
        &self,
        screen: &[u8],
        out: &mut [u8],
        with_border: bool,
        paper_start: u32,
        t_line: u32,
    ) {
        let (w, h, ox, oy) = if with_border {
            (352usize, 296usize, 48usize, 48usize)
        } else {
            (256, 192, 0, 0)
        };
        assert!(out.len() >= w * h * 4);

        if with_border {
            // Per-pixel border from beam T-state (mid-line / per-byte accurate).
            for y in 0..h {
                for x in 0..w {
                    let in_paper = (oy..oy + 192).contains(&y) && (ox..ox + 256).contains(&x);
                    if in_paper {
                        continue;
                    }
                    let t = Self::pixel_tstate(x, y, ox, oy, paper_start, t_line);
                    let rgb = palette_rgb(self.border_at(t), false);
                    let i = (y * w + x) * 4;
                    out[i] = rgb[0];
                    out[i + 1] = rgb[1];
                    out[i + 2] = rgb[2];
                    out[i + 3] = 255;
                }
            }
        } else {
            let border_rgb = palette_rgb(self.border, false);
            for i in (0..w * h * 4).step_by(4) {
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

    #[test]
    fn mid_line_border_change_splits_scanline() {
        let mut ula = Ula48::new();
        ula.border = 1; // blue
        ula.border_events = vec![(0, 1)];
        // Change to red mid-way through paper line 0 (framebuffer y = 48).
        // Paper starts at x=48; mid paper ~ x=48+128 → t ≈ PAPER_START + 64.
        let t_mid = PAPER_START_48 + 64;
        ula.set_border(t_mid, 2); // red

        let screen = vec![0u8; 6912];
        let mut out = vec![0u8; 352 * 296 * 4];
        ula.render_rgba(&screen, &mut out, true);

        let y = 48usize; // first paper line — left/right border on same line
        let left_i = (y * 352 + 10) * 4; // left border
        let right_i = (y * 352 + 340) * 4; // right border
        let blue = palette_rgb(1, false);
        let red = palette_rgb(2, false);
        assert_eq!(&out[left_i..left_i + 3], &blue);
        assert_eq!(
            &out[right_i..right_i + 3],
            &red,
            "right border after mid-line OUT should be new colour"
        );
    }

    #[test]
    fn mid_line_border_128_uses_228_pitch() {
        let mut ula = Ula48::new();
        ula.border = 0;
        ula.border_events = vec![(0, 0)];
        // Top border line y=10: line_base = PAPER_START_128 - (48-10)*228
        let y = 10usize;
        let line_base =
            PAPER_START_128.saturating_sub(((48 - y) as u32).saturating_mul(T_LINE_128));
        ula.set_border(line_base + 40, 4); // green after x≈80

        let screen = vec![0u8; 6912];
        let mut out = vec![0u8; 352 * 296 * 4];
        ula.render_rgba_timed(&screen, &mut out, true, PAPER_START_128, T_LINE_128);

        let before_i = (y * 352 + 60) * 4; // x/2 = 30
        let after_i = (y * 352 + 100) * 4; // x/2 = 50
        let black = palette_rgb(0, false);
        let green = palette_rgb(4, false);
        assert_eq!(&out[before_i..before_i + 3], &black);
        assert_eq!(&out[after_i..after_i + 3], &green);
    }
}
