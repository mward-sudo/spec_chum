//! #179 regression: +2A menu Loader + Deathchase (env-gated local TZX).
//!
//! `SPEC_CHUM_DEATHCHASE_TZX` → path to local TZX (do not commit). Skips when unset/ROMs missing.

use machine::{Machine, Model, TapeLoadOptions};
use std::path::PathBuf;
use tape::{TapPlayer, TzxPlayer};

fn rom_plus2a() -> Option<Vec<u8>> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/plus2a/plus2a.rom");
    std::fs::read(p).ok().filter(|r| r.len() >= 64 * 1024)
}

fn load_deathchase() -> Option<TapPlayer> {
    let path = std::env::var_os("SPEC_CHUM_DEATHCHASE_TZX")?;
    let data = std::fs::read(path).ok()?;
    TzxPlayer::to_tap_player(&data).ok()
}

fn game_running(m: &Machine) -> bool {
    m.cpu().regs.pc >= 0x6000 && !m.cpu().regs.iff1
}

#[test]
fn plus2a_menu_loader_instant_runs_deathchase() {
    let Some(rom) = rom_plus2a() else {
        eprintln!("skip: plus2a ROM missing");
        return;
    };
    let Some(player) = load_deathchase() else {
        eprintln!("skip: set SPEC_CHUM_DEATHCHASE_TZX");
        return;
    };
    let mut m = Machine::new_plus2a(&rom).expect("plus2a");
    assert_eq!(m.model(), Model::SpectrumPlus2A);
    m.set_tape_load_options(TapeLoadOptions {
        flash_load: true,
        ..Default::default()
    });
    m.insert_tape(player);
    m.set_tape_playing(false);
    for _ in 0..200 {
        let _ = m.run_frame();
    }
    m.type_load_quotes_plus2a(false); // menu Loader Enter
    m.set_tape_playing(true);
    for _ in 0..4_000u32 {
        let _ = m.run_frame();
        if game_running(&m) {
            return;
        }
    }
    panic!(
        "+2A menu Loader should run Deathchase; PC={:04X} 7FFD={:?}",
        m.cpu().regs.pc,
        m.inspect().paging.page_7ffd
    );
}

#[test]
fn plus2a_48basic_instant_still_runs_deathchase() {
    let Some(rom) = rom_plus2a() else {
        eprintln!("skip: plus2a ROM missing");
        return;
    };
    let Some(player) = load_deathchase() else {
        eprintln!("skip: set SPEC_CHUM_DEATHCHASE_TZX");
        return;
    };
    let mut m = Machine::new_plus2a(&rom).expect("plus2a");
    m.set_tape_load_options(TapeLoadOptions {
        flash_load: true,
        ..Default::default()
    });
    m.insert_tape(player);
    m.set_tape_playing(false);
    for _ in 0..200 {
        let _ = m.run_frame();
    }
    m.type_load_quotes_plus3(false);
    m.set_tape_playing(true);
    for _ in 0..4_000u32 {
        let _ = m.run_frame();
        if game_running(&m) {
            return;
        }
    }
    panic!(
        "+2A 48 BASIC Instant regression; PC={:04X}",
        m.cpu().regs.pc
    );
}

#[test]
fn plus2a_menu_loader_ear_runs_deathchase() {
    let Some(rom) = rom_plus2a() else {
        eprintln!("skip: plus2a ROM missing");
        return;
    };
    let Some(player) = load_deathchase() else {
        eprintln!("skip: set SPEC_CHUM_DEATHCHASE_TZX");
        return;
    };
    let mut m = Machine::new_plus2a(&rom).expect("plus2a");
    m.set_tape_load_options(TapeLoadOptions {
        flash_load: false,
        speed: 20,
        ..Default::default()
    });
    m.insert_tape(player);
    m.set_tape_playing(false);
    for _ in 0..200 {
        let _ = m.run_frame();
    }
    m.type_load_quotes_plus2a(false);
    m.set_tape_playing(true);
    for _ in 0..30_000u32 {
        let _ = m.run_frame();
        if game_running(&m) {
            return;
        }
    }
    panic!(
        "+2A menu Loader EAR×20 should run Deathchase; PC={:04X}",
        m.cpu().regs.pc
    );
}
