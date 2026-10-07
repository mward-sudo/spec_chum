//! Spectrum Next Copper instruction memory and beam-scheduled execution.

const INSTRUCTION_COUNT: usize = 1024;
const CLOCKS_PER_CPU_TSTATE: u64 = 8;

#[derive(Clone, Debug)]
pub(super) struct NextCopper {
    memory: [u8; INSTRUCTION_COUNT * 2],
    address: u16,
    control: u8,
    program_counter: u16,
    running: bool,
    clocks: u64,
    stalled: bool,
}

impl NextCopper {
    pub(super) fn new() -> Self {
        Self {
            memory: [0; INSTRUCTION_COUNT * 2],
            address: 0,
            control: 0,
            program_counter: 0,
            running: false,
            clocks: 0,
            stalled: false,
        }
    }

    pub(super) fn reset(&mut self) {
        self.address = 0;
        self.control = 0;
        self.program_counter = 0;
        self.running = false;
        self.clocks = 0;
        self.stalled = false;
    }

    pub(super) fn address_low(&self) -> u8 {
        self.address as u8
    }

    pub(super) fn control(&self) -> u8 {
        (self.control & 0xc0) | ((self.address >> 8) as u8 & 0x07)
    }

    pub(super) fn write_data(&mut self, value: u8) {
        self.memory[usize::from(self.address)] = value;
        self.address = self.address.wrapping_add(1) & 0x07ff;
    }

    pub(super) fn write_address_low(&mut self, value: u8) {
        self.address = (self.address & 0x0700) | u16::from(value);
    }

    pub(super) fn write_control(&mut self, value: u8, time: u64) {
        let previous_mode = self.control & 0xc0;
        self.address = (self.address & 0x00ff) | (u16::from(value & 0x07) << 8);
        self.control = value & 0xc7;
        let mode = self.control & 0xc0;
        if mode == previous_mode {
            return;
        }
        match mode {
            0x00 => {
                self.running = false;
                self.stalled = false;
            }
            0x40 | 0xc0 => {
                self.program_counter = 0;
                self.running = true;
                self.stalled = false;
                self.clocks = time.saturating_mul(CLOCKS_PER_CPU_TSTATE);
            }
            0x80 => {
                self.running = true;
                self.stalled = false;
                self.clocks = time.saturating_mul(CLOCKS_PER_CPU_TSTATE);
            }
            _ => {}
        }
    }

    /// Run due instructions and return `(time, register, value)` MOVE writes.
    /// Beam positions use one horizontal step per four CPU T-states (8 pixels
    /// at the 2-pixel-per-T-state base timing); core-3 subpixel ordering is not
    /// modeled. MOVE/NOOP execution uses the documented two/one Copper clocks.
    pub(super) fn advance(
        &mut self,
        time: u64,
        frame_tstates: u32,
        tstates_per_line: u32,
    ) -> Vec<(u64, u8, u8)> {
        let target_clocks = time.saturating_mul(CLOCKS_PER_CPU_TSTATE);
        let frame_clocks = u64::from(frame_tstates).saturating_mul(CLOCKS_PER_CPU_TSTATE);
        let line_clocks = u64::from(tstates_per_line).saturating_mul(CLOCKS_PER_CPU_TSTATE);
        let mut writes = Vec::new();

        while self.running && self.clocks <= target_clocks {
            if self.control & 0xc0 == 0xc0
                && frame_clocks > 0
                && self.clocks / frame_clocks < target_clocks / frame_clocks
            {
                self.program_counter = 0;
                self.stalled = false;
                self.clocks = (self.clocks / frame_clocks + 1) * frame_clocks;
                continue;
            }
            if self.stalled {
                break;
            }

            let offset = usize::from(self.program_counter) * 2;
            let instruction = u16::from_be_bytes([self.memory[offset], self.memory[offset + 1]]);
            let duration = if instruction == u16::MAX {
                self.stalled = true;
                break;
            } else if instruction == 0 {
                1
            } else if instruction & 0x8000 != 0 {
                let horizontal = u64::from((instruction >> 9) & 0x3f);
                let vertical =
                    (u64::from((instruction >> 8) & 1) << 8) | u64::from(instruction & 0xff);
                if vertical > 311 {
                    self.stalled = true;
                    break;
                }
                let frame_base = self.clocks / frame_clocks.max(1) * frame_clocks;
                let target = frame_base
                    .saturating_add(vertical.saturating_mul(line_clocks))
                    .saturating_add(horizontal.saturating_mul(4 * CLOCKS_PER_CPU_TSTATE));
                let target = if target < self.clocks {
                    target.saturating_add(frame_clocks)
                } else {
                    target
                };
                if target > self.clocks {
                    self.clocks = target;
                }
                1
            } else {
                let register = ((instruction >> 8) & 0x7f) as u8;
                if register != 0 && register < 0x80 {
                    writes.push((
                        self.clocks.div_ceil(CLOCKS_PER_CPU_TSTATE),
                        register,
                        instruction as u8,
                    ));
                }
                2
            };
            self.program_counter = self.program_counter.wrapping_add(1) & 0x03ff;
            self.clocks = self.clocks.saturating_add(duration);
        }
        self.clocks = self.clocks.max(target_clocks);
        writes
    }

    #[cfg(test)]
    fn instruction(&self, index: usize) -> u16 {
        u16::from_be_bytes([self.memory[index * 2], self.memory[index * 2 + 1]])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_address_wraps_after_2048_bytes() {
        let mut copper = NextCopper::new();
        copper.write_address_low(0xfe);
        copper.write_control(0x07, 0);
        copper.write_data(0x12);
        copper.write_data(0x34);
        assert_eq!(copper.instruction(1023), 0x1234);
        assert_eq!(copper.address, 0);
    }

    #[test]
    fn control_selects_stop_start_resume_and_keeps_index() {
        let mut copper = NextCopper::new();
        copper.write_control(0x41, 10);
        assert!(copper.running);
        assert_eq!(copper.program_counter, 0);
        copper.program_counter = 19;
        copper.write_control(0x81, 11);
        assert_eq!(copper.program_counter, 19);
        copper.write_control(0x00, 12);
        assert!(!copper.running);
        assert_eq!(copper.control(), 0);
        assert_eq!(copper.address, 0);
    }

    #[test]
    fn wait_then_move_emits_at_requested_raster_line() {
        let mut copper = NextCopper::new();
        copper.memory[0..4].copy_from_slice(&[0x80, 0x02, 0x15, 0x08]);
        copper.write_control(0x40, 0);
        let writes = copper.advance(3_000, 69_888, 224);
        assert_eq!(writes, [(2 * 224 + 1, 0x15, 0x08)]);
    }

    #[test]
    fn noop_advances_and_halt_stalls_until_restarted() {
        let mut copper = NextCopper::new();
        copper.memory[0..4].copy_from_slice(&[0, 0, 0xff, 0xff]);
        copper.write_control(0x40, 0);
        assert_eq!(copper.advance(10, 69_888, 224), []);
        assert!(copper.stalled);
        copper.write_control(0x00, 10);
        copper.write_control(0x40, 10);
        assert!(!copper.stalled);
    }

    #[test]
    fn core_control_mode_restarts_at_frame_boundary() {
        let mut copper = NextCopper::new();
        copper.memory[0..4].copy_from_slice(&[0x00, 0x00, 0x15, 0x08]);
        copper.write_control(0xc0, 0);
        let writes = copper.advance(69_889, 69_888, 224);
        assert!(writes.iter().any(|(time, register, value)| {
            *time >= 69_888 && *register == 0x15 && *value == 0x08
        }));
    }

    #[test]
    fn reset_stops_and_rewinds_without_clearing_instruction_memory() {
        let mut copper = NextCopper::new();
        copper.memory[0..2].copy_from_slice(&[0x15, 0x08]);
        copper.write_control(0x40, 0);
        copper.advance(1, 69_888, 224);
        copper.reset();
        assert!(!copper.running);
        assert_eq!(copper.program_counter, 0);
        assert_eq!(copper.address, 0);
        assert_eq!(copper.control(), 0);
        assert_eq!(copper.instruction(0), 0x1508);
    }

    #[test]
    fn nextreg_60_stream_uses_the_byte_address_selected_by_61_and_62() {
        let rom = vec![0; crate::NEXT_ROM_SIZE];
        let mut bus = crate::NextBus::new(&rom).expect("fixed-size test ROM");
        bus.write_nextreg(0x61, 0xfe);
        bus.write_nextreg(0x62, 0x07);
        bus.write_nextreg(0x60, 0x12);
        bus.write_nextreg(0x60, 0x34);

        assert_eq!(bus.read_nextreg(0x61), 0);
        assert_eq!(bus.read_nextreg(0x62) & 0x07, 0);
        bus.write_nextreg(0x61, 0xfe);
        bus.write_nextreg(0x62, 0x07);
        bus.write_nextreg(0x62, 0x47); // START from instruction zero.
        assert_eq!(bus.advance_copper(1), []);
        assert_eq!(bus.read_nextreg(0x62) & 0xc0, 0x40);
    }
}
