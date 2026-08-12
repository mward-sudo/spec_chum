//! Z80 register file.

use core::fmt;

/// Flag bit masks in F / F'.
pub mod flag {
    pub const C: u8 = 0x01;
    pub const N: u8 = 0x02;
    pub const PV: u8 = 0x04;
    pub const X: u8 = 0x08; // undocumented bit 3
    pub const H: u8 = 0x10;
    pub const Y: u8 = 0x20; // undocumented bit 5
    pub const Z: u8 = 0x40;
    pub const S: u8 = 0x80;
}

/// Complete Z80 architectural state (excluding memory).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Registers {
    pub a: u8,
    pub f: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,
    pub a_: u8,
    pub f_: u8,
    pub b_: u8,
    pub c_: u8,
    pub d_: u8,
    pub e_: u8,
    pub h_: u8,
    pub l_: u8,
    pub ixh: u8,
    pub ixl: u8,
    pub iyh: u8,
    pub iyl: u8,
    pub sp: u16,
    pub pc: u16,
    /// Interrupt / refresh (I high, R low with bit7 separate).
    pub i: u8,
    pub r: u8,
    /// MEMPTR / WZ internal register.
    pub memptr: u16,
    /// Q latch (affects SCF/CCF undocumented flags); cleared by most ops.
    pub q: u8,
    pub iff1: bool,
    pub iff2: bool,
    pub im: u8,
    pub halted: bool,
}

impl Registers {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        *self = Self::default();
        // Power-on: PC=0, IFF=0, IM=0; AF/SP often 0xFFFF on real silicon — Fuse uses 0.
    }

    #[inline]
    #[must_use]
    pub fn af(&self) -> u16 {
        u16::from(self.a) << 8 | u16::from(self.f)
    }

    #[inline]
    pub fn set_af(&mut self, v: u16) {
        self.a = (v >> 8) as u8;
        self.f = v as u8;
    }

    #[inline]
    #[must_use]
    pub fn bc(&self) -> u16 {
        u16::from(self.b) << 8 | u16::from(self.c)
    }

    #[inline]
    pub fn set_bc(&mut self, v: u16) {
        self.b = (v >> 8) as u8;
        self.c = v as u8;
    }

    #[inline]
    #[must_use]
    pub fn de(&self) -> u16 {
        u16::from(self.d) << 8 | u16::from(self.e)
    }

    #[inline]
    pub fn set_de(&mut self, v: u16) {
        self.d = (v >> 8) as u8;
        self.e = v as u8;
    }

    #[inline]
    #[must_use]
    pub fn hl(&self) -> u16 {
        u16::from(self.h) << 8 | u16::from(self.l)
    }

    #[inline]
    pub fn set_hl(&mut self, v: u16) {
        self.h = (v >> 8) as u8;
        self.l = v as u8;
    }

    #[inline]
    #[must_use]
    pub fn ix(&self) -> u16 {
        u16::from(self.ixh) << 8 | u16::from(self.ixl)
    }

    #[inline]
    pub fn set_ix(&mut self, v: u16) {
        self.ixh = (v >> 8) as u8;
        self.ixl = v as u8;
    }

    #[inline]
    #[must_use]
    pub fn iy(&self) -> u16 {
        u16::from(self.iyh) << 8 | u16::from(self.iyl)
    }

    #[inline]
    pub fn set_iy(&mut self, v: u16) {
        self.iyh = (v >> 8) as u8;
        self.iyl = v as u8;
    }

    #[inline]
    pub fn inc_r(&mut self) {
        self.r = (self.r & 0x80) | (self.r.wrapping_add(1) & 0x7f);
    }
}

impl fmt::Display for Registers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "AF={:04X} BC={:04X} DE={:04X} HL={:04X} IX={:04X} IY={:04X} SP={:04X} PC={:04X}",
            self.af(),
            self.bc(),
            self.de(),
            self.hl(),
            self.ix(),
            self.iy(),
            self.sp,
            self.pc
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_round_trip() {
        let mut r = Registers::new();
        r.set_af(0x1234);
        r.set_bc(0x5678);
        r.set_de(0x9abc);
        r.set_hl(0xdef0);
        r.set_ix(0x1122);
        r.set_iy(0x3344);
        assert_eq!(r.af(), 0x1234);
        assert_eq!(r.bc(), 0x5678);
        assert_eq!(r.de(), 0x9abc);
        assert_eq!(r.hl(), 0xdef0);
        assert_eq!(r.ix(), 0x1122);
        assert_eq!(r.iy(), 0x3344);
    }

    #[test]
    fn r_preserves_bit7() {
        let mut r = Registers::new();
        r.r = 0x80;
        r.inc_r();
        assert_eq!(r.r, 0x81);
        r.r = 0xff;
        r.inc_r();
        assert_eq!(r.r, 0x80);
    }
}

// debug placeholder removed
