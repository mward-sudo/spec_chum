use super::*;

/// Uncompressed RZX input block (same layout as `formats::rzx` tests).
fn minimal_rzx(frames: &[(u16, &[u8])]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(b"RZX!");
    v.extend_from_slice(&[0x00, 0x0d]);
    v.extend_from_slice(&[0, 0, 0, 0]);
    let mut body = Vec::new();
    body.extend_from_slice(&(frames.len() as u32).to_le_bytes());
    body.push(0); // reserved
    body.extend_from_slice(&0u32.to_le_bytes()); // T-states at block start
    body.extend_from_slice(&0u32.to_le_bytes()); // uncompressed input data
    for &(fetch, inputs) in frames {
        body.extend_from_slice(&fetch.to_le_bytes());
        body.extend_from_slice(&(inputs.len() as u16).to_le_bytes());
        body.extend_from_slice(inputs);
    }
    let block_len = (5 + body.len()) as u32;
    v.push(0x80);
    v.extend_from_slice(&block_len.to_le_bytes());
    v.extend_from_slice(&body);
    v
}

#[test]
fn rzx_replay_applies_keyboard_and_kempston() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    // Frame 0: row 1 keys=0x01 (byte 0x21). Frame 1: Kempston 0x15 (byte 0x95).
    // Frame 2: row 0 keys=0x10 (byte 0x10).
    let data = minimal_rzx(&[(100, &[0x21]), (100, &[0x95]), (100, &[0x10])]);
    let rec = RzxRecording::parse(&data).expect("rzx");
    let mut m = Machine::new_48k(&rom).unwrap();
    m.insert_rzx(rec);

    m.run_frame();
    assert_eq!(m.keyboard_mut().rows[1], 0x01);

    m.run_frame();
    assert_eq!(m.kempston_mut().read(), 0x15);
    if let Machine::Spec48 { bus, .. } = &mut m {
        assert_eq!(bus.in_port(0x001f), 0x15);
    }

    m.run_frame();
    assert_eq!(m.keyboard_mut().rows[0], 0x10);
}

pub(super) fn minimal_tzx_turbo_machine(payload: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(b"ZXTape!");
    v.extend_from_slice(&[0x1a, 1, 20]);
    v.push(0x11);
    v.extend_from_slice(&800u16.to_le_bytes());
    v.extend_from_slice(&400u16.to_le_bytes());
    v.extend_from_slice(&400u16.to_le_bytes());
    v.extend_from_slice(&300u16.to_le_bytes());
    v.extend_from_slice(&600u16.to_le_bytes());
    v.extend_from_slice(&20u16.to_le_bytes());
    v.push(8);
    v.extend_from_slice(&50u16.to_le_bytes());
    let len = payload.len() as u32;
    v.push((len & 0xff) as u8);
    v.push(((len >> 8) & 0xff) as u8);
    v.push(((len >> 16) & 0xff) as u8);
    v.extend_from_slice(payload);
    v
}

#[test]
fn turbo_tzx_ear_advances_without_flash_path() {
    let Some(rom) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let data = minimal_tzx_turbo_machine(&[0xff, 0x00, 0xaa]);
    assert!(
        !tape::TzxPlayer::is_standard_speed_only(&data),
        "turbo must not be treated as standard-speed TAP"
    );
    let tap = tape::TzxPlayer::to_tap_image(&data).expect("to_tap");
    assert!(
        tap.blocks.is_empty(),
        "flash/TAP extraction must skip ID 0x11"
    );

    let player = TzxPlayer::parse(&data).expect("tzx");
    assert!(player.scheduled_pulses() > 20);

    let mut m = Machine::new_48k(&rom).unwrap();
    m.set_tape_load_options(TapeLoadOptions {
        flash_load: false,
        speed: 1,
        ..Default::default()
    });
    m.insert_tzx(player);
    m.set_tape_playing(true);
    assert!(!m.ear(), "EAR idle before pulse advance");

    let mut saw_edges = false;
    let mut saw_progress = false;
    let mut last_pulse = 0u32;
    for _ in 0..8 {
        let audio = m.run_frame();
        if !audio.beeper_edges.is_empty() {
            saw_edges = true;
        }
        if let Some(p) = m.tape_progress() {
            if p.pulse_index > last_pulse {
                saw_progress = true;
                last_pulse = p.pulse_index;
            }
        }
    }
    // Tiny turbo decks can finish within a frame; idle EAR is forced low on
    // exhaust (#178), so end-of-frame `ear()` is not a reliable high sample.
    assert!(saw_edges, "turbo pilot must emit EAR/beeper edges");
    assert!(saw_progress, "pulse index must advance under EAR path");
    assert!(
        m.tape_progress().map_or(0, |p| p.pulse_count) > 0,
        "turbo deck reports scheduled pulses"
    );
}
