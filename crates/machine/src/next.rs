//! Spectrum Next CPU/MMU and classic ULA display for the verified boot path.

use bus::NextBus;
use std::path::Path;
use z80::{Cpu, CpuProfile, Io, Memory};

use crate::FrameAudio;
use crate::MachineBuildError;
use crate::{apply_joystick, JoystickMode, JoystickState};
use ula::Ula48;

/// Core-constructible Spectrum Next with Z80N instructions and an eight-slot
/// 8 KiB MMU. The host owns its verified SD boot path separately from classic machines.
#[derive(Debug)]
pub struct NextMachine {
    pub cpu: Cpu,
    pub bus: NextBus,
    /// One monotonic 28 MHz clock shared by the CPU, CTC, and video timeline.
    master_t: u64,
    ula: Ula48,
    debugger_paused: bool,
    beeper_level: bool,
    beeper_edges: Vec<(u64, bool)>,
    audio_phase: u64,
    ay_samples: Vec<f32>,
    ay_left: Vec<f32>,
    ay_right: Vec<f32>,
    audio_muted_samples: Vec<bool>,
    dac_samples: Vec<[f32; 4]>,
    pending_dac_writes: Vec<(u64, [u8; 4], bool)>,
    dac_sample_values: [u8; 4],
    dac_sample_enabled: bool,
    copper_video_frame: Option<u64>,
    render_frame: Option<u64>,
    copper_video_initial: Option<bus::NextBusVideoState>,
    copper_video_events: Vec<(u32, bus::NextBusVideoState)>,
}

impl NextMachine {
    /// Supply the 64 KiB system ROM image explicitly; firmware acquisition is
    /// handled by the host, not by core construction.
    pub fn new(rom: &[u8]) -> Result<Self, MachineBuildError> {
        Ok(Self {
            cpu: Cpu::with_profile(CpuProfile::Z80N),
            bus: NextBus::new(rom)?,
            master_t: 0,
            ula: Ula48::new(),
            debugger_paused: false,
            beeper_level: false,
            beeper_edges: Vec::new(),
            audio_phase: 0,
            ay_samples: Vec::new(),
            ay_left: Vec::new(),
            ay_right: Vec::new(),
            audio_muted_samples: Vec::new(),
            dac_samples: Vec::new(),
            pending_dac_writes: Vec::new(),
            dac_sample_values: [0x80; 4],
            dac_sample_enabled: false,
            copper_video_frame: None,
            render_frame: None,
            copper_video_initial: None,
            copper_video_events: Vec::new(),
        })
    }

    /// Install the FPGA IPL image that overlays address `$0000` until config
    /// mode is left through `NextReg` `$03`.
    pub fn install_ipl(&mut self, bytes: &[u8]) -> Result<(), MachineBuildError> {
        self.bus.install_boot_rom(bytes)?;
        self.cpu.reset();
        self.master_t = 0;
        self.beeper_level = false;
        self.beeper_edges.clear();
        self.audio_phase = 0;
        self.ay_samples.clear();
        self.ay_left.clear();
        self.ay_right.clear();
        self.audio_muted_samples.clear();
        self.dac_samples.clear();
        self.pending_dac_writes.clear();
        self.dac_sample_values = self.bus.dac_values();
        self.dac_sample_enabled = self.bus.dacs_enabled();
        self.clear_copper_video_history();
        Ok(())
    }

    /// Attach a file-backed Next SD card image without loading it into memory.
    pub fn attach_sd_image(&mut self, path: &Path) -> Result<(), bus::NextSdError> {
        self.bus.attach_sd_file(path)
    }

    /// Soft reset the CPU and MMU without erasing RAM, ROM or machine selection.
    pub fn reset(&mut self) {
        self.cpu.reset();
        self.bus.soft_reset();
        self.bus.advance_ctc_master(self.master_t);
        self.audio_phase = 0;
        self.ay_samples.clear();
        self.ay_left.clear();
        self.ay_right.clear();
        self.audio_muted_samples.clear();
        self.dac_samples.clear();
        self.pending_dac_writes.clear();
        self.dac_sample_values = self.bus.dac_values();
        self.dac_sample_enabled = self.bus.dacs_enabled();
        self.clear_copper_video_history();
    }

    /// Uninterrupted video clock, including across CPU soft resets.
    #[must_use]
    pub fn video_t(&self) -> u64 {
        self.master_t / 8
    }

    /// T-states per frame for the selected Next display timing.
    #[must_use]
    pub fn frame_tstates(&self) -> u32 {
        self.bus.frame_interrupt_timing().0 as u32
    }

    /// Pause host-driven frame advancement without changing the guest machine state.
    pub fn set_paused(&mut self, paused: bool) {
        self.debugger_paused = paused;
    }

    #[must_use]
    pub fn paused(&self) -> bool {
        self.debugger_paused
    }

    /// Execute one guest instruction through the Next-specific bus.
    pub fn step_once(&mut self) -> u32 {
        self.bus.adopt_cpu_speed();
        let cpu_t_master_ticks = self.bus.cpu_t_master_ticks();
        let step_start_master = self.master_t;
        let cpu_step_start = self.cpu.t;
        let step_start = self.video_t();
        self.bus.set_cpu_interrupt_mode(self.cpu.regs.im);
        let (frame_tstates, interrupt_tstates) = self.bus.frame_interrupt_timing();
        let cycles = {
            let mut io = NextMemIo {
                bus: &mut self.bus,
                step_start_master,
                cpu_step_start,
                cpu_t_master_ticks,
                beeper_level: &mut self.beeper_level,
                beeper_edges: &mut self.beeper_edges,
            };
            if io.bus.take_divmmc_nmi() {
                self.cpu.nmi(&mut io)
            } else if io.bus.ctc_interrupt_pending(self.cpu.regs.im == 2)
                || step_start % frame_tstates < interrupt_tstates
            {
                let interrupt_cycles = self.cpu.interrupt(&mut io);
                if interrupt_cycles > 0 {
                    interrupt_cycles
                } else {
                    self.cpu.step(&mut io)
                }
            } else {
                self.cpu.step(&mut io)
            }
        };
        let elapsed_master = u64::from(cycles) * cpu_t_master_ticks;
        let phase_start = self.audio_phase;
        self.master_t = self.master_t.wrapping_add(elapsed_master);
        let step_end = self.video_t();
        self.bus.advance_ctc_master(self.master_t);
        self.bus.advance_dma(step_end);
        let copper_boundary = (step_start / frame_tstates + 1) * frame_tstates;
        let copper_time = step_end.min(copper_boundary.saturating_sub(1));
        self.render_frame = None;
        for (write_t, register, value) in self.bus.advance_copper(copper_time) {
            self.capture_copper_video_before_write(write_t, register);
            self.bus.write_nextreg_at(register, value, write_t);
            self.capture_copper_video_after_write(write_t, register);
        }
        self.bus.advance_audio((step_end - step_start) as u32);
        self.bus.drain_dac_writes_into(&mut self.pending_dac_writes);
        let mut sample_phase = 28_000_000u64.saturating_sub(phase_start);
        self.audio_phase = self
            .audio_phase
            .saturating_add(elapsed_master.saturating_mul(44_100));
        while self.audio_phase >= 28_000_000 {
            self.audio_phase -= 28_000_000;
            let sample_t = step_start_master.saturating_add(sample_phase.div_ceil(44_100)) / 8;
            sample_phase = sample_phase.saturating_add(28_000_000);
            while self
                .pending_dac_writes
                .first()
                .is_some_and(|(write_t, _, _)| *write_t <= sample_t)
            {
                let (_, values, enabled) = self.pending_dac_writes.remove(0);
                self.dac_sample_values = values;
                self.dac_sample_enabled = enabled;
            }
            let (mono, left, right) = if self.bus.audio_configured() {
                self.bus.audio_sample()
            } else {
                // Host audio backends center AY silence at 0.5 before mapping
                // it to signed PCM, so zero here would introduce a DC offset.
                (0.5, 0.5, 0.5)
            };
            self.ay_samples.push(mono);
            self.ay_left.push(left);
            self.ay_right.push(right);
            self.audio_muted_samples.push(self.bus.audio_muted());
            self.dac_samples.push(if self.dac_sample_enabled {
                self.dac_sample_values.map(|value| f32::from(value) / 256.0)
            } else {
                [0.5; 4]
            });
        }
        if let Some(status) = self.bus.take_reset_request() {
            self.clear_copper_video_history();
            self.cpu.reset();
            if status == 0x01 {
                self.bus.soft_reset();
            } else {
                self.bus.hard_reset();
                if self.beeper_level {
                    self.beeper_level = false;
                    self.beeper_edges.push((self.video_t(), false));
                }
            }
            self.bus.advance_ctc_master(self.master_t);
            self.dac_sample_values = self.bus.dac_values();
            self.dac_sample_enabled = self.bus.dacs_enabled();
            self.pending_dac_writes.clear();
            self.bus.set_reset_status(status);
        }
        cycles
    }

    /// Run to the next selected video-frame boundary.
    pub fn run_frame(&mut self) -> FrameAudio {
        if self.debugger_paused {
            return FrameAudio::default();
        }
        let frame_tstates = self.frame_tstates();
        let frame_len = u64::from(frame_tstates);
        let frame_start = self.video_t();
        let boundary = (frame_start / frame_len + 1) * frame_len;
        while self.video_t() < boundary {
            self.step_once();
        }
        self.render_frame = Some(boundary / frame_len - 1);
        let mut frame_audio = FrameAudio {
            frame_tstates: Some(frame_tstates),
            ay_samples: std::mem::take(&mut self.ay_samples),
            ay_left: std::mem::take(&mut self.ay_left),
            ay_right: std::mem::take(&mut self.ay_right),
            audio_muted_samples: std::mem::take(&mut self.audio_muted_samples),
            dac_samples: std::mem::take(&mut self.dac_samples),
            ..FrameAudio::default()
        };
        let mut future_edges = Vec::new();
        for (edge_t, level) in self.beeper_edges.drain(..) {
            if edge_t < boundary {
                frame_audio
                    .beeper_edges
                    .push((edge_t.saturating_sub(frame_start) as u32, level));
            } else {
                future_edges.push((edge_t, level));
            }
        }
        self.beeper_edges = future_edges;
        frame_audio
    }

    /// Render the ULA screen, standard Layer 2, and base Next sprites.
    pub fn render_rgba(&self, out: &mut [u8], with_border: bool) {
        let frame_tstates = u64::from(self.frame_tstates().max(1));
        let displayed_frame = self
            .render_frame
            .unwrap_or_else(|| self.video_t().saturating_sub(1) / frame_tstates);
        if self.copper_video_frame == Some(displayed_frame) {
            if let Some(initial) = &self.copper_video_initial {
                self.render_rgba_with_copper_history(out, with_border, initial);
                return;
            }
        }
        self.render_rgba_with_bus(out, with_border);
    }

    fn render_rgba_with_bus(&self, out: &mut [u8], with_border: bool) {
        let renderer = self.bus.video_renderer();
        let screen = Self::load_ula_screen(&renderer, false);
        let (width, height) = Self::framebuffer_dims(with_border);
        let mut ula = self.ula.clone();
        ula.border = renderer.border();
        ula.render_rgba(&screen[..6912], out, with_border);
        Self::render_video_layers(&renderer, &screen, out, with_border, 0..height);
        debug_assert_eq!(out.len(), width * height * 4);
    }

    fn load_ula_screen(
        renderer: &bus::NextBusVideoRenderer<'_>,
        include_full_page: bool,
    ) -> [u8; 0x3800] {
        let first_page = renderer.display_screen_bank() * 2;
        let mut screen = [0u8; 0x3800];
        let screen_len = if include_full_page
            || renderer.lores_256_color_enabled()
            || renderer.radastan_lores_enabled()
        {
            screen.len()
        } else {
            6912
        };
        for (offset, byte) in screen[..screen_len].iter_mut().enumerate() {
            *byte = renderer
                .read_ram_page(first_page + (offset / 8192) as u8, offset % 8192)
                .expect("Next ULA bank is within installed physical RAM");
        }
        screen
    }

    fn render_video_layers(
        renderer: &bus::NextBusVideoRenderer<'_>,
        screen: &[u8],
        out: &mut [u8],
        with_border: bool,
        rows: std::ops::Range<usize>,
    ) {
        super::next_video::render_lores(renderer, screen, out, with_border, rows.clone());
        super::next_video::render_tilemap(renderer, out, with_border, rows.clone());
        super::next_video::compose(renderer, out, with_border, rows);
    }

    fn render_rgba_with_copper_history(
        &self,
        out: &mut [u8],
        with_border: bool,
        initial: &bus::NextBusVideoState,
    ) {
        let (width, height) = Self::framebuffer_dims(with_border);
        let row_bytes = width * 4;
        let paper_line = (ula::PAPER_START_48 / ula::T_LINE_48) as i64;
        let output_origin = paper_line - if with_border { ula::BORDER_Y as i64 } else { 0 };
        let base_renderer = self.bus.video_renderer();
        let screen = Self::load_ula_screen(&base_renderer, true);
        let mut ula = self.ula.clone();
        ula.border = base_renderer.border();
        ula.render_rgba(&screen[..6912], out, with_border);
        let mut current_state = initial.clone();
        let mut first_row = 0usize;

        for (raster_line, state) in &self.copper_video_events {
            let row = (i64::from(*raster_line) - output_origin).clamp(0, height as i64) as usize;
            if row > first_row {
                let renderer = self.bus.video_renderer_with_state(&current_state);
                Self::render_video_layers(&renderer, &screen, out, with_border, first_row..row);
                first_row = row;
            }
            current_state.clone_from(state);
        }
        if first_row < height {
            let renderer = self.bus.video_renderer();
            Self::render_video_layers(&renderer, &screen, out, with_border, first_row..height);
        }
        debug_assert_eq!(row_bytes * height, out.len());
    }

    fn capture_copper_video_before_write(&mut self, time: u64, register: u8) {
        if !NextBus::nextreg_affects_video(register) {
            return;
        }
        let frame_tstates = self.bus.frame_interrupt_timing().0.max(1);
        let frame = time / frame_tstates;
        if self.copper_video_frame != Some(frame) {
            self.copper_video_frame = Some(frame);
            self.copper_video_initial = Some(self.bus.video_state_snapshot());
            self.copper_video_events.clear();
        }
    }

    fn capture_copper_video_after_write(&mut self, time: u64, register: u8) {
        if !NextBus::nextreg_affects_video(register) {
            return;
        }
        let frame_tstates = self.bus.frame_interrupt_timing().0.max(1);
        let line_tstates = match self.bus.display_timing() {
            2 | 3 => u64::from(ula::T_LINE_128),
            4 => u64::from(ula::T_LINE_PENTAGON),
            _ => u64::from(ula::T_LINE_48),
        };
        let line = ((time % frame_tstates) / line_tstates) as u32;
        let snapshot = self.bus.video_state_snapshot();
        if self
            .copper_video_events
            .last()
            .is_some_and(|(last_line, _)| *last_line == line)
        {
            if let Some((_, state)) = self.copper_video_events.last_mut() {
                state.clone_from(&snapshot);
            }
        } else {
            self.copper_video_events.push((line, snapshot));
        }
    }

    fn clear_copper_video_history(&mut self) {
        self.copper_video_frame = None;
        self.render_frame = None;
        self.copper_video_initial = None;
        self.copper_video_events.clear();
    }

    #[must_use]
    pub fn framebuffer_dims(with_border: bool) -> (usize, usize) {
        ula::framebuffer_dims(with_border, false)
    }

    pub fn apply_joystick_state(&mut self, mode: JoystickMode, state: JoystickState) {
        self.bus.keyboard.reset();
        self.bus.kempston.reset();
        apply_joystick(mode, state, &mut self.bus.kempston, &mut self.bus.keyboard);
    }
}

/// Trait adapter keeps the lower-level bus crate independent of the CPU crate.
struct NextMemIo<'a> {
    bus: &'a mut NextBus,
    step_start_master: u64,
    cpu_step_start: u64,
    cpu_t_master_ticks: u64,
    beeper_level: &'a mut bool,
    beeper_edges: &'a mut Vec<(u64, bool)>,
}

impl NextMemIo<'_> {
    fn master_t(&self, cpu_t: u64) -> u64 {
        self.step_start_master.wrapping_add(
            cpu_t
                .wrapping_sub(self.cpu_step_start)
                .saturating_mul(self.cpu_t_master_ticks),
        )
    }
}

impl Memory for NextMemIo<'_> {
    fn read(&mut self, addr: u16, _t: u64) -> (u8, u32) {
        (self.bus.read(addr), 0)
    }

    fn read_opcode(&mut self, addr: u16, _t: u64) -> (u8, u32) {
        (self.bus.read_opcode(addr), 0)
    }

    fn write(&mut self, addr: u16, value: u8, _t: u64) -> u32 {
        self.bus.write(addr, value);
        0
    }
}

impl Io for NextMemIo<'_> {
    fn in_port(&mut self, port: u16, t: u64) -> (u8, u32) {
        let master_t = self.master_t(t);
        self.bus.advance_ctc_master(master_t);
        (self.bus.in_port_at(port, master_t / 8), 0)
    }

    fn out_port(&mut self, port: u16, value: u8, t: u64) -> u32 {
        let master_t = self.master_t(t);
        let absolute_t = master_t / 8;
        self.bus.advance_ctc_master(master_t);
        if port & 1 == 0 {
            let level = value & 0x10 != 0;
            if level != *self.beeper_level {
                *self.beeper_level = level;
                self.beeper_edges.push((absolute_t, level));
            }
        }
        let base_t_stall = self.bus.out_port_at(port, value, absolute_t);
        base_t_stall.saturating_mul(8 / self.cpu_t_master_ticks as u32)
    }

    fn nextreg_write(&mut self, register: u8, value: u8, t: u64) {
        let master_t = self.master_t(t);
        self.bus.advance_ctc_master(master_t);
        self.bus.write_nextreg_at(register, value, master_t / 8);
    }

    fn interrupt_acknowledge(&mut self, im2: bool) -> Option<u8> {
        self.bus.ctc_interrupt_acknowledge(im2)
    }

    fn reti(&mut self) {
        self.bus.ctc_reti();
    }
}

#[cfg(test)]
mod tests {
    use bus::NEXT_ROM_SIZE;

    use super::*;

    fn test_rom() -> Vec<u8> {
        vec![0; NEXT_ROM_SIZE]
    }

    fn rgba_pixel(frame: &[u8], width: usize, x: usize, y: usize) -> [u8; 4] {
        let offset = (y * width + x) * 4;
        frame[offset..offset + 4]
            .try_into()
            .expect("one RGBA pixel")
    }

    #[test]
    fn frame_interrupt_uses_selected_display_timing() {
        // The core currently treats internal/reserved timing IDs as 48K timing.
        for (timing, frame_tstates, interrupt_tstates) in [
            (0, 224 * 312, 32),
            (1, 224 * 312, 32),
            (2, 228 * 311, 36),
            (3, 228 * 311, 36),
            (4, 224 * 320, 32),
            (5, 224 * 312, 32),
            (6, 224 * 312, 32),
            (7, 224 * 312, 32),
        ] {
            let mut machine = NextMachine::new(&test_rom()).expect("valid test ROM");
            machine.bus.write_nextreg(0x03, 0x80 | timing << 4 | 3);
            machine.cpu.regs.pc = 0x4000;
            machine.cpu.regs.sp = 0xc000;
            machine.cpu.regs.iff1 = true;

            machine.master_t = (frame_tstates + interrupt_tstates) * 8;
            assert_eq!(machine.step_once(), 4, "timing {timing}: INT window ended");
            assert_eq!(machine.cpu.regs.pc, 0x4001, "timing {timing}");
            assert!(machine.cpu.regs.iff1, "timing {timing}");

            machine.master_t = frame_tstates * 2 * 8;
            assert_eq!(machine.step_once(), 13, "timing {timing}: frame INT");
            assert_eq!(machine.cpu.regs.pc, 0x0038, "timing {timing}");
            assert!(!machine.cpu.regs.iff1, "timing {timing}");
        }
    }

    #[test]
    fn cpu_turbo_runs_more_instructions_per_frame_without_speeding_up_ctc() {
        let mut elapsed_cpu_tstates = [0; 4];
        for speed in 0..4u8 {
            let mut rom = test_rom();
            rom[..7].copy_from_slice(&[
                0xed, 0x91, 0x07, speed, // NEXTREG $07,speed
                0xc3, 0x04, 0x00, // JP $0004
            ]);
            let mut machine = NextMachine::new(&rom).expect("valid test ROM");
            assert_eq!(machine.step_once(), 20);
            assert_eq!(machine.master_t, 160, "speed changes after its instruction");
            assert_eq!(machine.bus.read_nextreg(0x07), speed);
            assert_eq!(machine.step_once(), 10);
            assert_eq!(machine.bus.read_nextreg(0x07), speed | speed << 4);
            assert_eq!(machine.bus.cpu_t_master_ticks(), 8u64 >> speed);

            // Start timer channel 0 at the same absolute machine instant in
            // every case. It decrements once per 16 master clocks.
            machine.bus.out_port_at(0x183b, 0x05, machine.video_t());
            machine.bus.out_port_at(0x183b, 240, machine.video_t());
            let start_cpu_t = machine.cpu.t;
            let target_master_t = machine.master_t + 9_600;
            while machine.master_t < target_master_t {
                assert_eq!(machine.step_once(), 10);
            }
            assert_eq!(machine.master_t, target_master_t);
            assert_eq!(machine.video_t(), target_master_t / 8);
            assert_eq!(machine.bus.in_port_at(0x183b, machine.video_t()), 120);
            elapsed_cpu_tstates[usize::from(speed)] = machine.cpu.t - start_cpu_t;

            // The display frame remains on the 3.5 MHz equivalent clock.
            let frame_master_t = u64::from(machine.frame_tstates()) * 8;
            machine.run_frame();
            assert!(machine.master_t >= frame_master_t);
            assert!(machine.master_t < frame_master_t + 80);
            assert_eq!(machine.video_t() / u64::from(machine.frame_tstates()), 1);
        }
        assert_eq!(elapsed_cpu_tstates, [1_200, 2_400, 4_800, 9_600]);
    }

    #[test]
    fn ctc_hardware_im2_interrupt_runs_guest_handler_and_reti_releases_service() {
        let mut machine = NextMachine::new(&test_rom()).expect("valid test ROM");
        // Leave the CPU in a loop, with a small handler that writes a marker
        // and returns using RETI.
        machine.bus.write(0x4000, 0x18);
        machine.bus.write(0x4001, 0xfe);
        for (address, byte) in [
            (0x4567, 0x3e), // LD A, $42
            (0x4568, 0x42),
            (0x4569, 0x32), // LD ($8000), A
            (0x456a, 0x00),
            (0x456b, 0x80),
            (0x456c, 0xfb), // EI
            (0x456d, 0xed), // RETI
            (0x456e, 0x4d),
            (0xd0a6, 0x67), // CTC0 slot 3: vector $A6 -> $4567
            (0xd0a7, 0x45),
        ] {
            machine.bus.write(address, byte);
        }
        machine.bus.write_nextreg(0xc0, 0xa1);
        machine.bus.out_port(0x183b, 0x85);
        machine.bus.out_port(0x183b, 1);
        machine.bus.write_nextreg(0xc5, 1);
        machine.bus.advance_ctc(2);

        machine.cpu.regs.pc = 0x4000;
        machine.cpu.regs.sp = 0xbffe;
        machine.cpu.regs.i = 0xd0;
        machine.cpu.regs.im = 2;
        machine.cpu.regs.iff1 = true;
        machine.cpu.regs.iff2 = true;
        machine.master_t = 100 * 8;

        assert!(machine.step_once() > 0);
        assert_eq!(machine.cpu.regs.pc, 0x4567);
        machine.step_once();
        machine.step_once();
        assert_eq!(machine.bus.read(0x8000), 0x42);
        machine.step_once();
        machine.step_once();
        assert_eq!(machine.cpu.regs.pc, 0x4000);
        assert!(machine.bus.ctc_interrupt_pending(true));
        machine.step_once();
        assert_eq!(machine.cpu.regs.pc, 0x4567);
    }

    #[test]
    fn ctc_pulse_mode_repeats_after_im1_ei_ret_handler() {
        let mut rom = test_rom();
        rom[0x38..0x41].copy_from_slice(&[
            0x3a, 0x00, 0x80, // LD A,($8000)
            0x3c, // INC A
            0x32, 0x00, 0x80, // LD ($8000),A
            0xfb, // EI
            0xc9, // RET
        ]);
        let mut machine = NextMachine::new(&rom).expect("valid test ROM");
        machine.bus.write(0x4000, 0x18); // JR $4000
        machine.bus.write(0x4001, 0xfe);
        machine.bus.write(0x8000, 0);
        machine.cpu.regs.pc = 0x4000;
        machine.cpu.regs.sp = 0xbffe;
        machine.cpu.regs.im = 1;
        machine.cpu.regs.iff1 = true;
        machine.cpu.regs.iff2 = true;
        // Pulse mode is the reset default. Configure and prime the timer,
        // then align the machine clock with the setup before stepping CPU.
        machine.bus.out_port_at(0x183b, 0x85, 0);
        machine.bus.out_port_at(0x183b, 1, 0);
        machine.bus.write_nextreg(0xc5, 1);
        machine.bus.advance_ctc_master(16);
        machine.master_t = 16;
        machine.cpu.regs.pc = 0x4000;

        // The CTC IRQ enters the normal IM1 vector, and the ROM-style EI/RET
        // handler does not execute RETI. A later overflow must still interrupt.
        assert!(machine.step_once() > 0);
        assert_eq!(machine.cpu.regs.pc, 0x0038);
        for _ in 0..5 {
            machine.step_once();
        }
        assert_eq!(machine.bus.read(0x8000), 1);
        assert_eq!(machine.cpu.regs.pc, 0x4000);

        machine.master_t += 16;
        machine.bus.advance_ctc_master(machine.master_t);
        assert!(machine.bus.ctc_interrupt_pending(false));
        assert!(machine.step_once() > 0);
        assert_eq!(machine.cpu.regs.pc, 0x0038);
        for _ in 0..5 {
            machine.step_once();
        }
        assert_eq!(machine.bus.read(0x8000), 2);
        assert_eq!(machine.cpu.regs.pc, 0x4000);
    }

    fn dma_setup_program(prescaler: Option<u8>, destination: u16, io_destination: bool) -> Vec<u8> {
        let mut program = vec![0x01, 0x6b, 0x00]; // LD BC,$006B
        let mut writes = vec![
            0x7d, 0x00, 0x80, 0x03, 0x00, // WR0: A=$8000, length=3, A->B
            0x54, 0x02, // WR1: memory, increment, 2T
        ];
        let timing = if prescaler.is_some() { 0x22 } else { 0x02 };
        let port_b = if io_destination { 0x68 } else { 0x50 };
        writes.extend([port_b, timing]); // WR2: endpoint, address mode, variable timing
        if let Some(prescaler) = prescaler {
            writes.push(prescaler);
        }
        writes.extend([
            if prescaler.is_some() { 0xcd } else { 0xad },
            destination as u8,
            (destination >> 8) as u8, // WR4: selected mode and Port B address
            0x82,                     // WR5: stop at end of block
            0xcf,                     // LOAD
            0x87,                     // ENABLE
        ]);
        for value in writes {
            program.extend([0x3e, value, 0xed, 0x79]); // LD A,n; OUT (C),A
        }
        program.push(0x76); // HALT
        program
    }

    #[test]
    fn cpu_programmed_zxn_dma_copies_through_the_mmu() {
        let mut machine = NextMachine::new(&test_rom()).expect("valid test ROM");
        assert!(machine.bus.load_ram_page(4, 0, &[0xa1, 0xb2, 0xc3]));
        assert!(machine
            .bus
            .load_ram_page(10, 0, &dma_setup_program(None, 0xa000, false)));
        machine.cpu.regs.pc = 0x4000;
        machine.cpu.regs.sp = 0xfffe;

        for _ in 0..128 {
            machine.step_once();
            if machine.cpu.regs.halted {
                break;
            }
        }

        assert!(
            machine.cpu.regs.halted,
            "guest DMA program should reach HALT"
        );
        assert_eq!(machine.bus.read(0xa000), 0xa1);
        assert_eq!(machine.bus.read(0xa001), 0xb2);
        assert_eq!(machine.bus.read(0xa002), 0xc3);
    }

    #[test]
    fn zxn_dma_burst_prescaler_runs_cpu_while_waiting_between_bytes() {
        let mut machine = NextMachine::new(&test_rom()).expect("valid test ROM");
        assert!(machine.bus.load_ram_page(4, 0, &[0x20, 0x40, 0x60]));
        assert!(machine
            .bus
            .load_ram_page(10, 0, &dma_setup_program(Some(40), 0xa000, false)));
        machine.cpu.regs.pc = 0x4000;
        machine.cpu.regs.sp = 0xfffe;
        let program = dma_setup_program(Some(40), 0xa000, false);
        // ENABLE is the final data byte before HALT; run until that OUT has
        // completed, then confirm it did not add a block-length CPU stall.
        let enable_out_pc = 0x4000 + (program.len() - 3) as u16;
        let mut enable_cycles = None;
        for _ in 0..64 {
            let pc = machine.cpu.regs.pc;
            let cycles = machine.step_once();
            if pc == enable_out_pc {
                enable_cycles = Some(cycles);
                break;
            }
        }
        assert_eq!(enable_cycles, Some(12));
        assert!(machine.cpu.regs.pc > enable_out_pc);
        for _ in 0..128 {
            machine.step_once();
        }
        assert_eq!(machine.bus.read(0xa000), 0x20);
        assert_eq!(machine.bus.read(0xa001), 0x40);
        assert_eq!(machine.bus.read(0xa002), 0x60);
    }

    #[test]
    fn zxn_dma_burst_prescaler_sends_paced_samples_to_the_dac() {
        let mut machine = NextMachine::new(&test_rom()).expect("valid test ROM");
        machine.bus.write_nextreg(0x08, 0x08); // Enable the four DAC outputs.
        assert!(machine.bus.load_ram_page(4, 0, &[0x20, 0x40, 0x60]));
        let program = dma_setup_program(Some(40), 0x003f, true);
        assert!(machine.bus.load_ram_page(10, 0, &program));
        machine.cpu.regs.pc = 0x4000;
        machine.cpu.regs.sp = 0xfffe;
        for _ in 0..128 {
            machine.step_once();
            if machine.cpu.regs.halted {
                break;
            }
        }
        assert!(
            machine.cpu.regs.halted,
            "guest DMA program should reach HALT"
        );
        for _ in 0..128 {
            machine.step_once();
        }

        let indices = [0x20_u8, 0x40, 0x60].map(|value| {
            let expected = f32::from(value) / 256.0;
            machine
                .dac_samples
                .iter()
                .position(|sample| sample[0] == expected)
                .unwrap_or_else(|| {
                    panic!(
                        "timed DAC samples should include {expected}; samples={:?}",
                        machine.dac_samples
                    )
                })
        });
        assert!(indices[0] < indices[1] && indices[1] < indices[2]);
        assert!((1..=3).contains(&(indices[1] - indices[0])));
        assert!((1..=3).contains(&(indices[2] - indices[1])));
    }

    #[test]
    fn next_machine_renders_timed_ay_samples_into_frame_audio() {
        let mut machine = NextMachine::new(&test_rom()).expect("valid ROM");
        machine.bus.write_nextreg(0x06, 0x01); // AY mode.
        machine.bus.write_nextreg(0x08, 0x10); // Single AY mode, speaker enabled.
        for (register, value) in [(0, 3), (1, 0), (7, 0x3e), (8, 0x0f)] {
            machine.bus.out_port(0xfffd, register);
            machine.bus.out_port(0xbffd, value);
        }

        let audio = machine.run_frame();
        assert_eq!(audio.frame_tstates, Some(69_888));
        assert!((880..=883).contains(&audio.ay_samples.len()));
        assert_eq!(audio.ay_left.len(), audio.ay_samples.len());
        assert_eq!(audio.ay_right.len(), audio.ay_samples.len());
        assert_eq!(audio.audio_muted_samples.len(), audio.ay_samples.len());
        assert!(
            audio.ay_samples.iter().any(|sample| *sample > 0.0),
            "regs={:?} mixed={:?} first={:?}",
            machine.bus.ay_chip(0).map(|ay| ay.regs),
            machine.bus.audio_sample(),
            audio.ay_samples.get(..8)
        );

        let mut alternate = NextMachine::new(&test_rom()).expect("valid ROM");
        alternate.bus.write_nextreg(0x03, 0xa3); // Select 70,908T timing.
        alternate.bus.write_nextreg(0x06, 0x01);
        alternate.bus.write_nextreg(0x08, 0x10);
        for (register, value) in [(0, 3), (1, 0), (7, 0x3e), (8, 0x0f)] {
            alternate.bus.out_port(0xfffd, register);
            alternate.bus.out_port(0xbffd, value);
        }
        let alternate_audio = alternate.run_frame();
        assert_eq!(alternate_audio.frame_tstates, Some(70_908));
        assert!((892..=895).contains(&alternate_audio.ay_samples.len()));
    }

    #[test]
    fn next_machine_samples_held_dac_values_at_their_tstate_and_gates_output() {
        let mut machine = NextMachine::new(&test_rom()).expect("valid ROM");
        machine.bus.write_nextreg_at(0x08, 0x18, 0);
        machine.bus.out_port_at(0x001f, 0xff, 100);

        let audio = machine.run_frame();
        assert_eq!(audio.dac_samples.len(), audio.ay_samples.len());
        assert_eq!(audio.dac_samples[0], [0.5; 4]);
        assert_eq!(audio.dac_samples[1], [255.0 / 256.0, 0.5, 0.5, 0.5]);
        assert!(audio.dac_samples[2..]
            .iter()
            .all(|sample| *sample == [255.0 / 256.0, 0.5, 0.5, 0.5]));

        let mut disabled = NextMachine::new(&test_rom()).expect("valid ROM");
        disabled.bus.out_port_at(0x001f, 0xff, 0);
        let muted = disabled.run_frame();
        assert!(muted.dac_samples.iter().all(|sample| *sample == [0.5; 4]));
    }

    #[test]
    fn reset_map_and_slot_boundaries_match_next_defaults() {
        let mut rom = test_rom();
        rom[0] = 0x40;
        rom[0x1fff] = 0x41;
        rom[0x2000] = 0x42;
        rom[0x3fff] = 0x43;
        let mut machine = NextMachine::new(&rom).expect("test ROM has the required size");
        assert_eq!(machine.cpu.profile(), CpuProfile::Z80N);

        let pages = [0xff, 0xff, 10, 11, 4, 5, 0, 1];
        for (slot, page) in pages.into_iter().enumerate() {
            assert_eq!(machine.bus.read_nextreg(0x50 + slot as u8), page);
            if slot >= 2 {
                let offset = slot as u8;
                assert!(machine.bus.load_ram_page(page, 0, &[offset]));
                assert!(machine.bus.load_ram_page(page, 0x1fff, &[offset | 0x80]));
                assert_eq!(machine.bus.read((slot * 0x2000) as u16), offset);
                assert_eq!(
                    machine.bus.read((slot * 0x2000 + 0x1fff) as u16),
                    offset | 0x80
                );
            }
        }
        assert_eq!(machine.bus.read(0x0000), 0x40);
        assert_eq!(machine.bus.read(0x1fff), 0x41);
        assert_eq!(machine.bus.read(0x2000), 0x42);
        assert_eq!(machine.bus.read(0x3fff), 0x43);
        machine.bus.write(0, 0xaa);
        assert_eq!(machine.bus.read(0), 0x40);

        machine.bus.write_nextreg(0x50, 223);
        machine.bus.write(0, 0x99);
        assert_eq!(machine.bus.read_ram_page(223, 0), Some(0x99));
        machine.reset();
        assert_eq!(machine.bus.read_nextreg(0x50), 0xff);
        assert_eq!(machine.bus.read(0), 0x40);
        assert_eq!(machine.bus.read_ram_page(223, 0), Some(0x99));
    }

    #[test]
    fn guest_fetches_and_reads_writes_after_nextreg_remaps_pages() {
        let mut rom = test_rom();
        // Remap the current instruction slot from ROM to physical RAM page 2.
        rom[..4].copy_from_slice(&[0xed, 0x91, 0x50, 0x02]);
        let mut machine = NextMachine::new(&rom).expect("test ROM has the required size");
        // The instruction at PC=4 is fetched from page 2 after the first step.
        assert!(machine.bus.load_ram_page(
            2,
            4,
            &[
                0x3e, 0xa5, // LD A,A5
                0xed, 0x91, 0x56, 0x03, // NEXTREG 56,03 (slot 6 -> page 3)
                0x32, 0x00, 0xc1, // LD (C100),A
                0x3e, 0x00, // LD A,00
                0x3a, 0x00, 0xc1, // LD A,(C100)
                0x76, // HALT
            ]
        ));
        assert_eq!(machine.step_once(), 20);
        assert_eq!(machine.bus.read_nextreg(0x50), 2);
        for _ in 0..6 {
            machine.step_once();
        }
        assert!(machine.cpu.regs.halted);
        assert_eq!(machine.cpu.regs.a, 0xa5);
        assert_eq!(machine.bus.read_nextreg(0x56), 3);
        assert_eq!(machine.bus.read_ram_page(3, 0x100), Some(0xa5));
        assert_eq!(machine.bus.read_ram_page(0, 0x100), Some(0));
    }

    #[test]
    fn nextreg_instruction_and_register_ports_share_mmu_state() {
        let mut rom = test_rom();
        rom[..22].copy_from_slice(&[
            0xed, 0x91, 0x54, 0x22, // NEXTREG 54,22
            0x01, 0x3b, 0x24, // LD BC,243B
            0x3e, 0x54, // LD A,54
            0xed, 0x79, // OUT (C),A: select register 54
            0x01, 0x3b, 0x25, // LD BC,253B
            0x3e, 0x23, // LD A,23
            0xed, 0x79, // OUT (C),A: page 23
            0xed, 0x78, // IN A,(C): read it back
            0x76, // HALT
            0x00, // padding
        ]);
        let mut machine = NextMachine::new(&rom).expect("test ROM has the required size");
        assert_eq!(machine.step_once(), 20);
        assert_eq!(machine.bus.read_nextreg(0x54), 0x22);
        for _ in 0..8 {
            machine.step_once();
        }
        assert!(machine.cpu.regs.halted);
        assert_eq!(machine.bus.selected_nextreg(), 0x54);
        assert_eq!(machine.bus.read_nextreg(0x54), 0x23);
        assert_eq!(machine.cpu.regs.a, 0x23);
        assert_eq!(machine.bus.in_port(0x253b), 0x23);
    }

    #[test]
    fn ipl_can_leave_config_mode_and_fetch_from_main_rom() {
        let mut rom = test_rom();
        rom[5] = 0x76;
        let mut ipl = vec![0; 8192];
        ipl[..4].copy_from_slice(&[0x3e, 0x03, 0xed, 0x92]);
        ipl[4] = 0x03;
        let mut machine = NextMachine::new(&rom).expect("test ROM has the required size");
        machine
            .install_ipl(&ipl)
            .expect("test IPL has the required size");
        machine.step_once(); // LD A,03
        machine.step_once(); // NEXTREG 03,A disables the IPL
        machine.step_once(); // HALT from the main ROM
        assert_eq!(machine.cpu.regs.pc, 5);
        assert!(machine.cpu.regs.halted);
        assert_eq!(machine.bus.read_nextreg(0x03), 3);
    }

    #[test]
    fn divmmc_automap_uses_the_opcode_fetch_bus_hook() {
        let mut rom = test_rom();
        rom[0x66] = 0x00; // The delayed trigger fetches this main-ROM NOP.
        let mut machine = NextMachine::new(&rom).expect("test ROM has the required size");
        machine
            .install_ipl(&vec![0; 8192])
            .expect("test IPL has the required size");
        machine.bus.write_nextreg(0x03, 0);
        machine.bus.write_nextreg(0x04, 4);
        machine.bus.write(0x0067, 0x3e); // LD A,55 from the DivMMC ROM.
        machine.bus.write(0x0068, 0x55);
        machine.bus.write_nextreg(0x03, 3);
        machine.bus.write_nextreg(0x0a, 0x11);
        machine.cpu.regs.pc = 0x0066;

        assert_eq!(machine.step_once(), 4);
        assert_eq!(machine.cpu.regs.pc, 0x0067);
        assert_eq!(machine.step_once(), 7);
        assert_eq!(machine.cpu.regs.pc, 0x0069);
        assert_eq!(machine.cpu.regs.a, 0x55);
    }

    #[test]
    fn reset_register_divmmc_request_delivers_nmi_and_automaps_rom() {
        let mut rom = test_rom();
        rom[0x66] = 0x00;
        let mut machine = NextMachine::new(&rom).expect("test ROM has the required size");
        machine
            .install_ipl(&vec![0; 8192])
            .expect("test IPL has the required size");
        machine.bus.write_nextreg(0x03, 0);
        machine.bus.write_nextreg(0x04, 4);
        machine.bus.write(0x0066, 0x00); // Delayed mapping leaves this main-ROM NOP visible.
        machine.bus.write(0x0067, 0xed); // RETN from DivMMC ROM.
        machine.bus.write(0x0068, 0x45);
        machine.bus.write_nextreg(0x03, 3);
        machine.bus.write_nextreg(0x0a, 0x11); // SD0 select + DivMMC automap.
        assert_eq!(machine.bus.read_nextreg(0x0a), 0x11);
        assert_eq!(machine.bus.read_nextreg(0xbb), 0xcd);
        assert_eq!(machine.bus.read(0x0067), 0);
        machine.cpu.regs.pc = 0x1234;
        machine.cpu.regs.sp = 0xfffd;
        machine.cpu.regs.iff1 = true;
        machine.cpu.regs.iff2 = true;

        machine.bus.write_nextreg(0x02, 0x04);
        assert_eq!(machine.bus.read_nextreg(0x02) & 0x04, 0x04);
        assert_eq!(machine.step_once(), 11, "NextReg request delivers NMI");
        assert_eq!(machine.cpu.regs.pc, 0x0066);
        assert_eq!(
            machine.step_once(),
            4,
            "main-ROM NOP executes at the NMI vector"
        );
        assert_eq!(
            machine.bus.read(0x0067),
            0xed,
            "delayed automapping takes effect after the trigger fetch"
        );
        assert_eq!(
            machine.step_once(),
            14,
            "automapped DivMMC ROM executes RETN"
        );
        assert_eq!(machine.cpu.regs.pc, 0x1234);
        assert!(machine.cpu.regs.iff1);
    }

    #[test]
    fn hard_reset_restarts_the_ipl() {
        let mut ipl = vec![0; 8192];
        ipl[..5].copy_from_slice(&[0x3e, 0x02, 0xed, 0x92, 0x02]);
        let mut machine = NextMachine::new(&test_rom()).expect("valid test ROM");
        machine.install_ipl(&ipl).expect("valid test IPL");
        assert_eq!(machine.step_once(), 7);
        assert_eq!(machine.step_once(), 17);
        assert_eq!(machine.cpu.regs.pc, 0);
        assert_eq!(machine.bus.read(0), 0x3e);
        assert_eq!(machine.bus.read_nextreg(0x02) & 0x03, 2);
    }

    #[test]
    fn hard_reset_preserves_the_frame_clock() {
        let mut machine = NextMachine::new(&test_rom()).expect("valid test ROM");
        let frame_tstates = machine.frame_tstates() as u64;
        machine.master_t = (frame_tstates - 4) * 8;
        machine.bus.write_nextreg(0x02, 0x02);

        assert_eq!(machine.step_once(), 4);
        assert_eq!(machine.video_t(), frame_tstates);
        assert_eq!(machine.cpu.regs.pc, 0);
    }

    #[test]
    fn frame_audio_contains_next_beeper_edges() {
        let mut machine = NextMachine::new(&test_rom()).expect("valid test ROM");
        let program = [0x3e, 0x10, 0xd3, 0xfe, 0xaf, 0xd3, 0xfe, 0xc3, 0x00, 0xc0];
        for (offset, byte) in program.into_iter().enumerate() {
            machine.bus.write(0xc000 + offset as u16, byte);
        }
        machine.cpu.regs.pc = 0xc000;

        let audio = machine.run_frame();

        assert!(audio.beeper_edges.iter().any(|(_, level)| *level));
        assert!(audio.beeper_edges.iter().any(|(_, level)| !*level));
    }

    #[test]
    fn layer2_composes_only_for_next_and_honors_priority_and_layer_order() {
        let mut machine = NextMachine::new(&test_rom()).expect("valid test ROM");
        machine.bus.load_ram_page(16, 0, &[1, 2]);
        machine.bus.write_nextreg(0x43, 0x10); // Layer 2 palette 1.
        machine.bus.write_nextreg(0x40, 1);
        machine.bus.write_nextreg(0x44, 0b1110_0000); // Red, no priority.
        machine.bus.write_nextreg(0x44, 0x00);
        machine.bus.write_nextreg(0x40, 2);
        machine.bus.write_nextreg(0x44, 0b0000_0011); // Blue, priority.
        machine.bus.write_nextreg(0x44, 0x81);

        let (width, height) = NextMachine::framebuffer_dims(false);
        let mut framebuffer = vec![0; width * height * 4];
        machine.render_rgba(&mut framebuffer, false);
        let ula_before = framebuffer[..4].to_vec();

        machine.bus.write_nextreg(0x69, 0x80); // Layer 2 visible.
        machine.bus.write_nextreg(0x15, 0x08); // ULA over Layer 2.
        machine.render_rgba(&mut framebuffer, false);
        assert_eq!(
            &framebuffer[..4],
            &ula_before,
            "lower Layer 2 stays behind ULA"
        );
        assert_eq!(
            &framebuffer[4..8],
            &[0, 0, 255, 255],
            "priority color is above ULA"
        );

        machine.bus.write_nextreg(0x15, 0x00); // Layer 2 over ULA.
        machine.render_rgba(&mut framebuffer, false);
        assert_eq!(&framebuffer[..4], &[255, 0, 0, 255]);
        assert_eq!(&framebuffer[4..8], &[0, 0, 255, 255]);

        machine.bus.write_nextreg(0x14, 0b1110_0000); // Red is globally transparent.
        machine.render_rgba(&mut framebuffer, false);
        assert_eq!(
            &framebuffer[..4],
            &ula_before,
            "transparent Layer 2 reveals ULA"
        );

        machine.bus.write_nextreg(0x15, 0x18); // Blending order is outside this slice.
        machine.bus.write_nextreg(0x14, 0xe3);
        machine.render_rgba(&mut framebuffer, false);
        assert_eq!(
            &framebuffer[..4],
            &ula_before,
            "unsupported blending mode preserves the ULA framebuffer"
        );

        machine.bus.write_nextreg(0x15, 0x00);
        machine.bus.write_nextreg(0x70, 0x10); // Deferred 320x256 mode.
        machine.render_rgba(&mut framebuffer, false);
        assert_eq!(
            &framebuffer[..4],
            &ula_before,
            "unimplemented resolutions preserve the ULA framebuffer"
        );

        machine.bus.write_nextreg(0x69, 0x00);
        machine.bus.write_nextreg(0x70, 0x00);
        machine.bus.write_nextreg(0x15, 0x00);
        machine.render_rgba(&mut framebuffer, false);
        assert_eq!(
            &framebuffer[..4],
            &ula_before,
            "disabled Layer 2 preserves ULA output"
        );
    }

    #[test]
    fn copper_wait_move_updates_layer2_in_the_current_frame() {
        let mut machine = NextMachine::new(&test_rom()).expect("valid test ROM");
        let mut layer2 = vec![0; 17 * 256 + 1];
        layer2[0] = 1;
        layer2[17 * 256] = 1;
        machine.bus.load_ram_page(16, 0, &layer2);
        machine.bus.write_nextreg(0x43, 0x10); // Layer 2 palette 1.
        machine.bus.write_nextreg(0x40, 1);
        machine.bus.write_nextreg(0x44, 0xe0); // Red.
        machine.bus.write_nextreg(0x44, 0x80); // Priority over ULA.

        // Fixture contains WAIT line 80 followed by MOVE `$69, $80`.
        machine.bus.write_nextreg(0x61, 0);
        for byte in include_bytes!("../tests/fixtures/next/copper_wait_move.bin") {
            machine.bus.write_nextreg(0x60, *byte);
        }
        machine.bus.write_nextreg(0x62, 0x40);

        let target = 80 * ula::T_LINE_48 + 8;
        while machine.video_t() < u64::from(target) {
            machine.step_once();
        }
        assert_eq!(machine.bus.read_nextreg(0x69) & 0x80, 0x80);
        assert_eq!(
            machine.bus.layer2_video_pixel(0, 17).map(|color| color.rgb),
            Some([255, 0, 0])
        );

        let (width, height) = NextMachine::framebuffer_dims(false);
        let mut framebuffer = vec![0; width * height * 4];
        machine.render_rgba(&mut framebuffer, false);
        assert_ne!(rgba_pixel(&framebuffer, width, 0, 0), [255, 0, 0, 255]);
        assert_ne!(rgba_pixel(&framebuffer, width, 0, 16), [255, 0, 0, 255]);
        assert_eq!(rgba_pixel(&framebuffer, width, 0, 17), [255, 0, 0, 255]);
    }

    #[test]
    fn copper_move_at_frame_boundary_is_rendered_in_the_next_frame() {
        let mut machine = NextMachine::new(&test_rom()).expect("valid test ROM");
        let mut layer2 = vec![0; 17 * 256 + 1];
        layer2[0] = 1;
        layer2[17 * 256] = 1;
        machine.bus.load_ram_page(16, 0, &layer2);
        machine.bus.write_nextreg(0x43, 0x10); // Layer 2 palette 1.
        machine.bus.write_nextreg(0x40, 1);
        machine.bus.write_nextreg(0x44, 0xe0); // Red.
        machine.bus.write_nextreg(0x44, 0x80); // Priority over ULA.

        // WAIT for the end of line 311, then enable Layer 2 at the next frame.
        machine.bus.write_nextreg(0x61, 0);
        for byte in [0xf1, 0x37, 0x69, 0x80] {
            machine.bus.write_nextreg(0x60, byte);
        }
        machine.bus.write_nextreg(0x62, 0x40);

        let (width, height) = NextMachine::framebuffer_dims(false);
        let mut framebuffer = vec![0; width * height * 4];
        machine.run_frame();
        machine.render_rgba(&mut framebuffer, false);
        assert_ne!(rgba_pixel(&framebuffer, width, 0, 17), [255, 0, 0, 255]);

        machine.run_frame();
        machine.render_rgba(&mut framebuffer, false);
        assert_eq!(rgba_pixel(&framebuffer, width, 0, 17), [255, 0, 0, 255]);
    }

    #[test]
    fn standard_lores_replaces_only_paper_with_palette_pixels() {
        let mut machine = NextMachine::new(&test_rom()).expect("valid test ROM");
        // LoRes source is the active ULA bank: two contiguous 48-line images
        // separated by the classic ULA attribute area.
        assert!(machine.bus.load_ram_page(10, 0, &[1]));
        assert!(machine.bus.load_ram_page(11, 0, &[3]));
        assert!(machine.bus.load_ram_page(11, 0x17ff, &[1]));
        assert!(machine.bus.load_ram_page(14, 0, &[2]));
        assert!(machine.bus.load_ram_page(19, 0, &[1]));

        machine.bus.write_nextreg(0x43, 0x00); // ULA palette 1.
        for (index, rgb332) in [(1, 0xe0), (2, 0x1c), (3, 0xe3)] {
            machine.bus.write_nextreg(0x40, index);
            machine.bus.write_nextreg(0x41, rgb332);
        }
        machine.bus.write_nextreg(0x43, 0x40); // Write ULA palette 2.
        machine.bus.write_nextreg(0x40, 1);
        machine.bus.write_nextreg(0x41, 0x1c);
        machine.bus.write_nextreg(0x40, 2);
        machine.bus.write_nextreg(0x41, 0x03);
        machine.bus.write_nextreg(0x40, 3);
        machine.bus.write_nextreg(0x41, 0xe3);
        machine.bus.write_nextreg(0x43, 0x02); // Display ULA palette 2.
        machine.bus.write_nextreg(0x14, 0xe3); // Shared global transparency.

        let (width, height) = NextMachine::framebuffer_dims(true);
        let mut frame = vec![0; width * height * 4];
        machine.render_rgba(&mut frame, true);
        assert_eq!(
            rgba_pixel(&frame, width, 48, 48),
            [0, 0, 0, 0xff],
            "disabled LoRes uses ULA"
        );

        machine.bus.write_nextreg(0x15, 0x80); // Enable 256-colour LoRes.
        machine.render_rgba(&mut frame, true);
        assert_eq!(
            rgba_pixel(&frame, width, 48, 48),
            [0, 255, 0, 0xff],
            "active ULA palette"
        );
        assert_eq!(
            rgba_pixel(&frame, width, 49, 49),
            [0, 255, 0, 0xff],
            "2x2 expansion"
        );
        assert_eq!(
            rgba_pixel(&frame, width, 50, 48),
            [0, 0, 0, 0xff],
            "adjacent source pixel"
        );
        assert_eq!(
            rgba_pixel(&frame, width, 0, 48),
            [0, 0, 0, 0xff],
            "left border unchanged"
        );
        assert_eq!(
            rgba_pixel(&frame, width, 304, 48),
            [0, 0, 0, 0xff],
            "right border unchanged"
        );
        assert_eq!(
            rgba_pixel(&frame, width, 48, 144),
            [0, 0, 0, 0xff],
            "transparent pixel is skipped"
        );
        assert_eq!(
            rgba_pixel(&frame, width, 303, 239),
            [0, 255, 0, 0xff],
            "last LoRes pixel reaches the paper edge"
        );

        let (paper_width, paper_height) = NextMachine::framebuffer_dims(false);
        let mut paper = vec![0; paper_width * paper_height * 4];
        machine.render_rgba(&mut paper, false);
        assert_eq!(
            rgba_pixel(&paper, paper_width, 255, 191),
            [0, 255, 0, 0xff],
            "paper-only output expands to 256×192"
        );

        machine.bus.write_nextreg(0x43, 0x12); // Keep ULA palette 2 active; select Layer 2 writes.
        machine.bus.write_nextreg(0x40, 1);
        machine.bus.write_nextreg(0x41, 0xe0);
        machine.bus.write_nextreg(0x44, 0x00);
        machine.bus.write_nextreg(0x69, 0x80);
        machine.bus.write_nextreg(0x15, 0x94); // ULA above Layer 2.
        machine.render_rgba(&mut frame, true);
        assert_eq!(
            rgba_pixel(&frame, width, 48, 144),
            [255, 0, 0, 0xff],
            "transparent LoRes reveals lower Layer 2"
        );

        machine.bus.out_port(0x7ffd, 0x08); // Select shadow ULA bank 7.
        machine.render_rgba(&mut frame, true);
        assert_eq!(
            rgba_pixel(&frame, width, 48, 48),
            [0, 0, 255, 0xff],
            "bank 7 data is selected"
        );

        machine.bus.write_nextreg(0x6a, 0); // Return to standard 256-colour LoRes.
        machine.render_rgba(&mut frame, true);
        assert_eq!(
            rgba_pixel(&frame, width, 48, 48),
            [0, 0, 255, 0xff],
            "standard LoRes remains available"
        );
        machine.bus.write_nextreg(0x6a, 0);
        machine.bus.write_nextreg(0x15, 0);
        machine.render_rgba(&mut frame, true);
        assert_eq!(
            rgba_pixel(&frame, width, 48, 48),
            [0, 0, 0, 0xff],
            "disable restores ULA"
        );
        assert_eq!(frame.len(), width * height * 4);
    }

    #[test]
    fn radastan_uses_packed_palette_and_xor_selected_file_in_both_ula_banks() {
        let mut machine = NextMachine::new(&test_rom()).expect("valid test ROM");
        for (page, offset, value) in [
            (10, 0, 0x12),
            (11, 0, 0x34),
            (10, 0x17ff, 0x56),
            (14, 0, 0x78),
            (15, 0, 0x9a),
        ] {
            assert!(machine.bus.load_ram_page(page, offset, &[value]));
        }

        machine.bus.write_nextreg(0x43, 0);
        for (index, color) in [
            (3, 0x03),
            (4, 0xff),
            (6, 0x03),
            (7, 0xe0),
            (8, 0x1c),
            (9, 0xe0),
            (10, 0x1c),
            (0x21, 0xe0), // Red.
            (0x22, 0x1c), // Green.
            (0x23, 0x03), // Blue.
            (0x24, 0xff), // White.
            (0x25, 0xe3), // Transparent by default.
            (0x26, 0x03),
            (0x27, 0xe0),
            (0x28, 0x1c),
            (0x29, 0x03),
            (0x2a, 0xff),
            (0x31, 0xe0),
            (0x32, 0x1c),
            (0x33, 0x03),
            (0x34, 0xff),
            (0x35, 0xe3),
            (0x36, 0x03),
            (0x37, 0xff),
            (0x38, 0xe0),
            (0x39, 0x1c),
            (0x3a, 0x03),
            (0x3b, 0xff),
        ] {
            machine.bus.write_nextreg(0x40, index);
            machine.bus.write_nextreg(0x41, color);
        }
        machine.bus.write_nextreg(0x43, 0x42); // Write to and display ULA palette 2.
        machine.bus.write_nextreg(0x40, 0x21);
        machine.bus.write_nextreg(0x41, 0x03);
        machine.bus.write_nextreg(0x40, 0x22);
        machine.bus.write_nextreg(0x41, 0xff);
        machine.bus.write_nextreg(0x40, 3);
        machine.bus.write_nextreg(0x41, 0x03);
        machine.bus.write_nextreg(0x40, 4);
        machine.bus.write_nextreg(0x41, 0xff);
        machine.bus.write_nextreg(0x40, 0x23);
        machine.bus.write_nextreg(0x41, 0xe0);
        machine.bus.write_nextreg(0x40, 0x31);
        machine.bus.write_nextreg(0x41, 0x03);
        machine.bus.write_nextreg(0x40, 0x32);
        machine.bus.write_nextreg(0x41, 0xff);
        machine.bus.write_nextreg(0x43, 0x00); // Display ULA palette 1.

        machine.bus.write_nextreg(0x15, 0x80); // Enable LoRes.
        machine.bus.write_nextreg(0x6a, 0x22); // Enable Radastan, first file, palette offset 2.
        let (width, height) = NextMachine::framebuffer_dims(false);
        let mut frame = vec![0; width * height * 4];
        machine.render_rgba(&mut frame, false);
        assert_eq!(rgba_pixel(&frame, width, 0, 0), [255, 0, 0, 0xff]);
        assert_eq!(rgba_pixel(&frame, width, 1, 1), [255, 0, 0, 0xff]);
        assert_eq!(rgba_pixel(&frame, width, 2, 0), [0, 255, 0, 0xff]);
        assert_eq!(rgba_pixel(&frame, width, 3, 1), [0, 255, 0, 0xff]);
        let (border_width, border_height) = NextMachine::framebuffer_dims(true);
        let mut bordered = vec![0; border_width * border_height * 4];
        machine.render_rgba(&mut bordered, true);
        assert_eq!(rgba_pixel(&bordered, border_width, 0, 48), [0, 0, 0, 0xff]);
        assert_eq!(
            rgba_pixel(&bordered, border_width, 48, 48),
            [255, 0, 0, 0xff]
        );
        assert_eq!(
            rgba_pixel(&bordered, border_width, 303, 239),
            [0, 0, 255, 0xff]
        );

        machine.bus.out_port(0x00ff, 1); // $xxFF bit 0 selects the second file.
        machine.render_rgba(&mut frame, false);
        assert_eq!(rgba_pixel(&frame, width, 0, 0), [0, 0, 255, 0xff]);
        assert_eq!(rgba_pixel(&frame, width, 2, 0), [255, 255, 255, 0xff]);

        machine.bus.write_nextreg(0x69, 0); // NextReg $69 aliases the $xxFF selector.
        assert_eq!(machine.bus.read_nextreg(0x69), 0);
        machine.render_rgba(&mut frame, false);
        assert_eq!(rgba_pixel(&frame, width, 0, 0), [255, 0, 0, 0xff]);
        machine.bus.write_nextreg(0x69, 1);
        assert_eq!(machine.bus.read_nextreg(0x69), 1);
        machine.render_rgba(&mut frame, false);
        assert_eq!(rgba_pixel(&frame, width, 0, 0), [0, 0, 255, 0xff]);

        machine.bus.write_nextreg(0x6a, 0x32); // Register file bit XOR port bit selects file 0.
        machine.render_rgba(&mut frame, false);
        assert_eq!(rgba_pixel(&frame, width, 0, 0), [255, 0, 0, 0xff]);

        machine.bus.write_nextreg(0x6a, 0x33); // Offset 3; XOR selects the first file.
        machine.render_rgba(&mut frame, false);
        assert_eq!(rgba_pixel(&frame, width, 0, 0), [255, 0, 0, 0xff]);
        machine.bus.write_nextreg(0x43, 0x02); // Select ULA palette 2.
        machine.render_rgba(&mut frame, false);
        assert_eq!(rgba_pixel(&frame, width, 0, 0), [0, 0, 255, 0xff]);
        machine.bus.write_nextreg(0x43, 0x00); // Select ULA palette 1.
        machine.render_rgba(&mut frame, false);
        assert_eq!(rgba_pixel(&frame, width, 0, 0), [255, 0, 0, 0xff]);

        machine.bus.write_nextreg(0x6a, 0x32);
        machine.bus.write_nextreg(0x14, 0xe0); // Red palette entries are globally transparent.
        machine.render_rgba(&mut frame, false);
        assert_eq!(rgba_pixel(&frame, width, 0, 0), [0, 0, 0, 0xff]);
        assert_eq!(rgba_pixel(&frame, width, 255, 191), [0, 0, 255, 0xff]);

        machine.bus.out_port(0x7ffd, 0x08); // Switch the selected ULA bank to bank 7.
        machine.bus.write_nextreg(0x6a, 0x22);
        machine.bus.write_nextreg(0x14, 0xe3);
        machine.bus.out_port(0x00ff, 0); // Register and port selector bits are both zero.
        machine.render_rgba(&mut frame, false);
        assert_eq!(rgba_pixel(&frame, width, 0, 0), [255, 0, 0, 0xff]);
        assert_eq!(rgba_pixel(&frame, width, 2, 0), [0, 255, 0, 0xff]);

        machine.bus.out_port(0x00ff, 1); // Select bank 7's second display file.
        machine.render_rgba(&mut frame, false);
        assert_eq!(rgba_pixel(&frame, width, 0, 0), [0, 0, 255, 0xff]);
        assert_eq!(rgba_pixel(&frame, width, 2, 0), [255, 255, 255, 0xff]);

        machine.bus.write_nextreg(0x6a, 0); // 256-colour LoRes is still selected by bit 5 clear.
        assert!(machine.bus.lores_256_color_enabled());
        assert!(!machine.bus.radastan_lores_enabled());
        machine.bus.write_nextreg(0x15, 0);
        machine.render_rgba(&mut frame, false);
        assert_ne!(rgba_pixel(&frame, width, 0, 0), [255, 0, 0, 0xff]);
        assert_eq!(frame.len(), width * height * 4);
    }

    #[test]
    fn layer2_with_border_composes_at_the_ula_paper_origin() {
        let mut machine = NextMachine::new(&test_rom()).expect("valid test ROM");
        machine.bus.load_ram_page(16, 0, &[1]);
        machine.bus.write_nextreg(0x43, 0x10);
        machine.bus.write_nextreg(0x40, 1);
        machine.bus.write_nextreg(0x41, 0b1110_0000);
        machine.bus.write_nextreg(0x69, 0x80);
        let (width, height) = NextMachine::framebuffer_dims(true);
        let mut framebuffer = vec![0; width * height * 4];
        machine.render_rgba(&mut framebuffer, true);

        let paper_pixel = (48 * width + 48) * 4;
        assert_eq!(
            &framebuffer[paper_pixel..paper_pixel + 4],
            &[255, 0, 0, 255]
        );
        assert_eq!(
            &framebuffer[..4],
            &[0, 0, 0, 255],
            "border remains ULA-rendered"
        );
    }

    #[test]
    fn soft_reset_enters_the_selected_machine_rom() {
        let mut rom = test_rom();
        rom[..10].copy_from_slice(&[0x3e, 0x03, 0xed, 0x92, 0x03, 0x3e, 0x01, 0xed, 0x92, 0x02]);
        let mut ipl = vec![0; 8192];
        ipl[..5].copy_from_slice(&[0x3e, 0x03, 0xed, 0x92, 0x03]);
        let mut machine = NextMachine::new(&rom).expect("valid test ROM");
        machine.install_ipl(&ipl).expect("valid test IPL");

        assert_eq!(machine.step_once(), 7, "select Next machine ROM");
        assert_eq!(machine.step_once(), 17, "leave IPL mapping");
        assert!(!machine.bus.is_boot_rom_enabled());
        assert_eq!(machine.cpu.regs.pc, 5);
        assert_eq!(machine.step_once(), 7, "select soft reset");
        assert_eq!(machine.step_once(), 17, "request soft reset");

        assert_eq!(machine.cpu.regs.pc, 0);
        assert_eq!(machine.bus.read_nextreg(0x03), 3);
        assert!(!machine.bus.is_boot_rom_enabled());
        assert_eq!(machine.bus.read(0), rom[0]);
        assert_eq!(machine.bus.read_nextreg(0x02) & 0x03, 1);
        assert_eq!(machine.step_once(), 7, "execute from the selected ROM");
        assert_eq!(machine.cpu.regs.pc, 2);
        assert_eq!(machine.cpu.regs.a, 3);
    }
}
