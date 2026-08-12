//! Spec Chum tape — TAP loading via EAR bitstream.

#![allow(clippy::pedantic)]

use std::path::Path;

#[derive(Clone, Debug)]
pub struct TapImage {
    pub blocks: Vec<Vec<u8>>,
}

#[derive(Debug)]
pub enum TapeError {
    Io(std::io::Error),
    Format(String),
}

impl std::fmt::Display for TapeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Format(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for TapeError {}

impl TapImage {
    pub fn load(path: &Path) -> Result<Self, TapeError> {
        let data = std::fs::read(path).map_err(TapeError::Io)?;
        Self::parse(&data)
    }

    pub fn parse(mut data: &[u8]) -> Result<Self, TapeError> {
        let mut blocks = Vec::new();
        while !data.is_empty() {
            if data.len() < 2 {
                return Err(TapeError::Format("truncated TAP length".into()));
            }
            let len = u16::from_le_bytes([data[0], data[1]]) as usize;
            data = &data[2..];
            if data.len() < len {
                return Err(TapeError::Format("truncated TAP block".into()));
            }
            blocks.push(data[..len].to_vec());
            data = &data[len..];
        }
        Ok(Self { blocks })
    }
}

/// Generates EAR levels for a TAP block (simplified ROM timing).
#[derive(Clone, Debug)]
pub struct TapPlayer {
    pub image: TapImage,
    pub block: usize,
    /// Pulse schedule: remaining T-states at current level, then flip.
    pulses: Vec<(u32, bool)>,
    pulse_i: usize,
    remain: u32,
    level: bool,
}

impl TapPlayer {
    #[must_use]
    pub fn new(image: TapImage) -> Self {
        let mut p = Self {
            image,
            block: 0,
            pulses: Vec::new(),
            pulse_i: 0,
            remain: 0,
            level: false,
        };
        p.queue_block(0);
        p
    }

    fn queue_block(&mut self, idx: usize) {
        self.pulses.clear();
        self.pulse_i = 0;
        self.remain = 0;
        let Some(block) = self.image.blocks.get(idx) else {
            return;
        };
        // Pilot: 2168 T, 8063/3223 pulses depending on flag
        let flag = block.first().copied().unwrap_or(0);
        let pilot_count = if flag == 0 { 8063 } else { 3223 };
        for _ in 0..pilot_count {
            self.pulses.push((2168, true));
            self.pulses.push((2168, false));
        }
        // Sync
        self.pulses.push((667, true));
        self.pulses.push((735, false));
        // Data bits
        for &byte in block {
            for bit in (0..8).rev() {
                let one = byte & (1 << bit) != 0;
                let len = if one { 1710 } else { 855 };
                self.pulses.push((len, true));
                self.pulses.push((len, false));
            }
        }
        // pause
        self.pulses.push((3_500_000, false));
        if let Some(&(r, l)) = self.pulses.first() {
            self.remain = r;
            self.level = l;
        }
    }

    /// Advance `dt` T-states; returns current EAR level.
    pub fn advance(&mut self, mut dt: u32) -> bool {
        while dt > 0 && self.pulse_i < self.pulses.len() {
            if self.remain == 0 {
                self.pulse_i += 1;
                if self.pulse_i >= self.pulses.len() {
                    self.block += 1;
                    self.queue_block(self.block);
                    break;
                }
                let (r, l) = self.pulses[self.pulse_i];
                self.remain = r;
                self.level = l;
            }
            let step = dt.min(self.remain);
            self.remain -= step;
            dt -= step;
        }
        self.level
    }
}

/// Flash-load: poke TAP block into memory via LD-BYTES trap (0x056C).
pub fn flash_load_block(machine_ram_write: &mut dyn FnMut(u16, u8), block: &[u8], addr: u16) {
    for (i, b) in block.iter().enumerate() {
        machine_ram_write(addr.wrapping_add(i as u16), *b);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty() {
        let t = TapImage::parse(&[]).unwrap();
        assert!(t.blocks.is_empty());
    }

    #[test]
    fn parse_one_block() {
        let raw = vec![0x03, 0x00, 0x00, 0x11, 0x22];
        let t = TapImage::parse(&raw).unwrap();
        assert_eq!(t.blocks.len(), 1);
        assert_eq!(t.blocks[0], vec![0x00, 0x11, 0x22]);
        // checksum-ish not validated
        let _ = raw;
    }
}
