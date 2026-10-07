//! ZX Spectrum Next's single-channel zxnDMA command and transfer state.
//!
//! The bus owns memory and I/O effects; this module owns the DMA register
//! protocol, pointers, counters, and pacing state.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum AddressMode {
    Decrement,
    #[default]
    Increment,
    Fixed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Endpoint {
    #[default]
    Memory,
    Io,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DmaAction {
    None,
    Reset,
    Load,
    Continue,
    Enable,
    Disable,
}

#[derive(Clone, Debug)]
pub(super) struct NextDma {
    port_a: u16,
    port_b: u16,
    start_a: u16,
    start_b: u16,
    length: u16,
    remaining: u16,
    transferred: bool,
    completed: bool,
    direction_a_to_b: bool,
    endpoint_a: Endpoint,
    endpoint_b: Endpoint,
    mode_a: AddressMode,
    mode_b: AddressMode,
    timing_a: u8,
    timing_b: u8,
    prescaler: u8,
    burst: bool,
    active: bool,
    next_transfer_t: Option<u64>,
    read_mask: u8,
    read_sequence: [u8; 7],
    read_index: usize,
    read_status_pending: bool,
    read_mask_pending: bool,
    pending: std::collections::VecDeque<u8>,
}

impl Default for NextDma {
    fn default() -> Self {
        Self {
            port_a: 0,
            port_b: 0,
            start_a: 0,
            start_b: 0,
            length: 0,
            remaining: 0,
            transferred: false,
            completed: false,
            direction_a_to_b: true,
            endpoint_a: Endpoint::Memory,
            endpoint_b: Endpoint::Memory,
            mode_a: AddressMode::Increment,
            mode_b: AddressMode::Increment,
            timing_a: 2,
            timing_b: 2,
            prescaler: 0,
            burst: false,
            active: false,
            next_transfer_t: None,
            read_mask: 0x7f,
            read_sequence: [0x3a, 0, 0, 0, 0, 0, 0],
            read_index: 0,
            read_status_pending: false,
            read_mask_pending: false,
            pending: std::collections::VecDeque::new(),
        }
    }
}

impl NextDma {
    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }

    /// Consume one byte written to `$xx6B` and return its transfer action.
    pub(super) fn write(&mut self, value: u8) -> DmaAction {
        if let Some(register) = self.pending.pop_front() {
            self.write_parameter(register, value);
            return DmaAction::None;
        }
        if self.read_mask_pending {
            self.read_mask = value;
            self.read_mask_pending = false;
            self.read_index = 0;
            return DmaAction::None;
        }
        match value {
            0xc3 => {
                self.reset();
                DmaAction::Reset
            }
            0xc7 => {
                self.timing_a = 2;
                self.prescaler = 0;
                DmaAction::None
            }
            0xcb => {
                self.timing_b = 2;
                self.prescaler = 0;
                DmaAction::None
            }
            0xbf => {
                self.read_status_pending = true;
                DmaAction::None
            }
            0xa7 => {
                self.read_index = 0;
                DmaAction::None
            }
            0x8b => {
                self.read_mask = 0x7f;
                self.read_index = 0;
                DmaAction::None
            }
            0xbb => {
                self.read_mask_pending = true;
                DmaAction::None
            }
            0xcf => DmaAction::Load,
            0xd3 => DmaAction::Continue,
            0x87 => DmaAction::Enable,
            0x83 => {
                self.active = false;
                self.next_transfer_t = None;
                DmaAction::Disable
            }
            _ if value & 0x83 == 0x01 => {
                self.direction_a_to_b = value & 0x04 != 0;
                let options = (value >> 3) & 0x0f;
                for bit in 0..4 {
                    if options & (1 << bit) != 0 {
                        self.pending.push_back(match bit {
                            0 => 0, // Port A address low
                            1 => 1, // Port A address high
                            2 => 2, // Transfer length low
                            _ => 3, // Transfer length high
                        });
                    }
                }
                DmaAction::None
            }
            _ if value & 0x87 == 0x04 => {
                self.endpoint_a = if value & 0x08 != 0 {
                    Endpoint::Io
                } else {
                    Endpoint::Memory
                };
                self.mode_a = address_mode((value >> 4) & 0x03);
                if value & 0x40 != 0 {
                    self.pending.push_back(4);
                }
                DmaAction::None
            }
            _ if value & 0x87 == 0x00 => {
                self.endpoint_b = if value & 0x08 != 0 {
                    Endpoint::Io
                } else {
                    Endpoint::Memory
                };
                self.mode_b = address_mode((value >> 4) & 0x03);
                if value & 0x40 != 0 {
                    self.pending.push_back(5);
                }
                DmaAction::None
            }
            _ if value & 0x81 == 0x81 => {
                self.burst = (value >> 5) & 0x03 == 0b10;
                let options = (value >> 2) & 0x03;
                for bit in 0..2 {
                    if options & (1 << bit) != 0 {
                        self.pending.push_back(7 + bit); // Port B low, then high
                    }
                }
                DmaAction::None
            }
            _ if value & 0xcf == 0x82 => DmaAction::None,
            _ => DmaAction::None,
        }
    }

    pub(super) fn read(&mut self) -> u8 {
        if std::mem::take(&mut self.read_status_pending) {
            return self.read_sequence[0];
        }
        while self.read_index < 7 && self.read_mask & (1 << self.read_index) == 0 {
            self.read_index += 1;
        }
        if self.read_index == 7 {
            self.read_index = 0;
            while self.read_mask & 1 == 0 && self.read_index < 7 {
                self.read_index += 1;
            }
        }
        let index = self.read_index.min(6);
        self.read_index += 1;
        self.read_sequence[index]
    }

    pub(super) fn loaded(&mut self) {
        self.port_a = self.start_a;
        self.port_b = self.start_b;
        self.remaining = self.length;
        self.transferred = false;
        self.completed = self.length == 0;
        self.active = false;
        self.next_transfer_t = None;
        self.update_read_sequence();
    }

    pub(super) fn continued(&mut self) {
        self.remaining = self.length;
        self.transferred = false;
        self.completed = self.length == 0;
        self.active = false;
        self.next_transfer_t = None;
        self.update_read_sequence();
    }

    pub(super) fn enable(&mut self, t: u64) {
        self.active = self.remaining > 0;
        self.next_transfer_t = self.active.then_some(t);
    }

    pub(super) fn active(&self) -> bool {
        self.active
    }

    pub(super) fn remaining(&self) -> u16 {
        self.remaining
    }

    pub(super) fn scheduled_t(&self) -> Option<u64> {
        self.next_transfer_t
    }

    pub(super) fn transfer_ports(&self) -> (u16, u16, Endpoint, Endpoint, bool) {
        (
            self.port_a,
            self.port_b,
            self.endpoint_a,
            self.endpoint_b,
            self.direction_a_to_b,
        )
    }

    pub(super) fn advance_addresses(&mut self) {
        advance(&mut self.port_a, self.mode_a);
        advance(&mut self.port_b, self.mode_b);
    }

    pub(super) fn complete_byte(&mut self, transfer_t: u64, bus_cycles: u32) {
        self.advance_addresses();
        self.transferred = true;
        self.remaining = self.remaining.saturating_sub(1);
        if self.remaining == 0 {
            self.active = false;
            self.completed = true;
            self.next_transfer_t = None;
        } else {
            self.next_transfer_t = Some(transfer_t.saturating_add(self.period(bus_cycles)));
        }
        self.update_read_sequence();
    }

    pub(super) fn uses_burst_wait(&self) -> bool {
        self.burst && self.prescaler != 0
    }

    pub(super) fn byte_cycles(&self) -> u32 {
        u32::from(self.timing_a.max(2)) + u32::from(self.timing_b.max(2))
    }

    pub(super) fn period(&self, bus_cycles: u32) -> u64 {
        if self.prescaler == 0 {
            u64::from(bus_cycles)
        } else {
            (u64::from(self.prescaler) * 4).max(u64::from(bus_cycles))
        }
    }

    pub(super) fn continuous_stall(&self) -> u32 {
        let period = self.period(self.byte_cycles());
        period
            .saturating_mul(u64::from(self.remaining))
            .min(u64::from(u32::MAX)) as u32
    }

    fn write_parameter(&mut self, register: u8, value: u8) {
        match register {
            0 => self.start_a = (self.start_a & 0xff00) | u16::from(value),
            1 => self.start_a = (self.start_a & 0x00ff) | (u16::from(value) << 8),
            2 => self.length = (self.length & 0xff00) | u16::from(value),
            3 => self.length = (self.length & 0x00ff) | (u16::from(value) << 8),
            4 => self.timing_a = cycle_length(value),
            5 => {
                self.timing_b = cycle_length(value);
                if value & 0x20 != 0 {
                    self.pending.push_back(6);
                }
            }
            6 => self.prescaler = value,
            7 => self.start_b = (self.start_b & 0xff00) | u16::from(value),
            8 => self.start_b = (self.start_b & 0x00ff) | (u16::from(value) << 8),
            _ => {}
        }
    }

    fn update_read_sequence(&mut self) {
        let status = 0x1a | (u8::from(!self.completed) << 5) | u8::from(self.transferred);
        self.read_sequence = [
            status,
            self.remaining as u8,
            (self.remaining >> 8) as u8,
            self.port_a as u8,
            (self.port_a >> 8) as u8,
            self.port_b as u8,
            (self.port_b >> 8) as u8,
        ];
    }
}

fn address_mode(value: u8) -> AddressMode {
    match value {
        0 => AddressMode::Decrement,
        1 => AddressMode::Increment,
        _ => AddressMode::Fixed,
    }
}

fn cycle_length(value: u8) -> u8 {
    match value & 0x03 {
        0 => 4,
        1 => 3,
        _ => 2,
    }
}

fn advance(address: &mut u16, mode: AddressMode) {
    match mode {
        AddressMode::Decrement => *address = address.wrapping_sub(1),
        AddressMode::Increment => *address = address.wrapping_add(1),
        AddressMode::Fixed => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configure_dma(length: u16) -> NextDma {
        let mut dma = NextDma::default();
        for value in [0x7d, 0x00, 0x80, length as u8, (length >> 8) as u8] {
            dma.write(value);
        }
        dma.write(0x54); // Port A: incrementing memory, timing follows.
        dma.write(0x02);
        dma.write(0x50); // Port B: incrementing memory, timing follows.
        dma.write(0x02);
        dma.write(0xad); // Continuous, both Port B address bytes follow.
        dma.write(0x00);
        dma.write(0xa0);
        dma
    }

    #[test]
    fn register_groups_load_addresses_length_and_read_masked_status() {
        let mut dma = configure_dma(2);
        assert_eq!(dma.write(0xcf), DmaAction::Load);
        dma.loaded();
        assert_eq!((dma.port_a, dma.port_b, dma.remaining), (0x8000, 0xa000, 2));

        dma.enable(100);
        assert_eq!(dma.transfer_ports().0, 0x8000);
        dma.complete_byte(100, 4);
        assert_eq!((dma.port_a, dma.port_b, dma.remaining), (0x8001, 0xa001, 1));
        assert_eq!(dma.read_sequence[0], 0x3b); // Active and at least one byte transferred.

        dma.write(0xbb);
        dma.write(0x0b); // Status, byte counter low, Port A address low.
        assert_eq!(dma.read(), 0x3b);
        assert_eq!(dma.read(), 1);
        assert_eq!(dma.read(), 0x01);
        dma.write(0xa7);
        assert_eq!(dma.read(), 0x3b);
        dma.write(0xbb);
        dma.write(0x02); // Hide status from the normal read sequence.
        dma.write(0xbf);
        assert_eq!(dma.read(), 0x3b); // READ STATUS still returns the status byte.
        assert_eq!(dma.read(), 1); // The masked sequence remains independently positioned.
    }

    #[test]
    fn zero_length_reset_and_continue_keep_the_documented_pointer_state() {
        let mut dma = configure_dma(0);
        dma.write(0xcf);
        dma.loaded();
        dma.enable(0);
        assert!(!dma.active());
        assert_eq!(dma.read_sequence[0], 0x1a); // End of block, no bytes transferred.

        let mut dma = configure_dma(2);
        dma.write(0xcf);
        dma.loaded();
        dma.enable(0);
        dma.complete_byte(0, 4);
        dma.write(0xd3);
        dma.continued();
        assert_eq!((dma.port_a, dma.port_b, dma.remaining), (0x8001, 0xa001, 2));

        dma.write(0xc3);
        assert_eq!(
            (dma.port_a, dma.port_b, dma.length, dma.remaining),
            (0, 0, 0, 0)
        );
        assert!(!dma.active());
    }

    #[test]
    fn decrement_and_fixed_address_modes_advance_independently() {
        let mut dma = configure_dma(3);
        dma.write(0x44); // A decrements.
        dma.write(0x02);
        dma.write(0x60); // B is fixed.
        dma.write(0x02);
        dma.write(0xcf);
        dma.loaded();
        dma.enable(0);
        dma.complete_byte(0, 4);
        assert_eq!((dma.port_a, dma.port_b), (0x7fff, 0xa000));
    }

    #[test]
    fn port_b_timing_parameter_can_enable_the_zxn_fixed_prescaler() {
        let mut dma = configure_dma(1);
        dma.write(0x50);
        dma.write(0x22); // 2T timing plus prescaler parameter follows.
        dma.write(40);
        assert_eq!(dma.prescaler, 40);
        dma.write(0xcd);
        assert!(dma.burst);
        assert!(dma.uses_burst_wait());
        assert_eq!(dma.period(dma.byte_cycles()), 160);
    }
}
