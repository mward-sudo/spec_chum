//! Kempston joystick (port `0x1F`).
//!
//! Active-high bits: 0=Right, 1=Left, 2=Down, 3=Up, 4=Fire.

#[derive(Clone, Copy, Debug, Default)]
pub struct Kempston {
    pub right: bool,
    pub left: bool,
    pub down: bool,
    pub up: bool,
    pub fire: bool,
}

impl Kempston {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    #[must_use]
    pub fn read(&self) -> u8 {
        let mut v = 0u8;
        if self.right {
            v |= 0x01;
        }
        if self.left {
            v |= 0x02;
        }
        if self.down {
            v |= 0x04;
        }
        if self.up {
            v |= 0x08;
        }
        if self.fire {
            v |= 0x10;
        }
        v
    }

    pub fn set_bit(&mut self, bit: u8, pressed: bool) {
        match bit {
            0 => self.right = pressed,
            1 => self.left = pressed,
            2 => self.down = pressed,
            3 => self.up = pressed,
            4 => self.fire = pressed,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_active_high() {
        let mut k = Kempston::new();
        assert_eq!(k.read(), 0);
        k.fire = true;
        k.up = true;
        assert_eq!(k.read(), 0x18);
    }
}
