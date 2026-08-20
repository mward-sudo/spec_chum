//! Z80 disassembler (mnemonics + instruction length). No bus/T-state side effects.

/// One decoded instruction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Disasm {
    pub text: String,
    pub len: u8,
}

#[must_use]
pub fn disasm_one(bytes: &[u8]) -> Disasm {
    if bytes.is_empty() {
        return trunc(1);
    }
    match bytes[0] {
        0x00 => one("NOP"),
        0x01 => ld16(bytes, "BC"),
        0x02 => one("LD (BC),A"),
        0x03 => one("INC BC"),
        0x04 => one("INC B"),
        0x05 => one("DEC B"),
        0x06 => ld8(bytes, "B"),
        0x07 => one("RLCA"),
        0x08 => one("EX AF,AF'"),
        0x09 => one("ADD HL,BC"),
        0x0a => one("LD A,(BC)"),
        0x0b => one("DEC BC"),
        0x0c => one("INC C"),
        0x0d => one("DEC C"),
        0x0e => ld8(bytes, "C"),
        0x0f => one("RRCA"),
        0x10 => jr(bytes, "DJNZ"),
        0x11 => ld16(bytes, "DE"),
        0x12 => one("LD (DE),A"),
        0x13 => one("INC DE"),
        0x14 => one("INC D"),
        0x15 => one("DEC D"),
        0x16 => ld8(bytes, "D"),
        0x17 => one("RLA"),
        0x18 => jr(bytes, "JR"),
        0x19 => one("ADD HL,DE"),
        0x1a => one("LD A,(DE)"),
        0x1b => one("DEC DE"),
        0x1c => one("INC E"),
        0x1d => one("DEC E"),
        0x1e => ld8(bytes, "E"),
        0x1f => one("RRA"),
        0x20 => jr(bytes, "JR NZ"),
        0x21 => ld16(bytes, "HL"),
        0x22 => abs_mem(bytes, "LD (${:04X}),HL"),
        0x23 => one("INC HL"),
        0x24 => one("INC H"),
        0x25 => one("DEC H"),
        0x26 => ld8(bytes, "H"),
        0x27 => one("DAA"),
        0x28 => jr(bytes, "JR Z"),
        0x29 => one("ADD HL,HL"),
        0x2a => abs_mem(bytes, "LD HL,(${:04X})"),
        0x2b => one("DEC HL"),
        0x2c => one("INC L"),
        0x2d => one("DEC L"),
        0x2e => ld8(bytes, "L"),
        0x2f => one("CPL"),
        0x30 => jr(bytes, "JR NC"),
        0x31 => ld16(bytes, "SP"),
        0x32 => abs_mem(bytes, "LD (${:04X}),A"),
        0x33 => one("INC SP"),
        0x34 => one("INC (HL)"),
        0x35 => one("DEC (HL)"),
        0x36 => ld8(bytes, "(HL)"),
        0x37 => one("SCF"),
        0x38 => jr(bytes, "JR C"),
        0x39 => one("ADD HL,SP"),
        0x3a => abs_mem(bytes, "LD A,(${:04X})"),
        0x3b => one("DEC SP"),
        0x3c => one("INC A"),
        0x3d => one("DEC A"),
        0x3e => ld8(bytes, "A"),
        0x3f => one("CCF"),
        op @ 0x40..=0x75 | op @ 0x77..=0x7f => one(&format!("LD {},{}", r8(op >> 3), r8(op))),
        0x76 => one("HALT"),
        op @ 0x80..=0xbf => {
            let alu = [
                "ADD A,", "ADC A,", "SUB ", "SBC A,", "AND ", "XOR ", "OR ", "CP ",
            ];
            one(&format!("{}{}", alu[usize::from((op >> 3) & 7)], r8(op)))
        }
        0xc0 => one("RET NZ"),
        0xc1 => one("POP BC"),
        0xc2 => jp(bytes, "JP NZ"),
        0xc3 => jp(bytes, "JP"),
        0xc4 => jp(bytes, "CALL NZ"),
        0xc5 => one("PUSH BC"),
        0xc6 => alu_imm(bytes, "ADD A,"),
        0xc7 => one("RST $00"),
        0xc8 => one("RET Z"),
        0xc9 => one("RET"),
        0xca => jp(bytes, "JP Z"),
        0xcb => disasm_cb(bytes),
        0xcc => jp(bytes, "CALL Z"),
        0xcd => jp(bytes, "CALL"),
        0xce => alu_imm(bytes, "ADC A,"),
        0xcf => one("RST $08"),
        0xd0 => one("RET NC"),
        0xd1 => one("POP DE"),
        0xd2 => jp(bytes, "JP NC"),
        0xd3 => port8(bytes, "OUT (${:02X}),A"),
        0xd4 => jp(bytes, "CALL NC"),
        0xd5 => one("PUSH DE"),
        0xd6 => alu_imm(bytes, "SUB "),
        0xd7 => one("RST $10"),
        0xd8 => one("RET C"),
        0xd9 => one("EXX"),
        0xda => jp(bytes, "JP C"),
        0xdb => port8(bytes, "IN A,(${:02X})"),
        0xdc => jp(bytes, "CALL C"),
        0xdd => disasm_index(bytes, "IX"),
        0xde => alu_imm(bytes, "SBC A,"),
        0xdf => one("RST $18"),
        0xe0 => one("RET PO"),
        0xe1 => one("POP HL"),
        0xe2 => jp(bytes, "JP PO"),
        0xe3 => one("EX (SP),HL"),
        0xe4 => jp(bytes, "CALL PO"),
        0xe5 => one("PUSH HL"),
        0xe6 => alu_imm(bytes, "AND "),
        0xe7 => one("RST $20"),
        0xe8 => one("RET PE"),
        0xe9 => one("JP (HL)"),
        0xea => jp(bytes, "JP PE"),
        0xeb => one("EX DE,HL"),
        0xec => jp(bytes, "CALL PE"),
        0xed => disasm_ed(bytes),
        0xee => alu_imm(bytes, "XOR "),
        0xef => one("RST $28"),
        0xf0 => one("RET P"),
        0xf1 => one("POP AF"),
        0xf2 => jp(bytes, "JP P"),
        0xf3 => one("DI"),
        0xf4 => jp(bytes, "CALL P"),
        0xf5 => one("PUSH AF"),
        0xf6 => alu_imm(bytes, "OR "),
        0xf7 => one("RST $30"),
        0xf8 => one("RET M"),
        0xf9 => one("LD SP,HL"),
        0xfa => jp(bytes, "JP M"),
        0xfb => one("EI"),
        0xfc => jp(bytes, "CALL M"),
        0xfd => disasm_index(bytes, "IY"),
        0xfe => alu_imm(bytes, "CP "),
        0xff => one("RST $38"),
    }
}

fn trunc(len: u8) -> Disasm {
    Disasm {
        text: "???".into(),
        len,
    }
}

fn one(text: &str) -> Disasm {
    Disasm {
        text: text.into(),
        len: 1,
    }
}

fn need(bytes: &[u8], n: usize) -> bool {
    bytes.len() >= n
}

fn ld8(bytes: &[u8], dest: &str) -> Disasm {
    if !need(bytes, 2) {
        return trunc(2);
    }
    Disasm {
        text: format!("LD {dest},${:02X}", bytes[1]),
        len: 2,
    }
}

fn ld16(bytes: &[u8], dest: &str) -> Disasm {
    if !need(bytes, 3) {
        return trunc(3);
    }
    let nn = u16::from(bytes[1]) | (u16::from(bytes[2]) << 8);
    Disasm {
        text: format!("LD {dest},${nn:04X}"),
        len: 3,
    }
}

fn signed8(d: u8) -> String {
    let v = d as i8;
    if v < 0 {
        format!("-${:02X}", v.unsigned_abs())
    } else {
        format!("+${v:02X}")
    }
}

fn jr(bytes: &[u8], mnem: &str) -> Disasm {
    if !need(bytes, 2) {
        return trunc(2);
    }
    Disasm {
        text: format!("{mnem} {}", signed8(bytes[1])),
        len: 2,
    }
}

fn jp(bytes: &[u8], mnem: &str) -> Disasm {
    if !need(bytes, 3) {
        return trunc(3);
    }
    let nn = u16::from(bytes[1]) | (u16::from(bytes[2]) << 8);
    Disasm {
        text: format!("{mnem} ${nn:04X}"),
        len: 3,
    }
}

fn abs_mem(bytes: &[u8], kind: &str) -> Disasm {
    if !need(bytes, 3) {
        return trunc(3);
    }
    let nn = u16::from(bytes[1]) | (u16::from(bytes[2]) << 8);
    let text = match kind {
        "LD (${:04X}),HL" => format!("LD (${nn:04X}),HL"),
        "LD HL,(${:04X})" => format!("LD HL,(${nn:04X})"),
        "LD (${:04X}),A" => format!("LD (${nn:04X}),A"),
        "LD A,(${:04X})" => format!("LD A,(${nn:04X})"),
        _ => format!("LD (${nn:04X})"),
    };
    Disasm { text, len: 3 }
}

fn port8(bytes: &[u8], kind: &str) -> Disasm {
    if !need(bytes, 2) {
        return trunc(2);
    }
    let n = bytes[1];
    let text = if kind.starts_with("OUT") {
        format!("OUT (${n:02X}),A")
    } else {
        format!("IN A,(${n:02X})")
    };
    Disasm { text, len: 2 }
}

fn alu_imm(bytes: &[u8], mnem: &str) -> Disasm {
    if !need(bytes, 2) {
        return trunc(2);
    }
    Disasm {
        text: format!("{mnem}${:02X}", bytes[1]),
        len: 2,
    }
}

fn r8(op: u8) -> &'static str {
    match op & 7 {
        0 => "B",
        1 => "C",
        2 => "D",
        3 => "E",
        4 => "H",
        5 => "L",
        6 => "(HL)",
        _ => "A",
    }
}

fn rot_name(y: u8) -> &'static str {
    match y {
        0 => "RLC",
        1 => "RRC",
        2 => "RL",
        3 => "RR",
        4 => "SLA",
        5 => "SRA",
        6 => "SLL",
        _ => "SRL",
    }
}

fn disasm_cb(bytes: &[u8]) -> Disasm {
    if !need(bytes, 2) {
        return trunc(2);
    }
    let op = bytes[1];
    let y = (op >> 3) & 7;
    let z = r8(op);
    let text = match op >> 6 {
        0 => format!("{} {z}", rot_name(y)),
        1 => format!("BIT {y},{z}"),
        2 => format!("RES {y},{z}"),
        _ => format!("SET {y},{z}"),
    };
    Disasm { text, len: 2 }
}

fn disasm_ed(bytes: &[u8]) -> Disasm {
    if !need(bytes, 2) {
        return trunc(2);
    }
    let op = bytes[1];
    let text_len: (&str, u8) = match op {
        0x40 => ("IN B,(C)", 2),
        0x41 => ("OUT (C),B", 2),
        0x42 => ("SBC HL,BC", 2),
        0x43 => return abs_ed(bytes, EdAbs::MemBc),
        0x44 | 0x4c | 0x54 | 0x5c | 0x64 | 0x6c | 0x74 | 0x7c => ("NEG", 2),
        0x45 | 0x55 | 0x65 | 0x75 => ("RETN", 2),
        0x46 | 0x4e | 0x66 | 0x6e => ("IM 0", 2),
        0x47 => ("LD I,A", 2),
        0x48 => ("IN C,(C)", 2),
        0x49 => ("OUT (C),C", 2),
        0x4a => ("ADC HL,BC", 2),
        0x4b => return abs_ed(bytes, EdAbs::BcMem),
        0x4d | 0x5d | 0x6d | 0x7d => ("RETI", 2),
        0x4f => ("LD R,A", 2),
        0x50 => ("IN D,(C)", 2),
        0x51 => ("OUT (C),D", 2),
        0x52 => ("SBC HL,DE", 2),
        0x53 => return abs_ed(bytes, EdAbs::MemDe),
        0x56 | 0x76 => ("IM 1", 2),
        0x57 => ("LD A,I", 2),
        0x58 => ("IN E,(C)", 2),
        0x59 => ("OUT (C),E", 2),
        0x5a => ("ADC HL,DE", 2),
        0x5b => return abs_ed(bytes, EdAbs::DeMem),
        0x5e | 0x7e => ("IM 2", 2),
        0x5f => ("LD A,R", 2),
        0x60 => ("IN H,(C)", 2),
        0x61 => ("OUT (C),H", 2),
        0x62 => ("SBC HL,HL", 2),
        0x63 => return abs_ed(bytes, EdAbs::MemHl),
        0x67 => ("RRD", 2),
        0x68 => ("IN L,(C)", 2),
        0x69 => ("OUT (C),L", 2),
        0x6a => ("ADC HL,HL", 2),
        0x6b => return abs_ed(bytes, EdAbs::HlMem),
        0x6f => ("RLD", 2),
        0x70 => ("IN (C)", 2),
        0x71 => ("OUT (C),0", 2),
        0x72 => ("SBC HL,SP", 2),
        0x73 => return abs_ed(bytes, EdAbs::MemSp),
        0x78 => ("IN A,(C)", 2),
        0x79 => ("OUT (C),A", 2),
        0x7a => ("ADC HL,SP", 2),
        0x7b => return abs_ed(bytes, EdAbs::SpMem),
        0xa0 => ("LDI", 2),
        0xa1 => ("CPI", 2),
        0xa2 => ("INI", 2),
        0xa3 => ("OUTI", 2),
        0xa8 => ("LDD", 2),
        0xa9 => ("CPD", 2),
        0xaa => ("IND", 2),
        0xab => ("OUTD", 2),
        0xb0 => ("LDIR", 2),
        0xb1 => ("CPIR", 2),
        0xb2 => ("INIR", 2),
        0xb3 => ("OTIR", 2),
        0xb8 => ("LDDR", 2),
        0xb9 => ("CPDR", 2),
        0xba => ("INDR", 2),
        0xbb => ("OTDR", 2),
        _ => ("NOP*", 2),
    };
    Disasm {
        text: text_len.0.into(),
        len: text_len.1,
    }
}

enum EdAbs {
    MemBc,
    BcMem,
    MemDe,
    DeMem,
    MemHl,
    HlMem,
    MemSp,
    SpMem,
}

fn abs_ed(bytes: &[u8], kind: EdAbs) -> Disasm {
    if !need(bytes, 4) {
        return trunc(4);
    }
    let nn = u16::from(bytes[2]) | (u16::from(bytes[3]) << 8);
    let text = match kind {
        EdAbs::MemBc => format!("LD (${nn:04X}),BC"),
        EdAbs::BcMem => format!("LD BC,(${nn:04X})"),
        EdAbs::MemDe => format!("LD (${nn:04X}),DE"),
        EdAbs::DeMem => format!("LD DE,(${nn:04X})"),
        EdAbs::MemHl => format!("LD (${nn:04X}),HL"),
        EdAbs::HlMem => format!("LD HL,(${nn:04X})"),
        EdAbs::MemSp => format!("LD (${nn:04X}),SP"),
        EdAbs::SpMem => format!("LD SP,(${nn:04X})"),
    };
    Disasm { text, len: 4 }
}

fn disasm_index(bytes: &[u8], xy: &str) -> Disasm {
    if !need(bytes, 2) {
        return trunc(2);
    }
    let op = bytes[1];
    if op == 0xcb {
        return disasm_ddcb(bytes, xy);
    }
    // Nested prefixes: consume this prefix as a no-op byte.
    if op == 0xdd || op == 0xfd || op == 0xed {
        return one("NOP*");
    }
    let hl = xy;
    let h = if xy == "IX" { "IXH" } else { "IYH" };
    let l = if xy == "IX" { "IXL" } else { "IYL" };
    let disp = |b: &[u8]| -> Option<(i8, u8)> {
        if b.len() < 3 {
            None
        } else {
            Some((b[2] as i8, b[2]))
        }
    };
    let idx_mem = |d: u8| format!("({xy}{})", signed8(d));
    match op {
        0x09 => two(&format!("ADD {hl},BC")),
        0x19 => two(&format!("ADD {hl},DE")),
        0x21 => {
            if !need(bytes, 4) {
                return trunc(4);
            }
            let nn = u16::from(bytes[2]) | (u16::from(bytes[3]) << 8);
            Disasm {
                text: format!("LD {hl},${nn:04X}"),
                len: 4,
            }
        }
        0x22 => {
            if !need(bytes, 4) {
                return trunc(4);
            }
            let nn = u16::from(bytes[2]) | (u16::from(bytes[3]) << 8);
            Disasm {
                text: format!("LD (${nn:04X}),{hl}"),
                len: 4,
            }
        }
        0x23 => two(&format!("INC {hl}")),
        0x24 => two(&format!("INC {h}")),
        0x25 => two(&format!("DEC {h}")),
        0x26 => {
            if !need(bytes, 3) {
                return trunc(3);
            }
            Disasm {
                text: format!("LD {h},${:02X}", bytes[2]),
                len: 3,
            }
        }
        0x29 => two(&format!("ADD {hl},{hl}")),
        0x2a => {
            if !need(bytes, 4) {
                return trunc(4);
            }
            let nn = u16::from(bytes[2]) | (u16::from(bytes[3]) << 8);
            Disasm {
                text: format!("LD {hl},(${nn:04X})"),
                len: 4,
            }
        }
        0x2b => two(&format!("DEC {hl}")),
        0x2c => two(&format!("INC {l}")),
        0x2d => two(&format!("DEC {l}")),
        0x2e => {
            if !need(bytes, 3) {
                return trunc(3);
            }
            Disasm {
                text: format!("LD {l},${:02X}", bytes[2]),
                len: 3,
            }
        }
        0x34 => {
            let Some((.., d)) = disp(bytes) else {
                return trunc(3);
            };
            Disasm {
                text: format!("INC {}", idx_mem(d)),
                len: 3,
            }
        }
        0x35 => {
            let Some((.., d)) = disp(bytes) else {
                return trunc(3);
            };
            Disasm {
                text: format!("DEC {}", idx_mem(d)),
                len: 3,
            }
        }
        0x36 => {
            if !need(bytes, 4) {
                return trunc(4);
            }
            Disasm {
                text: format!("LD {},${:02X}", idx_mem(bytes[2]), bytes[3]),
                len: 4,
            }
        }
        0x39 => two(&format!("ADD {hl},SP")),
        op @ 0x40..=0x7f if op != 0x76 => {
            let y = (op >> 3) & 7;
            let z = op & 7;
            let dst_hl = y == 6;
            let src_hl = z == 6;
            if dst_hl && src_hl {
                return two("HALT");
            }
            if dst_hl || src_hl {
                let Some((.., d)) = disp(bytes) else {
                    return trunc(3);
                };
                let mem = idx_mem(d);
                let other = if dst_hl {
                    r8_xy(z, h, l)
                } else {
                    r8_xy(y, h, l)
                };
                let text = if dst_hl {
                    format!("LD {mem},{other}")
                } else {
                    format!("LD {other},{mem}")
                };
                return Disasm { text, len: 3 };
            }
            two(&format!("LD {},{}", r8_xy(y, h, l), r8_xy(z, h, l)))
        }
        op @ 0x80..=0xbf => {
            let y = (op >> 3) & 7;
            let z = op & 7;
            let alu = [
                "ADD A,", "ADC A,", "SUB ", "SBC A,", "AND ", "XOR ", "OR ", "CP ",
            ];
            if z == 6 {
                let Some((.., d)) = disp(bytes) else {
                    return trunc(3);
                };
                return Disasm {
                    text: format!("{}{}", alu[usize::from(y)], idx_mem(d)),
                    len: 3,
                };
            }
            two(&format!("{}{}", alu[usize::from(y)], r8_xy(z, h, l)))
        }
        0xe1 => two(&format!("POP {hl}")),
        0xe3 => two(&format!("EX (SP),{hl}")),
        0xe5 => two(&format!("PUSH {hl}")),
        0xe9 => two(&format!("JP ({hl})")),
        0xf9 => two(&format!("LD SP,{hl}")),
        _ => {
            // Unprefixed meaning with a wasted prefix byte.
            let rest = disasm_one(&bytes[1..]);
            Disasm {
                text: rest.text,
                len: rest.len.saturating_add(1),
            }
        }
    }
}

fn two(text: &str) -> Disasm {
    Disasm {
        text: text.into(),
        len: 2,
    }
}

fn r8_xy<'a>(r: u8, h: &'a str, l: &'a str) -> &'a str {
    match r & 7 {
        0 => "B",
        1 => "C",
        2 => "D",
        3 => "E",
        4 => h,
        5 => l,
        6 => "(HL)",
        _ => "A",
    }
}

fn disasm_ddcb(bytes: &[u8], xy: &str) -> Disasm {
    if !need(bytes, 4) {
        return trunc(4);
    }
    let d = bytes[2];
    let op = bytes[3];
    let y = (op >> 3) & 7;
    let z = op & 7;
    let mem = format!("({xy}{})", signed8(d));
    let r = r8(z);
    let text = match op >> 6 {
        0 => {
            if z == 6 {
                format!("{} {mem}", rot_name(y))
            } else {
                format!("{} {mem},{r}", rot_name(y))
            }
        }
        1 => format!("BIT {y},{mem}"),
        2 => {
            if z == 6 {
                format!("RES {y},{mem}")
            } else {
                format!("RES {y},{mem},{r}")
            }
        }
        _ => {
            if z == 6 {
                format!("SET {y},{mem}")
            } else {
                format!("SET {y},{mem},{r}")
            }
        }
    };
    Disasm { text, len: 4 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(bytes: &[u8]) -> (String, u8) {
        let d = disasm_one(bytes);
        (d.text, d.len)
    }

    #[test]
    fn unprefixed_lengths() {
        assert_eq!(t(&[0x00]), ("NOP".into(), 1));
        assert_eq!(t(&[0x3e, 0x41]), ("LD A,$41".into(), 2));
        assert_eq!(t(&[0x21, 0x00, 0x40]), ("LD HL,$4000".into(), 3));
        assert_eq!(t(&[0x18, 0xfe]), ("JR -$02".into(), 2));
        assert_eq!(t(&[0x76]), ("HALT".into(), 1));
        assert_eq!(t(&[0xc3, 0x6c, 0x05]), ("JP $056C".into(), 3));
        assert_eq!(t(&[0x78]), ("LD A,B".into(), 1));
        assert_eq!(t(&[0xa7]), ("AND A".into(), 1));
    }

    #[test]
    fn cb_ed_index() {
        assert_eq!(t(&[0xcb, 0x47]), ("BIT 0,A".into(), 2));
        assert_eq!(t(&[0xed, 0xb0]), ("LDIR".into(), 2));
        assert_eq!(t(&[0xed, 0x43, 0x00, 0x40]), ("LD ($4000),BC".into(), 4));
        assert_eq!(t(&[0xdd, 0xe9]), ("JP (IX)".into(), 2));
        assert_eq!(t(&[0xdd, 0xcb, 0x02, 0x46]), ("BIT 0,(IX+$02)".into(), 4));
        assert_eq!(t(&[0xfd, 0x21, 0x3a, 0x5c]), ("LD IY,$5C3A".into(), 4));
        assert_eq!(t(&[0xdd, 0x36, 0x01, 0xaa]), ("LD (IX+$01),$AA".into(), 4));
    }
}
