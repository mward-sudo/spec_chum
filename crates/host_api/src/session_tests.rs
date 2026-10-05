use super::*;
use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn rom48() -> Option<Vec<u8>> {
    let path = workspace_root().join("roms/spec48.rom");
    std::fs::read(path).ok()
}

#[test]
#[ignore = "requires user-installed official System/Next 24.11 assets"]
fn verified_next_assets_select_and_render_through_host() {
    let assets = crate::next_assets::NextAssets::discover().expect("verified Next asset set");
    assert!(crate::next_assets::NextAssets::available_in(
        &assets.directory
    ));
    assert!(crate::rom_setup::model_rom_available(
        ModelId::SpectrumNext,
        &crate::rom_setup::model_rom_paths_snapshot()
    ));
    let next_descriptor = crate::host_model_catalog()
        .into_iter()
        .find(|entry| entry.id == ModelId::SpectrumNext.numeric_id())
        .expect("Next in shared model catalog");
    assert!(
        next_descriptor.available,
        "verified Next should be selectable"
    );
    let mut session = HostSession::new(ModelId::SpectrumNext, true);
    session
        .select_model(ModelId::SpectrumNext)
        .expect("verified Next boot");
    assert!(session.has_machine());
    assert_eq!(session.model(), ModelId::SpectrumNext);
    let first_visible_frame = (0..1_500).find(|_| {
        session.run_frame();
        session
            .framebuffer()
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[..3].iter().any(|channel| *channel != 0))
    });
    assert_eq!(
        session.framebuffer().len(),
        session.width() * session.height() * 4
    );
    assert!(
        first_visible_frame.is_some(),
        "Next boot produced no visible pixels in 1,500 frames"
    );
    session.set_key(3, 0, true).expect("Next key down");
    session.set_key(3, 0, false).expect("Next key up");
    session.reset().expect("Next reset");
    assert!(session.has_machine());
}

#[test]
fn new_session_has_empty_framebuffer_dims() {
    let s = HostSession::new(ModelId::Spectrum48, true);
    assert_eq!(s.width(), 352);
    assert_eq!(s.height(), 296);
    assert_eq!(s.framebuffer().len(), 352 * 296 * 4);
    assert!(!s.has_machine());
}

#[test]
fn load_divmmc_sd_slot_attaches_both_images() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let mut s = HostSession::new(ModelId::Spectrum48, true);
    s.load_rom_bytes(&rom).expect("rom");
    let dir = std::env::temp_dir().join("spec_chum_host_divmmc_sd_slots");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let slot0 = dir.join("slot0.img");
    let slot1 = dir.join("slot1.img");
    std::fs::write(&slot0, vec![0x10u8; 512]).expect("slot0");
    std::fs::write(&slot1, vec![0x11u8; 512]).expect("slot1");
    s.load_divmmc_sd(&slot0).expect("slot 0 via legacy");
    s.load_divmmc_sd_slot(&slot1, 1).expect("slot 1");
    assert!(s.has_divmmc());
    let div = s.machine_mut().and_then(Machine::divmmc_mut).expect("div");
    assert_eq!(div.sd.first().copied(), Some(0x10));
    assert_eq!(div.sd1.first().copied(), Some(0x11));
    let err = s
        .load_divmmc_sd_slot(&slot0, 2)
        .expect_err("slot 2 invalid");
    assert!(
        err.to_string().contains("slot 2"),
        "unexpected error: {err}"
    );
}

#[test]
fn border_toggle_resizes_framebuffer() {
    let mut s = HostSession::new(ModelId::Spectrum48, true);
    s.set_border(false);
    assert_eq!(s.width(), 256);
    assert_eq!(s.height(), 192);
    assert_eq!(s.framebuffer().len(), 256 * 192 * 4);
}

#[test]
fn load_rom_and_run_frame_writes_pixels() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let mut s = HostSession::new(ModelId::Spectrum48, true);
    s.load_rom_bytes(&rom).expect("rom");
    assert!(s.has_machine());
    // Advance past cold boot snow a bit.
    for _ in 0..50 {
        s.run_frame();
    }
    let nonzero = s.framebuffer().iter().any(|&b| b != 0);
    assert!(nonzero, "expected rendered pixels after boot frames");
}

#[test]
fn tape_play_requires_inserted_tape() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let mut s = HostSession::new(ModelId::Spectrum48, true);
    s.load_rom_bytes(&rom).expect("rom");
    assert!(s.play_tape().is_err());
    assert!(!s.tape_playing());
}

#[test]
fn set_key_out_of_range_errors() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let mut s = HostSession::new(ModelId::Spectrum48, false);
    s.load_rom_bytes(&rom).expect("rom");
    assert!(s.set_key(8, 0, true).is_err());
    assert!(s.set_key(0, 5, true).is_err());
    s.set_key(0, 0, true).expect("caps");
}

#[test]
fn set_key_injects_and_clear_resets_matrix() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let mut s = HostSession::new(ModelId::Spectrum48, false);
    s.load_rom_bytes(&rom).expect("rom");

    // J = row 6 bit 3 (LOAD keyword); matrix bits are active-low.
    s.set_key(6, 3, true).expect("J down");
    {
        let rows = s.machine_mut().expect("machine").keyboard_mut().rows;
        assert_eq!(rows[6] & (1 << 3), 0, "J bit should be pressed (cleared)");
    }

    s.set_key(7, 1, true).expect("Symbol Shift");
    {
        let rows = s.machine_mut().expect("machine").keyboard_mut().rows;
        assert_eq!(rows[7] & (1 << 1), 0, "Sym bit pressed");
    }

    s.clear_keys().expect("clear");
    {
        let rows = s.machine_mut().expect("machine").keyboard_mut().rows;
        assert!(rows.iter().all(|&r| r == 0x1f), "all rows idle after clear");
    }
}

#[test]
fn set_key_holds_across_run_frames() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let mut s = HostSession::new(ModelId::Spectrum48, false);
    s.load_rom_bytes(&rom).expect("rom");

    // J must stay pressed for the whole hold — turbo hosts + flicker would
    // otherwise look like multiple keyword presses to 48K BASIC.
    s.set_key(6, 3, true).expect("J down");
    for i in 0..16 {
        s.run_frame();
        let rows = s.machine_mut().expect("machine").keyboard_mut().rows;
        assert_eq!(
            rows[6] & (1 << 3),
            0,
            "J must remain pressed after frame {i}"
        );
    }
    s.set_key(6, 3, false).expect("J up");
    {
        let rows = s.machine_mut().expect("machine").keyboard_mut().rows;
        assert_ne!(rows[6] & (1 << 3), 0, "J released");
    }
}

#[test]
fn next_host_key_release_clears_the_keyboard_matrix() {
    let rom = vec![0; 0x1_0000];
    let next = machine::NextMachine::new(&rom).expect("valid Next ROM");
    let mut session = HostSession::new(ModelId::SpectrumNext, false);
    session.machine = Some(HostRuntime::Next(next));

    session.set_key(6, 3, true).expect("Next key down");
    let HostRuntime::Next(next) = session.machine.as_ref().expect("Next runtime") else {
        panic!("expected Next runtime");
    };
    assert_eq!(next.bus.keyboard.rows[6] & (1 << 3), 0);

    session.set_key(6, 3, false).expect("Next key up");
    let HostRuntime::Next(next) = session.machine.as_ref().expect("Next runtime") else {
        panic!("expected Next runtime");
    };
    assert_ne!(next.bus.keyboard.rows[6] & (1 << 3), 0);
}

#[test]
fn next_beeper_audio_reaches_host_pcm() {
    let mut next = machine::NextMachine::new(&vec![0; 0x1_0000]).expect("valid Next ROM");
    let program = [0x3e, 0x10, 0xd3, 0xfe, 0xaf, 0xd3, 0xfe, 0xc3, 0x00, 0xc0];
    for (offset, byte) in program.into_iter().enumerate() {
        next.bus.write(0xc000 + offset as u16, byte);
    }
    next.cpu.regs.pc = 0xc000;
    let mut session = HostSession::new(ModelId::SpectrumNext, true);
    session.machine = Some(HostRuntime::Next(next));

    session.run_frame();

    assert_eq!(session.audio_pcm().len(), AUDIO_SAMPLES_PER_FRAME);
    assert!(session.audio_pcm().iter().any(|sample| *sample > 0.0));
    assert!(session.audio_pcm().iter().any(|sample| *sample < 0.0));
}

#[test]
fn kempston_mouse_ports_after_synthetic_deltas() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let mut s = HostSession::new(ModelId::Spectrum48, false);
    s.load_rom_bytes(&rom).expect("rom");
    s.set_mouse_delta(20, -4).unwrap();
    s.set_mouse_buttons(true, true, false).unwrap();
    let mouse = s.machine_mut().unwrap().mouse_mut();
    assert_eq!(mouse.x, 20);
    assert_eq!(mouse.y, 4);
    assert_eq!(mouse.buttons_byte(), 0xfc); // D0+D1 clear
    s.set_mouse_buttons(false, false, false).unwrap();
    assert_eq!(s.machine_mut().unwrap().mouse_mut().buttons_byte(), 0xff);
    s.set_mouse_delta(5, 0).unwrap();
    s.clear_mouse().unwrap();
    let mouse = s.machine_mut().unwrap().mouse_mut();
    assert_eq!(mouse.x, 0);
    assert_eq!(mouse.y, 0);
    assert_eq!(mouse.buttons_byte(), 0xff);
}

#[test]
fn joystick_kempston_mask_reaches_port() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let mut s = HostSession::new(ModelId::Spectrum48, false);
    s.load_rom_bytes(&rom).expect("rom");
    s.set_joystick_mode(JoystickMode::Kempston);
    s.set_joystick(0x11).unwrap();
    assert_eq!(s.machine_mut().unwrap().kempston_mut().read(), 0x11);
    s.clear_joystick().unwrap();
    assert_eq!(s.machine_mut().unwrap().kempston_mut().read(), 0);
}

#[test]
fn physical_num1_survives_sinclair_left_joystick_update() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let mut s = HostSession::new(ModelId::Spectrum48, false);
    s.load_rom_bytes(&rom).expect("rom");
    s.set_joystick_mode(JoystickMode::SinclairLeft);
    // Num1 = row 3 bit 0 (also Sinclair-left left).
    s.set_key(3, 0, true).unwrap();
    s.set_joystick(0).unwrap(); // would clear Sinclair matrix without reapply
    let rows = s.machine_mut().unwrap().keyboard_mut().rows;
    assert_eq!(rows[3] & (1 << 0), 0, "Num1 must stay pressed");
}

#[test]
fn physical_num5_survives_cursor_joystick_update() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let mut s = HostSession::new(ModelId::Spectrum48, false);
    s.load_rom_bytes(&rom).expect("rom");
    s.set_joystick_mode(JoystickMode::Cursor);
    // Num5 = row 3 bit 4 (also Cursor left).
    s.set_key(3, 4, true).unwrap();
    s.set_joystick(0).unwrap();
    let rows = s.machine_mut().unwrap().keyboard_mut().rows;
    assert_eq!(rows[3] & (1 << 4), 0, "Num5 must stay pressed");
}

#[test]
fn kempston_arrow_left_does_not_pollute_matrix() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let mut s = HostSession::new(ModelId::Spectrum48, false);
    s.load_rom_bytes(&rom).expect("rom");
    s.set_joystick_mode(JoystickMode::Kempston);
    s.set_joystick(0x02).unwrap(); // left
    let m = s.machine_mut().unwrap();
    assert!(m.kempston_mut().left);
    let rows = m.keyboard_mut().rows;
    assert_ne!(
        rows[0] & 1,
        0,
        "Caps must not be injected for Kempston arrows"
    );
    assert_ne!(
        rows[3] & (1 << 4),
        0,
        "5 must not be injected for Kempston arrows"
    );
}

#[test]
fn cursor_left_via_joystick_applies_caps_five() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let mut s = HostSession::new(ModelId::Spectrum48, false);
    s.load_rom_bytes(&rom).expect("rom");
    s.set_joystick_mode(JoystickMode::Cursor);
    s.set_joystick(0x02).unwrap(); // left
    let rows = s.machine_mut().unwrap().keyboard_mut().rows;
    assert_eq!(rows[0] & 1, 0, "Caps down for Cursor left");
    assert_eq!(rows[3] & (1 << 4), 0, "5 down for Cursor left");
}

#[test]
fn open_fixture_tap_progress_and_audio_pcm() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let mut s = HostSession::new(ModelId::Spectrum48, true);
    s.load_rom_bytes(&rom).expect("rom");
    let tap = workspace_root().join("tests/fixtures/tape/minimal_code.tap");
    s.open_tape(&tap).expect("tap");
    s.set_tape_load_options(machine::TapeLoadOptions {
        flash_load: false,
        speed: 1,
        ..Default::default()
    })
    .expect("opts");
    let p0 = s.tape_progress().expect("progress");
    assert_eq!(p0.block_index, 0);
    assert_eq!(p0.block_count, 2);
    s.play_tape().expect("play");
    s.run_frame();
    let p1 = s.tape_progress().expect("progress after play");
    assert!(
        p1.pulse_index > p0.pulse_index || p1.fraction() > p0.fraction(),
        "tape progress should advance after a playing frame (before={p0:?} after={p1:?})"
    );
    assert_eq!(s.audio_pcm().len(), AUDIO_SAMPLES_PER_FRAME);
    let (min, max) = s
        .audio_pcm()
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), &x| {
            (lo.min(x), hi.max(x))
        });
    assert!(
        max - min > 0.05,
        "playing tape should produce a non-trivial PCM range, got min={min} max={max}"
    );
}

#[test]
fn open_local_boggit_tzx_as_tap_when_present() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let boggit = PathBuf::from("/Users/michael/Downloads/BoggitThe/The Boggit - Side 1.tzx");
    if !boggit.is_file() {
        eprintln!("skip: local Boggit TZX not present");
        return;
    }
    let mut s = HostSession::new(ModelId::Spectrum48, true);
    s.load_rom_bytes(&rom).expect("rom");
    s.open_tape(&boggit).expect("boggit tzx");
    let p = s.tape_progress().expect("progress");
    assert!(p.block_count >= 2, "expected TAP conversion with blocks");
    assert!(s.status().contains("TAP") || s.status().contains("TZX"));
}

/// Synthetic Speedlock-style Loop Start/End TZX through `open_tape`
/// (`SpecChumMac` `sc_open_tape` / Open+Instant path). Always runs in CI —
/// would have failed before #372 with `unsupported TZX block ID 0x24`.
#[test]
fn open_tape_expands_tzx_loop_blocks() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let dir = tempfile_dir("spec_chum_tzx_loop");
    let path = dir.join("loop_pure_tone.tzx");
    let mut v = Vec::new();
    v.extend_from_slice(b"ZXTape!");
    v.extend_from_slice(&[0x1a, 1, 20]);
    // One standard block (so status / progress look like a real insert),
    // then a loop of pure tone — the Arkanoid failure mode in miniature.
    v.push(0x10);
    v.extend_from_slice(&100u16.to_le_bytes());
    let payload = [0x00u8, b'A', 0x00];
    v.extend_from_slice(&(payload.len() as u16).to_le_bytes());
    v.extend_from_slice(&payload);
    v.push(0x24);
    v.extend_from_slice(&3u16.to_le_bytes());
    v.push(0x12);
    v.extend_from_slice(&1000u16.to_le_bytes());
    v.extend_from_slice(&2u16.to_le_bytes());
    v.push(0x25);
    v.push(0x20);
    v.extend_from_slice(&0u16.to_le_bytes());
    std::fs::write(&path, &v).expect("write synthetic tzx");

    let mut s = HostSession::new(ModelId::Spectrum48, true);
    s.load_rom_bytes(&rom).expect("rom");
    s.open_tape(&path)
        .expect("open_tape must accept TZX Loop Start/End (#372)");
    assert!(s.has_tape(), "deck must show an inserted tape");
    let p = s.tape_progress().expect("progress");
    // 1× standard + 3× pure-tone iterations (+ optional zero pause).
    assert!(
        p.block_count >= 4,
        "loop expansion must schedule repeated blocks, got {}",
        p.block_count
    );
    assert!(
        s.status().contains("TZX"),
        "status should report TZX insert, got {:?}",
        s.status()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Media capability matrix (#374): each supported TZX block family must open
/// through `HostSession::open_tape` (`SpecChumMac` `sc_open_tape`) and leave
/// `has_tape()`. Info/skip markers are paired with a Pure Tone so the deck
/// is non-empty after insert.
#[test]
fn open_tape_tzx_block_matrix_supported_families() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };

    fn header() -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"ZXTape!");
        v.extend_from_slice(&[0x1a, 1, 20]);
        v
    }
    fn with_tone(mut v: Vec<u8>) -> Vec<u8> {
        v.push(0x12);
        v.extend_from_slice(&1000u16.to_le_bytes());
        v.extend_from_slice(&2u16.to_le_bytes());
        v
    }

    type FamilyBuild = fn() -> Vec<u8>;
    let families: &[(&str, FamilyBuild)] = &[
        ("id10_standard", || {
            let mut v = header();
            v.push(0x10);
            v.extend_from_slice(&100u16.to_le_bytes());
            let payload = [0x00u8, b'A', 0x00];
            v.extend_from_slice(&(payload.len() as u16).to_le_bytes());
            v.extend_from_slice(&payload);
            v
        }),
        ("id11_turbo", || {
            let mut v = header();
            v.push(0x11);
            v.extend_from_slice(&800u16.to_le_bytes());
            v.extend_from_slice(&400u16.to_le_bytes());
            v.extend_from_slice(&400u16.to_le_bytes());
            v.extend_from_slice(&300u16.to_le_bytes());
            v.extend_from_slice(&600u16.to_le_bytes());
            v.extend_from_slice(&10u16.to_le_bytes());
            v.push(8);
            v.extend_from_slice(&0u16.to_le_bytes());
            let payload = [0xffu8, 0x00];
            v.extend_from_slice(&(payload.len() as u32).to_le_bytes()[..3]);
            v.extend_from_slice(&payload);
            v
        }),
        ("id12_pure_tone", || with_tone(header())),
        ("id13_pulse_seq", || {
            let mut v = header();
            v.push(0x13);
            v.push(2);
            v.extend_from_slice(&500u16.to_le_bytes());
            v.extend_from_slice(&600u16.to_le_bytes());
            v
        }),
        ("id14_pure_data", || {
            let mut v = header();
            v.push(0x14);
            v.extend_from_slice(&100u16.to_le_bytes());
            v.extend_from_slice(&200u16.to_le_bytes());
            v.push(8);
            v.extend_from_slice(&0u16.to_le_bytes());
            let payload = [0xa5u8];
            v.extend_from_slice(&(payload.len() as u32).to_le_bytes()[..3]);
            v.extend_from_slice(&payload);
            v
        }),
        ("id15_direct", || {
            let mut v = header();
            v.push(0x15);
            v.extend_from_slice(&158u16.to_le_bytes());
            v.extend_from_slice(&0u16.to_le_bytes());
            v.push(8);
            let samples = [0b1010_1010u8];
            v.extend_from_slice(&(samples.len() as u32).to_le_bytes()[..3]);
            v.extend_from_slice(&samples);
            v
        }),
        ("id18_csw_rle", || {
            let mut v = header();
            v.push(0x18);
            let csw = [10u8, 20, 30];
            let body_len = 10 + csw.len();
            v.extend_from_slice(&(body_len as u32).to_le_bytes());
            v.extend_from_slice(&0u16.to_le_bytes());
            v.extend_from_slice(&22_050u32.to_le_bytes()[..3]);
            v.push(0x01);
            v.extend_from_slice(&(csw.len() as u32).to_le_bytes());
            v.extend_from_slice(&csw);
            v
        }),
        ("id19_gdb", || {
            let mut v = header();
            v.push(0x19);
            let mut body = Vec::new();
            body.extend_from_slice(&0u16.to_le_bytes());
            body.extend_from_slice(&0u32.to_le_bytes());
            body.push(0);
            body.push(0);
            body.extend_from_slice(&1u32.to_le_bytes());
            body.push(1);
            body.push(2);
            body.push(0x00);
            body.extend_from_slice(&500u16.to_le_bytes());
            body.push(0x00);
            body.extend_from_slice(&1000u16.to_le_bytes());
            body.push(0x00);
            v.extend_from_slice(&(body.len() as u32).to_le_bytes());
            v.extend_from_slice(&body);
            v
        }),
        ("id20_pause", || {
            let mut v = header();
            v.push(0x20);
            v.extend_from_slice(&10u16.to_le_bytes());
            v
        }),
        ("id21_group_start", || {
            let mut v = header();
            v.push(0x21);
            v.push(3);
            v.extend_from_slice(b"grp");
            with_tone(v)
        }),
        ("id22_group_end", || {
            let mut v = header();
            v.push(0x22);
            with_tone(v)
        }),
        ("id24_loop", || {
            let mut v = header();
            v.push(0x24);
            v.extend_from_slice(&2u16.to_le_bytes());
            v.push(0x12);
            v.extend_from_slice(&1000u16.to_le_bytes());
            v.extend_from_slice(&2u16.to_le_bytes());
            v.push(0x25);
            v
        }),
        ("id23_jump", || {
            let mut v = header();
            v.push(0x23);
            v.extend_from_slice(&2i16.to_le_bytes());
            v.push(0x12);
            v.extend_from_slice(&1000u16.to_le_bytes());
            v.extend_from_slice(&4u16.to_le_bytes());
            with_tone(v)
        }),
        ("id26_call", || {
            let mut v = header();
            // Call +3 → sub; resume tone; Jump past Return; sub; Return.
            v.push(0x26);
            v.extend_from_slice(&1u16.to_le_bytes());
            v.extend_from_slice(&3i16.to_le_bytes());
            v.push(0x12);
            v.extend_from_slice(&1000u16.to_le_bytes());
            v.extend_from_slice(&1u16.to_le_bytes());
            v.push(0x23);
            v.extend_from_slice(&3i16.to_le_bytes());
            v.push(0x12);
            v.extend_from_slice(&1000u16.to_le_bytes());
            v.extend_from_slice(&2u16.to_le_bytes());
            v.push(0x27);
            v
        }),
        ("id28_select", || {
            let mut v = header();
            v.push(0x28);
            let desc = b"A";
            let body_len = 1 + 2 + 1 + desc.len();
            v.extend_from_slice(&(body_len as u16).to_le_bytes());
            v.push(1);
            v.extend_from_slice(&2i16.to_le_bytes());
            v.push(desc.len() as u8);
            v.extend_from_slice(desc);
            v.push(0x12);
            v.extend_from_slice(&1000u16.to_le_bytes());
            v.extend_from_slice(&4u16.to_le_bytes());
            with_tone(v)
        }),
        ("id2a_stop48", || {
            let mut v = header();
            v.push(0x2a);
            v.extend_from_slice(&0u32.to_le_bytes());
            with_tone(v)
        }),
        ("id2b_set_signal", || {
            let mut v = header();
            v.push(0x2b);
            v.extend_from_slice(&1u32.to_le_bytes());
            v.push(1);
            with_tone(v)
        }),
        ("id30_text", || {
            let mut v = header();
            v.push(0x30);
            v.push(2);
            v.extend_from_slice(b"hi");
            with_tone(v)
        }),
        ("id32_archive_info", || {
            let mut v = header();
            v.push(0x32);
            let body = [0x00u8, 1, b'x'];
            v.extend_from_slice(&(body.len() as u16).to_le_bytes());
            v.extend_from_slice(&body);
            with_tone(v)
        }),
        ("id33_hardware_type", || {
            let mut v = header();
            v.push(0x33);
            v.push(1);
            v.extend_from_slice(&[0x00, 0x00, 0x00]);
            with_tone(v)
        }),
        ("id35_custom_info", || {
            let mut v = header();
            v.push(0x35);
            v.extend_from_slice(b"CUSTOMINFOBLOCK!");
            v.extend_from_slice(&0u32.to_le_bytes());
            with_tone(v)
        }),
        ("id5a_glue", || {
            let mut v = header();
            v.push(0x5a);
            v.extend_from_slice(&[0; 9]);
            with_tone(v)
        }),
    ];

    let dir = tempfile_dir("spec_chum_tzx_matrix");
    for (name, build) in families {
        let path = dir.join(format!("{name}.tzx"));
        std::fs::write(&path, build()).expect("write synthetic tzx");
        let mut s = HostSession::new(ModelId::Spectrum48, true);
        s.load_rom_bytes(&rom).expect("rom");
        s.open_tape(&path).unwrap_or_else(|e| {
            panic!("open_tape must accept supported family {name}: {e}");
        });
        assert!(
            s.has_tape(),
            "supported family {name} must leave has_tape() true"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Unsupported TZX IDs must fail open with the ID in the message and leave
/// any previously inserted tape in the deck (#374).
#[test]
fn open_tape_tzx_block_matrix_unsupported_leaves_deck() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let dir = tempfile_dir("spec_chum_tzx_unsup");
    let good = dir.join("good.tzx");
    let mut good_bytes = Vec::new();
    good_bytes.extend_from_slice(b"ZXTape!");
    good_bytes.extend_from_slice(&[0x1a, 1, 20]);
    good_bytes.push(0x12);
    good_bytes.extend_from_slice(&1000u16.to_le_bytes());
    good_bytes.extend_from_slice(&2u16.to_le_bytes());
    std::fs::write(&good, &good_bytes).expect("write good");

    const UNSUPPORTED: &[u8] = &[0x16, 0x17, 0x34, 0x40];
    for &id in UNSUPPORTED {
        let bad = dir.join(format!("bad_{id:02x}.tzx"));
        let mut v = Vec::new();
        v.extend_from_slice(b"ZXTape!");
        v.extend_from_slice(&[0x1a, 1, 20]);
        v.push(id);
        v.extend_from_slice(&[0u8; 16]);
        std::fs::write(&bad, &v).expect("write bad");

        let mut s = HostSession::new(ModelId::Spectrum48, true);
        s.load_rom_bytes(&rom).expect("rom");
        s.open_tape(&good).expect("seed good tape");
        assert!(s.has_tape());
        let err = s
            .open_tape(&bad)
            .expect_err("unsupported TZX must fail open");
        let msg = err.to_string();
        assert!(
            msg.contains(&format!("0x{id:02x}")),
            "open error must name 0x{id:02x}, got {msg}"
        );
        assert!(
            s.has_tape(),
            "failed open must leave prior tape inserted (0x{id:02x})"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

fn tempfile_dir(prefix: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("{prefix}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// Speedlock Loop Start/End TZX — `SpecChumMac` Open path (#372).
#[test]
fn open_local_arkanoid_tzx_when_present() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let arkanoid = PathBuf::from(home).join("Downloads/Arkanoid.tzx");
    if !arkanoid.is_file() {
        eprintln!("skip: ~/Downloads/Arkanoid.tzx not present");
        return;
    }
    let mut s = HostSession::new(ModelId::Spectrum48, true);
    s.load_rom_bytes(&rom).expect("rom");
    s.open_tape(&arkanoid).expect("Arkanoid.tzx open (#372)");
    assert!(s.has_tape());
    let p = s.tape_progress().expect("progress");
    assert!(
        p.block_count >= 4,
        "expected Speedlock TZX pulse blocks, got {}",
        p.block_count
    );
    assert!(s.status().contains("TZX") || s.media_title().is_some());
}

/// Spectrum seconds until the guest sets `IFF1`, stepping instructions.
///
/// Speedlock re-enables interrupts only briefly, so sampling `iff1` at frame
/// boundaries walks straight past it (#390).
fn step_until_iff1(s: &mut HostSession, budget_frames: u64) -> Option<f64> {
    const T_PER_FRAME: u64 = 69_888;
    let m = s.machine_mut().expect("machine");
    let t0 = m.cpu().t;
    while m.cpu().t.saturating_sub(t0) < budget_frames * T_PER_FRAME {
        m.step_once();
        if m.cpu().regs.iff1 {
            let dt = m.cpu().t.saturating_sub(t0);
            return Some(dt as f64 / (T_PER_FRAME * 50) as f64);
        }
    }
    None
}

/// Arkanoid's post-tape Speedlock protection must complete at **1×** (#390).
///
/// Measured: `EI` lands ~20 s of Spectrum time after the deck finishes with
/// no input at all, so the delay never needed turbo. Debug builds run this
/// slower than realtime, hence `--release` and `--ignored`.
#[ignore = "local Arkanoid fixture; run with --release"]
#[test]
fn arkanoid_speedlock_completes_at_realtime_when_present() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let arkanoid = PathBuf::from(home).join("Downloads/Arkanoid.tzx");
    if !arkanoid.is_file() {
        eprintln!("skip: ~/Downloads/Arkanoid.tzx not present");
        return;
    }
    let mut s = HostSession::new(ModelId::Spectrum48, true);
    s.load_rom_bytes(&rom).expect("rom");
    s.open_tape(&arkanoid).expect("open");
    {
        let m = s.machine_mut().expect("machine");
        m.set_tape_load_options(machine::TapeLoadOptions {
            flash_load: false,
            speed: 64,
            experience_load: false,
        });
        m.set_tape_playing(false);
    }
    for _ in 0..200 {
        let _ = s.machine_mut().expect("machine").run_frame();
    }
    {
        let m = s.machine_mut().expect("machine");
        m.type_load_quotes(false);
        m.set_tape_playing(true);
    }
    for _ in 0..30_000u32 {
        let _ = s.machine_mut().expect("machine").run_frame();
        let m = s.machine().expect("machine");
        if m.tape_finished() || !m.tape_playing() {
            break;
        }
    }
    {
        let m = s.machine_mut().expect("machine");
        assert!(m.tape_finished() || !m.tape_playing(), "deck should finish");
        assert_ne!(m.cpu().regs.pc, 0xFD2A, "still in the EAR edge sampler");
        assert_eq!(
            m.read_mem(0xF44E),
            0xDD,
            "Speedlock uses LD E,IXH at $F44E (not CALL)"
        );
        let nz = (0u16..256)
            .map(|i| m.read_mem(0x4000 + i))
            .filter(|&b| b != 0)
            .count();
        assert!(
            nz >= 32,
            "expected loader screen activity at tape end, nonzero={nz}/256"
        );
        // Drop to a true 1×: nothing may multiply frames after tape end.
        m.set_tape_load_options(machine::TapeLoadOptions {
            flash_load: false,
            speed: 1,
            experience_load: false,
        });
        assert_eq!(m.effective_speed_multiplier(), 1);
    }
    // 40 Spectrum seconds is ~2× the measured 20 s protection run.
    let ei_at = step_until_iff1(&mut s, 2_000).expect("protection did not EI within 40s at 1×");
    eprintln!("protection completed after {ei_at:.1}s of Spectrum time at 1×");
    assert_ne!(
        s.machine().expect("machine").cpu().regs.pc,
        0xFD2A,
        "returned to the EAR sampler"
    );
}

/// Instant on Arkanoid must not crawl at the Play speed control (#390).
///
/// Speedlock is a pulse TZX: there is no LD-BYTES trap for Instant to poke,
/// and the old fallback was a realtime EAR load — the slowest option in the
/// UI, from the button labelled Instant.
#[ignore = "local Arkanoid fixture; run with --release"]
#[test]
fn arkanoid_instant_falls_back_to_ear_turbo_when_present() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let arkanoid = PathBuf::from(home).join("Downloads/Arkanoid.tzx");
    if !arkanoid.is_file() {
        eprintln!("skip: ~/Downloads/Arkanoid.tzx not present");
        return;
    }
    let mut s = HostSession::new(ModelId::Spectrum48, true);
    s.load_rom_bytes(&rom).expect("rom");
    s.open_tape(&arkanoid).expect("open");
    {
        let m = s.machine_mut().expect("machine");
        assert!(
            !m.tape_supports_flash_load(),
            "Speedlock TZX has no TAP blocks to flash"
        );
        // Instant with the Play speed left at 1× — the reported case.
        m.set_tape_load_options(machine::TapeLoadOptions {
            flash_load: true,
            speed: 1,
            experience_load: false,
        });
        m.set_tape_playing(false);
    }
    for _ in 0..200 {
        let _ = s.machine_mut().expect("machine").run_frame();
    }
    {
        let m = s.machine_mut().expect("machine");
        m.type_load_quotes(false);
        m.set_tape_playing(true);
        assert_eq!(
            m.effective_speed_multiplier(),
            machine::INSTANT_EAR_FALLBACK_SPEED,
            "Instant must fall back to EAR turbo, not the 1× setting"
        );
    }
    // At 1× this deck needs ~15k host ticks; the fallback should need ~250.
    const TICK_BUDGET: u32 = 2_000;
    let mut ticks = None;
    for i in 0..TICK_BUDGET {
        let _ = s.machine_mut().expect("machine").run_frame();
        let m = s.machine().expect("machine");
        if m.tape_finished() || !m.tape_playing() {
            ticks = Some(i);
            break;
        }
    }
    let ticks = ticks.expect("Instant fallback did not finish the deck in budget");
    eprintln!("Instant EAR fallback finished the deck in {ticks} host ticks");
    {
        let m = s.machine().expect("machine");
        assert_ne!(m.cpu().regs.pc, 0xFD2A, "still in the EAR edge sampler");
        assert_eq!(
            m.effective_speed_multiplier(),
            1,
            "fallback turbo must stop with the deck, like EAR speed"
        );
    }
    // …and the load is real: Speedlock's ~20 s protection still completes,
    // exactly as it does after a plain EAR load.
    let ei_at = step_until_iff1(&mut s, 2_000)
        .expect("Instant EAR fallback did not clear protection within 40s");
    eprintln!("protection completed {ei_at:.1}s after the Instant fallback load");
}

/// Fast smoke: after EAR exhaust, PC must leave `$FD2A` (sampler desync).
/// Full delay→game entry is covered by the slow ignored test below.
#[ignore = "requires local Arkanoid fixture"]
#[test]
fn arkanoid_ear_leaves_sampler_when_present() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let arkanoid = PathBuf::from(home).join("Downloads/Arkanoid.tzx");
    if !arkanoid.is_file() {
        eprintln!("skip: ~/Downloads/Arkanoid.tzx not present");
        return;
    }
    let mut s = HostSession::new(ModelId::Spectrum48, true);
    s.load_rom_bytes(&rom).expect("rom");
    s.open_tape(&arkanoid).expect("open");
    {
        let m = s.machine_mut().expect("machine");
        m.set_tape_load_options(machine::TapeLoadOptions {
            flash_load: false,
            speed: 64,
            experience_load: false,
        });
        m.set_tape_playing(false);
    }
    for _ in 0..200 {
        let m = s.machine_mut().expect("machine");
        let _ = m.run_frame();
    }
    {
        let m = s.machine_mut().expect("machine");
        m.type_load_quotes(false);
        m.set_tape_playing(true);
    }
    let mut finished = false;
    for _ in 0..20_000 {
        {
            let m = s.machine_mut().expect("machine");
            let _ = m.run_frame();
        }
        let m = s.machine().expect("machine");
        if m.tape_finished() || !m.tape_playing() {
            finished = true;
            break;
        }
    }
    assert!(finished, "EAR deck did not finish");
    let m = s.machine().expect("machine");
    let pc = m.cpu().regs.pc;
    let bc = m.cpu().regs.bc();
    let bytes: Vec<u8> = (0u16..16).map(|i| m.read_mem(pc.wrapping_add(i))).collect();
    let screen_nz = (0u16..256)
        .map(|i| m.read_mem(0x4000 + i))
        .filter(|&b| b != 0)
        .count();
    eprintln!(
        "post-tape pc={pc:#06x} bc={bc:#06x} iff1={} screen_nz={screen_nz} bytes={:02x?}",
        u8::from(m.cpu().regs.iff1),
        bytes
    );
    assert_ne!(pc, 0xFD2A, "still in Speedlock edge sampler — EAR desync");
    // Which loaded routine holds PC at tape end depends on where the deck
    // runs out (`$80xx` setup, the `$F3xx`/`$F4xx` protection nest, …); the
    // invariant is that loaded code above `$8000` is running.
    assert!(
        pc >= 0x8000 || m.cpu().regs.iff1,
        "expected loaded code in upper RAM, pc={pc:#06x}"
    );
    assert!(
        screen_nz >= 32,
        "expected loader screen content, nonzero={screen_nz}/256"
    );
    // EAR speed 64 was selected, but the deck is done: one host tick must
    // now advance exactly one Spectrum frame (#390).
    assert_eq!(
        m.effective_speed_multiplier(),
        1,
        "finished deck must report 1× while the loaded program runs"
    );
    let t0 = {
        let m = s.machine().expect("machine");
        m.cpu().t
    };
    {
        let m = s.machine_mut().expect("machine");
        let _ = m.run_frame();
    }
    let m = s.machine().expect("machine");
    let dt = m.cpu().t.saturating_sub(t0);
    eprintln!("post-tape dt={dt}");
    assert!(dt < 150_000, "expected ~1 frame after tape end, dt={dt}");
}

#[test]
fn model_id_roundtrip() {
    assert_eq!(ModelId::from_u32(0), Some(ModelId::Spectrum48));
    assert_eq!(ModelId::from_u32(1), Some(ModelId::Spectrum128));
    assert_eq!(ModelId::from_u32(2), Some(ModelId::SpectrumPlus3));
    assert_eq!(ModelId::from_u32(3), Some(ModelId::SpectrumPlus2A));
    assert_eq!(ModelId::from_u32(4), Some(ModelId::SpectrumPlus2));
    assert_eq!(ModelId::from_u32(5), Some(ModelId::Spectrum16K));
    assert_eq!(ModelId::from_u32(6), Some(ModelId::Pentagon128));
    assert_eq!(ModelId::from_u32(7), Some(ModelId::TimexTC2048));
    assert_eq!(ModelId::from_u32(8), Some(ModelId::TimexTS2068));
    assert_eq!(ModelId::from_u32(9), Some(ModelId::SpectrumPlus3e));
    assert_eq!(ModelId::from_u32(10), Some(ModelId::ScorpionZs256));
    assert_eq!(ModelId::from_u32(11), Some(ModelId::SpectrumNext));
    assert_eq!(ModelId::from_u32(12), None);
    assert_eq!(ModelId::Spectrum48.to_model(), Model::Spectrum48);
    assert_eq!(ModelId::SpectrumPlus2.to_model(), Model::SpectrumPlus2);
    assert_eq!(ModelId::Spectrum16K.to_model(), Model::Spectrum16K);
    assert_eq!(ModelId::SpectrumPlus2A.to_model(), Model::SpectrumPlus2A);
    assert_eq!(ModelId::SpectrumPlus3e.to_model(), Model::SpectrumPlus3e);
    assert_eq!(ModelId::ScorpionZs256.to_model(), Model::ScorpionZs256);
}

#[test]
fn select_model_keeps_session_model_in_sync() {
    let mut s = HostSession::new(ModelId::Spectrum48, true);
    for model in [
        ModelId::Spectrum16K,
        ModelId::Spectrum48,
        ModelId::Spectrum128,
        ModelId::SpectrumPlus2,
        ModelId::SpectrumPlus3,
        ModelId::SpectrumPlus2A,
        ModelId::Spectrum48,
    ] {
        let result = s.select_model(model);
        assert_eq!(
            s.model(),
            model,
            "session.model() must match selection even if ROM load fails"
        );
        match result {
            Ok(()) => {
                assert!(s.has_machine(), "autoload should install a machine");
                assert_eq!(
                    s.machine().expect("machine").model(),
                    model.to_model(),
                    "loaded Machine must match selected ModelId"
                );
            }
            Err(_) => {
                assert!(
                    !s.has_machine(),
                    "failed autoload must leave the machine unloaded"
                );
            }
        }
    }
}

#[test]
fn peek_poke_and_inspect_json() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let mut s = HostSession::new(ModelId::Spectrum48, false);
    s.load_rom_bytes(&rom).expect("rom");
    let pc0 = s.regs().expect("regs").pc;
    s.poke(0xC000, 0xA5).expect("poke");
    assert_eq!(s.peek(0xC000).expect("peek ram"), 0xA5);
    let json = s.inspect_json().expect("json");
    assert!(
        json.contains("\"pc\":"),
        "inspect json should include pc: {json}"
    );
    s.step().expect("step");
    assert_ne!(
        s.regs().expect("regs").pc,
        pc0,
        "step should advance PC from ROM"
    );
    s.add_breakpoint(0x1234).expect("break");
    s.set_paused(true);
    assert!(s.paused());
}

#[test]
fn run_frame_skips_when_debugger_paused() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let mut s = HostSession::new(ModelId::Spectrum48, false);
    s.load_rom_bytes(&rom).expect("rom");
    s.run_frame();
    let t0 = s.machine().expect("machine").cpu().t;
    s.set_paused(true);
    assert!(s.paused());
    s.run_frame();
    let t1 = s.machine().expect("machine").cpu().t;
    assert_eq!(t0, t1, "paused debugger must not advance the machine");
    s.set_paused(false);
    s.run_frame();
    let t2 = s.machine().expect("machine").cpu().t;
    assert!(t2 > t1, "unpaused run_frame should advance T-states");
}

#[test]
fn peek_without_machine_errors() {
    let s = HostSession::new(ModelId::Spectrum48, true);
    assert!(matches!(s.peek(0), Err(HostError::NoMachine)));
    assert!(matches!(s.inspect_json(), Err(HostError::NoMachine)));
    assert!(matches!(s.regs(), Err(HostError::NoMachine)));
}

#[test]
fn patch_regs_sets_pc_sp_af() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let mut s = HostSession::new(ModelId::Spectrum48, false);
    s.load_rom_bytes(&rom).expect("rom");
    let empty = s.patch_regs(RegsPatch::default());
    assert!(matches!(empty, Err(HostError::Message(_))));
    let out = s
        .patch_regs(RegsPatch {
            pc: Some(0x3d00),
            sp: Some(0xff4a),
            af: Some(0xffff),
        })
        .expect("patch");
    assert_eq!(out.pc, 0x3d00);
    assert_eq!(out.sp, 0xff4a);
    assert_eq!(out.af, 0xffff);
    let regs = s.regs().expect("regs");
    assert_eq!(regs.pc, 0x3d00);
    assert_eq!(regs.sp, 0xff4a);
    assert_eq!(regs.af, 0xffff);
}

#[test]
fn open_missing_tape_errors() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let mut s = HostSession::new(ModelId::Spectrum48, true);
    s.load_rom_bytes(&rom).expect("rom");
    let err = s
        .open_tape(Path::new("/tmp/spec_chum_definitely_missing.tap"))
        .expect_err("missing");
    assert!(matches!(err, HostError::Io(_) | HostError::Message(_)));
}

/// Minimal 48K SNA (49179 bytes) with PC=0x8000 via SP pop and one RAM marker.
fn synthetic_sna48_bytes() -> Vec<u8> {
    let mut data = vec![0u8; 49179];
    data[26] = 5; // border
    data[23] = 0x00;
    data[24] = 0x40; // SP = 0x4000 → pop PC from RAM[0x4000]
    data[27] = 0x00;
    data[28] = 0x80; // PC = 0x8000
    data[27 + 0x4000] = 0xaa; // byte at 0x8000
    data
}

fn synthetic_rzx_without_snapshot() -> Vec<u8> {
    b"RZX!\x00\x0d\0\0\0\0".to_vec()
}

/// Minimal uncompressed Z80 v1 (48K).
fn synthetic_z80_v1_bytes() -> Vec<u8> {
    let mut data = vec![0u8; 30 + 49152];
    data[0] = 0x11; // A
    data[1] = 0x22; // F
    data[6] = 0x00;
    data[7] = 0x81; // PC = 0x8100
    data[8] = 0x00;
    data[9] = 0x70; // SP = 0x7000
    data[12] = (6 << 1) & 0x0e; // border 6, uncompressed
    data[30] = 0xbe;
    data[31] = 0xef;
    data[30 + 0x1000] = 0x42;
    data
}

#[test]
fn load_snapshot_sna48_sets_pc_and_ram() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let dir = std::env::temp_dir();
    let path = dir.join("spec_chum_host_api_test.sna");
    std::fs::write(&path, synthetic_sna48_bytes()).expect("write sna");
    let mut s = HostSession::new(ModelId::Spectrum48, false);
    s.load_rom_bytes(&rom).expect("rom");
    s.load_snapshot(&path).expect("sna");
    let regs = s.regs().expect("regs");
    assert_eq!(regs.pc, 0x8000);
    assert_eq!(s.peek(0x8000).expect("peek"), 0xaa);
    assert!(s.status().contains("snapshot"));
    let _ = std::fs::remove_file(&path);
}

fn load_128_or_plus3_rom(s: &mut HostSession, model: ModelId) -> bool {
    s.set_model(model);
    s.try_autoload_rom();
    s.has_machine()
}

#[test]
fn load_snapshot48_switches_from_128k() {
    if rom48().is_none() {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    }
    let mut s = HostSession::new(ModelId::Spectrum128, false);
    if !load_128_or_plus3_rom(&mut s, ModelId::Spectrum128) {
        eprintln!("skip: 128K ROM missing");
        return;
    }
    assert_eq!(s.model(), ModelId::Spectrum128);
    s.set_joystick_mode(JoystickMode::Kempston);
    s.set_joystick(0x11).unwrap();
    let path = std::env::temp_dir().join("spec_chum_host_api_128_to_48.sna");
    std::fs::write(&path, synthetic_sna48_bytes()).expect("write sna");
    s.load_snapshot(&path).expect("48k sna on 128");
    assert_eq!(s.model(), ModelId::Spectrum48);
    assert_eq!(
        s.machine().map(machine::Machine::model),
        Some(Model::Spectrum48)
    );
    assert_eq!(s.regs().expect("regs").pc, 0x8000);
    assert_eq!(
        s.machine_mut().unwrap().kempston_mut().read(),
        0x11,
        "held joystick must survive model-switching snapshot load"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn load_snapshot48_switches_from_plus3() {
    if rom48().is_none() {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    }
    let mut s = HostSession::new(ModelId::SpectrumPlus3, false);
    if !load_128_or_plus3_rom(&mut s, ModelId::SpectrumPlus3) {
        eprintln!("skip: +3 ROM missing");
        return;
    }
    assert_eq!(s.model(), ModelId::SpectrumPlus3);
    let path = std::env::temp_dir().join("spec_chum_host_api_plus3_to_48.sna");
    std::fs::write(&path, synthetic_sna48_bytes()).expect("write sna");
    s.load_snapshot(&path).expect("48k sna on +3");
    assert_eq!(s.model(), ModelId::Spectrum48);
    assert_eq!(
        s.machine().map(machine::Machine::model),
        Some(Model::Spectrum48)
    );
    assert_eq!(s.regs().expect("regs").pc, 0x8000);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn load_snapshot_z80_sets_pc_and_ram() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let dir = std::env::temp_dir();
    let path = dir.join("spec_chum_host_api_test.z80");
    std::fs::write(&path, synthetic_z80_v1_bytes()).expect("write z80");
    let mut s = HostSession::new(ModelId::Spectrum48, false);
    s.load_rom_bytes(&rom).expect("rom");
    s.load_snapshot(&path).expect("z80");
    let regs = s.regs().expect("regs");
    assert_eq!(regs.pc, 0x8100);
    assert_eq!(regs.sp, 0x7000);
    assert_eq!(s.peek(0x4000).expect("peek"), 0xbe);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn load_snapshot_without_machine_autoloads_48k_rom() {
    let fixture = workspace_root().join("tests/fixtures/snapshots/minimal48.sna");
    let (path, cleanup) = if fixture.is_file() {
        (fixture, false)
    } else {
        let path = std::env::temp_dir().join("spec_chum_host_api_autoload.sna");
        std::fs::write(&path, synthetic_sna48_bytes()).expect("write sna");
        (path, true)
    };
    let mut s = HostSession::new(ModelId::Spectrum48, false);
    match s.load_snapshot(&path) {
        Ok(()) => {
            assert!(s.has_machine());
            assert_eq!(s.regs().expect("regs").pc, 0x8000);
        }
        Err(HostError::NoMachine) => {
            eprintln!("skip: could not autoload ROM for snapshot");
        }
        Err(HostError::Message(msg))
            if msg.contains("ROM required") || msg.contains("fetch_roms") =>
        {
            eprintln!("skip: {msg}");
        }
        Err(e) => panic!("unexpected error: {e}"),
    }
    if cleanup {
        let _ = std::fs::remove_file(&path);
    }
}

#[test]
fn load_rzx_and_dsk_require_machine() {
    let mut s = HostSession::new(ModelId::SpectrumPlus3, false);
    let rzx_path = std::env::temp_dir().join("spec_chum_host_api_no_snapshot.rzx");
    std::fs::write(&rzx_path, synthetic_rzx_without_snapshot()).expect("write RZX");
    assert!(matches!(s.load_rzx(&rzx_path), Err(HostError::NoMachine)));
    let _ = std::fs::remove_file(&rzx_path);
    assert!(matches!(
        s.load_dsk(Path::new("/tmp/missing.dsk")),
        Err(HostError::NoMachine)
    ));
    assert!(matches!(
        s.load_trd(Path::new("/tmp/missing.trd")),
        Err(HostError::NoMachine)
    ));
}

#[test]
fn load_rzx_embedded_snapshot_initializes_machine_and_replays_input() {
    if rom48().is_none() {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    }
    let path = std::env::temp_dir().join("spec_chum_host_api_embedded_snapshot.rzx");
    std::fs::write(
        &path,
        include_bytes!("../../../tests/fixtures/rzx/embedded_sna_compressed.rzx"),
    )
    .expect("write RZX fixture");

    let mut session = HostSession::new(ModelId::SpectrumPlus3, false);
    session
        .load_rzx(&path)
        .expect("load embedded snapshot replay");
    assert!(session.has_machine());
    assert_eq!(session.model(), ModelId::Spectrum48);
    assert_eq!(session.regs().expect("regs").pc, 0x8000);
    assert_eq!(session.peek(0x8000).expect("snapshot RAM"), 0xaa);
    session
        .machine_mut()
        .expect("machine initialized from snapshot")
        .run_frame();
    assert_eq!(
        session
            .machine_mut()
            .expect("machine remains loaded")
            .keyboard_mut()
            .rows[1],
        0x01
    );
    assert!(session.status().contains("Loaded RZX"));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn load_rzx_embedded_z80_snapshot_initializes_machine() {
    if rom48().is_none() {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    }
    let path = std::env::temp_dir().join("spec_chum_host_api_embedded_z80.rzx");
    std::fs::write(
        &path,
        include_bytes!("../../../tests/fixtures/rzx/embedded_z80_compressed.rzx"),
    )
    .expect("write RZX fixture");

    let mut session = HostSession::new(ModelId::SpectrumPlus3, false);
    session.load_rzx(&path).expect("load embedded Z80 replay");
    assert_eq!(session.model(), ModelId::Spectrum48);
    assert_eq!(session.regs().expect("regs").pc, 0x8100);
    assert_eq!(session.peek(0x5000).expect("snapshot RAM"), 0x42);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn load_dsk_rejects_non_plus3() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let mut s = HostSession::new(ModelId::Spectrum48, false);
    s.load_rom_bytes(&rom).expect("rom");

    // Minimal parseable MV-CPC DSK so load reaches insert_disk.
    let dsk = formats::DskImage::synthetic_empty_track_bytes();

    let dir = std::env::temp_dir().join("spec_chum_host_api_dsk_reject");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("reject.dsk");
    std::fs::write(&path, &dsk).expect("write dsk");

    let err = s.load_dsk(&path).expect_err("48K must reject DSK");
    match err {
        HostError::Message(msg) => {
            assert!(
                msg.contains("+3") || msg.contains("Plus3") || msg.contains("plus3"),
                "expected model-rejection message, got {msg}"
            );
        }
        other => panic!("expected Message rejection, got {other:?}"),
    }
}

#[test]
fn load_trd_rejects_plus3() {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/plus3/plus3.rom");
    let Ok(rom) = std::fs::read(&p) else {
        eprintln!("skip: roms/plus3/plus3.rom missing");
        return;
    };
    let mut s = HostSession::new(ModelId::SpectrumPlus3, false);
    s.load_rom_bytes(&rom).expect("rom");

    let mut raw = vec![0u8; formats::TRD_SECTOR_SIZE * formats::TRD_SECTORS_PER_TRACK];
    raw[0xe3] = 0;
    let dir = std::env::temp_dir().join("spec_chum_host_api_trd_reject");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("reject.trd");
    std::fs::write(&path, &raw).expect("write trd");

    let err = s.load_trd(&path).expect_err("+3 must reject TRD");
    match err {
        HostError::Message(msg) => {
            assert!(
                msg.contains("Beta") || msg.contains("+2A") || msg.contains("+3"),
                "expected Beta rejection, got {msg}"
            );
        }
        other => panic!("expected Message rejection, got {other:?}"),
    }
}

#[test]
fn attach_beta_on_48k_and_reject_plus3() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let mut s = HostSession::new(ModelId::Spectrum48, false);
    s.load_rom_bytes(&rom).expect("rom");
    assert!(!s.has_beta());
    s.attach_beta().expect("attach");
    assert!(s.has_beta());
    assert!(s.status().contains("Beta"));

    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/plus3/plus3.rom");
    let Ok(rom) = std::fs::read(&p) else {
        eprintln!("skip: roms/plus3/plus3.rom missing");
        return;
    };
    let mut plus3 = HostSession::new(ModelId::SpectrumPlus3, false);
    plus3.load_rom_bytes(&rom).expect("rom");
    let err = plus3.attach_beta().expect_err("+3 must reject Beta");
    match err {
        HostError::Message(msg) => {
            assert!(
                msg.contains("Beta") || msg.contains("+2A") || msg.contains("+3"),
                "expected Beta rejection, got {msg}"
            );
        }
        other => panic!("expected Message rejection, got {other:?}"),
    }
}

#[test]
fn open_tape_resolves_catalogue_media_title() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let mut s = HostSession::new(ModelId::Spectrum48, false);
    s.load_rom_bytes(&rom).expect("rom");
    let tap =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/tape/print_ok.tap");
    s.open_tape(&tap).expect("open tape");
    assert_eq!(s.media_title(), Some("PRINT \"OK\""));
    assert_eq!(s.media_sha512().map(str::len), Some(128));
    s.eject_tape().expect("eject");
    assert_eq!(s.media_title(), None);
    assert_eq!(s.media_sha512(), None);
}

#[test]
fn pending_online_title_applies_when_generation_matches() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let mut s = HostSession::new(ModelId::Spectrum48, true);
    s.load_rom_bytes(&rom).expect("rom");
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/tape/print_ok.tap");
    s.open_tape(&fixture).expect("open");
    // Simulate filename fallback + a completed background ZXInfo hit (#373).
    let sha = s.media_sha512().expect("sha").to_string();
    s.media_title = Some("print_ok.tap".into());
    s.media_title_source = Some(MediaTitleSource::Filename);
    let gen = s.media_title_generation;
    *s.pending_media_title.lock() = Some(PendingMediaTitle {
        generation: gen,
        sha512_hex: sha,
        title: "Online Title".into(),
        source: MediaTitleSource::Online,
    });
    assert_eq!(s.media_title(), Some("Online Title"));
    assert_eq!(s.media_title_source(), Some(MediaTitleSource::Online));
}

#[test]
fn media_title_clears_when_machine_replaced_without_tape() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let mut s = HostSession::new(ModelId::Spectrum48, false);
    s.load_rom_bytes(&rom).expect("rom");
    let tap =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/tape/print_ok.tap");
    s.open_tape(&tap).expect("open tape");
    assert!(s.media_title().is_some());
    s.clear_machine();
    assert_eq!(s.media_title(), None);
    assert_eq!(s.media_sha512(), None);
}

#[test]
fn render_frame_pcm_carries_late_edge_into_next_level() {
    let frame_tstates = 69_888u32;
    let audio = machine::FrameAudio {
        beeper_edges: vec![(frame_tstates - 1, true)],
        ay_samples: Vec::new(),
        ay_left: Vec::new(),
        ay_right: Vec::new(),
    };
    let mut out = Vec::new();
    let level = render_frame_pcm(&audio, frame_tstates, false, &mut out);
    assert!(
        level,
        "edge near end of frame must update returned speaker level"
    );
    assert_eq!(out.len(), AUDIO_SAMPLES_PER_FRAME);
}
