use super::*;

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
