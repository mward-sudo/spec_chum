//! Opcode decode and execute.

#![allow(clippy::many_single_char_names)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::similar_names)]
#![allow(clippy::cast_lossless)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_lossless)]
#![allow(clippy::match_same_arms)]
#![allow(clippy::identity_op)]

use crate::bus::{Io, Memory};
use crate::cpu::Cpu;
use crate::flags::{
    adc8, add16, add8, and8, cp8, dec8_flags, inc8_flags, or8, parity, sbc8, sub8, sz53, szp, xor8,
};
use crate::registers::flag;

#[derive(Clone, Copy)]
enum Idx {
    Hl,
    Ix,
    Iy,
}

pub(crate) fn execute<M: Memory, I: Io>(cpu: &mut Cpu, mem: &mut M, io: &mut I) -> u32 {
    let t0 = cpu.t;
    // Q is cleared by default; flag-affecting ops assign `regs.q = f`.
    cpu.regs.q = 0;
    let op = cpu.fetch_opcode(mem);
    match op {
        0xdd => exec_indexed(cpu, mem, io, Idx::Ix),
        0xfd => exec_indexed(cpu, mem, io, Idx::Iy),
        0xcb => exec_cb(cpu, mem, Idx::Hl, 0),
        0xed => exec_ed(cpu, mem, io),
        _ => exec_main(cpu, mem, io, op, Idx::Hl),
    }
    (cpu.t - t0) as u32
}

fn idx_addr(cpu: &Cpu, idx: Idx) -> u16 {
    match idx {
        Idx::Hl => cpu.regs.hl(),
        Idx::Ix => cpu.regs.ix(),
        Idx::Iy => cpu.regs.iy(),
    }
}

fn set_idx(cpu: &mut Cpu, idx: Idx, v: u16) {
    match idx {
        Idx::Hl => cpu.regs.set_hl(v),
        Idx::Ix => cpu.regs.set_ix(v),
        Idx::Iy => cpu.regs.set_iy(v),
    }
}

fn read_r(cpu: &Cpu, r: u8) -> u8 {
    match r & 7 {
        0 => cpu.regs.b,
        1 => cpu.regs.c,
        2 => cpu.regs.d,
        3 => cpu.regs.e,
        4 => cpu.regs.h,
        5 => cpu.regs.l,
        7 => cpu.regs.a,
        _ => unreachable!(),
    }
}

fn write_r(cpu: &mut Cpu, r: u8, v: u8) {
    match r & 7 {
        0 => cpu.regs.b = v,
        1 => cpu.regs.c = v,
        2 => cpu.regs.d = v,
        3 => cpu.regs.e = v,
        4 => cpu.regs.h = v,
        5 => cpu.regs.l = v,
        7 => cpu.regs.a = v,
        _ => unreachable!(),
    }
}

fn condition(cpu: &Cpu, cc: u8) -> bool {
    match cc & 7 {
        0 => cpu.regs.f & flag::Z == 0,
        1 => cpu.regs.f & flag::Z != 0,
        2 => cpu.regs.f & flag::C == 0,
        3 => cpu.regs.f & flag::C != 0,
        4 => cpu.regs.f & flag::PV == 0,
        5 => cpu.regs.f & flag::PV != 0,
        6 => cpu.regs.f & flag::S == 0,
        7 => cpu.regs.f & flag::S != 0,
        _ => unreachable!(),
    }
}

fn get_disp_addr<M: Memory>(cpu: &mut Cpu, mem: &mut M, idx: Idx) -> u16 {
    let d = cpu.fetch8(mem) as i8 as i16;
    let base = idx_addr(cpu, idx);
    let addr = (base as i16).wrapping_add(d) as u16;
    cpu.regs.memptr = addr;
    // 5 T internal for indexed address calculation
    cpu.add_t(5);
    addr
}

fn exec_indexed<M: Memory, I: Io>(cpu: &mut Cpu, mem: &mut M, io: &mut I, idx: Idx) {
    let op = cpu.fetch_opcode(mem);
    match op {
        0xcb => {
            let d = cpu.fetch8(mem) as i8 as i16;
            let op2 = cpu.fetch8(mem);
            // Displacement timing: 2 bytes already fetched as data (3+3), plus 2 IR waits?
            // Fuse: after DD CB d op — total timing baked into mem accesses + 5+5 for index
            let base = idx_addr(cpu, idx);
            let addr = (base as i16).wrapping_add(d) as u16;
            cpu.regs.memptr = addr;
            cpu.add_t(2); // additional internal
            exec_cb_addr(cpu, mem, op2, addr);
        }
        0xdd | 0xfd => {
            // nested prefix: treat as new prefix (consume and restart)
            // Simpler: ignore and re-fetch — real Z80 redefines; Fuse rarely nests.
            exec_indexed(cpu, mem, io, if op == 0xdd { Idx::Ix } else { Idx::Iy });
        }
        _ => exec_main(cpu, mem, io, op, idx),
    }
}

#[allow(clippy::too_many_lines)]
fn exec_main<M: Memory, I: Io>(cpu: &mut Cpu, mem: &mut M, io: &mut I, op: u8, idx: Idx) {
    match op {
        0x00 => {} // NOP
        0x01 | 0x11 | 0x21 | 0x31 => {
            let v = cpu.fetch16(mem);
            match (op >> 4) & 3 {
                0 => cpu.regs.set_bc(v),
                1 => cpu.regs.set_de(v),
                2 => set_idx(cpu, idx, v),
                3 => cpu.regs.sp = v,
                _ => unreachable!(),
            }
        }
        0x02 => {
            let a = cpu.regs.bc();
            cpu.write_mem(mem, a, cpu.regs.a);
            cpu.regs.memptr = (u16::from(cpu.regs.a) << 8) | (a.wrapping_add(1) & 0xff);
        }
        0x12 => {
            let a = cpu.regs.de();
            cpu.write_mem(mem, a, cpu.regs.a);
            cpu.regs.memptr = (u16::from(cpu.regs.a) << 8) | (a.wrapping_add(1) & 0xff);
        }
        0x0a => {
            let a = cpu.regs.bc();
            cpu.regs.a = cpu.read_mem(mem, a);
            cpu.regs.memptr = a.wrapping_add(1);
        }
        0x1a => {
            let a = cpu.regs.de();
            cpu.regs.a = cpu.read_mem(mem, a);
            cpu.regs.memptr = a.wrapping_add(1);
        }
        0x22 => {
            let a = cpu.fetch16(mem);
            let v = idx_addr(cpu, idx);
            cpu.write_mem(mem, a, v as u8);
            cpu.write_mem(mem, a.wrapping_add(1), (v >> 8) as u8);
            cpu.regs.memptr = a.wrapping_add(1);
        }
        0x2a => {
            let a = cpu.fetch16(mem);
            let lo = cpu.read_mem(mem, a);
            let hi = cpu.read_mem(mem, a.wrapping_add(1));
            set_idx(cpu, idx, u16::from(hi) << 8 | u16::from(lo));
            cpu.regs.memptr = a.wrapping_add(1);
        }
        0x32 => {
            let a = cpu.fetch16(mem);
            cpu.write_mem(mem, a, cpu.regs.a);
            cpu.regs.memptr = (u16::from(cpu.regs.a) << 8) | (a.wrapping_add(1) & 0xff);
        }
        0x3a => {
            let a = cpu.fetch16(mem);
            cpu.regs.a = cpu.read_mem(mem, a);
            cpu.regs.memptr = a.wrapping_add(1);
        }
        0x03 | 0x13 | 0x23 | 0x33 => {
            cpu.add_t(2);
            match (op >> 4) & 3 {
                0 => cpu.regs.set_bc(cpu.regs.bc().wrapping_add(1)),
                1 => cpu.regs.set_de(cpu.regs.de().wrapping_add(1)),
                2 => set_idx(cpu, idx, idx_addr(cpu, idx).wrapping_add(1)),
                3 => cpu.regs.sp = cpu.regs.sp.wrapping_add(1),
                _ => unreachable!(),
            }
        }
        0x0b | 0x1b | 0x2b | 0x3b => {
            cpu.add_t(2);
            match (op >> 4) & 3 {
                0 => cpu.regs.set_bc(cpu.regs.bc().wrapping_sub(1)),
                1 => cpu.regs.set_de(cpu.regs.de().wrapping_sub(1)),
                2 => set_idx(cpu, idx, idx_addr(cpu, idx).wrapping_sub(1)),
                3 => cpu.regs.sp = cpu.regs.sp.wrapping_sub(1),
                _ => unreachable!(),
            }
        }
        0x04 | 0x0c | 0x14 | 0x1c | 0x24 | 0x2c | 0x3c => {
            let r = (op >> 3) & 7;
            let old = read_r_idx(cpu, r, idx);
            let v = old.wrapping_add(1);
            write_r_idx(cpu, r, idx, v);
            cpu.regs.f = inc8_flags(old, cpu.regs.f);
        }
        0x34 => {
            let addr = if matches!(idx, Idx::Hl) {
                idx_addr(cpu, idx)
            } else {
                get_disp_addr(cpu, mem, idx)
            };
            let old = cpu.read_mem(mem, addr);
            cpu.add_t(1);
            let v = old.wrapping_add(1);
            cpu.write_mem(mem, addr, v);
            cpu.regs.f = inc8_flags(old, cpu.regs.f);
        }
        0x05 | 0x0d | 0x15 | 0x1d | 0x25 | 0x2d | 0x3d => {
            let r = (op >> 3) & 7;
            let old = read_r_idx(cpu, r, idx);
            let v = old.wrapping_sub(1);
            write_r_idx(cpu, r, idx, v);
            cpu.regs.f = dec8_flags(old, cpu.regs.f);
        }
        0x35 => {
            let addr = if matches!(idx, Idx::Hl) {
                idx_addr(cpu, idx)
            } else {
                get_disp_addr(cpu, mem, idx)
            };
            let old = cpu.read_mem(mem, addr);
            cpu.add_t(1);
            let v = old.wrapping_sub(1);
            cpu.write_mem(mem, addr, v);
            cpu.regs.f = dec8_flags(old, cpu.regs.f);
        }
        0x06 | 0x0e | 0x16 | 0x1e | 0x26 | 0x2e | 0x3e => {
            let r = (op >> 3) & 7;
            let v = cpu.fetch8(mem);
            write_r_idx(cpu, r, idx, v);
        }
        0x36 => {
            let addr = if matches!(idx, Idx::Hl) {
                let a = idx_addr(cpu, idx);
                let v = cpu.fetch8(mem);
                (a, v)
            } else {
                let d = cpu.fetch8(mem) as i8 as i16;
                let v = cpu.fetch8(mem);
                let base = idx_addr(cpu, idx);
                let addr = (base as i16).wrapping_add(d) as u16;
                cpu.regs.memptr = addr;
                cpu.add_t(2);
                (addr, v)
            };
            cpu.write_mem(mem, addr.0, addr.1);
        }
        0x07 => {
            // RLCA
            let a = cpu.regs.a;
            let c = a >> 7;
            let r = (a << 1) | c;
            cpu.regs.a = r;
            cpu.regs.f = (cpu.regs.f & (flag::S | flag::Z | flag::PV))
                | (r & (flag::X | flag::Y))
                | if c != 0 { flag::C } else { 0 };
            cpu.regs.q = cpu.regs.f;
        }
        0x0f => {
            let a = cpu.regs.a;
            let c = a & 1;
            let r = (a >> 1) | (c << 7);
            cpu.regs.a = r;
            cpu.regs.f = (cpu.regs.f & (flag::S | flag::Z | flag::PV))
                | (r & (flag::X | flag::Y))
                | if c != 0 { flag::C } else { 0 };
            cpu.regs.q = cpu.regs.f;
        }
        0x17 => {
            let a = cpu.regs.a;
            let old_c = u8::from(cpu.regs.f & flag::C != 0);
            let c = a >> 7;
            let r = (a << 1) | old_c;
            cpu.regs.a = r;
            cpu.regs.f = (cpu.regs.f & (flag::S | flag::Z | flag::PV))
                | (r & (flag::X | flag::Y))
                | if c != 0 { flag::C } else { 0 };
            cpu.regs.q = cpu.regs.f;
        }
        0x1f => {
            let a = cpu.regs.a;
            let old_c = u8::from(cpu.regs.f & flag::C != 0);
            let c = a & 1;
            let r = (a >> 1) | (old_c << 7);
            cpu.regs.a = r;
            cpu.regs.f = (cpu.regs.f & (flag::S | flag::Z | flag::PV))
                | (r & (flag::X | flag::Y))
                | if c != 0 { flag::C } else { 0 };
            cpu.regs.q = cpu.regs.f;
        }
        0x08 => {
            let af = cpu.regs.af();
            cpu.regs
                .set_af(u16::from(cpu.regs.a_) << 8 | u16::from(cpu.regs.f_));
            cpu.regs.a_ = (af >> 8) as u8;
            cpu.regs.f_ = af as u8;
        }
        0x09 | 0x19 | 0x29 | 0x39 => {
            cpu.add_t(7);
            let a = idx_addr(cpu, idx);
            let b = match (op >> 4) & 3 {
                0 => cpu.regs.bc(),
                1 => cpu.regs.de(),
                2 => idx_addr(cpu, idx),
                3 => cpu.regs.sp,
                _ => unreachable!(),
            };
            let (r, f_add, xy) = add16(a, b);
            set_idx(cpu, idx, r);
            cpu.regs.memptr = a.wrapping_add(1);
            cpu.regs.f = (cpu.regs.f & (flag::S | flag::Z | flag::PV)) | f_add | xy;
            cpu.regs.q = cpu.regs.f;
        }
        0x10 => {
            // DJNZ
            cpu.add_t(1);
            cpu.regs.b = cpu.regs.b.wrapping_sub(1);
            let d = cpu.fetch8(mem) as i8 as i16;
            if cpu.regs.b != 0 {
                cpu.add_t(5);
                let dest = (cpu.regs.pc as i16).wrapping_add(d) as u16;
                cpu.regs.pc = dest;
                cpu.regs.memptr = dest;
            }
        }
        0x18 => {
            let d = cpu.fetch8(mem) as i8 as i16;
            cpu.add_t(5);
            let dest = (cpu.regs.pc as i16).wrapping_add(d) as u16;
            cpu.regs.pc = dest;
            cpu.regs.memptr = dest;
        }
        0x20 | 0x28 | 0x30 | 0x38 => {
            let d = cpu.fetch8(mem) as i8 as i16;
            let cc = (op >> 3) & 3;
            // NZ Z NC C — encoding uses 4..7 style via bits; 0x20=NZ(4?); actually:
            // 0x20 NZ, 0x28 Z, 0x30 NC, 0x38 C → cc = (op>>3)&3 maps to 0..3 but condition() uses 0=NZ
            let taken = condition(cpu, cc);
            if taken {
                cpu.add_t(5);
                let dest = (cpu.regs.pc as i16).wrapping_add(d) as u16;
                cpu.regs.pc = dest;
                cpu.regs.memptr = dest;
            }
        }
        0x27 => daa(cpu),
        0x2f => {
            cpu.regs.a ^= 0xff;
            cpu.regs.f = (cpu.regs.f & (flag::S | flag::Z | flag::PV | flag::C))
                | flag::H
                | flag::N
                | (cpu.regs.a & (flag::X | flag::Y));
            cpu.regs.q = cpu.regs.f;
        }
        0x37 => {
            // SCF — XY from A|F (Fuse / NMOS)
            let old = cpu.regs.f;
            cpu.regs.f = (old & (flag::S | flag::Z | flag::PV))
                | flag::C
                | ((cpu.regs.a | old) & (flag::X | flag::Y));
            cpu.regs.q = cpu.regs.f;
        }
        0x3f => {
            // CCF — XY from A|F
            let old = cpu.regs.f;
            let c = old & flag::C;
            cpu.regs.f = (old & (flag::S | flag::Z | flag::PV))
                | ((cpu.regs.a | old) & (flag::X | flag::Y))
                | if c != 0 { flag::H } else { 0 }
                | if c == 0 { flag::C } else { 0 };
            cpu.regs.q = cpu.regs.f;
        }
        0x76 => {
            cpu.regs.halted = true;
            // PC points at HALT so interrupt resumes it
            cpu.regs.pc = cpu.regs.pc.wrapping_sub(1);
        }
        // LD r,r' / ALU / etc.
        0x40..=0x75 | 0x77..=0x7f => {
            let dst = (op >> 3) & 7;
            let src = op & 7;
            if dst == 6 {
                let addr = if matches!(idx, Idx::Hl) {
                    idx_addr(cpu, idx)
                } else {
                    get_disp_addr(cpu, mem, idx)
                };
                let v = read_r(cpu, src);
                cpu.write_mem(mem, addr, v);
            } else if src == 6 {
                let addr = if matches!(idx, Idx::Hl) {
                    idx_addr(cpu, idx)
                } else {
                    get_disp_addr(cpu, mem, idx)
                };
                let v = cpu.read_mem(mem, addr);
                write_r(cpu, dst, v);
            } else {
                // For DD/FD, H/L refer to IXH/IXL when not (HL)
                let v = read_r_idx(cpu, src, idx);
                write_r_idx(cpu, dst, idx, v);
            }
        }
        0x80..=0xbf => {
            let src = op & 7;
            let alu = (op >> 3) & 7;
            let b = if src == 6 {
                let addr = if matches!(idx, Idx::Hl) {
                    idx_addr(cpu, idx)
                } else {
                    get_disp_addr(cpu, mem, idx)
                };
                cpu.read_mem(mem, addr)
            } else {
                read_r_idx(cpu, src, idx)
            };
            alu_a(cpu, alu, b);
        }
        0xc6 | 0xce | 0xd6 | 0xde | 0xe6 | 0xee | 0xf6 | 0xfe => {
            let alu = (op >> 3) & 7;
            let b = cpu.fetch8(mem);
            alu_a(cpu, alu, b);
        }
        0xc3 => {
            let a = cpu.fetch16(mem);
            cpu.regs.pc = a;
            cpu.regs.memptr = a;
        }
        0xc2 | 0xca | 0xd2 | 0xda | 0xe2 | 0xea | 0xf2 | 0xfa => {
            let a = cpu.fetch16(mem);
            cpu.regs.memptr = a;
            if condition(cpu, (op >> 3) & 7) {
                cpu.regs.pc = a;
            }
        }
        0xc4 | 0xcc | 0xd4 | 0xdc | 0xe4 | 0xec | 0xf4 | 0xfc => {
            let a = cpu.fetch16(mem);
            cpu.regs.memptr = a;
            if condition(cpu, (op >> 3) & 7) {
                cpu.add_t(1);
                cpu.push(mem, cpu.regs.pc);
                cpu.regs.pc = a;
            }
        }
        0xcd => {
            let a = cpu.fetch16(mem);
            cpu.add_t(1);
            cpu.push(mem, cpu.regs.pc);
            cpu.regs.pc = a;
            cpu.regs.memptr = a;
        }
        0xc9 => {
            let a = cpu.pop(mem);
            cpu.regs.pc = a;
            cpu.regs.memptr = a;
        }
        0xc0 | 0xc8 | 0xd0 | 0xd8 | 0xe0 | 0xe8 | 0xf0 | 0xf8 => {
            cpu.add_t(1);
            if condition(cpu, (op >> 3) & 7) {
                let a = cpu.pop(mem);
                cpu.regs.pc = a;
                cpu.regs.memptr = a;
            }
        }
        0xc1 | 0xd1 | 0xe1 | 0xf1 => {
            let v = cpu.pop(mem);
            match (op >> 4) & 3 {
                0 => cpu.regs.set_bc(v),
                1 => cpu.regs.set_de(v),
                2 => set_idx(cpu, idx, v),
                3 => cpu.regs.set_af(v),
                _ => unreachable!(),
            }
        }
        0xc5 | 0xd5 | 0xe5 | 0xf5 => {
            cpu.add_t(1);
            let v = match (op >> 4) & 3 {
                0 => cpu.regs.bc(),
                1 => cpu.regs.de(),
                2 => idx_addr(cpu, idx),
                3 => cpu.regs.af(),
                _ => unreachable!(),
            };
            cpu.push(mem, v);
        }
        0xc7 | 0xcf | 0xd7 | 0xdf | 0xe7 | 0xef | 0xf7 | 0xff => {
            cpu.add_t(1);
            let a = u16::from(op & 0x38);
            cpu.push(mem, cpu.regs.pc);
            cpu.regs.pc = a;
            cpu.regs.memptr = a;
        }
        0xd3 => {
            let n = cpu.fetch8(mem);
            let port = u16::from(cpu.regs.a) << 8 | u16::from(n);
            cpu.out_port(io, port, cpu.regs.a);
        }
        0xdb => {
            let n = cpu.fetch8(mem);
            let port = u16::from(cpu.regs.a) << 8 | u16::from(n);
            cpu.regs.a = cpu.in_port(io, port);
        }
        0xd9 => {
            let (b, c, d, e, h, l) = (
                cpu.regs.b, cpu.regs.c, cpu.regs.d, cpu.regs.e, cpu.regs.h, cpu.regs.l,
            );
            cpu.regs.b = cpu.regs.b_;
            cpu.regs.c = cpu.regs.c_;
            cpu.regs.d = cpu.regs.d_;
            cpu.regs.e = cpu.regs.e_;
            cpu.regs.h = cpu.regs.h_;
            cpu.regs.l = cpu.regs.l_;
            cpu.regs.b_ = b;
            cpu.regs.c_ = c;
            cpu.regs.d_ = d;
            cpu.regs.e_ = e;
            cpu.regs.h_ = h;
            cpu.regs.l_ = l;
        }
        0xe3 => {
            // EX (SP), HL/IX/IY — 19 T (23 with DD/FD prefix already counted)
            let sp = cpu.regs.sp;
            let lo = cpu.read_mem(mem, sp);
            let hi = cpu.read_mem(mem, sp.wrapping_add(1));
            cpu.add_t(1);
            let hl = idx_addr(cpu, idx);
            cpu.write_mem(mem, sp.wrapping_add(1), (hl >> 8) as u8);
            cpu.write_mem(mem, sp, hl as u8);
            cpu.add_t(2);
            set_idx(cpu, idx, u16::from(hi) << 8 | u16::from(lo));
            cpu.regs.memptr = idx_addr(cpu, idx);
        }
        0xe9 => cpu.regs.pc = idx_addr(cpu, idx),
        0xeb => {
            let de = cpu.regs.de();
            cpu.regs.set_de(cpu.regs.hl());
            cpu.regs.set_hl(de);
        }
        0xf3 => {
            cpu.regs.iff1 = false;
            cpu.regs.iff2 = false;
        }
        0xfb => {
            cpu.regs.iff1 = true;
            cpu.regs.iff2 = true;
            cpu.interrupt_deferred = true;
        }
        0xf9 => {
            cpu.add_t(2);
            cpu.regs.sp = idx_addr(cpu, idx);
        }
        _ => {}
    }
}

fn read_r_idx(cpu: &Cpu, r: u8, idx: Idx) -> u8 {
    match (r & 7, idx) {
        (4, Idx::Ix) => cpu.regs.ixh,
        (5, Idx::Ix) => cpu.regs.ixl,
        (4, Idx::Iy) => cpu.regs.iyh,
        (5, Idx::Iy) => cpu.regs.iyl,
        _ => read_r(cpu, r),
    }
}

fn write_r_idx(cpu: &mut Cpu, r: u8, idx: Idx, v: u8) {
    match (r & 7, idx) {
        (4, Idx::Ix) => cpu.regs.ixh = v,
        (5, Idx::Ix) => cpu.regs.ixl = v,
        (4, Idx::Iy) => cpu.regs.iyh = v,
        (5, Idx::Iy) => cpu.regs.iyl = v,
        _ => write_r(cpu, r, v),
    }
}

fn alu_a(cpu: &mut Cpu, alu: u8, b: u8) {
    let a = cpu.regs.a;
    let (r, f) = match alu {
        0 => add8(a, b),
        1 => adc8(a, b, cpu.regs.f & flag::C != 0),
        2 => sub8(a, b),
        3 => sbc8(a, b, cpu.regs.f & flag::C != 0),
        4 => and8(a, b),
        5 => xor8(a, b),
        6 => or8(a, b),
        7 => {
            cpu.regs.f = cp8(a, b);
            cpu.regs.q = cpu.regs.f;
            return;
        }
        _ => unreachable!(),
    };
    cpu.regs.a = r;
    cpu.regs.f = f;
    cpu.regs.q = f;
}

fn daa(cpu: &mut Cpu) {
    let mut a = u16::from(cpu.regs.a);
    let mut f = cpu.regs.f;
    let n = f & flag::N != 0;
    let mut correction = 0u16;
    // Low/high corrections apply regardless of N (N only chooses add vs sub).
    if f & flag::H != 0 || a & 0x0f > 9 {
        correction |= 0x06;
    }
    if f & flag::C != 0 || a > 0x99 {
        correction |= 0x60;
        f |= flag::C;
    }
    if n {
        a = a.wrapping_sub(correction);
    } else {
        a = a.wrapping_add(correction);
    }
    let r = a as u8;
    f = (f & (flag::C | flag::N)) | szp(r);
    if (cpu.regs.a ^ r) & 0x10 != 0 {
        f |= flag::H;
    }
    cpu.regs.a = r;
    cpu.regs.f = f;
    cpu.regs.q = f;
}

fn exec_cb<M: Memory>(cpu: &mut Cpu, mem: &mut M, idx: Idx, addr_override: u16) {
    let _ = (idx, addr_override);
    let op = cpu.fetch_opcode(mem);
    exec_cb_op(cpu, mem, op, None);
}

fn exec_cb_addr<M: Memory>(cpu: &mut Cpu, mem: &mut M, op: u8, addr: u16) {
    exec_cb_op(cpu, mem, op, Some(addr));
}

fn exec_cb_op<M: Memory>(cpu: &mut Cpu, mem: &mut M, op: u8, addr: Option<u16>) {
    let r = op & 7;
    let group = op >> 6;
    let y = (op >> 3) & 7;

    let get = |cpu: &mut Cpu, mem: &mut M| -> u8 {
        if let Some(a) = addr {
            cpu.read_mem(mem, a)
        } else if r == 6 {
            cpu.read_mem(mem, cpu.regs.hl())
        } else {
            read_r(cpu, r)
        }
    };
    let set = |cpu: &mut Cpu, mem: &mut M, v: u8| {
        if let Some(a) = addr {
            cpu.write_mem(mem, a, v);
            if r != 6 {
                write_r(cpu, r, v);
            }
        } else if r == 6 {
            cpu.write_mem(mem, cpu.regs.hl(), v);
        } else {
            write_r(cpu, r, v);
        }
    };

    match group {
        0 => {
            let v = get(cpu, mem);
            let (out, f) = rot_shift(y, v, cpu.regs.f & flag::C != 0);
            if y != 6 {
                // BIT is group 1
            }
            set(cpu, mem, out);
            cpu.regs.f = f;
            cpu.regs.q = f;
            if addr.is_some() || r == 6 {
                cpu.add_t(1);
            }
        }
        1 => {
            // BIT
            let v = get(cpu, mem);
            let bit = 1 << y;
            let mut f = (cpu.regs.f & flag::C) | flag::H | sz53(v & bit);
            if v & bit == 0 {
                f |= flag::Z | flag::PV;
            }
            if addr.is_some() || r == 6 {
                // XY from memptr high
                f = (f & !(flag::X | flag::Y))
                    | ((cpu.regs.memptr >> 8) as u8 & (flag::X | flag::Y));
                cpu.add_t(1);
            } else {
                f = (f & !(flag::X | flag::Y)) | (v & (flag::X | flag::Y));
            }
            // S from tested bit 7
            if y == 7 && v & bit != 0 {
                f |= flag::S;
            }
            cpu.regs.f = f;
            cpu.regs.q = f;
        }
        2 => {
            let v = get(cpu, mem) & !(1 << y);
            set(cpu, mem, v);
            if addr.is_some() || r == 6 {
                cpu.add_t(1);
            }
        }
        3 => {
            let v = get(cpu, mem) | (1 << y);
            set(cpu, mem, v);
            if addr.is_some() || r == 6 {
                cpu.add_t(1);
            }
        }
        _ => {}
    }
}

fn rot_shift(y: u8, v: u8, c_in: bool) -> (u8, u8) {
    let (r, c) = match y {
        0 => {
            // RLC
            let c = v >> 7;
            ((v << 1) | c, c != 0)
        }
        1 => {
            let c = v & 1;
            ((v >> 1) | (c << 7), c != 0)
        }
        2 => {
            let c = v >> 7;
            ((v << 1) | u8::from(c_in), c != 0)
        }
        3 => {
            let c = v & 1;
            ((v >> 1) | (u8::from(c_in) << 7), c != 0)
        }
        4 => ((v << 1), v & 0x80 != 0),           // SLA
        5 => ((v >> 1) | (v & 0x80), v & 1 != 0), // SRA
        6 => ((v << 1) | 1, v & 0x80 != 0),       // SLL undocumented
        7 => ((v >> 1), v & 1 != 0),              // SRL
        _ => unreachable!(),
    };
    let mut f = szp(r);
    if c {
        f |= flag::C;
    }
    (r, f)
}

#[allow(clippy::too_many_lines)]
fn exec_ed<M: Memory, I: Io>(cpu: &mut Cpu, mem: &mut M, io: &mut I) {
    let op = cpu.fetch_opcode(mem);
    match op {
        0x40 | 0x48 | 0x50 | 0x58 | 0x60 | 0x68 | 0x70 | 0x78 => {
            let r = (op >> 3) & 7;
            let port = cpu.regs.bc();
            let v = cpu.in_port(io, port);
            if r != 6 {
                write_r(cpu, r, v);
            }
            // IN sets flags from value (including undocumented IN F,(C) / 0x70)
            cpu.regs.f = (cpu.regs.f & flag::C) | szp(v);
            cpu.regs.q = cpu.regs.f;
        }
        0x41 | 0x49 | 0x51 | 0x59 | 0x61 | 0x69 | 0x71 | 0x79 => {
            let r = (op >> 3) & 7;
            let v = if r == 6 { 0 } else { read_r(cpu, r) };
            cpu.out_port(io, cpu.regs.bc(), v);
        }
        0x42 | 0x52 | 0x62 | 0x72 => {
            cpu.add_t(7);
            let a = cpu.regs.hl();
            let b = match (op >> 4) & 3 {
                0 => cpu.regs.bc(),
                1 => cpu.regs.de(),
                2 => cpu.regs.hl(),
                3 => cpu.regs.sp,
                _ => unreachable!(),
            };
            let (r, f) = sbc16(a, b, cpu.regs.f & flag::C != 0);
            cpu.regs.set_hl(r);
            cpu.regs.memptr = a.wrapping_add(1);
            cpu.regs.f = f;
            cpu.regs.q = f;
        }
        0x4a | 0x5a | 0x6a | 0x7a => {
            cpu.add_t(7);
            let a = cpu.regs.hl();
            let b = match (op >> 4) & 3 {
                0 => cpu.regs.bc(),
                1 => cpu.regs.de(),
                2 => cpu.regs.hl(),
                3 => cpu.regs.sp,
                _ => unreachable!(),
            };
            let (r, f) = adc16(a, b, cpu.regs.f & flag::C != 0);
            cpu.regs.set_hl(r);
            cpu.regs.memptr = a.wrapping_add(1);
            cpu.regs.f = f;
            cpu.regs.q = f;
        }
        0x43 | 0x53 | 0x63 | 0x73 => {
            let a = cpu.fetch16(mem);
            let v = match (op >> 4) & 3 {
                0 => cpu.regs.bc(),
                1 => cpu.regs.de(),
                2 => cpu.regs.hl(),
                3 => cpu.regs.sp,
                _ => unreachable!(),
            };
            cpu.write_mem(mem, a, v as u8);
            cpu.write_mem(mem, a.wrapping_add(1), (v >> 8) as u8);
            cpu.regs.memptr = a.wrapping_add(1);
        }
        0x4b | 0x5b | 0x6b | 0x7b => {
            let a = cpu.fetch16(mem);
            let lo = cpu.read_mem(mem, a);
            let hi = cpu.read_mem(mem, a.wrapping_add(1));
            let v = u16::from(hi) << 8 | u16::from(lo);
            match (op >> 4) & 3 {
                0 => cpu.regs.set_bc(v),
                1 => cpu.regs.set_de(v),
                2 => cpu.regs.set_hl(v),
                3 => cpu.regs.sp = v,
                _ => unreachable!(),
            }
            cpu.regs.memptr = a.wrapping_add(1);
        }
        0x44 | 0x4c | 0x54 | 0x5c | 0x64 | 0x6c | 0x74 | 0x7c => {
            let (r, f) = sub8(0, cpu.regs.a);
            cpu.regs.a = r;
            cpu.regs.f = f;
            cpu.regs.q = f;
        }
        0x45 | 0x4d | 0x55 | 0x5d | 0x65 | 0x6d | 0x75 | 0x7d => {
            // RETN / RETI
            cpu.regs.iff1 = cpu.regs.iff2;
            let a = cpu.pop(mem);
            cpu.regs.pc = a;
            cpu.regs.memptr = a;
        }
        0x46 | 0x4e | 0x66 | 0x6e => cpu.regs.im = 0,
        0x56 | 0x76 => cpu.regs.im = 1,
        0x5e | 0x7e => cpu.regs.im = 2,
        0x47 => {
            cpu.add_t(1);
            cpu.regs.i = cpu.regs.a;
        }
        0x4f => {
            cpu.add_t(1);
            cpu.regs.r = cpu.regs.a;
        }
        0x57 => {
            cpu.add_t(1);
            cpu.regs.a = cpu.regs.i;
            cpu.regs.f = (cpu.regs.f & flag::C) | sz53(cpu.regs.a);
            if cpu.regs.iff2 {
                cpu.regs.f |= flag::PV;
            }
            cpu.regs.q = cpu.regs.f;
        }
        0x5f => {
            cpu.add_t(1);
            cpu.regs.a = cpu.regs.r;
            cpu.regs.f = (cpu.regs.f & flag::C) | sz53(cpu.regs.a);
            if cpu.regs.iff2 {
                cpu.regs.f |= flag::PV;
            }
            cpu.regs.q = cpu.regs.f;
        }
        0x67 => rrd(cpu, mem),
        0x6f => rld(cpu, mem),
        0xa0 => block_ld(cpu, mem, true, false),
        0xa8 => block_ld(cpu, mem, false, false),
        0xb0 => block_ld(cpu, mem, true, true),
        0xb8 => block_ld(cpu, mem, false, true),
        0xa1 => block_cp(cpu, mem, true, false),
        0xa9 => block_cp(cpu, mem, false, false),
        0xb1 => block_cp(cpu, mem, true, true),
        0xb9 => block_cp(cpu, mem, false, true),
        0xa2 => block_in(cpu, mem, io, true, false),
        0xaa => block_in(cpu, mem, io, false, false),
        0xb2 => block_in(cpu, mem, io, true, true),
        0xba => block_in(cpu, mem, io, false, true),
        0xa3 => block_out(cpu, mem, io, true, false),
        0xab => block_out(cpu, mem, io, false, false),
        0xb3 => block_out(cpu, mem, io, true, true),
        0xbb => block_out(cpu, mem, io, false, true),
        _ => {}
    }
}

fn sbc16(a: u16, b: u16, c: bool) -> (u16, u8) {
    let c_in = u16::from(c);
    let r32 = u32::from(a)
        .wrapping_sub(u32::from(b))
        .wrapping_sub(u32::from(c_in));
    let r = r32 as u16;
    let mut f = flag::N | sz53((r >> 8) as u8);
    if r == 0 {
        f |= flag::Z;
    }
    if r32 & 0x1_0000 != 0 {
        f |= flag::C;
    }
    if (a ^ b ^ r) & 0x1000 != 0 {
        f |= flag::H;
    }
    if (a ^ b) & (a ^ r) & 0x8000 != 0 {
        f |= flag::PV;
    }
    if r & 0x8000 != 0 {
        f |= flag::S;
    }
    (r, f)
}

fn adc16(a: u16, b: u16, c: bool) -> (u16, u8) {
    let c_in = u16::from(c);
    let r32 = u32::from(a) + u32::from(b) + u32::from(c_in);
    let r = r32 as u16;
    let mut f = sz53((r >> 8) as u8);
    if r == 0 {
        f |= flag::Z;
    }
    if r32 > 0xffff {
        f |= flag::C;
    }
    if (a ^ b ^ r) & 0x1000 != 0 {
        f |= flag::H;
    }
    if !(a ^ b) & (a ^ r) & 0x8000 != 0 {
        f |= flag::PV;
    }
    if r & 0x8000 != 0 {
        f |= flag::S;
    }
    (r, f)
}

fn rrd<M: Memory>(cpu: &mut Cpu, mem: &mut M) {
    let addr = cpu.regs.hl();
    let m = cpu.read_mem(mem, addr);
    let a = cpu.regs.a;
    cpu.write_mem(mem, addr, (a << 4) | (m >> 4));
    cpu.add_t(4);
    cpu.regs.a = (a & 0xf0) | (m & 0x0f);
    cpu.regs.f = (cpu.regs.f & flag::C) | szp(cpu.regs.a);
    cpu.regs.memptr = addr.wrapping_add(1);
    cpu.regs.q = cpu.regs.f;
}

fn rld<M: Memory>(cpu: &mut Cpu, mem: &mut M) {
    let addr = cpu.regs.hl();
    let m = cpu.read_mem(mem, addr);
    let a = cpu.regs.a;
    cpu.write_mem(mem, addr, (m << 4) | (a & 0x0f));
    cpu.add_t(4);
    cpu.regs.a = (a & 0xf0) | (m >> 4);
    cpu.regs.f = (cpu.regs.f & flag::C) | szp(cpu.regs.a);
    cpu.regs.memptr = addr.wrapping_add(1);
    cpu.regs.q = cpu.regs.f;
}

fn block_ld<M: Memory>(cpu: &mut Cpu, mem: &mut M, inc: bool, repeat: bool) {
    let hl = cpu.regs.hl();
    let de = cpu.regs.de();
    let v = cpu.read_mem(mem, hl);
    cpu.write_mem(mem, de, v);
    cpu.add_t(2);
    if inc {
        cpu.regs.set_hl(hl.wrapping_add(1));
        cpu.regs.set_de(de.wrapping_add(1));
    } else {
        cpu.regs.set_hl(hl.wrapping_sub(1));
        cpu.regs.set_de(de.wrapping_sub(1));
    }
    let bc = cpu.regs.bc().wrapping_sub(1);
    cpu.regs.set_bc(bc);
    let n = cpu.regs.a.wrapping_add(v);
    let mut f = cpu.regs.f & (flag::S | flag::Z | flag::C);
    if bc != 0 {
        f |= flag::PV;
    }
    if n & 0x02 != 0 {
        f |= flag::Y;
    }
    if n & 0x08 != 0 {
        f |= flag::X;
    }
    cpu.regs.f = f;
    cpu.regs.q = f;
    if repeat && bc != 0 {
        cpu.add_t(5);
        cpu.regs.pc = cpu.regs.pc.wrapping_sub(2);
        cpu.regs.memptr = cpu.regs.pc.wrapping_add(1);
    }
}

fn block_cp<M: Memory>(cpu: &mut Cpu, mem: &mut M, inc: bool, repeat: bool) {
    let hl = cpu.regs.hl();
    let v = cpu.read_mem(mem, hl);
    cpu.add_t(5);
    let (_, mut f) = sub8(cpu.regs.a, v);
    f = (f & !(flag::C | flag::PV | flag::X | flag::Y)) | (cpu.regs.f & flag::C);
    if inc {
        cpu.regs.set_hl(hl.wrapping_add(1));
    } else {
        cpu.regs.set_hl(hl.wrapping_sub(1));
    }
    let bc = cpu.regs.bc().wrapping_sub(1);
    cpu.regs.set_bc(bc);
    if bc != 0 {
        f |= flag::PV;
    }
    let mut n = cpu.regs.a.wrapping_sub(v);
    if f & flag::H != 0 {
        n = n.wrapping_sub(1);
    }
    f |= n & flag::X;
    if n & 0x02 != 0 {
        f |= flag::Y;
    }
    cpu.regs.f = f;
    cpu.regs.q = f;
    cpu.regs.memptr = cpu.regs.memptr.wrapping_add(if inc { 1 } else { u16::MAX });
    if repeat && bc != 0 && f & flag::Z == 0 {
        cpu.add_t(5);
        cpu.regs.pc = cpu.regs.pc.wrapping_sub(2);
        cpu.regs.memptr = cpu.regs.pc.wrapping_add(1);
    }
}

fn block_in<M: Memory, I: Io>(cpu: &mut Cpu, mem: &mut M, io: &mut I, inc: bool, repeat: bool) {
    cpu.add_t(1);
    let bc = cpu.regs.bc();
    let v = cpu.in_port(io, bc);
    let hl = cpu.regs.hl();
    cpu.write_mem(mem, hl, v);
    cpu.regs.b = cpu.regs.b.wrapping_sub(1);
    let b = cpu.regs.b;
    if inc {
        cpu.regs.set_hl(hl.wrapping_add(1));
        cpu.regs.memptr = bc.wrapping_add(1);
    } else {
        cpu.regs.set_hl(hl.wrapping_sub(1));
        cpu.regs.memptr = bc.wrapping_sub(1);
    }
    // INI: C+1; IND: C-1 (C from port address before B--).
    let c_side = if inc {
        (bc as u8).wrapping_add(1)
    } else {
        (bc as u8).wrapping_sub(1)
    };
    let k = u16::from(v) + u16::from(c_side);
    let mut f = sz53(b);
    if k > 0xff {
        f |= flag::C | flag::H;
    }
    if parity((k as u8 & 7) ^ b) {
        f |= flag::PV;
    }
    if v & 0x80 != 0 {
        f |= flag::N;
    }
    cpu.regs.f = f;
    cpu.regs.q = f;
    if repeat && b != 0 {
        cpu.add_t(5);
        cpu.regs.pc = cpu.regs.pc.wrapping_sub(2);
    }
}

fn block_out<M: Memory, I: Io>(cpu: &mut Cpu, mem: &mut M, io: &mut I, inc: bool, repeat: bool) {
    cpu.add_t(1);
    let hl = cpu.regs.hl();
    let v = cpu.read_mem(mem, hl);
    cpu.regs.b = cpu.regs.b.wrapping_sub(1);
    let b = cpu.regs.b;
    let bc = cpu.regs.bc();
    cpu.out_port(io, bc, v);
    let hl2 = if inc {
        let a = hl.wrapping_add(1);
        cpu.regs.set_hl(a);
        cpu.regs.memptr = bc.wrapping_add(1);
        a
    } else {
        let a = hl.wrapping_sub(1);
        cpu.regs.set_hl(a);
        cpu.regs.memptr = bc.wrapping_sub(1);
        a
    };
    // OUTI/OUTD: k = value + L after HL update.
    let k = u16::from(v) + u16::from(hl2 as u8);
    let mut f = sz53(b);
    if k > 0xff {
        f |= flag::C | flag::H;
    }
    if parity((k as u8 & 7) ^ b) {
        f |= flag::PV;
    }
    if v & 0x80 != 0 {
        f |= flag::N;
    }
    cpu.regs.f = f;
    cpu.regs.q = f;
    if repeat && b != 0 {
        cpu.add_t(5);
        cpu.regs.pc = cpu.regs.pc.wrapping_sub(2);
    }
}
