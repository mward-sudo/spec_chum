//! Timex TS2068 / TC2068 dock cartridge state (#192).
//!
//! Warajevo `.dck` banks overlay DOCK (MMU when ALTMEMBANK clear), optional
//! HOME ROM/RAM replace, and optional EX-ROM chunk replace.

use formats::{DckBankId, DckChunkAccess, DckImage, DCK_CHUNK_SIZE};

/// One 8 KiB Timex bank chunk from a dock cartridge.
#[derive(Clone, Debug)]
pub enum TimexDockChunk {
    /// No cartridge memory — empty dock reads `0xFF`.
    Absent,
    Rom([u8; DCK_CHUNK_SIZE]),
    Ram([u8; DCK_CHUNK_SIZE]),
}

impl TimexDockChunk {
    #[must_use]
    pub fn from_dck(access: DckChunkAccess, page: Option<&[u8; DCK_CHUNK_SIZE]>) -> Self {
        match access {
            DckChunkAccess::Null => Self::Absent,
            DckChunkAccess::Rom => {
                let mut data = [0u8; DCK_CHUNK_SIZE];
                if let Some(p) = page {
                    data.copy_from_slice(p);
                }
                Self::Rom(data)
            }
            DckChunkAccess::RamEmpty => Self::Ram([0u8; DCK_CHUNK_SIZE]),
            DckChunkAccess::Ram => {
                let mut data = [0u8; DCK_CHUNK_SIZE];
                if let Some(p) = page {
                    data.copy_from_slice(p);
                }
                Self::Ram(data)
            }
        }
    }

    #[must_use]
    pub fn read(&self, offset: usize) -> Option<u8> {
        match self {
            Self::Absent => None,
            Self::Rom(d) | Self::Ram(d) => Some(d[offset & (DCK_CHUNK_SIZE - 1)]),
        }
    }

    pub fn write(&mut self, offset: usize, value: u8) -> bool {
        match self {
            Self::Ram(d) => {
                d[offset & (DCK_CHUNK_SIZE - 1)] = value;
                true
            }
            Self::Absent | Self::Rom(_) => false,
        }
    }

    #[must_use]
    pub fn is_present(&self) -> bool {
        !matches!(self, Self::Absent)
    }

    #[must_use]
    pub fn blocks_home_write(&self) -> bool {
        matches!(self, Self::Rom(_))
    }
}

/// Inserted Timex dock cartridge (may touch DOCK / HOME / EX-ROM banks).
#[derive(Clone, Debug)]
pub struct TimexDock {
    pub dock: [TimexDockChunk; 8],
    pub home: [TimexDockChunk; 8],
    pub exrom: [TimexDockChunk; 8],
}

impl Default for TimexDock {
    fn default() -> Self {
        Self::empty()
    }
}

impl TimexDock {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            dock: std::array::from_fn(|_| TimexDockChunk::Absent),
            home: std::array::from_fn(|_| TimexDockChunk::Absent),
            exrom: std::array::from_fn(|_| TimexDockChunk::Absent),
        }
    }

    /// Apply a parsed `.dck` image (later banks override earlier ones per chunk).
    pub fn from_dck(image: &DckImage) -> Self {
        let mut dock = Self::empty();
        for bank in &image.banks {
            let target = match bank.bank {
                DckBankId::Dock => &mut dock.dock,
                DckBankId::Home => &mut dock.home,
                DckBankId::Exrom => &mut dock.exrom,
            };
            for (i, slot) in target.iter_mut().enumerate() {
                *slot = TimexDockChunk::from_dck(bank.access[i], bank.pages[i].as_ref());
            }
        }
        dock
    }

    #[must_use]
    pub fn has_any_content(&self) -> bool {
        self.dock.iter().any(TimexDockChunk::is_present)
            || self.home.iter().any(TimexDockChunk::is_present)
            || self.exrom.iter().any(TimexDockChunk::is_present)
    }
}
