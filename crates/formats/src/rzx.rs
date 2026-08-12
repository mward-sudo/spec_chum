//! Minimal RZX input replay (snapshot + input frames).
//!
//! Supports the common RZX layout used by Fuse-compatible recordings:
//! signature `RZX!`, block types 0x01 (creator), 0x0A (security?), 0x30 (snapshot),
//! 0x80 (input recording). Only uncompressed input frames are applied.

use std::path::Path;

use crate::FormatError;

/// One frame of recorded input (IORQ port reads / keyboard matrix bytes).
#[derive(Clone, Debug, Default)]
pub struct RzxFrame {
    /// Number of CPU instructions in this frame (informational).
    pub fetch_count: u16,
    /// Raw input bytes supplied for IN operations during the frame.
    pub inputs: Vec<u8>,
}

/// Loaded RZX recording (input stream only; embedded snapshots ignored for now).
#[derive(Clone, Debug, Default)]
pub struct RzxRecording {
    pub frames: Vec<RzxFrame>,
}

impl RzxRecording {
    pub fn load(path: &Path) -> Result<Self, FormatError> {
        let data = std::fs::read(path).map_err(FormatError::Io)?;
        Self::parse(&data)
    }

    pub fn parse(data: &[u8]) -> Result<Self, FormatError> {
        if data.len() < 10 || &data[0..4] != b"RZX!" {
            return Err(FormatError::Format("missing RZX signature".into()));
        }
        let mut frames = Vec::new();
        let mut i = 10usize; // skip signature + ver + flags + reserved
        while i < data.len() {
            let block_id = data[i];
            if i + 5 > data.len() {
                break;
            }
            let block_len =
                u32::from_le_bytes([data[i + 1], data[i + 2], data[i + 3], data[i + 4]]) as usize;
            if block_len < 5 || i + block_len > data.len() {
                return Err(FormatError::Format(format!(
                    "bad RZX block length {block_len} at {i}"
                )));
            }
            let body = &data[i + 5..i + block_len];
            match block_id {
                0x80 => {
                    // Input recording block
                    if body.len() < 5 {
                        return Err(FormatError::Format("short input block".into()));
                    }
                    let flags = body[4];
                    if flags & 0x02 != 0 {
                        return Err(FormatError::Format(
                            "compressed RZX input not supported".into(),
                        ));
                    }
                    let mut p = 5usize;
                    while p + 4 <= body.len() {
                        let fetch = u16::from_le_bytes([body[p], body[p + 1]]);
                        let in_count = u16::from_le_bytes([body[p + 2], body[p + 3]]) as usize;
                        p += 4;
                        if in_count == 0xffff {
                            // Repeated frame — copy previous inputs
                            let prev: RzxFrame = frames.last().cloned().unwrap_or_default();
                            frames.push(RzxFrame {
                                fetch_count: fetch,
                                inputs: prev.inputs,
                            });
                            continue;
                        }
                        if p + in_count > body.len() {
                            return Err(FormatError::Format("RZX frame inputs truncated".into()));
                        }
                        let inputs = body[p..p + in_count].to_vec();
                        p += in_count;
                        frames.push(RzxFrame {
                            fetch_count: fetch,
                            inputs,
                        });
                    }
                }
                0x30 | 0x01 | 0x02 | 0x0A | 0x0B => {
                    // snapshot / creator / security — skip
                }
                _ => {
                    // Unknown — skip by length
                }
            }
            i += block_len;
        }
        Ok(Self { frames })
    }
}

/// Applies RZX keyboard-style input bytes onto an 8-row matrix (active-low).
///
/// Convention: bit7 clear → matrix poke (`row = bits5–6`, keys = bits0–4);
/// bit7 set → Kempston bits0–4 (right/left/down/up/fire).
pub fn apply_input_byte(byte: u8, keyboard_rows: &mut [u8; 8], mut set_kempston: impl FnMut(u8)) {
    if byte & 0x80 != 0 {
        set_kempston(byte & 0x1f);
    } else {
        let row = usize::from((byte >> 5) & 7);
        keyboard_rows[row] = byte & 0x1f;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_rzx(frames: &[(u16, &[u8])]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"RZX!");
        v.extend_from_slice(&[0x00, 0x0d]); // ver
        v.extend_from_slice(&[0, 0, 0, 0]); // flags/reserved
                                            // Input block 0x80
        let mut body = Vec::new();
        body.extend_from_slice(&[0, 0, 0, 0]); // frame count at start — Fuse puts tstates etc.
        body.push(0); // flags uncompressed
        for &(fetch, inputs) in frames {
            body.extend_from_slice(&fetch.to_le_bytes());
            body.extend_from_slice(&(inputs.len() as u16).to_le_bytes());
            body.extend_from_slice(inputs);
        }
        let block_len = (5 + body.len()) as u32;
        v.push(0x80);
        v.extend_from_slice(&block_len.to_le_bytes());
        v.extend_from_slice(&body);
        v
    }

    #[test]
    fn parse_input_frames() {
        let data = minimal_rzx(&[(10, &[0x1f]), (5, &[0x00, 0x10])]);
        let r = RzxRecording::parse(&data).unwrap();
        assert_eq!(r.frames.len(), 2);
        assert_eq!(r.frames[0].inputs, vec![0x1f]);
        assert_eq!(r.frames[1].inputs, vec![0x00, 0x10]);
    }

    #[test]
    fn apply_matrix_and_kempston() {
        let mut rows = [0x1f; 8];
        let mut kemp = 0u8;
        apply_input_byte(0x21, &mut rows, |v| kemp = v); // row 1, keys 0x01
        assert_eq!(rows[1], 0x01);
        apply_input_byte(0x95, &mut rows, |v| kemp = v); // bit7 → kempston 0x15
        assert_eq!(kemp, 0x15);
    }
}
