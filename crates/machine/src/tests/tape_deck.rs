use super::*;

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
