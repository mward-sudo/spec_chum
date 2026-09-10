//! Microdrive cartridge `.MDR` image container.
//!
//! Standard layout: 254 sectors × 543 bytes (+ optional trailing write-protect byte).
//! Sector layout matches Spectator / Fuse / Carlo Delhez MDR docs (Refs [#139](https://github.com/mward-sudo/spec_chum/issues/139)):
//!
//! | Off | Len | Field |
//! | --- | --- | --- |
//! | 0 | 1 | `HDFLAG` (1 = header block) |
//! | 1 | 1 | `HDNUMB` (254 … 1) |
//! | 2 | 2 | unused |
//! | 4 | 10 | `HDNAME` (cartridge name, space-padded) |
//! | 14 | 1 | `HDCHK` (sum of bytes 0..13 mod 255) |
//! | 15 | 1 | `RECFLG` |
//! | 16 | 1 | `RECNUM` |
//! | 17 | 2 | `RECLEN` (LE) |
//! | 19 | 10 | `RECNAM` |
//! | 29 | 1 | `DESCHK` (sum of bytes 15..28 mod 255) |
//! | 30 | 512 | data |
//! | 542 | 1 | `DCHK` (sum of 512 data bytes mod 255) |
//!
//! Checksums never equal `0xFF` (sum mod 255).

use crate::error::FormatError;

/// Sectors on a full Microdrive cartridge image.
pub const MDR_SECTORS: usize = 254;
/// Bytes per MDR sector (header + data + checksums as stored in `.mdr` files).
pub const MDR_SECTOR_SIZE: usize = 543;
/// Header record length within each sector (Fuse `LIBSPECTRUM_MICRODRIVE_HEAD_LEN`).
pub const MDR_HEAD_LEN: usize = 15;
/// Data bytes per record (Fuse `DATA_LEN`); plus 1 checksum follows in the sector.
pub const MDR_DATA_LEN: usize = 512;
/// Record descriptor length before data (`RECFLG`…`DESCHK`).
pub const MDR_DES_LEN: usize = 15;
/// Full cartridge without write-protect flag.
pub const MDR_IMAGE_SIZE: usize = MDR_SECTORS * MDR_SECTOR_SIZE;
/// Max cartridge / file name length in header / record fields.
pub const MDR_NAME_LEN: usize = 10;

#[derive(Clone, Debug)]
pub struct MdrImage {
    pub sectors: Vec<[u8; MDR_SECTOR_SIZE]>,
    /// Trailing write-protect flag when present in the file (`0` = writable).
    pub write_protected: bool,
}

/// Spectator / Fuse MDR checksum: sum of bytes modulo 255 (never yields 255).
#[must_use]
pub fn mdr_checksum(data: &[u8]) -> u8 {
    let mut sum: u16 = 0;
    for &b in data {
        sum = (sum + u16::from(b)) % 255;
    }
    sum as u8
}

fn pad_name(name: &str) -> [u8; MDR_NAME_LEN] {
    let mut out = [b' '; MDR_NAME_LEN];
    let bytes = name.as_bytes();
    let n = bytes.len().min(MDR_NAME_LEN);
    out[..n].copy_from_slice(&bytes[..n]);
    out
}

impl MdrImage {
    /// Zero-filled sectors (unformatted / not IF1-ready). Prefer [`Self::formatted`].
    #[must_use]
    pub fn blank() -> Self {
        Self {
            sectors: vec![[0u8; MDR_SECTOR_SIZE]; MDR_SECTORS],
            write_protected: false,
        }
    }

    /// Empty but **formatted** cartridge: valid headers (`HDNUMB` 254…1), empty records, Fuse checksums.
    ///
    /// Suitable for IF1 stream smoke tests without running ROM `FORMAT`.
    #[must_use]
    pub fn formatted(cartridge_name: &str) -> Self {
        let mut img = Self::blank();
        let name = pad_name(cartridge_name);
        for (i, sector) in img.sectors.iter_mut().enumerate() {
            let hdnumb = (MDR_SECTORS - i) as u8; // 254 … 1
            sector[0] = 0x01; // HDFLAG: header block
            sector[1] = hdnumb;
            sector[2] = 0;
            sector[3] = 0;
            sector[4..14].copy_from_slice(&name);
            sector[14] = mdr_checksum(&sector[0..14]);

            // Empty record: RECFLG=0, RECNUM=0, RECLEN=0, blank name, zero data.
            sector[15] = 0x00;
            sector[16] = 0x00;
            sector[17] = 0x00;
            sector[18] = 0x00;
            sector[19..29].fill(b' ');
            sector[29] = mdr_checksum(&sector[15..29]);
            sector[30..542].fill(0);
            sector[542] = mdr_checksum(&sector[30..542]);
        }
        img
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

    /// True when every sector has a valid header + record descriptor + data checksum.
    #[must_use]
    pub fn checksums_ok(&self) -> bool {
        self.sectors.iter().all(sector_checksums_ok)
    }

    /// True when headers look formatted (`HDFLAG==1`, `HDNUMB` 254…1, checksums OK).
    #[must_use]
    pub fn looks_formatted(&self) -> bool {
        self.sectors.iter().enumerate().all(|(i, s)| {
            let expect = (MDR_SECTORS - i) as u8;
            s[0] == 0x01 && s[1] == expect && sector_checksums_ok(s)
        })
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

/// Validate `HDCHK`, `DESCHK`, and `DCHK` for one 543-byte sector.
#[must_use]
pub fn sector_checksums_ok(sector: &[u8; MDR_SECTOR_SIZE]) -> bool {
    sector[14] == mdr_checksum(&sector[0..14])
        && sector[29] == mdr_checksum(&sector[15..29])
        && sector[542] == mdr_checksum(&sector[30..542])
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

    #[test]
    fn checksum_is_sum_mod_255_never_ff() {
        assert_eq!(mdr_checksum(&[0]), 0);
        assert_eq!(mdr_checksum(&[254]), 254);
        assert_eq!(mdr_checksum(&[255]), 0); // 255 % 255 == 0
        assert_eq!(mdr_checksum(&[100, 100, 100]), 45); // 300 % 255
        assert_ne!(mdr_checksum(&[0xff; 14]), 0xff);
    }

    #[test]
    fn formatted_cartridge_has_fuse_layout() {
        let img = MdrImage::formatted("CART");
        assert!(img.looks_formatted());
        assert!(img.checksums_ok());
        let s0 = img.read_sector(0).unwrap();
        assert_eq!(s0[0], 0x01);
        assert_eq!(s0[1], 254);
        assert_eq!(&s0[4..8], b"CART");
        assert_eq!(s0[8], b' ');
        let s_last = img.read_sector(MDR_SECTORS - 1).unwrap();
        assert_eq!(s_last[1], 1);
        // Empty record
        assert_eq!(s0[15], 0);
        assert_eq!(u16::from_le_bytes([s0[17], s0[18]]), 0);
    }

    #[test]
    fn blank_is_not_formatted() {
        let img = MdrImage::blank();
        assert!(!img.looks_formatted());
        // All-zero HDCHK happens to match sum(0)=0, but HDFLAG≠1 / HDNUMB wrong.
        assert!(!img.looks_formatted());
    }

    #[test]
    fn corrupting_hdchk_fails_sector_check() {
        let mut img = MdrImage::formatted("X");
        img.sectors[0][14] ^= 0x01;
        assert!(!sector_checksums_ok(&img.sectors[0]));
        assert!(!img.checksums_ok());
    }

    #[test]
    fn formatted_roundtrips_parse() {
        let img = MdrImage::formatted("TEST NAME!");
        let img2 = MdrImage::parse(&img.to_bytes()).unwrap();
        assert!(img2.looks_formatted());
        assert_eq!(&img2.sectors[0][4..14], b"TEST NAME!");
    }
}
