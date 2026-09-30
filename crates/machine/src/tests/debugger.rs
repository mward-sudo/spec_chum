use super::*;

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
