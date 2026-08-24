//! Spec Chum formats — SNA/Z80 snapshots, RZX, DSK.

#![allow(clippy::pedantic)]

mod dsk;
mod rzx;

pub use dsk::{DskImage, Plus3Fdc, Sector};
pub use rzx::{apply_input_byte, RzxFrame, RzxRecording};

use std::path::Path;

#[derive(Debug)]
pub enum FormatError {
    Io(std::io::Error),
    Format(String),
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Format(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for FormatError {}

/// Loaded 48K machine state from a snapshot.
#[derive(Clone, Debug)]
pub struct Snapshot48 {
    pub i: u8,
    pub hl_: u16,
    pub de_: u16,
    pub bc_: u16,
    pub af_: u16,
    pub hl: u16,
    pub de: u16,
    pub bc: u16,
    pub iy: u16,
    pub ix: u16,
    pub iff2: bool,
    pub r: u8,
    pub af: u16,
    pub sp: u16,
    pub im: u8,
    pub border: u8,
    pub pc: u16,
    pub ram: [u8; 49152],
}

impl Snapshot48 {
    pub fn load_sna(path: &Path) -> Result<Self, FormatError> {
        let data = std::fs::read(path).map_err(FormatError::Io)?;
        Self::parse_sna(&data)
    }

    pub fn parse_sna(data: &[u8]) -> Result<Self, FormatError> {
        if data.len() != 49179 {
            return Err(FormatError::Format(format!(
                "SNA48 expected 49179 bytes, got {}",
                data.len()
            )));
        }
        let mut s = Self {
            i: data[0],
            hl_: u16::from_le_bytes([data[1], data[2]]),
            de_: u16::from_le_bytes([data[3], data[4]]),
            bc_: u16::from_le_bytes([data[5], data[6]]),
            af_: u16::from(data[7]) << 8 | u16::from(data[8]),
            hl: u16::from_le_bytes([data[9], data[10]]),
            de: u16::from_le_bytes([data[11], data[12]]),
            bc: u16::from_le_bytes([data[13], data[14]]),
            iy: u16::from_le_bytes([data[15], data[16]]),
            ix: u16::from_le_bytes([data[17], data[18]]),
            iff2: data[19] & 4 != 0,
            r: data[20],
            af: u16::from(data[21]) << 8 | u16::from(data[22]),
            sp: u16::from_le_bytes([data[23], data[24]]),
            im: data[25],
            border: data[26] & 7,
            pc: 0,
            ram: [0; 49152],
        };
        s.ram.copy_from_slice(&data[27..49179]);
        let sp = s.sp as usize - 0x4000;
        if sp + 1 < s.ram.len() {
            s.pc = u16::from(s.ram[sp]) | (u16::from(s.ram[sp + 1]) << 8);
            s.sp = s.sp.wrapping_add(2);
        }
        Ok(s)
    }

    pub fn load_z80(path: &Path) -> Result<Self, FormatError> {
        let data = std::fs::read(path).map_err(FormatError::Io)?;
        Self::parse_z80(&data)
    }

    pub fn parse_z80(data: &[u8]) -> Result<Self, FormatError> {
        if data.len() < 30 {
            return Err(FormatError::Format("Z80 too short".into()));
        }
        let mut pc = u16::from_le_bytes([data[6], data[7]]);
        let mut header_len = 30usize;
        if pc == 0 {
            if data.len() < 34 {
                return Err(FormatError::Format("Z80 v2 header truncated".into()));
            }
            let extra = u16::from_le_bytes([data[30], data[31]]) as usize;
            header_len = 32 + extra;
            if data.len() < header_len {
                return Err(FormatError::Format("Z80 v2 truncated".into()));
            }
            pc = u16::from_le_bytes([data[32], data[33]]);
        }
        let compressed = data[12] & 0x20 != 0;
        let mut ram = [0u8; 49152];
        let src = &data[header_len..];
        if header_len == 30 {
            if compressed {
                decode_z80_v1(src, &mut ram)?;
            } else {
                if src.len() < 49152 {
                    return Err(FormatError::Format("Z80 v1 RAM short".into()));
                }
                ram.copy_from_slice(&src[..49152]);
            }
        } else {
            load_z80_v2_pages(src, &mut ram)?;
        }
        Ok(Self {
            i: data[10],
            af: u16::from(data[0]) << 8 | u16::from(data[1]),
            bc: u16::from_le_bytes([data[2], data[3]]),
            hl: u16::from_le_bytes([data[4], data[5]]),
            sp: u16::from_le_bytes([data[8], data[9]]),
            r: data[11],
            de: u16::from_le_bytes([data[13], data[14]]),
            bc_: u16::from_le_bytes([data[15], data[16]]),
            de_: u16::from_le_bytes([data[17], data[18]]),
            hl_: u16::from_le_bytes([data[19], data[20]]),
            af_: u16::from(data[21]) << 8 | u16::from(data[22]),
            iy: u16::from_le_bytes([data[23], data[24]]),
            ix: u16::from_le_bytes([data[25], data[26]]),
            iff2: data[27] != 0,
            im: data[29] & 3,
            border: (data[12] >> 1) & 7,
            pc,
            ram,
        })
    }
}

fn decode_z80_v1(src: &[u8], ram: &mut [u8; 49152]) -> Result<(), FormatError> {
    let mut i = 0usize;
    let mut o = 0usize;
    while i < src.len() && o < 49152 {
        if i + 3 < src.len()
            && src[i] == 0x00
            && src[i + 1] == 0xed
            && src[i + 2] == 0xed
            && src[i + 3] == 0x00
        {
            break;
        }
        if i + 3 < src.len() && src[i] == 0xed && src[i + 1] == 0xed {
            let n = src[i + 2] as usize;
            let v = src[i + 3];
            for _ in 0..n {
                if o >= 49152 {
                    break;
                }
                ram[o] = v;
                o += 1;
            }
            i += 4;
        } else {
            ram[o] = src[i];
            o += 1;
            i += 1;
        }
    }
    Ok(())
}

fn load_z80_v2_pages(mut src: &[u8], ram: &mut [u8; 49152]) -> Result<(), FormatError> {
    while src.len() >= 3 {
        let block_len = u16::from_le_bytes([src[0], src[1]]) as usize;
        let page = src[2];
        src = &src[3..];
        let data = if block_len == 0xffff {
            if src.len() < 16384 {
                return Err(FormatError::Format("page short".into()));
            }
            let d = src[..16384].to_vec();
            src = &src[16384..];
            d
        } else {
            if src.len() < block_len {
                return Err(FormatError::Format("compressed page short".into()));
            }
            let mut page_ram = vec![0u8; 16384];
            decode_z80_page(&src[..block_len], &mut page_ram)?;
            src = &src[block_len..];
            page_ram
        };
        // 48K .z80 pages: 8→4000-7FFF, 4→8000-BFFF, 5→C000-FFFF.
        // `ram[]` is the 48K block starting at Spectrum 0x4000.
        let page_map = match page {
            8 => 0usize,
            4 => 0x4000,
            5 => 0x8000,
            _ => continue,
        };
        ram[page_map..page_map + 16384].copy_from_slice(&data);
    }
    Ok(())
}

fn decode_z80_page(src: &[u8], dest: &mut [u8]) -> Result<(), FormatError> {
    let mut i = 0;
    let mut o = 0;
    while i < src.len() && o < dest.len() {
        if i + 3 < src.len() && src[i] == 0xed && src[i + 1] == 0xed {
            let n = src[i + 2] as usize;
            let v = src[i + 3];
            for _ in 0..n {
                if o >= dest.len() {
                    break;
                }
                dest[o] = v;
                o += 1;
            }
            i += 4;
        } else {
            dest[o] = src[i];
            o += 1;
            i += 1;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sna_size_check() {
        let err = Snapshot48::parse_sna(&[0u8; 10]).unwrap_err();
        assert!(matches!(err, FormatError::Format(_)));
    }

    #[test]
    fn sna_round_regs() {
        let mut data = vec![0u8; 49179];
        data[21] = 0x12;
        data[22] = 0x34;
        data[23] = 0x00;
        data[24] = 0x40;
        data[27] = 0x56;
        data[28] = 0x78;
        let s = Snapshot48::parse_sna(&data).unwrap();
        assert_eq!(s.af, 0x1234);
        assert_eq!(s.pc, 0x7856);
    }

    /// Minimal uncompressed Z80 v1 (30-byte header + 48K RAM).
    fn synthetic_z80_v1_uncompressed() -> Vec<u8> {
        let mut data = vec![0u8; 30 + 49152];
        data[0] = 0xab; // A
        data[1] = 0xcd; // F
        data[2] = 0x34; // C
        data[3] = 0x12; // B → BC = 0x1234
        data[4] = 0x78; // L
        data[5] = 0x56; // H → HL = 0x5678
        data[6] = 0x00; // PC lo
        data[7] = 0x80; // PC hi → 0x8000
        data[8] = 0xfe; // SP lo
        data[9] = 0xff; // SP hi → 0xfffe
        data[10] = 0x3f; // I
        data[11] = 0x11; // R
        data[12] = (5 << 1) & 0x0e; // border 5, uncompressed
        data[13] = 0xbc; // E
        data[14] = 0x9a; // D → DE = 0x9abc
        data[27] = 1; // IFF1/IFF2
        data[29] = 1; // IM 1
        data[30] = 0xaa;
        data[31] = 0x55;
        data
    }

    /// Compressed Z80 v1: literals + ED ED runs, ends with 00 ED ED 00.
    fn synthetic_z80_v1_compressed() -> Vec<u8> {
        let mut data = vec![0u8; 30];
        data[0] = 0x10;
        data[1] = 0x20;
        data[6] = 0x34;
        data[7] = 0x12; // PC = 0x1234
        data[8] = 0x00;
        data[9] = 0x60; // SP = 0x6000
        data[12] = 0x20 | (3 << 1); // compressed + border 3
        let mut ram_enc = Vec::new();
        ram_enc.push(0xaa);
        ram_enc.push(0x55);
        let mut remaining = 49152 - 2;
        while remaining > 0 {
            let n = remaining.min(255);
            ram_enc.extend_from_slice(&[0xed, 0xed, n as u8, 0x00]);
            remaining -= n;
        }
        ram_enc.extend_from_slice(&[0x00, 0xed, 0xed, 0x00]);
        data.extend_from_slice(&ram_enc);
        data
    }

    fn write_z80_v2_page(out: &mut Vec<u8>, page: u8, first: u8, second: u8) {
        out.extend_from_slice(&0xffffu16.to_le_bytes());
        out.push(page);
        let mut page_ram = vec![0u8; 16384];
        page_ram[0] = first;
        page_ram[1] = second;
        out.extend_from_slice(&page_ram);
    }

    /// Minimal Z80 v2 (extra header length 23) with pages 8/4/5.
    fn synthetic_z80_v2_pages() -> Vec<u8> {
        let mut data = vec![0u8; 55];
        data[0] = 0xde; // A
        data[1] = 0xad; // F
        data[6] = 0; // PC == 0 → v2+
        data[7] = 0;
        data[8] = 0x00;
        data[9] = 0x50; // SP = 0x5000
        data[12] = (2 << 1) & 0x0e; // border 2
        data[30] = 23; // extra header length (v2)
        data[31] = 0;
        data[32] = 0x00; // real PC lo
        data[33] = 0x90; // real PC hi → 0x9000
        data[34] = 0; // 48K hardware mode
        write_z80_v2_page(&mut data, 8, 0x11, 0x22);
        write_z80_v2_page(&mut data, 4, 0x33, 0x44);
        write_z80_v2_page(&mut data, 5, 0x55, 0x66);
        data
    }

    #[test]
    fn parse_z80_v1_uncompressed_regs_and_ram() {
        let s = Snapshot48::parse_z80(&synthetic_z80_v1_uncompressed()).unwrap();
        assert_eq!(s.af, 0xabcd);
        assert_eq!(s.bc, 0x1234);
        assert_eq!(s.hl, 0x5678);
        assert_eq!(s.de, 0x9abc);
        assert_eq!(s.pc, 0x8000);
        assert_eq!(s.sp, 0xfffe);
        assert_eq!(s.i, 0x3f);
        assert_eq!(s.r, 0x11);
        assert_eq!(s.border, 5);
        assert_eq!(s.im, 1);
        assert!(s.iff2);
        assert_eq!(s.ram[0], 0xaa);
        assert_eq!(s.ram[1], 0x55);
    }

    #[test]
    fn parse_z80_v1_compressed_regs_and_ram() {
        let s = Snapshot48::parse_z80(&synthetic_z80_v1_compressed()).unwrap();
        assert_eq!(s.af, 0x1020);
        assert_eq!(s.pc, 0x1234);
        assert_eq!(s.sp, 0x6000);
        assert_eq!(s.border, 3);
        assert_eq!(s.ram[0], 0xaa);
        assert_eq!(s.ram[1], 0x55);
        assert_eq!(s.ram[2], 0x00);
    }

    #[test]
    fn parse_z80_v2_pages_land_at_48k_addresses() {
        let s = Snapshot48::parse_z80(&synthetic_z80_v2_pages()).unwrap();
        assert_eq!(s.af, 0xdead);
        assert_eq!(s.pc, 0x9000);
        assert_eq!(s.sp, 0x5000);
        assert_eq!(s.border, 2);
        // page 8 → Spectrum 0x4000 → ram[0]
        assert_eq!(s.ram[0], 0x11);
        assert_eq!(s.ram[1], 0x22);
        // page 4 → Spectrum 0x8000 → ram[0x4000]
        assert_eq!(s.ram[0x4000], 0x33);
        assert_eq!(s.ram[0x4001], 0x44);
        // page 5 → Spectrum 0xC000 → ram[0x8000]
        assert_eq!(s.ram[0x8000], 0x55);
        assert_eq!(s.ram[0x8001], 0x66);
    }
}
