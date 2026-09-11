use super::*;
use crate::dsk::DskImage;

fn loaded(img: DskImage) -> Plus3Fdc {
    let mut fdc = Plus3Fdc::new();
    fdc.insert(img);
    fdc
}

fn feed_read_data(fdc: &mut Plus3Fdc, c: u8, h: u8, r: u8) {
    for b in [0x46, 0, c, h, r, 1, 0x09, 0x2a, 0xff] {
        fdc.write_command_byte(b);
    }
}

fn drain_result(fdc: &mut Plus3Fdc) -> Vec<u8> {
    assert_eq!(fdc.main_status(), MSR_RQM | MSR_DIO | MSR_CB);
    let mut out = Vec::new();
    while fdc.main_status() & MSR_CB != 0 {
        out.push(fdc.read_data_byte());
        if out.len() > 16 {
            break;
        }
    }
    out
}

#[test]
fn parse_and_read_sector() {
    let mut fdc = loaded(DskImage::synthetic_one_sector());
    assert!(fdc.read_sector(0, 0, 0xc1));
    assert_eq!(fdc.read_data_byte(), 0x42);
    assert_eq!(fdc.read_data_byte(), 0x43);
}

#[test]
fn read_data_command_stream_loads_sector() {
    let mut fdc = loaded(DskImage::synthetic_one_sector());
    feed_read_data(&mut fdc, 0, 0, 0xc1);
    assert!(fdc.data_remaining() > 0);
    assert_eq!(fdc.main_status() & 0xc0, 0xc0);
    assert_eq!(fdc.read_data_byte(), 0x42);
    assert_eq!(fdc.read_data_byte(), 0x43);
}

#[test]
fn multi_sector_dsk_read_two_sectors() {
    let mut fdc = loaded(DskImage::synthetic_two_sectors());
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
    let mut fdc = loaded(DskImage::synthetic_one_sector());
    for b in [0x06u8, 0, 0, 0, 0xc1, 1, 0x09, 0x2a, 0xff] {
        fdc.write_command_byte(b);
    }
    assert_eq!(fdc.main_status() & 0xc0, 0xc0);
    assert_eq!(fdc.read_data_byte(), 0x42);
    assert_eq!(fdc.read_data_byte(), 0x43);
}

#[test]
fn read_data_nine_byte_phase_selects_correct_sector() {
    let mut fdc = loaded(DskImage::synthetic_two_sectors());
    feed_read_data(&mut fdc, 0, 0, 0xc2);
    assert_eq!(fdc.read_data_byte(), 0xb1);
    assert_eq!(fdc.read_data_byte(), 0xb2);
}

#[test]
fn specify_returns_to_idle_without_result() {
    let mut fdc = Plus3Fdc::new();
    assert_eq!(fdc.main_status(), MSR_RQM);
    fdc.write_command_byte(0x03);
    assert_eq!(fdc.main_status(), MSR_RQM | MSR_CB);
    fdc.write_command_byte(0xaf);
    fdc.write_command_byte(0x03);
    assert_eq!(fdc.main_status(), MSR_RQM);
}

fn sis(fdc: &mut Plus3Fdc) -> Vec<u8> {
    fdc.write_command_byte(0x08);
    drain_result(fdc)
}

#[test]
fn seek_then_sis_returns_seek_end_and_pcn() {
    let mut fdc = Plus3Fdc::new();
    fdc.write_command_byte(0x0f);
    fdc.write_command_byte(0x00);
    fdc.write_command_byte(0x05);
    assert_eq!(fdc.main_status(), MSR_RQM);
    assert_eq!(fdc.pcn(0), 5);
    assert_eq!(sis(&mut fdc), vec![ST0_SE, 0x05]);
}

#[test]
fn recalibrate_sets_pcn_zero() {
    let mut fdc = Plus3Fdc::new();
    fdc.write_command_byte(0x0f);
    fdc.write_command_byte(0x00);
    fdc.write_command_byte(0x0c);
    let _ = sis(&mut fdc);
    fdc.write_command_byte(0x07);
    fdc.write_command_byte(0x00);
    assert_eq!(fdc.pcn(0), 0);
    assert_eq!(sis(&mut fdc), vec![ST0_SE, 0x00]);
}

#[test]
fn sense_drive_ready_depends_on_motor_and_disk() {
    let mut fdc = Plus3Fdc::new();
    fdc.write_command_byte(0x04);
    fdc.write_command_byte(0x00);
    let st3 = drain_result(&mut fdc)[0];
    assert_eq!(st3 & ST3_RY, 0, "not ready: no disk, motor off");
    assert_eq!(st3 & ST3_T0, ST3_T0);

    fdc.insert(DskImage::synthetic_plus3_data());
    fdc.write_command_byte(0x04);
    fdc.write_command_byte(0x00);
    let st3 = drain_result(&mut fdc)[0];
    assert_eq!(st3 & ST3_RY, 0, "not ready: motor still off");

    fdc.set_motor(true);
    fdc.write_command_byte(0x04);
    fdc.write_command_byte(0x00);
    let st3 = drain_result(&mut fdc)[0];
    assert_eq!(st3 & ST3_RY, ST3_RY);

    fdc.set_write_protect(true);
    fdc.write_command_byte(0x04);
    fdc.write_command_byte(0x00);
    let st3 = drain_result(&mut fdc)[0];
    assert_eq!(st3 & ST3_WP, ST3_WP);
}

#[test]
fn read_data_result_phase_en_when_r_equals_eot() {
    let mut fdc = loaded(DskImage::synthetic_one_sector());
    for b in [0x46u8, 0, 0, 0, 0xc1, 1, 0xc1, 0x2a, 0xff] {
        fdc.write_command_byte(b);
    }
    assert_eq!(fdc.main_status(), MSR_RQM | MSR_DIO | MSR_EXM | MSR_CB);
    for _ in 0..256 {
        let _ = fdc.read_data_byte();
    }
    let res = drain_result(&mut fdc);
    assert_eq!(res.len(), 7);
    assert_eq!(res[0], ST0_IC_ABNORMAL);
    assert_eq!(res[1], ST1_EN);
    assert_eq!(res[2], 0);
    assert_eq!(&res[3..7], &[0, 0, 0xc1, 1]);
    assert_eq!(fdc.main_status(), MSR_RQM);
}

#[test]
fn read_id_returns_chrn_of_first_sector() {
    let mut fdc = Plus3Fdc::new();
    fdc.insert(DskImage::synthetic_plus3_data());
    fdc.write_command_byte(0x0a);
    fdc.write_command_byte(0x00);
    let res = drain_result(&mut fdc);
    assert_eq!(res.len(), 7);
    assert_eq!(res[0] & 0xc0, 0, "normal termination");
    assert_eq!(&res[3..7], &[0, 0, 1, 2]); // C H R N of first DATA sector
}

#[test]
fn write_data_round_trip() {
    let mut fdc = loaded(DskImage::synthetic_one_sector());
    for b in [0x05u8, 0, 0, 0, 0xc1, 1, 0xc1, 0x2a, 0xff] {
        fdc.write_command_byte(b);
    }
    assert_eq!(fdc.main_status(), MSR_RQM | MSR_EXM | MSR_CB);
    fdc.write_command_byte(0xaa);
    fdc.write_command_byte(0xbb);
    for _ in 2..256 {
        fdc.write_command_byte(0x00);
    }
    let res = drain_result(&mut fdc);
    assert_eq!(res[1], ST1_EN);

    for b in [0x06u8, 0, 0, 0, 0xc1, 1, 0xc1, 0x2a, 0xff] {
        fdc.write_command_byte(b);
    }
    assert_eq!(fdc.read_data_byte(), 0xaa);
    assert_eq!(fdc.read_data_byte(), 0xbb);
}

#[test]
fn invalid_command_st0_0x80() {
    let mut fdc = Plus3Fdc::new();
    fdc.write_command_byte(0x11); // SCAN EQUAL — unsupported
    let res = drain_result(&mut fdc);
    assert_eq!(res, vec![ST0_IC_INVALID]);
    assert_eq!(fdc.main_status(), MSR_RQM);
}

/// SCAN* / READ TRACK stay unsupported: first command byte → ST0=`0x80`.
#[test]
fn unsupported_scan_and_read_track_opcodes_are_invalid() {
    // Base opcodes and MT (`0x40`) / SK (`0x20`) variants (masked to low 5 bits).
    const OPS: &[u8] = &[
        0x11, 0x31, 0x51, 0x71, // SCAN EQUAL
        0x19, 0x39, 0x59, 0x79, // SCAN LOW OR EQUAL
        0x1d, 0x3d, 0x5d, 0x7d, // SCAN HIGH OR EQUAL
        0x02, 0x22, 0x42, 0x62, // READ TRACK
    ];
    for &op in OPS {
        let mut fdc = loaded(DskImage::synthetic_one_sector());
        fdc.write_command_byte(op);
        let res = drain_result(&mut fdc);
        assert_eq!(res, vec![ST0_IC_INVALID], "op={op:#04x}");
        assert_eq!(fdc.main_status(), MSR_RQM, "op={op:#04x} idle after result");
        assert_eq!(fdc.format_count, 0);
        assert_eq!(fdc.read_count, 0);
        assert_eq!(fdc.write_count, 0);
    }
}

#[test]
fn sis_without_interrupt_is_invalid() {
    let mut fdc = Plus3Fdc::new();
    let _ = sis(&mut fdc);
    assert_eq!(sis(&mut fdc), vec![ST0_IC_INVALID]);
}

#[test]
fn write_protect_skips_execution() {
    let mut fdc = loaded(DskImage::synthetic_one_sector());
    fdc.set_write_protect(true);
    for b in [0x05u8, 0, 0, 0, 0xc1, 1, 0xc1, 0x2a, 0xff] {
        fdc.write_command_byte(b);
    }
    let res = drain_result(&mut fdc);
    assert_eq!(res[1] & ST1_NW, ST1_NW);
    assert_eq!(fdc.main_status(), MSR_RQM);
}

#[test]
fn format_track_replaces_sectors_on_disk() {
    let mut fdc = loaded(DskImage::synthetic_one_sector());
    // FORMAT TRACK: opcode, HD/US, N, SC, GPL, fill
    for b in [0x0du8, 0x00, 0x01, 0x02, 0x2a, 0xe5] {
        fdc.write_command_byte(b);
    }
    assert_eq!(fdc.main_status(), MSR_RQM | MSR_EXM | MSR_CB);
    // Two sectors: C H R N each
    for b in [0x00, 0x00, 0xc1, 0x01, 0x00, 0x00, 0xc2, 0x01] {
        fdc.write_command_byte(b);
    }
    let res = drain_result(&mut fdc);
    assert_eq!(res.len(), 7);
    assert_eq!(res[0] & 0xc0, 0, "normal termination");
    assert_eq!(res[3..7], [0x00, 0x00, 0xc2, 0x01]);
    assert_eq!(fdc.format_count, 1);

    feed_read_data(&mut fdc, 0, 0, 0xc1);
    assert_eq!(fdc.read_data_byte(), 0xe5);
    assert_eq!(fdc.read_data_byte(), 0xe5);
    feed_read_data(&mut fdc, 0, 0, 0xc2);
    assert_eq!(fdc.read_data_byte(), 0xe5);
}

#[test]
fn format_track_write_protect_returns_nw() {
    let mut fdc = loaded(DskImage::synthetic_one_sector());
    fdc.set_write_protect(true);
    for b in [0x0du8, 0x00, 0x01, 0x01, 0x2a, 0xe5] {
        fdc.write_command_byte(b);
    }
    let res = drain_result(&mut fdc);
    assert_eq!(res[1] & ST1_NW, ST1_NW);
    assert_eq!(fdc.main_status(), MSR_RQM);
}

fn feed_format_track(fdc: &mut Plus3Fdc, sc: u8, ids: &[(u8, u8, u8, u8)]) {
    for b in [0x0du8, 0x00, 0x01, sc, 0x2a, 0xe5] {
        fdc.write_command_byte(b);
    }
    for &(c, h, r, n) in ids {
        for b in [c, h, r, n] {
            fdc.write_command_byte(b);
        }
    }
}

#[test]
fn format_track_no_image_returns_abnormal_nd() {
    let mut fdc = Plus3Fdc::new();
    feed_format_track(&mut fdc, 1, &[(0, 0, 0xc1, 1)]);
    let res = drain_result(&mut fdc);
    assert_eq!(res.len(), 7);
    assert_eq!(res[0] & ST0_IC_ABNORMAL, ST0_IC_ABNORMAL);
    assert_eq!(res[1] & ST1_ND, ST1_ND);
    assert_eq!(res[3..7], [0, 0, 0, 1]);
    assert_eq!(fdc.format_count, 0);
}

#[test]
fn format_track_out_of_range_returns_abnormal_nd() {
    let mut fdc = loaded(DskImage::synthetic_one_sector());
    fdc.write_command_byte(0x0f);
    fdc.write_command_byte(0x00);
    fdc.write_command_byte(99);
    let _ = sis(&mut fdc);
    feed_format_track(&mut fdc, 1, &[(99, 0, 0xc1, 1)]);
    let res = drain_result(&mut fdc);
    assert_eq!(res.len(), 7);
    assert_eq!(res[0] & ST0_IC_ABNORMAL, ST0_IC_ABNORMAL);
    assert_eq!(res[1] & ST1_ND, ST1_ND);
    assert_eq!(res[3..7], [99, 0, 0, 1]);
    assert_eq!(fdc.format_count, 0);
}
