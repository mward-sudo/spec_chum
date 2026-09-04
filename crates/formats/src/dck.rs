//! Warajevo Timex `.dck` cartridge images (Fuse / libspectrum compatible).
//!
//! Layout: one or more 9-byte bank headers followed by 8 KiB pages for each
//! chunk whose access byte is ROM (`2`) or RAM-with-image (`3`).

use crate::error::FormatError;

/// Bytes per Timex horizontal-MMU chunk.
pub const DCK_CHUNK_SIZE: usize = 0x2000;
/// Bank / chunk header size before optional page images.
pub const DCK_HEADER_SIZE: usize = 9;

/// Warajevo bank IDs (libspectrum `LIBSPECTRUM_DCK_BANK_*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum DckBankId {
    Dock = 0,
    Exrom = 254,
    Home = 255,
}

impl DckBankId {
    pub fn from_u8(v: u8) -> Result<Self, FormatError> {
        match v {
            0 => Ok(Self::Dock),
            254 => Ok(Self::Exrom),
            255 => Ok(Self::Home),
            other => Err(FormatError::Format(format!("DCK unknown bank ID {other}"))),
        }
    }
}

/// Per-chunk access type (libspectrum `LIBSPECTRUM_DCK_PAGE_*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum DckChunkAccess {
    /// No cartridge memory in this chunk (empty dock reads `0xFF`).
    Null = 0,
    /// Writable RAM, initially zeroed (no image in file).
    RamEmpty = 1,
    /// Read-only ROM (8 KiB image follows in file).
    Rom = 2,
    /// Writable RAM with initial image in file.
    Ram = 3,
}

impl DckChunkAccess {
    pub fn from_u8(v: u8) -> Result<Self, FormatError> {
        match v {
            0 => Ok(Self::Null),
            1 => Ok(Self::RamEmpty),
            2 => Ok(Self::Rom),
            3 => Ok(Self::Ram),
            other => Err(FormatError::Format(format!(
                "DCK unknown chunk access {other}"
            ))),
        }
    }

    #[must_use]
    pub fn has_image(self) -> bool {
        matches!(self, Self::Rom | Self::Ram)
    }

    #[must_use]
    pub fn writable(self) -> bool {
        matches!(self, Self::RamEmpty | Self::Ram)
    }
}

/// One bank block inside a `.dck` file.
#[derive(Clone, Debug)]
pub struct DckBank {
    pub bank: DckBankId,
    pub access: [DckChunkAccess; 8],
    /// Present only when `access[i]` is `Rom` or `Ram`.
    pub pages: [Option<[u8; DCK_CHUNK_SIZE]>; 8],
}

/// Parsed Timex dock cartridge.
#[derive(Clone, Debug, Default)]
pub struct DckImage {
    pub banks: Vec<DckBank>,
}

impl DckImage {
    pub fn parse(data: &[u8]) -> Result<Self, FormatError> {
        if data.is_empty() {
            return Err(FormatError::Format("DCK empty".into()));
        }
        let mut banks = Vec::new();
        let mut off = 0usize;
        while off < data.len() {
            if data.len() - off < DCK_HEADER_SIZE {
                return Err(FormatError::Format("DCK truncated bank header".into()));
            }
            let bank = DckBankId::from_u8(data[off])?;
            let mut access = [DckChunkAccess::Null; 8];
            let mut image_count = 0usize;
            for i in 0..8 {
                let a = DckChunkAccess::from_u8(data[off + 1 + i])?;
                access[i] = a;
                if a.has_image() {
                    image_count += 1;
                }
            }
            off += DCK_HEADER_SIZE;
            let need = image_count * DCK_CHUNK_SIZE;
            if data.len() - off < need {
                return Err(FormatError::Format(format!(
                    "DCK truncated: need {need} page bytes, have {}",
                    data.len() - off
                )));
            }
            let mut pages: [Option<[u8; DCK_CHUNK_SIZE]>; 8] = [None; 8];
            for i in 0..8 {
                if access[i].has_image() {
                    let mut page = [0u8; DCK_CHUNK_SIZE];
                    page.copy_from_slice(&data[off..off + DCK_CHUNK_SIZE]);
                    pages[i] = Some(page);
                    off += DCK_CHUNK_SIZE;
                }
            }
            banks.push(DckBank {
                bank,
                access,
                pages,
            });
        }
        Ok(Self { banks })
    }

    /// Build a DOCK-bank Spectrum 16K ROM cartridge (`OUT 244,3` pages it).
    #[must_use]
    pub fn spectrum_rom_dock(rom16k: &[u8; 16384]) -> Self {
        let mut lo = [0u8; DCK_CHUNK_SIZE];
        let mut hi = [0u8; DCK_CHUNK_SIZE];
        lo.copy_from_slice(&rom16k[..DCK_CHUNK_SIZE]);
        hi.copy_from_slice(&rom16k[DCK_CHUNK_SIZE..]);
        let mut pages = [None; 8];
        pages[0] = Some(lo);
        pages[1] = Some(hi);
        Self {
            banks: vec![DckBank {
                bank: DckBankId::Dock,
                access: [
                    DckChunkAccess::Rom,
                    DckChunkAccess::Rom,
                    DckChunkAccess::Null,
                    DckChunkAccess::Null,
                    DckChunkAccess::Null,
                    DckChunkAccess::Null,
                    DckChunkAccess::Null,
                    DckChunkAccess::Null,
                ],
                pages,
            }],
        }
    }

    /// Build a HOME-bank Spectrum ROM replace (Timex → Spectrum-like home).
    #[must_use]
    pub fn spectrum_rom_home(rom16k: &[u8; 16384]) -> Self {
        let mut lo = [0u8; DCK_CHUNK_SIZE];
        let mut hi = [0u8; DCK_CHUNK_SIZE];
        lo.copy_from_slice(&rom16k[..DCK_CHUNK_SIZE]);
        hi.copy_from_slice(&rom16k[DCK_CHUNK_SIZE..]);
        let mut pages = [None; 8];
        pages[0] = Some(lo);
        pages[1] = Some(hi);
        Self {
            banks: vec![DckBank {
                bank: DckBankId::Home,
                access: [
                    DckChunkAccess::Rom,
                    DckChunkAccess::Rom,
                    DckChunkAccess::Null,
                    DckChunkAccess::Null,
                    DckChunkAccess::Null,
                    DckChunkAccess::Null,
                    DckChunkAccess::Null,
                    DckChunkAccess::Null,
                ],
                pages,
            }],
        }
    }

    /// Serialize back to Warajevo `.dck` bytes (for tests / helpers).
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for bank in &self.banks {
            out.push(bank.bank as u8);
            for a in &bank.access {
                out.push(*a as u8);
            }
            for i in 0..8 {
                if let Some(page) = &bank.pages[i] {
                    out.extend_from_slice(page);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_spectrum_dock_header() {
        let mut rom = [0u8; 16384];
        rom[0] = 0xF3;
        rom[1] = 0xAF;
        rom[0x2000] = 0xAA;
        let img = DckImage::spectrum_rom_dock(&rom);
        let bytes = img.to_bytes();
        assert_eq!(bytes.len(), 9 + 2 * DCK_CHUNK_SIZE);
        assert_eq!(&bytes[..9], &[0, 2, 2, 0, 0, 0, 0, 0, 0]);
        let parsed = DckImage::parse(&bytes).unwrap();
        assert_eq!(parsed.banks.len(), 1);
        assert_eq!(parsed.banks[0].bank, DckBankId::Dock);
        assert_eq!(parsed.banks[0].pages[0].unwrap()[0], 0xF3);
        assert_eq!(parsed.banks[0].pages[1].unwrap()[0], 0xAA);
    }

    #[test]
    fn parse_home_replace_and_empty_ram() {
        // 64K DOCK RAM disc: header only.
        let empty_ram = [0u8, 1, 1, 1, 1, 1, 1, 1, 1];
        let img = DckImage::parse(&empty_ram).unwrap();
        assert_eq!(img.banks[0].bank, DckBankId::Dock);
        assert!(img.banks[0].pages.iter().all(|p| p.is_none()));
        assert!(img.banks[0]
            .access
            .iter()
            .all(|a| *a == DckChunkAccess::RamEmpty));

        let mut rom = [0u8; 16384];
        rom[0] = 0x42;
        let home = DckImage::spectrum_rom_home(&rom).to_bytes();
        assert_eq!(&home[..9], &[255, 2, 2, 0, 0, 0, 0, 0, 0]);
        let parsed = DckImage::parse(&home).unwrap();
        assert_eq!(parsed.banks[0].bank, DckBankId::Home);
        assert_eq!(parsed.banks[0].pages[0].unwrap()[0], 0x42);
    }

    #[test]
    fn reject_unknown_bank() {
        let err = DckImage::parse(&[1, 0, 0, 0, 0, 0, 0, 0, 0]).unwrap_err();
        assert!(err.to_string().contains("unknown bank"));
    }

    #[test]
    fn reject_truncated_pages() {
        let err = DckImage::parse(&[0, 2, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3]).unwrap_err();
        assert!(err.to_string().contains("truncated"));
    }
}
