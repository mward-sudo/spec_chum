use super::*;

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
