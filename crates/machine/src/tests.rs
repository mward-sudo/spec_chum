use super::*;
use std::path::PathBuf;
use tape::{tap_checksum, TapImage};
use ula::{FRAME_TSTATES_48, INT_LENGTH_48};
use z80::{Io, Memory};

fn rom48() -> Option<Vec<u8>> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/spec48.rom");
    std::fs::read(p).ok()
}

fn fixture_tap() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/tape/minimal_code.tap")
}

#[test]
fn multiface_in_pages_out_and_back_for_return() {
    let mut rom = [0u8; bus::MULTIFACE1_SIZE];
    rom[0x66] = 0x76; // HALT at NMI vector
    let mut m = Machine::new_48k(&[0u8; 16384]).unwrap();
    m.attach_multiface(&rom).unwrap();
    m.cpu_mut().regs.sp = 0xfffd;
    let _ = m.multiface_nmi().expect("MF attached");
    match &m {
        Machine::Spec48 { bus, .. } => {
            assert!(bus.multiface.as_ref().unwrap().paged);
        }
        _ => unreachable!(),
    }
    // Toolkit-style page out / page in without another button press.
    match &mut m {
        Machine::Spec48 { bus, .. } => {
            let _ = bus.in_port(0x001f);
            assert!(!bus.multiface.as_ref().unwrap().paged);
            let _ = bus.in_port(0x009f);
            assert!(bus.multiface.as_ref().unwrap().paged);
            assert_eq!(bus.read(0x0066), 0x76);
        }
        _ => unreachable!(),
    }
}

/// Synthetic MF ROM: at NMI vector, `LD A,42h / LD (2000h),A / HALT` — flag in MF RAM.
#[test]
fn multiface_nmi_executes_attached_rom() {
    let mut rom = [0u8; bus::MULTIFACE1_SIZE];
    // 0066: 3E 42       LD A,42h
    // 0068: 32 00 20    LD (2000h),A
    // 006B: 76          HALT
    rom[0x66] = 0x3e;
    rom[0x67] = 0x42;
    rom[0x68] = 0x32;
    rom[0x69] = 0x00;
    rom[0x6a] = 0x20;
    rom[0x6b] = 0x76;

    let mut m = Machine::new_48k(&[0u8; 16384]).unwrap();
    m.attach_multiface(&rom).unwrap();
    m.cpu_mut().regs.sp = 0xfffd;
    m.cpu_mut().regs.pc = 0x8000;

    let t = m.multiface_nmi().expect("MF attached");
    assert_eq!(t, 11);
    assert_eq!(m.cpu().regs.pc, 0x0066);
    match &m {
        Machine::Spec48 { bus, .. } => {
            assert!(bus.multiface.as_ref().unwrap().paged);
        }
        _ => unreachable!(),
    }

    // Run until HALT stores the flag.
    for _ in 0..8 {
        if m.cpu().regs.halted {
            break;
        }
        m.step_once();
    }
    assert!(m.cpu().regs.halted);
    assert_eq!(
        m.read_mem(0x2000),
        0x42,
        "NMI handler should have written flag to MF RAM"
    );
}

#[test]
fn peripheral_attach_rejects_unsupported_models_with_typed_errors() {
    let mut plus3 = Machine::new_plus3(&[0u8; 65536]).unwrap();
    assert!(matches!(
        plus3.attach_divmmc(),
        Err(DivMmcError::UnsupportedModel)
    ));
    assert!(matches!(
        plus3.attach_divmmc_sd_slot(0, vec![0u8; 512]),
        Err(DivMmcError::UnsupportedModel)
    ));
    assert!(matches!(
        plus3.attach_beta(),
        Err(BetaDiskError::UnsupportedModel)
    ));
    assert!(matches!(
        plus3.attach_interface1(),
        Err(Interface1Error::UnsupportedModel)
    ));

    let mut m128 = Machine::new_128k(&[0u8; 32768]).unwrap();
    m128.attach_multiface(&[0u8; bus::MULTIFACE128_SIZE])
        .expect("Multiface 128 attaches on 128K");
    assert!(m128.has_multiface());

    let mut plus2a = Machine::new_plus2a(&[0u8; 65536]).unwrap();
    assert!(matches!(
        plus2a.attach_multiface(&[0u8; bus::MULTIFACE128_SIZE]),
        Err(MultifaceError::UnsupportedModel)
    ));
}

#[test]
fn attach_divmmc_sd_slot_loads_both_and_rejects_invalid() {
    let mut m = Machine::new_48k(&[0u8; 16384]).unwrap();
    let mut slot0 = vec![0u8; 512];
    slot0[0] = 0xa0;
    let mut slot1 = vec![0u8; 512];
    slot1[0] = 0xa1;
    m.attach_divmmc_sd_slot(0, slot0).expect("slot 0");
    m.attach_divmmc_sd_slot(1, slot1).expect("slot 1");
    let div = m.divmmc_mut().expect("divmmc");
    assert_eq!(div.sd.first().copied(), Some(0xa0));
    assert_eq!(div.sd1.first().copied(), Some(0xa1));
    assert!(matches!(
        m.attach_divmmc_sd_slot(2, vec![0u8; 512]),
        Err(DivMmcError::InvalidSdSlot { slot: 2 })
    ));
}

/// Resolve user-supplied ESXDOS EEPROM (≥8 KiB). Never committed — see `docs/ROMS.md`.
fn esxdos_eeprom_path() -> Option<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    [
        "roms/divmmc/ESXMMC.BIN",
        "roms/divmmc/esxmmc.bin",
        "roms/esxdos.rom",
        "roms/divmmc.rom",
    ]
    .into_iter()
    .map(|rel| root.join(rel))
    .find(|p| p.is_file())
}

/// Flat FAT SD image with `/SYS` (and usually `/BIN`). Optional companion to the EEPROM.
fn esxdos_sd_path() -> Option<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    ["roms/divmmc/esxdos.img", "roms/divmmc/sd.img"]
        .into_iter()
        .map(|rel| root.join(rel))
        .find(|p| p.is_file())
}

fn bitmap_addr(col: u16, row: u16, scan: u16) -> u16 {
    0x4000 + (row / 8) * 2048 + (row % 8) * 32 + scan * 256 + col
}

/// Decode screen using the 48K ROM charset (not via `DivMMC` overlay at `$3D00`).
fn screen_text_from_rom_font(m: &Machine, rom: &[u8]) -> String {
    const FONT_OFF: usize = 0x3d00;
    let mut out = String::with_capacity(24 * 33);
    for row in 0..24u8 {
        for col in 0..32u8 {
            let mut glyph = [0u8; 8];
            for scan in 0..8u16 {
                glyph[scan as usize] =
                    m.read_mem(bitmap_addr(u16::from(col), u16::from(row), scan));
            }
            let ch = if glyph.iter().all(|&b| b == 0) {
                ' '
            } else {
                (32u8..=127)
                    .find(|&code| {
                        let base = FONT_OFF + usize::from(code - 32) * 8;
                        (0..8).all(|scan| {
                            let font = rom[base + scan];
                            glyph[scan] == font || glyph[scan] == !font
                        })
                    })
                    .map_or('?', |c| if c == 127 { '©' } else { char::from(c) })
            };
            out.push(ch);
        }
        out.push('\n');
    }
    out
}

/// Skip-clean when EEPROM missing. With EEPROM + flat FAT SD, boot until the
/// ESXDOS banner / prompt appears on the Spectrum screen.
///
/// Refs [#138](https://github.com/mward-sudo/spec_chum/issues/138).
#[test]
fn esxdos_eeprom_boots_prompt_when_fixtures_present() {
    let Some(sys) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let Some(eeprom_path) = esxdos_eeprom_path() else {
        eprintln!(
            "skip: ESXDOS EEPROM missing — place roms/divmmc/ESXMMC.BIN (see docs/ROMS.md / #138)"
        );
        return;
    };
    let Some(sd_path) = esxdos_sd_path() else {
        eprintln!(
                "skip: flat ESXDOS SD missing — place roms/divmmc/esxdos.img with /SYS (see docs/ROMS.md / #138)"
            );
        return;
    };

    let eeprom = std::fs::read(&eeprom_path).expect("read ESXDOS EEPROM");
    assert!(
        eeprom.len() >= 8192,
        "EEPROM too small: {} bytes at {}",
        eeprom.len(),
        eeprom_path.display()
    );
    let sd = std::fs::read(&sd_path).expect("read ESXDOS SD image");
    assert!(
        sd.len() >= 512,
        "SD image too small: {} bytes at {}",
        sd.len(),
        sd_path.display()
    );

    let mut m = Machine::new_48k(&sys).unwrap();
    m.attach_divmmc_eeprom(&eeprom).expect("attach EEPROM");
    m.attach_divmmc_sd(sd).expect("attach SD");
    assert!(m.has_divmmc_eeprom());

    // First opcode at reset is Spectrum DI; DivMMC then maps for operands.
    let rom0 = sys[0];
    assert_eq!(rom0, 0xf3, "48K ROM reset should be DI");
    m.step_cpu_only();
    assert!(
        m.divmmc_mut().is_some_and(|d| d.automap),
        "delayed automap should latch after reset M1"
    );
    // After the first instruction, PC should be in ESXDOS (LD SP / JP path).
    let pc = m.cpu().regs.pc;
    assert!(
        pc != 0x11cb && pc < 0x4000,
        "expected ESXDOS boot path after reset DI, pc={pc:#06x}"
    );

    let mut saw = String::new();
    for _ in 0..2_000u32 {
        let _ = m.run_frame();
        saw = screen_text_from_rom_font(&m, &sys);
        let lower = saw.to_ascii_lowercase();
        if lower.contains("esxdos")
            || lower.contains("v0.8")
            || saw.contains("Mounting")
            || saw.contains("Papaya")
        {
            break;
        }
    }
    let lower = saw.to_ascii_lowercase();
    assert!(
        lower.contains("esxdos")
            || lower.contains("v0.8")
            || saw.contains("Mounting")
            || saw.contains("Papaya"),
        "expected ESXDOS boot banner/prompt on screen; got:\n{saw}"
    );
}

#[test]
fn multiface128_nmi_pages_and_bf_3f_toggle() {
    let mut mf_rom = [0u8; bus::MULTIFACE128_SIZE];
    // 0066: 3E 42       LD A,42h
    // 0068: 32 00 20    LD (2000h),A
    // 006B: 76          HALT
    mf_rom[0x66] = 0x3e;
    mf_rom[0x67] = 0x42;
    mf_rom[0x68] = 0x32;
    mf_rom[0x69] = 0x00;
    mf_rom[0x6a] = 0x20;
    mf_rom[0x6b] = 0x76;

    let mut m = Machine::new_128k(&[0u8; 32768]).unwrap();
    m.attach_multiface(&mf_rom).unwrap();
    m.cpu_mut().regs.sp = 0xfffd;
    m.cpu_mut().regs.pc = 0x8000;

    // Stealth OFF: IN BFh must not page.
    match &mut m {
        Machine::Spec128 { bus, .. } => {
            assert_eq!(bus.in_port(0x00bf), 0xff);
            assert!(!bus.multiface.as_ref().unwrap().paged);
        }
        _ => unreachable!(),
    }

    let t = m.multiface_nmi().expect("MF128 attached");
    assert_eq!(t, 11);
    assert_eq!(m.cpu().regs.pc, 0x0066);
    match &m {
        Machine::Spec128 { bus, .. } => {
            assert!(bus.multiface.as_ref().unwrap().paged);
            assert!(bus.multiface.as_ref().unwrap().enabled);
        }
        _ => unreachable!(),
    }

    for _ in 0..8 {
        if m.cpu().regs.halted {
            break;
        }
        m.step_once();
    }
    assert!(m.cpu().regs.halted);
    assert_eq!(m.read_mem(0x2000), 0x42);

    match &mut m {
        Machine::Spec128 { bus, .. } => {
            assert_eq!(bus.in_port(0x003f), 0xff);
            assert!(!bus.multiface.as_ref().unwrap().paged);
            assert_eq!(bus.in_port(0x00bf), 0x7f);
            assert!(bus.multiface.as_ref().unwrap().paged);
            assert_eq!(bus.read(0x0066), 0x3e);
            bus.out_7ffd(0x08);
            assert_eq!(bus.in_port(0x00bf), 0xff, "screen bit set → D7 high");
        }
        _ => unreachable!(),
    }
}

#[test]
fn multiface128_real_rom_soak_skips_when_missing() {
    // User-supplied dump only (Romantic Robot; never redistributed). See docs/MULTIFACE.md.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let candidates = [
        root.join("roms/multiface/mf128.rom"),
        root.join("roms/mf128.rom"),
    ];
    let Some(path) = candidates.iter().find(|p| p.is_file()) else {
        eprintln!("skip: no roms/multiface/mf128.rom — place a user Multiface 128 dump to soak");
        return;
    };
    let data = std::fs::read(path).expect("read mf128");
    assert_eq!(data.len(), bus::MULTIFACE128_SIZE);
    let mut m = Machine::new_128k(&[0u8; 32768]).unwrap();
    m.attach_multiface(&data).unwrap();
    m.cpu_mut().regs.sp = 0xfffd;
    let _ = m.multiface_nmi().expect("NMI");
    match &m {
        Machine::Spec128 { bus, .. } => {
            assert!(bus.multiface.as_ref().unwrap().paged);
            assert!(bus.multiface.as_ref().unwrap().enabled);
            assert_ne!(bus.read(0x0066), 0x00);
        }
        _ => unreachable!(),
    }
}

fn synthetic_trd_with_marker(b0: u8, b1: u8) -> formats::TrdImage {
    let mut raw = vec![0u8; formats::TRD_SECTOR_SIZE * formats::TRD_SECTORS_PER_TRACK];
    raw[0] = b0;
    raw[1] = b1;
    formats::TrdImage::parse(&raw).unwrap()
}

/// TR-DOS-style `IN A,(#FF)` / `INI` loop at `USR 15616` (`0x3D00`).
fn trdos_read_sector_rom() -> [u8; bus::TRDOS_ROM_SIZE] {
    let mut rom = [0u8; bus::TRDOS_ROM_SIZE];
    let code: &[u8] = &[
        0x3e, 0x3c, // LD A,3Ch
        0xd3, 0xff, // OUT (FFh),A
        0xaf, // XOR A
        0xd3, 0x3f, // OUT (3Fh),A  track 0
        0x3e, 0x00, // LD A,0 — sector 0 (same size as old LD A,1 for jump targets)
        0xd3, 0x5f, // OUT (5Fh),A
        0x3e, 0x80, // LD A,80h
        0xd3, 0x1f, // OUT (1Fh),A
        0x21, 0x00, 0x40, // LD HL,4000h
        0x01, 0x7f, 0x00, // LD BC,007Fh
        0xdb, 0xff, // IN A,(FFh)
        0xe6, 0xc0, // AND C0h
        0x28, 0xfa, // JR Z, wait
        0xfa, 0x22, 0x3d, // JP M, done
        0xed, 0xa2, // INI
        0x18, 0xf3, // JR wait
        0x76, // HALT
    ];
    rom[0x3d00..0x3d00 + code.len()].copy_from_slice(code);
    rom
}

#[test]
fn beta_trdos_rom_loop_reads_trd_sector_into_ram() {
    let mut m = Machine::new_48k(&[0u8; 16384]).unwrap();
    m.load_trdos_rom(&trdos_read_sector_rom()).unwrap();
    m.insert_trd(synthetic_trd_with_marker(0x12, 0x34)).unwrap();
    m.cpu_mut().regs.pc = 0x3d00;
    m.cpu_mut().regs.sp = 0xfffd;
    for _ in 0..100_000 {
        if m.cpu().regs.halted {
            break;
        }
        m.step_once();
    }
    assert!(m.cpu().regs.halted, "synthetic TR-DOS loop should HALT");
    assert_eq!(m.read_mem(0x4000), 0x12);
    assert_eq!(m.read_mem(0x4001), 0x34);
    assert!(m.has_beta());
}

/// Optional: real `roms/trdos.rom` + 48K ROM. Skips cleanly when either is missing.
#[test]
fn trdos_rom_usr_15616_pages_when_fixture_present() {
    let Some(spec) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let Some(trdos) = trdos_rom_bytes() else {
        eprintln!("skip: roms/trdos.rom missing (optional #140 TR-DOS boot fixture)");
        return;
    };
    let mut m = Machine::new_48k(&spec).unwrap();
    m.load_trdos_rom(&trdos).unwrap();
    m.insert_trd(synthetic_trd_with_marker(0, 0)).unwrap();
    m.cpu_mut().regs.pc = 0x3d00;
    m.cpu_mut().regs.sp = 0xfffd;
    let mut saw_paged = false;
    for _ in 0..50_000 {
        m.step_once();
        if let Machine::Spec48 { bus, .. } = &m {
            if bus.beta.as_ref().is_some_and(|b| b.paged) {
                saw_paged = true;
                break;
            }
        }
    }
    assert!(
        saw_paged,
        "fetch at 0x3D00 should page TR-DOS ROM (USR 15616)"
    );
}

fn trdos_rom_bytes() -> Option<Vec<u8>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let path = resolve_trdos_rom_preferring_file_services(
        std::slice::from_ref(&root),
        trdos_rom_candidates(Model::Pentagon128),
    )?;
    let data = std::fs::read(path).ok()?;
    (data.len() == bus::TRDOS_ROM_SIZE).then_some(data)
}

/// Hole-filled 5.04 (or any dump) for the harnessed `19ECh` stand-in path.
/// Prefers `roms/pentagon/trdos.rom` so a complete `trdos-5.04t.rom` does not
/// change the established RUN→boot fixture behaviour. Never returns a dump
/// with native `08D2h`/`0D6Bh` services (those belong on the complete path).
fn trdos_rom_bytes_harness() -> Option<Vec<u8>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let preferred = [
        "roms/pentagon/trdos.rom",
        "roms/trdos/trdos.rom",
        "roms/trdos.rom",
    ];
    let mut fallback: Option<Vec<u8>> = None;
    for rel in preferred
        .iter()
        .copied()
        .chain(trdos_rom_candidates(Model::Pentagon128).iter().copied())
    {
        let p = root.join(rel);
        let Ok(data) = std::fs::read(&p) else {
            continue;
        };
        if data.len() != bus::TRDOS_ROM_SIZE {
            continue;
        }
        if trdos_rom_fills_0800_hole(&data) {
            continue;
        }
        // Prefer explicit hole-dump paths when present.
        if preferred.contains(&rel) {
            return Some(data);
        }
        if fallback.is_none() {
            fallback = Some(data);
        }
    }
    fallback
}

/// Complete dump only (fills the usual 5.04 `0800h` hole), if present.
fn trdos_rom_bytes_complete() -> Option<Vec<u8>> {
    let data = trdos_rom_bytes()?;
    trdos_rom_fills_0800_hole(&data).then_some(data)
}

/// Synthetic TR-DOS ROM: read track 1 sector 1 (BASIC `boot`) into `8000h`.
fn trdos_read_boot_basic_rom() -> [u8; bus::TRDOS_ROM_SIZE] {
    let mut rom = [0u8; bus::TRDOS_ROM_SIZE];
    let code: &[u8] = &[
        0x3e, 0x3c, // LD A,3Ch
        0xd3, 0xff, // OUT (FFh),A
        0x3e, 0x01, // LD A,1
        0xd3, 0x3f, // OUT (3Fh),A  track 1
        0x3e, 0x00, // LD A,0
        0xd3, 0x5f, // OUT (5Fh),A  sector 0
        0x3e, 0x80, // LD A,80h
        0xd3, 0x1f, // OUT (1Fh),A
        0x21, 0x00, 0x80, // LD HL,8000h
        0x01, 0x7f, 0x00, // LD BC,007Fh
        0xdb, 0xff, // IN A,(FFh)
        0xe6, 0xc0, // AND C0h
        0x28, 0xfa, // JR Z, wait
        0xfa, 0x23, 0x3d, // JP M, HALT (LD A,track is one byte longer than XOR A)
        0xed, 0xa2, // INI
        0x18, 0xf3, // JR wait
        0x76, // HALT
    ];
    rom[0x3d00..0x3d00 + code.len()].copy_from_slice(code);
    rom
}

#[test]
fn beta_reads_synthetic_boot_basic_into_ram() {
    let mut m = Machine::new_48k(&[0u8; 16384]).unwrap();
    m.load_trdos_rom(&trdos_read_boot_basic_rom()).unwrap();
    m.insert_trd(formats::TrdImage::synthetic_trdos_boot_basic())
        .unwrap();
    m.cpu_mut().regs.pc = 0x3d00;
    m.cpu_mut().regs.sp = 0xfffd;
    for _ in 0..4000 {
        if m.cpu().regs.halted {
            break;
        }
        m.step_once();
    }
    assert!(m.cpu().regs.halted, "synthetic TR-DOS loop should HALT");
    assert_eq!(m.read_mem(0x8000), 0x00);
    assert_eq!(m.read_mem(0x8001), 0x0a);
    assert_eq!(m.read_mem(0x8002), 0x17);
    assert_eq!(m.read_mem(0x8003), 0x00);
    assert_eq!(m.read_mem(0x8004), 0xf4); // POKE
    assert_eq!(m.beta_mut().map(|b| b.sector_read_count), Some(1));
}

/// Synthetic TR-DOS ROM: WRITE TRACK one sector then read it back.
fn trdos_write_track_rom() -> [u8; bus::TRDOS_ROM_SIZE] {
    let mut rom = [0u8; bus::TRDOS_ROM_SIZE];
    let code: &[u8] = &[
        0x3e, 0x3c, // LD A,3Ch
        0xd3, 0xff, // OUT (FFh),A
        0xaf, // XOR A
        0xd3, 0x3f, // OUT (3Fh),A  track 0
        0x3e, 0xf0, // LD A,F0h
        0xd3, 0x1f, // OUT (1Fh),A  WRITE TRACK
        0x3e, 0xfe, // ID: FE
        0xd3, 0x7f, // OUT (7Fh),A
        0xaf, // track 0
        0xd3, 0x7f, 0xaf, // side 0
        0xd3, 0x7f, 0x3e, 0x02, // sector 2
        0xd3, 0x7f, 0x3e, 0x01, // 256 bytes
        0xd3, 0x7f, 0x3e, 0xf7, // CRC
        0xd3, 0x7f, 0x3e, 0xfb, // data mark
        0xd3, 0x7f, 0x3e, 0xbe, // fill byte
        0x06, 0x00, // LD B,0  (256 bytes)
        0xd3, 0x7f, // loop: OUT (7Fh),A
        0x10, 0xfc, // DJNZ loop (-4 → 3D29h)
        0x3e, 0xf7, 0xd3, 0x7f, 0x3e, 0xd8, // Force interrupt
        0xd3, 0x1f, 0x3e, 0x02, // read sector ID 2 (VG93 sector register)
        0xd3, 0x5f, 0x3e, 0x80, 0xd3, 0x1f, 0x21, 0x00, 0x60, // HL=6000h
        0x01, 0x7f, 0x00, 0xdb, 0xff, 0xe6, 0xc0, 0x28, 0xfa, 0xfa, 0x50, 0x3d, // JP M, HALT
        0xed, 0xa2, 0x18, 0xf3, 0x76,
    ];
    rom[0x3d00..0x3d00 + code.len()].copy_from_slice(code);
    rom
}

#[test]
fn beta_write_track_via_synthetic_rom() {
    let mut m = Machine::new_48k(&[0u8; 16384]).unwrap();
    m.load_trdos_rom(&trdos_write_track_rom()).unwrap();
    m.insert_trd(synthetic_trd_with_marker(0, 0)).unwrap();
    m.cpu_mut().regs.pc = 0x3d00;
    m.cpu_mut().regs.sp = 0xfffd;
    for _ in 0..50_000 {
        if m.cpu().regs.halted {
            break;
        }
        m.step_once();
    }
    assert!(m.cpu().regs.halted);
    assert_eq!(m.read_mem(0x6000), 0xbe);
    assert_eq!(m.beta_mut().map(|b| b.write_track_count), Some(1));
}

fn rom_pentagon() -> Option<Vec<u8>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    for rel in ["roms/pentagon/pentagon.rom", "roms/pentagon/128p.rom"] {
        if let Ok(data) = std::fs::read(root.join(rel)) {
            if data.len() == 32768 {
                return Some(data);
            }
        }
    }
    None
}

fn init_trdos_usr_call_frame(m: &mut Machine) {
    m.cpu_mut().regs.sp = 0xfffe;
    m.cpu_mut().regs.set_hl(0);
}

fn enter_128k_basic_from_menu(m: &mut Machine) {
    const PRESS: u32 = 15;
    const GAP: u32 = 5;
    for _ in 0..250 {
        let _ = m.run_frame();
    }
    m.hold_keys(&[(0, 0), (4, 4)], PRESS);
    m.hold_keys(&[], GAP);
    m.hold_keys(&[(6, 0)], PRESS);
    m.hold_keys(&[], 30);
    for _ in 0..400 {
        let _ = m.run_frame();
    }
}

fn ensure_trdos_beta128_prog(m: &mut Machine) {
    let prog = u16::from(m.read_mem(0x5c4f)) | (u16::from(m.read_mem(0x5c50)) << 8);
    let chans = u16::from(m.read_mem(0x5c4d)) | (u16::from(m.read_mem(0x5c4e)) << 8);
    if chans == 0 {
        // Keep CHANS clear of `5D25h` sector buffer and PROG at `5E00h`.
        const CHANS: u16 = 0x5f00;
        m.write_mem(0x5c4d, (CHANS & 0xff) as u8);
        m.write_mem(0x5c4e, (CHANS >> 8) as u8);
    }
    // Beta128 `3D21h` requires `(PROG) >= 5D25h`, but `5D25h` is also the TR-DOS
    // 256-byte sector buffer (`1E4Bh` / `197Eh`). Park PROG above that window.
    if prog < 0x5e00 {
        const PROG: u16 = 0x5e00;
        m.write_mem(0x5c4f, (PROG & 0xff) as u8);
        m.write_mem(0x5c50, (PROG >> 8) as u8);
        m.write_mem(0x5c51, ((PROG + 1) & 0xff) as u8);
        m.write_mem(0x5c52, ((PROG + 1) >> 8) as u8);
        m.write_mem(PROG, 0x80);
    }
    // TR-DOS command parse (`3032h` / `02FCh`) reads Spectrum `(PROG)` at `5C59h`,
    // while Beta128 entry checks `5C4Fh`. Keep both pointers on the same line buffer.
    let prog = u16::from(m.read_mem(0x5c4f)) | (u16::from(m.read_mem(0x5c50)) << 8);
    m.write_mem(0x5c59, (prog & 0xff) as u8);
    m.write_mem(0x5c5a, (prog >> 8) as u8);
    // Find-boot (`195Ch`): `LD A,(5CF9); CP #FF; JP NZ,1E3Dh`. Non-`FF` skips the
    // catalog scan and enters load with `B=0` → `1E74h RET Z` (no Type-II). Init
    // copies `(5CF6)→(5CF9)`; `1812h` sets `#FF` on the named-RUN path we may miss.
    m.write_mem(0x5cf6, 0xff);
    m.write_mem(0x5cf9, 0xff);
    m.write_mem(0x5d17, 0xaa);
    m.write_mem(0x5d0f, 0x00);
    m.write_mem(0x5d16, 0x3c);
}

/// Map PC to a TR-DOS ROM offset. [`BetaDisk::read_rom`] only overlays
/// `0000–3FFF`, so any higher PC is RAM and must not be treated as ROM.
fn trdos_rom_pc(pc: u16) -> Option<u16> {
    (pc < 0x4000).then_some(pc)
}

/// `5CC2h` RST `#20` gate used by `2F72h`.
///
/// Stock DOS init writes a lone `C9`. With that stub, `3D94h` `RST #20` / inline
/// `0010h` falls into `JP 3D82h` and recurses (`CALL 3D94h` again). Skip only the
/// `DE==0010h` service so `3D94h` can stay unpatched; other vectors (e.g. `1F54h`)
/// still run. Bytes live in the DOS printer-buffer hole at `5CC2h` (11 bytes).
fn install_trdos_rst20_5cc2_hook(m: &mut Machine) {
    // POP HL / CP L,#10 / JR NZ,do / OR H / RET Z / PUSH HL / RET
    const HOOK: [u8; 11] = [
        0xe1, 0x7d, 0xfe, 0x10, 0x20, 0x03, 0x7c, 0xb7, 0xc8, 0xe5, 0xc9,
    ];
    for (i, &b) in HOOK.iter().enumerate() {
        m.write_mem(0x5cc2 + i as u16, b);
    }
}

/// Harness entry for ROM-gated `RUN` → `boot` (#266 / #140).
///
/// CAT / VG93-wait / PROG-wipe sites stay **stock** — see
/// [`apply_trdos_run_native_abi`]. `3D94h` uses [`install_trdos_rst20_5cc2_hook`].
/// Remaining gap: this 5.04 image has FF from `0800h`–`0E71h` (`08D2h` and
/// `0D6Bh`). Native `012Ah` re-enters catalog before LINE-NEW; `19ECh`
/// VG93+LINE-NEW handoff lives in [`apply_trdos_run_native_abi`].
fn patch_trdos_run_harness_rom(_m: &mut Machine) {
    // No ROM writes — CAT/wait/`19ECh` reductions live in `apply_trdos_run_native_abi`.
}

/// Stock find-boot ABI so catalog ROM stays unpatched (#140 / #266).
///
/// `195Ch` stores caller `DE` as catalog CHS (`1964h`) then `LD C,0` (`1968h`).
/// The sibling entry `1946h` skips that and loads `C` from `(5CDB)`. Seed name
/// `HL=5EE0h`, CHS `DE=0`, one catalog sector `B=1` at `195Ch`, and `C=16` at
/// `196Ah` (after `LD C,0`) so the 16-byte dirent compare / `DJNZ` RET need no
/// ROM writes.
fn apply_trdos_find_boot_native_abi(m: &mut Machine) {
    const NAME: u16 = 0x5ee0;
    match m.cpu().regs.pc {
        0x195c => {
            m.cpu_mut().regs.set_hl(NAME);
            m.cpu_mut().regs.set_de(0);
            m.cpu_mut().regs.b = 1;
        }
        0x196a => {
            m.cpu_mut().regs.set_hl(NAME);
            m.cpu_mut().regs.set_de(0);
            m.cpu_mut().regs.set_bc(0x0110); // B=1, C=16
        }
        _ => {}
    }
}

/// True when the *currently loaded* TR-DOS image has a classic file-load at `08D2h`.
///
/// Must not consult the preferred-on-disk resolver: harness tests load the hole
/// dump even when `trdos-5.04t.rom` exists beside it. Alone Coder 5.04T fills
/// `08D2h` with a VG93 port stub — that is **not** a file service.
fn trdos_rom_has_native_file_services_paged(m: &mut Machine) -> bool {
    let was = m.beta_mut().is_some_and(|b| b.paged);
    if let Some(beta) = m.beta_mut() {
        beta.page_trdos(true);
    }
    let mut img = [0u8; bus::TRDOS_ROM_SIZE];
    for (i, b) in img.iter_mut().enumerate() {
        *b = m.read_mem(i as u16);
    }
    let ok = trdos_rom_has_native_file_services(&img);
    if let Some(beta) = m.beta_mut() {
        beta.page_trdos(was);
    }
    ok
}

/// Register / PC ABI so RUN harness ROM stays stock (#140).
///
/// Replaces former ROM RET/NOP/JR patches at `3D9Dh` / `02D4h` / `213Eh` /
/// `2155h` plus the `3DFFh` A=1 delay seed, and the post-match `19ECh`
/// VG93+LINE-NEW stand-in (this image’s `08D2h` is FF padding).
fn apply_trdos_run_native_abi(m: &mut Machine) {
    apply_trdos_find_boot_native_abi(m);
    let pc = m.cpu().regs.pc;
    match pc {
        // Stock `3DFFh`: `LD C,#FF` / `DEC C` until Z / `DEC A` / JR NZ.
        // Callers use `A=5` (`02C3h`) or `A=#FF` × `B=3` (`3EA4h` motor spin).
        // `A=1` runs one inner 255-iter loop instead of a ROM RET patch.
        0x3dff => {
            m.cpu_mut().regs.a = 1;
        }
        // `3D9Ah` Type-I wait: stock `RST #20`→`1F54h` never unwinds with
        // instant completion — skip to `3DA5h` `POP HL` (was `JR 3DA5h` patch).
        0x3d9d => {
            m.cpu_mut().regs.pc = 0x3da5;
        }
        // Warm `02CBh` `CALL 1D83h` CAT blocks on a key (`161Dh`) — skip the CALL.
        0x02d4 => {
            m.cpu_mut().regs.pc = 0x02d7;
        }
        // 5.04T warm path: `0249h` is `CALL 3AE6h` (XOR A/OUT (9)/LD HL,5D17/RET)
        // where hole 5.04 inlines `LD HL,5D17`. Skip the CALL so SP/IFF stay aligned
        // with the harnessed hole path (native `08D2h` still runs at `19ECh`).
        0x0249
            if m.read_mem(0x0249) == 0xcd
                && m.read_mem(0x024a) == 0xe6
                && m.read_mem(0x024b) == 0x3a =>
        {
            m.cpu_mut().regs.set_hl(0x5d17);
            m.cpu_mut().regs.pc = 0x024c;
        }
        // `213Eh` `CALL Z,211Eh` wipes `(PROG)` when Z (`5D0F=0`); keep seeded
        // `RUN\\r` for `3032h` by skipping the call (was three NOPs).
        0x213e => {
            m.cpu_mut().regs.pc = 0x2141;
        }
        // `2155h` stock `JP 1D90h` (CAT) never returns to `02ECh` — RET to caller.
        0x2155 => {
            let sp = m.cpu().regs.sp;
            let ret = u16::from(m.read_mem(sp)) | (u16::from(m.read_mem(sp.wrapping_add(1))) << 8);
            m.cpu_mut().regs.sp = sp.wrapping_add(2);
            m.cpu_mut().regs.pc = ret;
        }
        // 5.04T: `1FEBh` ends `JP 0897h` instead of hole `OUT (#FF),A / RET`.
        // `CALL 1FEBh` from `3E63h` must return so catalog `1E3Dh` can finish.
        0x1ff3
            if m.read_mem(0x1ff3) == 0xc3
                && m.read_mem(0x1ff4) == 0x97
                && m.read_mem(0x1ff5) == 0x08 =>
        {
            let sys = m.cpu().regs.a;
            if let Some(beta) = m.beta_mut() {
                let _ = beta.out_port(0x00ff, sys);
            }
            let sp = m.cpu().regs.sp;
            let ret = u16::from(m.read_mem(sp)) | (u16::from(m.read_mem(sp.wrapping_add(1))) << 8);
            m.cpu_mut().regs.sp = sp.wrapping_add(2);
            m.cpu_mut().regs.pc = ret;
        }
        // Stock `19ECh`: `RST #20` / `DW 08D2h`. On hole dumps, never enter
        // `08D2h` FF padding — FDC-load `boot` and enter Spectrum `LINE-NEW`.
        // Complete dumps (non-FF at `08D2h`) run the native service.
        0x19ec
            if !trdos_rom_has_native_file_services_paged(m) && trdos_fdc_load_boot_into_prog(m) =>
        {
            m.cpu_mut().regs.pc = 0x1b76;
        }
        _ => {}
    }
}

/// Invoke TR-DOS `RUN` with no filename (loads `boot`).
///
/// Warm entry `0239h`→`02E9h`→`3032h` reaches find-boot Type-II catalog reads.
/// Post-match `19ECh` is stock `RST #20` / inline `08D2h`; this ROM's `08D2h` is
/// FF padding, so [`apply_trdos_run_native_abi`] FDC-loads `boot` at the **call
/// site**, unpages TR-DOS, and enters Spectrum `LINE-NEW` (`1B76h`) — never
/// executes the hole.
/// Name block lives at `5EE0h` so it does not overlap the `5D25h` sector buffer.
fn invoke_trdos_run_boot(m: &mut Machine) -> bool {
    m.write_mem(0x5cb6, 0xf4);
    m.write_mem(0x5cb7, 0x0d);
    // RST #20 epilogue (`2F72h`) enters `5CC2h` before the inline service address.
    // Skip recursive `0010h`→`3D82h` so stock `3D94h` (`RST #20` / `0010h`) returns.
    install_trdos_rst20_5cc2_hook(m);
    m.write_mem(0x5d0f, 0);
    // Find-boot sentinel for `1921h` `CALL Z,195Ch`.
    m.write_mem(0x5d10, 0xff);
    // Seed Spectrum `(PROG)` at `5C59h`: ASCII `RUN` + CR so `3032h` tokenizes.
    let prog = u16::from(m.read_mem(0x5c59)) | (u16::from(m.read_mem(0x5c5a)) << 8);
    for (i, &b) in b"RUN\r\x80".iter().enumerate() {
        m.write_mem(prog.wrapping_add(i as u16), b);
    }
    // Re-assert find-boot catalog gate after `USR 15616` / warm path.
    m.write_mem(0x5cf6, 0xff);
    m.write_mem(0x5cf9, 0xff);
    // Sector count for find-boot outer `B` (`5CDC`) + `1946h` `C` from `(5CDB)`
    // (dirent length; `195Ch` still `LD C,0` and is fixed at `196Ah`).
    m.write_mem(0x5cdb, 0x10);
    m.write_mem(0x5cdc, 0x08);
    // Catalog start CHS (`5CD9` / `5CF4`); `195Ch` `LD (5CF4),DE` needs `DE=0`.
    m.write_mem(0x5cd9, 0x00);
    m.write_mem(0x5cda, 0x00);
    m.write_mem(0x5cf4, 0x00);
    m.write_mem(0x5cf5, 0x00);
    // Full 16-byte TR-DOS dirent for synthetic `boot` — **above** `5D25h` buffer.
    const NAME: u16 = 0x5ee0;
    let dirent: [u8; 16] = [
        b'b', b'o', b'o', b't', b' ', b' ', b' ', b' ', b'B', 28, 0, 27, 0, 1, 0, 1,
    ];
    for (i, &b) in dirent.iter().enumerate() {
        m.write_mem(NAME + i as u16, b);
    }
    m.write_mem(0x5cd7, (NAME & 0xff) as u8);
    m.write_mem(0x5cd8, (NAME >> 8) as u8);
    patch_trdos_run_harness_rom(m);
    let sp = m.cpu().regs.sp;
    m.write_mem(0x5c3d, (sp & 0xff) as u8);
    m.write_mem(0x5c3e, (sp >> 8) as u8);
    // `3D34h` `PUSH HL` after `3D21h` (`HL=5CC2h`), then `3D35h` → `0239h`.
    m.cpu_mut().regs.set_hl(0x5cc2);
    m.cpu_mut().regs.set_de(0);
    m.cpu_mut().regs.sp = sp.wrapping_sub(2);
    m.write_mem(sp.wrapping_sub(2), 0xc2);
    m.write_mem(sp.wrapping_sub(1), 0x5c);
    m.write_mem(0x5d17, 0xaa);
    m.cpu_mut().regs.pc = 0x0239;
    wait_for_trdos_boot_marker(m, 10_000)
}

fn manual_read_track1_sector1(m: &mut Machine) -> bool {
    let Some(beta) = m.beta_mut() else {
        return false;
    };
    beta.page_trdos(true);
    beta.out_port(0x00ff, 0x3c);
    beta.out_port(0x003f, 1);
    beta.out_port(0x005f, 1);
    beta.out_port(0x001f, 0x80);
    if beta.sector_read_count == 0 {
        return false;
    }
    let mut ok = true;
    for _ in 0..256 {
        let Some(st) = beta.in_port(0x001f) else {
            ok = false;
            break;
        };
        if st & 0x02 != 0 {
            let _ = beta.in_port(0x007f);
        }
        if st & 0x80 != 0 {
            break;
        }
    }
    ok
}

/// Enter TR-DOS command mode via `USR 15616` (`3D00h` → `3D31h`).
///
/// Paging is asserted (it works today); reaching the `3D31h` command loop is
/// returned rather than asserted because #140 RUN boot is still open.
fn enter_trdos_command_mode(m: &mut Machine) -> bool {
    init_trdos_usr_call_frame(m);
    m.cpu_mut().regs.pc = 0x3d00;
    let mut saw_paged = false;
    let mut at_prompt = false;
    for _ in 0..20_000_000 {
        m.step_once();
        if m.beta_mut().is_some_and(|b| b.paged) {
            saw_paged = true;
        }
        if saw_paged && trdos_rom_pc(m.cpu().regs.pc) == Some(0x3D31) {
            at_prompt = true;
            break;
        }
    }
    assert!(
        saw_paged,
        "TR-DOS should page (PC={:#06x})",
        m.cpu().regs.pc
    );
    at_prompt
}

/// After find-boot matches `boot`, stock `19ECh` would `RST #20` into `08D2h`,
/// which is FF padding on this ROM image. Load the file body through the real
/// VG93 path into `(PROG)`, wire Spectrum sysvars / `NEWPPC` / FLAGS bit 7
/// (running), unpage TR-DOS, page 48K BASIC ROM, and enter `LINE-NEW` (`1B76h`).
///
/// Why not TR-DOS `012Ah` / native `08D2h` service (this image):
/// - FF padding `0800h`–`0E71h` covers `08D2h` and `0D6Bh` (`012Ah` `CALL 1D97h`).
/// - Entering `012Ah` from `19ECh` re-enters catalog (`30B2h`) / Type-I wait
///   (`3D9Ch`) before LINE-NEW; `RST #20`/`16B0h` is mid-`CALL 166Fh`.
/// - Beta keeps the TR-DOS latch across RAM, so stock `5CC2h`→`1B76h` would still
///   fetch TR-DOS at `1B76h`.
/// - 128/Pentagon ROM0 is the editor; `1B76h` LINE-NEW lives in ROM1 (`7FFDh` bit 4).
fn trdos_fdc_load_boot_into_prog(m: &mut Machine) -> bool {
    let prog = u16::from(m.read_mem(0x5c59)) | (u16::from(m.read_mem(0x5c5a)) << 8);
    let start_sec = m.read_mem(0x5d25 + 14);
    let start_trk = m.read_mem(0x5d25 + 15);
    let file_type = m.read_mem(0x5d25 + 8);
    let len = u16::from(m.read_mem(0x5d25 + 9)) | (u16::from(m.read_mem(0x5d25 + 10)) << 8);
    if file_type != b'B' || start_trk == 0 || len == 0 || len > 256 {
        return false;
    }
    let mut buf = [0u8; 256];
    {
        let Some(beta) = m.beta_mut() else {
            return false;
        };
        beta.page_trdos(true);
        beta.out_port(0x00ff, 0x3c);
        beta.out_port(0x003f, start_trk);
        // TR-DOS dirent sector 0 → VG93 ID 1 (see `BetaDisk::sector_index`).
        beta.out_port(0x005f, start_sec.max(1));
        beta.out_port(0x001f, 0x80);
        if beta.sector_read_count == 0 {
            return false;
        }
        for b in &mut buf {
            let mut spins = 0u32;
            loop {
                let st = beta.in_port(0x001f).unwrap_or(0);
                if st & 0x02 != 0 {
                    break;
                }
                if st & 0x80 != 0 && st & 0x02 == 0 {
                    return false;
                }
                spins += 1;
                if spins > 10_000 {
                    return false;
                }
            }
            *b = beta.in_port(0x007f).unwrap_or(0);
        }
    }
    // Program + empty VARS only (`len`); autostart `AAh` trailer stays out of E_LINE.
    for (i, &b) in buf[..len as usize].iter().enumerate() {
        m.write_mem(prog.wrapping_add(i as u16), b);
    }
    let vars_off = u16::from(m.read_mem(0x5d25 + 11)) | (u16::from(m.read_mem(0x5d25 + 12)) << 8);
    let vars = prog.wrapping_add(vars_off);
    let e_line = vars.wrapping_add(1);
    let write_u16 = |m: &mut Machine, addr: u16, val: u16| {
        m.write_mem(addr, (val & 0xff) as u8);
        m.write_mem(addr.wrapping_add(1), (val >> 8) as u8);
    };
    // Standard Spectrum sysvars (TR-DOS harness aliases `5C4Fh`/`5C59h` differently).
    // Harness parked channel info at `5C4Dh` and PROG at `5C4Fh` — restore CHANS.
    let chans = u16::from(m.read_mem(0x5c4d)) | (u16::from(m.read_mem(0x5c4e)) << 8);
    write_u16(m, 0x5c4b, vars); // VARS
                                // Prefer a live channel block left by 128 BASIC entry. The TR-DOS harness
                                // parks a blank `5F00h` window and aliases CHANS at `5C4Dh` while overwriting
                                // standard `5C4Fh` with PROG — recover the post-menu pointer when present.
    let standard_chans = u16::from(m.read_mem(0x5c4f)) | (u16::from(m.read_mem(0x5c50)) << 8);
    let chans_ptr = if (0x5b00..0x5e00).contains(&standard_chans) {
        standard_chans
    } else if (0x5b00..0x5e00).contains(&chans) {
        chans
    } else {
        // Minimal K/S/R/P channels (5 bytes each: OUT, IN, letter) as NEW installs.
        const CHANS: u16 = 0x5f00;
        let block: [u8; 21] = [
            0xf4, 0x09, 0xa8, 0x10, b'K', // PRINT-OUT / KEY-INPUT
            0xf4, 0x09, 0xc4, 0x15, b'S', // PRINT-OUT / KEY-INPUT
            0x81, 0x0f, 0xc4, 0x15, b'R', // ADD-CHAR / KEY-INPUT
            0xf4, 0x09, 0xc4, 0x15, b'P', // PRINT-OUT / KEY-INPUT
            0x80, // end marker
        ];
        for (i, &b) in block.iter().enumerate() {
            m.write_mem(CHANS.wrapping_add(i as u16), b);
        }
        CHANS
    };
    write_u16(m, 0x5c4f, chans_ptr); // CHANS
    write_u16(m, 0x5c51, chans_ptr); // CURCHL → first channel
                                     // Stream offsets from CHANS (NEW defaults).
    for (i, off) in [0x0001u16, 0x0006, 0x000b, 0x0001, 0x0001, 0x0006, 0x0010]
        .into_iter()
        .enumerate()
    {
        write_u16(m, 0x5c10 + (i as u16) * 2, off);
    }
    write_u16(m, 0x5c53, prog); // PROG
    write_u16(m, 0x5c59, e_line); // E_LINE
    write_u16(m, 0x5c61, e_line); // WORKSP
    write_u16(m, 0x5c63, e_line); // STKBOT
    write_u16(m, 0x5c65, e_line); // STKEND
                                  // Empty edit line terminator at E_LINE (required by many ROM walks).
    m.write_mem(e_line, 0x0d);
    m.write_mem(e_line.wrapping_add(1), 0x80);
    // Autostart LINE from TR-DOS trailer (`AAh`, line LE) or first program line.
    let newppc = if buf.get(len as usize) == Some(&0xaa) {
        u16::from(buf[len as usize + 1]) | (u16::from(buf[len as usize + 2]) << 8)
    } else {
        u16::from(buf[1]) | (u16::from(buf[0]) << 8)
    };
    write_u16(m, 0x5c42, newppc); // NEWPPC
    m.write_mem(0x5c44, 0); // NSPPC = first statement
                            // SYNTAX-Z (`1C11h`) is `BIT 7,(IY+1)`: Z set when bit 7 is
                            // clear, i.e. syntax-checking. TR-DOS/128 editor leftover FLAGS
                            // `1Dh` keeps DECIMAL inserting a second `0x0E` (`00 00 00 80 00`
                            // for 32768) in front of the stored `90…` float → Report C.
    m.write_mem(0x5c3b, m.read_mem(0x5c3b) | 0x80);
    // Leave DOS + select 48K BASIC ROM so `1B76h` is LINE-NEW.
    if let Some(beta) = m.beta_mut() {
        beta.page_trdos(false);
    }
    if let Machine::Spec128 { bus, .. } = m {
        let page = bus.page | 0x10; // bit 4 → ROM1 (48K BASIC)
        bus.out_7ffd(page);
        // Keep BANK_M (`5B5C`) in sync — 128 SWAP at `5B00h` XORs bit 4 from
        // this shadow copy. Desync (e.g. `07` while port is `17`) makes ROM1
        // `3B4Dh`→`0112h` run ROM1 message bytes instead of ROM0 Statement Return.
        m.write_mem(0x5b5c, page);
    }
    true
}

fn wait_for_trdos_boot_marker(m: &mut Machine, max_frames: u32) -> bool {
    let native = trdos_rom_has_native_file_services_paged(m);
    let mut saw_08d2 = false;
    // Prefer instruction steps so we cannot miss the one-instruction `19ECh`
    // call-site window inside a full frame (`apply_trdos_run_native_abi`).
    let max_steps = u64::from(max_frames).saturating_mul(70_000).min(3_000_000);
    for _ in 0..max_steps {
        if m.cpu().regs.pc == 0x08d2 {
            saw_08d2 = true;
        }
        apply_trdos_run_native_abi(m);
        m.step_once();
        if m.read_mem(0x8000) == 0xa5 {
            if native {
                assert!(
                    saw_08d2,
                    "complete ROM: RUN boot must enter native 08D2h file-load service"
                );
            } else {
                assert!(
                    !saw_08d2,
                    "hole dump: RUN boot must not execute 08D2h FF padding (handoff at 19ECh)"
                );
            }
            return true;
        }
    }
    false
}

/// Optional: real `roms/trdos.rom` + 128K main ROM. Skips when either is missing.
#[test]
fn trdos_rom_reads_boot_when_128k_chans_ok_and_fixture_present() {
    let Some(main) = rom_pentagon().or_else(rom128) else {
        eprintln!("skip: pentagon/128 main ROM missing");
        return;
    };
    let Some(trdos) = trdos_rom_bytes_harness() else {
        eprintln!("skip: roms/trdos.rom missing (optional #140 TR-DOS boot fixture)");
        return;
    };
    let mut m = Machine::new_pentagon128(&main, &trdos).unwrap();
    m.insert_trd(formats::TrdImage::synthetic_trdos_boot_basic())
        .unwrap();
    enter_128k_basic_from_menu(&mut m);
    ensure_trdos_beta128_prog(&mut m);
    let at_prompt = enter_trdos_command_mode(&mut m);
    assert!(
        manual_read_track1_sector1(&mut m),
        "FDC should read boot sector after TR-DOS entry (PC={:#06x}, at_prompt={at_prompt})",
        m.cpu().regs.pc
    );
}

/// Debug: dump LINE-NEW handoff sysvars + PC trail until RST `#08` / marker.
#[test]
#[ignore]
fn debug_trdos_line_new_handoff() {
    let Some(main) = rom_pentagon().or_else(rom128) else {
        return;
    };
    let Some(trdos) = trdos_rom_bytes() else {
        return;
    };
    let mut m = Machine::new_pentagon128(&main, &trdos).unwrap();
    m.insert_trd(formats::TrdImage::synthetic_trdos_boot_basic())
        .unwrap();
    enter_128k_basic_from_menu(&mut m);
    ensure_trdos_beta128_prog(&mut m);
    assert!(enter_trdos_command_mode(&mut m));

    // Drive RUN until the harness loads + jumps to LINE-NEW, then stop stepping
    // the DOS path and inspect Spectrum BASIC state.
    m.write_mem(0x5cb6, 0xf4);
    m.write_mem(0x5cb7, 0x0d);
    install_trdos_rst20_5cc2_hook(&mut m);
    m.write_mem(0x5d0f, 0);
    m.write_mem(0x5d10, 0xff);
    let prog0 = u16::from(m.read_mem(0x5c59)) | (u16::from(m.read_mem(0x5c5a)) << 8);
    for (i, &b) in b"RUN\r\x80".iter().enumerate() {
        m.write_mem(prog0.wrapping_add(i as u16), b);
    }
    m.write_mem(0x5cf6, 0xff);
    m.write_mem(0x5cf9, 0xff);
    m.write_mem(0x5cdb, 0x10);
    m.write_mem(0x5cdc, 0x08);
    m.write_mem(0x5cd9, 0x00);
    m.write_mem(0x5cda, 0x00);
    m.write_mem(0x5cf4, 0x00);
    m.write_mem(0x5cf5, 0x00);
    const NAME: u16 = 0x5ee0;
    let dirent: [u8; 16] = [
        b'b', b'o', b'o', b't', b' ', b' ', b' ', b' ', b'B', 28, 0, 27, 0, 1, 0, 1,
    ];
    for (i, &b) in dirent.iter().enumerate() {
        m.write_mem(NAME + i as u16, b);
    }
    m.write_mem(0x5cd7, (NAME & 0xff) as u8);
    m.write_mem(0x5cd8, (NAME >> 8) as u8);
    patch_trdos_run_harness_rom(&mut m);
    let sp = m.cpu().regs.sp;
    m.write_mem(0x5c3d, (sp & 0xff) as u8);
    m.write_mem(0x5c3e, (sp >> 8) as u8);
    m.cpu_mut().regs.set_hl(0x5cc2);
    m.cpu_mut().regs.set_de(0);
    m.cpu_mut().regs.sp = sp.wrapping_sub(2);
    m.write_mem(sp.wrapping_sub(2), 0xc2);
    m.write_mem(sp.wrapping_sub(1), 0x5c);
    m.write_mem(0x5d17, 0xaa);
    m.cpu_mut().regs.pc = 0x0239;

    let mut loaded = false;
    for _ in 0..3_000_000u32 {
        assert_ne!(
            m.cpu().regs.pc,
            0x08d2,
            "debug path must not enter 08D2h FF padding"
        );
        let at_19ec = m.cpu().regs.pc == 0x19ec;
        apply_trdos_run_native_abi(&mut m);
        if at_19ec && m.cpu().regs.pc == 0x1b76 {
            loaded = true;
            break;
        }
        m.step_once();
    }
    assert!(loaded, "did not reach 19ECh FDC handoff");

    let rd16 = |m: &Machine, a: u16| -> u16 {
        u16::from(m.read_mem(a)) | (u16::from(m.read_mem(a.wrapping_add(1))) << 8)
    };
    let page = match &m {
        Machine::Spec128 { bus, .. } => bus.page,
        _ => 0,
    };
    let paged = m.beta_mut().is_some_and(|b| b.paged);
    eprintln!(
        "handoff PC={:#06x} SP={:#06x} IY={:#06x} page={page:#04x} trdos={paged}",
        m.cpu().regs.pc,
        m.cpu().regs.sp,
        m.cpu().regs.iy()
    );
    let stub: Vec<u8> = (0..32).map(|i| m.read_mem(0x5b00 + i)).collect();
    eprintln!("  5B00 stub={stub:02x?}");
    eprintln!(
        "  ERR_NR={:#04x} FLAGS={:#04x} FLAGS2={:#04x} ERR_SP={:#06x} RAMTOP={:#06x}",
        m.read_mem(0x5c3a),
        m.read_mem(0x5c3b),
        m.read_mem(0x5c3c),
        rd16(&m, 0x5c3d),
        rd16(&m, 0x5cb2)
    );
    eprintln!(
        "  NEWPPC={:#06x} NSPPC={:#04x} PPC={:#06x} SUBPPC={:#04x}",
        rd16(&m, 0x5c42),
        m.read_mem(0x5c44),
        rd16(&m, 0x5c45),
        m.read_mem(0x5c47)
    );
    eprintln!(
        "  VARS={:#06x} CHANS={:#06x} PROG={:#06x} E_LINE={:#06x} WORKSP={:#06x} STKEND={:#06x}",
        rd16(&m, 0x5c4b),
        rd16(&m, 0x5c4f),
        rd16(&m, 0x5c53),
        rd16(&m, 0x5c59),
        rd16(&m, 0x5c61),
        rd16(&m, 0x5c65)
    );
    let strms: Vec<u16> = (0..7).map(|i| rd16(&m, 0x5c10 + i * 2)).collect();
    eprintln!("  STRMS={strms:04x?}");
    let prog = rd16(&m, 0x5c53);
    let prog_bytes: Vec<u8> = (0..32).map(|i| m.read_mem(prog.wrapping_add(i))).collect();
    eprintln!("  PROG bytes={prog_bytes:02x?}");
    let chans = rd16(&m, 0x5c4f);
    let chans_bytes: Vec<u8> = (0..21).map(|i| m.read_mem(chans.wrapping_add(i))).collect();
    eprintln!("  CHANS bytes={chans_bytes:02x?}");

    let mut last = 0xffffu16;
    let mut trail: Vec<u16> = Vec::new();
    for step in 0..50_000u32 {
        let pc = m.cpu().regs.pc;
        if pc != last {
            trail.push(pc);
            if trail.len() <= 40 || matches!(pc, 0x0008 | 0x3b4d | 0x0112 | 0x1b76 | 0x1b7d) {
                eprintln!(
                    "step={step} PC={pc:#06x} A={:#04x} HL={:#06x} ERR_NR={:#04x} FLAGS={:#04x}",
                    m.cpu().regs.a,
                    m.cpu().regs.hl(),
                    m.read_mem(0x5c3a),
                    m.read_mem(0x5c3b)
                );
            }
            last = pc;
        }
        if pc == 0x0008 {
            let sp = m.cpu().regs.sp;
            let ret = u16::from(m.read_mem(sp)) | (u16::from(m.read_mem(sp.wrapping_add(1))) << 8);
            let err_byte = m.read_mem(ret);
            let ch_add = rd16(&m, 0x5c5d);
            let prog_now = rd16(&m, 0x5c53);
            let around: Vec<u8> = (0..32)
                .map(|i| m.read_mem(prog_now.wrapping_add(i)))
                .collect();
            let ch_around: Vec<u8> = (0i16..8)
                .map(|i| m.read_mem(ch_add.wrapping_add((i - 2) as u16)))
                .collect();
            eprintln!(
                    "RST8 at step={step} err_byte={err_byte:#04x} CH_ADD={ch_add:#06x} STKEND={:#06x} PROG={prog_now:#06x}",
                    rd16(&m, 0x5c65)
                );
            eprintln!("  PROG now={around:02x?}");
            eprintln!("  near CH_ADD(-2..+5)={ch_around:02x?}");
            eprintln!("  trail={:04x?}", &trail[trail.len().saturating_sub(24)..]);
            break;
        }
        m.step_once();
        if m.read_mem(0x8000) == 0xa5 {
            eprintln!("MARKER at step={step}");
            break;
        }
    }
    eprintln!(
        "final PC={:#06x} 8000={:#04x} ERR_NR={:#04x} page trail_len={} last={:04x?}",
        m.cpu().regs.pc,
        m.read_mem(0x8000),
        m.read_mem(0x5c3a),
        trail.len(),
        &trail[trail.len().saturating_sub(20)..]
    );
}

/// Debug: short PC/FDC trace for #266 RUN path (ignore in CI).
#[test]
#[ignore]
fn debug_trdos_run_pc_trace() {
    let Some(main) = rom_pentagon().or_else(rom128) else {
        return;
    };
    let Some(trdos) = trdos_rom_bytes() else {
        return;
    };
    let mut m = Machine::new_pentagon128(&main, &trdos).unwrap();
    m.insert_trd(formats::TrdImage::synthetic_trdos_boot_basic())
        .unwrap();
    enter_128k_basic_from_menu(&mut m);
    ensure_trdos_beta128_prog(&mut m);
    assert!(enter_trdos_command_mode(&mut m));
    // Same setup as invoke_trdos_run_boot without waiting.
    m.write_mem(0x5cb6, 0xf4);
    m.write_mem(0x5cb7, 0x0d);
    install_trdos_rst20_5cc2_hook(&mut m);
    m.write_mem(0x5d0f, 0);
    m.write_mem(0x5d10, 0xff);
    let prog = u16::from(m.read_mem(0x5c59)) | (u16::from(m.read_mem(0x5c5a)) << 8);
    for (i, &b) in b"RUN\r\x80".iter().enumerate() {
        m.write_mem(prog.wrapping_add(i as u16), b);
    }
    m.write_mem(0x5cf6, 0xff);
    m.write_mem(0x5cf9, 0xff);
    m.write_mem(0x5cdb, 0x10);
    m.write_mem(0x5cdc, 0x08);
    m.write_mem(0x5cd9, 0x00);
    m.write_mem(0x5cda, 0x00);
    m.write_mem(0x5cf4, 0x00);
    m.write_mem(0x5cf5, 0x00);
    const NAME: u16 = 0x5ee0;
    let dirent: [u8; 16] = [
        b'b', b'o', b'o', b't', b' ', b' ', b' ', b' ', b'B', 28, 0, 27, 0, 1, 0, 1,
    ];
    for (i, &b) in dirent.iter().enumerate() {
        m.write_mem(NAME + i as u16, b);
    }
    m.write_mem(0x5cd7, (NAME & 0xff) as u8);
    m.write_mem(0x5cd8, (NAME >> 8) as u8);
    patch_trdos_run_harness_rom(&mut m);
    let sp = m.cpu().regs.sp;
    m.write_mem(0x5c3d, (sp & 0xff) as u8);
    m.write_mem(0x5c3e, (sp >> 8) as u8);
    m.cpu_mut().regs.set_hl(0x5cc2);
    m.cpu_mut().regs.set_de(0);
    m.cpu_mut().regs.sp = sp.wrapping_sub(2);
    m.write_mem(sp.wrapping_sub(2), 0xc2);
    m.write_mem(sp.wrapping_sub(1), 0x5c);
    m.write_mem(0x5d17, 0xaa);
    m.cpu_mut().regs.pc = 0x0239;
    let mut last = 0xffffu16;
    let mut hits = 0u32;
    let mut escaped = 0u32;
    let mut last_ring_len = 0usize;
    let mut saw_c0 = false;
    let watch = [
        0x02e9u16, 0x02ec, 0x2135, 0x2155, 0x3032, 0x030a, 0x031a, 0x1d4d, 0x1d50, 0x1836, 0x187a,
        0x18a1, 0x1921, 0x195c, 0x197e, 0x1997, 0x199c, 0x19dd, 0x08d2, 0x03fa, 0x1e3d, 0x1e40,
        0x1e4d, 0x1e62, 0x1e74, 0x1e75, 0x1e83, 0x3e63, 0x012a, 0x3dc8, 0x3dfa, 0x3f0e, 0x3f25,
        0x07d6, 0x0787,
    ];
    for step in 0..400_000u32 {
        let pc = m.cpu().regs.pc;
        if step % 50_000 == 0 {
            eprintln!(
                "tick step={step} PC={pc:#06x} 5CB6={:#04x} 5D16={:#04x}",
                m.read_mem(0x5cb6),
                m.read_mem(0x5d16)
            );
        }
        if let Some(b) = m.beta_mut() {
            let len = b.command_ring().len();
            if len != last_ring_len {
                eprintln!(
                    "cmd step={step} PC={pc:#06x} ring={:02x?} track={} sys={:#04x} drive={}",
                    b.command_ring(),
                    b.track,
                    b.system,
                    b.system & 3,
                );
                if b.command_ring().last() == Some(&0xc0)
                    || b.command_ring().last().is_some_and(|c| c & 0xe0 == 0x80)
                {
                    saw_c0 |= b.command_ring().last() == Some(&0xc0);
                    hits = 0;
                }
                last_ring_len = len;
            }
        }
        if pc != last {
            let breg = m.cpu().regs.b;
            let sp = m.cpu().regs.sp;
            let sectors = m.beta_mut().map_or(0, |x| x.sector_read_count);
            let in_rom = pc < 0x4000;
            let dos_stub = pc == 0x5cc2 || (0x5c00..0x5e00).contains(&pc);
            let interesting = watch.contains(&pc);
            let limit = if saw_c0 { 250 } else { 80 };
            if interesting || (in_rom && hits < limit) {
                eprintln!("step={step} PC={pc:#06x} B={breg:#04x} SP={sp:#06x} sectors={sectors}");
                if matches!(pc, 0x1e3d | 0x1e40 | 0x1e62 | 0x1e74 | 0x195c | 0x1d4d) {
                    let dir: Vec<u8> = (0..16).map(|i| m.read_mem(0x5d25 + i)).collect();
                    eprintln!(
                        "  A={:#04x} HL={:#06x} 5CF9={:#04x} 5C59→{:#06x} 5D25={:02x?}",
                        m.cpu().regs.a,
                        m.cpu().regs.hl(),
                        m.read_mem(0x5cf9),
                        u16::from(m.read_mem(0x5c59)) | (u16::from(m.read_mem(0x5c5a)) << 8),
                        dir
                    );
                }
                if in_rom && !interesting {
                    hits += 1;
                }
            } else if !in_rom && !dos_stub {
                eprintln!("escaped PC={pc:#06x} at step={step} SP={sp:#06x}");
                escaped += 1;
                if escaped >= 3 {
                    break;
                }
            }
            last = pc;
        }
        apply_trdos_run_native_abi(&mut m);
        m.step_once();
        if m.read_mem(0x8000) == 0xa5 {
            eprintln!("MARKER at step={step}");
            break;
        }
    }
    let pc = m.cpu().regs.pc;
    let marker = m.read_mem(0x8000);
    if let Some(b) = m.beta_mut() {
        eprintln!(
            "final PC={pc:#06x} ring={:02x?} track={} sys={:#04x} sectors={} mem8000={marker:#04x}",
            b.command_ring(),
            b.track,
            b.system,
            b.sector_read_count,
        );
    }
}

/// ROM-gated: stock `3D94h` (`RST #20` / inline `0010h`) returns via the `5CC2h`
/// hook without RET-patching the ROM (#266).
#[test]
fn trdos_3d94_rst20_returns_without_rom_ret_patch_when_fixture_present() {
    let Some(main) = rom_pentagon().or_else(rom128) else {
        eprintln!("skip: pentagon/128 main ROM missing");
        return;
    };
    let Some(trdos) = trdos_rom_bytes() else {
        eprintln!("skip: roms/trdos.rom missing (optional #140 TR-DOS boot fixture)");
        return;
    };
    assert_eq!(
        trdos.get(0x3d94).copied(),
        Some(0xe7),
        "fixture ROM must keep stock RST #20 at 3D94h"
    );
    let mut m = Machine::new_pentagon128(&main, &trdos).unwrap();
    if let Some(beta) = m.beta_mut() {
        beta.page_trdos(true);
    }
    install_trdos_rst20_5cc2_hook(&mut m);
    assert!(
        m.beta_mut().is_some_and(|b| b.paged),
        "TR-DOS must be paged before 3D94h"
    );
    assert_eq!(
        m.read_mem(0x3d94),
        0xe7,
        "paged fetch at 3D94h should be RST #20"
    );
    let at20 = m.read_mem(0x0020);
    assert_eq!(
        at20, 0xc3,
        "paged fetch at 0020h should be TR-DOS JP 2F72h, got {at20:#04x}"
    );
    // RAM trampoline: `CALL 3D94h` / `HALT` — return addr sits below RST #20 traffic.
    const STUB: u16 = 0x8000;
    m.write_mem(STUB, 0xcd);
    m.write_mem(STUB + 1, 0x94);
    m.write_mem(STUB + 2, 0x3d);
    m.write_mem(STUB + 3, 0x76); // HALT
    m.cpu_mut().regs.sp = 0x6000;
    m.cpu_mut().regs.pc = STUB;
    let mut returned = false;
    for _ in 0..50_000 {
        m.step_once();
        if m.cpu().regs.pc == STUB + 3 {
            returned = true;
            break;
        }
    }
    let final_pc = m.cpu().regs.pc;
    let final_paged = m.beta_mut().is_some_and(|b| b.paged);
    assert!(
        returned,
        "3D94h RST #20 should return with 5CC2h hook (PC={final_pc:#06x}, paged={final_paged})"
    );
}

/// ROM-gated: find-boot catalog opcodes stay stock (ABI fixup, not ROM RET/NOP).
#[test]
fn trdos_find_boot_rom_unpatched_when_fixture_present() {
    let Some(main) = rom_pentagon().or_else(rom128) else {
        eprintln!("skip: pentagon/128 main ROM missing");
        return;
    };
    let Some(trdos) = trdos_rom_bytes() else {
        eprintln!("skip: roms/trdos.rom missing (optional #140 TR-DOS boot fixture)");
        return;
    };
    assert_eq!(
        trdos.get(0x1968).copied(),
        Some(0x0e),
        "stock LD C,0 at 1968h"
    );
    assert_eq!(
        trdos.get(0x1977).copied(),
        Some(0xed),
        "stock LD DE,(nn) at 1977h"
    );
    assert_eq!(
        trdos.get(0x1988).copied(),
        Some(0x2a),
        "stock LD HL,(nn) at 1988h"
    );
    assert_eq!(
        trdos.get(0x199a).copied(),
        Some(0x10),
        "stock DJNZ at 199Ah"
    );
    let mut m = Machine::new_pentagon128(&main, &trdos).unwrap();
    if let Some(beta) = m.beta_mut() {
        beta.page_trdos(true);
    }
    patch_trdos_run_harness_rom(&mut m);
    assert_eq!(
        m.read_mem(0x1968),
        0x0e,
        "harness must not patch 1968h LD C,0"
    );
    assert_eq!(
        m.read_mem(0x1977),
        0xed,
        "harness must not patch 1977h LD DE,(5CD9)"
    );
    assert_eq!(
        m.read_mem(0x1988),
        0x2a,
        "harness must not patch 1988h LD HL,(5CD7)"
    );
    assert_eq!(
        m.read_mem(0x199a),
        0x10,
        "harness must not RET-patch 199Ah DJNZ"
    );
}

/// ROM-gated: `3DFFh` delay loop stays stock (A=1 ABI, not ROM RET).
#[test]
fn trdos_3dff_delay_rom_unpatched_when_fixture_present() {
    let Some(main) = rom_pentagon().or_else(rom128) else {
        eprintln!("skip: pentagon/128 main ROM missing");
        return;
    };
    let Some(trdos) = trdos_rom_bytes() else {
        eprintln!("skip: roms/trdos.rom missing (optional #140 TR-DOS boot fixture)");
        return;
    };
    assert_eq!(
        trdos.get(0x3dff).copied(),
        Some(0x0e),
        "stock LD C,#FF at 3DFFh"
    );
    let mut m = Machine::new_pentagon128(&main, &trdos).unwrap();
    if let Some(beta) = m.beta_mut() {
        beta.page_trdos(true);
    }
    patch_trdos_run_harness_rom(&mut m);
    assert_eq!(
        m.read_mem(0x3dff),
        0x0e,
        "harness must not RET-patch 3DFFh delay"
    );
}

/// ROM-gated: CAT / VG93-wait / PROG-wipe sites stay stock (PC/RET ABI).
#[test]
fn trdos_cat_wait_rom_unpatched_when_fixture_present() {
    let Some(main) = rom_pentagon().or_else(rom128) else {
        eprintln!("skip: pentagon/128 main ROM missing");
        return;
    };
    let Some(trdos) = trdos_rom_bytes() else {
        eprintln!("skip: roms/trdos.rom missing (optional #140 TR-DOS boot fixture)");
        return;
    };
    assert_eq!(
        trdos.get(0x3d9d).copied(),
        Some(0xe7),
        "stock RST #20 at 3D9Dh"
    );
    assert_eq!(
        trdos.get(0x02d4).copied(),
        Some(0xcd),
        "stock CALL at 02D4h"
    );
    assert_eq!(
        trdos.get(0x213e).copied(),
        Some(0xcc),
        "stock CALL Z at 213Eh"
    );
    assert_eq!(trdos.get(0x2155).copied(), Some(0xc3), "stock JP at 2155h");
    let mut m = Machine::new_pentagon128(&main, &trdos).unwrap();
    if let Some(beta) = m.beta_mut() {
        beta.page_trdos(true);
    }
    patch_trdos_run_harness_rom(&mut m);
    assert_eq!(
        m.read_mem(0x3d9d),
        0xe7,
        "harness must not JR-patch 3D9Dh wait"
    );
    assert_eq!(
        m.read_mem(0x02d4),
        0xcd,
        "harness must not NOP 02D4h CAT CALL"
    );
    assert_eq!(
        m.read_mem(0x213e),
        0xcc,
        "harness must not NOP 213Eh CALL Z"
    );
    assert_eq!(
        m.read_mem(0x2155),
        0xc3,
        "harness must not RET-patch 2155h JP CAT"
    );
}

/// ROM-gated: `19ECh` stays stock `RST #20`/`08D2h`; padding is never patched.
#[test]
fn trdos_19ec_08d2_callsite_rom_unpatched_when_fixture_present() {
    let Some(main) = rom_pentagon().or_else(rom128) else {
        eprintln!("skip: pentagon/128 main ROM missing");
        return;
    };
    let Some(trdos) = trdos_rom_bytes_harness() else {
        eprintln!("skip: roms/trdos.rom missing (optional #140 TR-DOS boot fixture)");
        return;
    };
    assert_eq!(
        trdos.get(0x19ec).copied(),
        Some(0xe7),
        "stock RST #20 at 19ECh"
    );
    assert_eq!(
        trdos.get(0x19ed).copied(),
        Some(0xd2),
        "stock inline service lo at 19EDh"
    );
    assert_eq!(
        trdos.get(0x19ee).copied(),
        Some(0x08),
        "stock inline service hi → 08D2h"
    );
    let hole = !trdos_rom_fills_0800_hole(&trdos);
    if hole {
        assert_eq!(
            trdos.get(0x08d2).copied(),
            Some(0xff),
            "hole dump has FF padding at 08D2h"
        );
    }
    let mut m = Machine::new_pentagon128(&main, &trdos).unwrap();
    if let Some(beta) = m.beta_mut() {
        beta.page_trdos(true);
    }
    patch_trdos_run_harness_rom(&mut m);
    assert_eq!(
        m.read_mem(0x19ec),
        0xe7,
        "harness must not patch 19ECh RST #20"
    );
    assert_eq!(
        [m.read_mem(0x19ed), m.read_mem(0x19ee)],
        [0xd2, 0x08],
        "harness must not retarget 19ECh service word"
    );
    if hole {
        assert_eq!(
            m.read_mem(0x08d2),
            0xff,
            "harness must not write into 08D2h FF padding"
        );
    } else {
        assert_ne!(
            m.read_mem(0x08d2),
            0xff,
            "complete dump: harness must leave 08D2h service code intact"
        );
    }
}

/// ROM-gated: when a filled-hole dump is present, classify `08D2h`.
///
/// Soft-pass on the usual hole-filled 5.04 image (documents the blocker).
/// Alone Coder **5.04T** fills `0800h`+ with VG93 helpers — `08D2h` is a port
/// stub, not classic file-load — so `19ECh` still uses the FDC stand-in.
#[test]
fn trdos_native_file_services_gate_when_fixture_present() {
    let Some(trdos) = trdos_rom_bytes() else {
        eprintln!("skip: roms/trdos.rom missing (optional #140 TR-DOS boot fixture)");
        return;
    };
    if !trdos_rom_fills_0800_hole(&trdos) {
        assert_eq!(
            trdos.get(0x08d2).copied(),
            Some(0xff),
            "hole dump: 08D2h stays FF"
        );
        assert_eq!(
            trdos.get(0x0d6b).copied(),
            Some(0xff),
            "hole dump: 0D6Bh stays FF"
        );
        eprintln!(
            "trdos native file-services gate: hole dump (0800h–0E71h FF) — \
                 place trdos-5.04t.rom under roms/pentagon/ (Refs #140)"
        );
        return;
    }
    if trdos_rom_08d2_is_vg93_port_stub(&trdos) {
        assert!(!trdos_rom_has_native_file_services(&trdos));
        eprintln!(
            "trdos native file-services gate: 5.04T VG93 port stub at 08D2h — \
                 RUN boot uses 19ECh FDC stand-in after catalog match (Refs #140)"
        );
        return;
    }
    assert!(trdos_rom_has_native_file_services(&trdos));
    assert_eq!(
        [trdos.get(0x19ec), trdos.get(0x19ed), trdos.get(0x19ee)],
        [Some(&0xe7), Some(&0xd2), Some(&0x08)],
        "classic complete dump should keep stock 19ECh → 08D2h"
    );
    eprintln!(
        "trdos native file-services gate: OPEN — classic file-load at 08D2h \
             (native RUN path eligible; Refs #140)"
    );
}

/// ROM-gated: native `012Ah` / `1D97h` stay stock; `0D6Bh` hole is not patched.
#[test]
fn trdos_012a_0d6b_service_rom_unpatched_when_fixture_present() {
    let Some(main) = rom_pentagon().or_else(rom128) else {
        eprintln!("skip: pentagon/128 main ROM missing");
        return;
    };
    let Some(trdos) = trdos_rom_bytes_harness() else {
        eprintln!("skip: roms/trdos.rom missing (optional #140 TR-DOS boot fixture)");
        return;
    };
    assert_eq!(
        trdos.get(0x012a).copied(),
        Some(0xcd),
        "stock CALL at 012Ah"
    );
    assert_eq!(
        [trdos.get(0x012b).copied(), trdos.get(0x012c).copied()],
        [Some(0xe5), Some(0x20)],
        "stock CALL 20E5h at 012Ah"
    );
    assert_eq!(
        [
            trdos.get(0x012d).copied(),
            trdos.get(0x012e).copied(),
            trdos.get(0x012f).copied()
        ],
        [Some(0xcd), Some(0x97), Some(0x1d)],
        "stock CALL 1D97h after 20E5h"
    );
    assert_eq!(
        [
            trdos.get(0x1d97).copied(),
            trdos.get(0x1d98).copied(),
            trdos.get(0x1d99).copied()
        ],
        [Some(0xe7), Some(0x6b), Some(0x0d)],
        "stock RST #20 / 0D6Bh at 1D97h"
    );
    assert_eq!(
        trdos.get(0x1d9a).copied(),
        Some(0xc9),
        "stock RET after 1D97h service word"
    );
    let hole = !trdos_rom_fills_0800_hole(&trdos);
    if hole {
        assert_eq!(
            trdos.get(0x0d6b).copied(),
            Some(0xff),
            "hole dump has FF padding at 0D6Bh (0800h–0E71h)"
        );
    } else {
        assert_ne!(
            trdos.get(0x0d6b).copied(),
            Some(0xff),
            "complete dump has code at 0D6Bh"
        );
    }
    assert_eq!(
        trdos.get(0x16b0).copied(),
        Some(0x16),
        "16B0h is the high byte of CALL 166Fh, not a RST #20 body"
    );
    let mut m = Machine::new_pentagon128(&main, &trdos).unwrap();
    if let Some(beta) = m.beta_mut() {
        beta.page_trdos(true);
    }
    patch_trdos_run_harness_rom(&mut m);
    assert_eq!(
        [m.read_mem(0x012a), m.read_mem(0x012d), m.read_mem(0x1d97)],
        [0xcd, 0xcd, 0xe7],
        "harness must not patch 012Ah / 1D97h"
    );
    if hole {
        assert_eq!(
            m.read_mem(0x0d6b),
            0xff,
            "harness must not write into 0D6Bh FF padding"
        );
    } else {
        assert_ne!(
            m.read_mem(0x0d6b),
            0xff,
            "complete dump: harness must leave 0D6Bh service code intact"
        );
    }
}

/// ROM-gated: TR-DOS `RUN` (no filename) loads synthetic `boot` → `POKE 32768,165`.
#[test]
fn trdos_rom_run_boot_basic_when_fixture_present() {
    let Some(main) = rom_pentagon().or_else(rom128) else {
        eprintln!("skip: pentagon/128 main ROM missing");
        return;
    };
    let Some(trdos) = trdos_rom_bytes_harness() else {
        eprintln!("skip: roms/trdos.rom missing (optional #140 TR-DOS boot fixture)");
        return;
    };
    let mut m = Machine::new_pentagon128(&main, &trdos).unwrap();
    m.insert_trd(formats::TrdImage::synthetic_trdos_boot_basic())
        .unwrap();
    enter_128k_basic_from_menu(&mut m);
    ensure_trdos_beta128_prog(&mut m);
    assert!(
        enter_trdos_command_mode(&mut m),
        "TR-DOS command prompt (3D31h) not reached (PC={:#06x})",
        m.cpu().regs.pc
    );
    let ok = invoke_trdos_run_boot(&mut m);
    let pc = m.cpu().regs.pc;
    let (sectors, track, ring) = m.beta_mut().map_or((0, 0, Vec::new()), |b| {
        (b.sector_read_count, b.track, b.command_ring().to_vec())
    });
    assert!(
            ok,
            "TR-DOS RUN boot should POKE 32768,165 (PC={pc:#06x}, sectors={sectors}, track={track}, ring={ring:02x?}, 8000={:#04x})",
            m.read_mem(0x8000)
        );
    assert_eq!(m.read_mem(0x8000), 0xa5, "boot BASIC POKE 32768,165");
}

/// 5.04T: `08D2h` is a VG93 port stub, so `19ECh` must still take the FDC stand-in.
#[test]
fn trdos_19ec_takes_fdc_standin_when_504t_port_stub_present() {
    let Some(main) = rom_pentagon().or_else(rom128) else {
        eprintln!("skip: pentagon/128 main ROM missing");
        return;
    };
    let Some(trdos) = trdos_rom_bytes_complete() else {
        eprintln!(
            "skip: complete TR-DOS dump missing — place trdos-5.04t.rom / \
                 trdos-complete.rom under roms/pentagon/ (Refs #140)"
        );
        return;
    };
    assert!(trdos_rom_fills_0800_hole(&trdos));
    if !trdos_rom_08d2_is_vg93_port_stub(&trdos) {
        eprintln!("skip: complete dump has classic 08D2h file-load (not 5.04T stub)");
        return;
    }
    assert!(!trdos_rom_has_native_file_services(&trdos));
    let mut m = Machine::new_pentagon128(&main, &trdos).unwrap();
    m.insert_trd(formats::TrdImage::synthetic_trdos_boot_basic())
        .unwrap();
    if let Some(beta) = m.beta_mut() {
        beta.page_trdos(true);
    }
    m.write_mem(0x5d25 + 8, b'B');
    m.write_mem(0x5d25 + 9, 28);
    m.write_mem(0x5d25 + 10, 0);
    m.write_mem(0x5d25 + 14, 1);
    m.write_mem(0x5d25 + 15, 1);
    m.write_mem(0x5c59, 0x00);
    m.write_mem(0x5c5a, 0x5e);
    m.cpu_mut().regs.pc = 0x19ec;
    apply_trdos_run_native_abi(&mut m);
    assert_eq!(
        m.cpu().regs.pc,
        0x1b76,
        "5.04T port-stub 08D2h: 19ECh must FDC-load and enter LINE-NEW"
    );
}

/// 5.04T RUN→boot: Type-II BUSY so catalog `1E3Dh` returns to `1981h`; `08D2h` is a
/// VG93 port stub so `19ECh` uses the FDC/`LINE-NEW` stand-in (hard `0x8000==0xA5`).
#[test]
fn trdos_rom_run_boot_504t_catalog_match_when_complete_present() {
    let Some(main) = rom_pentagon().or_else(rom128) else {
        return;
    };
    let Some(trdos) = trdos_rom_bytes_complete() else {
        eprintln!(
            "skip: complete TR-DOS dump missing — place trdos-5.04t.rom / \
                 trdos-complete.rom under roms/pentagon/ (Refs #140)"
        );
        return;
    };
    if !trdos_rom_08d2_is_vg93_port_stub(&trdos) {
        eprintln!("skip: complete dump has classic 08D2h file-load (not 5.04T stub)");
        return;
    }
    let mut m = Machine::new_pentagon128(&main, &trdos).unwrap();
    m.insert_trd(formats::TrdImage::synthetic_trdos_boot_basic())
        .unwrap();
    enter_128k_basic_from_menu(&mut m);
    ensure_trdos_beta128_prog(&mut m);
    if let Some(beta) = m.beta_mut() {
        beta.page_trdos(true);
    }
    init_trdos_usr_call_frame(&mut m);
    assert!(
        invoke_trdos_run_boot(&mut m),
        "5.04T RUN boot should POKE 32768,165 after catalog match + 19ECh stand-in"
    );
    assert_eq!(m.read_mem(0x8000), 0xa5);
}

/// Mid-instruction ULA time: `frame_t` at insn start + `(cpu.t - t_step_start)`.

#[test]
fn interface1_opcode_fetch_pages_shadow_rom() {
    let rom = [0u8; 16384];
    let mut m = Machine::new_48k(&rom).unwrap();
    let if1 = m.attach_interface1().unwrap();
    let mut if1_rom = [0u8; bus::IF1_ROM_SIZE];
    if1_rom[0x0008] = 0x00; // NOP
    if1_rom[0x0700] = 0x00; // NOP — post-fetch unpages
    if1.load_rom(&if1_rom).unwrap();
    m.cpu_mut().regs.pc = 0x0008;
    m.step_cpu_only();
    assert!(
        m.interface1_mut().unwrap().rom_paged,
        "IF1 should stay paged after fetch at 0x0008 (until 0x0700)"
    );
    m.cpu_mut().regs.pc = 0x0700;
    m.interface1_mut().unwrap().page_rom(true);
    m.step_cpu_only();
    assert!(
        !m.interface1_mut().unwrap().rom_paged,
        "IF1 should unpage after opcode fetch at 0x0700"
    );
}

#[test]
fn interface1_rom_load_skips_cleanly_when_missing() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/if1.rom");
    if !path.is_file() {
        eprintln!("skip: roms/if1.rom missing");
        return;
    }
    let rom = [0u8; 16384];
    let mut m = Machine::new_48k(&rom).unwrap();
    let data = std::fs::read(&path).unwrap();
    m.load_interface1_rom(&data).unwrap();
    assert!(m.interface1_rom_loaded());
}

/// User-supplied IF1 ROM soak: boot 48K, insert Fuse-formatted MDR, `CAT 1`,
/// wait for motor-off, expect `ERR_NR==OK`.
///
/// Skips cleanly when `roms/if1.rom` or `roms/spec48.rom` is absent (never commit IF1 dumps).
/// Refs [#139](https://github.com/mward-sudo/spec_chum/issues/139) /
/// [#397](https://github.com/mward-sudo/spec_chum/issues/397).
#[test]
fn interface1_real_rom_cat_formatted_mdr_skips_when_missing() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let if1_path = root.join("roms/if1.rom");
    let Some(sys) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    if !if1_path.is_file() {
        eprintln!("skip: roms/if1.rom missing — place a user IF1 dump to soak (see #188)");
        return;
    }
    let if1_rom = std::fs::read(&if1_path).expect("read if1.rom");
    assert_eq!(if1_rom.len(), bus::IF1_ROM_SIZE);

    let mut m = Machine::new_48k(&sys).unwrap();
    m.load_interface1_rom(&if1_rom).unwrap();
    m.interface1_mut()
        .unwrap()
        .insert_mdr(formats::MdrImage::formatted("CART"));

    // Copyright → BASIC K cursor (do not poke PC — keep ROM debounce / CHAN state intact).
    for _ in 0..400 {
        let _ = m.run_frame();
    }
    m.wait_48_basic_prompt(500);

    // Shadow ROM pages on RST 8 and exposes the real IF1 image (probe then restore).
    {
        let saved_pc = m.cpu().regs.pc;
        m.cpu_mut().regs.pc = 0x0008;
        m.step_cpu_only();
        assert!(m.interface1_mut().unwrap().rom_paged);
        assert_eq!(m.read_mem(0x0000), if1_rom[0]);
        assert_eq!(m.read_mem(0x0001), if1_rom[1]);
        m.interface1_mut().unwrap().page_rom(false);
        m.cpu_mut().regs.pc = saved_pc;
    }

    // Caps+Symbol → Extended mode so Sym+9 yields CAT (not ')').
    const PRESS: u32 = 12;
    const GAP: u32 = 8;
    m.hold_keys(&[(0, 0), (7, 1)], PRESS);
    m.hold_keys(&[], GAP);
    let e_line = u16::from(m.read_mem(0x5c59)) | (u16::from(m.read_mem(0x5c5a)) << 8);
    m.hold_keys(&[(7, 1), (4, 1)], PRESS); // Symbol + 9 = CAT
    m.hold_keys(&[], GAP);
    let cat_tok = m.read_mem(e_line);
    assert_eq!(
        cat_tok, 0xcf,
        "expected CAT token 0xCF at E_LINE, got {cat_tok:#04x}"
    );
    m.hold_keys(&[(3, 0)], PRESS); // 1
    m.hold_keys(&[], GAP);
    m.hold_keys(&[(6, 0)], PRESS); // Enter
    m.hold_keys(&[], 20);
    m.keyboard_mut().reset();

    // IF1 presets ERR_NR to $17 ("Microdrive not present") while hunting
    // GAP/SYNC; only the value after the motor stops is authoritative.
    let mut saw_motor = false;
    let mut saw_motor_off = false;
    let mut last_err = 0xffu8;
    for _ in 0..25_000u32 {
        let motor = m.interface1_mut().is_some_and(|i| i.any_motor_on());
        if motor {
            saw_motor = true;
        } else if saw_motor {
            saw_motor_off = true;
        }
        let _ = m.run_frame();
        last_err = m.read_mem(0x5c3a);
        if saw_motor_off && !motor {
            break;
        }
    }
    assert!(
        saw_motor,
        "CAT should select a Microdrive motor at least once"
    );
    assert!(
        saw_motor_off,
        "CAT should release the motor (ERR_NR={last_err:#04x})"
    );
    assert_eq!(last_err, 0xff, "CAT should end with ERR_NR OK");
    assert!(
        m.interface1_mut().unwrap().drive_checksums_ok(0),
        "formatted MDR checksums should survive CAT"
    );
}

/// User-supplied IF1 ROM: `FORMAT "m";1;"T"` then motor-off, `ERR_NR==OK`, formatted MDR.
#[test]
fn interface1_real_rom_format_mdr_skips_when_missing() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let if1_path = root.join("roms/if1.rom");
    let Some(sys) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    if !if1_path.is_file() {
        eprintln!("skip: roms/if1.rom missing — place a user IF1 dump to soak (see #188)");
        return;
    }
    let if1_rom = std::fs::read(&if1_path).expect("read if1.rom");

    let mut m = Machine::new_48k(&sys).unwrap();
    m.load_interface1_rom(&if1_rom).unwrap();
    m.interface1_mut()
        .unwrap()
        .insert_mdr(formats::MdrImage::blank());
    assert!(!m.interface1_mut().unwrap().mdr().unwrap().looks_formatted());

    for _ in 0..400 {
        let _ = m.run_frame();
    }
    m.wait_48_basic_prompt(500);

    // Caps+Symbol → Extended mode so Sym+0 yields FORMAT (not '_').
    const PRESS: u32 = 12;
    const GAP: u32 = 8;
    m.hold_keys(&[(0, 0), (7, 1)], PRESS);
    m.hold_keys(&[], GAP);
    let e_line = u16::from(m.read_mem(0x5c59)) | (u16::from(m.read_mem(0x5c5a)) << 8);
    m.hold_keys(&[(7, 1), (4, 0)], PRESS); // Symbol + 0 = FORMAT
    m.hold_keys(&[], GAP);
    assert_eq!(m.read_mem(e_line), 0xd0, "expected FORMAT token 0xD0");
    m.hold_keys(&[(7, 1), (5, 0)], PRESS); // "
    m.hold_keys(&[], GAP);
    m.hold_keys(&[(7, 2)], PRESS); // m
    m.hold_keys(&[], GAP);
    m.hold_keys(&[(7, 1), (5, 0)], PRESS); // "
    m.hold_keys(&[], GAP);
    m.hold_keys(&[(7, 1), (5, 1)], PRESS); // ;
    m.hold_keys(&[], GAP);
    m.hold_keys(&[(3, 0)], PRESS); // 1
    m.hold_keys(&[], GAP);
    m.hold_keys(&[(7, 1), (5, 1)], PRESS); // ;
    m.hold_keys(&[], GAP);
    m.hold_keys(&[(7, 1), (5, 0)], PRESS); // "
    m.hold_keys(&[], GAP);
    m.hold_keys(&[(2, 4)], PRESS); // T
    m.hold_keys(&[], GAP);
    m.hold_keys(&[(7, 1), (5, 0)], PRESS); // "
    m.hold_keys(&[], GAP);
    m.hold_keys(&[(6, 0)], PRESS); // Enter
    m.hold_keys(&[], 20);
    m.keyboard_mut().reset();

    let mut saw_motor = false;
    let mut saw_motor_off = false;
    let mut last_err = 0xffu8;
    // FORMAT rewrites every sector; allow a long soak (byte-level MDR is still
    // many revolutions of ROM loops).
    for _ in 0..80_000u32 {
        let motor = m.interface1_mut().is_some_and(|i| i.any_motor_on());
        if motor {
            saw_motor = true;
        } else if saw_motor {
            saw_motor_off = true;
        }
        let _ = m.run_frame();
        last_err = m.read_mem(0x5c3a);
        if saw_motor_off && !motor {
            break;
        }
    }
    assert!(saw_motor, "FORMAT should run a Microdrive motor");
    assert!(
        saw_motor_off,
        "FORMAT should release the motor (ERR_NR={last_err:#04x})"
    );
    assert_eq!(last_err, 0xff, "FORMAT should end with ERR_NR OK");
    assert!(
        m.interface1_mut()
            .unwrap()
            .mdr()
            .is_some_and(formats::MdrImage::looks_formatted),
        "FORMAT should leave a Fuse-layout formatted cartridge"
    );

    // FORMAT blank → CAT empty (acceptance): Extended-mode CAT 1 after OK.
    for _ in 0..200 {
        let _ = m.run_frame();
    }
    m.wait_48_basic_prompt(500);
    m.hold_keys(&[(0, 0), (7, 1)], PRESS);
    m.hold_keys(&[], GAP);
    let e_line = u16::from(m.read_mem(0x5c59)) | (u16::from(m.read_mem(0x5c5a)) << 8);
    m.hold_keys(&[(7, 1), (4, 1)], PRESS); // CAT
    m.hold_keys(&[], GAP);
    assert_eq!(m.read_mem(e_line), 0xcf, "post-FORMAT CAT token");
    m.hold_keys(&[(3, 0)], PRESS); // 1
    m.hold_keys(&[], GAP);
    m.hold_keys(&[(6, 0)], PRESS); // Enter
    m.hold_keys(&[], 20);
    m.keyboard_mut().reset();

    let mut cat_motor = false;
    let mut cat_motor_off = false;
    let mut cat_err = 0xffu8;
    for _ in 0..25_000u32 {
        let motor = m.interface1_mut().is_some_and(|i| i.any_motor_on());
        if motor {
            cat_motor = true;
        } else if cat_motor {
            cat_motor_off = true;
        }
        let _ = m.run_frame();
        cat_err = m.read_mem(0x5c3a);
        if cat_motor_off && !motor {
            break;
        }
    }
    assert!(
        cat_motor && cat_motor_off,
        "post-FORMAT CAT should run motor"
    );
    assert_eq!(cat_err, 0xff, "post-FORMAT CAT should end with ERR_NR OK");
}

#[test]
fn memio_mid_instruction_contention_table() {
    // Contended screen address; FAQ Contended I/O ports.
    const ADDR: u16 = 0x4000;
    const FE: u16 = 0x00FE;
    const FF: u16 = 0x00FF;
    const HI_FE: u16 = 0x40FE;
    const HI_FF: u16 = 0x40FF;
    const C0_FE: u16 = 0xC0FE;
    // (frame_t, dt, mem_wait, io_FE, io_40FE, io_40FF)
    const ROWS_48: &[(u32, u64, u32, u32, u32, u32)] = &[
        (ula::PAPER_START_48, 0, 6, 5, 6, 12),
        (ula::PAPER_START_48, 1, 5, 4, 5, 11),
        (ula::PAPER_START_48, 2, 4, 3, 4, 10),
        (ula::PAPER_START_48, 3, 3, 2, 3, 9),
        (ula::PAPER_START_48, 4, 2, 1, 2, 8),
        (ula::PAPER_START_48, 5, 1, 0, 1, 7),
        (ula::PAPER_START_48, 6, 0, 0, 0, 6),
        (ula::PAPER_START_48, 7, 0, 6, 6, 12),
    ];
    const ROWS_128: &[(u32, u64, u32, u32)] = &[
        (ula::PAPER_START_128, 0, 6, 5),
        (ula::PAPER_START_128, 1, 5, 4),
        (ula::PAPER_START_128, 2, 4, 3),
        (ula::PAPER_START_128, 3, 3, 2),
        (ula::PAPER_START_128, 4, 2, 1),
        (ula::PAPER_START_128, 5, 1, 0),
        (ula::PAPER_START_128, 6, 0, 0),
        (ula::PAPER_START_128, 7, 0, 6),
    ];
    // C:1,C:3 totals when high byte contends at paper start + dt.
    const C0_CONTENDED: [u32; 8] = [6, 5, 4, 3, 2, 1, 0, 6];

    for &(frame_t, dt, expect, io_fe, io_hife, io_hiff) in ROWS_48 {
        let mut bus = Bus48::new();
        bus.frame_t = frame_t;
        let mut mem = MemIo48 {
            bus: &mut bus,
            watch: None,
            t_step_start: 100,
            opcode_pc: None,
        };
        let t = 100 + dt;
        assert_eq!(
            mem.read(ADDR, t).1,
            expect,
            "48 mem R frame={frame_t} dt={dt}"
        );
        assert_eq!(
            mem.write(ADDR, 0, t),
            expect,
            "48 mem W frame={frame_t} dt={dt}"
        );
        assert_eq!(
            mem.in_port(FE, t).1,
            io_fe,
            "48 IN FE frame={frame_t} dt={dt}"
        );
        assert_eq!(mem.in_port(FF, t).1, 0, "48 IN FF never contends");
        assert_eq!(
            mem.in_port(HI_FE, t).1,
            io_hife,
            "48 IN 40FE frame={frame_t} dt={dt}"
        );
        assert_eq!(
            mem.in_port(HI_FF, t).1,
            io_hiff,
            "48 IN 40FF frame={frame_t} dt={dt}"
        );
        // Uncontended high RAM
        assert_eq!(mem.read(0x8000, t).1, 0);
    }

    for &(frame_t, dt, expect, io_fe) in ROWS_128 {
        let mut bus = Bus128::new();
        bus.frame_t = frame_t;
        // Default page = bank 0 at C000 (uncontended).
        let mut mem = MemIo128 {
            bus: &mut bus,
            watch: None,
            t_step_start: 100,
            opcode_pc: None,
            pentagon: false,
        };
        let t = 100 + dt;
        assert_eq!(
            mem.read(ADDR, t).1,
            expect,
            "128 mem R frame={frame_t} dt={dt}"
        );
        assert_eq!(
            mem.in_port(FE, t).1,
            io_fe,
            "128 IN FE frame={frame_t} dt={dt}"
        );
        // High 0xC0 with uncontended bank 0: same as FE (N:1,C:3).
        assert_eq!(
            mem.in_port(C0_FE, t).1,
            io_fe,
            "128 IN C0FE bank0 frame={frame_t} dt={dt}"
        );

        // Page contended bank 1 at C000 → C0FE uses C:1,C:3 (same totals as 40FE).
        mem.bus.out_7ffd(1);
        assert_eq!(
            mem.in_port(C0_FE, t).1,
            C0_CONTENDED[dt as usize],
            "128 IN C0FE bank1 frame={frame_t} dt={dt}"
        );
        // Reset page for next iteration clarity
        mem.bus.page = 0;
        mem.bus.locked = false;

        let mut bus3 = BusPlus3::new();
        bus3.frame_t = frame_t;
        let mut mem3 = MemIoPlus3 {
            bus: &mut bus3,
            watch: None,
            t_step_start: 100,
        };
        assert_eq!(
            mem3.read(ADDR, t).1,
            expect,
            "+3 mem R frame={frame_t} dt={dt}"
        );
        assert_eq!(mem3.in_port(FE, t).1, 0, "+3 I/O never contends");
        assert_eq!(mem3.in_port(HI_FF, t).1, 0, "+3 I/O never contends");
        assert_eq!(mem3.in_port(0x2ffd, t).1, 0, "+3 FDC status uncontended");
        assert_eq!(mem3.in_port(0x3ffd, t).1, 0, "+3 FDC data uncontended");
    }

    // Access after the instruction has already burned into the contended window.
    let mut bus = Bus48::new();
    bus.frame_t = ula::PAPER_START_48.wrapping_sub(3);
    let mut mem = MemIo48 {
        bus: &mut bus,
        watch: None,
        t_step_start: 50,
        opcode_pc: None,
    };
    assert_eq!(
        mem.read(ADDR, 53).1,
        6,
        "ULA time = frame_t+3 lands on first contended cycle"
    );
}

/// Original Sinclair ULA snow: M1 refresh hook records overrides on 48K/128K paths.
#[test]
fn m1_refresh_records_snow_48k() {
    use z80::Memory;
    let mut bus = Bus48::new();
    bus.frame_t = ula::PAPER_START_48 + 3;
    bus.ram[0] = 0xAA;
    bus.ram[1] = 0x55;
    let mut mem = MemIo48 {
        bus: &mut bus,
        watch: None,
        t_step_start: 0,
        opcode_pc: None,
    };
    mem.m1_refresh(0x4001, 0, false);
    assert!(
        !bus.ula.snow_overrides().is_empty(),
        "48K-class ULA must record snow overrides"
    );
}

#[test]
fn m1_refresh_skips_snow_when_m1_contended() {
    use z80::Memory;
    let mut bus = Bus48::new();
    bus.frame_t = ula::PAPER_START_48 + 3;
    bus.ram[0] = 0xAA;
    bus.ram[1] = 0x55;
    let mut mem = MemIo48 {
        bus: &mut bus,
        watch: None,
        t_step_start: 0,
        opcode_pc: None,
    };
    mem.m1_refresh(0x4001, 0, true);
    assert!(
        bus.ula.snow_overrides().is_empty(),
        "contended M1 must block snow"
    );
}

/// Each M1 refresh uses contention from that fetch, not a stale `opcode_pc` marker.
#[test]
fn m1_refresh_per_fetch_contention_not_stale() {
    use z80::Memory;
    let mut bus = Bus48::new();
    bus.frame_t = ula::PAPER_START_48 + 3;
    bus.ram[0] = 0xAA;
    bus.ram[1] = 0x55;
    {
        let mut mem = MemIo48 {
            bus: &mut bus,
            watch: None,
            t_step_start: 0,
            opcode_pc: Some(0x4000),
        };
        let (_, wait1) = mem.read(0x4000, 0);
        mem.m1_refresh(0x4001, 0, wait1 > 0);
    }
    assert!(
        bus.ula.snow_overrides().is_empty(),
        "contended first fetch must not snow"
    );
    {
        let mut mem = MemIo48 {
            bus: &mut bus,
            watch: None,
            t_step_start: 0,
            opcode_pc: None,
        };
        let (_, wait2) = mem.read(0x8000, 0);
        mem.m1_refresh(0x4001, 0, wait2 > 0);
    }
    assert!(
        !bus.ula.snow_overrides().is_empty(),
        "uncontended second M1 must snow even after contended first fetch"
    );
}

#[test]
fn m1_refresh_records_snow_128k() {
    use z80::Memory;
    let mut bus = Bus128::new();
    bus.frame_t = ula::PAPER_START_128 + 3;
    bus.banks[5][0] = 0xAA;
    bus.banks[5][1] = 0x55;
    let mut mem = MemIo128 {
        bus: &mut bus,
        watch: None,
        t_step_start: 0,
        opcode_pc: None,
        pentagon: false,
    };
    mem.m1_refresh(0x4001, 0, false);
    assert!(
        !bus.ula.snow_overrides().is_empty(),
        "128K/grey+2 ULA must record snow overrides"
    );
}

/// Pentagon 128: no memory contention → no ULA snow hook effect.
#[test]
fn m1_refresh_pentagon128_skips_snow() {
    use z80::Memory;
    let mut bus = Bus128::new();
    bus.frame_t = ula::PAPER_START_128 + 3;
    bus.banks[5][0] = 0xAA;
    bus.banks[5][1] = 0x55;
    let mut mem = MemIo128 {
        bus: &mut bus,
        watch: None,
        t_step_start: 0,
        opcode_pc: None,
        pentagon: true,
    };
    mem.m1_refresh(0x4001, 0, false);
    assert!(
        bus.ula.snow_overrides().is_empty(),
        "Pentagon must not apply ULA snow"
    );
}

/// Amstrad +2A/+3: different ULA — default `m1_refresh` is a no-op.
#[test]
fn m1_refresh_plus3_skips_snow() {
    use z80::Memory;
    let mut bus = BusPlus3::new_with_disk(true);
    bus.frame_t = ula::PAPER_START_128 + 3;
    let mut mem = MemIoPlus3 {
        bus: &mut bus,
        watch: None,
        t_step_start: 0,
    };
    mem.m1_refresh(0x4001, 0, false);
    assert!(
        bus.ula.snow_overrides().is_empty(),
        "+3 Amstrad ULA must not apply snow"
    );
}

#[test]
fn boot_frames_advance() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let mut m = Machine::new_48k(&rom).unwrap();
    for _ in 0..10 {
        m.run_frame();
    }
    assert!(m.cpu().t > 0);
}

#[test]
fn tape_ear_toggles_when_player_inserted() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let img = TapImage::load(&fixture_tap()).expect("fixture");
    let mut m = Machine::new_48k(&rom).unwrap();
    m.set_tape_load_options(TapeLoadOptions {
        flash_load: false,
        speed: 1,
        ..Default::default()
    });
    assert!(!m.ear(), "EAR idle without tape");
    m.insert_tape(TapPlayer::new(img));
    m.set_tape_playing(true);
    // Pilot starts high; first instruction group must raise EAR.
    let _ = m.run_frame();
    // Parallel probe advanced by one frame of T-states should share block index.
    let mut probe = TapPlayer::new(TapImage::load(&fixture_tap()).expect("fixture"));
    probe.advance(FRAME_TSTATES_48);
    assert_eq!(m.tape_block(), Some(probe.block));
    // EAR must leave the idle-low power-on default during pilot.
    let mut saw_high = m.ear();
    for _ in 0..5 {
        let _ = m.run_frame();
        if m.ear() {
            saw_high = true;
            break;
        }
    }
    assert!(saw_high, "pilot tone must drive EAR high");
}

#[test]
fn tape_tracks_frame_tstates() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let img = TapImage {
        blocks: vec![vec![0x00]],
        ..Default::default()
    };
    let mut m = Machine::new_48k(&rom).unwrap();
    m.set_tape_load_options(TapeLoadOptions {
        flash_load: false,
        speed: 1,
        ..Default::default()
    });
    m.insert_tape(TapPlayer::new(img.clone()));
    m.set_tape_playing(true);
    let mut probe = TapPlayer::new(img);
    for _ in 0..10 {
        m.run_frame();
        probe.advance(FRAME_TSTATES_48);
        assert_eq!(
            m.tape_block(),
            Some(probe.block),
            "tape must advance ~FRAME_TSTATES per frame (not 1 T/instruction)"
        );
    }
}

#[test]
fn tape_paused_does_not_advance_ear_until_play() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let img = TapImage::load(&fixture_tap()).expect("fixture");
    let mut m = Machine::new_48k(&rom).unwrap();
    m.set_tape_load_options(TapeLoadOptions {
        flash_load: false,
        speed: 1,
        ..Default::default()
    });
    m.insert_tape(TapPlayer::new(img));
    assert!(!m.tape_playing());
    let block0 = m.tape_block();
    for _ in 0..5 {
        let _ = m.run_frame();
    }
    assert_eq!(m.tape_block(), block0, "paused tape must not advance");
    assert!(!m.ear(), "paused pilot must not drive EAR");
    m.set_tape_playing(true);
    let mut saw_high = false;
    for _ in 0..5 {
        let _ = m.run_frame();
        if m.ear() {
            saw_high = true;
            break;
        }
    }
    assert!(saw_high, "Play must advance EAR during pilot");
    assert!(
        m.tape_block() != block0 || m.ear(),
        "Play should move the deck or at least raise EAR"
    );
}

#[test]
fn flash_load_trap_loads_data_block() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let img = TapImage::load(&fixture_tap()).expect("fixture");
    let data = img.blocks[1].clone();
    let mut m = Machine::new_48k(&rom).unwrap();
    m.set_tape_load_options(TapeLoadOptions {
        flash_load: true,
        speed: 1,
        ..Default::default()
    });
    // Skip header block so trap sees the data block.
    let mut player = TapPlayer::new(img);
    player.consume_block();
    m.insert_tape(player);
    m.set_tape_playing(true);

    // Set up a fake CALL return address and LD-BYTES register state.
    let ret = 0x1234u16;
    m.cpu_mut().regs.sp = 0x5f00;
    m.write_mem(0x5f00, (ret & 0xff) as u8);
    m.write_mem(0x5f01, (ret >> 8) as u8);
    m.cpu_mut().regs.pc = LD_BYTES_TRAP_PC;
    // Mimic ROM after EX AF,AF': flag + carry live in A′/F′; A is dirty.
    m.cpu_mut().regs.a = 0x0f;
    m.cpu_mut().regs.f = 0;
    m.cpu_mut().regs.a_ = 0xff;
    m.cpu_mut().regs.f_ = flag::C;
    m.cpu_mut().regs.set_ix(0x8000);
    m.cpu_mut().regs.set_de((data.len() - 2) as u16);

    // Avoid IRQ at frame_t=0 stealing control before the trap runs.
    if let Machine::Spec48 { bus, .. } = &mut m {
        bus.frame_t = INT_LENGTH_48;
    }
    m.step_once();

    assert_eq!(m.cpu().regs.pc, ret, "trap should RET");
    assert_eq!(m.read_mem(0x8000), 0x21);
    assert_eq!(m.read_mem(0x8001), 0x00);
    assert_eq!(m.read_mem(0x8002), 0x40);
    assert_eq!(m.read_mem(0x8003), 0x36);
    assert_eq!(m.read_mem(0x8004), 0x42);
    assert_eq!(m.read_mem(0x8005), 0xc9);
    assert_eq!(m.tape_block(), Some(2));
}

#[test]
fn ld_bytes_waits_while_tape_paused_then_flash_loads_on_play() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let img = TapImage::load(&fixture_tap()).expect("fixture");
    let header = img.blocks[0].clone();
    let mut m = Machine::new_48k(&rom).unwrap();
    m.set_tape_load_options(TapeLoadOptions {
        flash_load: true,
        speed: 1,
        ..Default::default()
    });
    m.insert_tape(TapPlayer::new(img));
    assert!(!m.tape_playing());

    let ret = 0x12abu16;
    m.cpu_mut().regs.sp = 0x5f00;
    m.write_mem(0x5f00, (ret & 0xff) as u8);
    m.write_mem(0x5f01, (ret >> 8) as u8);
    m.cpu_mut().regs.pc = LD_BYTES_TRAP_PC;
    m.cpu_mut().regs.a = 0x0f;
    m.cpu_mut().regs.f = 0;
    m.cpu_mut().regs.a_ = 0x00; // header flag in A′
    m.cpu_mut().regs.f_ = flag::C;
    m.cpu_mut().regs.set_ix(0x5c00);
    m.cpu_mut().regs.set_de((header.len() - 2) as u16);
    if let Machine::Spec48 { bus, .. } = &mut m {
        bus.frame_t = INT_LENGTH_48;
    }

    // While paused, ROM must not run past LD-BYTES (the old stall root cause).
    for _ in 0..64 {
        m.step_once();
        assert_eq!(
            m.cpu().regs.pc,
            LD_BYTES_TRAP_PC,
            "must hold at LD-BYTES until Play"
        );
    }

    m.set_tape_playing(true);
    m.step_once();
    assert_eq!(m.cpu().regs.pc, ret, "Play should flash-load and RET");
    assert_eq!(m.tape_block(), Some(1));
    // Header payload: type + filename etc. — first data byte after flag is type.
    assert_eq!(m.read_mem(0x5c00), header[1]);
}

#[test]
fn tape_progress_reports_blocks_and_fraction() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let img = TapImage::load(&fixture_tap()).expect("fixture");
    let n = img.blocks.len() as u32;
    let mut m = Machine::new_48k(&rom).unwrap();
    m.insert_tape(TapPlayer::new(img));
    let p = m.tape_progress().expect("progress");
    assert_eq!(p.block_index, 0);
    assert_eq!(p.block_count, n);
    assert!(p.pulse_count > 0);
    assert!(p.fraction() < 1.0);
}

#[test]
fn tape_progress_trailing_empty_block_reports_complete() {
    // Mirrors TZX ending in a consumed 0x20 pause_ms=0 (zero pulses on final block).
    let p = TapeProgress {
        block_index: 2,
        block_count: 3,
        pulse_index: 0,
        pulse_count: 0,
    };
    assert_eq!(p.fraction(), 1.0);
    // Non-final empty block stays at the block boundary (not complete).
    let mid = TapeProgress {
        block_index: 1,
        block_count: 3,
        pulse_index: 0,
        pulse_count: 0,
    };
    assert!((mid.fraction() - 1.0 / 3.0).abs() < f32::EPSILON);
    // u32::MAX + 1 must not panic in debug; treat as past the end → complete.
    let max = TapeProgress {
        block_index: u32::MAX,
        block_count: 1,
        pulse_index: 0,
        pulse_count: 0,
    };
    assert_eq!(max.fraction(), 1.0);
}

#[test]
fn tape_ear_emits_speaker_edges_while_playing() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let img = TapImage::load(&fixture_tap()).expect("fixture");
    let mut m = Machine::new_48k(&rom).unwrap();
    m.set_tape_load_options(TapeLoadOptions {
        flash_load: false,
        speed: 1,
        ..Default::default()
    });
    m.insert_tape(TapPlayer::new(img));
    m.set_tape_playing(true);
    let audio = m.run_frame();
    assert!(
        !audio.beeper_edges.is_empty(),
        "EAR pilot should produce speaker edges for load tones"
    );
}

#[test]
fn rom_ld_bytes_entry_flash_loads_via_shadow_af() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let img = TapImage::load(&fixture_tap()).expect("fixture");
    let header = img.blocks[0].clone();
    let mut m = Machine::new_48k(&rom).unwrap();
    m.set_tape_load_options(TapeLoadOptions {
        flash_load: true,
        speed: 1,
        ..Default::default()
    });
    m.insert_tape(TapPlayer::new(img));
    m.set_tape_playing(true);

    // Call ROM LD-BYTES (0x0556) with flag in A and load carry — ROM will EX AF,AF'
    // before the 0x056C trap; flash-load must read A′/F′.
    let ret = 0x7000u16;
    m.cpu_mut().regs.sp = 0x5f00;
    m.write_mem(0x5f00, (ret & 0xff) as u8);
    m.write_mem(0x5f01, (ret >> 8) as u8);
    m.cpu_mut().regs.a = 0x00;
    m.cpu_mut().regs.f = flag::C;
    m.cpu_mut().regs.set_ix(0x5c00);
    m.cpu_mut().regs.set_de(17);
    m.cpu_mut().regs.pc = 0x0556;
    if let Machine::Spec48 { bus, .. } = &mut m {
        bus.frame_t = INT_LENGTH_48;
    }

    for _ in 0..256 {
        m.step_once();
        if m.cpu().regs.pc == ret {
            break;
        }
    }
    assert_eq!(m.cpu().regs.pc, ret, "LD-BYTES should return via SA/LD-RET");
    assert_eq!(m.cpu().regs.f & flag::C, flag::C, "carry set on success");
    // Filename in header payload starts at offset 1 (type) + 1… name at IX+1
    assert_eq!(m.read_mem(0x5c00), header[1], "type byte");
    assert_eq!(m.read_mem(0x5c01), b't');
    assert_eq!(m.read_mem(0x5c02), b'e');
    assert_eq!(m.tape_block(), Some(1));
}

#[test]
fn attr_mark_fixture_flash_loads_code_bytes() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/tape/attr_mark.tap");
    let img = TapImage::load(&path).expect("attr_mark");
    let data = img.blocks[1].clone();
    let mut m = Machine::new_48k(&rom).unwrap();
    m.set_tape_load_options(TapeLoadOptions {
        flash_load: true,
        speed: 1,
        ..Default::default()
    });
    let mut player = TapPlayer::new(img);
    player.consume_block();
    m.insert_tape(player);
    m.set_tape_playing(true);

    let ret = 0x1234u16;
    m.cpu_mut().regs.sp = 0x5f00;
    m.write_mem(0x5f00, (ret & 0xff) as u8);
    m.write_mem(0x5f01, (ret >> 8) as u8);
    m.cpu_mut().regs.pc = LD_BYTES_TRAP_PC;
    m.cpu_mut().regs.a = 0x0f;
    m.cpu_mut().regs.a_ = 0xff;
    m.cpu_mut().regs.f_ = flag::C;
    m.cpu_mut().regs.set_ix(0x8000);
    m.cpu_mut().regs.set_de((data.len() - 2) as u16);
    if let Machine::Spec48 { bus, .. } = &mut m {
        bus.frame_t = INT_LENGTH_48;
    }
    m.step_once();
    assert_eq!(m.cpu().regs.pc, ret);
    assert_eq!(m.read_mem(0x8000), 0x21);
    assert_eq!(m.read_mem(0x8001), 0x00);
    assert_eq!(m.read_mem(0x8002), 0x58);
    assert_eq!(m.read_mem(0x8003), 0x36);
    assert_eq!(m.read_mem(0x8004), 0xd7);
    assert_eq!(m.read_mem(0x8005), 0xc9);
}

#[test]
fn rom_ld_bytes_ear_loads_attr_mark_data_block() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/tape/attr_mark.tap");
    let img = TapImage::load(&path).expect("attr_mark");
    let data = img.blocks[1].clone();
    let mut m = Machine::new_48k(&rom).unwrap();
    m.set_tape_load_options(TapeLoadOptions {
        flash_load: false,
        speed: 1,
        ..Default::default()
    });
    let mut player = TapPlayer::new(img);
    player.consume_block();
    m.insert_tape(player);
    m.set_tape_playing(true);

    let ret = 0x1234u16;
    m.cpu_mut().regs.sp = 0x5f00;
    m.write_mem(0x5f00, (ret & 0xff) as u8);
    m.write_mem(0x5f01, (ret >> 8) as u8);
    m.cpu_mut().regs.a = 0xff;
    m.cpu_mut().regs.f = flag::C;
    m.cpu_mut().regs.set_ix(0x8000);
    m.cpu_mut().regs.set_de((data.len() - 2) as u16);
    m.cpu_mut().regs.pc = 0x0556;
    if let Machine::Spec48 { bus, .. } = &mut m {
        bus.frame_t = INT_LENGTH_48;
    }
    let mut ok = false;
    for _ in 0..400 {
        let _ = m.run_frame();
        if attr_mark_code_ok(&m) {
            ok = true;
            break;
        }
    }
    if !ok {
        eprintln!(
                "EAR data fail PC={:04X} mem {:02X}{:02X}{:02X}{:02X}{:02X}{:02X} block={:?} IX={:04X} DE={:04X} F={:02X}",
                m.cpu().regs.pc,
                m.read_mem(0x8000),
                m.read_mem(0x8001),
                m.read_mem(0x8002),
                m.read_mem(0x8003),
                m.read_mem(0x8004),
                m.read_mem(0x8005),
                m.tape_block(),
                m.cpu().regs.ix(),
                m.cpu().regs.de(),
                m.cpu().regs.f,
            );
    }
    assert!(ok, "ROM LD-BYTES EAR path should load attr_mark CODE bytes");
}

#[test]
fn flash_load_can_be_disabled_for_ear_path() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let img = TapImage::load(&fixture_tap()).expect("fixture");
    let mut m = Machine::new_48k(&rom).unwrap();
    m.set_tape_load_options(TapeLoadOptions {
        flash_load: false,
        speed: 1,
        ..Default::default()
    });
    m.insert_tape(TapPlayer::new(img));
    m.set_tape_playing(true);
    m.cpu_mut().regs.pc = LD_BYTES_TRAP_PC;
    m.cpu_mut().regs.a_ = 0x00;
    m.cpu_mut().regs.f_ = flag::C;
    m.cpu_mut().regs.set_de(17);
    m.cpu_mut().regs.sp = 0x5f00;
    m.write_mem(0x5f00, 0x00);
    m.write_mem(0x5f01, 0x70);
    if let Machine::Spec48 { bus, .. } = &mut m {
        bus.frame_t = INT_LENGTH_48;
    }
    m.step_once();
    // Without flash-load, ROM proceeds into edge-detect (not an instant RET).
    assert_ne!(m.cpu().regs.pc, 0x7000);
    assert_eq!(
        m.tape_block(),
        Some(0),
        "EAR path should not consume via trap"
    );
}

#[test]
fn ear_turbo_returns_to_1x_after_tape_finishes() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    // Tiny TAP so EAR turbo finishes quickly.
    let img = TapImage {
        blocks: vec![vec![0xff, 0x00, 0xff]],
        ..Default::default()
    };
    let mut m = Machine::new_48k(&rom).unwrap();
    m.set_tape_load_options(TapeLoadOptions {
        flash_load: false,
        speed: 20,
        ..Default::default()
    });
    m.insert_tape(TapPlayer::new(img));
    m.set_tape_playing(true);
    assert!(m.tape_playing());
    // Mid-load: turbo must run multiple Spectrum frames per host tick.
    let t0 = m.cpu().t;
    let _ = m.run_frame();
    let mid_dt = m.cpu().t.saturating_sub(t0);
    assert!(
        mid_dt > 20_000,
        "expected turbo mid-load T-state advance, got {mid_dt}"
    );
    for _ in 0..20_000 {
        let _ = m.run_frame();
        if m.tape_finished() || !m.tape_playing() {
            break;
        }
    }
    assert!(
        m.tape_finished() || !m.tape_playing(),
        "deck should finish/pause after EAR exhausts"
    );
    assert!(
        !m.tape_playing(),
        "playing must clear when finished so turbo stops for ROM/BASIC (#178)"
    );
    // Tiny TAP leaves PC in ROM with interrupts enabled → must be 1× (#178).
    assert!(
        m.cpu().regs.pc < 0x8000 || m.cpu().regs.iff1,
        "fixture should not sit in a high-RAM DI loader"
    );
    let t1 = m.cpu().t;
    let _ = m.run_frame();
    let post_dt = m.cpu().t.saturating_sub(t1);
    // One Spectrum frame ≈ 69888 T-states (48K); allow slack, but not N× turbo.
    assert!(
        post_dt < 150_000,
        "after tape end in ROM/BASIC, run_frame should be ~1× (got {post_dt} T-states)"
    );
}

#[test]
fn ear_turbo_stops_when_deck_finished_even_with_di_in_high_ram() {
    // A *tape* speed must stop mattering once the deck is done, including
    // for programs that run with interrupts disabled from upper RAM
    // (Arkanoid plays like that) — #390.
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let img = TapImage {
        blocks: vec![vec![0xff, 0x00, 0xff]],
        ..Default::default()
    };
    let mut m = Machine::new_48k(&rom).unwrap();
    m.set_tape_load_options(TapeLoadOptions {
        flash_load: false,
        speed: 20,
        ..Default::default()
    });
    m.insert_tape(TapPlayer::new(img));
    m.set_tape_playing(true);
    for _ in 0..20_000 {
        let _ = m.run_frame();
        if m.tape_finished() || !m.tape_playing() {
            break;
        }
    }
    assert!(m.tape_finished() || !m.tape_playing());
    // DI loop in upper RAM — the old gate kept turbo here forever.
    m.cpu_mut().regs.iff1 = false;
    m.cpu_mut().regs.iff2 = false;
    m.cpu_mut().regs.pc = 0xF448;
    m.write_mem(0xF448, 0x18); // JR 0 — tight loop
    m.write_mem(0xF449, 0xFE);
    assert_eq!(
        m.effective_speed_multiplier(),
        1,
        "finished deck must report 1× even for DI code in upper RAM"
    );
    let t0 = m.cpu().t;
    let _ = m.run_frame();
    let dt = m.cpu().t.saturating_sub(t0);
    assert!(
        dt < 150_000,
        "post-tape DI code must run at ~1× (one 69888 T frame), got {dt}"
    );
}

#[test]
fn post_tape_turbo_never_injects_guest_keys() {
    // The old turbo auto-acked Arkanoid's `$8224` any-key poll with Space.
    // Synthetic guest input is not ours to fabricate, and measurement shows
    // the protection completes on its own at 1× (#390).
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let img = TapImage {
        blocks: vec![vec![0xff, 0x00, 0xff]],
        ..Default::default()
    };
    let mut m = Machine::new_48k(&rom).unwrap();
    m.set_tape_load_options(TapeLoadOptions {
        flash_load: false,
        speed: 64,
        ..Default::default()
    });
    m.insert_tape(TapPlayer::new(img));
    m.set_tape_playing(true);
    for _ in 0..20_000 {
        let _ = m.run_frame();
        if m.tape_finished() || !m.tape_playing() {
            break;
        }
    }
    assert!(m.tape_finished() || !m.tape_playing());

    // Speedlock-shaped outer stub: CALL key gate / INC E / JR Z / EI / RET.
    let stub: &[u8] = &[
        0xCD, 0x24, 0x82, // F3D1 CALL 8224
        0x1C, // F3D4 INC E
        0x28, 0xF7, // F3D5 JR Z,F3CE
        0xFB, // F3D7 EI
        0xC9, // F3D8 RET
    ];
    for (i, b) in stub.iter().enumerate() {
        m.write_mem(0xF3D1 + i as u16, *b);
    }
    let gate: &[u8] = &[
        0xC5, 0xF5, // PUSH BC / PUSH AF
        0x01, 0xFE, 0x00, // LD BC,$00FE
        0xED, 0x78, // IN A,(C)
        0xF6, 0xE0, // OR $E0
        0x3C, // INC A
        0x28, 0xEF, // JR Z,$821F abort
        0xF1, 0xC1, // POP AF / POP BC
        0xC9, // RET
    ];
    for (i, b) in gate.iter().enumerate() {
        m.write_mem(0x8224 + i as u16, *b);
    }
    m.write_mem(0x821F, 0xF1);
    m.write_mem(0x8220, 0xC1);
    m.write_mem(0x8221, 0x1E);
    m.write_mem(0x8222, 0xFF); // LD E,$FF (abort)
    m.write_mem(0x8223, 0xC9);

    m.cpu_mut().regs.iff1 = false;
    m.cpu_mut().regs.iff2 = false;
    m.cpu_mut().regs.pc = 0xF3D1;
    m.cpu_mut().regs.sp = 0xFDE1;
    m.write_mem(0xFDE1, 0xCE);
    m.write_mem(0xFDE2, 0xF3);
    m.cpu_mut().regs.e = 0x10;
    m.keyboard_mut().reset();
    assert_eq!(
        m.keyboard_mut().read(0x7f),
        0x1f,
        "precondition: no key held on row 7"
    );

    for _ in 0..4 {
        let _ = m.run_frame();
        assert_eq!(
            m.keyboard_mut().read(0x7f),
            0x1f,
            "emulator must not press Space for the guest; pc={:#06x}",
            m.cpu().regs.pc
        );
    }
    assert!(
        !m.cpu().regs.iff1,
        "gate must abort on its own with no key held; pc={:#06x} e={:#04x}",
        m.cpu().regs.pc,
        m.cpu().regs.e
    );
}

#[test]
fn effective_speed_multiplier_tracks_the_active_rate() {
    // Hosts show this, so it must match what `run_frame` really does (#390).
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let img = TapImage {
        blocks: vec![vec![0xff, 0x00, 0xff]],
        ..Default::default()
    };
    let mut m = Machine::new_48k(&rom).unwrap();
    assert_eq!(m.effective_speed_multiplier(), 1, "no deck → 1×");
    m.set_tape_load_options(TapeLoadOptions {
        flash_load: false,
        speed: 20,
        ..Default::default()
    });
    m.insert_tape(TapPlayer::new(img));
    assert_eq!(m.effective_speed_multiplier(), 1, "deck paused → 1×");
    m.set_tape_playing(true);
    assert_eq!(m.effective_speed_multiplier(), 20, "playing deck → speed");
    let t0 = m.cpu().t;
    let _ = m.run_frame();
    let dt = m.cpu().t.saturating_sub(t0);
    assert!(
        dt > 20_000 * 10,
        "reported 20× should match the T-states run, got {dt}"
    );
    for _ in 0..20_000 {
        let _ = m.run_frame();
        if m.tape_finished() || !m.tape_playing() {
            break;
        }
    }
    assert_eq!(
        m.effective_speed_multiplier(),
        1,
        "finished deck → 1× regardless of the speed setting"
    );
    // Flash-load on a TAP deck pokes at the trap — never multiplies frames.
    m.set_tape_load_options(TapeLoadOptions {
        flash_load: true,
        speed: 64,
        ..Default::default()
    });
    assert_eq!(m.effective_speed_multiplier(), 1, "flash-load → 1×");
}

/// Pulse-only TZX (turbo block + long pause) — nothing for a trap to poke.
fn minimal_tzx_pulse_deck() -> Vec<u8> {
    let mut v = minimal_tzx_turbo_machine(&[0xff, 0x00, 0xaa]);
    v.push(0x20); // pause (ms) — keeps the deck busy for several host ticks
    v.extend_from_slice(&5_000u16.to_le_bytes());
    v
}

#[test]
fn instant_on_pulse_only_deck_uses_ear_turbo_not_the_play_speed() {
    // Instant on a deck with no LD-BYTES trap (Speedlock-style TZX) used to
    // fall through to a realtime EAR load, so "Instant" was the slowest
    // option on offer and looked tied to the Play speed control (#390).
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let data = minimal_tzx_pulse_deck();
    let player = TzxPlayer::parse(&data).expect("tzx");
    let mut m = Machine::new_48k(&rom).unwrap();
    m.set_tape_load_options(TapeLoadOptions {
        flash_load: true,
        speed: 1,
        ..Default::default()
    });
    m.insert_tzx(player);
    assert!(
        !m.tape_supports_flash_load(),
        "pulse TZX has no TAP blocks to flash"
    );
    assert_eq!(m.effective_speed_multiplier(), 1, "paused deck → 1×");

    m.set_tape_playing(true);
    assert_eq!(
        m.effective_speed_multiplier(),
        INSTANT_EAR_FALLBACK_SPEED,
        "Instant must stay fast instead of inheriting the 1× EAR setting"
    );
    let t0 = m.cpu().t;
    let _ = m.run_frame();
    let dt = m.cpu().t.saturating_sub(t0);
    assert!(
        dt > u64::from(FRAME_TSTATES_48) * 8,
        "one host tick should run many Spectrum frames, got {dt}"
    );
}

#[test]
fn instant_ear_fallback_still_stops_when_the_deck_finishes() {
    // The fallback is a *tape* convenience like EAR turbo: once the deck is
    // exhausted the loaded program runs at 1× (#390).
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let data = minimal_tzx_pulse_deck();
    let mut m = Machine::new_48k(&rom).unwrap();
    m.set_tape_load_options(TapeLoadOptions {
        flash_load: true,
        speed: 1,
        ..Default::default()
    });
    m.insert_tzx(TzxPlayer::parse(&data).expect("tzx"));
    m.set_tape_playing(true);
    for _ in 0..20_000 {
        let _ = m.run_frame();
        if m.tape_finished() || !m.tape_playing() {
            break;
        }
    }
    assert!(
        m.tape_finished() || !m.tape_playing(),
        "deck did not finish"
    );
    assert_eq!(
        m.effective_speed_multiplier(),
        1,
        "finished pulse deck → 1× even with Instant still latched"
    );
    let t0 = m.cpu().t;
    let _ = m.run_frame();
    let dt = m.cpu().t.saturating_sub(t0);
    assert!(
        dt < 150_000,
        "post-tape code must run at ~1× after an Instant EAR fallback, got {dt}"
    );
}

#[test]
fn plus2a_stack_repair_ignores_coincidental_0038_marker() {
    let player = TapPlayer::new(TapImage::default());
    let plant = |bus: &mut BusPlus3| {
        bus.write(0x5CB2, 0xFF);
        bus.write(0x5CB3, 0x7F); // RAMTOP = 0x7FFF
        bus.write(0x7FEC, 0x38);
        bus.write(0x7FED, 0x00); // coincidental 0x0038 marker
        bus.write(0x7FFC, 0xAA);
        bus.write(0x7FFD, 0xBB); // sentinel 0xBBAA
    };
    let assert_untouched = |bus: &BusPlus3| {
        assert_eq!(
            u16::from_le_bytes([bus.read(0x7FEC), bus.read(0x7FED)]),
            0x0038
        );
        assert_eq!(
            u16::from_le_bytes([bus.read(0x7FFC), bus.read(0x7FFD)]),
            0xBBAA
        );
    };

    // SP outside CLEAR-32767 loader window — must not rewrite high RAM.
    {
        let mut bus = BusPlus3::new_with_disk(false);
        plant(&mut bus);
        let mut cpu = Cpu::new();
        cpu.regs.sp = 0xFF50;
        cpu.regs.pc = 0x15E8;
        Machine::plus2a_repair_menu_loader_stack_if_needed(&mut bus, &cpu, &player);
        assert_untouched(&bus);
    }

    // SP looks like Loader depth but PC already left ROM — must not rewrite.
    {
        let mut bus = BusPlus3::new_with_disk(false);
        plant(&mut bus);
        let mut cpu = Cpu::new();
        cpu.regs.sp = 0x7FE6;
        cpu.regs.pc = 0x8000;
        Machine::plus2a_repair_menu_loader_stack_if_needed(&mut bus, &cpu, &player);
        assert_untouched(&bus);
    }
}

#[test]
fn ear_speed_finishes_block_in_fewer_run_frames() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let img = TapImage {
        blocks: vec![vec![0x00, 0x00]],
        ..Default::default()
    };
    let frames_until_block1 = |speed: u32| -> u32 {
        let mut m = Machine::new_48k(&rom).unwrap();
        m.set_tape_load_options(TapeLoadOptions {
            flash_load: false,
            speed,
            ..Default::default()
        });
        m.insert_tape(TapPlayer::new(img.clone()));
        m.set_tape_playing(true);
        for n in 1..=50_000u32 {
            let _ = m.run_frame();
            if m.tape_block() == Some(1) {
                return n;
            }
        }
        panic!("speed {speed}: did not reach block 1");
    };
    let slow = frames_until_block1(1);
    let fast = frames_until_block1(10);
    assert!(
        fast * 7 < slow,
        "EAR speed 10 should finish in ~1/10 host run_frames (1x={slow}, 10x={fast})"
    );
}

#[test]
fn reset_keeps_tape_inserted_and_paused_at_position() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let img = TapImage::load(&fixture_tap()).expect("fixture");
    let mut m = Machine::new_48k(&rom).unwrap();
    m.set_tape_load_options(TapeLoadOptions {
        flash_load: false,
        speed: 1,
        ..Default::default()
    });
    m.insert_tape(TapPlayer::new(img));
    m.set_tape_playing(true);
    for _ in 0..3 {
        let _ = m.run_frame();
    }
    let block_before = m.tape_block();
    let pulse_before = m.tape_progress().map(|p| p.pulse_index);
    assert!(m.has_tape());
    assert!(m.tape_playing());
    m.reset();
    assert!(m.has_tape(), "reset must not eject the tape");
    assert!(!m.tape_playing(), "reset should pause the deck");
    assert_eq!(
        m.tape_block(),
        block_before,
        "reset should keep tape position"
    );
    assert_eq!(
        m.tape_progress().map(|p| p.pulse_index),
        pulse_before,
        "reset should keep pulse position"
    );
}

#[test]
fn reset_keeps_plus3_disk_inserted() {
    let Some(rom) = rom_plus3_only().or_else(rom_plus2a_only) else {
        eprintln!("skip: plus3 ROM missing");
        return;
    };
    let mut m = Machine::new_plus3(&rom).unwrap();
    let img = formats::DskImage::synthetic_empty_track();
    m.insert_disk(img).expect("insert");
    {
        let Machine::SpecPlus3 { bus, .. } = &m else {
            panic!("expected SpecPlus3");
        };
        assert!(bus.fdc.image.is_some());
    }
    m.reset();
    let Machine::SpecPlus3 { bus, .. } = &m else {
        panic!("expected SpecPlus3");
    };
    assert!(
        bus.fdc.image.is_some(),
        "reset must not eject the +3 disk image"
    );
}

#[test]
fn rom_ld_bytes_ear_survives_mid_load_speed_change() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/tape/attr_mark.tap");
    let img = TapImage::load(&path).expect("attr_mark");
    let data = img.blocks[1].clone();
    let mut m = Machine::new_48k(&rom).unwrap();
    m.set_tape_load_options(TapeLoadOptions {
        flash_load: false,
        speed: 5,
        ..Default::default()
    });
    let mut player = TapPlayer::new(img);
    player.consume_block();
    m.insert_tape(player);
    m.set_tape_playing(true);

    let ret = 0x1234u16;
    m.cpu_mut().regs.sp = 0x5f00;
    m.write_mem(0x5f00, (ret & 0xff) as u8);
    m.write_mem(0x5f01, (ret >> 8) as u8);
    m.cpu_mut().regs.a = 0xff;
    m.cpu_mut().regs.f = flag::C;
    m.cpu_mut().regs.set_ix(0x8000);
    m.cpu_mut().regs.set_de((data.len() - 2) as u16);
    m.cpu_mut().regs.pc = 0x0556;
    if let Machine::Spec48 { bus, .. } = &mut m {
        bus.frame_t = INT_LENGTH_48;
    }
    // Into the leader/data, then raise EAR turbo — must not restart the block.
    for _ in 0..40 {
        let _ = m.run_frame();
    }
    m.set_tape_load_options(TapeLoadOptions {
        flash_load: false,
        speed: 15,
        ..Default::default()
    });
    let mut ok = false;
    for _ in 0..400 {
        let _ = m.run_frame();
        if attr_mark_code_ok(&m) {
            ok = true;
            break;
        }
    }
    assert!(
        ok,
        "ROM LD-BYTES EAR path should complete after mid-load speed change (PC={:04X} block={:?})",
        m.cpu().regs.pc,
        m.tape_block()
    );
}

#[test]
fn boggit_header_flash_loads_when_present() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let Some(boggit) = std::env::var_os("SPEC_CHUM_BOGGIT_TZX").map(PathBuf::from) else {
        eprintln!("skip: set SPEC_CHUM_BOGGIT_TZX to run the Boggit regression");
        return;
    };
    if !boggit.is_file() {
        eprintln!("skip: SPEC_CHUM_BOGGIT_TZX path is not a file ({boggit:?})");
        return;
    }
    let data = std::fs::read(&boggit).expect("read boggit");
    assert!(
        tape::TzxPlayer::is_standard_speed_only(&data),
        "Boggit side 1 should convert to TAP"
    );
    let img = tape::TzxPlayer::to_tap_image(&data).expect("to tap");
    let header = img.blocks[0].clone();
    let mut m = Machine::new_48k(&rom).unwrap();
    m.set_tape_load_options(TapeLoadOptions {
        flash_load: true,
        speed: 1,
        ..Default::default()
    });
    m.insert_tape(TapPlayer::new(img));
    m.set_tape_playing(true);

    let ret = 0x7000u16;
    m.cpu_mut().regs.sp = 0x5f00;
    m.write_mem(0x5f00, (ret & 0xff) as u8);
    m.write_mem(0x5f01, (ret >> 8) as u8);
    m.cpu_mut().regs.a = 0x00;
    m.cpu_mut().regs.f = flag::C;
    m.cpu_mut().regs.set_ix(0x5c00);
    m.cpu_mut().regs.set_de(17);
    m.cpu_mut().regs.pc = 0x0556;
    if let Machine::Spec48 { bus, .. } = &mut m {
        bus.frame_t = INT_LENGTH_48;
    }
    for _ in 0..256 {
        m.step_once();
        if m.cpu().regs.pc == ret {
            break;
        }
    }
    assert_eq!(m.cpu().regs.pc, ret);
    assert_eq!(m.read_mem(0x5c00), header[1]);
    // "BOGGIT pt1"
    assert_eq!(m.read_mem(0x5c01), b'B');
    assert_eq!(m.read_mem(0x5c02), b'O');
    assert_eq!(m.read_mem(0x5c06), b'T');
}

/// Optional local e2e: scripted `LOAD ""` + Play flash-loads Boggit PROGRAM header.
/// Set `SPEC_CHUM_BOGGIT_TZX` (do not commit the TZX).
#[test]
fn boggit_load_quotes_flash_loads_header_when_present() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let Some(boggit) = std::env::var_os("SPEC_CHUM_BOGGIT_TZX").map(PathBuf::from) else {
        eprintln!("skip: set SPEC_CHUM_BOGGIT_TZX for Boggit LOAD \"\" e2e");
        return;
    };
    if !boggit.is_file() {
        eprintln!("skip: SPEC_CHUM_BOGGIT_TZX not a file ({boggit:?})");
        return;
    }
    let data = std::fs::read(&boggit).expect("read boggit");
    let img = tape::TzxPlayer::to_tap_image(&data).expect("to tap");
    let mut m = Machine::new_48k(&rom).unwrap();
    m.set_tape_load_options(TapeLoadOptions {
        flash_load: true,
        speed: 1,
        ..Default::default()
    });
    m.insert_tape(TapPlayer::new(img));
    for _ in 0..200 {
        let _ = m.run_frame();
    }
    m.type_load_quotes_48k(false);
    assert_eq!(m.cpu().regs.pc, LD_BYTES_TRAP_PC);
    m.set_tape_playing(true);
    let mut progressed = false;
    for _ in 0..200 {
        let _ = m.run_frame();
        let block = m.tape_block();
        // Header (+ follow-on blocks) consumed, or PC left ROM into the loader.
        if block.is_some_and(|b| b >= 1) || m.cpu().regs.pc >= 0x4000 {
            progressed = true;
            break;
        }
    }
    assert!(
        progressed,
        "Boggit LOAD \"\" should flash-load past the first block (PC={:04X} block={:?})",
        m.cpu().regs.pc,
        m.tape_block()
    );
    eprintln!(
        "Boggit LOAD \"\" progressed: PC={:04X} block={:?}",
        m.cpu().regs.pc,
        m.tape_block()
    );
}

fn rom128() -> Option<Vec<u8>> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/128/spec128uk.rom");
    std::fs::read(p).ok()
}

#[test]
fn ay_frame_audio_nonzero_when_tone_programmed() {
    let Some(rom) = rom128() else {
        eprintln!("skip: roms/128/spec128uk.rom missing");
        return;
    };
    let mut m = Machine::new_128k(&rom).unwrap();
    // Program AY tone A via ports
    if let Machine::Spec128 { bus, .. } = &mut m {
        bus.out_port(0xfffd, 0); // select R0
        bus.out_port(0xbffd, 16); // fine
        bus.out_port(0xfffd, 1);
        bus.out_port(0xbffd, 0); // coarse
        bus.out_port(0xfffd, 8);
        bus.out_port(0xbffd, 0x0f); // volume
        bus.out_port(0xfffd, 7);
        bus.out_port(0xbffd, 0x38); // tone A only
    }
    let audio = m.run_frame();
    assert!(!audio.ay_samples.is_empty());
    let energy: f32 = audio.ay_samples.iter().map(|s| s * s).sum();
    assert!(
        energy > 0.01,
        "AY tone should produce frame audio energy, got {energy}"
    );
}

fn rom_plus3() -> Option<Vec<u8>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms");
    for rel in ["plus3/plus3.rom", "plus2a/plus2a.rom"] {
        if let Ok(data) = std::fs::read(root.join(rel)) {
            return Some(data);
        }
    }
    None
}

fn rom_plus2a_only() -> Option<Vec<u8>> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/plus2a/plus2a.rom");
    std::fs::read(p).ok()
}

fn rom_plus3_only() -> Option<Vec<u8>> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/plus3/plus3.rom");
    std::fs::read(p).ok()
}

fn rom_plus3e_only() -> Option<Vec<u8>> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/plus3e/plus3e.rom");
    std::fs::read(p).ok()
}

fn rom_plus2() -> Option<Vec<u8>> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/plus2/plus2uk.rom");
    std::fs::read(p).ok()
}

fn rom_timex_tc2048() -> Option<Vec<u8>> {
    let path = resolve_rom_path(Model::TimexTC2048)?;
    std::fs::read(path).ok()
}

fn rom_timex_ts2068() -> Option<(Vec<u8>, Vec<u8>)> {
    let home = resolve_rom_path(Model::TimexTS2068)?;
    let exrom = resolve_exrom_path(Model::TimexTS2068)?;
    Some((std::fs::read(home).ok()?, std::fs::read(exrom).ok()?))
}

#[test]
fn timex_tc2048_boot_smoke() {
    let Some(path) = resolve_rom_path(Model::TimexTC2048) else {
        eprintln!("skip: roms/timex/tc2048.rom missing");
        return;
    };
    let rom = std::fs::read(path).expect("read timex rom");
    let mut m = Machine::new_timex_tc2048(&rom).unwrap();
    assert_eq!(m.model(), Model::TimexTC2048);
    for _ in 0..50 {
        let _ = m.run_frame();
    }
}

#[test]
fn timex_scld_ext_colour_render_uses_alt_attrs() {
    // Screen-RAM / SCLD rendering only — no real Timex ROM required.
    let mut m = Machine::new_timex_tc2048(&[0; 16 * 1024]).unwrap();
    // Paint primary bitmap solid; primary 8×8 attr blue; alt 8×1 attr red.
    if let Machine::Spec48 { bus, .. } = &mut m {
        bus.write(0x4000, 0xFF);
        bus.write(0x5800, 0x01); // blue ink — must not win in ext colour
        bus.write(0x6000, 0x02); // red 8×1 attr (scrambled line 0)
        bus.out_port(0x00FF, 0x02); // EXTCOLOUR
    } else {
        panic!("expected Spec48");
    }
    let mut out = vec![0u8; 256 * 192 * 4];
    m.render_rgba(&mut out, false);
    let red = ula::palette_rgb(2, false);
    assert_eq!(&out[0..3], &red);
}

#[test]
fn timex_scld_hires_render_interleaves_files() {
    let mut m = Machine::new_timex_tc2048(&[0; 16 * 1024]).unwrap();
    if let Machine::Spec48 { bus, .. } = &mut m {
        bus.write(0x4000, 0xFF); // primary solid
        bus.write(0x6000, 0x00); // alt empty
                                 // Mode 6 + white ink / black paper (bits 3–5 = 7).
        bus.out_port(0x00FF, 0x06 | (7 << 3));
    } else {
        panic!("expected Spec48");
    }
    assert_eq!(m.framebuffer_dims(false), (512, 192));
    let mut out = vec![0u8; 512 * 192 * 4];
    m.render_rgba(&mut out, false);
    let white = ula::palette_rgb(7, true);
    let black = ula::palette_rgb(0, true);
    assert_eq!(&out[0..3], &white);
    assert_eq!(&out[8 * 4..8 * 4 + 3], &black);
}

#[test]
fn timex_ts2068_boot_smoke() {
    let Some((home, exrom)) = rom_timex_ts2068() else {
        eprintln!("skip: roms/timex/tc2068-*.rom missing");
        return;
    };
    let mut m = Machine::new_timex_ts2068(&home, &exrom).unwrap();
    assert_eq!(m.model(), Model::TimexTS2068);
    for _ in 0..50 {
        let _ = m.run_frame();
    }
    // Horizontal MMU: page EX-ROM over chunk 0.
    if let Machine::Spec48 { bus, .. } = &mut m {
        bus.out_port(0x00FF, 0x80);
        bus.out_port(0x00F4, 0x01);
        assert_eq!(bus.read(0x0000), exrom[0]);
    } else {
        panic!("expected Spec48 bus for TS2068");
    }
}

#[test]
fn timex_ts2068_home_dck_replaces_rom_with_spectrum() {
    let Some((home, exrom)) = rom_timex_ts2068() else {
        eprintln!("skip: roms/timex/tc2068-*.rom missing");
        return;
    };
    let Some(spec) = resolve_rom_path(Model::Spectrum48).and_then(|p| std::fs::read(p).ok()) else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let mut rom16 = [0u8; 16384];
    rom16.copy_from_slice(&spec[..16384]);
    let dck = formats::DckImage::spectrum_rom_home(&rom16);
    let mut m = Machine::new_timex_ts2068(&home, &exrom).unwrap();
    assert_ne!(
        m.read_mem(0x0556),
        rom16[0x0556],
        "precondition: Timex LD-BYTES site ≠ Spectrum"
    );
    m.insert_timex_dock(&dck).unwrap();
    assert!(m.has_timex_dock());
    assert_eq!(m.read_mem(0x0000), rom16[0]);
    assert_eq!(m.read_mem(0x0001), rom16[1]);
    // Spectrum LD-BYTES entry lives at $0556 in the home ROM overlay.
    assert_eq!(m.read_mem(0x0556), rom16[0x0556]);
    assert_eq!(m.read_mem(0x0557), rom16[0x0557]);
    m.eject_timex_dock().unwrap();
    assert!(!m.has_timex_dock());
    assert_eq!(m.read_mem(0x0556), home[0x0556]);
}

#[test]
fn timex_ts2068_redirects_spectrum_ld_bytes_call_from_ram() {
    let Some((home, exrom)) = rom_timex_ts2068() else {
        eprintln!("skip: roms/timex/tc2068-*.rom missing");
        return;
    };
    let mut m = Machine::new_timex_ts2068(&home, &exrom).unwrap();
    // Simulate `CALL $0556` from RAM (Death Chase-style Spectrum loader).
    if let Machine::Spec48 { cpu, bus, .. } = &mut m {
        bus.write(0x8000, 0xC9); // RET landing pad for stack ret
        cpu.regs.sp = 0xFFFD;
        bus.write(0xFFFD, 0x00);
        bus.write(0xFFFE, 0x80); // ret → $8000
        cpu.regs.pc = 0x0556;
    } else {
        panic!("expected Spec48 bus for TS2068");
    }
    m.step_once();
    assert_eq!(
        m.cpu().regs.pc,
        TIMEX_EXROM_LD_BYTES_PC.wrapping_add(1),
        "after one opcode at Timex LD-BYTES entry"
    );
    if let Machine::Spec48 { bus, .. } = &m {
        assert!(bus.timex_scld.use_exrom());
        assert!(bus.timex_scld.chunk_paged(0));
        assert_eq!(
            bus.read(TIMEX_EXROM_LD_BYTES_PC),
            tape::LD_BYTES_PROLOGUE[0]
        );
    }
}

#[test]
fn timex_ts2068_ay_advances_on_step_apis() {
    let Some((home, exrom)) = rom_timex_ts2068() else {
        eprintln!("skip: roms/timex/tc2068-*.rom missing");
        return;
    };
    let mut m = Machine::new_timex_ts2068(&home, &exrom).unwrap();
    // Short period tone A so sample_mono goes non-zero once AY advances.
    if let Machine::Spec48 { bus, .. } = &mut m {
        bus.out_port(0x00F5, 0);
        bus.out_port(0x00F6, 1); // period fine = 1
        bus.out_port(0x00F5, 1);
        bus.out_port(0x00F6, 0); // period coarse = 0
        bus.out_port(0x00F5, 7);
        bus.out_port(0x00F6, 0x3e); // enable tone A
        bus.out_port(0x00F5, 8);
        bus.out_port(0x00F6, 0x0f); // full volume A
    } else {
        panic!("expected Spec48 bus for TS2068");
    }
    let mut saw = false;
    for _ in 0..4_000 {
        m.step_once();
        if let Machine::Spec48 { bus, .. } = &m {
            if bus.ay.sample_mono() > 0.0 {
                saw = true;
                break;
            }
        }
    }
    assert!(saw, "step_once must advance Timex AY");
    m.run_tstates(2_000);
    m.step_cpu_only();
    let _ = m.run_frame();
    if let Machine::Spec48 { bus, .. } = &m {
        assert!(bus.ay.sample_mono().is_finite());
    }
}

#[test]
fn model_16k_limits_ram_to_16k() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let mut m = Machine::new_16k(&rom).unwrap();
    assert_eq!(m.model(), Model::Spectrum16K);
    m.write_mem(0x4000, 0xAB);
    m.write_mem(0x8000, 0xCD);
    assert_eq!(m.read_mem(0x4000), 0xAB);
    assert_eq!(m.read_mem(0x8000), 0xFF);
}

#[test]
fn model_plus2_tags_grey_plus2() {
    let Some(rom) = rom_plus2() else {
        eprintln!("skip: roms/plus2/plus2uk.rom missing");
        return;
    };
    let m = Machine::new_plus2(&rom).unwrap();
    assert_eq!(m.model(), Model::SpectrumPlus2);
}

#[test]
fn plus2a_model_has_no_disk_and_rejects_dsk() {
    let Some(rom) = rom_plus2a_only().or_else(rom_plus3_only) else {
        eprintln!("skip: plus2a/plus3 ROM missing");
        return;
    };
    let mut m = Machine::new_plus2a(&rom).unwrap();
    assert_eq!(m.model(), Model::SpectrumPlus2A);
    {
        let Machine::SpecPlus3 { bus, .. } = &mut m else {
            panic!("expected SpecPlus3");
        };
        assert!(!bus.disk_interface);
        assert_eq!(bus.in_port(0x2ffd), 0xff);
    }
    let img = formats::DskImage::synthetic_empty_track();
    assert_eq!(
        m.insert_disk(img).unwrap_err(),
        InsertDiskError::Plus2ANoDiskInterface
    );
}

#[test]
fn plus3_model_keeps_disk_interface() {
    let Some(rom) = rom_plus3_only().or_else(rom_plus2a_only) else {
        eprintln!("skip: plus3 ROM missing");
        return;
    };
    let mut m = Machine::new_plus3(&rom).unwrap();
    assert_eq!(m.model(), Model::SpectrumPlus3);
    let Machine::SpecPlus3 { bus, .. } = &mut m else {
        panic!("expected SpecPlus3");
    };
    assert!(bus.disk_interface);
    assert_eq!(bus.in_port(0x2ffd) & 0x80, 0x80);
}

/// +3e is the same +3 gate array / disk path with Garry Lancaster firmware (#194).
#[test]
fn plus3e_boots_as_enhanced_plus3() {
    let Some(rom) = rom_plus3e_only() else {
        eprintln!("skip: roms/plus3e/plus3e.rom missing — run ./scripts/fetch_roms.sh");
        return;
    };
    assert_eq!(rom.len(), 64 * 1024);
    assert!(
        rom.windows(b"128 +3e".len()).any(|w| w == b"128 +3e"),
        "expected +3e banner bytes in concatenated Fuse plus3e ROM"
    );
    let mut m = Machine::new_plus3e(&rom).unwrap();
    assert_eq!(m.model(), Model::SpectrumPlus3e);
    assert!(m.model().has_plus3_disk());
    assert!(m.model().is_amstrad_plus());
    for _ in 0..120 {
        m.run_frame();
    }
    let Machine::SpecPlus3 {
        bus,
        plus3e: true,
        cpu,
        ..
    } = &m
    else {
        panic!("expected SpecPlus3 with plus3e");
    };
    assert!(bus.disk_interface);
    let screen_nz = bus.screen_bytes().iter().filter(|&&b| b != 0).count();
    assert!(
        screen_nz > 100,
        "expected +3e menu pixels, got {screen_nz} nonzero (PC={:04X})",
        cpu.regs.pc
    );
    let snap = m.inspect();
    assert_eq!(snap.model, Model::SpectrumPlus3e);
    assert!(
        snap.to_json().contains("\"model\":\"plus3e\""),
        "inspect JSON must label +3e distinctly from stock +3"
    );
}

/// Scorpion ZS-256: synthetic ROMs exercise model id, 256K page, and `#1FFD` (#193).
#[test]
fn scorpion_zs256_model_and_upper_ram() {
    let main = vec![0u8; 48 * 1024];
    let trdos = [0u8; bus::TRDOS_ROM_SIZE];
    let mut m = Machine::new_scorpion_zs256(&main, &trdos).unwrap();
    assert_eq!(m.model(), Model::ScorpionZs256);
    assert!(m.model().is_128k_class());
    assert!(m.beta_mut().is_some());
    {
        let Machine::Spec128 {
            bus,
            scorpion: true,
            pentagon: false,
            ..
        } = &mut m
        else {
            panic!("expected Spec128 scorpion");
        };
        assert!(bus.scorpion);
        assert_eq!(bus.frame_tstates, FRAME_TSTATES_PENTAGON);
        bus.banks[8][0] = 0xA5;
        bus.out_port(0x1ffd, 0x10);
        bus.out_port(0x7ffd, 0x00);
        assert_eq!(bus.paged_bank(), 8);
        assert_eq!(bus.read(0xc000), 0xA5);
        bus.out_port(0x1ffd, 0x01);
        bus.write(0x0000, 0x42);
        assert_eq!(bus.banks[0][0], 0x42);
    }
    let snap = m.inspect();
    assert_eq!(snap.model, Model::ScorpionZs256);
    assert!(
        snap.to_json().contains("\"model\":\"scorpion_zs256\""),
        "inspect JSON must name scorpion"
    );
}

#[test]
fn plus3_boots_and_1ffd_special_maps() {
    let Some(rom) = rom_plus3() else {
        eprintln!("skip: plus3/plus2a ROM missing — run ./scripts/fetch_roms.sh");
        return;
    };
    let mut m = Machine::new_plus3(&rom).unwrap();
    assert_eq!(m.model(), Model::SpectrumPlus3);
    // Boot long enough for the editor menu to paint (was blank when 7FFD→1FFD).
    for _ in 0..120 {
        m.run_frame();
    }
    if let Machine::SpecPlus3 { bus, cpu, .. } = &mut m {
        assert_eq!(
            bus.page_1ffd & 0x01,
            0,
            "must leave special paging off at menu"
        );
        let screen_nz = bus.screen_bytes().iter().filter(|&&b| b != 0).count();
        assert!(
            screen_nz > 100,
            "expected menu pixels, got {screen_nz} nonzero (PC={:04X} 7FFD={:02X} 1FFD={:02X})",
            cpu.regs.pc,
            bus.page_7ffd,
            bus.page_1ffd
        );
        bus.banks[0][0] = 0x5a;
        bus.out_1ffd(0x01);
        assert_eq!(bus.read(0x0000), 0x5a);
        assert_eq!(bus.in_port(0x00ff), 0xff, "no floating bus");
    }
}

/// ROM-gated: Loader (menu Enter) must talk to the `µPD765` on a synthetic
/// +3DOS DATA disk. Skips if `roms/plus3/plus3.rom` is missing — do not
/// fall back to +2A ROM.
#[test]
fn plus3_loader_talks_to_fdc_on_data_disk() {
    let Some(rom) = rom_plus3_only() else {
        eprintln!("skip: roms/plus3/plus3.rom missing — run ./scripts/fetch_roms.sh");
        return;
    };
    let mut m = Machine::new_plus3(&rom).unwrap();
    m.insert_disk(formats::DskImage::synthetic_plus3_data())
        .expect("insert");
    for _ in 0..120 {
        let _ = m.run_frame();
    }
    // Loader is the first menu item; Enter = row 6 bit 0.
    m.hold_keys(&[(6, 0)], 10);
    m.hold_keys(&[], 5);
    m.keyboard_mut().reset();
    for _ in 0..500 {
        let _ = m.run_frame();
    }
    let Machine::SpecPlus3 { bus, cpu, .. } = &m else {
        panic!("expected SpecPlus3");
    };
    assert!(
            bus.fdc.read_count > 0,
            "Loader/+3DOS should READ from the FDC (seek={} read={} write={} PC={:04X} 1FFD={:02X} PCN={})",
            bus.fdc.seek_count,
            bus.fdc.read_count,
            bus.fdc.write_count,
            cpu.regs.pc,
            bus.page_1ffd,
            bus.fdc.pcn(0)
        );
    assert!(
        bus.fdc.seek_count > 0,
        "expected SEEK or RECALIBRATE before/during disk boot (seek=0 read={})",
        bus.fdc.read_count
    );
}

/// ROM-gated: menu Loader + `DOS_BOOT` (checksum 3) must run the titled
/// bootstrap. Commercial +3 disks and DSKTOOL use this path; Fuse's phantom
/// typist only presses Enter — the ROM does the rest.
#[test]
fn plus3_loader_dos_boot_runs_titled_marker() {
    let Some(rom) = rom_plus3_only() else {
        eprintln!("skip: roms/plus3/plus3.rom missing — run ./scripts/fetch_roms.sh");
        return;
    };
    let mut m = Machine::new_plus3(&rom).unwrap();
    m.insert_disk(formats::DskImage::synthetic_plus3_boot_marker())
        .expect("insert");
    for _ in 0..120 {
        let _ = m.run_frame();
    }
    m.hold_keys(&[(6, 0)], 10);
    m.hold_keys(&[], 5);
    m.keyboard_mut().reset();
    let mut ok = false;
    for _ in 0..800 {
        let _ = m.run_frame();
        if m.inspect().border == 2 || m.read_mem(0xFE20) == 0xA5 {
            ok = true;
            break;
        }
    }
    assert!(
            ok,
            "Loader DOS_BOOT should set border 2 or poke FE20 (PC={:04X} border={} FE20={:02X} special={})",
            m.cpu().regs.pc,
            m.inspect().border,
            m.read_mem(0xFE20),
            m.inspect().paging.special
        );
}

/// ROM-gated: non-bootable titled disk → Loader `LOAD "DISK"` → BASIC RUN.
#[test]
fn plus3_loader_load_disk_runs_basic_marker() {
    let Some(rom) = rom_plus3_only() else {
        eprintln!("skip: roms/plus3/plus3.rom missing — run ./scripts/fetch_roms.sh");
        return;
    };
    let mut m = Machine::new_plus3(&rom).unwrap();
    m.insert_disk(formats::DskImage::synthetic_plus3_disk_basic())
        .expect("insert");
    for _ in 0..120 {
        let _ = m.run_frame();
    }
    m.hold_keys(&[(6, 0)], 10);
    m.hold_keys(&[], 5);
    m.keyboard_mut().reset();
    let mut ok = false;
    for _ in 0..2_500 {
        let _ = m.run_frame();
        if m.read_mem(0x8000) == 0xA5 {
            ok = true;
            break;
        }
    }
    let pc = m.cpu().regs.pc;
    let poke = m.read_mem(0x8000);
    let (read, seek) = match &m {
        Machine::SpecPlus3 { bus, .. } => (bus.fdc.read_count, bus.fdc.seek_count),
        _ => panic!("expected SpecPlus3"),
    };
    assert!(
            ok,
            "Loader LOAD \"DISK\" should RUN BASIC poke at 8000 (PC={pc:04X} 8000={poke:02X} read={read} seek={seek})"
        );
}

/// Shared `LOAD "" CODE` harness for `attr_mark.tap`. Returns whether CODE
/// bytes landed at 0x8000. Caller must hold `trace::test_lock()` when tracing.
fn run_attr_mark_load_path(rom: &[u8]) -> (Machine, bool) {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/tape/attr_mark.tap");
    let img = TapImage::load(&path).expect("attr_mark.tap");
    let mut m = Machine::new_48k(rom).unwrap();
    m.set_tape_load_options(TapeLoadOptions {
        flash_load: true,
        speed: 1,
        ..Default::default()
    });
    m.insert_tape(TapPlayer::new(img));
    for _ in 0..200 {
        let _ = m.run_frame();
    }
    // attr_mark is a CODE block — plain LOAD "" only accepts PROGRAM headers.
    m.type_load_quotes_48k(true);
    m.set_tape_playing(true);
    let mut loaded = false;
    for _ in 0..200 {
        let _ = m.run_frame();
        if m.read_mem(0x8000) == 0x21
            && m.read_mem(0x8001) == 0x00
            && m.read_mem(0x8002) == 0x58
            && m.read_mem(0x8003) == 0x36
            && m.read_mem(0x8004) == 0xd7
            && m.read_mem(0x8005) == 0xc9
        {
            loaded = true;
            break;
        }
    }
    (m, loaded)
}

fn attr_mark_code_ok(m: &Machine) -> bool {
    m.read_mem(0x8000) == 0x21
        && m.read_mem(0x8001) == 0x00
        && m.read_mem(0x8002) == 0x58
        && m.read_mem(0x8003) == 0x36
        && m.read_mem(0x8004) == 0xd7
        && m.read_mem(0x8005) == 0xc9
}

fn run_attr_mark_typed(
    mut m: Machine,
    img: TapImage,
    flash_load: bool,
    speed: u32,
    warmup: u32,
    max_frames: u32,
) -> (Machine, bool) {
    m.set_tape_load_options(TapeLoadOptions {
        flash_load,
        speed,
        ..Default::default()
    });
    m.insert_tape(TapPlayer::new(img));
    for _ in 0..warmup {
        let _ = m.run_frame();
    }
    m.type_load_quotes(true);
    m.set_tape_playing(true);
    let mut loaded = false;
    for _ in 0..max_frames {
        let _ = m.run_frame();
        if attr_mark_code_ok(&m) {
            loaded = true;
            break;
        }
    }
    (m, loaded)
}

#[test]
fn attr_mark_experience_load_succeeds() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/tape/attr_mark.tap");
    let img = TapImage::load(&path).expect("attr_mark.tap");
    let mut m = Machine::new_48k(&rom).unwrap();
    m.set_tape_load_options(TapeLoadOptions::experience());
    m.insert_tape(TapPlayer::new(img));
    for _ in 0..200 {
        let _ = m.run_frame();
    }
    m.type_load_quotes(true);
    m.set_tape_playing(true);
    let mut loaded = false;
    for _ in 0..800 {
        let _ = m.run_frame();
        if attr_mark_code_ok(&m) {
            loaded = true;
            break;
        }
    }
    assert!(
        loaded,
        "experience LOAD \"\" CODE should poke attr_mark bytes at 0x8000"
    );
    let opts = m.tape_load_options();
    assert!(opts.experience_load);
    assert_eq!(opts.speed, tape::EXPERIENCE_EAR_SPEED);
}

#[test]
fn ld_bytes_waits_while_tape_paused_in_experience_mode() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let img = TapImage::load(&fixture_tap()).expect("fixture");
    let header_len = img.blocks[0].len();
    let mut m = Machine::new_48k(&rom).unwrap();
    m.set_tape_load_options(TapeLoadOptions::experience());
    m.insert_tape(TapPlayer::new(img));
    let ret = 0x12abu16;
    m.cpu_mut().regs.sp = 0x5f00;
    m.write_mem(0x5f00, (ret & 0xff) as u8);
    m.write_mem(0x5f01, (ret >> 8) as u8);
    m.cpu_mut().regs.pc = LD_BYTES_TRAP_PC;
    m.cpu_mut().regs.a_ = 0x00;
    m.cpu_mut().regs.f_ = flag::C;
    m.cpu_mut().regs.set_ix(0x5c00);
    m.cpu_mut().regs.set_de((header_len - 2) as u16);
    if let Machine::Spec48 { bus, .. } = &mut m {
        bus.frame_t = INT_LENGTH_48;
    }
    for _ in 0..64 {
        m.step_once();
        assert_eq!(
            m.cpu().regs.pc,
            LD_BYTES_TRAP_PC,
            "experience must hold at LD-BYTES until Play"
        );
    }
}

#[test]
fn experience_multi_block_load_within_wall_clock_budget() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let payload_len = 1024usize;
    let mut blocks = Vec::new();
    for i in 0..30u8 {
        let mut block = vec![0u8; payload_len + 2];
        block[0] = 0xff;
        block[1..=payload_len].fill(i);
        block[payload_len + 1] = tap_checksum(&block[..=payload_len]);
        blocks.push(block);
    }
    let img = TapImage {
        blocks,
        ..Default::default()
    };
    let mut m = Machine::new_48k(&rom).unwrap();
    m.set_tape_load_options(TapeLoadOptions::experience());
    m.insert_tape(TapPlayer::new(img));
    m.set_tape_playing(true);
    // Hybrid Experience advances TAP via LD-BYTES flash + cosmetic holds
    // (EAR is skipped). Drive each block from the trap like Instant would.
    // Block index advances when the trap is *scheduled*; wait until cosmetic
    // pending clears before starting the next block.
    let max_host_frames = 25 * 50;
    let mut host_frames = 0u32;
    for block_i in 0..30usize {
        // Finish any prior cosmetic hold first.
        while matches!(
            &m,
            Machine::Spec48 {
                tape: Some(TapeDeck::Tap(p)),
                ..
            } if p.experience_pending.is_some()
        ) {
            let _ = m.run_frame();
            host_frames += 1;
            assert!(
                host_frames <= max_host_frames,
                "experience hybrid cosmetic overrun before block {block_i}"
            );
        }
        let ret = 0x12abu16;
        m.cpu_mut().regs.sp = 0x5f00;
        m.write_mem(0x5f00, (ret & 0xff) as u8);
        m.write_mem(0x5f01, (ret >> 8) as u8);
        m.cpu_mut().regs.pc = LD_BYTES_TRAP_PC;
        m.cpu_mut().regs.a = 0;
        m.cpu_mut().regs.f = 0;
        m.cpu_mut().regs.a_ = 0xff;
        m.cpu_mut().regs.f_ = flag::C;
        m.cpu_mut().regs.set_ix(0x8000);
        m.cpu_mut().regs.set_de(payload_len as u16);
        if let Machine::Spec48 { bus, .. } = &mut m {
            bus.frame_t = INT_LENGTH_48;
        }
        let mut saw_pending = false;
        let mut block_frames = 0u32;
        loop {
            let _ = m.run_frame();
            host_frames += 1;
            block_frames += 1;
            let pending = matches!(
                &m,
                Machine::Spec48 {
                    tape: Some(TapeDeck::Tap(p)),
                    ..
                } if p.experience_pending.is_some()
            );
            if pending {
                saw_pending = true;
            }
            if saw_pending && !pending {
                break;
            }
            assert!(
                    block_frames <= 80,
                    "experience hybrid block {block_i} stuck after {block_frames} frames (block={:?} playing={} pc={:04X} pending={pending})",
                    m.tape_block(),
                    m.tape_playing(),
                    m.cpu().regs.pc,
                );
            assert!(
                    host_frames <= max_host_frames,
                    "experience hybrid exceeded {max_host_frames} host frames at block {block_i} (used {host_frames})"
                );
        }
    }
    assert!(
        m.tape_block().is_none_or(|b| b >= 30),
        "expected all 30 blocks consumed, got {:?}",
        m.tape_block()
    );
}

#[test]
fn attr_mark_ear_load_quotes_code_succeeds_at_speed_10() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/tape/attr_mark.tap");
    let img = TapImage::load(&path).expect("attr_mark.tap");
    let m = Machine::new_48k(&rom).unwrap();
    // Speed N runs N Spectrum frames per run_frame while EAR playing.
    let (_m, loaded) = run_attr_mark_typed(m, img, false, 10, 200, 2_000);
    assert!(
        loaded,
        "EAR LOAD \"\" CODE should poke CODE at 0x8000 (speed 10; budget 2000 frames)"
    );
}

#[test]
fn attr_mark_type_load_128k_flash() {
    let Some(rom) = rom128() else {
        eprintln!("skip: roms/128/spec128uk.rom missing");
        return;
    };
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/tape/attr_mark.tap");
    let img = TapImage::load(&path).expect("attr_mark.tap");
    let m = Machine::new_128k(&rom).unwrap();
    let (_m, loaded) = run_attr_mark_typed(m, img, true, 1, 200, 400);
    assert!(
        loaded,
        "128K 48 BASIC LOAD \"\" CODE should flash-load attr_mark"
    );
}

#[test]
fn attr_mark_type_load_plus3_flash() {
    let Some(rom) = rom_plus3() else {
        eprintln!("skip: plus3/plus2a ROM missing");
        return;
    };
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/tape/attr_mark.tap");
    let img = TapImage::load(&path).expect("attr_mark.tap");
    let m = Machine::new_plus3(&rom).unwrap();
    let (_m, loaded) = run_attr_mark_typed(m, img, true, 1, 250, 400);
    assert!(
        loaded,
        "+3 48 BASIC LOAD \"\" CODE should flash-load attr_mark"
    );
}

/// Deterministic tape repro harness (observability + success).
///
/// Runs 48K `LOAD "" CODE` against `tests/fixtures/tape/attr_mark.tap` with
/// the structured trace enabled. On load failure, dumps the ring to stderr.
///
/// Local commercial tape (do **not** commit):
/// `<path-to-local-commercial-tape>/The Boggit - Side 1.tzx`
#[test]
fn attr_mark_load_path_dumps_trace_on_failure() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };

    let _lock = trace::test_lock();
    struct TraceRestore;
    impl Drop for TraceRestore {
        fn drop(&mut self) {
            trace::disable();
            trace::clear();
        }
    }
    let _restore = TraceRestore;
    trace::clear();
    trace::enable(trace::Category::DEFAULT | trace::Category::TAPE);

    let (m, loaded) = run_attr_mark_load_path(&rom);
    let dump = trace::dump_string();
    if !loaded {
        eprintln!("=== attr_mark LOAD path failed — dumping trace ===");
        eprintln!(
            "PC={:04X} tape_block={:?} playing={} AF'={:02X}{:02X}",
            m.cpu().regs.pc,
            m.tape_block(),
            m.tape_playing(),
            m.cpu().regs.a_,
            m.cpu().regs.f_
        );
        eprintln!("{dump}");
        let _ = trace::dump_to_env_file();
    }

    assert!(
        dump.contains("tape.play")
            || dump.contains("tape.flash")
            || dump.contains("tape.block")
            || dump.contains("tape.ear_rate"),
        "expected tape.* trace events when exercising LOAD path; dump head:\n{}",
        dump.chars().take(1200).collect::<String>()
    );
    assert!(
        dump.contains("tape.flash.enter") && dump.contains("tape.flash.exit"),
        "expected flash-load enter/exit; dump head:\n{}",
        dump.chars().take(1200).collect::<String>()
    );
    assert!(
        loaded,
        "attr_mark LOAD \"\" CODE did not place CODE at 0x8000"
    );
    eprintln!("attr_mark LOAD \"\" CODE succeeded (CODE at 0x8000)");
}

/// Hard success gate for `attr_mark` `LOAD "" CODE`.
#[test]
fn attr_mark_load_path_must_succeed() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let _lock = trace::test_lock();
    struct TraceRestore;
    impl Drop for TraceRestore {
        fn drop(&mut self) {
            trace::disable();
            trace::clear();
        }
    }
    let _restore = TraceRestore;
    trace::clear();
    trace::enable(trace::Category::DEFAULT | trace::Category::TAPE);
    let (_m, loaded) = run_attr_mark_load_path(&rom);
    if !loaded {
        eprintln!("=== attr_mark hard gate failed — dumping trace ===");
        trace::dump_to_stderr();
    }
    assert!(loaded, "attr_mark CODE missing at 0x8000");
}

/// `print_ok.tap` is a PROGRAM — plain `LOAD ""` must flash-load both blocks.
#[test]
fn print_ok_load_quotes_succeeds() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/tape/print_ok.tap");
    let img = TapImage::load(&path).expect("print_ok.tap");
    let mut m = Machine::new_48k(&rom).unwrap();
    m.set_tape_load_options(TapeLoadOptions {
        flash_load: true,
        speed: 1,
        ..Default::default()
    });
    m.insert_tape(TapPlayer::new(img));
    for _ in 0..200 {
        let _ = m.run_frame();
    }
    m.type_load_quotes_48k(false);
    assert_eq!(
        m.cpu().regs.pc,
        LD_BYTES_TRAP_PC,
        "LOAD \"\" should reach LD-BYTES while paused"
    );
    m.set_tape_playing(true);
    let mut done = false;
    for _ in 0..200 {
        let _ = m.run_frame();
        // Both TAP blocks consumed and back in the editor / running.
        if m.tape_block() == Some(2) || m.tape_block().is_none() {
            // Program line 10 starts with length bytes; look for PRINT token 0xF5
            // or the "OK" string in the loaded BASIC area.
            let prog = u16::from_le_bytes([m.read_mem(0x5C53), m.read_mem(0x5C54)]);
            let eline = u16::from_le_bytes([m.read_mem(0x5C59), m.read_mem(0x5C5A)]);
            let mut found_ok = false;
            for a in prog..eline {
                if m.read_mem(a) == b'O' && m.read_mem(a.wrapping_add(1)) == b'K' {
                    found_ok = true;
                    break;
                }
            }
            if found_ok {
                done = true;
                break;
            }
        }
    }
    assert!(
        done,
        "print_ok LOAD \"\" should place BASIC containing OK (PC={:04X} block={:?})",
        m.cpu().regs.pc,
        m.tape_block()
    );
}

#[test]
fn flash_load_skip_appears_in_trace_dump() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let img = TapImage {
        blocks: vec![vec![0xff, 0x11, 0xff ^ 0x11], vec![0x00, 0x22, 0x22]],
        ..Default::default()
    };
    let _lock = trace::test_lock();
    trace::clear();
    trace::enable(trace::Category::TAPE);
    let mut m = Machine::new_48k(&rom).unwrap();
    m.set_tape_load_options(TapeLoadOptions {
        flash_load: true,
        speed: 1,
        ..Default::default()
    });
    m.insert_tape(TapPlayer::new(img));
    m.set_tape_playing(true);
    // Expect header flag 0x00 but first block is data 0xff → skip then load.
    let ret = 0x7000u16;
    m.cpu_mut().regs.sp = 0x5f00;
    m.write_mem(0x5f00, (ret & 0xff) as u8);
    m.write_mem(0x5f01, (ret >> 8) as u8);
    m.cpu_mut().regs.pc = LD_BYTES_TRAP_PC;
    m.cpu_mut().regs.a_ = 0x00;
    m.cpu_mut().regs.f_ = flag::C;
    m.cpu_mut().regs.set_ix(0x5c00);
    m.cpu_mut().regs.set_de(1);
    if let Machine::Spec48 { bus, .. } = &mut m {
        bus.frame_t = INT_LENGTH_48;
    }
    m.step_once();
    let dump = trace::dump_string();
    assert!(
        dump.contains("wrong_flag") || dump.contains("tape.flash"),
        "dump=\n{dump}"
    );
    trace::disable();
    trace::clear();
}

#[test]
fn inspect_after_boot_steps() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let mut m = Machine::new_48k(&rom).unwrap();
    for _ in 0..8 {
        m.step_once();
    }
    let i = m.inspect();
    assert_eq!(i.model, Model::Spectrum48);
    assert!(i.cpu_t > 0);
    assert_eq!(i.frame_tstates, FRAME_TSTATES_48);
    let hex = m.hexdump(0x0000, 16);
    assert!(hex.contains("0000"));
    let d = m.disasm_window(0x0000, 4);
    assert!(d.contains("0000"));
    let json = i.to_json();
    assert!(json.contains("\"model\":\"48k\""));
}

#[test]
fn inspect_128k_paging() {
    let Some(rom) = rom128() else {
        eprintln!("skip: roms/128/spec128uk.rom missing");
        return;
    };
    let m = Machine::new_128k(&rom).unwrap();
    let i = m.inspect();
    assert_eq!(i.model, Model::Spectrum128);
    assert!(i.paging.page_7ffd.is_some());
}

#[test]
fn apply_sna48_sets_pc_ram_and_border() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let mut data = vec![0u8; 49179];
    data[26] = 5; // border
    data[23] = 0x00;
    data[24] = 0x40; // SP = 0x4000 → pop PC from RAM[0x4000]
    data[27] = 0x00;
    data[28] = 0x80; // PC = 0x8000
    data[27 + 0x4000] = 0xaa; // byte at 0x8000
    let snap = Snapshot48::parse_sna(&data).expect("synthetic SNA48");

    let _lock = trace::test_lock();
    let dump = trace::with_trace(trace::Category::MACHINE, || {
        let mut m = Machine::new_48k(&rom).unwrap();
        m.apply_snapshot48(&snap);
        let i = m.inspect();
        assert_eq!(i.regs.pc, 0x8000);
        assert_eq!(m.read_mem(0x8000), 0xaa);
        assert_eq!(i.border, 5);
        trace::dump_string()
    });
    assert!(
        dump.contains("machine.snapshot"),
        "expected machine.snapshot in dump:\n{dump}"
    );
}

/// Minimal uncompressed Z80 v1 for `apply_snapshot48` golden.
fn synthetic_z80_v1_for_machine() -> Vec<u8> {
    let mut data = vec![0u8; 30 + 49152];
    data[0] = 0x11; // A
    data[1] = 0x22; // F
    data[6] = 0x00;
    data[7] = 0x81; // PC = 0x8100
    data[8] = 0x00;
    data[9] = 0x70; // SP = 0x7000
    data[12] = (6 << 1) & 0x0e; // border 6, uncompressed
    data[30] = 0xbe; // RAM @ Spectrum 0x4000
    data[31] = 0xef;
    data[30 + 0x1000] = 0x42; // Spectrum 0x5000
    data
}

#[test]
fn apply_z80_snapshot48_sets_pc_ram_and_border() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let snap = Snapshot48::parse_z80(&synthetic_z80_v1_for_machine()).expect("z80 v1");
    let mut m = Machine::new_48k(&rom).unwrap();
    m.apply_snapshot48(&snap);
    let i = m.inspect();
    assert_eq!(i.regs.pc, 0x8100);
    assert_eq!(i.regs.sp, 0x7000);
    assert_eq!(i.border, 6);
    assert_eq!(m.read_mem(0x4000), 0xbe);
    assert_eq!(m.read_mem(0x4001), 0xef);
    assert_eq!(m.read_mem(0x5000), 0x42);
}

fn synthetic_z80_v2_128_for_machine() -> Vec<u8> {
    let mut data = vec![0u8; 55];
    data[0] = 0x11;
    data[1] = 0x22;
    data[6] = 0;
    data[7] = 0;
    data[8] = 0x00;
    data[9] = 0x70; // SP
    data[12] = (3 << 1) & 0x0e; // border 3
    data[30] = 23;
    data[31] = 0;
    data[32] = 0x00;
    data[33] = 0x90; // PC = 0x9000
    data[34] = 3; // v2 128K
    data[35] = 0x04; // 7FFD: bank 4 at C000
    for page in 3u8..=10 {
        let bank = page - 3;
        data.extend_from_slice(&0xffffu16.to_le_bytes());
        data.push(page);
        let mut page_ram = vec![0u8; 16384];
        page_ram[0] = 0xf0 | bank;
        page_ram[1] = bank;
        data.extend_from_slice(&page_ram);
    }
    data
}

fn synthetic_z80_v3_plus3_for_machine() -> Vec<u8> {
    let mut data = vec![0u8; 87];
    data[6] = 0;
    data[7] = 0;
    data[8] = 0xfe;
    data[9] = 0xff;
    data[12] = 7 << 1; // bits 1–3: last OUT to 7FFD bank select (bank 7)
    data[30] = 55;
    data[31] = 0;
    data[32] = 0x00;
    data[33] = 0xc0; // PC in paged bank window
    data[34] = 7; // +3
    data[35] = 0x01; // bank 1 at C000
    data[86] = 0x04; // 1FFD ROM high
    for page in 3u8..=10 {
        let bank = page - 3;
        data.extend_from_slice(&0xffffu16.to_le_bytes());
        data.push(page);
        let mut page_ram = vec![0u8; 16384];
        page_ram[0] = 0xa0 | bank;
        data.extend_from_slice(&page_ram);
    }
    data
}

#[test]
fn apply_snapshot128_z80_pages_and_7ffd() {
    let Some(rom) = rom128() else {
        eprintln!("skip: roms/128/spec128uk.rom missing");
        return;
    };
    let snap = Snapshot128::parse_z80(&synthetic_z80_v2_128_for_machine()).expect("z80 128");
    let mut m = Machine::new_128k(&rom).unwrap();
    m.apply_snapshot128(&snap);
    let i = m.inspect();
    assert_eq!(i.regs.pc, 0x9000);
    assert_eq!(i.border, 3);
    assert_eq!(i.paging.page_7ffd, Some(0x04));
    assert_eq!(m.read_mem(0x4000), 0xf5); // bank 5
    assert_eq!(m.read_mem(0x8000), 0xf2); // bank 2
    assert_eq!(m.read_mem(0xc000), 0xf4); // bank 4 via 7FFD
    if let Machine::Spec128 { bus, .. } = &m {
        assert_eq!(bus.banks[0][0], 0xf0);
        assert_eq!(bus.banks[7][0], 0xf7);
        assert_eq!(bus.page, 0x04);
    } else {
        panic!("expected Spec128");
    }
}

#[test]
fn apply_snapshot128_plus3_applies_1ffd() {
    let Some(rom) = rom_plus3() else {
        eprintln!("skip: plus3/plus2a ROM missing");
        return;
    };
    let snap = Snapshot128::parse_z80(&synthetic_z80_v3_plus3_for_machine()).expect("z80 +3");
    let mut m = Machine::new_plus3(&rom).unwrap();
    m.apply_snapshot128(&snap);
    let i = m.inspect();
    assert_eq!(i.regs.pc, 0xc000);
    assert_eq!(i.paging.page_7ffd, Some(0x01));
    assert_eq!(i.paging.page_1ffd, Some(0x04));
    assert_eq!(m.read_mem(0xc000), 0xa1); // bank 1
    if let Machine::SpecPlus3 { bus, .. } = &m {
        assert_eq!(bus.page_1ffd, 0x04);
        assert_eq!(bus.banks[6][0], 0xa6);
    } else {
        panic!("expected SpecPlus3");
    }
}

/// Uncompressed RZX input block (same layout as `formats::rzx` tests).
fn minimal_rzx(frames: &[(u16, &[u8])]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(b"RZX!");
    v.extend_from_slice(&[0x00, 0x0d]);
    v.extend_from_slice(&[0, 0, 0, 0]);
    let mut body = Vec::new();
    body.extend_from_slice(&[0, 0, 0, 0]);
    body.push(0); // uncompressed
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
fn rzx_replay_applies_keyboard_and_kempston() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    // Frame 0: row 1 keys=0x01 (byte 0x21). Frame 1: Kempston 0x15 (byte 0x95).
    // Frame 2: row 0 keys=0x10 (byte 0x10).
    let data = minimal_rzx(&[(100, &[0x21]), (100, &[0x95]), (100, &[0x10])]);
    let rec = RzxRecording::parse(&data).expect("rzx");
    let mut m = Machine::new_48k(&rom).unwrap();
    m.insert_rzx(rec);

    m.run_frame();
    assert_eq!(m.keyboard_mut().rows[1], 0x01);

    m.run_frame();
    assert_eq!(m.kempston_mut().read(), 0x15);
    if let Machine::Spec48 { bus, .. } = &mut m {
        assert_eq!(bus.in_port(0x001f), 0x15);
    }

    m.run_frame();
    assert_eq!(m.keyboard_mut().rows[0], 0x10);
}

fn minimal_tzx_turbo_machine(payload: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(b"ZXTape!");
    v.extend_from_slice(&[0x1a, 1, 20]);
    v.push(0x11);
    v.extend_from_slice(&800u16.to_le_bytes());
    v.extend_from_slice(&400u16.to_le_bytes());
    v.extend_from_slice(&400u16.to_le_bytes());
    v.extend_from_slice(&300u16.to_le_bytes());
    v.extend_from_slice(&600u16.to_le_bytes());
    v.extend_from_slice(&20u16.to_le_bytes());
    v.push(8);
    v.extend_from_slice(&50u16.to_le_bytes());
    let len = payload.len() as u32;
    v.push((len & 0xff) as u8);
    v.push(((len >> 8) & 0xff) as u8);
    v.push(((len >> 16) & 0xff) as u8);
    v.extend_from_slice(payload);
    v
}

#[test]
fn turbo_tzx_ear_advances_without_flash_path() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let data = minimal_tzx_turbo_machine(&[0xff, 0x00, 0xaa]);
    assert!(
        !tape::TzxPlayer::is_standard_speed_only(&data),
        "turbo must not be treated as standard-speed TAP"
    );
    let tap = tape::TzxPlayer::to_tap_image(&data).expect("to_tap");
    assert!(
        tap.blocks.is_empty(),
        "flash/TAP extraction must skip ID 0x11"
    );

    let player = TzxPlayer::parse(&data).expect("tzx");
    assert!(player.scheduled_pulses() > 20);

    let mut m = Machine::new_48k(&rom).unwrap();
    m.set_tape_load_options(TapeLoadOptions {
        flash_load: false,
        speed: 1,
        ..Default::default()
    });
    m.insert_tzx(player);
    m.set_tape_playing(true);
    assert!(!m.ear(), "EAR idle before pulse advance");

    let mut saw_edges = false;
    let mut saw_progress = false;
    let mut last_pulse = 0u32;
    for _ in 0..8 {
        let audio = m.run_frame();
        if !audio.beeper_edges.is_empty() {
            saw_edges = true;
        }
        if let Some(p) = m.tape_progress() {
            if p.pulse_index > last_pulse {
                saw_progress = true;
                last_pulse = p.pulse_index;
            }
        }
    }
    // Tiny turbo decks can finish within a frame; idle EAR is forced low on
    // exhaust (#178), so end-of-frame `ear()` is not a reliable high sample.
    assert!(saw_edges, "turbo pilot must emit EAR/beeper edges");
    assert!(saw_progress, "pulse index must advance under EAR path");
    assert!(
        m.tape_progress().map_or(0, |p| p.pulse_count) > 0,
        "turbo deck reports scheduled pulses"
    );
}

#[test]
fn until_pc_and_mem_watch() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let mut m = Machine::new_48k(&rom).unwrap();
    m.write_mem(0x8000, 0x18);
    m.write_mem(0x8001, 0xfe); // JR $
    m.cpu_mut().regs.pc = 0x8000;
    m.debugger_mut().add_pc_break(0x8000);
    let reason = m.run_until_break(8);
    assert_eq!(reason, BreakReason::Pc(0x8000));
    assert_eq!(m.cpu().regs.pc, 0x8000);

    let mut m = Machine::new_48k(&rom).unwrap();
    m.write_mem(0x8000, 0x00); // NOP
    m.write_mem(0x8001, 0x00);
    m.cpu_mut().regs.pc = 0x8000;
    if let Machine::Spec48 { bus, .. } = &mut m {
        bus.frame_t = INT_LENGTH_48;
    }
    assert_eq!(m.run_until_break(2), BreakReason::Budget);
    let pc_after = m.cpu().regs.pc;
    assert_eq!(m.run_until_break(8), BreakReason::Budget);
    assert_ne!(
        m.cpu().regs.pc,
        pc_after,
        "second budget run must not stall"
    );

    let mut m = Machine::new_48k(&rom).unwrap();
    m.write_mem(0x8000, 0x77); // LD (HL),A
    m.cpu_mut().regs.pc = 0x8000;
    m.cpu_mut().regs.set_hl(0x4000);
    m.cpu_mut().regs.a = 0xaa;
    if let Machine::Spec48 { bus, .. } = &mut m {
        bus.frame_t = INT_LENGTH_48; // avoid IRQ
    }
    m.debugger_mut()
        .add_mem_watch(Watch::new(0x4000, false, true));
    m.step_once();
    assert_eq!(m.read_mem(0x4000), 0xaa);
    assert!(matches!(
        m.debugger().last_hit,
        BreakReason::Mem {
            addr: 0x4000,
            write: true,
            value: 0xaa
        }
    ));

    let mut m = Machine::new_48k(&rom).unwrap();
    m.write_mem(0x8000, 0x77); // LD (HL),A
    m.cpu_mut().regs.pc = 0x8000;
    m.cpu_mut().regs.set_hl(0x4000);
    m.cpu_mut().regs.a = 0x55;
    m.debugger_mut()
        .add_mem_watch(Watch::new(0x4000, false, true));
    m.run_frame();
    assert_eq!(m.read_mem(0x4000), 0x55);
    assert!(m.debugger().paused);
    assert!(matches!(
        m.debugger().last_hit,
        BreakReason::Mem {
            addr: 0x4000,
            write: true,
            value: 0x55
        }
    ));
    assert!(m.frame_t() > 0, "mid-frame watch must keep raster time");
}

#[test]
fn port_read_watch_fires_on_in_a_c() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let mut m = Machine::new_48k(&rom).unwrap();
    // LD BC,$00FE / IN A,(C) / HALT
    m.write_mem(0x8000, 0x01);
    m.write_mem(0x8001, 0xFE);
    m.write_mem(0x8002, 0x00);
    m.write_mem(0x8003, 0xED);
    m.write_mem(0x8004, 0x78);
    m.write_mem(0x8005, 0x76);
    m.cpu_mut().regs.pc = 0x8000;
    if let Machine::Spec48 { bus, .. } = &mut m {
        bus.frame_t = INT_LENGTH_48;
    }
    m.debugger_mut()
        .add_port_watch(Watch::new(0x00FE, true, false));
    let reason = m.run_until_break(16);
    assert!(
        matches!(
            reason,
            BreakReason::Port {
                port: 0x00FE,
                write: false,
                ..
            }
        ),
        "expected Port{{00FE read}}, got {reason:?}"
    );
    assert!(m.debugger().paused);

    // Row-select keyboard ports (high byte ≠ 0) must NOT match an exact $00FE watch.
    let mut m = Machine::new_48k(&rom).unwrap();
    m.write_mem(0x8000, 0x01); // LD BC,$3CFE
    m.write_mem(0x8001, 0xFE);
    m.write_mem(0x8002, 0x3C);
    m.write_mem(0x8003, 0xED); // IN A,(C)
    m.write_mem(0x8004, 0x78);
    m.write_mem(0x8005, 0x76);
    m.cpu_mut().regs.pc = 0x8000;
    if let Machine::Spec48 { bus, .. } = &mut m {
        bus.frame_t = INT_LENGTH_48;
    }
    m.debugger_mut()
        .add_port_watch(Watch::new(0x00FE, true, false));
    let reason = m.run_until_break(16);
    assert!(
        matches!(reason, BreakReason::Halt | BreakReason::Budget),
        "exact $00FE watch must miss $3CFE row poll, got {reason:?}"
    );

    // Low-byte mask catches any keyboard-row IN (#387).
    let mut m = Machine::new_48k(&rom).unwrap();
    m.write_mem(0x8000, 0x01); // LD BC,$3CFE
    m.write_mem(0x8001, 0xFE);
    m.write_mem(0x8002, 0x3C);
    m.write_mem(0x8003, 0xED); // IN A,(C)
    m.write_mem(0x8004, 0x78);
    m.write_mem(0x8005, 0x76);
    m.cpu_mut().regs.pc = 0x8000;
    if let Machine::Spec48 { bus, .. } = &mut m {
        bus.frame_t = INT_LENGTH_48;
    }
    m.debugger_mut()
        .add_port_watch(Watch::with_mask(0x00FE, 0x00FF, true, false));
    let reason = m.run_until_break(16);
    assert!(
        matches!(
            reason,
            BreakReason::Port {
                port: 0x3CFE,
                write: false,
                ..
            }
        ),
        "masked $00FE/$00FF watch must hit $3CFE row poll, got {reason:?}"
    );
    assert!(m.debugger().paused);
}

#[test]
fn cpu_step_appears_in_trace() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let _lock = trace::test_lock();
    trace::clear();
    trace::enable(trace::Category::CPU);
    let mut m = Machine::new_48k(&rom).unwrap();
    if let Machine::Spec48 { bus, .. } = &mut m {
        bus.frame_t = INT_LENGTH_48;
    }
    m.step_once();
    let dump = trace::dump_string();
    assert!(dump.contains("cpu.step"), "dump=\n{dump}");
    let json = trace::dump_json();
    assert!(json.contains("cpu"));
    trace::disable();
    trace::clear();
}

fn custom_loader_tap() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/tape/custom_loader.tap")
}

fn custom_loader_ok(m: &Machine) -> bool {
    m.read_mem(0x9000) == 0xa5
}

fn test_machine(model: Model, rom: &[u8]) -> Machine {
    match model {
        Model::Spectrum16K => Machine::new_16k(rom).expect("16K machine"),
        Model::Spectrum48 => Machine::new_48k(rom).expect("48K machine"),
        Model::Spectrum128 => Machine::new_128k(rom).expect("128K machine"),
        Model::SpectrumPlus2 => Machine::new_plus2(rom).expect("+2 machine"),
        Model::SpectrumPlus2A => Machine::new_plus2a(rom).expect("+2A machine"),
        Model::SpectrumPlus3 => Machine::new_plus3(rom).expect("+3 machine"),
        Model::SpectrumPlus3e => Machine::new_plus3e(rom).expect("+3e machine"),
        Model::Pentagon128 => {
            let trdos = read_trdos_rom(model).expect("pentagon trdos");
            Machine::new_pentagon128(rom, &trdos).expect("Pentagon machine")
        }
        Model::ScorpionZs256 => {
            let trdos = read_trdos_rom(model).expect("scorpion trdos");
            Machine::new_scorpion_zs256(rom, &trdos).expect("Scorpion machine")
        }
        Model::TimexTC2048 => Machine::new_timex_tc2048(rom).expect("TC2048 machine"),
        Model::TimexTS2068 => {
            let exrom = read_exrom(model).expect("TS2068 EX-ROM");
            Machine::new_timex_ts2068(rom, &exrom).expect("TS2068 machine")
        }
    }
}

fn run_typed_load(
    mut m: Machine,
    img: TapImage,
    flash_load: bool,
    speed: u32,
    with_code: bool,
    warmup: u32,
    max_frames: u32,
    done: impl Fn(&Machine) -> bool,
) -> (Machine, bool) {
    m.set_tape_load_options(TapeLoadOptions {
        flash_load,
        speed,
        ..Default::default()
    });
    m.insert_tape(TapPlayer::new(img));
    for _ in 0..warmup {
        let _ = m.run_frame();
    }
    m.type_load_quotes(with_code);
    m.set_tape_playing(true);
    let mut loaded = false;
    for _ in 0..max_frames {
        let _ = m.run_frame();
        if done(&m) {
            loaded = true;
            break;
        }
    }
    (m, loaded)
}

/// `attr_mark` CODE across models × Instant + EAR speeds (CLI `--speed` 1..=64).
#[test]
fn attr_mark_load_matrix_models_and_speeds() {
    let speeds = [1u32, 2, 5, 10, 20];
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/tape/attr_mark.tap");
    let img = TapImage::load(&path).expect("attr_mark");

    let mut cases: Vec<(Model, Vec<u8>, &str)> = Vec::new();
    if let Some(r) = rom48() {
        cases.push((Model::Spectrum48, r, "48k"));
    }
    if let Some(r) = rom128() {
        cases.push((Model::Spectrum128, r, "128k"));
    }
    if let Some(r) = rom_plus3() {
        cases.push((Model::SpectrumPlus3, r, "plus3"));
    }
    if let Some(r) = rom_timex_tc2048() {
        cases.push((Model::TimexTC2048, r, "timex"));
    }
    if let Some((home, _)) = rom_timex_ts2068() {
        cases.push((Model::TimexTS2068, home, "ts2068"));
    }
    if cases.is_empty() {
        eprintln!("skip: no ROMs for attr_mark matrix");
        return;
    }

    let mut report = String::from("attr_mark matrix:\n");
    let mut failed = Vec::new();
    for (model, rom, label) in &cases {
        let warmup = if matches!(
            model,
            Model::Spectrum48 | Model::TimexTC2048 | Model::TimexTS2068
        ) {
            200
        } else {
            250
        };
        // Instant / flash-load traps Spectrum LD-BYTES PCs; Timex BASIC differs.
        if matches!(model, Model::TimexTS2068) {
            report.push_str(&format!(
                "  {label} instant: SKIP (Timex ROM — no Spectrum flash trap)\n"
            ));
        } else {
            let (_m, ok) = run_typed_load(
                test_machine(*model, rom),
                img.clone(),
                true,
                1,
                true,
                warmup,
                500,
                attr_mark_code_ok,
            );
            report.push_str(&format!(
                "  {label} instant: {}\n",
                if ok { "PASS" } else { "FAIL" }
            ));
            if !ok {
                failed.push(format!("{label}/instant"));
            }
        }
        let full = std::env::var_os("SPEC_CHUM_FULL_TAPE_MATRIX").is_some();
        for speed in speeds {
            // EAR@1 is slow (~minutes of Spectrum time); default CI keeps it
            // on 48K only. Set SPEC_CHUM_FULL_TAPE_MATRIX=1 for 128K/+3 @1×.
            if speed == 1
                && !matches!(
                    model,
                    Model::Spectrum48 | Model::TimexTC2048 | Model::TimexTS2068
                )
                && !full
            {
                report.push_str(&format!(
                    "  {label} ear@{speed}: SKIP (set SPEC_CHUM_FULL_TAPE_MATRIX=1)\n"
                ));
                continue;
            }
            let max = match speed {
                1 => 25_000,
                2 => 15_000,
                5 => 6_000,
                10 => 3_000,
                _ => 2_000,
            };
            let (_m, ok) = run_typed_load(
                test_machine(*model, rom),
                img.clone(),
                false,
                speed,
                true,
                warmup,
                max,
                attr_mark_code_ok,
            );
            report.push_str(&format!(
                "  {label} ear@{speed}: {}\n",
                if ok { "PASS" } else { "FAIL" }
            ));
            if !ok {
                failed.push(format!("{label}/ear@{speed}"));
            }
        }
    }
    eprintln!("{report}");
    assert!(
        failed.is_empty(),
        "attr_mark failures: {}",
        failed.join(", ")
    );
}

/// Boggit-style PROGRAM + CODE + flag `0xC8` via RAM LD-BYTES clone.

#[test]
fn custom_loader_matrix_models_instant_and_ear() {
    let path = custom_loader_tap();
    let img = TapImage::load(&path).expect("custom_loader");
    assert_eq!(img.blocks.len(), 3);

    let mut cases: Vec<(Model, Vec<u8>, &str)> = Vec::new();
    if let Some(r) = rom48() {
        cases.push((Model::Spectrum48, r, "48k"));
    }
    if let Some(r) = rom128() {
        cases.push((Model::Spectrum128, r, "128k"));
    }
    if let Some(r) = rom_plus3() {
        cases.push((Model::SpectrumPlus3, r, "plus3"));
    }
    if let Some(r) = rom_timex_tc2048() {
        cases.push((Model::TimexTC2048, r, "timex"));
    }
    // Timex TS2068: Timex BASIC is not Spectrum-compatible for this custom-loader
    // fixture (attr_mark EAR covers TS2068 tape via the shared match arms).
    if cases.is_empty() {
        eprintln!("skip: no ROMs for custom_loader matrix");
        return;
    }

    let mut report = String::from("custom_loader matrix:\n");
    let mut failed = Vec::new();
    for (model, rom, label) in &cases {
        let warmup = if matches!(
            model,
            Model::Spectrum48 | Model::TimexTC2048 | Model::TimexTS2068
        ) {
            200
        } else {
            250
        };
        // Instant + EAR speeds matching CLI `--speed` presets.
        let full = std::env::var_os("SPEC_CHUM_FULL_TAPE_MATRIX").is_some();
        let mut modes: Vec<(bool, u32, &str, u32)> =
            vec![(true, 1, "instant", 800), (false, 2, "ear@2", 12_000)];
        for (speed, tag, max) in [
            (5u32, "ear@5", 6_000u32),
            (10, "ear@10", 4_000),
            (20, "ear@20", 2_500),
        ] {
            modes.push((false, speed, tag, max));
        }
        if matches!(
            model,
            Model::Spectrum48 | Model::TimexTC2048 | Model::TimexTS2068
        ) || full
        {
            modes.insert(1, (false, 1, "ear@1", 25_000));
        } else {
            report.push_str(&format!(
                "  {label} ear@1: SKIP (set SPEC_CHUM_FULL_TAPE_MATRIX=1)\n"
            ));
        }
        for (flash, speed, tag, max) in modes {
            let mut m = test_machine(*model, rom);
            m.set_tape_load_options(TapeLoadOptions {
                flash_load: flash,
                speed,
                ..Default::default()
            });
            m.insert_tape(TapPlayer::new(img.clone()));
            for _ in 0..warmup {
                let _ = m.run_frame();
            }
            m.type_load_quotes(true); // LOAD "" CODE
            m.set_tape_playing(true);
            let mut code_ready = false;
            for _ in 0..max {
                let _ = m.run_frame();
                // Wait until CODE data has finished (block ≥ 2), not merely the first byte.
                if m.tape_block().is_some_and(|b| b >= 2)
                    && m.read_mem(0x8000) == 0xdd
                    && m.read_mem(0x800a) == 0xcd
                {
                    code_ready = true;
                    break;
                }
            }
            if code_ready {
                // Instant must leave the C8 block queued (no EAR race). EAR may
                // already be into the post-CODE pause / next pilot while BASIC
                // returns, so rewind before USR for those cells only.
                if flash {
                    assert_eq!(
                        m.tape_block(),
                        Some(2),
                        "{label} instant must not advance past CODE into C8 before USR"
                    );
                } else {
                    // Pause while rewinding/arming USR so multi-frame turbo cannot
                    // burn C8 pilot before the RAM loader starts edge-detect.
                    m.set_tape_playing(false);
                    if let Some(TapeDeck::Tap(p)) = match &mut m {
                        Machine::Spec48 { tape, .. }
                        | Machine::Spec128 { tape, .. }
                        | Machine::SpecPlus3 { tape, .. } => tape.as_mut(),
                    } {
                        p.rewind_to_block(2);
                    }
                }
                // RANDOMIZE USR 32768 — enter the custom-flag loader.
                let ret = 0x15e6u16;
                m.cpu_mut().regs.sp = 0xfffd;
                m.write_mem(0xfffd, (ret & 0xff) as u8);
                m.write_mem(0xfffe, (ret >> 8) as u8);
                m.cpu_mut().regs.halted = false;
                m.cpu_mut().regs.pc = 0x8000;
                // Fake USR return (0x15E6) is not a real BASIC continuation: the
                // C8 byte at 0x9000 is visible for only ~15 frames then cleared.
                // Poll at 1× so warp-S host ticks cannot skip that window; the
                // earlier LOAD "" CODE phase still used full EAR warp.
                if !flash {
                    m.set_tape_load_options(TapeLoadOptions {
                        flash_load: false,
                        speed: 1,
                        ..Default::default()
                    });
                }
                m.set_tape_playing(true);
                for _ in 0..max {
                    let _ = m.run_frame();
                    if custom_loader_ok(&m) {
                        break;
                    }
                }
            }
            let ok = custom_loader_ok(&m);
            report.push_str(&format!(
                    "  {label} {tag}: {} code_ready={code_ready} PC={:04X} block={:?} 8000={:02X} 9000={:02X}\n",
                    if ok { "PASS" } else { "FAIL" },
                    m.cpu().regs.pc,
                    m.tape_block(),
                    m.read_mem(0x8000),
                    m.read_mem(0x9000),
                ));
            if !ok {
                failed.push(format!("{label}/{tag}"));
            }
        }
    }
    eprintln!("{report}");
    assert!(
        failed.is_empty(),
        "custom_loader failures: {}",
        failed.join(", ")
    );
}

/// Optional: Boggit Side 1 through custom `0xC8` loads (set `SPEC_CHUM_BOGGIT_TZX`).
///
/// Default: Instant + EAR@2 on 48K/128K/+3 (complete to block 8 / `JP 5B00`).
/// EAR@5+ shortens inter-block pauses below what Boggit's RAM loader needs —
/// those cells are skipped unless `SPEC_CHUM_FULL_TAPE_MATRIX=1` (still may fail).
/// `SPEC_CHUM_FULL_TAPE_MATRIX=1` also adds EAR@1.
#[test]
fn boggit_side1_matrix_when_present() {
    let Some(boggit) = std::env::var_os("SPEC_CHUM_BOGGIT_TZX").map(PathBuf::from) else {
        eprintln!("skip: set SPEC_CHUM_BOGGIT_TZX for full Boggit matrix");
        return;
    };
    if !boggit.is_file() {
        eprintln!("skip: SPEC_CHUM_BOGGIT_TZX not a file");
        return;
    }
    let data = std::fs::read(&boggit).expect("boggit");
    assert!(
        tape::TzxPlayer::is_standard_speed_only(&data),
        "SPEC_CHUM_BOGGIT_TZX must be standard-speed (0x10) only for TAP conversion"
    );
    let player = tape::TzxPlayer::to_tap_player(&data).expect("to tap");
    let img = player.image.clone();
    // PROG+CODE = blocks 0..3; first custom `0xC8` is block 4. Require that block
    // consumed (index ≥ 5) or game entry. Full Side-1 (block ≥ 8) is Instant-fast;
    // EAR@5+ still bit-accurate so huge C8s need minutes — not required for CI.
    // PROG+CODE = blocks 0..3; four custom `0xC8` blocks are 4..7 → success at ≥8.
    let done = |m: &Machine| m.cpu().regs.pc == 0x5b00 || m.tape_block().is_some_and(|b| b >= 8);

    let full = std::env::var_os("SPEC_CHUM_FULL_TAPE_MATRIX").is_some();
    let mut report = String::from("boggit matrix:\n");
    let mut failed = Vec::new();
    for (rom, label, model) in [
        (rom48(), "48k", Model::Spectrum48),
        (rom128(), "128k", Model::Spectrum128),
        (rom_plus3(), "plus3", Model::SpectrumPlus3),
    ] {
        let Some(rom) = rom else {
            report.push_str(&format!("  {label}: SKIP (no ROM)\n"));
            continue;
        };
        let mut modes: Vec<(bool, u32, &str, u32)> =
            vec![(true, 1, "instant", 2_000), (false, 2, "ear@2", 20_000)];
        if full {
            modes.push((false, 1, "ear@1", 40_000));
            for (speed, tag, max) in [
                (5u32, "ear@5", 10_000u32),
                (10, "ear@10", 8_000),
                (20, "ear@20", 5_000),
            ] {
                modes.push((false, speed, tag, max));
            }
        } else {
            report.push_str(&format!(
                    "  {label} ear@1/@5/@10/@20: SKIP (set SPEC_CHUM_FULL_TAPE_MATRIX=1; ≥5x may fail)\n"
                ));
        }
        for (flash, speed, tag, max) in modes {
            let mut machine = test_machine(model, &rom);
            let deck = TapPlayer::new(img.clone());
            machine.set_tape_load_options(TapeLoadOptions {
                flash_load: flash,
                speed,
                ..Default::default()
            });
            machine.insert_tape(deck);
            for _ in 0..200 {
                let _ = machine.run_frame();
            }
            machine.type_load_quotes(false);
            machine.set_tape_playing(true);
            let mut ok = false;
            for _ in 0..max {
                let _ = machine.run_frame();
                if done(&machine) {
                    ok = true;
                    break;
                }
            }
            report.push_str(&format!(
                "  {label} {tag}: {} (PC={:04X} block={:?})\n",
                if ok { "PASS" } else { "FAIL" },
                machine.cpu().regs.pc,
                machine.tape_block()
            ));
            if !ok {
                failed.push(format!("{label}/{tag}"));
            }
        }
    }
    eprintln!("{report}");
    assert!(failed.is_empty(), "boggit failures: {}", failed.join(", "));
}
