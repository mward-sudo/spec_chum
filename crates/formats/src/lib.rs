//! Spec Chum formats — SNA/Z80 snapshots, RZX, DSK, TRD, Timex DCK.

mod dck;
mod dsk;
mod error;
mod fdc;
mod mdr;
mod rzx;
mod trd;

pub use dck::{DckBank, DckBankId, DckChunkAccess, DckImage, DCK_CHUNK_SIZE, DCK_HEADER_SIZE};
pub use dsk::{DskImage, Sector};
pub use error::FormatError;
pub use fdc::Plus3Fdc;
pub use mdr::{MdrImage, MDR_DATA_LEN, MDR_HEAD_LEN, MDR_IMAGE_SIZE, MDR_SECTORS, MDR_SECTOR_SIZE};
pub use rzx::{apply_input_byte, RzxFrame, RzxRecording};
pub use trd::{TrdImage, TRD_SECTORS_PER_TRACK, TRD_SECTOR_SIZE};

use std::path::Path;

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
        let hdr = parse_z80_header(data)?;
        if hdr.class != Z80MachineClass::Spectrum48 {
            return Err(FormatError::Format(
                "128K/+3 Z80 snapshot: use Snapshot128::parse_z80".into(),
            ));
        }
        let compressed = data[12] & 0x20 != 0;
        let mut ram = [0u8; 49152];
        let src = &data[hdr.header_len..];
        if hdr.header_len == 30 {
            if compressed {
                decode_z80_v1(src, &mut ram);
            } else {
                if src.len() < 49152 {
                    return Err(FormatError::Format("Z80 v1 RAM short".into()));
                }
                ram.copy_from_slice(&src[..49152]);
            }
        } else {
            load_z80_v2_pages_48(src, &mut ram)?;
        }
        Ok(regs_from_z80_header(data, hdr.pc, ram))
    }
}

/// Machine class implied by a Z80 v2/v3 hardware-mode byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Z80MachineClass {
    Spectrum48,
    Spectrum128,
    SpectrumPlus2A,
    SpectrumPlus3,
}

/// Target machine for a banked 128K-family snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Snapshot128Model {
    Spectrum128,
    SpectrumPlus2A,
    SpectrumPlus3,
}

struct Z80HeaderInfo {
    header_len: usize,
    pc: u16,
    class: Z80MachineClass,
    page_7ffd: u8,
    page_1ffd: Option<u8>,
}

fn parse_z80_header(data: &[u8]) -> Result<Z80HeaderInfo, FormatError> {
    if data.len() < 30 {
        return Err(FormatError::Format("Z80 too short".into()));
    }
    let mut pc = u16::from_le_bytes([data[6], data[7]]);
    if pc != 0 {
        // v1 — always 48K.
        return Ok(Z80HeaderInfo {
            header_len: 30,
            pc,
            class: Z80MachineClass::Spectrum48,
            page_7ffd: 0,
            page_1ffd: None,
        });
    }
    if data.len() < 34 {
        return Err(FormatError::Format("Z80 v2 header truncated".into()));
    }
    let extra = u16::from_le_bytes([data[30], data[31]]) as usize;
    // Legal v2 extended header is at least 23 bytes (total header_len >= 55), so
    // hw at [34] and optional page_7ffd at [35] are always in range.
    if extra < 23 {
        return Err(FormatError::Format(format!(
            "Z80 v2 extended header length {extra} too small"
        )));
    }
    let header_len = 32 + extra;
    if data.len() < header_len {
        return Err(FormatError::Format("Z80 v2 truncated".into()));
    }
    pc = u16::from_le_bytes([data[32], data[33]]);
    let hw = data[34];
    let misc = if header_len > 37 { data[37] } else { 0 };
    let class = z80_machine_class(extra, hw, misc);
    let page_7ffd = if class == Z80MachineClass::Spectrum48 {
        0
    } else {
        data[35]
    };
    // 1FFD for +2A/+3 (xzx hw 7/8). Ignore trailing byte on plain 128K
    // snapshots that happen to use a 55-byte extended header.
    let page_1ffd = match class {
        Z80MachineClass::SpectrumPlus2A | Z80MachineClass::SpectrumPlus3 => {
            if extra >= 55 && header_len > 86 {
                Some(data[86])
            } else {
                Some(0)
            }
        }
        _ => None,
    };
    Ok(Z80HeaderInfo {
        header_len,
        pc,
        class,
        page_7ffd,
        page_1ffd,
    })
}

fn z80_machine_class(extra: usize, hw: u8, misc: u8) -> Z80MachineClass {
    let modify_hardware = misc & 0x80 != 0;
    // xzx: hw 7 = +3, hw 8 = +2A. FAQ "modify hardware" (byte 37 bit 7) remaps
    // +3 identifiers (7/8) to +2A.
    if matches!(hw, 7 | 8) {
        return if modify_hardware || hw == 8 {
            Z80MachineClass::SpectrumPlus2A
        } else {
            Z80MachineClass::SpectrumPlus3
        };
    }
    let is_v2 = extra == 23;
    let is_128 = if is_v2 {
        matches!(hw, 3 | 4)
    } else {
        // v3 (extra 54/55): 4–6 = 128K family.
        matches!(hw, 4..=6)
    };
    // Bit 7 of byte 37 ("modify hardware") maps 128K → +2, still 7FFD-only.
    if is_128 {
        Z80MachineClass::Spectrum128
    } else {
        Z80MachineClass::Spectrum48
    }
}

fn regs_from_z80_header(data: &[u8], pc: u16, ram: [u8; 49152]) -> Snapshot48 {
    Snapshot48 {
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
    }
}

/// Loaded 128K / +2A/+3 machine state (eight 16K RAM banks + paging).
#[derive(Clone, Debug)]
pub struct Snapshot128 {
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
    /// Last OUT to port `0x7FFD`.
    pub page_7ffd: u8,
    /// Last OUT to port `0x1FFD` when present (+2A/+3 Z80).
    pub page_1ffd: Option<u8>,
    /// Target machine implied by the snapshot header (SNA128 is always 128K).
    pub model: Snapshot128Model,
    /// Physical RAM banks 0..7.
    pub banks: [[u8; 16384]; 8],
}

impl Snapshot128 {
    pub fn load_sna(path: &Path) -> Result<Self, FormatError> {
        let data = std::fs::read(path).map_err(FormatError::Io)?;
        Self::parse_sna(&data)
    }

    /// Parse a 128K `.sna` (131103 or 147487 bytes).
    pub fn parse_sna(data: &[u8]) -> Result<Self, FormatError> {
        const SNA128_5: usize = 131_103;
        const SNA128_6: usize = 147_487;
        if data.len() != SNA128_5 && data.len() != SNA128_6 {
            return Err(FormatError::Format(format!(
                "SNA128 expected {SNA128_5} or {SNA128_6} bytes, got {}",
                data.len()
            )));
        }
        let page_7ffd = data[49_181];
        let paged = usize::from(page_7ffd & 7);
        let mut banks = [[0u8; 16384]; 8];
        banks[5].copy_from_slice(&data[27..27 + 16384]);
        banks[2].copy_from_slice(&data[27 + 16384..27 + 32768]);
        banks[paged].copy_from_slice(&data[27 + 32768..27 + 49152]);

        let mut present = [false; 8];
        present[5] = true;
        present[2] = true;
        present[paged] = true;
        let remaining = present.iter().filter(|&&p| !p).count();
        let expect_len = 49_183 + remaining * 16384;
        if data.len() != expect_len {
            return Err(FormatError::Format(format!(
                "SNA128 size {0} inconsistent with paged bank {paged} (expected {expect_len})",
                data.len()
            )));
        }
        let mut off = 49_183;
        for b in 0..8 {
            if !present[b] {
                banks[b].copy_from_slice(&data[off..off + 16384]);
                off += 16384;
            }
        }

        let pc = u16::from_le_bytes([data[49_179], data[49_180]]);
        Ok(Self {
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
            pc,
            page_7ffd,
            page_1ffd: None,
            model: Snapshot128Model::Spectrum128,
            banks,
        })
    }

    pub fn load_z80(path: &Path) -> Result<Self, FormatError> {
        let data = std::fs::read(path).map_err(FormatError::Io)?;
        Self::parse_z80(&data)
    }

    pub fn parse_z80(data: &[u8]) -> Result<Self, FormatError> {
        let hdr = parse_z80_header(data)?;
        if hdr.class == Z80MachineClass::Spectrum48 {
            return Err(FormatError::Format(
                "48K Z80 snapshot: use Snapshot48::parse_z80".into(),
            ));
        }
        let mut banks = [[0u8; 16384]; 8];
        load_z80_v2_pages_128(&data[hdr.header_len..], &mut banks)?;
        let s48 = regs_from_z80_header(data, hdr.pc, [0; 49152]);
        Ok(Self {
            i: s48.i,
            hl_: s48.hl_,
            de_: s48.de_,
            bc_: s48.bc_,
            af_: s48.af_,
            hl: s48.hl,
            de: s48.de,
            bc: s48.bc,
            iy: s48.iy,
            ix: s48.ix,
            iff2: s48.iff2,
            r: s48.r,
            af: s48.af,
            sp: s48.sp,
            im: s48.im,
            border: s48.border,
            pc: s48.pc,
            page_7ffd: hdr.page_7ffd,
            page_1ffd: hdr.page_1ffd,
            model: match hdr.class {
                Z80MachineClass::SpectrumPlus2A => Snapshot128Model::SpectrumPlus2A,
                Z80MachineClass::SpectrumPlus3 => Snapshot128Model::SpectrumPlus3,
                Z80MachineClass::Spectrum128 => Snapshot128Model::Spectrum128,
                Z80MachineClass::Spectrum48 => unreachable!("rejected above"),
            },
            banks,
        })
    }

    #[must_use]
    pub fn is_plus3(&self) -> bool {
        self.model == Snapshot128Model::SpectrumPlus3
    }

    #[must_use]
    pub fn is_plus2a(&self) -> bool {
        self.model == Snapshot128Model::SpectrumPlus2A
    }
}

fn decode_z80_v1(src: &[u8], ram: &mut [u8; 49152]) {
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
}

fn take_z80_page_block(src: &mut &[u8]) -> Result<(u8, Vec<u8>), FormatError> {
    if src.len() < 3 {
        return Err(FormatError::Format("page header short".into()));
    }
    let block_len = u16::from_le_bytes([src[0], src[1]]) as usize;
    let page = src[2];
    *src = &src[3..];
    let data = if block_len == 0xffff {
        if src.len() < 16384 {
            return Err(FormatError::Format("page short".into()));
        }
        let d = src[..16384].to_vec();
        *src = &src[16384..];
        d
    } else {
        if src.len() < block_len {
            return Err(FormatError::Format("compressed page short".into()));
        }
        let mut page_ram = vec![0u8; 16384];
        decode_z80_page(&src[..block_len], &mut page_ram);
        *src = &src[block_len..];
        page_ram
    };
    Ok((page, data))
}

fn load_z80_v2_pages_48(mut src: &[u8], ram: &mut [u8; 49152]) -> Result<(), FormatError> {
    while src.len() >= 3 {
        let (page, data) = take_z80_page_block(&mut src)?;
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

fn load_z80_v2_pages_128(mut src: &[u8], banks: &mut [[u8; 16384]; 8]) -> Result<(), FormatError> {
    while src.len() >= 3 {
        let (page, data) = take_z80_page_block(&mut src)?;
        // 128K .z80: pages 3..10 → physical RAM banks 0..7.
        let bank = match page {
            3..=10 => usize::from(page - 3),
            _ => continue,
        };
        banks[bank].copy_from_slice(&data);
    }
    Ok(())
}

fn decode_z80_page(src: &[u8], dest: &mut [u8]) {
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
    fn parse_z80_rejects_undersized_extended_header() {
        let mut data = vec![0u8; 34];
        // PC=0 → v2 path; extra=0 would previously panic on data[34].
        data[30] = 0;
        data[31] = 0;
        let err = Snapshot48::parse_z80(&data).unwrap_err();
        assert!(err.to_string().contains("too small"), "got {err}");
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

    /// Minimal Z80 v2 128K (extra=23, hw=3) with pages 3..10 → banks 0..7.
    fn synthetic_z80_v2_128() -> Vec<u8> {
        let mut data = vec![0u8; 55];
        data[0] = 0xbe; // A
        data[1] = 0xef; // F
        data[6] = 0;
        data[7] = 0; // PC==0 → v2+
        data[8] = 0x00;
        data[9] = 0x60; // SP
        data[12] = (4 << 1) & 0x0e; // border 4
        data[30] = 23;
        data[31] = 0;
        data[32] = 0x34; // PC lo
        data[33] = 0x12; // PC = 0x1234
        data[34] = 3; // v2: 128K
        data[35] = 0x16; // 7FFD: bank 6, ROM1, screen 5
        for bank in 0u8..8 {
            write_z80_v2_page(&mut data, bank + 3, 0xa0 + bank, 0xb0 + bank);
        }
        data
    }

    /// Z80 v3 +3 (extra=55, hw=7) with 1FFD and eight banks.
    fn synthetic_z80_v3_plus3() -> Vec<u8> {
        let mut data = vec![0u8; 87];
        data[6] = 0;
        data[7] = 0;
        data[12] = (1 << 1) & 0x0e;
        data[30] = 55; // v3 + 1FFD byte
        data[31] = 0;
        data[32] = 0x00;
        data[33] = 0xc0; // PC = 0xc000
        data[34] = 7; // +3
        data[35] = 0x03; // 7FFD bank 3
        data[86] = 0x04; // 1FFD: ROM high bit
        for bank in 0u8..8 {
            write_z80_v2_page(&mut data, bank + 3, 0x10 + bank, 0x20 + bank);
        }
        data
    }

    fn synthetic_sna128(paged: u8) -> Vec<u8> {
        let paged = usize::from(paged & 7);
        let mut present = [false; 8];
        present[5] = true;
        present[2] = true;
        present[paged] = true;
        let remaining = present.iter().filter(|&&p| !p).count();
        let mut data = vec![0u8; 49_183 + remaining * 16384];
        data[21] = 0xaa;
        data[22] = 0x55; // AF
        data[23] = 0xfe;
        data[24] = 0xff; // SP
        data[26] = 2; // border
                      // Bank 5 / 2 / paged markers in the 48K dump.
        data[27] = 0x51; // bank 5[0]
        data[27 + 16384] = 0x21; // bank 2[0]
        data[27 + 32768] = 0xc0 | paged as u8; // paged bank[0]
        data[49_179] = 0x00;
        data[49_180] = 0x80; // PC = 0x8000 (not popped from stack)
        data[49_181] = paged as u8; // 7FFD
        data[49_182] = 0; // TR-DOS
        let mut off = 49_183;
        for (b, &is_present) in present.iter().enumerate() {
            if !is_present {
                data[off] = 0x40 | b as u8;
                off += 16384;
            }
        }
        data
    }

    #[test]
    fn parse_z80_v2_128_banks_and_7ffd() {
        let s = Snapshot128::parse_z80(&synthetic_z80_v2_128()).unwrap();
        assert_eq!(s.af, 0xbeef);
        assert_eq!(s.pc, 0x1234);
        assert_eq!(s.border, 4);
        assert_eq!(s.page_7ffd, 0x16);
        assert!(s.page_1ffd.is_none());
        for b in 0..8 {
            assert_eq!(s.banks[b][0], 0xa0 + b as u8);
            assert_eq!(s.banks[b][1], 0xb0 + b as u8);
        }
    }

    #[test]
    fn parse_z80_v3_plus3_1ffd() {
        let s = Snapshot128::parse_z80(&synthetic_z80_v3_plus3()).unwrap();
        assert!(s.is_plus3());
        assert!(!s.is_plus2a());
        assert_eq!(s.model, Snapshot128Model::SpectrumPlus3);
        assert_eq!(s.pc, 0xc000);
        assert_eq!(s.page_7ffd, 0x03);
        assert_eq!(s.page_1ffd, Some(0x04));
        assert_eq!(s.banks[3][0], 0x13);
        assert_eq!(s.banks[7][0], 0x17);
    }

    #[test]
    fn parse_z80_v3_plus2a_via_modify_hardware() {
        let mut data = synthetic_z80_v3_plus3();
        data[37] |= 0x80; // modify hardware → +2A
        let s = Snapshot128::parse_z80(&data).unwrap();
        assert!(s.is_plus2a());
        assert!(!s.is_plus3());
        assert_eq!(s.model, Snapshot128Model::SpectrumPlus2A);
        assert_eq!(s.page_1ffd, Some(0x04));
    }

    #[test]
    fn parse_z80_v3_hw8_is_plus2a() {
        let mut data = synthetic_z80_v3_plus3();
        data[34] = 8; // xzx +2A
        let s = Snapshot128::parse_z80(&data).unwrap();
        assert!(s.is_plus2a());
        assert_eq!(s.model, Snapshot128Model::SpectrumPlus2A);
    }

    #[test]
    fn snapshot48_rejects_128_z80() {
        let err = Snapshot48::parse_z80(&synthetic_z80_v2_128()).unwrap_err();
        assert!(matches!(err, FormatError::Format(_)));
    }

    #[test]
    fn parse_sna128_regs_banks_pc() {
        let s = Snapshot128::parse_sna(&synthetic_sna128(6)).unwrap();
        assert_eq!(s.af, 0xaa55);
        assert_eq!(s.sp, 0xfffe);
        assert_eq!(s.pc, 0x8000);
        assert_eq!(s.page_7ffd, 6);
        assert_eq!(s.border, 2);
        assert_eq!(s.banks[5][0], 0x51);
        assert_eq!(s.banks[2][0], 0x21);
        assert_eq!(s.banks[6][0], 0xc6);
        assert_eq!(s.banks[0][0], 0x40);
        assert_eq!(s.banks[1][0], 0x41);
        assert_eq!(s.banks[3][0], 0x43);
        assert_eq!(s.banks[4][0], 0x44);
        assert_eq!(s.banks[7][0], 0x47);
    }

    #[test]
    fn parse_sna128_when_paged_is_bank5() {
        let s = Snapshot128::parse_sna(&synthetic_sna128(5)).unwrap();
        assert_eq!(s.page_7ffd, 5);
        assert_eq!(s.banks[5][0], 0xc5); // last of the three slots overwrites bank 5
        assert_eq!(s.banks[0][0], 0x40);
        assert_eq!(s.banks[7][0], 0x47);
    }
}
