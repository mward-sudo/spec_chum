//! Opt-in headless boot checks against user-installed official Next assets.

use std::path::PathBuf;

use bus::{NextBus, NEXT_ROM_SIZE};

use crate::NextMachine;

const NEXT_ROM_RESET_ENTRY: [u8; 4] = [0xf3, 0xc3, 0xef, 0x00];
const NEXT_PLUS3_FRAME_TSTATES: u64 = 228 * 311;

fn displayed_ula_screen(bus: &NextBus) -> Vec<u8> {
    let first_page = bus.display_screen_bank() * 2;
    (0..6912)
        .map(|offset| {
            bus.read_ram_page(first_page + (offset / 8192) as u8, offset % 8192)
                .expect("ULA screen is in installed physical RAM")
        })
        .collect()
}

fn displayed_ula_text(bus: &NextBus, rom: &[u8]) -> String {
    let screen = displayed_ula_screen(bus);
    let mut text = String::with_capacity(24 * 33);
    for row in 0..24usize {
        for col in 0..32usize {
            let attr = screen[0x1800 + row * 32 + col];
            if attr & 0x07 == (attr >> 3) & 0x07 {
                text.push(' ');
                continue;
            }
            let mut glyph = [0; 8];
            for scan in 0..8usize {
                let address = (row / 8) * 2048 + (row % 8) * 32 + scan * 256 + col;
                glyph[scan] = screen[address];
            }
            if glyph.iter().all(|&byte| byte == 0) {
                text.push(' ');
                continue;
            }
            let character = (32u8..=127)
                .find(|&code| {
                    let offset = 0xfd00 + usize::from(code - 32) * 8;
                    let font = &rom[offset..offset + 8];
                    glyph == font
                        || glyph
                            .iter()
                            .zip(font)
                            .all(|(&pixel, &source)| pixel == !source)
                })
                .map_or('?', char::from);
            text.push(character);
        }
        text.push('\n');
    }
    text
}

fn reset_entry_matches_rom(machine: &NextMachine, rom: &[u8]) -> bool {
    rom.starts_with(&NEXT_ROM_RESET_ENTRY)
        && NEXT_ROM_RESET_ENTRY
            .iter()
            .enumerate()
            .all(|(address, expected)| machine.bus.read(address as u16) == *expected)
}

fn machine_at_ipl(card_image: &std::path::Path, ipl: &[u8]) -> NextMachine {
    let sentinel_rom = vec![0xff; NEXT_ROM_SIZE];
    let mut machine = NextMachine::new(&sentinel_rom).expect("construct Next with sentinel ROM");
    machine.install_ipl(ipl).expect("install pinned Next IPL");
    machine
        .attach_sd_image(card_image)
        .expect("attach full official FAT16 card image");
    machine
}

fn assets_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/system-next/24.11")
}

#[test]
fn next_official_full_card_executes_sd_loaded_rom_reset_entry() {
    let Some(card_image) = std::env::var_os("SPEC_CHUM_NEXT_BOOT_CARD") else {
        eprintln!("skip: run ./scripts/run_next_boot_test.py for the full-card Next boot check");
        return;
    };
    let control_card = std::env::var_os("SPEC_CHUM_NEXT_BOOT_CONTROL_CARD")
        .expect("full-card harness supplies an independent control card image");
    let Some(main_rom) = std::env::var_os("SPEC_CHUM_NEXT_BOOT_ROM") else {
        eprintln!("skip: SPEC_CHUM_NEXT_BOOT_ROM is not set");
        return;
    };
    let Some(divmmc_rom) = std::env::var_os("SPEC_CHUM_NEXT_BOOT_DIVMMC_ROM") else {
        eprintln!("skip: SPEC_CHUM_NEXT_BOOT_DIVMMC_ROM is not set");
        return;
    };

    let assets = assets_dir();
    let rom = std::fs::read(main_rom).expect("read extracted enNextZX.rom");
    let divmmc_rom = std::fs::read(divmmc_rom).expect("read extracted enNxtmmc.rom");
    assert_eq!(divmmc_rom.len(), 8192, "pinned DivMMC ROM is 8 KiB");
    let ipl = std::fs::read(assets.join("boot-30204.bin")).expect("read pinned Next IPL");
    let mut machine = machine_at_ipl(&PathBuf::from(card_image), &ipl);

    let mut previous_t = machine.cpu.t;
    let mut soft_reset_seen = false;
    for _ in 0..100_000_000 {
        machine.step_once();
        let reset = machine.cpu.t < previous_t;
        previous_t = machine.cpu.t;
        if !reset || machine.bus.read_nextreg(0x02) & 0x03 != 0x01 {
            continue;
        }
        soft_reset_seen = true;
        assert_eq!(machine.cpu.regs.pc, 0, "soft reset must start at $0000");
        assert!(!machine.bus.is_boot_rom_enabled(), "IPL must be disabled");
        assert_ne!(
            machine.bus.read_nextreg(0x03),
            0,
            "configuration mode must be left"
        );
        assert!(
            reset_entry_matches_rom(&machine, &rom),
            "soft reset did not expose the SD-loaded main ROM reset entry"
        );
        machine.step_once();
        assert_eq!(machine.cpu.regs.pc, 1, "main ROM DI at $0000 must execute");
        for address in 1..=4 {
            assert_eq!(
                machine.bus.read(address),
                divmmc_rom[usize::from(address)],
                "SD-loaded DivMMC ROM must map after the reset-entry M1 at ${address:04X}"
            );
        }
        break;
    }
    assert!(
        soft_reset_seen,
        "firmware did not request a soft reset after first-run configuration"
    );

    let mut previous_frame = machine.video_t() / NEXT_PLUS3_FRAME_TSTATES;
    let mut welcome_frames = 0;
    let mut screen_text = String::new();
    for _ in 0..120_000_000 {
        machine.step_once();
        let frame = machine.video_t() / NEXT_PLUS3_FRAME_TSTATES;
        if frame == previous_frame {
            continue;
        }
        previous_frame = frame;
        screen_text = displayed_ula_text(&machine.bus, &rom);
        if screen_text.contains("Welcome to NextZXOS") {
            welcome_frames += 1;
            if welcome_frames >= 3 {
                break;
            }
        } else {
            welcome_frames = 0;
        }
    }
    assert!(
        welcome_frames >= 3,
        "NextZXOS welcome did not stabilize (pc=${:04X}, screen=\n{})",
        machine.cpu.regs.pc,
        screen_text
    );

    let mut without_key = machine_at_ipl(&PathBuf::from(control_card), &ipl);
    while without_key.video_t() < machine.video_t() {
        without_key.step_once();
    }
    assert!(
        displayed_ula_text(&without_key.bus, &rom).contains("Welcome to NextZXOS"),
        "independent control card must reach the same welcome screen"
    );
    machine.bus.keyboard.set_key(7, 0, true);
    assert_eq!(
        machine.bus.in_port(0x7ffe),
        0xbe,
        "Space must be visible in the active-low Spectrum keyboard matrix"
    );
    let until_t = machine
        .video_t()
        .saturating_add(100 * NEXT_PLUS3_FRAME_TSTATES);
    while machine.video_t() < until_t || without_key.video_t() < until_t {
        if machine.video_t() < until_t {
            machine.step_once();
        }
        if without_key.video_t() < until_t {
            without_key.step_once();
        }
    }
    assert!(
        machine.video_t().abs_diff(without_key.video_t()) < 32,
        "Space and control runs should finish at nearly the same simulated time"
    );
    assert_ne!(
        machine.cpu.regs.pc, without_key.cpu.regs.pc,
        "NextZXOS did not respond to Space within 100 frames"
    );
    assert!(
        !displayed_ula_text(&machine.bus, &rom).contains("Welcome to NextZXOS"),
        "NextZXOS welcome marker should clear after Space starts the system"
    );
    assert!(
        displayed_ula_text(&without_key.bus, &rom).contains("Welcome to NextZXOS"),
        "unpressed control run should remain at the same welcome screen"
    );
    machine.bus.keyboard.set_key(7, 0, false);
}

#[test]
fn next_official_firmware_reports_a_missing_card_file() {
    let Some(card_image) = std::env::var_os("SPEC_CHUM_NEXT_BOOT_ERROR_CARD") else {
        eprintln!("skip: run ./scripts/run_next_boot_test.py for the missing-file check");
        return;
    };
    let main_rom = std::env::var_os("SPEC_CHUM_NEXT_BOOT_ROM")
        .expect("full-card harness supplies the extracted system ROM");
    let rom = std::fs::read(main_rom).expect("read extracted enNextZX.rom");
    let assets = assets_dir();
    let ipl = std::fs::read(assets.join("boot-30204.bin")).expect("read pinned Next IPL");
    let mut machine = machine_at_ipl(&PathBuf::from(card_image), &ipl);

    let mut previous_frame = u64::MAX;
    let mut screen_text = String::new();
    for _ in 0..100_000_000 {
        machine.step_once();
        let frame = machine.video_t() / NEXT_PLUS3_FRAME_TSTATES;
        if frame == previous_frame {
            continue;
        }
        previous_frame = frame;
        screen_text = displayed_ula_text(&machine.bus, &rom);
        if screen_text.contains("Error opening 'menu.ini/.def'!") {
            break;
        }
    }
    assert!(
        screen_text.contains("Error opening 'menu.ini/.def'!"),
        "missing menu.def should produce a readable firmware error (pc=${:04X}, screen=\n{})",
        machine.cpu.regs.pc,
        screen_text
    );
}

#[test]
fn next_boot_reports_missing_or_corrupt_inputs() {
    assert!(
        NextMachine::new(&[]).is_err(),
        "wrong-sized system ROM must fail"
    );

    let mut machine = NextMachine::new(&vec![0; bus::NEXT_ROM_SIZE]).expect("valid system ROM");
    let missing = std::env::temp_dir().join(format!(
        "spec-chum-next-card-missing-{}.raw",
        std::process::id()
    ));
    let missing_error = machine
        .attach_sd_image(&missing)
        .expect_err("missing card image must fail");
    assert!(
        matches!(&missing_error, bus::NextSdError::OpenImage { .. }),
        "missing card image should use the path-aware open error"
    );
    assert!(missing_error
        .to_string()
        .contains(&missing.display().to_string()));

    let corrupt = std::env::temp_dir().join(format!(
        "spec-chum-next-card-invalid-{}.raw",
        std::process::id()
    ));
    std::fs::write(&corrupt, [0x5a]).expect("write invalid one-byte card image");
    let result = machine.attach_sd_image(&corrupt);
    std::fs::remove_file(&corrupt).expect("remove invalid card image");
    let corrupt_error = result.expect_err("mis-sized card image must fail");
    assert!(matches!(
        &corrupt_error,
        bus::NextSdError::InvalidLength { length: 1, .. }
    ));
    assert!(corrupt_error
        .to_string()
        .contains(&corrupt.display().to_string()));
    assert!(corrupt_error.to_string().contains("got 1"));
}
