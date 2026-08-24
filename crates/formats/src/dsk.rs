//! Amstrad/Spectrum +3 `.DSK` / Extended DSK image reader.
//!
//! Provides sector lookup sufficient for unit tests and a minimal FDC path.

use std::path::Path;

use crate::FormatError;

#[derive(Clone, Debug)]
pub struct DskImage {
    pub tracks: u8,
    pub sides: u8,
    pub extended: bool,
    /// Raw track data blobs in file order.
    tracks_data: Vec<TrackData>,
}

#[derive(Clone, Debug)]
struct TrackData {
    sectors: Vec<Sector>,
}

#[derive(Clone, Debug)]
pub struct Sector {
    pub track: u8,
    pub side: u8,
    pub sector_id: u8,
    pub size_code: u8,
    pub data: Vec<u8>,
}

impl DskImage {
    pub fn load(path: &Path) -> Result<Self, FormatError> {
        let data = std::fs::read(path).map_err(FormatError::Io)?;
        Self::parse(&data)
    }

    pub fn parse(data: &[u8]) -> Result<Self, FormatError> {
        if data.len() < 0x100 {
            return Err(FormatError::Format("DSK too short".into()));
        }
        let magic = std::str::from_utf8(&data[0..8]).unwrap_or("");
        let extended = magic.starts_with("EXTENDED");
        if !extended && !magic.starts_with("MV - CPC") {
            return Err(FormatError::Format("not a DSK image".into()));
        }
        let tracks = data[0x30];
        let sides = data[0x31].max(1);
        let mut tracks_data = Vec::new();
        let mut offset = 0x100usize;
        let total = usize::from(tracks) * usize::from(sides);

        if extended {
            // Track size table at 0x34
            for t in 0..total {
                let size = usize::from(data[0x34 + t]) * 256;
                if size == 0 {
                    tracks_data.push(TrackData {
                        sectors: Vec::new(),
                    });
                    continue;
                }
                if offset + size > data.len() {
                    return Err(FormatError::Format("extended track truncated".into()));
                }
                let track = parse_track(&data[offset..offset + size])?;
                tracks_data.push(track);
                offset += size;
            }
        } else {
            let track_size = u16::from_le_bytes([data[0x32], data[0x33]]) as usize;
            if track_size == 0 {
                return Err(FormatError::Format("invalid track size".into()));
            }
            for _ in 0..total {
                if offset + track_size > data.len() {
                    return Err(FormatError::Format("track truncated".into()));
                }
                let track = parse_track(&data[offset..offset + track_size])?;
                tracks_data.push(track);
                offset += track_size;
            }
        }

        Ok(Self {
            tracks,
            sides,
            extended,
            tracks_data,
        })
    }

    /// Find sector by physical track/side/id.
    #[must_use]
    pub fn find_sector(&self, track: u8, side: u8, sector_id: u8) -> Option<&Sector> {
        let idx = usize::from(track) * usize::from(self.sides) + usize::from(side);
        self.tracks_data
            .get(idx)?
            .sectors
            .iter()
            .find(|s| s.track == track && s.side == side && s.sector_id == sector_id)
    }
}

fn parse_track(data: &[u8]) -> Result<TrackData, FormatError> {
    if data.len() < 0x100 {
        return Err(FormatError::Format("track header short".into()));
    }
    if &data[0..0x0c] != b"Track-Info\r\n" {
        return Err(FormatError::Format("missing Track-Info".into()));
    }
    let sector_count = data[0x15] as usize;
    let mut sectors = Vec::with_capacity(sector_count);
    let mut data_off = 0x100usize;
    for s in 0..sector_count {
        let info_off = 0x18 + s * 8;
        if info_off + 8 > data.len() {
            break;
        }
        let track = data[info_off];
        let side = data[info_off + 1];
        let sector_id = data[info_off + 2];
        let size_code = data[info_off + 3];
        let size = 128usize << size_code.min(6);
        if data_off + size > data.len() {
            return Err(FormatError::Format("sector data truncated".into()));
        }
        let sector_data = data[data_off..data_off + size].to_vec();
        data_off += size;
        sectors.push(Sector {
            track,
            side,
            sector_id,
            size_code,
            data: sector_data,
        });
    }
    Ok(TrackData { sectors })
}

/// Minimal +3 µPD765 path: READ DATA command stream → sector bytes on data port.
#[derive(Clone, Debug, Default)]
pub struct Plus3Fdc {
    pub image: Option<DskImage>,
    pub track: u8,
    pub side: u8,
    pub sector: u8,
    pub status: u8,
    last_data: Vec<u8>,
    data_index: usize,
    /// Command-phase bytes for READ DATA (`0x06` / `0x46`) then C/H/R/N…
    cmd: Vec<u8>,
    /// After a successful READ DATA: main status reports data ready.
    data_ready: bool,
}

impl Plus3Fdc {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, image: DskImage) {
        self.image = Some(image);
        self.status = 0;
        self.cmd.clear();
        self.data_ready = false;
        self.last_data.clear();
        self.data_index = 0;
    }

    /// Seek and prepare sector buffer; returns true if found.
    pub fn read_sector(&mut self, track: u8, side: u8, sector: u8) -> bool {
        self.track = track;
        self.side = side;
        self.sector = sector;
        self.data_index = 0;
        if let Some(img) = self.image.as_ref() {
            if let Some(sec) = img.find_sector(track, side, sector) {
                self.last_data = sec.data.clone();
                self.status = 0;
                self.data_ready = true;
                return true;
            }
        }
        self.last_data.clear();
        self.status = 0x10;
        self.data_ready = false;
        false
    }

    /// Write a command/parameter byte (port `3FFD` data).
    ///
    /// Accepts READ DATA (`0x06`/`0x46`) followed by C, H, R, N.
    pub fn write_command_byte(&mut self, value: u8) {
        if self.cmd.is_empty() {
            if value & 0x1f == 0x06 {
                self.cmd.push(value);
            }
            return;
        }
        self.cmd.push(value);
        if self.cmd.len() >= 5 {
            let c = self.cmd[1];
            let h = self.cmd[2];
            let r = self.cmd[3];
            let _n = self.cmd[4];
            let _ = self.read_sector(c, h, r);
            self.cmd.clear();
        }
    }

    /// Main status register (`2FFD`): bit7 RQM, bit6 DIO (1=FDC→CPU), bit4 busy.
    #[must_use]
    pub fn main_status(&self) -> u8 {
        let mut s = 0x80;
        if self.data_ready && self.data_remaining() > 0 {
            s |= 0x40 | 0x10;
        }
        s
    }

    #[must_use]
    pub fn data_remaining(&self) -> usize {
        self.last_data.len().saturating_sub(self.data_index)
    }

    pub fn read_data_byte(&mut self) -> u8 {
        if self.data_index < self.last_data.len() {
            let b = self.last_data[self.data_index];
            self.data_index += 1;
            if self.data_index >= self.last_data.len() {
                self.data_ready = false;
            }
            b
        } else {
            self.data_ready = false;
            0xff
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_dsk() -> Vec<u8> {
        let mut data = vec![0u8; 0x100];
        data[0..8].copy_from_slice(b"MV - CPC");
        data[0x30] = 1; // tracks
        data[0x31] = 1; // sides
        let track_size: u16 = 0x100 + 256; // header + one 256-byte sector
        data[0x32..0x34].copy_from_slice(&track_size.to_le_bytes());

        let mut track = vec![0u8; track_size as usize];
        track[0..12].copy_from_slice(b"Track-Info\r\n");
        track[0x10] = 0; // track
        track[0x11] = 0; // side
        track[0x14] = 1; // sector size code → 256
        track[0x15] = 1; // sector count
        track[0x18] = 0; // C
        track[0x19] = 0; // H
        track[0x1a] = 0xc1; // R
        track[0x1b] = 1; // N
                         // sector data
        track[0x100] = 0x42;
        track[0x101] = 0x43;
        data.extend_from_slice(&track);
        data
    }

    #[test]
    fn parse_and_read_sector() {
        let img = DskImage::parse(&synthetic_dsk()).unwrap();
        let sec = img.find_sector(0, 0, 0xc1).unwrap();
        assert_eq!(sec.data[0], 0x42);
        assert_eq!(sec.data[1], 0x43);
        let mut fdc = Plus3Fdc::new();
        fdc.insert(img);
        assert!(fdc.read_sector(0, 0, 0xc1));
        assert_eq!(fdc.read_data_byte(), 0x42);
        assert_eq!(fdc.read_data_byte(), 0x43);
    }

    #[test]
    fn read_data_command_stream_loads_sector() {
        let img = DskImage::parse(&synthetic_dsk()).unwrap();
        let mut fdc = Plus3Fdc::new();
        fdc.insert(img);
        fdc.write_command_byte(0x46);
        fdc.write_command_byte(0);
        fdc.write_command_byte(0);
        fdc.write_command_byte(0xc1);
        fdc.write_command_byte(1);
        assert!(fdc.data_remaining() > 0);
        assert_eq!(fdc.main_status() & 0xc0, 0xc0);
        assert_eq!(fdc.read_data_byte(), 0x42);
        assert_eq!(fdc.read_data_byte(), 0x43);
    }

    /// Two 256-byte sectors on track 0 for FDC sector-lookup goldens.
    ///
    /// `Plus3Fdc::write_command_byte` accepts READ DATA (`0x06`/`0x46`) + C/H/R/N;
    /// BusPlus3 routes ports `3FFD`/`2FFD` to that path. Full +3DOS boot still needs
    /// more µPD765 commands ([#141](https://github.com/mward-sudo/spec_chum/issues/141)).
    fn synthetic_dsk_two_sectors() -> Vec<u8> {
        let mut data = vec![0u8; 0x100];
        data[0..8].copy_from_slice(b"MV - CPC");
        data[0x30] = 1; // tracks
        data[0x31] = 1; // sides
        let track_size: u16 = 0x100 + 256 * 2;
        data[0x32..0x34].copy_from_slice(&track_size.to_le_bytes());

        let mut track = vec![0u8; track_size as usize];
        track[0..12].copy_from_slice(b"Track-Info\r\n");
        track[0x10] = 0;
        track[0x11] = 0;
        track[0x14] = 1; // N=1 → 256 bytes
        track[0x15] = 2; // two sectors
        track[0x18] = 0;
        track[0x19] = 0;
        track[0x1a] = 0xc1;
        track[0x1b] = 1;
        track[0x20] = 0;
        track[0x21] = 0;
        track[0x22] = 0xc2;
        track[0x23] = 1;
        track[0x100] = 0xa1;
        track[0x101] = 0xa2;
        track[0x200] = 0xb1;
        track[0x201] = 0xb2;
        data.extend_from_slice(&track);
        data
    }

    #[test]
    fn multi_sector_dsk_read_two_sectors() {
        let img = DskImage::parse(&synthetic_dsk_two_sectors()).unwrap();
        let s1 = img.find_sector(0, 0, 0xc1).unwrap();
        let s2 = img.find_sector(0, 0, 0xc2).unwrap();
        assert_eq!([s1.data[0], s1.data[1]], [0xa1, 0xa2]);
        assert_eq!([s2.data[0], s2.data[1]], [0xb1, 0xb2]);

        let mut fdc = Plus3Fdc::new();
        fdc.insert(img);
        assert!(fdc.read_sector(0, 0, 0xc1));
        assert_eq!(fdc.read_data_byte(), 0xa1);
        assert_eq!(fdc.read_data_byte(), 0xa2);
        assert!(fdc.read_sector(0, 0, 0xc2));
        assert_eq!(fdc.read_data_byte(), 0xb1);
        assert_eq!(fdc.read_data_byte(), 0xb2);
        assert!(!fdc.read_sector(0, 0, 0xc3), "missing sector id");
        assert_eq!(fdc.status, 0x10);
    }

    #[test]
    fn read_data_command_bytes_load_sector() {
        let img = DskImage::parse(&synthetic_dsk()).unwrap();
        let mut fdc = Plus3Fdc::new();
        fdc.insert(img);
        fdc.write_command_byte(0x06);
        fdc.write_command_byte(0);
        fdc.write_command_byte(0);
        fdc.write_command_byte(0xc1);
        fdc.write_command_byte(1);
        assert_eq!(fdc.main_status() & 0xc0, 0xc0);
        assert_eq!(fdc.read_data_byte(), 0x42);
        assert_eq!(fdc.read_data_byte(), 0x43);
    }
}
