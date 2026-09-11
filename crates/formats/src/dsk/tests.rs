//! DSK parse / synthetic fixture unit tests.

use super::synthetic::{plus3_cpm_chs, sector_checksum, PLUS3_BOOT_MARKER_STUB, PLUS3_PCW_SPEC};
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

#[test]
fn extended_dsk_rejects_track_table_overflow() {
    let mut data = vec![0u8; 0x100];
    data[0..8].copy_from_slice(b"EXTENDED");
    // 205 tracks × 1 side exceeds the 204-byte size table at 0x34..0x100.
    data[0x30] = 205;
    data[0x31] = 1;
    let err = DskImage::parse(&data).expect_err("oversized extended table");
    assert!(
        err.to_string().contains("extended track table overflow"),
        "got {err}"
    );
}
