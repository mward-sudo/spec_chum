//! Arkanoid Speedlock post-load gate probe (#379).
//!
//! Findings (this tool):
//! - `$8230` `"PBRAIN"` compare is a **high-score cheat** path (`$8247`→`$8254`),
//!   not the load-path exit. Forcing the string reaches `$8247` but the hit-path
//!   key-wait falls into shared `$8280` without `IFF1`.
//! - `$94F6` is a screen/cursor pointer written from `$99BB` / `$8F53`.
//! - After the `$9F03` 4-count / `$94EE` countdown, execution reaches `$8025`
//!   (high-score table scan vs `$E98A` — names like `RO`/`JO`/`ST`…) then
//!   **`$808E` title/high-score draw** (`CALL $8368` clear, blit `$DC40`).
//! - Attract mode then re-enters `$F448`. Holding Space through the delay
//!   corrupts the nest; `IFF1` still not observed after `$808E` in soaks.
//!
//! Modes (env `SPEC_CHUM_PBRAIN_MODE`):
//! - `watch` (default): write-watch `$94F6`/`$94F7`/`$955A` + PC break `$8230`
//! - `force`: poke `"PBRAIN"` at `($94F6)+1` on each `$8230` hit
//! - `fuse`: dump `$955A` / `$E98A` high-score candidates from a Fuse `.z80`
//! - `soak`: break on `$814E`/`$8025`/`$808E` and dump signature state
//! - `title`: wait for `$808E` (title draw), tap Space only in `$80xx`–`$8Fxx`
//!
//! ```bash
//! SPEC_CHUM_SPEEDLOCK_BOOST=16 cargo run -p host_api --release \
//!   --example arkanoid_pbrain_watch -- [tzx] [max_events]
//! ```

use machine::{BreakReason, TapeLoadOptions, Watch};
use spec_chum_host::{HostSession, ModelId};
use std::env;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

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

fn word(m: &machine::Machine, a: u16) -> u16 {
    u16::from(m.read_mem(a)) | (u16::from(m.read_mem(a.wrapping_add(1))) << 8)
}

fn hex_range(m: &machine::Machine, base: u16, n: u16) -> String {
    (0..n)
        .map(|i| format!("{:02x}", m.read_mem(base.wrapping_add(i))))
        .collect::<Vec<_>>()
        .join(" ")
}

fn load_tape(s: &mut HostSession, tape: &Path) {
    s.open_tape(tape).expect("open");
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
    for i in 0..40_000u32 {
        let _ = s.machine_mut().expect("m").run_frame();
        let m = s.machine().expect("m");
        if m.tape_finished() || !m.tape_playing() {
            eprintln!("tape end +{i}");
            break;
        }
    }
}

fn main() {
    let tape = arkanoid_path();
    let max_events: u32 = env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(80);
    let mode = env::var("SPEC_CHUM_PBRAIN_MODE").unwrap_or_else(|_| "watch".into());
    let force = mode == "force";
    let soak = mode == "soak";
    let fuse_dump = mode == "fuse";
    let title = mode == "title";

    let rom = std::fs::read(workspace_root().join("roms/spec48.rom")).expect("rom");
    let mut s = HostSession::new(ModelId::Spectrum48, true);
    s.load_rom_bytes(&rom).expect("rom");

    if fuse_dump {
        let snap = env::args().nth(1).map_or_else(
            || workspace_root().join("tmp/fuse_oracle/arkanoid_fuse_postload.z80"),
            PathBuf::from,
        );
        s.load_snapshot(&snap).expect("snap");
        let m = s.machine().expect("m");
        eprintln!(
            "fuse {} pc={:#06x} iff1={}",
            snap.display(),
            m.cpu().regs.pc,
            u8::from(m.cpu().regs.iff1)
        );
        eprintln!("  sig $955A: {}", hex_range(m, 0x955A, 8));
        eprintln!(
            "  $9596={:#04x} $94EE={:#04x} $9F03={:#04x}",
            m.read_mem(0x9596),
            m.read_mem(0x94EE),
            m.read_mem(0x9F03)
        );
        for n in 0..6u16 {
            let base = 0xE98Au16.wrapping_add(n * 0x1A);
            eprintln!("  cand{n} {base:#06x}: {}", hex_range(m, base, 8));
        }
        return;
    }

    load_tape(&mut s, &tape);

    if title {
        let track: u32 = env::var("SPEC_CHUM_TRACK_FRAMES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(20_000);
        {
            let m = s.machine_mut().expect("m");
            m.debugger_mut().pc_breaks = vec![0x808E];
        }
        let mut saw_title = false;
        for i in 0..track {
            {
                let m = s.machine_mut().expect("m");
                if m.debugger().paused && m.cpu().regs.pc == 0x808E {
                    eprintln!("TITLE $808E at +{i}f — clearing breaks, holding Space");
                    m.debugger_mut().clear_breaks();
                    m.debugger_mut().paused = false;
                    // Kempston fire + matrix Space (0,7) + 0 key often used as fire.
                    m.keyboard_mut().set_key(7, 0, true);
                    saw_title = true;
                } else if m.debugger().paused {
                    let pc = m.cpu().regs.pc;
                    m.debugger_mut().continue_from_pc(pc);
                }
                if saw_title {
                    let pc = m.cpu().regs.pc;
                    // Only assert Space while in title/menu code — holding it
                    // through `$F448` re-enters the key-gate and corrupts the nest.
                    let in_menu =
                        (0x8000..=0x83FF).contains(&pc) || (0x8400..=0x8FFF).contains(&pc);
                    m.keyboard_mut().set_key(7, 0, in_menu);
                }
                let _ = m.run_frame();
            }
            let m = s.machine().expect("m");
            if i % 200 == 0 || m.cpu().regs.iff1 {
                eprintln!(
                    "+{i}f pc={:#06x} iff1={} title={saw_title} stage={}",
                    m.cpu().regs.pc,
                    u8::from(m.cpu().regs.iff1),
                    m.speedlock_stage().as_str(),
                );
            }
            if m.cpu().regs.iff1 {
                eprintln!("GAME ENTRY after title +{i}f pc={:#06x}", m.cpu().regs.pc);
                break;
            }
        }
        let m = s.machine().expect("m");
        eprintln!(
            "title done saw={saw_title} pc={:#06x} iff1={}",
            m.cpu().regs.pc,
            u8::from(m.cpu().regs.iff1)
        );
        return;
    }

    if soak {
        let track: u32 = env::var("SPEC_CHUM_TRACK_FRAMES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(50_000);
        {
            let m = s.machine_mut().expect("m");
            // Catch post-4-count fallthrough and miss-path zero-count exit.
            m.debugger_mut().pc_breaks = vec![
                0x814E, 0x8155, 0x8289, 0x83B9, 0x83D1, 0x8025, 0x804C, 0x804F, 0x808E, 0xF3D7,
                0xF3D8,
            ];
        }
        let mut prev_ee = 0xffu8;
        let mut prev_stage = String::new();
        let mut hits = 0u32;
        for i in 0..track {
            {
                let m = s.machine_mut().expect("m");
                if m.debugger().paused {
                    let pc = m.cpu().regs.pc;
                    hits += 1;
                    eprintln!(
                        "BREAK#{hits} +{i}f pc={pc:#06x} iff1={} $94EE={:#04x} $9F03={:#04x} $956F={:#04x} $9596={:#04x} reason={:?}",
                        u8::from(m.cpu().regs.iff1),
                        m.read_mem(0x94EE),
                        m.read_mem(0x9F03),
                        m.read_mem(0x956F),
                        m.read_mem(0x9596),
                        m.debugger().last_hit,
                    );
                    if pc == 0x8025 {
                        eprintln!("  sig $955A: {}", hex_range(m, 0x955A, 5));
                        for n in 0..6u16 {
                            let base = 0xE98Au16.wrapping_add(n * 0x1A);
                            eprintln!("  cand{n} {base:#06x}: {}", hex_range(m, base, 5));
                        }
                    }
                    if pc == 0x808E || pc == 0x804C {
                        eprintln!("  FAIL/fallback $9596={:#04x}", m.read_mem(0x9596));
                    }
                    m.debugger_mut().continue_from_pc(pc);
                }
                let _ = m.run_frame();
            }
            let m = s.machine().expect("m");
            let ee = m.read_mem(0x94EE);
            let stage = m.speedlock_stage().as_str().to_string();
            let pc = m.cpu().regs.pc;
            let changed = ee != prev_ee || stage != prev_stage || i % 1000 == 0;
            if changed {
                eprintln!(
                    "+{i}f pc={pc:#06x} iff1={} $94EE={ee:#04x} $9F03={:#04x} $956F={:#04x} stage={stage} ($94F6)={:#06x}",
                    u8::from(m.cpu().regs.iff1),
                    m.read_mem(0x9F03),
                    m.read_mem(0x956F),
                    word(m, 0x94F6),
                );
                let _ = io::stderr().flush();
                prev_ee = ee;
                prev_stage = stage;
            }
            if m.cpu().regs.iff1 {
                eprintln!("GAME ENTRY +{i}f pc={pc:#06x}");
                break;
            }
            if hits >= 30 {
                eprintln!("hit budget");
                break;
            }
        }
        {
            let m = s.machine().expect("m");
            eprintln!("--- post-soak disasm around PC ---");
            let pc = m.cpu().regs.pc;
            match s.disasm(Some(pc.wrapping_sub(8)), 24) {
                Ok(t) => eprintln!("{t}"),
                Err(e) => eprintln!("{e}"),
            }
            match s.disasm(Some(0x83B9), 40) {
                Ok(t) => eprintln!("--- 83B9 ---\n{t}"),
                Err(e) => eprintln!("{e}"),
            }
            match s.disasm(Some(0x8025), 32) {
                Ok(t) => eprintln!("--- 8025 ---\n{t}"),
                Err(e) => eprintln!("{e}"),
            }
            match s.disasm(Some(0x8289), 24) {
                Ok(t) => eprintln!("--- 8289 ---\n{t}"),
                Err(e) => eprintln!("{e}"),
            }
            match s.disasm(Some(0xF296), 20) {
                Ok(t) => eprintln!("--- F296 ---\n{t}"),
                Err(e) => eprintln!("{e}"),
            }
            match s.disasm(Some(0x808E), 80) {
                Ok(t) => eprintln!("--- 808E ---\n{t}"),
                Err(e) => eprintln!("{e}"),
            }
            match s.disasm(Some(0x804F), 24) {
                Ok(t) => eprintln!("--- 804F hit ---\n{t}"),
                Err(e) => eprintln!("{e}"),
            }
            eprintln!(
                "soak done pc={:#06x} iff1={} $94EE={:#04x} hits={hits}",
                m.cpu().regs.pc,
                u8::from(m.cpu().regs.iff1),
                m.read_mem(0x94EE)
            );
        }
        return;
    }

    {
        let m = s.machine_mut().expect("m");
        m.debugger_mut().pc_breaks = vec![0x8230, 0x8247, 0x824E, 0x8280, 0xF3D8, 0x83B9];
        if !force {
            for addr in [0x94F6u16, 0x94F7, 0x955A, 0x955B, 0x955C, 0x955D, 0x955E] {
                m.debugger_mut().add_mem_watch(Watch {
                    addr,
                    read: false,
                    write: true,
                });
            }
        }
    }

    {
        let m = s.machine().expect("m");
        eprintln!("--- post-load gate state ---");
        eprintln!(
            "  ($94F6)={:#06x} $956F={:#04x} $9F03={:#04x} pc={:#06x}",
            word(m, 0x94F6),
            m.read_mem(0x956F),
            m.read_mem(0x9F03),
            m.cpu().regs.pc
        );
        eprintln!("  expected $9570: {}", hex_range(m, 0x9570, 6));
        let _ = io::stderr().flush();
    }
    for (label, addr, n) in [
        ("F3C1 entry", 0xF3C1u16, 16usize),
        ("post-4count", 0x814E, 24),
        ("8254 hit-path", 0x8254, 24),
        ("8368 helper", 0x8368, 20),
        ("839D miss-cont", 0x839D, 16),
    ] {
        match s.disasm(Some(addr), n) {
            Ok(text) => eprintln!("--- {label} @ {addr:#06x} ---\n{text}"),
            Err(e) => eprintln!("--- {label} FAILED: {e} ---"),
        }
    }

    let mut events = 0u32;
    let mut frames = 0u32;
    let mut forced = 0u32;
    let mut hit_8247 = 0u32;
    while events < max_events && frames < 80_000 {
        {
            let m = s.machine_mut().expect("m");
            let at = m.cpu().regs.pc;
            m.debugger_mut().continue_from_pc(at);
            // After a mem-watch stop, clear paused so the write can complete next step.
            if m.debugger().paused {
                m.debugger_mut().paused = false;
            }
            let _ = m.run_frame();
            frames += 1;
        }
        let force_at = {
            let m = s.machine().expect("m");
            if !m.debugger().paused {
                if m.cpu().regs.iff1 {
                    eprintln!("GAME ENTRY iff1=1 +{frames}f forced={forced} hit_8247={hit_8247}");
                    break;
                }
                continue;
            }
            events += 1;
            let reason = m.debugger().last_hit;
            let r = &m.cpu().regs;
            let mut force_at = None;
            match reason {
                BreakReason::Mem { addr, write, value } => {
                    eprintln!(
                        "w#{events} +{frames}f MEM {addr:#06x} write={write} val={value:#04x} pc={:#06x} hl={:#06x} de={:#06x} bc={:#06x} ($94F6)={:#06x}",
                        r.pc,
                        r.hl(),
                        r.de(),
                        r.bc(),
                        word(m, 0x94F6),
                    );
                }
                BreakReason::Pc(pc) => {
                    let ptr = word(m, 0x94F6);
                    let hl = ptr.wrapping_add(1);
                    eprintln!(
                        "p#{events} +{frames}f PC {pc:#06x} af={:#06x} bc={:#06x} de={:#06x} hl={:#06x} ($94F6)={ptr:#06x} bytes@hl={} $956F={:#04x} $9F03={:#04x}",
                        r.af(),
                        r.bc(),
                        r.de(),
                        r.hl(),
                        hex_range(m, hl, 6),
                        m.read_mem(0x956F),
                        m.read_mem(0x9F03),
                    );
                    if pc == 0x8247 {
                        hit_8247 += 1;
                    }
                    if force && pc == 0x8230 {
                        force_at = Some(hl);
                    }
                }
                other => {
                    eprintln!("e#{events} +{frames}f {other:?} pc={:#06x}", r.pc);
                }
            }
            let _ = io::stderr().flush();
            force_at
        };
        if let Some(hl) = force_at {
            let m = s.machine_mut().expect("m");
            for (i, b) in b"PBRAIN".iter().enumerate() {
                m.write_mem(hl.wrapping_add(i as u16), *b);
            }
            forced += 1;
            eprintln!("  FORCE wrote PBRAIN at {hl:#06x}: {}", hex_range(m, hl, 6));
        }
    }

    let m = s.machine().expect("m");
    eprintln!(
        "summary: events={events} frames={frames} forced={forced} hit_8247={hit_8247} pc={:#06x} iff1={} ($94F6)={:#06x} $956F={:#04x} $9F03={:#04x}",
        m.cpu().regs.pc,
        u8::from(m.cpu().regs.iff1),
        word(m, 0x94F6),
        m.read_mem(0x956F),
        m.read_mem(0x9F03),
    );
}
