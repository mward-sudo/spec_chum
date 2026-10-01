use super::*;

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
    assert_ne!(audio.ay_samples.len(), 0);
    let energy: f32 = audio.ay_samples.iter().map(|s| s * s).sum();
    assert!(
        energy > 0.01,
        "AY tone should produce frame audio energy, got {energy}"
    );
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
