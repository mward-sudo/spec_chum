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
    video_t: u64,
    ula: Ula48,
    debugger_paused: bool,
    beeper_level: bool,
    beeper_edges: Vec<(u64, bool)>,
}

impl NextMachine {
    /// Supply the 64 KiB system ROM image explicitly; firmware acquisition is
    /// handled by the host, not by core construction.
    pub fn new(rom: &[u8]) -> Result<Self, MachineBuildError> {
        Ok(Self {
            cpu: Cpu::with_profile(CpuProfile::Z80N),
            bus: NextBus::new(rom)?,
            video_t: 0,
            ula: Ula48::new(),
            debugger_paused: false,
            beeper_level: false,
            beeper_edges: Vec::new(),
        })
    }

    /// Install the FPGA IPL image that overlays address `$0000` until config
    /// mode is left through `NextReg` `$03`.
    pub fn install_ipl(&mut self, bytes: &[u8]) -> Result<(), MachineBuildError> {
        self.bus.install_boot_rom(bytes)?;
        self.cpu.reset();
        self.video_t = 0;
        self.beeper_level = false;
        self.beeper_edges.clear();
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
    }

    /// Uninterrupted video clock, including across CPU soft resets.
    #[must_use]
    pub fn video_t(&self) -> u64 {
        self.video_t
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
        let (frame_tstates, interrupt_tstates) = self.bus.frame_interrupt_timing();
        let cycles = {
            let mut io = NextMemIo {
                bus: &mut self.bus,
                time_base: self.video_t.saturating_sub(self.cpu.t),
                beeper_level: &mut self.beeper_level,
                beeper_edges: &mut self.beeper_edges,
            };
            if io.bus.take_divmmc_nmi() {
                self.cpu.nmi(&mut io)
            } else if self.video_t % frame_tstates < interrupt_tstates {
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
        self.video_t = self.video_t.wrapping_add(u64::from(cycles));
        if let Some(status) = self.bus.take_reset_request() {
            self.cpu.reset();
            if status == 0x01 {
                self.bus.soft_reset();
            } else {
                self.bus.hard_reset();
                if self.beeper_level {
                    self.beeper_level = false;
                    self.beeper_edges.push((self.video_t, false));
                }
            }
            self.bus.set_reset_status(status);
        }
        cycles
    }

    /// Run to the next selected video-frame boundary.
    pub fn run_frame(&mut self) -> FrameAudio {
        if self.debugger_paused {
            return FrameAudio::default();
        }
        let frame_len = self.bus.frame_interrupt_timing().0;
        let frame_start = self.video_t;
        let boundary = (frame_start / frame_len + 1) * frame_len;
        while self.video_t < boundary {
            self.step_once();
        }
        let mut frame_audio = FrameAudio::default();
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
        let first_page = self.bus.display_screen_bank() * 2;
        let mut screen = [0u8; 0x3800];
        let screen_len = if self.bus.lores_256_color_enabled() {
            screen.len()
        } else {
            6912
        };
        for (offset, byte) in screen[..screen_len].iter_mut().enumerate() {
            *byte = self
                .bus
                .read_ram_page(first_page + (offset / 8192) as u8, offset % 8192)
                .expect("Next ULA bank is within installed physical RAM");
        }
        let mut ula = self.ula.clone();
        ula.border = self.bus.border();
        ula.render_rgba(&screen[..6912], out, with_border);
        super::next_video::render_lores(&self.bus, &screen, out, with_border);
        super::next_video::compose(&self.bus, out, with_border);
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
    time_base: u64,
    beeper_level: &'a mut bool,
    beeper_edges: &'a mut Vec<(u64, bool)>,
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
        (self.bus.in_port_at(port, self.time_base.wrapping_add(t)), 0)
    }

    fn out_port(&mut self, port: u16, value: u8, t: u64) -> u32 {
        let absolute_t = self.time_base.wrapping_add(t);
        if port & 1 == 0 {
            let level = value & 0x10 != 0;
            if level != *self.beeper_level {
                *self.beeper_level = level;
                self.beeper_edges.push((absolute_t, level));
            }
        }
        self.bus.out_port_at(port, value, absolute_t);
        0
    }

    fn nextreg_write(&mut self, register: u8, value: u8, _t: u64) {
        self.bus.write_nextreg(register, value);
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

            machine.video_t = frame_tstates + interrupt_tstates;
            assert_eq!(machine.step_once(), 4, "timing {timing}: INT window ended");
            assert_eq!(machine.cpu.regs.pc, 0x4001, "timing {timing}");
            assert!(machine.cpu.regs.iff1, "timing {timing}");

            machine.video_t = frame_tstates * 2;
            assert_eq!(machine.step_once(), 13, "timing {timing}: frame INT");
            assert_eq!(machine.cpu.regs.pc, 0x0038, "timing {timing}");
            assert!(!machine.cpu.regs.iff1, "timing {timing}");
        }
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
        machine.video_t = frame_tstates - 4;
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

        machine.bus.write_nextreg(0x6a, 0xe0); // 16-colour mode is out of scope.
        assert_eq!(machine.bus.read_nextreg(0x6a), 0x20);
        machine.render_rgba(&mut frame, true);
        assert_eq!(
            rgba_pixel(&frame, width, 48, 48),
            [0, 0, 0, 0xff],
            "unsupported mode uses ULA"
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
