use super::*;

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

pub(super) fn attr_mark_code_ok(m: &Machine) -> bool {
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
