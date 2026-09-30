use super::*;

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

fn assert_beam_attribute_split(out: &[u8], model: &str) {
    let pixel = |py: usize| {
        let i = py * 256 * 4;
        [out[i], out[i + 1], out[i + 2]]
    };
    let old_ink = ula::palette_rgb(7, false);
    let new_ink = ula::palette_rgb(2, false);
    assert_eq!(
        pixel(0),
        old_ink,
        "{model}: earlier row keeps its old attribute"
    );
    assert_eq!(
        pixel(4),
        old_ink,
        "{model}: fetched row keeps its old attribute"
    );
    assert_eq!(
        pixel(5),
        new_ink,
        "{model}: later row uses the new attribute"
    );
    assert_eq!(
        pixel(7),
        new_ink,
        "{model}: later row uses the new attribute"
    );
}

fn fill_attribute_split_screen(screen: &mut [u8]) {
    screen[6144..6912].fill(7);
    for py in 0..8 {
        screen[py * 256] = 0xff;
    }
}

#[test]
fn screen_write_uses_memory_access_time_on_48k() {
    let mut bus = Bus48::new();
    bus.ula.begin_frame();
    fill_attribute_split_screen(&mut bus.ram);
    let write_t = ula::PAPER_START_48 + 4 * ula::T_LINE_48 + 5;
    {
        let mut mem = MemIo48 {
            bus: &mut bus,
            watch: None,
            t_step_start: 0,
            opcode_pc: None,
        };
        mem.write(0x5800, 2, u64::from(write_t));
    }
    assert_eq!(
        bus.frame_t, 0,
        "temporary access time must not advance the bus"
    );

    let mut out = vec![0; 256 * 192 * 4];
    bus.ula.render_rgba(bus.screen_bytes(), &mut out, false);
    assert_beam_attribute_split(&out, "48K");
}

#[test]
fn screen_write_history_tracks_bank_7_on_128k() {
    let mut bus = Bus128::new();
    bus.ula.begin_frame();
    fill_attribute_split_screen(&mut bus.banks[7]);
    bus.out_7ffd(0x0f); // bank 7 displayed and mapped at C000
    let write_t = ula::PAPER_START_128 + 4 * ula::T_LINE_128 + 5;
    {
        let mut mem = MemIo128 {
            bus: &mut bus,
            watch: None,
            t_step_start: 0,
            opcode_pc: None,
            pentagon: false,
        };
        mem.write(0xd800, 2, u64::from(write_t));
    }
    assert_eq!(
        bus.frame_t, 0,
        "temporary access time must not advance the bus"
    );

    let mut out = vec![0; 256 * 192 * 4];
    bus.ula.render_rgba_timed_dual(
        &bus.banks[5][..6912],
        &bus.banks[7][..6912],
        &mut out,
        false,
        ula::PAPER_START_128,
        ula::T_LINE_128,
    );
    assert_beam_attribute_split(&out, "128K bank 7");
}

#[test]
fn screen_write_history_tracks_bank_7_on_plus3() {
    let mut bus = BusPlus3::new();
    bus.ula.begin_frame();
    fill_attribute_split_screen(&mut bus.banks[7]);
    bus.out_7ffd(0x0f); // bank 7 displayed and mapped at C000
    let write_t = ula::PAPER_START_128 + 4 * ula::T_LINE_128 + 5;
    {
        let mut mem = MemIoPlus3 {
            bus: &mut bus,
            watch: None,
            t_step_start: 0,
        };
        mem.write(0xd800, 2, u64::from(write_t));
    }
    assert_eq!(
        bus.frame_t, 0,
        "temporary access time must not advance the bus"
    );

    let mut out = vec![0; 256 * 192 * 4];
    bus.ula.render_rgba_timed_dual(
        &bus.banks[5][..6912],
        &bus.banks[7][..6912],
        &mut out,
        false,
        ula::PAPER_START_128,
        ula::T_LINE_128,
    );
    assert_beam_attribute_split(&out, "+3 bank 7");
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
