//! Spec Chum ULA — frame timing, contention, floating bus, video render.

#![allow(clippy::pedantic)]

/// 48K PAL frame constants.
pub const T_LINE_48: u32 = 224;
pub const LINES_48: u32 = 312;
pub const FRAME_TSTATES_48: u32 = T_LINE_48 * LINES_48; // 69888
pub const INT_LENGTH_48: u32 = 32;
/// First ULA contended cycle after INT (48K PAL, early timing, INT low = T0).
pub const PAPER_START_48: u32 = 14335;

/// 128K / grey +2 PAL frame constants.
pub const T_LINE_128: u32 = 228;
pub const LINES_128: u32 = 311;
pub const FRAME_TSTATES_128: u32 = T_LINE_128 * LINES_128; // 70908
pub const INT_LENGTH_128: u32 = 36;
pub const PAPER_START_128: u32 = 14361;

/// Pentagon 128 PAL frame constants (320×224 T-states; no memory contention).
pub const T_LINE_PENTAGON: u32 = 224;
pub const LINES_PENTAGON: u32 = 320;
pub const FRAME_TSTATES_PENTAGON: u32 = T_LINE_PENTAGON * LINES_PENTAGON; // 71680
pub const INT_LENGTH_PENTAGON: u32 = 32;

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
    // Contention starts at `paper_start`; first display fetch is 3T later (wiki:
    // 48K early-timing bitmap at 14338 when paper/contention starts at 14335).
    if !(3..131).contains(&x) {
        return None;
    }
    let xf = x - 3;
    // 8T window: bm, at, bm+1, at+1, idle×4 — two character columns per window.
    let phase = xf % 8;
    if phase >= 4 {
        return None;
    }
    let col = ((xf / 8) * 2 + phase / 2) as usize;
    if col > 31 {
        return None;
    }
    let row = line as usize;
    let y = row;
    let third = y / 64;
    let yb = y % 8;
    let yo = (y / 8) % 8;
    let is_attr = phase % 2 == 1;
    if is_attr {
        let attr_off = 6144 + (row / 8) * 32 + col;
        Some(screen[attr_off.min(6911)])
    } else {
        let bitmap_off = (third * 2048) + (yo * 32) + (yb * 256) + col;
        Some(screen[bitmap_off.min(6143)])
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

/// True when a port high byte falls in the 48K contended RAM window (`0x40xx`–`0x7Fxx`).
#[must_use]
#[inline]
pub fn port_high_contended_48(port: u16) -> bool {
    (0x4000..0x8000).contains(&(port & 0xff00))
}

/// True when a 128K/grey-+2 port high byte is contended.
///
/// High bytes `0x40`–`0x7F` always contend. High bytes `0xC0`–`0xFF` contend only
/// when a contended RAM bank (1/3/5/7) is paged at `0xC000`.
#[must_use]
#[inline]
pub fn port_high_contended_128(port: u16, c000_bank_contended: bool) -> bool {
    let hi = port & 0xff00;
    if (0x4000..0x8000).contains(&hi) {
        true
    } else if hi >= 0xc000 {
        c000_bank_contended
    } else {
        false
    }
}

/// Extra T-states from 48K ULA I/O contention (excluding the base 4T of IN/OUT).
///
/// Sinclair FAQ Contended I/O patterns:
/// - high not contended, ULA (`A0=0`): `N:1, C:3`
/// - high not contended, not ULA: `N:4`
/// - high contended, ULA: `C:1, C:3`
/// - high contended, not ULA: `C:1` × 4
#[must_use]
pub fn io_contention_extra_48(frame_t: u32, port: u16) -> u32 {
    io_contention_extra(
        frame_t,
        port,
        port_high_contended_48(port),
        contention_delay_48,
    )
}

/// 128K / grey +2 I/O contention extra (same FAQ patterns; `c000_bank_contended`
/// enables high-byte `0xC0`–`0xFF` contention when bank 1/3/5/7 is at `C000`).
#[must_use]
pub fn io_contention_extra_128(frame_t: u32, port: u16, c000_bank_contended: bool) -> u32 {
    io_contention_extra(
        frame_t,
        port,
        port_high_contended_128(port, c000_bank_contended),
        contention_delay_128,
    )
}

fn io_contention_extra(
    mut frame_t: u32,
    port: u16,
    high_contended: bool,
    delay: fn(u32) -> u32,
) -> u32 {
    let ula_port = port & 1 == 0;
    let mut extra = 0u32;
    let contend_then = |steps: u32, ft: &mut u32, extra: &mut u32| {
        let d = delay(*ft);
        *extra += d;
        *ft = ft.wrapping_add(d).wrapping_add(steps);
    };
    match (high_contended, ula_port) {
        (false, true) => {
            frame_t = frame_t.wrapping_add(1);
            contend_then(3, &mut frame_t, &mut extra);
        }
        (false, false) => {}
        (true, true) => {
            contend_then(1, &mut frame_t, &mut extra);
            contend_then(3, &mut frame_t, &mut extra);
        }
        (true, false) => {
            for _ in 0..4 {
                contend_then(1, &mut frame_t, &mut extra);
            }
        }
    }
    extra
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

/// Pentagon 128 INT window (32 T-states at frame start; Unreal / JC test class).
#[must_use]
pub fn int_active_pentagon(frame_t: u32) -> bool {
    frame_t < INT_LENGTH_PENTAGON
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_constants() {
        assert_eq!(FRAME_TSTATES_48, 69888);
        assert_eq!(FRAME_TSTATES_128, 70908);
        assert_eq!(FRAME_TSTATES_PENTAGON, 71680);
    }

    #[test]
    fn contention_table() {
        assert_eq!(contention_delay(0), 0);
        let t = PAPER_START_48;
        // Full early-timing 8-cycle window (FAQ / Sinclair wiki).
        const DELAYS: [u32; 8] = [6, 5, 4, 3, 2, 1, 0, 0];
        for (i, &d) in DELAYS.iter().enumerate() {
            assert_eq!(contention_delay(t + i as u32), d, "48K delay at PAPER+{i}");
        }
        assert_eq!(contention_delay(t + 8), 6, "pattern repeats next window");
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
    fn floating_bus_48_fetch_pairs() {
        let mut screen = vec![0u8; 6912];
        screen[0] = 0x10;
        screen[1] = 0x11;
        screen[6144] = 0xA0;
        screen[6145] = 0xA1;
        // First fetch is paper_start+3: bm0, at0, bm1, at1, then idle.
        let t0 = PAPER_START_48 + 3;
        assert_eq!(floating_bus_byte_48(t0, &screen), Some(0x10));
        assert_eq!(floating_bus_byte_48(t0 + 1, &screen), Some(0xA0));
        assert_eq!(floating_bus_byte_48(t0 + 2, &screen), Some(0x11));
        assert_eq!(floating_bus_byte_48(t0 + 3, &screen), Some(0xA1));
        assert_eq!(floating_bus_byte_48(t0 + 4, &screen), None);
        assert_eq!(floating_bus_byte_48(PAPER_START_48, &screen), None);
    }

    #[test]
    fn floating_bus_128_active_in_paper() {
        let mut screen = vec![0u8; 6912];
        screen[0] = 0xA5;
        assert_eq!(
            floating_bus_byte_128(PAPER_START_128 + 3, &screen),
            Some(0xA5)
        );
        assert_eq!(floating_bus_byte_128(0, &screen), None);
    }

    #[test]
    fn io_contention_patterns_48() {
        let t = PAPER_START_48;
        assert_eq!(io_contention_extra_48(t, 0x00fe), 5);
        assert_eq!(io_contention_extra_48(t, 0x00ff), 0);
        assert_eq!(io_contention_extra_48(t, 0x40fe), 6);
        let mut expect = 0u32;
        let mut ft = t;
        for _ in 0..4 {
            let d = contention_delay_48(ft);
            expect += d;
            ft = ft.wrapping_add(d).wrapping_add(1);
        }
        assert_eq!(io_contention_extra_48(t, 0x40ff), expect);
    }

    #[test]
    fn io_contention_128_c000_depends_on_bank() {
        let t = PAPER_START_128;
        assert!(!port_high_contended_128(0xc0fe, false));
        assert!(port_high_contended_128(0xc0fe, true));
        assert_eq!(io_contention_extra_128(t, 0xc0fe, false), 5);
        assert_eq!(io_contention_extra_128(t, 0xc0fe, true), 6);
        assert_eq!(io_contention_extra_128(t, 0x40fe, false), 6);
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
