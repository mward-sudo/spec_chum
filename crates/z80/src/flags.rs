//! Flag helpers for 8-bit ALU results.

#![allow(clippy::many_single_char_names)]
#![allow(clippy::cast_lossless)]

use crate::registers::flag;

#[inline]
#[must_use]
pub fn parity(v: u8) -> bool {
    v.count_ones().is_multiple_of(2)
}

#[inline]
#[must_use]
pub fn szp(v: u8) -> u8 {
    let mut f = v & (flag::X | flag::Y | flag::S);
    if v == 0 {
        f |= flag::Z;
    }
    if parity(v) {
        f |= flag::PV;
    }
    f
}

#[inline]
#[must_use]
pub fn sz53(v: u8) -> u8 {
    let mut f = v & (flag::X | flag::Y | flag::S);
    if v == 0 {
        f |= flag::Z;
    }
    f
}

/// INC r flags (preserves C).
#[inline]
#[must_use]
pub fn inc8_flags(old: u8, c: u8) -> u8 {
    let v = old.wrapping_add(1);
    let mut f = sz53(v) | (c & flag::C);
    if v & 0x0f == 0 {
        f |= flag::H;
    }
    if v == 0x80 {
        f |= flag::PV;
    }
    f
}

/// DEC r flags (preserves C).
#[inline]
#[must_use]
pub fn dec8_flags(old: u8, c: u8) -> u8 {
    let v = old.wrapping_sub(1);
    let mut f = sz53(v) | flag::N | (c & flag::C);
    if old & 0x0f == 0 {
        f |= flag::H;
    }
    if v == 0x7f {
        f |= flag::PV;
    }
    f
}

#[inline]
#[must_use]
pub fn add8(a: u8, b: u8) -> (u8, u8) {
    let (r, c) = a.overflowing_add(b);
    let mut f = sz53(r);
    if c {
        f |= flag::C;
    }
    if (a ^ b ^ r) & 0x10 != 0 {
        f |= flag::H;
    }
    if !(a ^ b) & (a ^ r) & 0x80 != 0 {
        f |= flag::PV;
    }
    (r, f)
}

#[inline]
#[must_use]
pub fn adc8(a: u8, b: u8, carry: bool) -> (u8, u8) {
    let c_in = u8::from(carry);
    let r16 = u16::from(a) + u16::from(b) + u16::from(c_in);
    let r = r16 as u8;
    let mut f = sz53(r);
    if r16 > 0xff {
        f |= flag::C;
    }
    if (a as u16 ^ b as u16 ^ r16) & 0x10 != 0 {
        f |= flag::H;
    }
    if !(a ^ b) & (a ^ r) & 0x80 != 0 {
        f |= flag::PV;
    }
    (r, f)
}

#[inline]
#[must_use]
pub fn sub8(a: u8, b: u8) -> (u8, u8) {
    let r = a.wrapping_sub(b);
    let mut f = sz53(r) | flag::N;
    if a < b {
        f |= flag::C;
    }
    if (a ^ b ^ r) & 0x10 != 0 {
        f |= flag::H;
    }
    if (a ^ b) & (a ^ r) & 0x80 != 0 {
        f |= flag::PV;
    }
    (r, f)
}

#[inline]
#[must_use]
pub fn sbc8(a: u8, b: u8, carry: bool) -> (u8, u8) {
    let c_in = u8::from(carry);
    let r16 = u16::from(a)
        .wrapping_sub(u16::from(b))
        .wrapping_sub(u16::from(c_in));
    let r = r16 as u8;
    let mut f = sz53(r) | flag::N;
    if r16 & 0x100 != 0 {
        f |= flag::C;
    }
    if (a as u16 ^ b as u16 ^ r16) & 0x10 != 0 {
        f |= flag::H;
    }
    if (a ^ b) & (a ^ r) & 0x80 != 0 {
        f |= flag::PV;
    }
    (r, f)
}

#[inline]
#[must_use]
pub fn and8(a: u8, b: u8) -> (u8, u8) {
    let r = a & b;
    (r, szp(r) | flag::H)
}

#[inline]
#[must_use]
pub fn xor8(a: u8, b: u8) -> (u8, u8) {
    let r = a ^ b;
    (r, szp(r))
}

#[inline]
#[must_use]
pub fn or8(a: u8, b: u8) -> (u8, u8) {
    let r = a | b;
    (r, szp(r))
}

#[inline]
#[must_use]
pub fn cp8(a: u8, b: u8) -> u8 {
    let (_, mut f) = sub8(a, b);
    // CP uses operand bits 3/5, not result
    f = (f & !(flag::X | flag::Y)) | (b & (flag::X | flag::Y));
    f
}

#[inline]
#[must_use]
pub fn add16(a: u16, b: u16) -> (u16, u8, u8) {
    let (r, c) = a.overflowing_add(b);
    let mut f_mask = 0u8;
    if c {
        f_mask |= flag::C;
    }
    if (a ^ b ^ r) & 0x1000 != 0 {
        f_mask |= flag::H;
    }
    // XY from high byte of result
    let xy = ((r >> 8) as u8) & (flag::X | flag::Y);
    (r, f_mask, xy)
}
