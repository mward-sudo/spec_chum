use super::*;

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
