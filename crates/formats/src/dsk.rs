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

    /// Raw CPC DSK bytes: one track with an empty Track-Info header (no sectors).
    ///
    /// Shared test fixture for “parseable but empty” disks (insert / model-reject paths).
    #[must_use]
    pub fn synthetic_empty_track_bytes() -> Vec<u8> {
        let mut data = vec![0u8; 0x100];
        data[0..8].copy_from_slice(b"MV - CPC");
        data[0x30] = 1;
        data[0x31] = 1;
        let track_size: u16 = 0x100;
        data[0x32..0x34].copy_from_slice(&track_size.to_le_bytes());
        let mut track = vec![0u8; track_size as usize];
        track[0..12].copy_from_slice(b"Track-Info\r\n");
        data.extend_from_slice(&track);
        data
    }

    /// Parsed [`Self::synthetic_empty_track_bytes`].
    #[must_use]
    pub fn synthetic_empty_track() -> Self {
        Self::parse(&Self::synthetic_empty_track_bytes()).expect("synthetic_empty_track fixture")
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
    /// bytes are `0xE5` (empty CP/M entries). Not DOS_BOOT-able (checksum ≠ 3).
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
                    data[..10].copy_from_slice(&PLUS3_PCW_SPEC);
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

    /// Titled +3 disk that menu **Loader** boots via `DOS_BOOT` (commercial path).
    ///
    /// Layout matches the +3 manual (chapter 8 parts 26–27), DSKTOOL/VTAPE, and
    /// [zx3-drive-tester](https://github.com/corbym/zx3-drive-tester): track 0
    /// sector 1 sums to 3 (mod 256), entry at `FE10h`. The stub sets border 2
    /// and pokes `0xA5` at `0xFE20` (visible while special paging 4-5-6-3 is on).
    #[must_use]
    pub fn synthetic_plus3_boot_marker() -> Self {
        let mut img = Self::synthetic_plus3_data();
        let sec = img
            .find_id_mut(0, 0, 1)
            .expect("synthetic +3 DATA has track 0 sector 1");
        for b in &mut sec.data[10..] {
            *b = 0;
        }
        sec.data[0x10..0x10 + PLUS3_BOOT_MARKER_STUB.len()]
            .copy_from_slice(&PLUS3_BOOT_MARKER_STUB);
        set_plus3_boot_checksum(&mut sec.data);
        img
    }

    /// Non-bootable +3DOS DATA disk with a `DISK` BASIC program (`10 POKE 32768,165`).
    ///
    /// When `DOS_BOOT` rejects the sector (checksum ≠ 3), the +3 Loader falls back
    /// to `LOAD "DISK"` ([john_e](https://retrocomputing.stackexchange.com/questions/14574)).
    /// Directory sits on track 1 (PCW 180K `OFF=1`), same as zx3dsk / seasip XDPB.
    #[must_use]
    pub fn synthetic_plus3_disk_basic() -> Self {
        let mut img = Self::synthetic_plus3_data();
        let program = plus3_basic_poke_marker();
        let file = plus3dos_basic_file(&program, 10);
        let entry = cpm_dir_entry(b"DISK    ", b"   ", 2, file.len());
        img.write_data_sector(0, &entry);
        img.write_data_sector(4, &file);
        img
    }

    /// Write `bytes` at the start of a data-area sector (`OFF=1`, 9×512, ids 1–9).
    ///
    /// Index 0 is track 1 sector 1 (CP/M block 0); index 4 is block 2.
    fn write_data_sector(&mut self, data_sector_index: u8, bytes: &[u8]) {
        let (track, id) = plus3_cpm_chs(data_sector_index);
        if let Some(sec) = self.find_id_mut(track, 0, id) {
            let n = bytes.len().min(sec.data.len());
            sec.data[..n].copy_from_slice(&bytes[..n]);
        }
    }
}

/// PCW / Spectrum +3 180K spec (seasip XDPB; same bytes as a real +3 FORMAT).
const PLUS3_PCW_SPEC: [u8; 10] = [0x00, 0x00, 40, 9, 2, 1, 3, 2, 0x2A, 0x52];

/// `LD A,2 / OUT (254),A / LD A,0xA5 / LD (0xFE20),A / JR $`
const PLUS3_BOOT_MARKER_STUB: [u8; 11] = [
    0x3E, 0x02, 0xD3, 0xFE, 0x3E, 0xA5, 0x32, 0x20, 0xFE, 0x18, 0xFE,
];

fn set_plus3_boot_checksum(sector: &mut [u8]) {
    if sector.len() < 16 {
        return;
    }
    sector[15] = 0;
    let sum = sector.iter().fold(0u8, |a, b| a.wrapping_add(*b));
    sector[15] = 3u8.wrapping_sub(sum);
}

#[cfg(test)]
fn sector_checksum(data: &[u8]) -> u8 {
    data.iter().fold(0u8, |a, b| a.wrapping_add(*b))
}

/// Map a data-area sector index to (track, physical id) on a PCW 180K disk (`OFF=1`).
///
/// Index 0 = track 1 sector 1 (start of CP/M block 0). Two consecutive indices
/// make one 1K block (zx3dsk / seasip PCW 180K).
fn plus3_cpm_chs(data_sector_index: u8) -> (u8, u8) {
    let idx = u16::from(data_sector_index);
    let track = 1 + (idx / 9) as u8;
    let id = 1 + (idx % 9) as u8;
    (track, id)
}

fn plus3_basic_poke_marker() -> Vec<u8> {
    // 10 POKE 32768,165
    // 32768 does not fit the signed-int 0x0E form (`00 00 00 80 00` is -32768);
    // ZX float uses mantissa in [0.5, 1), so 32768 = 0.5×2^16 → `90 00 00 00 00`.
    vec![
        0x00, 0x0A, 0x17, 0x00, 0xF4, b'3', b'2', b'7', b'6', b'8', 0x0E, 0x90, 0x00, 0x00, 0x00,
        0x00, b',', b'1', b'6', b'5', 0x0E, 0x00, 0x00, 0xA5, 0x00, 0x00, 0x0D,
    ]
}

fn plus3dos_basic_file(program: &[u8], autostart_line: u16) -> Vec<u8> {
    let total = 128u32 + program.len() as u32;
    let mut hdr = [0u8; 128];
    hdr[0..8].copy_from_slice(b"PLUS3DOS");
    hdr[8] = 0x1A;
    hdr[9] = 1;
    hdr[11..15].copy_from_slice(&total.to_le_bytes());
    hdr[15] = 0; // BASIC
    hdr[16..18].copy_from_slice(&(program.len() as u16).to_le_bytes());
    hdr[18..20].copy_from_slice(&autostart_line.to_le_bytes());
    hdr[20..22].copy_from_slice(&(program.len() as u16).to_le_bytes());
    hdr[127] = hdr[..127].iter().fold(0u8, |a, b| a.wrapping_add(*b));
    let mut file = Vec::with_capacity(total as usize);
    file.extend_from_slice(&hdr);
    file.extend_from_slice(program);
    file
}

fn cpm_dir_entry(name8: &[u8; 8], ext3: &[u8; 3], block: u8, file_len: usize) -> [u8; 32] {
    let mut e = [0u8; 32];
    e[0] = 0;
    e[1..9].copy_from_slice(name8);
    e[9..12].copy_from_slice(ext3);
    let records = file_len.div_ceil(128).min(128) as u8;
    e[15] = records;
    e[16] = block;
    e
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
    fn synthetic_empty_track_parses_with_no_sectors() {
        let img = DskImage::synthetic_empty_track();
        assert_eq!(img.tracks, 1);
        assert_eq!(img.sides, 1);
        assert_eq!(img.tracks_data.len(), 1);
        assert!(img.tracks_data[0].sectors.is_empty());
        assert!(img.find_sector(0, 0, 0xc1).is_none());
    }

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
        assert_ne!(
            sector_checksum(&img.find_id(0, 0, 1).unwrap().data),
            3,
            "empty DATA disk must not look like a +3 bootstrap (checksum 3)"
        );
    }

    #[test]
    fn synthetic_plus3_boot_marker_checksum_is_3() {
        let img = DskImage::synthetic_plus3_boot_marker();
        let sec = img.find_id(0, 0, 1).unwrap();
        assert_eq!(sector_checksum(&sec.data), 3);
        assert_eq!(&sec.data[..10], &PLUS3_PCW_SPEC);
        assert_eq!(
            &sec.data[0x10..0x10 + PLUS3_BOOT_MARKER_STUB.len()],
            &PLUS3_BOOT_MARKER_STUB
        );
    }

    #[test]
    fn synthetic_plus3_disk_basic_has_plus3dos_disk_file() {
        let img = DskImage::synthetic_plus3_disk_basic();
        let dir = img.find_id(1, 0, 1).unwrap();
        assert_eq!(dir.data[0], 0);
        assert_eq!(&dir.data[1..9], b"DISK    ");
        assert_eq!(dir.data[16], 2, "first alloc block");
        let data = img.find_id(1, 0, 5).unwrap();
        assert_eq!(&data.data[0..8], b"PLUS3DOS");
        assert_eq!(data.data[15], 0, "BASIC type");
        // 32768 after 0x0E must be ZX float 0.5×2^16 (`90…`), not signed-int -32768.
        assert_eq!(
            &data.data[128 + 11..128 + 16],
            [0x90, 0x00, 0x00, 0x00, 0x00]
        );
        assert_ne!(sector_checksum(&img.find_id(0, 0, 1).unwrap().data), 3);
        assert_eq!(plus3_cpm_chs(0), (1, 1));
        assert_eq!(plus3_cpm_chs(4), (1, 5));
    }
}
