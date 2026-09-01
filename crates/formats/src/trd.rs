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
                let tracks = (have / TRD_SECTORS_PER_TRACK).clamp(1, 80) as u8;
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
        let idx = self.index_chs(track, side, sector)?;
        self.sectors.get(idx)
    }

    fn index_chs(&self, track: u8, side: u8, sector: u8) -> Option<usize> {
        if side >= self.sides || sector as usize >= TRD_SECTORS_PER_TRACK {
            return None;
        }
        if usize::from(track) >= usize::from(self.tracks) {
            return None;
        }
        let idx = (usize::from(track) * usize::from(self.sides) + usize::from(side))
            * TRD_SECTORS_PER_TRACK
            + usize::from(sector);
        if idx >= self.sectors.len() {
            return None;
        }
        Some(idx)
    }

    /// Write a 256-byte sector on side 0 (`sector` is 0-based 0..15).
    pub fn write_sector(
        &mut self,
        track: usize,
        sector: usize,
        data: &[u8; TRD_SECTOR_SIZE],
    ) -> bool {
        let Some(idx) = self.index(track, sector) else {
            return false;
        };
        self.sectors[idx] = *data;
        true
    }

    /// Write a 256-byte sector by CHS (`sector` is 0-based 0..15).
    pub fn write_sector_chs(
        &mut self,
        track: u8,
        side: u8,
        sector: u8,
        data: &[u8; TRD_SECTOR_SIZE],
    ) -> bool {
        let Some(idx) = self.index_chs(track, side, sector) else {
            return false;
        };
        self.sectors[idx] = *data;
        true
    }

    /// 40-track SS TR-DOS disk with BASIC `boot` (`10 POKE 32768,165`).
    ///
    /// `RUN` with no filename (Beta manual / TR-DOS 5) loads and runs this
    /// program. Geometry: 40T SS 16×256 (`disk_type = 0x19`). Directory + info
    /// on track 0; file body at track 1 sector 0. In-repo fixture — not a
    /// committed `.trd` blob ([#140](https://github.com/mward-sudo/spec_chum/issues/140)).
    #[must_use]
    pub fn synthetic_trdos_boot_basic() -> Self {
        const TRACKS: u8 = 40;
        const DISK_TYPE: u8 = 0x19; // 40-track single-sided
        let total_sectors = usize::from(TRACKS) * TRD_SECTORS_PER_TRACK;
        let mut raw = vec![0u8; total_sectors * TRD_SECTOR_SIZE];

        let program = trdos_basic_poke_marker();
        let mut file = Vec::with_capacity(program.len() + 4);
        file.extend_from_slice(&program);
        file.push(0x80); // empty VARS
        file.extend_from_slice(&[0xAA, 0x0A, 0x00]); // autostart LINE 10

        let prog_plus_vars = (program.len() + 1) as u16; // minus 0xAA line bytes
        let vars_off = program.len() as u16;
        let mut dirent = [0u8; 16];
        dirent[0..8].copy_from_slice(b"boot    ");
        dirent[8] = b'B';
        dirent[9..11].copy_from_slice(&prog_plus_vars.to_le_bytes());
        dirent[11..13].copy_from_slice(&vars_off.to_le_bytes());
        dirent[13] = 1; // one sector
        dirent[14] = 0; // start sector
        dirent[15] = 1; // start track
        raw[..16].copy_from_slice(&dirent);

        let info_off = 8 * TRD_SECTOR_SIZE;
        let info = &mut raw[info_off..info_off + TRD_SECTOR_SIZE];
        info[0xe1] = 1; // first free sector
        info[0xe2] = 1; // first free track
        info[0xe3] = DISK_TYPE;
        info[0xe4] = 1; // file count
        let free = (total_sectors - TRD_SECTORS_PER_TRACK - 1) as u16;
        info[0xe5..0xe7].copy_from_slice(&free.to_le_bytes());
        info[0xe7] = 0x10; // TR-DOS ID
        info[0xe9..0xf2].fill(b' ');
        info[0xf5..0xfd].copy_from_slice(b"BOOTDISK");

        let data_off = TRD_SECTORS_PER_TRACK * TRD_SECTOR_SIZE; // track 1 sector 0
        let n = file.len().min(TRD_SECTOR_SIZE);
        raw[data_off..data_off + n].copy_from_slice(&file[..n]);

        let mut sectors = Vec::with_capacity(total_sectors);
        for chunk in raw.as_chunks::<TRD_SECTOR_SIZE>().0 {
            let mut s = [0u8; TRD_SECTOR_SIZE];
            s.copy_from_slice(chunk);
            sectors.push(s);
        }
        Self {
            tracks: TRACKS,
            sides: 1,
            disk_type: DISK_TYPE,
            label: *b"BOOTDISK",
            sectors,
        }
    }
}

/// `10 POKE 32768,165` — same ZX float encoding as the +3DOS `DISK` fixture.
fn trdos_basic_poke_marker() -> Vec<u8> {
    vec![
        0x00, 0x0A, 0x17, 0x00, 0xF4, b'3', b'2', b'7', b'6', b'8', 0x0E, 0x90, 0x00, 0x00, 0x00,
        0x00, b',', b'1', b'6', b'5', 0x0E, 0x00, 0x00, 0xA5, 0x00, 0x00, 0x0D,
    ]
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

    #[test]
    fn write_sector_roundtrip() {
        let mut img = TrdImage::parse(&synthetic_trd()).unwrap();
        let mut data = [0u8; TRD_SECTOR_SIZE];
        data[0] = 0xbe;
        data[1] = 0xef;
        assert!(img.write_sector(0, 1, &data));
        let sec = img.read_sector(0, 1).unwrap();
        assert_eq!([sec[0], sec[1]], [0xbe, 0xef]);
        assert!(!img.write_sector(0, 16, &data));
    }

    #[test]
    fn synthetic_trdos_boot_basic_has_boot_file() {
        let img = TrdImage::synthetic_trdos_boot_basic();
        assert_eq!(img.tracks, 40);
        assert_eq!(img.sides, 1);
        assert_eq!(img.disk_type, 0x19);
        assert_eq!(&img.label, b"BOOTDISK");

        let dir = img.read_sector(0, 0).unwrap();
        assert_eq!(&dir[0..8], b"boot    ");
        assert_eq!(dir[8], b'B');
        assert_eq!(&dir[9..11], 28u16.to_le_bytes()); // program + empty VARS
        assert_eq!(&dir[11..13], 27u16.to_le_bytes()); // variables offset
        assert_eq!(dir[13], 1);
        assert_eq!(dir[14], 0);
        assert_eq!(dir[15], 1);

        let info = img.read_sector(0, 8).unwrap();
        assert_eq!(info[0xe1], 1);
        assert_eq!(info[0xe2], 1);
        assert_eq!(info[0xe3], 0x19);
        assert_eq!(info[0xe4], 1);
        assert_eq!(&info[0xe5..0xe7], 623u16.to_le_bytes());
        assert_eq!(info[0xe7], 0x10);

        let body = img.read_sector_chs(1, 0, 0).unwrap();
        assert_eq!(&body[0..4], [0x00, 0x0A, 0x17, 0x00]);
        assert_eq!(&body[11..16], [0x90, 0x00, 0x00, 0x00, 0x00]);
        assert_eq!(body[27], 0x80);
        assert_eq!(&body[28..31], [0xAA, 0x0A, 0x00]);
    }
}
