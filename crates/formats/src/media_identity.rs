//! Content-derived media identity for tape (and later disk) display titles.
//!
//! # Hash algorithm
//!
//! **SHA-512 of the raw file bytes** (no header stripping, no payload
//! normalisation). Digests are lowercase hex (128 characters). This matches
//! the [ZXInfo API](https://api.zxinfo.dk/v3/) `GET /filecheck/{hash}` contract
//! (MD5 length 32 or SHA-512 length 128).
//!
//! # Lookup source (v1)
//!
//! Offline resolution uses an **embedded local catalogue** of metadata-only
//! entries (hash → human title). No ROM or tape images are shipped. Misses fall
//! back to the filesystem basename. Optional live `ZXInfo` lookup lives above
//! this crate (`host_api::media_title_lookup`, [#373](https://github.com/mward-sudo/spec_chum/issues/373))
//! and is not required for core emulation.
//!
//! See [#366](https://github.com/mward-sudo/spec_chum/issues/366) / [#373](https://github.com/mward-sudo/spec_chum/issues/373).

use std::path::Path;

use sha2::{Digest, Sha512};

use crate::FormatError;

/// Where a display title came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaTitleSource {
    /// Hit in the embedded local catalogue.
    LocalCatalogue,
    /// Hit in the on-disk `ZXInfo` / prior-lookup cache (host layer).
    Cached,
    /// Live `ZXInfo` `filecheck` hit (host layer; opt-in).
    Online,
    /// No catalogue/cache/online hit — basename (or full path fallback).
    Filename,
}

/// Resolved identity for an inserted media file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaIdentity {
    /// Lowercase hex SHA-512 of the raw file bytes.
    pub sha512_hex: String,
    /// Title suitable for chrome / status (catalogue hit or filename).
    pub display_title: String,
    pub source: MediaTitleSource,
}

/// SHA-512 of `bytes` as lowercase hex (128 chars).
#[must_use]
pub fn sha512_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha512::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

/// Look up a known SHA-512 in the embedded catalogue.
#[must_use]
pub fn catalogue_title(sha512_hex: &str) -> Option<&'static str> {
    LOCAL_CATALOGUE
        .iter()
        .find(|(hash, _)| *hash == sha512_hex)
        .map(|(_, title)| *title)
}

/// Resolve identity from already-read file bytes.
#[must_use]
pub fn identify_bytes(bytes: &[u8], path: &Path) -> MediaIdentity {
    let sha512_hex = sha512_hex(bytes);
    if let Some(title) = catalogue_title(&sha512_hex) {
        return MediaIdentity {
            sha512_hex,
            display_title: title.to_string(),
            source: MediaTitleSource::LocalCatalogue,
        };
    }
    MediaIdentity {
        sha512_hex,
        display_title: filename_fallback(path),
        source: MediaTitleSource::Filename,
    }
}

/// Read `path` and resolve identity (I/O errors propagate).
pub fn identify_path(path: &Path) -> Result<MediaIdentity, FormatError> {
    let bytes = std::fs::read(path).map_err(FormatError::Io)?;
    Ok(identify_bytes(&bytes, path))
}

fn filename_fallback(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| path.display().to_string())
}

/// Embedded metadata-only catalogue (hash → title). Expand as fixtures / known
/// redistributable images are added. Never store copyrighted tape payloads.
const LOCAL_CATALOGUE: &[(&str, &str)] = &[
    // tests/fixtures/tape/print_ok.tap
    (
        "1819ed84fc12189c6b862fcb297e6dfcd9c2f6f27db099d868a35b4627cb000e1b8386c67e3aff8498b94a7f30a9cbd1d538a3e6a283d6afa838ba0318c3b92a",
        "PRINT \"OK\"",
    ),
    // tests/fixtures/tape/attr_mark.tap
    (
        "a7beab321d9b3e40b1e6f5f19044cd8a5c1ac304c4812d2309a0259c45930ca58cd0cb14e64303c50ebc879d3849815ad228f4a3d8c356889f2869a90f98aa3b",
        "Attr mark",
    ),
    // tests/fixtures/tape/minimal_code.tap
    (
        "b5bd2ff5915075eeb40dec62df399a2ae3ede3860236e920ef385ac70122edef643ee54f0e61387108adcc301f85f9a4f886a78a69f2139ece92f73a1cae4c73",
        "Minimal CODE",
    ),
    // tests/fixtures/tape/custom_loader.tap
    (
        "9d1ad983d2d67bafbdd19f448081229eb8186be31563bbed7ed5afb90a0707868f0d4f725ad589b5878cbc0b347912ce4b5e707357d392ee4ce994660b570a87",
        "Custom loader",
    ),
    // tests/fixtures/tape/minimal.tzx
    (
        "3734cb6a7e9a829ff13a8386dedc80aafbd8f5102aa3e0186389ccf145543d33709a6251d810467e7e57d95b5108d10c91995b13ea3fd7e3aaf2c06c2e2554d6",
        "Minimal TZX",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/tape")
            .join(name)
    }

    #[test]
    fn print_ok_resolves_catalogue_title() {
        let path = fixture("print_ok.tap");
        let id = identify_path(&path).expect("read fixture");
        assert_eq!(id.display_title, "PRINT \"OK\"");
        assert_eq!(id.source, MediaTitleSource::LocalCatalogue);
        assert_eq!(id.sha512_hex.len(), 128);
    }

    #[test]
    fn unknown_bytes_fall_back_to_filename() {
        let path = Path::new("/tmp/noisy-download-name.tap");
        let id = identify_bytes(b"not-a-known-tape", path);
        assert_eq!(id.display_title, "noisy-download-name.tap");
        assert_eq!(id.source, MediaTitleSource::Filename);
        assert_eq!(id.sha512_hex.len(), 128);
    }

    #[test]
    fn all_fixture_catalogue_entries_match_disk() {
        for name in [
            "print_ok.tap",
            "attr_mark.tap",
            "minimal_code.tap",
            "custom_loader.tap",
            "minimal.tzx",
        ] {
            let id = identify_path(&fixture(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(
                id.source,
                MediaTitleSource::LocalCatalogue,
                "{name} should hit catalogue"
            );
            assert!(!id.display_title.is_empty(), "{name}");
        }
    }
}
