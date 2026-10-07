//! Four-channel ZX Spectrum Next CTC timer subset.
//!
//! The CTC is clocked from the 28 MHz machine master clock. External CLK/TRG
//! pins and channels 4–7 are intentionally outside this implementation.

const CHANNELS: usize = 4;

#[derive(Clone, Copy, Debug)]
struct Channel {
    control: u8,
    time_constant: u16,
    counter: u16,
    prescaler: u16,
    prescaler_left: u16,
    expect_constant: bool,
    pending_constant: Option<u16>,
    configured: bool,
    hard_reset: bool,
    waiting_for_trigger: bool,
    running: bool,
    pending: bool,
    in_service: bool,
    output: bool,
}

impl Default for Channel {
    fn default() -> Self {
        Self {
            control: 0,
            time_constant: 0,
            counter: 0,
            prescaler: 16,
            prescaler_left: 16,
            expect_constant: false,
            pending_constant: None,
            configured: false,
            hard_reset: true,
            waiting_for_trigger: false,
            running: false,
            pending: false,
            in_service: false,
            output: false,
        }
    }
}

impl Channel {
    fn load_constant(&mut self, value: u8) {
        let constant = if value == 0 { 256 } else { u16::from(value) };
        self.expect_constant = false;
        self.hard_reset = false;
        if self.configured && self.running {
            self.pending_constant = Some(constant);
        } else {
            self.time_constant = constant;
            self.counter = constant;
            self.prescaler_left = self.prescaler;
            self.configured = true;
            self.running = !self.waiting_for_trigger;
        }
    }

    fn control(&mut self, value: u8) -> bool {
        if value & 1 == 0 {
            return false;
        }
        if self.hard_reset && value & 0x04 == 0 {
            return false;
        }
        let edge_changed = self.control & 0x10 != value & 0x10;
        self.control = value;
        self.prescaler = if value & 0x20 == 0 { 16 } else { 256 };
        self.waiting_for_trigger = value & 0x08 != 0;
        if !self.hard_reset && self.configured && !self.waiting_for_trigger && !self.running {
            self.running = true;
        }
        if value & 0x02 != 0 {
            self.running = false;
            self.pending = false;
            self.output = false;
            self.counter = self.time_constant;
            self.prescaler_left = self.prescaler;
            self.pending_constant = None;
            self.configured = false;
            self.hard_reset = value & 0x04 == 0;
        }
        if value & 0x04 != 0 {
            self.hard_reset = false;
            self.expect_constant = true;
        }
        edge_changed
    }

    fn trigger(&mut self) -> bool {
        if self.hard_reset || !self.configured {
            return false;
        }
        if self.control & 0x40 != 0 {
            return self.decrement();
        }
        if self.waiting_for_trigger {
            self.waiting_for_trigger = false;
            self.running = true;
            self.prescaler_left = self.prescaler;
        }
        false
    }

    fn decrement(&mut self) -> bool {
        if self.hard_reset || !self.configured || self.counter == 0 {
            return false;
        }
        if self.counter > 1 {
            self.counter -= 1;
            return false;
        }
        self.time_constant = self.pending_constant.take().unwrap_or(self.time_constant);
        self.counter = self.time_constant;
        self.prescaler_left = self.prescaler;
        self.output = true;
        if self.control & 0x80 != 0 {
            self.pending = true;
        }
        true
    }

    fn read_counter(&self) -> u8 {
        self.counter as u8
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct NextCtc {
    channels: [Channel; CHANNELS],
    status: u8,
    last_clock: u64,
}

impl NextCtc {
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn read_port(&self, port: u16) -> Option<u8> {
        let channel = ctc_channel(port)?;
        self.channels.get(channel).map(Channel::read_counter)
    }

    pub(crate) fn write_port(&mut self, port: u16, value: u8) {
        let Some(channel) = ctc_channel(port) else {
            return;
        };
        let Some(state) = self.channels.get_mut(channel) else {
            return;
        };
        if state.expect_constant {
            state.load_constant(value);
        } else if value & 1 != 0 {
            let edge_changed = state.control(value);
            if edge_changed {
                let overflow = state.trigger();
                if overflow {
                    self.status |= 1 << channel;
                }
            }
        }
    }

    pub(crate) fn advance_to(&mut self, clock: u64) {
        if clock <= self.last_clock {
            return;
        }
        let mut elapsed = clock.saturating_sub(self.last_clock);
        self.last_clock = clock;
        while elapsed != 0 {
            let next_event = self
                .channels
                .iter()
                .filter(|channel| {
                    channel.running && channel.control & 0x40 == 0 && channel.configured
                })
                .map(|channel| u64::from(channel.prescaler_left.max(1)))
                .min()
                .unwrap_or(elapsed)
                .min(elapsed);
            for channel in &mut self.channels {
                channel.output = false;
                if channel.running && channel.control & 0x40 == 0 && channel.configured {
                    channel.prescaler_left =
                        channel.prescaler_left.saturating_sub(next_event as u16);
                }
            }
            elapsed -= next_event;
            if self
                .channels
                .iter()
                .any(|channel| channel.running && channel.configured && channel.prescaler_left == 0)
            {
                self.tick();
            }
        }
    }

    fn tick(&mut self) {
        let mut overflow = [false; CHANNELS];
        for (index, channel) in self.channels.iter_mut().enumerate() {
            if channel.running && channel.configured && channel.prescaler_left == 0 {
                channel.prescaler_left = channel.prescaler;
                overflow[index] = channel.decrement();
            }
        }
        let mut triggered = [false; CHANNELS];
        let mut queue = [0; CHANNELS];
        let mut queue_len = 0;
        for index in 0..CHANNELS {
            if overflow[index] {
                triggered[index] = true;
                queue[queue_len] = index;
                queue_len += 1;
            }
        }
        let mut cursor = 0;
        while cursor < queue_len {
            let target = (queue[cursor] + 1) % CHANNELS;
            cursor += 1;
            if !triggered[target] {
                triggered[target] = true;
                if self.channels[target].trigger() {
                    overflow[target] = true;
                    queue[queue_len] = target;
                    queue_len += 1;
                }
            }
        }
        for (index, channel) in self.channels.iter_mut().enumerate() {
            if overflow[index] {
                self.status |= 1 << index;
            }
            channel.output = overflow[index];
        }
    }

    pub(crate) fn write_interrupt_enable(&mut self, value: u8) {
        for (index, channel) in self.channels.iter_mut().enumerate() {
            channel.control =
                (channel.control & !0x80) | (u8::from(value & (1 << index) != 0) << 7);
        }
    }

    pub(crate) fn interrupt_enable(&self) -> u8 {
        self.channels
            .iter()
            .enumerate()
            .fold(0, |bits, (index, channel)| {
                bits | (u8::from(channel.control & 0x80 != 0) << index)
            })
    }

    pub(crate) fn interrupt_status(&self) -> u8 {
        self.status
    }

    pub(crate) fn clear_interrupt_status(&mut self, value: u8) {
        for index in 0..CHANNELS {
            let bit = 1 << index;
            if value & bit != 0 && !self.channels[index].pending {
                self.status &= !bit;
            }
        }
    }

    pub(crate) fn interrupt_pending(&self) -> bool {
        self.highest_request().is_some()
    }

    pub(crate) fn acknowledge(&mut self) -> Option<usize> {
        let channel = self.highest_request()?;
        self.channels[channel].pending = false;
        self.channels[channel].in_service = true;
        Some(channel)
    }

    pub(crate) fn reti(&mut self) {
        if let Some(channel) = self.channels.iter().position(|channel| channel.in_service) {
            self.channels[channel].in_service = false;
        }
    }

    fn highest_request(&self) -> Option<usize> {
        for (index, channel) in self.channels.iter().enumerate() {
            if channel.in_service {
                return None;
            }
            if channel.pending && channel.control & 0x80 != 0 {
                return Some(index);
            }
        }
        None
    }
}

fn ctc_channel(port: u16) -> Option<usize> {
    // Each channel responds to its documented port with the board's partial
    // decode: A15..A8 are ignored, while the low byte must match exactly.
    if port & 0xf8ff != 0x183b {
        return None;
    }
    let channel = ((port >> 8) & 0x07) as usize;
    (channel < CHANNELS).then_some(channel)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTROL_TIMER_AUTO: u8 = 0b1000_0101;
    const CONTROL_TIMER_TRIGGERED: u8 = 0b1000_1101;
    const CONTROL_COUNTER: u8 = 0b1100_0101;

    #[test]
    fn four_ports_decode_independently_and_channels_four_through_seven_are_absent() {
        let mut ctc = NextCtc::default();
        for channel in 0..4 {
            let port = 0x183b + channel * 0x100;
            ctc.write_port(port, CONTROL_TIMER_AUTO);
            ctc.write_port(port, channel as u8 + 2);
            assert_eq!(ctc.read_port(port), Some(channel as u8 + 2));
        }
        for port in [0x1c3b, 0x1d3b, 0x1e3b, 0x1f3b] {
            assert_eq!(ctc.read_port(port), None);
        }
        for port in [
            0x183a,
            0x183c,
            0x103b,
            0x203b,
            0x183b_u16.wrapping_add(0x8000),
        ] {
            assert_eq!(ctc.read_port(port), None, "unrelated port {port:#06x}");
        }
    }

    #[test]
    fn zero_time_constant_and_sixteen_clock_prescaler_are_applied() {
        let mut ctc = NextCtc::default();
        ctc.write_port(0x183b, CONTROL_TIMER_AUTO);
        ctc.write_port(0x183b, 0);
        ctc.advance_to(16 * 255);
        assert_eq!(ctc.read_port(0x183b), Some(1));
        ctc.advance_to(16 * 256);
        assert_eq!(ctc.read_port(0x183b), Some(0));
        assert_eq!(ctc.interrupt_status() & 1, 1);
    }

    #[test]
    fn trigger_edge_starts_timer_and_reset_cancels_it() {
        let mut ctc = NextCtc::default();
        ctc.write_port(0x183b, CONTROL_TIMER_TRIGGERED);
        ctc.write_port(0x183b, 1);
        ctc.advance_to(128);
        assert_eq!(ctc.read_port(0x183b), Some(1));
        ctc.write_port(0x183b, 0b1001_1001); // Change edge selection; this is an edge.
        ctc.advance_to(144);
        assert_eq!(ctc.read_port(0x183b), Some(1));
        ctc.advance_to(160);
        assert_eq!(ctc.interrupt_status() & 1, 1);
        ctc.write_port(0x183b, 0b1000_0011); // Soft reset, no constant follows.
        ctc.advance_to(320);
        assert_eq!(ctc.read_port(0x183b), Some(1));
    }

    #[test]
    fn channels_cascade_and_interrupts_follow_enable_priority_ack_and_reti() {
        let mut ctc = NextCtc::default();
        ctc.write_port(0x183b, CONTROL_TIMER_AUTO);
        ctc.write_port(0x183b, 1);
        ctc.write_port(0x193b, CONTROL_COUNTER);
        ctc.write_port(0x193b, 1);
        ctc.write_port(0x1a3b, CONTROL_COUNTER);
        ctc.write_port(0x1a3b, 1);
        ctc.write_interrupt_enable(0x07);
        ctc.advance_to(16);
        assert_eq!(ctc.read_port(0x193b), Some(1));
        assert_eq!(ctc.read_port(0x1a3b), Some(1));
        assert_eq!(ctc.interrupt_status() & 0x07, 0x07);
        assert_eq!(ctc.acknowledge(), Some(0));
        assert!(!ctc.interrupt_pending());
        ctc.reti();
        assert_eq!(ctc.acknowledge(), Some(1));
    }

    #[test]
    fn nextreg_enable_updates_only_control_d7_and_status_latches_without_ie() {
        let mut ctc = NextCtc::default();
        // Channel 0: timer, rising trigger edge, 256 prescaler, constant follows.
        ctc.write_port(0x183b, 0b0010_0101);
        ctc.write_port(0x183b, 1);
        ctc.write_interrupt_enable(0x01);
        assert_eq!(ctc.interrupt_enable(), 0x01);
        assert_eq!(ctc.channels[0].control, 0b1010_0101);
        assert!(ctc.channels[0].running);

        // Clear the D7 IE while preserving mode bits. Overflow still sets C9,
        // while the interrupt request itself remains masked.
        ctc.write_interrupt_enable(0);
        assert_eq!(ctc.channels[0].control, 0b0010_0101);
        assert!(ctc.channels[0].running);
        ctc.advance_to(256);
        assert_eq!(ctc.interrupt_status() & 1, 1);
        assert!(!ctc.interrupt_pending());
    }

    #[test]
    fn channel_three_cascades_back_to_channel_zero() {
        let mut ctc = NextCtc::default();
        for port in [0x183b, 0x193b, 0x1a3b] {
            ctc.write_port(port, CONTROL_COUNTER);
            ctc.write_port(port, 1);
        }
        // Channel 3's timer output wraps to channel 0, then propagates through
        // the other counter channels in the same master-clock edge.
        ctc.write_port(0x1b3b, CONTROL_TIMER_AUTO);
        ctc.write_port(0x1b3b, 1);
        ctc.advance_to(16);
        assert_eq!(ctc.interrupt_status() & 0x0f, 0x0f);
        assert_eq!(ctc.read_port(0x183b), Some(1));
    }
}
