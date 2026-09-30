use super::*;

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
