//! AY-3-8912 PSG synthesis for Spectrum 128K / +2.
//!
//! Driven from register writes plus a CPU T-state timebase. On the 128K the chip
//! is clocked at CPU/2; tone/noise/envelope counters advance every 16 AY clocks
//! (32 CPU T-states). Stereo pan modes: mono, ACB, and ABC.
//!
//! Shortcuts:
//! - Fixed 16-step logarithmic volume table (no DC blocking filter).
//! - Envelope resolution follows the classic 16-step saw/triangle shapes.
//! - Output is a simple average / pan of the three channels (no non-linear DAC mix).

/// Logarithmic volume levels approximating the AY DAC (0..1).
const VOL_TABLE: [f32; 16] = [
    0.0000, 0.0137, 0.0205, 0.0291, 0.0426, 0.0562, 0.0811, 0.1078, 0.1580, 0.2100, 0.2960, 0.3800,
    0.5400, 0.7000, 0.9100, 1.0000,
];

/// AY channel pan layout for stereo output.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StereoMode {
    /// Same sample on L and R (equal mix of A/B/C).
    #[default]
    Mono,
    /// A left, C center, B right (common East-European / Russian wiring).
    Acb,
    /// A left, B center, C right (Melodik / Western wiring).
    Abc,
}

/// Envelope shape lookup: `step` is 0..31 (two 16-step halves).
fn envelope_level(shape: u8, step: u8) -> u8 {
    let shape = shape & 0x0f;
    let s = u32::from(step & 31);
    let cont = shape & 8 != 0;
    let attack = shape & 4 != 0;
    let alt = shape & 2 != 0;
    let hold = shape & 1 != 0;

    if !cont {
        if s < 16 {
            let v = if attack { s } else { 15 - s };
            return v as u8;
        }
        return 0;
    }

    let cycle = s / 16;
    let pos = s % 16;
    let mut up = attack;
    if alt && cycle % 2 == 1 {
        up = !up;
    }
    if hold && cycle >= 1 {
        return if (attack && !alt) || (!attack && alt) {
            15
        } else {
            0
        };
    }
    if up {
        pos as u8
    } else {
        (15 - pos) as u8
    }
}

/// Mix left / center / right channel amplitudes into stereo (center split 50/50).
fn pan_mix(left: f32, center: f32, right: f32) -> (f32, f32) {
    let l = left + center * 0.5;
    let r = right + center * 0.5;
    (l / 1.5, r / 1.5)
}

#[derive(Clone, Debug)]
pub struct Ay8912 {
    pub regs: [u8; 16],
    pub selected: u8,
    pub stereo_mode: StereoMode,
    tone_count: [u16; 3],
    tone_out: [bool; 3],
    noise_count: u16,
    noise_lfsr: u32,
    noise_out: bool,
    env_count: u16,
    env_step: u8,
    env_holding: bool,
    /// Accumulated CPU T-states toward the next AY internal tick (32 T).
    tick_accum: u32,
}

impl Default for Ay8912 {
    fn default() -> Self {
        Self::new()
    }
}

impl Ay8912 {
    #[must_use]
    pub fn new() -> Self {
        Self {
            regs: [0; 16],
            selected: 0,
            stereo_mode: StereoMode::Mono,
            tone_count: [0; 3],
            tone_out: [false; 3],
            noise_count: 0,
            noise_lfsr: 1,
            noise_out: false,
            env_count: 0,
            env_step: 0,
            env_holding: false,
            tick_accum: 0,
        }
    }

    pub fn reset(&mut self) {
        let mode = self.stereo_mode;
        *self = Self::new();
        self.stereo_mode = mode;
    }

    pub fn select(&mut self, reg: u8) {
        self.selected = reg & 0x0f;
    }

    pub fn write_data(&mut self, value: u8) {
        let r = usize::from(self.selected);
        let mut v = value;
        match r {
            0 | 2 | 4 => {}
            1 | 3 | 5 => v &= 0x0f,
            6 => v &= 0x1f,
            7 => {}
            8..=10 => v &= 0x1f,
            11 | 12 => {}
            13 => {
                v &= 0x0f;
                self.env_step = 0;
                self.env_holding = false;
                self.env_count = 0;
            }
            14 | 15 => {}
            _ => {}
        }
        self.regs[r] = v;
    }

    #[must_use]
    pub fn read_data(&self) -> u8 {
        self.regs[usize::from(self.selected)]
    }

    fn tone_period(&self, ch: usize) -> u16 {
        let fine = u16::from(self.regs[ch * 2]);
        let coarse = u16::from(self.regs[ch * 2 + 1] & 0x0f);
        let p = fine | (coarse << 8);
        if p == 0 {
            1
        } else {
            p
        }
    }

    fn noise_period(&self) -> u16 {
        let p = u16::from(self.regs[6] & 0x1f);
        if p == 0 {
            1
        } else {
            p
        }
    }

    fn env_period(&self) -> u16 {
        let p = u16::from(self.regs[11]) | (u16::from(self.regs[12]) << 8);
        if p == 0 {
            1
        } else {
            p
        }
    }

    fn channel_volume(&self, ch: usize) -> f32 {
        let amp = self.regs[8 + ch];
        let level = if amp & 0x10 != 0 {
            envelope_level(self.regs[13], self.env_step)
        } else {
            amp & 0x0f
        };
        VOL_TABLE[usize::from(level)]
    }

    /// Per-channel instantaneous amplitudes (mixer applied), roughly `0.0..1.0`.
    #[must_use]
    pub fn channel_levels(&self) -> [f32; 3] {
        let mixer = self.regs[7];
        let mut levels = [0.0f32; 3];
        for (ch, level) in levels.iter_mut().enumerate() {
            let tone_off = mixer & (1 << ch) != 0;
            let noise_off = mixer & (1 << (ch + 3)) != 0;
            let tone = self.tone_out[ch];
            let noise = self.noise_out;
            let enabled = match (tone_off, noise_off) {
                (true, true) => false,
                (false, true) => tone,
                (true, false) => noise,
                (false, false) => tone && noise,
            };
            let vol = self.channel_volume(ch);
            *level = if enabled { vol } else { 0.0 };
        }
        levels
    }

    /// Advance synthesis by `tstates` CPU T-states.
    pub fn advance(&mut self, tstates: u32) {
        if tstates == 0 {
            return;
        }
        self.tick_accum += tstates;
        while self.tick_accum >= 32 {
            self.tick_accum -= 32;
            self.tick_once();
        }
    }

    fn tick_once(&mut self) {
        for ch in 0..3 {
            self.tone_count[ch] = self.tone_count[ch].wrapping_add(1);
            if self.tone_count[ch] >= self.tone_period(ch) {
                self.tone_count[ch] = 0;
                self.tone_out[ch] = !self.tone_out[ch];
            }
        }

        self.noise_count = self.noise_count.wrapping_add(1);
        if self.noise_count >= self.noise_period() {
            self.noise_count = 0;
            let bit0 = self.noise_lfsr & 1;
            let bit3 = (self.noise_lfsr >> 3) & 1;
            self.noise_lfsr = (self.noise_lfsr >> 1) | ((bit0 ^ bit3) << 16);
            self.noise_out = self.noise_lfsr & 1 != 0;
        }

        if self.env_holding {
            return;
        }
        self.env_count = self.env_count.wrapping_add(1);
        if self.env_count < self.env_period() {
            return;
        }
        self.env_count = 0;
        let shape = self.regs[13] & 0x0f;
        if shape & 8 == 0 {
            if self.env_step < 31 {
                self.env_step += 1;
            }
        } else if shape & 1 != 0 {
            if self.env_step < 15 {
                self.env_step += 1;
            } else {
                self.env_holding = true;
                self.env_step = 16;
            }
        } else {
            self.env_step = self.env_step.wrapping_add(1) & 31;
        }
    }

    /// Current mono mix sample in roughly `0.0..1.0` (uncentered channel sum / 3).
    #[must_use]
    pub fn sample_mono(&self) -> f32 {
        let levels = self.channel_levels();
        (levels[0] + levels[1] + levels[2]) / 3.0
    }

    /// Stereo sample using [`Self::stereo_mode`]. Amplitudes roughly `0.0..1.0`.
    #[must_use]
    pub fn sample_stereo(&self) -> (f32, f32) {
        self.sample_stereo_mode(self.stereo_mode)
    }

    /// Stereo sample for an explicit pan mode.
    #[must_use]
    pub fn sample_stereo_mode(&self, mode: StereoMode) -> (f32, f32) {
        let levels = self.channel_levels();
        match mode {
            StereoMode::Mono => {
                let m = (levels[0] + levels[1] + levels[2]) / 3.0;
                (m, m)
            }
            // A left, C center, B right
            StereoMode::Acb => pan_mix(levels[0], levels[2], levels[1]),
            // A left, B center, C right
            StereoMode::Abc => pan_mix(levels[0], levels[1], levels[2]),
        }
    }

    /// Render `sample_count` mono samples spanning `frame_tstates` CPU T-states.
    pub fn render_frame(&mut self, frame_tstates: u32, sample_count: usize) -> Vec<f32> {
        let mut out = Vec::with_capacity(sample_count);
        if sample_count == 0 {
            return out;
        }
        let t_per = f64::from(frame_tstates) / sample_count as f64;
        let mut last = 0u32;
        for i in 0..sample_count {
            let target = ((i as f64 + 1.0) * t_per) as u32;
            let dt = target.saturating_sub(last);
            last = target;
            self.advance(dt);
            out.push(self.sample_mono());
        }
        if last < frame_tstates {
            self.advance(frame_tstates - last);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tone_period_register_roundtrip() {
        let mut ay = Ay8912::new();
        ay.select(0);
        ay.write_data(0x10);
        ay.select(1);
        ay.write_data(0x02);
        assert_eq!(ay.tone_period(0), 0x0210);
    }

    #[test]
    fn mixer_mute_silence() {
        let mut ay = Ay8912::new();
        ay.select(0);
        ay.write_data(1);
        ay.select(8);
        ay.write_data(0x0f);
        ay.select(7);
        ay.write_data(0x3f);
        ay.advance(32 * 100);
        let s = ay.sample_mono();
        assert!(s.abs() < 1e-3, "muted mixer should be silent, got {s}");
    }

    #[test]
    fn acb_vs_abc_pan_differs() {
        let mut ay = Ay8912::new();
        // Enable only channel A (tone) at full volume.
        ay.select(0);
        ay.write_data(1);
        ay.select(8);
        ay.write_data(0x0f);
        ay.select(7);
        ay.write_data(0x38); // tone A on, B/C off, noise off
        ay.tone_out = [true, false, false];
        ay.stereo_mode = StereoMode::Acb;
        let (l_acb, r_acb) = ay.sample_stereo();
        ay.stereo_mode = StereoMode::Abc;
        let (l_abc, r_abc) = ay.sample_stereo();
        assert!(l_acb > r_acb, "ACB: A should favour left");
        assert!(l_abc > r_abc, "ABC: A should favour left");
        // With only A, ACB and ABC left should match; differ when B vs C.
        ay.tone_out = [false, true, false];
        ay.select(9);
        ay.write_data(0x0f);
        ay.select(8);
        ay.write_data(0);
        ay.stereo_mode = StereoMode::Acb;
        let (_, r_b) = ay.sample_stereo();
        ay.stereo_mode = StereoMode::Abc;
        let (l_b, _) = ay.sample_stereo();
        assert!(r_b > 0.01 && l_b > 0.01);
    }

    #[test]
    fn tone_produces_nonzero_energy() {
        let mut ay = Ay8912::new();
        ay.select(0);
        ay.write_data(8);
        ay.select(1);
        ay.write_data(0);
        ay.select(8);
        ay.write_data(0x0f);
        ay.select(7);
        ay.write_data(0x38);
        let samples = ay.render_frame(70_908, 256);
        let energy: f32 = samples.iter().map(|s| s * s).sum();
        assert!(energy > 0.01, "tone should produce energy, got {energy}");
    }

    #[test]
    fn envelope_write_restarts() {
        let mut ay = Ay8912::new();
        ay.select(13);
        ay.write_data(0x0e);
        ay.env_step = 20;
        ay.select(13);
        ay.write_data(0x08);
        assert_eq!(ay.env_step, 0);
        assert!(!ay.env_holding);
    }

    #[test]
    fn envelope_shape_attack_levels() {
        assert_eq!(envelope_level(0x0e, 0), 0);
        assert_eq!(envelope_level(0x0e, 15), 15);
        assert_eq!(envelope_level(0x0e, 16), 15);
        assert_eq!(envelope_level(0x0e, 31), 0);
    }

    /// Force channel B (index 1) full-on with tone stuck high for pan checks.
    fn ay_channel_b_only() -> Ay8912 {
        let mut ay = Ay8912::new();
        ay.tone_out = [false, true, false];
        ay.select(9); // volume B
        ay.write_data(0x0f);
        ay.select(7);
        ay.write_data(0x3d); // tone off A+C, tone on B; noise off all
        ay
    }

    #[test]
    fn acb_vs_abc_swap_b_and_c_pans() {
        let ay = ay_channel_b_only();
        let (acb_l, acb_r) = ay.sample_stereo_mode(StereoMode::Acb);
        let (abc_l, abc_r) = ay.sample_stereo_mode(StereoMode::Abc);
        // ACB: B is right → more energy on R than L.
        assert!(
            acb_r > acb_l + 0.1,
            "ACB should pan B right: L={acb_l} R={acb_r}"
        );
        // ABC: B is center → equal L/R.
        assert!(
            (abc_l - abc_r).abs() < 1e-4,
            "ABC should center B: L={abc_l} R={abc_r}"
        );

        // Channel C only: ACB centers C, ABC pans C right.
        let mut ay_c = Ay8912::new();
        ay_c.tone_out = [false, false, true];
        ay_c.select(10);
        ay_c.write_data(0x0f);
        ay_c.select(7);
        ay_c.write_data(0x3b); // tone on C only
        let (acb_l, acb_r) = ay_c.sample_stereo_mode(StereoMode::Acb);
        let (abc_l, abc_r) = ay_c.sample_stereo_mode(StereoMode::Abc);
        assert!(
            (acb_l - acb_r).abs() < 1e-4,
            "ACB should center C: L={acb_l} R={acb_r}"
        );
        assert!(
            abc_r > abc_l + 0.1,
            "ABC should pan C right: L={abc_l} R={abc_r}"
        );
    }

    #[test]
    fn mono_stereo_matches_sample_mono() {
        let mut ay = Ay8912::new();
        ay.select(0);
        ay.write_data(8);
        ay.select(8);
        ay.write_data(0x0f);
        ay.select(7);
        ay.write_data(0x38);
        ay.advance(32 * 50);
        ay.stereo_mode = StereoMode::Mono;
        let m = ay.sample_mono();
        let (l, r) = ay.sample_stereo();
        assert!((l - m).abs() < 1e-6 && (r - m).abs() < 1e-6);
    }
}
