//! Kempston mouse interface.
//!
//! Ports used by Fuse / Spectrum software (exact addresses):
//! - `0xFBDF` — X axis (wraps 0..=255)
//! - `0xFFDF` — Y axis (wraps 0..=255)
//! - `0xFADF` — buttons (active-low): D0=right, D1=left, D2=middle
//!
//! Unused button bits read as `1`. Host pointer deltas are applied via
//! [`KempstonMouse::set_delta`] (positive `dy` = host down → Y decreases).

/// Kempston mouse X port.
pub const PORT_X: u16 = 0xFBDF;
/// Kempston mouse Y port.
pub const PORT_Y: u16 = 0xFFDF;
/// Kempston mouse buttons port.
pub const PORT_BUTTONS: u16 = 0xFADF;

#[derive(Clone, Copy, Debug, Default)]
pub struct KempstonMouse {
    pub x: u8,
    pub y: u8,
    pub left: bool,
    pub right: bool,
    pub middle: bool,
}

impl KempstonMouse {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Accumulate host pointer motion. Positive `dx` = right; positive `dy` = down
    /// (Y counter decrements, matching common Spectrum / Fuse conventions).
    pub fn set_delta(&mut self, dx: i8, dy: i8) {
        self.x = self.x.wrapping_add(dx as u8);
        self.y = self.y.wrapping_sub(dy as u8);
    }

    pub fn set_buttons(&mut self, left: bool, right: bool, middle: bool) {
        self.left = left;
        self.right = right;
        self.middle = middle;
    }

    /// Active-low buttons with unused bits set.
    #[must_use]
    pub fn buttons_byte(&self) -> u8 {
        let mut v = 0xffu8;
        if self.right {
            v &= !0x01;
        }
        if self.left {
            v &= !0x02;
        }
        if self.middle {
            v &= !0x04;
        }
        v
    }

    /// Decode a Kempston mouse port read, if `port` matches.
    #[must_use]
    pub fn read_port(&self, port: u16) -> Option<u8> {
        match port {
            PORT_X => Some(self.x),
            PORT_Y => Some(self.y),
            PORT_BUTTONS => Some(self.buttons_byte()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_wraps_axes() {
        let mut m = KempstonMouse::new();
        m.set_delta(10, 0);
        assert_eq!(m.x, 10);
        m.set_delta(-3, 5);
        assert_eq!(m.x, 7);
        assert_eq!(m.y, 251); // wrapping_sub(5)
        m.x = 250;
        m.set_delta(10, 0);
        assert_eq!(m.x, 4);
    }

    #[test]
    fn buttons_active_low() {
        let mut m = KempstonMouse::new();
        assert_eq!(m.buttons_byte(), 0xff);
        m.set_buttons(true, false, false);
        assert_eq!(m.buttons_byte(), 0xfd); // D1 clear
        m.set_buttons(false, true, true);
        assert_eq!(m.buttons_byte(), 0xfa); // D0 and D2 clear
    }

    #[test]
    fn port_reads() {
        let mut m = KempstonMouse::new();
        m.x = 0x42;
        m.y = 0x11;
        m.left = true;
        assert_eq!(m.read_port(PORT_X), Some(0x42));
        assert_eq!(m.read_port(PORT_Y), Some(0x11));
        assert_eq!(m.read_port(PORT_BUTTONS), Some(0xfd));
        assert_eq!(m.read_port(0x001f), None);
    }
}
