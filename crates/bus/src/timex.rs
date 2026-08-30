//! Timex SCLD ports for TC2048 / TC2068 (#192).
//!
//! Phase 1 (TC2048): latch ports 0xFF and 0xF4 so Timex ROM and BASIC extensions
//! can configure display modes; extended 512×192 rendering is follow-up work.

/// Timex SCLD latch state (ports 0xFF and 0xF4).
#[derive(Clone, Debug, Default)]
pub struct TimexScld {
    port_ff: u8,
    port_f4: u8,
}

impl TimexScld {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    #[must_use]
    pub fn port_ff(&self) -> u8 {
        self.port_ff
    }

    #[must_use]
    pub fn port_f4(&self) -> u8 {
        self.port_f4
    }

    /// Handle IN; returns Some when this port is Timex-decoded.
    #[must_use]
    pub fn in_port(&self, port: u16) -> Option<u8> {
        match port & 0x00FF {
            0x00FF => Some(self.port_ff),
            0x00F4 => Some(self.port_f4),
            _ => None,
        }
    }

    /// Handle OUT; returns true when consumed.
    pub fn out_port(&mut self, port: u16, value: u8) -> bool {
        match port & 0x00FF {
            0x00FF => {
                self.port_ff = value;
                true
            }
            0x00F4 => {
                self.port_f4 = value;
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_ff_read_returns_last_write() {
        let mut scld = TimexScld::new();
        assert_eq!(scld.in_port(0x00FF), Some(0));
        scld.out_port(0x00FF, 0x42);
        assert_eq!(scld.in_port(0x00FF), Some(0x42));
    }

    #[test]
    fn port_f4_latches() {
        let mut scld = TimexScld::new();
        scld.out_port(0x00F4, 0x0F);
        assert_eq!(scld.in_port(0x00F4), Some(0x0F));
    }
}
