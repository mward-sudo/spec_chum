//! Amstrad/Spectrum +3 `.DSK` / Extended DSK image reader.
//!
//! Sector lookup is by physical track (file order) plus CHRN. The µPD765
//! state machine lives in [`crate::Plus3Fdc`].

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

    fn track_index(&self, track: u8, side: u8) -> Option<usize> {
        if side >= self.sides {
            return None;
        }
        let idx = usize::from(track) * usize::from(self.sides) + usize::from(side);
        (idx < self.tracks_data.len()).then_some(idx)
    }

    /// Find sector by physical track/side/id, requiring CHRN C to match `track`.
    #[must_use]
    pub fn find_sector(&self, track: u8, side: u8, sector_id: u8) -> Option<&Sector> {
        self.track_index(track, side).and_then(|idx| {
            self.tracks_data[idx]
                .sectors
                .iter()
                .find(|s| s.track == track && s.side == side && s.sector_id == sector_id)
        })
    }

    /// Sector on a physical track matching id `R` (and side `H` when present).
    ///
    /// Unlike [`Self::find_sector`], CHRN C need not equal the physical cylinder —
    /// SYSTEM-format ids such as `0xC1` still resolve after SEEK to track 0.
    #[must_use]
    pub fn find_id(&self, physical_track: u8, side: u8, sector_id: u8) -> Option<&Sector> {
        let idx = self.track_index(physical_track, side)?;
        let secs = &self.tracks_data[idx].sectors;
        secs.iter()
            .find(|s| s.sector_id == sector_id && s.side == side)
            .or_else(|| secs.iter().find(|s| s.sector_id == sector_id))
    }

    /// Mutable [`Self::find_id`] for WRITE DATA.
    pub fn find_id_mut(
        &mut self,
        physical_track: u8,
        side: u8,
        sector_id: u8,
    ) -> Option<&mut Sector> {
        let idx = self.track_index(physical_track, side)?;
        let secs = &mut self.tracks_data[idx].sectors;
        if let Some(i) = secs
            .iter()
            .position(|s| s.sector_id == sector_id && s.side == side)
        {
            return secs.get_mut(i);
        }
        let i = secs.iter().position(|s| s.sector_id == sector_id)?;
        secs.get_mut(i)
    }

    /// First sector listed on a physical track/side (READ ID).
    #[must_use]
    pub fn first_sector(&self, physical_track: u8, side: u8) -> Option<&Sector> {
        let idx = self.track_index(physical_track, side)?;
        self.tracks_data[idx].sectors.first()
    }

    /// Replace sectors on a physical track (µPD765 FORMAT TRACK).
    pub fn format_track(
        &mut self,
        physical_track: u8,
        head: u8,
        fill: u8,
        entries: &[(u8, u8, u8, u8)],
    ) {
        let Some(idx) = self.track_index(physical_track, head) else {
            return;
        };
        self.tracks_data[idx].sectors = entries
            .iter()
            .map(|&(c, h, r, n)| {
                let size = 128usize << n.min(6);
                Sector {
                    track: c,
                    side: h,
                    sector_id: r,
                    size_code: n,
                    data: vec![fill; size],
                }
            })
            .collect();
    }

    /// Raw CPC DSK bytes: one track, one 256-byte sector (id `0xC1`, payload `0x42 0x43`).
    ///
    /// Shared test fixture — see [`Self::synthetic_one_sector`].
    #[must_use]
    pub fn synthetic_one_sector_bytes() -> Vec<u8> {
        let mut data = vec![0u8; 0x100];
        data[0..8].copy_from_slice(b"MV - CPC");
        data[0x30] = 1;
        data[0x31] = 1;
        let track_size: u16 = 0x100 + 256;
        data[0x32..0x34].copy_from_slice(&track_size.to_le_bytes());

        let mut track = vec![0u8; track_size as usize];
        track[0..12].copy_from_slice(b"Track-Info\r\n");
        track[0x10] = 0;
        track[0x11] = 0;
        track[0x14] = 1;
        track[0x15] = 1;
        track[0x18] = 0;
        track[0x19] = 0;
        track[0x1a] = 0xc1;
        track[0x1b] = 1;
        track[0x100] = 0x42;
        track[0x101] = 0x43;
        data.extend_from_slice(&track);
        data
    }

    /// Parsed [`Self::synthetic_one_sector_bytes`].
    #[must_use]
    pub fn synthetic_one_sector() -> Self {
        Self::parse(&Self::synthetic_one_sector_bytes()).expect("synthetic_one_sector fixture")
    }

    /// Raw CPC DSK bytes: one track, two 256-byte sectors (ids `0xC1`, `0xC2`).
    #[must_use]
    pub fn synthetic_two_sectors_bytes() -> Vec<u8> {
        let mut data = vec![0u8; 0x100];
        data[0..8].copy_from_slice(b"MV - CPC");
        data[0x30] = 1;
        data[0x31] = 1;
        let track_size: u16 = 0x100 + 256 * 2;
        data[0x32..0x34].copy_from_slice(&track_size.to_le_bytes());

        let mut track = vec![0u8; track_size as usize];
        track[0..12].copy_from_slice(b"Track-Info\r\n");
        track[0x10] = 0;
        track[0x11] = 0;
        track[0x14] = 1;
        track[0x15] = 2;
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

    /// Parsed [`Self::synthetic_two_sectors_bytes`].
    #[must_use]
    pub fn synthetic_two_sectors() -> Self {
        Self::parse(&Self::synthetic_two_sectors_bytes()).expect("synthetic_two_sectors fixture")
    }

    /// In-memory +3DOS 180K DATA disk: 40 tracks × 1 side × 9 × 512-byte sectors (ids 1–9).
    ///
    /// Track 0 sector 1 starts with the PCW/+3 10-byte spec; remaining directory
    /// bytes are `0xE5` (empty CP/M entries).
    #[must_use]
    pub fn synthetic_plus3_data() -> Self {
        const TRACKS: u8 = 40;
        const SPT: u8 = 9;
        const N: u8 = 2;
        let size = 128usize << N;
        let mut tracks_data = Vec::with_capacity(usize::from(TRACKS));
        for t in 0..TRACKS {
            let mut sectors = Vec::with_capacity(usize::from(SPT));
            for s in 1..=SPT {
                let mut data = vec![0xE5; size];
                if t == 0 && s == 1 {
                    data[..10].copy_from_slice(&[0x00, 0x00, 40, 9, 2, 1, 3, 2, 0x2A, 0x52]);
                }
                sectors.push(Sector {
                    track: t,
                    side: 0,
                    sector_id: s,
                    size_code: N,
                    data,
                });
            }
            tracks_data.push(TrackData { sectors });
        }
        Self {
            tracks: TRACKS,
            sides: 1,
            extended: false,
            tracks_data,
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_read_sector() {
        let img = DskImage::synthetic_one_sector();
        let sec = img.find_sector(0, 0, 0xc1).unwrap();
        assert_eq!(sec.data[0], 0x42);
        assert_eq!(sec.data[1], 0x43);
    }

    #[test]
    fn multi_sector_dsk_lookup() {
        let img = DskImage::synthetic_two_sectors();
        let s1 = img.find_sector(0, 0, 0xc1).unwrap();
        let s2 = img.find_sector(0, 0, 0xc2).unwrap();
        assert_eq!([s1.data[0], s1.data[1]], [0xa1, 0xa2]);
        assert_eq!([s2.data[0], s2.data[1]], [0xb1, 0xb2]);
        assert!(img.find_sector(0, 0, 0xc3).is_none());
    }

    #[test]
    fn find_id_matches_r_without_chrn_c() {
        let mut img = DskImage::synthetic_one_sector();
        img.tracks_data[0].sectors[0].track = 0xff;
        assert!(img.find_sector(0, 0, 0xc1).is_none());
        let sec = img.find_id(0, 0, 0xc1).unwrap();
        assert_eq!(sec.data[0], 0x42);
        img.find_id_mut(0, 0, 0xc1).unwrap().data[0] = 0x99;
        assert_eq!(img.find_id(0, 0, 0xc1).unwrap().data[0], 0x99);
    }

    #[test]
    fn synthetic_plus3_data_has_pcw_spec() {
        let img = DskImage::synthetic_plus3_data();
        assert_eq!(img.tracks, 40);
        assert_eq!(img.sides, 1);
        let sec = img.find_id(0, 0, 1).unwrap();
        assert_eq!(sec.size_code, 2);
        assert_eq!(
            &sec.data[..10],
            &[0x00, 0x00, 40, 9, 2, 1, 3, 2, 0x2A, 0x52]
        );
        assert_eq!(img.find_id(0, 0, 9).unwrap().data.len(), 512);
        assert!(img.first_sector(0, 0).is_some());
    }
}
