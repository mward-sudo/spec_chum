//! Arkanoid Speedlock stage tracer (#379).
//!
//! Reports [`machine::SpeedlockStage`] edges + `f408_entries` so expected delay
//! (D) can be told from an unknown hang (E). Env:
//! - `SPEC_CHUM_ARKANOID_TZX` — tape path (else `~/Downloads/Arkanoid.tzx`)
//! - `SPEC_CHUM_SPEEDLOCK_SNAP=1` — experimental F476 snap (default off; H4)
//!
//! ```bash
//! cargo run -p host_api --release --example arkanoid_delay_probe -- [tzx] [max_host]
//! ```

use machine::{SpeedlockStage, TapeLoadOptions};
use spec_chum_host::{HostSession, ModelId};
use std::env;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Instant;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn arkanoid_path() -> PathBuf {
    env::var_os("SPEC_CHUM_ARKANOID_TZX")
        .map(PathBuf::from)
        .or_else(|| env::args().nth(1).map(PathBuf::from))
        .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join("Downloads/Arkanoid.tzx")))
        .expect("tape path")
}

fn main() {
    let tape = arkanoid_path();
    let max_post: u32 = env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .or_else(|| env::args().nth(1).and_then(|s| s.parse().ok()))
        .unwrap_or(20_000);

    let rom = std::fs::read(workspace_root().join("roms/spec48.rom")).expect("rom");
    let mut s = HostSession::new(ModelId::Spectrum48, true);
    s.load_rom_bytes(&rom).expect("rom");
    s.open_tape(&tape).expect("open");
    {
        let m = s.machine_mut().expect("m");
        m.set_tape_load_options(TapeLoadOptions {
            flash_load: false,
            speed: 64,
            experience_load: false,
        });
        m.set_tape_playing(false);
    }
    for _ in 0..200 {
        let _ = s.machine_mut().expect("m").run_frame();
    }
    {
        let m = s.machine_mut().expect("m");
        m.type_load_quotes(false);
        m.set_tape_playing(true);
    }

    let wall0 = Instant::now();
    eprintln!(
        "EAR @64× → tape end… (snap={})",
        env::var_os("SPEC_CHUM_SPEEDLOCK_SNAP").is_some()
    );
    for i in 0..40_000u32 {
        let _ = s.machine_mut().expect("m").run_frame();
        let m = s.machine().expect("m");
        if m.tape_finished() || !m.tape_playing() {
            eprintln!("tape end +{i} ({:.1}s)", wall0.elapsed().as_secs_f64());
            break;
        }
    }

    let post0 = Instant::now();
    let mut last = SpeedlockStage::None;
    let mut edges = 0u32;
    let mut saw_iff1 = false;
    let mut dump_f476 = false;

    for i in 0..max_post {
        let _ = s.machine_mut().expect("m").run_frame();
        let m = s.machine().expect("m");
        let stage = m.speedlock_stage();
        let watch = m.speedlock_watch();
        let pc = m.cpu().regs.pc;
        let e = m.cpu().regs.e;
        let bc = m.cpu().regs.bc();

        if stage != last && edges < 60 {
            eprintln!(
                "edge +{i} ({:.1}s): {} → {} pc={pc:#06x} e={e:#04x} bc={bc:#06x} f408={} frames={}",
                post0.elapsed().as_secs_f64(),
                last.as_str(),
                stage.as_str(),
                watch.f408_entries,
                watch.frames_in_stage,
            );
            edges += 1;
            let _ = io::stderr().flush();
        }
        last = stage;

        if !dump_f476 && pc == 0xF476 && bc == 0 {
            dump_f476 = true;
            eprintln!(
                "F476-state +{i}: af={:#06x} bc={bc:#06x} de={:#06x} hl={:#06x} ix={:#06x} iy={:#06x} sp={:#06x} r={:#04x} f408={}",
                m.cpu().regs.af(),
                m.cpu().regs.de(),
                m.cpu().regs.hl(),
                m.cpu().regs.ix(),
                m.cpu().regs.iy(),
                m.cpu().regs.sp,
                m.cpu().regs.r,
                m.speedlock_stage_count(),
            );
            // Mem fingerprint for Fuse bisect (H5): first 64 bytes at $F400 / $8220.
            eprint!("  mem F400:");
            for a in 0xF400u16..0xF440 {
                eprint!(" {:02x}", m.read_mem(a));
            }
            eprintln!();
            eprint!("  mem 8220:");
            for a in 0x8220u16..0x8260 {
                eprint!(" {:02x}", m.read_mem(a));
            }
            eprintln!();
        }

        if m.cpu().regs.iff1 {
            saw_iff1 = true;
            eprintln!(
                "GAME ENTRY +{i} / {:.1}s post; f408={} edges={edges} pc={pc:#06x}",
                post0.elapsed().as_secs_f64(),
                m.speedlock_stage_count(),
            );
            break;
        }

        if i > 0 && i % 5_000 == 0 {
            eprintln!(
                "hb +{i} ({:.0}s): stage={} pc={pc:#06x} f408={} changes={}",
                post0.elapsed().as_secs_f64(),
                stage.as_str(),
                watch.f408_entries,
                watch.stage_changes,
            );
            let _ = io::stderr().flush();
        }
        assert_ne!(pc, 0xFD2A, "sampler");
    }

    let m = s.machine().expect("m");
    eprintln!(
        "summary: iff1={} stage={} f408={} changes={} post_wall={:.1}s",
        u8::from(saw_iff1),
        m.speedlock_stage().as_str(),
        m.speedlock_stage_count(),
        m.speedlock_watch().stage_changes,
        post0.elapsed().as_secs_f64()
    );
    if !saw_iff1 {
        std::process::exit(2);
    }
}
