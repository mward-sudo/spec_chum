//! Amstrad/Spectrum +3 `.DSK` / Extended DSK image reader.
//!
//! Sector lookup is by physical track (file order) plus `CHRN`. The `µPD765`
//! state machine lives in [`crate::Plus3Fdc`].
//!
//! Layout (#409 cohesion split): [`synthetic`] (test/fixture builders),
//! [`tests`]. Parse/lookup behaviour is unchanged from the former single file.

mod synthetic;

#[cfg(test)]
mod tests;

use std::path::Path;

use crate::error::FormatError;

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

    /// Replace sectors on a physical track (`µPD765` FORMAT TRACK).
    ///
    /// Returns `false` when `physical_track` / `head` are out of range.
    #[must_use]
    pub fn format_track(
        &mut self,
        physical_track: u8,
        head: u8,
        fill: u8,
        entries: &[(u8, u8, u8, u8)],
    ) -> bool {
        let Some(idx) = self.track_index(physical_track, head) else {
            return false;
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
        true
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
        let sector_len = 128usize << size_code.min(6);
        if data_off + sector_len > data.len() {
            return Err(FormatError::Format("sector data truncated".into()));
        }
        let sector_data = data[data_off..data_off + sector_len].to_vec();
        data_off += sector_len;
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
