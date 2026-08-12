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
        let page_map = match page {
            5 => 0usize,
            8 => 0x4000,
            4 => 0x8000,
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
}
