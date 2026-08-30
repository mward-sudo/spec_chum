//! Death Chase EAR on Timex — env-gated local TZX (`SPEC_CHUM_DEATHCHASE_TZX`).
//!
//! 3D Deathchase uses Spectrum-absolute `USR` addresses (PROG at `$5CCB`). Timex
//! TS2068 places PROG at `$6856`, so the embedded loader never runs — that is
//! authentic Timex incompatibility (use TC2048 / 48K, or a HOME Spectrum `.dck`
//! on TS2068 — see docs/TIMEX.md). TC2048 keeps a Spectrum-compatible ROM and
//! must still load.

use machine::{resolve_exrom_path, resolve_rom_path, Machine, Model, TapeLoadOptions};
use tape::TzxPlayer;

fn load_deathchase() -> Option<tape::TapPlayer> {
    let path = std::env::var_os("SPEC_CHUM_DEATHCHASE_TZX")?;
    let data = std::fs::read(path).ok()?;
    TzxPlayer::to_tap_player(&data).ok()
}

/// Deathchase screen/game image starts at 0x4000; after a real CODE load, 0x5000 is non-zero.
fn deathchase_code_resident(m: &Machine) -> bool {
    m.read_mem(0x5000) != 0
}

fn game_running(m: &Machine) -> bool {
    m.cpu().regs.pc >= 0x6000 && !m.cpu().regs.iff1 && deathchase_code_resident(m)
}

fn try_ear(model: Model, speed: u32, max_frames: u32) -> Option<(bool, u16, u16)> {
    let player = load_deathchase()?;
    let mut m = match model {
        Model::Spectrum48 => {
            let rom = resolve_rom_path(Model::Spectrum48).and_then(|p| std::fs::read(p).ok())?;
            Machine::new_48k(&rom).ok()?
        }
        Model::TimexTC2048 => {
            let rom = resolve_rom_path(Model::TimexTC2048).and_then(|p| std::fs::read(p).ok())?;
            Machine::new_timex_tc2048(&rom).ok()?
        }
        Model::TimexTS2068 => {
            let home = resolve_rom_path(Model::TimexTS2068).and_then(|p| std::fs::read(p).ok())?;
            let ex = resolve_exrom_path(Model::TimexTS2068).and_then(|p| std::fs::read(p).ok())?;
            Machine::new_timex_ts2068(&home, &ex).ok()?
        }
        _ => return None,
    };
    m.set_tape_load_options(TapeLoadOptions {
        flash_load: false,
        speed,
        ..Default::default()
    });
    m.insert_tape(player);
    m.set_tape_playing(false);
    for _ in 0..250 {
        let _ = m.run_frame();
    }
    let prog = u16::from(m.read_mem(0x5C53)) | (u16::from(m.read_mem(0x5C54)) << 8);
    m.type_load_quotes(false);
    m.set_tape_playing(true);
    let mut ok = false;
    for _ in 0..max_frames {
        let _ = m.run_frame();
        if game_running(&m) {
            ok = true;
            break;
        }
    }
    Some((ok, m.cpu().regs.pc, prog))
}

#[test]
fn deathchase_ear_loads_on_spectrum_rom_models() {
    if std::env::var_os("SPEC_CHUM_DEATHCHASE_TZX").is_none() {
        eprintln!("skip: set SPEC_CHUM_DEATHCHASE_TZX");
        return;
    }
    let speed = 16u32;
    let max = 50_000u32;
    for (model, label) in [(Model::Spectrum48, "48k"), (Model::TimexTC2048, "tc2048")] {
        let (ok, pc, prog) = try_ear(model, speed, max).expect("rom/tzx");
        eprintln!("{label}: ok={ok} PC={pc:04X} PROG={prog:04X}");
        assert!(ok, "{label} EAR should run Death Chase");
    }
}

#[test]
fn deathchase_ts2068_prog_incompatible_with_absolute_usr() {
    if std::env::var_os("SPEC_CHUM_DEATHCHASE_TZX").is_none() {
        eprintln!("skip: set SPEC_CHUM_DEATHCHASE_TZX");
        return;
    }
    let (ok, pc, prog) = try_ear(Model::TimexTS2068, 16, 20_000).expect("rom/tzx");
    eprintln!("ts2068: ok={ok} PC={pc:04X} PROG={prog:04X} (Spectrum PROG is 5CCB)");
    // Authentic Timex map: PROG lives above the Spectrum printer-buffer USR target.
    assert_eq!(prog, 0x6856, "TS2068 PROG must stay at Timex default");
    assert!(
        !ok,
        "Death Chase must not falsely 'run' on TS2068 without Spectrum cart"
    );
}
