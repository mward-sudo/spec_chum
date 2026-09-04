//! Microdrive cartridge `.MDR` image container.
//!
//! Standard layout: 254 sectors × 543 bytes (+ optional trailing write-protect byte).
//! Sector R/W is opaque 543-byte blocks — enough for IF1 smoke tests without ROM.

use crate::error::FormatError;

/// Sectors on a full Microdrive cartridge image.
pub const MDR_SECTORS: usize = 254;
/// Bytes per MDR sector (header + data + checksums as stored in `.mdr` files).
pub const MDR_SECTOR_SIZE: usize = 543;
/// Header record length within each sector (Fuse `LIBSPECTRUM_MICRODRIVE_HEAD_LEN`).
pub const MDR_HEAD_LEN: usize = 15;
/// Data bytes per record (Fuse `DATA_LEN`); plus 1 checksum follows in the sector.
pub const MDR_DATA_LEN: usize = 512;
/// Full cartridge without write-protect flag.
pub const MDR_IMAGE_SIZE: usize = MDR_SECTORS * MDR_SECTOR_SIZE;

#[derive(Clone, Debug)]
pub struct MdrImage {
    pub sectors: Vec<[u8; MDR_SECTOR_SIZE]>,
    /// Trailing write-protect flag when present in the file (`0` = writable).
    pub write_protected: bool,
}

impl MdrImage {
    #[must_use]
    pub fn blank() -> Self {
        Self {
            sectors: vec![[0u8; MDR_SECTOR_SIZE]; MDR_SECTORS],
            write_protected: false,
        }
    }

    pub fn parse(data: &[u8]) -> Result<Self, FormatError> {
        if data.len() != MDR_IMAGE_SIZE && data.len() != MDR_IMAGE_SIZE + 1 {
            return Err(FormatError::Format(format!(
                "MDR expected {MDR_IMAGE_SIZE} or {} bytes, got {}",
                MDR_IMAGE_SIZE + 1,
                data.len()
            )));
        }
        let mut sectors = Vec::with_capacity(MDR_SECTORS);
        for i in 0..MDR_SECTORS {
            let off = i * MDR_SECTOR_SIZE;
            let mut sec = [0u8; MDR_SECTOR_SIZE];
            sec.copy_from_slice(&data[off..off + MDR_SECTOR_SIZE]);
            sectors.push(sec);
        }
        let write_protected = data.len() == MDR_IMAGE_SIZE + 1 && data[MDR_IMAGE_SIZE] != 0;
        Ok(Self {
            sectors,
            write_protected,
        })
    }

    #[must_use]
    pub fn read_sector(&self, index: usize) -> Option<&[u8; MDR_SECTOR_SIZE]> {
        self.sectors.get(index)
    }

    pub fn write_sector(&mut self, index: usize, data: &[u8]) -> Result<(), FormatError> {
        if self.write_protected {
            return Err(FormatError::Format("MDR is write-protected".into()));
        }
        let Some(slot) = self.sectors.get_mut(index) else {
            return Err(FormatError::Format("MDR sector out of range".into()));
        };
        if data.len() > MDR_SECTOR_SIZE {
            return Err(FormatError::Format("MDR sector too long".into()));
        }
        slot.fill(0);
        slot[..data.len()].copy_from_slice(data);
        Ok(())
    }

    /// Serialize to `.mdr` bytes (always includes write-protect trailing byte).
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(MDR_IMAGE_SIZE + 1);
        for sec in &self.sectors {
            out.extend_from_slice(sec);
        }
        out.push(u8::from(self.write_protected));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sector_roundtrip() {
        let mut img = MdrImage::blank();
        img.write_sector(3, &[0xaa, 0xbb, 0xcc]).unwrap();
        assert_eq!(img.read_sector(3).unwrap()[0], 0xaa);
        assert_eq!(img.read_sector(3).unwrap()[2], 0xcc);
        let bytes = img.to_bytes();
        let img2 = MdrImage::parse(&bytes).unwrap();
        assert_eq!(img2.read_sector(3).unwrap()[1], 0xbb);
    }

    #[test]
    fn parse_rejects_bad_size() {
        assert!(MdrImage::parse(&[0u8; 10]).is_err());
    }
}
