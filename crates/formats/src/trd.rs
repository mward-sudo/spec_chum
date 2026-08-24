//! TR-DOS `.TRD` disk image parser.
//!
//! Geometry: 16 sectors/track × 256 bytes. Images are stored as a linear sector
//! array (track-major for single-sided synthetics). Full 80×2 DS disks are
//! accepted when the info sector type byte is set.

use std::path::Path;

use crate::FormatError;

/// Bytes per TR-DOS sector.
pub const TRD_SECTOR_SIZE: usize = 256;
/// Sectors per track (fixed for TR-DOS).
pub const TRD_SECTORS_PER_TRACK: usize = 16;

/// Parsed TR-DOS disk image.
#[derive(Clone, Debug)]
pub struct TrdImage {
    pub tracks: u8,
    pub sides: u8,
    /// Disk type byte from the info sector (e.g. `0x16` = 80-track DS).
    pub disk_type: u8,
    /// Volume label (8 chars, padded) when present.
    pub label: [u8; 8],
    /// Raw sectors in file order.
    sectors: Vec<[u8; TRD_SECTOR_SIZE]>,
}

impl TrdImage {
    pub fn load(path: &Path) -> Result<Self, FormatError> {
        let data = std::fs::read(path).map_err(FormatError::Io)?;
        Self::parse(&data)
    }

    pub fn parse(data: &[u8]) -> Result<Self, FormatError> {
        if data.len() < TRD_SECTOR_SIZE {
            return Err(FormatError::Format("TRD too short".into()));
        }
        let have = data.len() / TRD_SECTOR_SIZE;
        // Info sector is usually logical sector 8 on track 0; fall back to sector 0.
        let info = if have > 8 {
            &data[8 * TRD_SECTOR_SIZE..9 * TRD_SECTOR_SIZE]
        } else {
            &data[..TRD_SECTOR_SIZE]
        };

        let disk_type = info[0xe3];
        let (tracks, sides) = match disk_type {
            0x16 => (80u8, 2u8),
            0x17 => (40, 2),
            0x18 => (80, 1),
            0x19 => (40, 1),
            _ => {
                let tracks = ((have / TRD_SECTORS_PER_TRACK).max(1).min(80)) as u8;
                (tracks, 1u8)
            }
        };

        let expected = usize::from(tracks) * usize::from(sides) * TRD_SECTORS_PER_TRACK;
        if have < expected && disk_type != 0 {
            return Err(FormatError::Format(format!(
                "TRD truncated: {have} sectors, expected {expected}"
            )));
        }

        let mut sectors = Vec::with_capacity(have);
        for i in 0..have {
            let mut s = [0u8; TRD_SECTOR_SIZE];
            let off = i * TRD_SECTOR_SIZE;
            s.copy_from_slice(&data[off..off + TRD_SECTOR_SIZE]);
            sectors.push(s);
        }

        let mut label = [0u8; 8];
        label.copy_from_slice(&info[0xf5..0xfd]);

        Ok(Self {
            tracks,
            sides,
            disk_type,
            label,
            sectors,
        })
    }

    /// Linear sector index for `track` + `sector` (0..15) on side 0.
    fn index(&self, track: usize, sector: usize) -> Option<usize> {
        if sector >= TRD_SECTORS_PER_TRACK || track >= usize::from(self.tracks) {
            return None;
        }
        let idx = track * usize::from(self.sides) * TRD_SECTORS_PER_TRACK + sector;
        if idx >= self.sectors.len() {
            return None;
        }
        Some(idx)
    }

    /// Read sector by track + sector index (0-based sector 0..15, side 0).
    #[must_use]
    pub fn read_sector(&self, track: usize, sector: usize) -> Option<&[u8; TRD_SECTOR_SIZE]> {
        let idx = self.index(track, sector)?;
        self.sectors.get(idx)
    }

    /// CHS access (side used when `sides > 1`).
    #[must_use]
    pub fn read_sector_chs(
        &self,
        track: u8,
        side: u8,
        sector: u8,
    ) -> Option<&[u8; TRD_SECTOR_SIZE]> {
        if side >= self.sides || sector as usize >= TRD_SECTORS_PER_TRACK {
            return None;
        }
        let idx = (usize::from(track) * usize::from(self.sides) + usize::from(side))
            * TRD_SECTORS_PER_TRACK
            + usize::from(sector);
        self.sectors.get(idx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_trd() -> Vec<u8> {
        let mut data = vec![0u8; TRD_SECTORS_PER_TRACK * TRD_SECTOR_SIZE];
        data[TRD_SECTOR_SIZE] = 0xde;
        data[TRD_SECTOR_SIZE + 1] = 0xad;
        data[0xe3] = 0;
        data[0xf5..0xfd].copy_from_slice(b"TESTDISK");
        data
    }

    #[test]
    fn parse_and_read_sector() {
        let img = TrdImage::parse(&synthetic_trd()).unwrap();
        assert_eq!(img.tracks, 1);
        assert_eq!(img.sides, 1);
        let sec = img.read_sector(0, 1).unwrap();
        assert_eq!([sec[0], sec[1]], [0xde, 0xad]);
        assert!(img.read_sector(0, 16).is_none());
    }
}
