use super::*;

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
