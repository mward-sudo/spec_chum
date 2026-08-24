//! Beta Disk Interface / TR-DOS: VG93-ish status/track/sector/data registers.
//!
//! Ports (low byte): `0x1F` cmd/status, `0x3F` track, `0x5F` sector, `0x7F` data,
//! `0xFF` system. A Type-II Read Sector command (`0x80`–`0x9F`) loads one TRD
//! sector into the data buffer for port reads.

use formats::{TrdImage, TRD_SECTOR_SIZE};

#[derive(Clone, Debug)]
pub struct BetaDisk {
    pub track: u8,
    pub sector: u8,
    pub status: u8,
    pub system: u8,
    /// When true, TR-DOS is paged and Beta claims ports `1Fh`/`3Fh`/`5Fh`/`7Fh`/`FFh`.
    pub paged: bool,
    buffer: Vec<u8>,
    buffer_i: usize,
    pub image: Option<TrdImage>,
}

impl Default for BetaDisk {
    fn default() -> Self {
        Self::new()
    }
}

impl BetaDisk {
    #[must_use]
    pub fn new() -> Self {
        Self {
            track: 0,
            sector: 0,
            status: 0,
            system: 0,
            paged: false,
            buffer: Vec::new(),
            buffer_i: 0,
            image: None,
        }
    }

    pub fn insert(&mut self, image: TrdImage) {
        self.image = Some(image);
    }

    /// Page / hide TR-DOS (enables Beta port decode when true).
    pub fn page_trdos(&mut self, on: bool) {
        self.paged = on;
    }

    /// `OUT` to Beta ports. Returns true if handled.
    pub fn out_port(&mut self, port: u16, value: u8) -> bool {
        if !self.paged {
            return false;
        }
        match port & 0xff {
            0x1f => {
                self.write_command(value);
                true
            }
            0x3f => {
                self.track = value;
                true
            }
            0x5f => {
                self.sector = value;
                true
            }
            0x7f => {
                // Data writes ignored in read-only foundational path.
                true
            }
            0xff => {
                self.system = value;
                true
            }
            _ => false,
        }
    }

    /// `IN` from Beta ports.
    pub fn in_port(&mut self, port: u16) -> Option<u8> {
        if !self.paged {
            return None;
        }
        match port & 0xff {
            0x1f => Some(self.status),
            0x3f => Some(self.track),
            0x5f => Some(self.sector),
            0x7f => Some(self.read_data()),
            0xff => Some(self.system),
            _ => None,
        }
    }

    fn write_command(&mut self, cmd: u8) {
        // Type II read sector: 100xxxxx
        if cmd & 0xe0 == 0x80 {
            let _ = self.load_sector_to_buffer();
        } else {
            // Force Interrupt / other: clear busy
            self.status = 0;
        }
    }

    /// Latch track / sector then load sector into the data buffer.
    pub fn load_sector_to_buffer(&mut self) -> bool {
        let Some(img) = self.image.as_ref() else {
            self.status = 0x80; // not ready
            return false;
        };
        // VG93 sector register is typically 1-based; accept 0 as sector 0 for tests.
        let sec_idx = if self.sector == 0 {
            0
        } else {
            usize::from(self.sector.saturating_sub(1))
        };
        let Some(sec) = img.read_sector(usize::from(self.track), sec_idx) else {
            self.status = 0x10; // record not found
            self.buffer.clear();
            self.buffer_i = 0;
            return false;
        };
        self.buffer = sec.to_vec();
        self.buffer_i = 0;
        self.status = 0x02; // DRQ
        debug_assert_eq!(self.buffer.len(), TRD_SECTOR_SIZE);
        true
    }

    pub fn read_data(&mut self) -> u8 {
        if self.buffer_i >= self.buffer.len() {
            self.status &= !0x02;
            return 0xff;
        }
        let v = self.buffer[self.buffer_i];
        self.buffer_i += 1;
        if self.buffer_i >= self.buffer.len() {
            self.status &= !0x02;
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use formats::TRD_SECTORS_PER_TRACK;

    #[test]
    fn port_protocol_reads_sector() {
        let mut raw = vec![0u8; TRD_SECTOR_SIZE * TRD_SECTORS_PER_TRACK];
        raw[0] = 0x12;
        raw[1] = 0x34;
        let img = TrdImage::parse(&raw).unwrap();
        let mut beta = BetaDisk::new();
        beta.insert(img);
        beta.page_trdos(true);
        beta.out_port(0x003f, 0); // track
        beta.out_port(0x005f, 1); // sector 1 → index 0
        beta.out_port(0x001f, 0x80); // read sector
        assert_eq!(beta.in_port(0x001f), Some(0x02)); // DRQ
        assert_eq!(beta.in_port(0x007f), Some(0x12));
        assert_eq!(beta.in_port(0x007f), Some(0x34));
    }
}
