//! Opt-in third-party Spectrum system tests (`--features system-tests`).
//!
//! TAP fixtures are fetched into `.rom-cache/system-tests/` (gitignored).
//! See `tests/fixtures/system/README.md`. Not run by default CI.

use std::path::{Path, PathBuf};

use tape::{TapImage, TapPlayer};

use crate::{Machine, Model, TapeLoadOptions};

const FONT_BASE: u16 = 0x3D00;
const AFTER_LOAD_FRAMES: u32 = 2_500;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.rom-cache/system-tests")
}

fn rom48_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/spec48.rom")
}

fn rom128_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/128/spec128uk.rom")
}

fn rom_plus3() -> Option<Vec<u8>> {
    // Do not fall back to +2A: +3 boot tests must exercise the Sinclair +3 ROM path.
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/plus3/plus3.rom");
    std::fs::read(path).ok()
}

fn tap_path(name: &str) -> Option<PathBuf> {
    let path = fixture_dir().join(name);
    if path.exists() {
        Some(path)
    } else {
        eprintln!(
            "skip: {} missing — run ./scripts/fetch_system_tests.sh",
            path.display()
        );
        None
    }
}

fn bitmap_addr(col: u16, row: u16, scan: u16) -> u16 {
    0x4000 + (row / 8) * 2048 + (row % 8) * 32 + scan * 256 + col
}

fn glyph_at(machine: &Machine, col: u8, row: u8) -> [u8; 8] {
    let mut g = [0u8; 8];
    for scan in 0..8u16 {
        g[scan as usize] = machine.read_mem(bitmap_addr(u16::from(col), u16::from(row), scan));
    }
    g
}

fn decode_cell(machine: &Machine, col: u8, row: u8) -> char {
    let glyph = glyph_at(machine, col, row);
    if glyph.iter().all(|&b| b == 0) {
        return ' ';
    }
    for code in 32u8..=127 {
        let mut normal = true;
        let mut inverse = true;
        for scan in 0..8u16 {
            let font = machine.read_mem(FONT_BASE + u16::from(code - 32) * 8 + scan);
            if glyph[scan as usize] != font {
                normal = false;
            }
            if glyph[scan as usize] != !font {
                inverse = false;
            }
        }
        if normal || inverse {
            return if code == 127 { '©' } else { char::from(code) };
        }
    }
    '?'
}

fn screen_text(machine: &Machine) -> String {
    let mut out = String::with_capacity(24 * 33);
    for row in 0..24u8 {
        for col in 0..32u8 {
            out.push(decode_cell(machine, col, row));
        }
        out.push('\n');
    }
    out
}

fn screen_nonzero(machine: &Machine) -> usize {
    (0x4000..0x5b00)
        .filter(|&a| machine.read_mem(a) != 0)
        .count()
}

fn hold_keys(machine: &mut Machine, keys: &[(usize, u8)], frames: u32) {
    for _ in 0..frames {
        machine.keyboard_mut().reset();
        for &(row, bit) in keys {
            machine.keyboard_mut().set_key(row, bit, true);
        }
        let _ = machine.run_frame();
    }
}

fn wait_48_basic_prompt(machine: &mut Machine, max_frames: u32) {
    let mut stable = 0u32;
    for _ in 0..max_frames {
        let pc = machine.cpu().regs.pc;
        if (0x12A0..=0x1600).contains(&pc) {
            stable += 1;
            if stable >= 20 {
                return;
            }
        } else {
            stable = 0;
        }
        let _ = machine.run_frame();
    }
}

fn load_program_tap(machine: &mut Machine, tap: &Path) {
    let image = TapImage::load(tap).unwrap_or_else(|e| panic!("TAP {}: {e}", tap.display()));
    machine.set_tape_load_options(TapeLoadOptions {
        flash_load: true,
        speed: 1,
    });
    machine.insert_tape(TapPlayer::new(image));
    match machine.model() {
        Model::Spectrum48 => wait_48_basic_prompt(machine, 500),
        Model::Spectrum128 | Model::SpectrumPlus3 => {
            for _ in 0..200 {
                let _ = machine.run_frame();
            }
        }
    }
    machine.type_load_quotes(false);
    machine.set_tape_playing(true);
}

fn run_until_contains(machine: &mut Machine, needle: &str, max_frames: u32) -> String {
    let mut text = String::new();
    for _ in 0..max_frames {
        let _ = machine.run_frame();
        text = screen_text(machine);
        if text.contains(needle) {
            return text;
        }
    }
    text
}

fn assert_screen_has(text: &str, needle: &str) {
    assert!(
        text.contains(needle),
        "expected screen to contain {needle:?}\n{text}"
    );
}

/// Contended-NOP totals (4T + delay): eight columns `10 9 8 7 6 5 4 4`.
fn contended_nop_pattern_present(text: &str) -> bool {
    const EXPECTED: [i32; 8] = [10, 9, 8, 7, 6, 5, 4, 4];
    text.lines().any(|line| {
        let nums: Vec<i32> = line
            .split_whitespace()
            .filter_map(|tok| tok.parse().ok())
            .collect();
        nums.windows(EXPECTED.len())
            .any(|window| window == EXPECTED)
    })
}

fn assert_contended_nop_pattern(text: &str) {
    assert!(
        contended_nop_pattern_present(text),
        "expected contended-NOP totals [10,9,8,7,6,5,4,4] on one screen row\n{text}"
    );
}

fn new_model(model: Model) -> Option<Machine> {
    match model {
        Model::Spectrum48 => {
            let rom = std::fs::read(rom48_path()).ok()?;
            Machine::new_48k(&rom).ok()
        }
        Model::Spectrum128 => {
            let rom = std::fs::read(rom128_path()).ok()?;
            Machine::new_128k(&rom).ok()
        }
        Model::SpectrumPlus3 => {
            let rom = rom_plus3()?;
            Machine::new_plus3(&rom).ok()
        }
    }
}

fn skip_no_rom(model: Model) -> Option<Machine> {
    match new_model(model) {
        Some(m) => Some(m),
        None => {
            eprintln!("skip: ROM missing for {model:?} — run ./scripts/fetch_roms.sh");
            None
        }
    }
}

#[test]
fn rom48_contains_sinclair_copyright_string() {
    let Ok(rom) = std::fs::read(rom48_path()) else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let needle = b"1982 Sinclair Research";
    assert!(
        rom.windows(needle.len()).any(|w| w == needle),
        "fetched 48K ROM must be a Sinclair 48K image"
    );
}

#[test]
fn rom48_boot_prints_copyright_on_screen() {
    let Some(mut machine) = skip_no_rom(Model::Spectrum48) else {
        return;
    };
    let text = run_until_contains(&mut machine, "1982 Sinclair", 400);
    assert_screen_has(&text, "1982 Sinclair");
}

#[test]
fn rom128_boot_paints_editor_menu() {
    let Some(mut machine) = skip_no_rom(Model::Spectrum128) else {
        return;
    };
    for _ in 0..120 {
        let _ = machine.run_frame();
    }
    let nz = screen_nonzero(&machine);
    assert!(
        nz > 100,
        "128K menu should paint pixels, got {nz} nonzero (PC={:04X})",
        machine.cpu().regs.pc
    );
}

#[test]
fn plus3_boot_paints_editor_menu() {
    let Some(mut machine) = skip_no_rom(Model::SpectrumPlus3) else {
        return;
    };
    for _ in 0..120 {
        let _ = machine.run_frame();
    }
    let nz = screen_nonzero(&machine);
    assert!(
        nz > 100,
        "+3 menu should paint pixels, got {nz} nonzero (PC={:04X})",
        machine.cpu().regs.pc
    );
}

fn minfo_on(model: Model, frame_t: &str) {
    let Some(mut machine) = skip_no_rom(model) else {
        return;
    };
    let Some(tap) = tap_path("minfo.tap") else {
        return;
    };
    load_program_tap(&mut machine, &tap);
    let text = run_until_contains(&mut machine, frame_t, AFTER_LOAD_FRAMES);
    assert_screen_has(&text, "Frame time:");
    assert_screen_has(&text, frame_t);
    assert!(
        !text.contains("Frame time: Failed"),
        "Minfo failed to measure frame time\n{text}"
    );
}

#[test]
fn minfo_48k_frame_time_69888() {
    minfo_on(Model::Spectrum48, "69888");
}

#[test]
fn minfo_128k_frame_time_70908() {
    minfo_on(Model::Spectrum128, "70908");
}

#[test]
fn minfo_plus3_frame_time_70908() {
    minfo_on(Model::SpectrumPlus3, "70908");
}

#[test]
fn minfo_48k_int_and_first_contended() {
    let Some(mut machine) = skip_no_rom(Model::Spectrum48) else {
        return;
    };
    let Some(tap) = tap_path("minfo.tap") else {
        return;
    };
    load_program_tap(&mut machine, &tap);
    // Minfo’s INT-relative counter uses the “INT low = T1” numbering, so the
    // FAQ early-timing first contended cycle (14335 when INT low = T0) prints
    // as 14336 — matching Timing Test’s first contended-NOP row.
    let text = run_until_contains(&mut machine, "14336", AFTER_LOAD_FRAMES);
    assert!(
        text.lines()
            .any(|line| line.contains("INT time:") && line.contains("32")),
        "expected \"INT time:\" and 32 on one screen row\n{text}"
    );
    assert_screen_has(&text, "14336");
}

#[test]
fn timingtest_48k_frame_duration_69888() {
    let Some(mut machine) = skip_no_rom(Model::Spectrum48) else {
        return;
    };
    let Some(tap) = tap_path("timingtest.tap") else {
        return;
    };
    load_program_tap(&mut machine, &tap);
    let text = run_until_contains(&mut machine, "Frame duration:", AFTER_LOAD_FRAMES);
    assert_screen_has(&text, "Frame duration:");
    // Value is filled after the HALT-based measurement completes.
    let text = run_until_contains(&mut machine, "69888", AFTER_LOAD_FRAMES);
    assert_screen_has(&text, "69888");
}

#[test]
fn timingtest_48k_contended_nop_shows_delay_pattern() {
    let Some(mut machine) = skip_no_rom(Model::Spectrum48) else {
        return;
    };
    let Some(tap) = tap_path("timingtest.tap") else {
        return;
    };
    load_program_tap(&mut machine, &tap);
    let text = run_until_contains(&mut machine, "Choose test:", AFTER_LOAD_FRAMES);
    assert_screen_has(&text, "Choose test:");
    // Digit 0, then Enter (Spectrum matrix).
    hold_keys(&mut machine, &[(4, 0)], 10);
    hold_keys(&mut machine, &[], 5);
    hold_keys(&mut machine, &[(6, 0)], 10);
    hold_keys(&mut machine, &[], 5);
    let text = run_until_contains(&mut machine, "contended NOP", AFTER_LOAD_FRAMES);
    assert_screen_has(&text, "contended NOP");
    let mut text = String::new();
    for _ in 0..AFTER_LOAD_FRAMES {
        let _ = machine.run_frame();
        text = screen_text(&machine);
        if contended_nop_pattern_present(&text) {
            break;
        }
    }
    assert_contended_nop_pattern(&text);
}

#[test]
fn ulatest3_48k_banner_frame_and_grid() {
    let Some(mut machine) = skip_no_rom(Model::Spectrum48) else {
        return;
    };
    let Some(tap) = tap_path("ulatest3.tap") else {
        return;
    };
    load_program_tap(&mut machine, &tap);
    // Title first; the floating-bus/contention matrix then paints below it.
    let text = run_until_contains(&mut machine, "ULA test 3 by JB", AFTER_LOAD_FRAMES);
    assert_screen_has(&text, "ULA test 3 by JB");
    let text = run_until_contains(&mut machine, "69888", AFTER_LOAD_FRAMES);
    assert_screen_has(&text, "69888");
    // Grid cells are largely attribute-coloured (empty bitmap); require painted attrs
    // in the matrix region rather than brittle glyph OCR of inverted cells.
    let mut matrix_attr_nz = 0usize;
    for row in 2..22u8 {
        for col in 0..32u8 {
            let attr_addr = 0x5800 + u16::from(row) * 32 + u16::from(col);
            if machine.read_mem(attr_addr) != 0 {
                matrix_attr_nz += 1;
            }
        }
    }
    assert!(
        matrix_attr_nz > 64,
        "ULA test 3 floating-bus/contention grid should paint attrs below the title, got {matrix_attr_nz} nonzero attr cells\n{text}"
    );
}
