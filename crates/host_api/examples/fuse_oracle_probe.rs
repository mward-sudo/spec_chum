//! Fuse Oracle C helper for #379.
//!
//! Load a Fuse-saved Arkanoid `.z80` (convert `.szx` offline) into Spec Chum and
//! see whether post-load Speedlock reaches `IFF1` / game entry. If yes → defect
//! is on the EAR/TZX load path; if no → post-load CPU/timing (H5 `LD A,R`, …).
//!
//! ```bash
//! cargo run -p host_api --release --example fuse_oracle_probe -- \
//!   tmp/fuse_oracle/arkanoid_fuse_postload.z80 [max_host_frames]
//! ```

use machine::{SpeedlockStage, TapeLoadOptions};
use spec_chum_host::{HostSession, ModelId};
use std::env;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Instant;
use tape::{TapImage, TapPlayer};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn main() {
    let snap = env::args().nth(1).map_or_else(
        || workspace_root().join("tmp/fuse_oracle/arkanoid_fuse_postload.z80"),
        PathBuf::from,
    );
    let max_frames: u32 = env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(30_000);

    let rom = std::fs::read(workspace_root().join("roms/spec48.rom")).expect("rom");
    let mut s = HostSession::new(ModelId::Spectrum48, true);
    s.load_rom_bytes(&rom).expect("rom");
    s.load_snapshot(&snap).expect("snap");

    {
        let m = s.machine_mut().expect("m");
        // Empty finished deck → `tape_finished` so EAR turbo + Speedlock watch apply.
        let mut empty = TapPlayer::new(TapImage::default());
        empty.set_playing(false);
        m.insert_tape(empty);
        m.set_tape_load_options(TapeLoadOptions {
            flash_load: false,
            speed: 64,
            experience_load: false,
        });
        eprintln!(
            "loaded {} pc={:#06x} iff1={} r={:#04x} af={:#06x} bc={:#06x} de={:#06x} hl={:#06x} sp={:#06x} tape_fin={}",
            snap.display(),
            m.cpu().regs.pc,
            u8::from(m.cpu().regs.iff1),
            m.cpu().regs.r,
            m.cpu().regs.af(),
            m.cpu().regs.bc(),
            m.cpu().regs.de(),
            m.cpu().regs.hl(),
            m.cpu().regs.sp,
            m.tape_finished(),
        );
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

    let wall0 = Instant::now();
    let mut last = SpeedlockStage::None;
    let mut edges = 0u32;
    let mut saw_iff1 = false;

    for i in 0..max_frames {
        let _ = s.machine_mut().expect("m").run_frame();
        let m = s.machine().expect("m");
        // Without a finished tape, speedlock_stage() stays None — report raw PC/IFF1.
        let pc = m.cpu().regs.pc;
        let iff1 = m.cpu().regs.iff1;
        if iff1 {
            saw_iff1 = true;
            eprintln!(
                "GAME ENTRY +{i} / {:.1}s pc={pc:#06x}",
                wall0.elapsed().as_secs_f64()
            );
            break;
        }
        // Synthetic stage from PC alone for snap-without-tape soaks.
        let stage = if (0xF408..=0xF476).contains(&pc) {
            SpeedlockStage::DelayF448
        } else if (0x8224..=0x83FF).contains(&pc) {
            SpeedlockStage::Continue8230
        } else if (0x9300..=0x94FF).contains(&pc) {
            SpeedlockStage::Decrypt93
        } else if pc >= 0x8000 {
            SpeedlockStage::OtherHighDi
        } else {
            SpeedlockStage::None
        };
        if stage != last && edges < 40 {
            eprintln!(
                "edge +{i} ({:.1}s): {} → {} pc={pc:#06x} r={:#04x}",
                wall0.elapsed().as_secs_f64(),
                last.as_str(),
                stage.as_str(),
                m.cpu().regs.r,
            );
            edges += 1;
            let _ = io::stderr().flush();
        }
        last = stage;
        if i > 0 && i % 5_000 == 0 {
            eprintln!(
                "hb +{i} ({:.0}s): pc={pc:#06x} stage={}",
                wall0.elapsed().as_secs_f64(),
                stage.as_str()
            );
        }
    }

    let m = s.machine().expect("m");
    eprintln!(
        "summary: iff1={} pc={:#06x} post_wall={:.1}s",
        u8::from(saw_iff1),
        m.cpu().regs.pc,
        wall0.elapsed().as_secs_f64()
    );
    if !saw_iff1 {
        std::process::exit(2);
    }
}
