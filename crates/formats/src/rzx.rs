//! RZX input recording and initial embedded snapshot parsing.

use std::io::Read;
use std::path::Path;

use flate2::read::ZlibDecoder;

use crate::error::FormatError;

const INPUT_BLOCK_HEADER_LEN: usize = 13;
const SNAPSHOT_BLOCK_HEADER_LEN: usize = 12;
const MAX_INPUT_BLOCK_BYTES: usize = 64 * 1024 * 1024;
const MAX_SNAPSHOT_BYTES: usize = 16 * 1024 * 1024;
const MAX_FRAMES: usize = 1_000_000;

/// One frame of recorded input (IORQ port reads / keyboard matrix bytes).
#[derive(Clone, Debug, Default)]
pub struct RzxFrame {
    /// Number of CPU instructions in this frame (informational).
    pub fetch_count: u16,
    /// Raw input bytes supplied for IN operations during the frame.
    pub inputs: Vec<u8>,
}

/// Initial snapshot embedded in an RZX recording.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RzxSnapshot {
    /// Snapshot format identifier from the RZX block, such as `SNA` or `Z80`.
    pub extension: String,
    /// Snapshot file contents, decompressed when the RZX block used zlib.
    pub data: Vec<u8>,
}

/// Loaded RZX recording with its input stream and optional initial snapshot.
#[derive(Clone, Debug, Default)]
pub struct RzxRecording {
    pub frames: Vec<RzxFrame>,
    /// The first embedded snapshot, available for replay initialization.
    pub snapshot: Option<RzxSnapshot>,
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
        let major = data[4];
        let minor = data[5];
        if major != 0 || minor > 0x0d {
            return Err(FormatError::Format(format!(
                "unsupported RZX version {major}.{minor:02x}"
            )));
        }

        let mut recording = Self::default();
        let mut input_started = false;
        let mut offset = 10usize;
        while offset < data.len() {
            if data.len() - offset < 5 {
                return Err(FormatError::Format("truncated RZX block header".into()));
            }
            let block_id = data[offset];
            let block_len = u32::from_le_bytes([
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
                data[offset + 4],
            ]) as usize;
            let Some(block_end) = offset.checked_add(block_len) else {
                return Err(FormatError::Format("RZX block length overflow".into()));
            };
            if block_len < 5 || block_end > data.len() {
                return Err(FormatError::Format(format!(
                    "bad RZX block length {block_len} at {offset}"
                )));
            }

            let body = &data[offset + 5..block_end];
            match block_id {
                0x30 => {
                    if input_started {
                        return Err(FormatError::Format(
                            "mid-recording RZX snapshots are not supported; only an initial embedded snapshot can be replayed".into(),
                        ));
                    }
                    if recording.snapshot.is_some() {
                        return Err(FormatError::Format(
                            "multiple initial RZX snapshots are not supported".into(),
                        ));
                    }
                    recording.snapshot = Some(parse_snapshot_block(body)?);
                }
                0x80 => {
                    input_started = true;
                    parse_input_block(body, &mut recording.frames)?;
                }
                _ => {} // Creator, security and unknown blocks are skipped by their length.
            }
            offset = block_end;
        }
        Ok(recording)
    }
}

fn parse_input_block(body: &[u8], frames: &mut Vec<RzxFrame>) -> Result<(), FormatError> {
    if body.len() < INPUT_BLOCK_HEADER_LEN {
        return Err(FormatError::Format("short RZX input block header".into()));
    }
    let frame_count = usize::try_from(read_u32(body, 0))
        .map_err(|_| FormatError::Format("RZX frame count does not fit this platform".into()))?;
    if frame_count > MAX_FRAMES.saturating_sub(frames.len()) {
        return Err(FormatError::Format(format!(
            "RZX frame count exceeds limit of {MAX_FRAMES}"
        )));
    }
    let flags = read_u32(body, 9);
    if flags & !0x03 != 0 {
        return Err(FormatError::Format(format!(
            "unsupported RZX input flags {flags:#010x}"
        )));
    }
    if flags & 0x01 != 0 {
        return Err(FormatError::Format(
            "protected RZX input blocks are not supported".into(),
        ));
    }

    let encoded = &body[INPUT_BLOCK_HEADER_LEN..];
    let decoded;
    let data = if flags & 0x02 != 0 {
        decoded = decompress_zlib(encoded, MAX_INPUT_BLOCK_BYTES, "RZX input block")?;
        decoded.as_slice()
    } else {
        encoded
    };

    let mut cursor = 0usize;
    let mut parsed = 0usize;
    while cursor < data.len() {
        if data.len() - cursor < 4 {
            return Err(FormatError::Format(
                "truncated RZX frame header in input block".into(),
            ));
        }
        let fetch_count = u16::from_le_bytes([data[cursor], data[cursor + 1]]);
        let input_count = u16::from_le_bytes([data[cursor + 2], data[cursor + 3]]) as usize;
        cursor += 4;
        if input_count == 0xffff {
            let inputs = frames
                .last()
                .map_or_else(Vec::new, |frame| frame.inputs.clone());
            frames.push(RzxFrame {
                fetch_count,
                inputs,
            });
        } else {
            let Some(end) = cursor.checked_add(input_count) else {
                return Err(FormatError::Format(
                    "RZX frame input length overflow".into(),
                ));
            };
            if end > data.len() {
                return Err(FormatError::Format("RZX frame inputs truncated".into()));
            }
            frames.push(RzxFrame {
                fetch_count,
                inputs: data[cursor..end].to_vec(),
            });
            cursor = end;
        }
        parsed += 1;
    }
    if parsed != frame_count {
        return Err(FormatError::Format(format!(
            "RZX input frame count mismatch: header says {frame_count}, found {parsed}"
        )));
    }
    Ok(())
}

fn parse_snapshot_block(body: &[u8]) -> Result<RzxSnapshot, FormatError> {
    if body.len() < SNAPSHOT_BLOCK_HEADER_LEN {
        return Err(FormatError::Format(
            "short RZX snapshot block header".into(),
        ));
    }
    let flags = read_u32(body, 0);
    if flags & !0x03 != 0 {
        return Err(FormatError::Format(format!(
            "unsupported RZX snapshot flags {flags:#010x}"
        )));
    }
    if flags & 0x01 != 0 {
        return Err(FormatError::Format(
            "external RZX snapshot descriptors are not supported".into(),
        ));
    }
    let extension_bytes = &body[4..8];
    let extension_len = extension_bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(extension_bytes.len());
    if !extension_bytes[..extension_len].is_ascii() {
        return Err(FormatError::Format(
            "RZX snapshot extension is not ASCII".into(),
        ));
    }
    let extension = std::str::from_utf8(&extension_bytes[..extension_len])
        .map_err(|_| FormatError::Format("RZX snapshot extension is not ASCII".into()))?
        .to_ascii_uppercase();
    if !matches!(extension.as_str(), "SNA" | "Z80") {
        return Err(FormatError::Format(format!(
            "unsupported embedded RZX snapshot format '{extension}' (supported: SNA, Z80)"
        )));
    }

    let expected_len = usize::try_from(read_u32(body, 8)).map_err(|_| {
        FormatError::Format("embedded RZX snapshot length does not fit this platform".into())
    })?;
    if expected_len > MAX_SNAPSHOT_BYTES {
        return Err(FormatError::Format(format!(
            "embedded RZX snapshot length {expected_len} exceeds limit of {MAX_SNAPSHOT_BYTES}"
        )));
    }
    let encoded = &body[SNAPSHOT_BLOCK_HEADER_LEN..];
    let data = if flags & 0x02 != 0 {
        let data = decompress_zlib(encoded, MAX_SNAPSHOT_BYTES, "embedded RZX snapshot")?;
        if data.len() != expected_len {
            return Err(FormatError::Format(format!(
                "embedded RZX snapshot length mismatch: expected {expected_len}, decompressed {} bytes",
                data.len()
            )));
        }
        data
    } else {
        if encoded.len() != expected_len {
            return Err(FormatError::Format(format!(
                "embedded RZX snapshot length mismatch: expected {expected_len}, found {}",
                encoded.len()
            )));
        }
        encoded.to_vec()
    };
    Ok(RzxSnapshot { extension, data })
}

fn decompress_zlib(data: &[u8], limit: usize, label: &str) -> Result<Vec<u8>, FormatError> {
    let decoder = ZlibDecoder::new(data);
    let mut output = Vec::new();
    decoder
        .take(limit as u64 + 1)
        .read_to_end(&mut output)
        .map_err(|error| FormatError::Format(format!("invalid compressed {label}: {error}")))?;
    if output.len() > limit {
        return Err(FormatError::Format(format!(
            "decompressed {label} exceeds limit of {limit} bytes"
        )));
    }
    Ok(output)
}

fn read_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
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
    use flate2::{write::ZlibEncoder, Compression};
    use std::io::Write;

    fn rzx_with_block(block_id: u8, body: &[u8]) -> Vec<u8> {
        let mut data = b"RZX!\x00\x0d\0\0\0\0".to_vec();
        data.push(block_id);
        data.extend_from_slice(&u32::try_from(body.len() + 5).unwrap().to_le_bytes());
        data.extend_from_slice(body);
        data
    }

    fn input_block(frames: &[(u16, &[u8])], compressed: bool) -> Vec<u8> {
        let mut frame_data = Vec::new();
        for &(fetch, inputs) in frames {
            frame_data.extend_from_slice(&fetch.to_le_bytes());
            frame_data.extend_from_slice(&(inputs.len() as u16).to_le_bytes());
            frame_data.extend_from_slice(inputs);
        }
        let payload = if compressed {
            let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
            encoder
                .write_all(&frame_data)
                .expect("encode frame fixture");
            encoder.finish().expect("finish frame fixture")
        } else {
            frame_data
        };
        let mut body = Vec::new();
        body.extend_from_slice(&(frames.len() as u32).to_le_bytes());
        body.push(0); // reserved
        body.extend_from_slice(&0u32.to_le_bytes()); // T-states at block start
        body.extend_from_slice(&(if compressed { 0x02u32 } else { 0 }).to_le_bytes());
        body.extend_from_slice(&payload);
        body
    }

    fn snapshot_block(extension: [u8; 4], snapshot: &[u8], compressed: bool) -> Vec<u8> {
        let payload = if compressed {
            let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
            encoder
                .write_all(snapshot)
                .expect("encode snapshot fixture");
            encoder.finish().expect("finish snapshot fixture")
        } else {
            snapshot.to_vec()
        };
        let mut body = Vec::new();
        body.extend_from_slice(&(if compressed { 0x02u32 } else { 0 }).to_le_bytes());
        body.extend_from_slice(&extension);
        body.extend_from_slice(&(snapshot.len() as u32).to_le_bytes());
        body.extend_from_slice(&payload);
        body
    }

    #[test]
    fn parse_uncompressed_input_frames_and_repeated_frame() {
        let body = input_block(&[(10, &[0x1f]), (5, &[0x00, 0x10])], false);
        let mut data = rzx_with_block(0x80, &body);
        let repeated = input_block(&[(3, &[])], false);
        // Replace the final frame with the RZX repeated-frame marker.
        let mut repeated = repeated;
        let marker = repeated.len() - 2;
        repeated[marker..].copy_from_slice(&u16::MAX.to_le_bytes());
        let second_block = rzx_with_block(0x80, &repeated);
        data.extend_from_slice(&second_block[10..]);

        let recording = RzxRecording::parse(&data).expect("parse RZX");
        assert_eq!(recording.frames.len(), 3);
        assert_eq!(recording.frames[0].inputs, [0x1f]);
        assert_eq!(recording.frames[1].inputs, [0x00, 0x10]);
        assert_eq!(recording.frames[2].fetch_count, 3);
        assert_eq!(recording.frames[2].inputs, [0x00, 0x10]);
    }

    #[test]
    fn parse_compressed_input_frames() {
        let recording = RzxRecording::parse(include_bytes!(
            "../../../tests/fixtures/rzx/compressed_inputs.rzx"
        ))
        .expect("parse compressed fixture");
        assert_eq!(recording.frames.len(), 2);
        assert_eq!(recording.frames[0].inputs, [0x21]);
        assert_eq!(recording.frames[1].inputs, [0x95]);
    }

    #[test]
    fn parse_uncompressed_and_compressed_initial_snapshots() {
        let snapshot = b"synthetic snapshot bytes";
        let body = snapshot_block(*b"Z80\0", snapshot, false);
        let recording =
            RzxRecording::parse(&rzx_with_block(0x30, &body)).expect("parse uncompressed snapshot");
        let embedded = recording.snapshot.expect("snapshot present");
        assert_eq!(embedded.extension, "Z80");
        assert_eq!(embedded.data, snapshot);

        let recording = RzxRecording::parse(include_bytes!(
            "../../../tests/fixtures/rzx/embedded_sna_compressed.rzx"
        ))
        .expect("parse compressed snapshot fixture");
        let embedded = recording.snapshot.expect("snapshot present");
        assert_eq!(embedded.extension, "SNA");
        assert_eq!(embedded.data.len(), 49_179);
        assert_eq!(embedded.data[27 + 0x4000], 0xaa);
        assert_eq!(recording.frames[0].inputs, [0x21]);

        let recording = RzxRecording::parse(include_bytes!(
            "../../../tests/fixtures/rzx/embedded_z80_compressed.rzx"
        ))
        .expect("parse compressed Z80 snapshot fixture");
        let embedded = recording.snapshot.expect("snapshot present");
        assert_eq!(embedded.extension, "Z80");
        assert_eq!(embedded.data.len(), 49_182);
        assert_eq!(embedded.data[30 + 0x1000], 0x42);
    }

    #[test]
    fn reject_truncated_lengths_and_unsupported_snapshot_positions() {
        let body = input_block(&[(10, &[0x21])], false);
        let mut truncated = rzx_with_block(0x80, &body);
        truncated.pop();
        assert!(RzxRecording::parse(&truncated)
            .expect_err("truncated block")
            .to_string()
            .contains("block length"));

        let mut data = rzx_with_block(0x80, &body);
        let snapshot = snapshot_block(*b"SNA\0", b"snap", false);
        data.extend_from_slice(&rzx_with_block(0x30, &snapshot)[10..]);
        assert!(RzxRecording::parse(&data)
            .expect_err("midstream snapshot")
            .to_string()
            .contains("mid-recording RZX snapshots"));
    }

    #[test]
    fn reject_mismatched_frame_count_and_truncated_frame_input() {
        let mut body = input_block(&[(10, &[0x21])], false);
        body[0..4].copy_from_slice(&2u32.to_le_bytes());
        assert!(RzxRecording::parse(&rzx_with_block(0x80, &body))
            .expect_err("mismatched count")
            .to_string()
            .contains("frame count mismatch"));

        let mut body = input_block(&[(10, &[0x21])], false);
        body[INPUT_BLOCK_HEADER_LEN + 2..INPUT_BLOCK_HEADER_LEN + 4]
            .copy_from_slice(&2u16.to_le_bytes());
        assert!(RzxRecording::parse(&rzx_with_block(0x80, &body))
            .expect_err("truncated frame input")
            .to_string()
            .contains("frame inputs truncated"));
    }

    #[test]
    fn reject_unsupported_snapshot_format_and_bad_zlib_stream() {
        let body = snapshot_block(*b"SZX\0", b"snapshot", false);
        assert!(RzxRecording::parse(&rzx_with_block(0x30, &body))
            .expect_err("unsupported snapshot format")
            .to_string()
            .contains("supported: SNA, Z80"));

        let mut body = input_block(&[(10, &[0x21])], true);
        *body.last_mut().expect("compressed byte") ^= 0xff;
        assert!(RzxRecording::parse(&rzx_with_block(0x80, &body))
            .expect_err("invalid zlib stream")
            .to_string()
            .contains("invalid compressed RZX input block"));
    }

    #[test]
    fn zlib_output_is_bounded() {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&[0u8; 33]).expect("write test payload");
        let compressed = encoder.finish().expect("finish test payload");
        let error = decompress_zlib(&compressed, 32, "test payload").expect_err("limit applies");
        assert!(error.to_string().contains("exceeds limit"));
    }

    #[test]
    fn apply_matrix_and_kempston() {
        let mut rows = [0x1f; 8];
        let mut kemp = 0u8;
        apply_input_byte(0x21, &mut rows, |value| kemp = value);
        assert_eq!(rows[1], 0x01);
        apply_input_byte(0x95, &mut rows, |value| kemp = value);
        assert_eq!(kemp, 0x15);
    }
}
