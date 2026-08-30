//! Timex SCLD ports for TC2048 / TS2068 (#192).
//!
//! Phase 1 (TC2048): latch ports 0xFF and 0xF4 so Timex ROM and BASIC extensions
//! can configure display modes; extended 512×192 rendering is follow-up work.
//!
//! Phase 2a (TS2068 / TC2068): horizontal MMU uses those latches — bit 7 of
//! port 0xFF selects EX-ROM vs empty DOCK; port 0xF4 selects which 8K chunks
//! are overlaid on the home bank. AY ports F5/F6 plus R14 Timex joysticks
//! (Fuse-compatible bit layout).

/// Size of the Timex TS2068 / TC2068 EX-ROM bank (chunk 0').
pub const TIMEX_EXROM_SIZE: usize = 8192;

/// Active-high Timex joystick mask (Fuse `timex_mask`): up / down / left / right / fire.
#[must_use]
pub fn timex_joystick_mask(up: bool, down: bool, left: bool, right: bool, fire: bool) -> u8 {
    let mut v = 0u8;
    if up {
        v |= 0x01;
    }
    if down {
        v |= 0x02;
    }
    if left {
        v |= 0x04;
    }
    if right {
        v |= 0x08;
    }
    if fire {
        v |= 0x80;
    }
    v
}

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

    /// Bit 7 of port 0xFF: `true` = EX-ROM bank, `false` = DOCK / cartridge.
    #[must_use]
    pub fn use_exrom(&self) -> bool {
        self.port_ff & 0x80 != 0
    }

    /// Whether horizontal-select bit `chunk` (0..7) pages that 8K over home.
    #[must_use]
    pub fn chunk_paged(&self, chunk: u8) -> bool {
        chunk < 8 && self.port_f4 & (1 << chunk) != 0
    }

    /// 8K chunk index for a Z80 address (`addr >> 13`).
    #[must_use]
    pub fn chunk_of(addr: u16) -> u8 {
        (addr >> 13) as u8
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
    fn joystick_mask_fuse_layout() {
        assert_eq!(timex_joystick_mask(true, false, false, false, false), 0x01);
        assert_eq!(timex_joystick_mask(false, true, false, false, false), 0x02);
        assert_eq!(timex_joystick_mask(false, false, true, false, false), 0x04);
        assert_eq!(timex_joystick_mask(false, false, false, true, false), 0x08);
        assert_eq!(timex_joystick_mask(false, false, false, false, true), 0x80);
    }

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

    #[test]
    fn altmembank_and_chunk_bits() {
        let mut scld = TimexScld::new();
        assert!(!scld.use_exrom());
        assert!(!scld.chunk_paged(0));
        scld.out_port(0x00FF, 0x80);
        scld.out_port(0x00F4, 0x01);
        assert!(scld.use_exrom());
        assert!(scld.chunk_paged(0));
        assert!(!scld.chunk_paged(1));
        assert_eq!(TimexScld::chunk_of(0x1FFF), 0);
        assert_eq!(TimexScld::chunk_of(0x2000), 1);
    }
}
