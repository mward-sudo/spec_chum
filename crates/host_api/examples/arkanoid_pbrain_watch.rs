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
//! - Attract mode then re-enters `$F448`. Title draw patches `$F3D1` CALL to
//!   `$820C` (key-release) when `$9596==0`, else `$8166` (hiscore name entry).
//! - After the `$9F03` 4-count, `$8155` sets `($8403)=$8DC5`. Key-down there with
//!   `$94C5==0` only does `JP $8025` (attract restart) — not game start.
//! - `$8224` key-down patches `($8403)=$8DF1` (options / fire handler) then
//!   runs the `"PBRAIN"` compare. Turbo auto-ack must read return at SP+4 after
//!   `PUSH BC`/`PUSH AF` (#388).
//! - **Game entry (observed):** after that gate pass the main loop runs at
//!   `$83DF`/`$841x` with `($8403)=$8DF1`, live `$94C4` countdown, and
//!   **`IFF1` stays 0** (game polls under DI; `$F2AC` EI is only a brief island).
//!   Prefer these markers over `IFF1=1` for playable entry.
//! - Port watches are exact 16-bit (`#387`); prefer PC-breaks on known `IN` sites.
//!
//! Modes (env `SPEC_CHUM_PBRAIN_MODE`):
//! - `watch` (default): write-watch `$94F6`/`$94F7`/`$955A` + PC break `$8230`
//! - `force`: poke `"PBRAIN"` at `($94F6)+1` on each `$8230` hit
//! - `fuse`: dump `$955A` / `$E98A` high-score candidates from a Fuse `.z80`
//! - `soak`: break on `$814E`/`$8025`/`$808E` and dump signature state
//! - `title` / `menu`: after `$808E`, port-watch `$FE` keyboard polls; tap Space
//!   only when the poll PC is in menu RAM (never during `$F448`)
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
    let title = mode == "title" || mode == "menu";

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
            .unwrap_or(8_000);
        let max_fe: u32 = env::var("SPEC_CHUM_FE_EVENTS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(80);

        // Phase 1: reach title draw.
        {
            let m = s.machine_mut().expect("m");
            m.debugger_mut().pc_breaks = vec![0x808E];
        }
        let mut saw_title = false;
        let mut frames = 0u32;
        while frames < track && !saw_title {
            {
                let m = s.machine_mut().expect("m");
                if m.debugger().paused && m.cpu().regs.pc == 0x808E {
                    eprintln!("TITLE $808E at +{frames}f");
                    saw_title = true;
                    break;
                }
                if m.debugger().paused {
                    let pc = m.cpu().regs.pc;
                    m.debugger_mut().continue_from_pc(pc);
                }
                let _ = m.run_frame();
            }
            frames += 1;
        }
        if !saw_title {
            eprintln!("never hit $808E in {frames}f");
            return;
        }
        for (label, addr, n) in [
            ("808E title", 0x808Eu16, 80usize),
            ("8100 cont", 0x8100, 50),
            ("8166 key?", 0x8166, 50),
            ("8207?", 0x8207, 40),
            ("8DC5 attract-key", 0x8DC5, 50),
            ("8DF1 fire-handler", 0x8DF1, 60),
            ("83A0 post-title", 0x83A0, 40),
            ("F2AC EI island", 0xF2A0, 20),
        ] {
            match s.disasm(Some(addr), n) {
                Ok(t) => eprintln!("--- {label} @ {addr:#06x} ---\n{t}"),
                Err(e) => eprintln!("--- {label} FAILED: {e} ---"),
            }
        }

        // Phase 2: chase fire-to-start. `$8DC5` with `$94C5==0` only restarts
        // attract (`JP $8025`). `$8224` patches `($8403)=$8DF1`. Watch `$94C5`
        // writes and `$F2AC` EI. Do not hold Space through `$F448`.
        {
            let m = s.machine_mut().expect("m");
            m.debugger_mut().clear_breaks();
            m.debugger_mut().paused = false;
            m.debugger_mut().pc_breaks =
                vec![0x8211, 0x8229, 0x8230, 0x8DC5, 0x8DF1, 0x8E2D, 0xF2AC];
            m.debugger_mut().add_port_watch(Watch {
                addr: 0x00FE,
                read: true,
                write: false,
            });
            m.debugger_mut().add_mem_watch(Watch {
                addr: 0x94C5,
                read: false,
                write: true,
            });
        }

        let mut fe_events = 0u32;
        let mut attract_polls = 0u32;
        let mut fire_hits = 0u32;
        let mut space_taps = 0u32;
        let mut unique_fe: Vec<(u16, u32)> = Vec::new();
        let mut saw_94c5_write = false;
        let mut gate_passed = false;
        let mut dumped_hang = false;
        let phase2_budget = track.saturating_sub(frames).max(2_000);
        // Pulse Space every Nth attract poll so we don't latch forever.
        let pulse_every: u32 = env::var("SPEC_CHUM_FIRE_PULSE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(64);

        for i in 0..phase2_budget {
            // After the title `$8224` gate passes, soak on run_frame so turbo
            // inject is not cleared between IN polls (#388).
            if gate_passed {
                let (pc, iff1, c5, vec8403, stage_now) = {
                    let m = s.machine_mut().expect("m");
                    if m.debugger().paused {
                        let pc = m.cpu().regs.pc;
                        if matches!(m.debugger().last_hit, BreakReason::Pc(0xF2AC)) {
                            eprintln!(
                                "EI $F2AC +{frames}f iff1={} $94C5={:#04x}",
                                u8::from(m.cpu().regs.iff1),
                                m.read_mem(0x94C5)
                            );
                        }
                        m.debugger_mut().continue_from_pc(pc);
                        m.debugger_mut().paused = false;
                    }
                    // Occasional fire pulse while soaking.
                    if i.is_multiple_of(64) {
                        m.keyboard_mut().set_key(7, 0, true);
                        m.keyboard_mut().set_key(4, 0, true);
                        space_taps += 1;
                    } else if i % 64 == 8 {
                        m.keyboard_mut().set_key(7, 0, false);
                        m.keyboard_mut().set_key(4, 0, false);
                    }
                    let _ = m.run_frame();
                    (
                        m.cpu().regs.pc,
                        m.cpu().regs.iff1,
                        m.read_mem(0x94C5),
                        word(m, 0x8403),
                        m.speedlock_stage().as_str().to_string(),
                    )
                };
                frames += 1;
                if !dumped_hang && (0x8410..=0x8428).contains(&pc) {
                    dumped_hang = true;
                    for (label, addr, n) in [
                        ("8370 after-gate", 0x8370u16, 40usize),
                        ("8400 call-site", 0x8400, 50),
                        ("8410 hang?", 0x8410, 40),
                        ("8F1A next", 0x8F1A, 30),
                    ] {
                        match s.disasm(Some(addr), n) {
                            Ok(t) => eprintln!("--- {label} @ {addr:#06x} ---\n{t}"),
                            Err(e) => eprintln!("--- {label} FAILED: {e} ---"),
                        }
                    }
                }
                if iff1 {
                    eprintln!(
                        "GAME ENTRY +{frames}f pc={pc:#06x} taps={space_taps} $94C5={c5:#04x} ($8403)={vec8403:#06x}"
                    );
                    break;
                }
                if frames.is_multiple_of(500) {
                    let (c4, dc, db, dd) = {
                        let m = s.machine().expect("m");
                        (
                            word(m, 0x94C4),
                            m.read_mem(0x94DC),
                            m.read_mem(0x94DB),
                            m.read_mem(0x94DD),
                        )
                    };
                    eprintln!(
                        "soak +{frames}f pc={pc:#06x} iff1={} $94C5={c5:#04x} $94C4={c4:#06x} $94DC={dc:#04x} $94DB={db:#04x} $94DD={dd:#04x} ($8403)={vec8403:#06x} stage={stage_now}",
                        u8::from(iff1)
                    );
                }
                continue;
            }

            // Pre-gate: release probe keys; turbo re-injects inside run_frame.
            {
                let m = s.machine_mut().expect("m");
                m.keyboard_mut().set_key(7, 0, false);
                m.keyboard_mut().set_key(4, 0, false);
            }

            let (reason, pc_now, iff1_now, stage_now, c5, ef, s95, vec8403, sp_word, ret4) = {
                let m = s.machine_mut().expect("m");
                if m.debugger().paused {
                    let pc = m.cpu().regs.pc;
                    m.debugger_mut().continue_from_pc(pc);
                    m.debugger_mut().paused = false;
                }
                let reason = m.run_until_break(200_000);
                let sp = m.cpu().regs.sp;
                (
                    reason,
                    m.cpu().regs.pc,
                    m.cpu().regs.iff1,
                    m.speedlock_stage().as_str().to_string(),
                    m.read_mem(0x94C5),
                    m.read_mem(0x94EF),
                    m.read_mem(0x9595),
                    word(m, 0x8403),
                    word(m, sp),
                    word(m, sp.wrapping_add(4)),
                )
            };

            frames += 1;
            if iff1_now {
                eprintln!(
                    "GAME ENTRY +{frames}f pc={pc_now:#06x} attract={attract_polls} fire={fire_hits} taps={space_taps} $94C5={c5:#04x}"
                );
                break;
            }

            match &reason {
                BreakReason::Mem {
                    addr: 0x94C5,
                    write: true,
                    ..
                } => {
                    saw_94c5_write = true;
                    eprintln!(
                        "WRITE $94C5 +{frames}f pc={pc_now:#06x} val={c5:#04x} $94EF={ef:#04x} ($8403)={vec8403:#06x}"
                    );
                }
                BreakReason::Pc(0xF2AC) => {
                    eprintln!(
                        "EI $F2AC +{frames}f iff1={} $94C5={c5:#04x} ($8403)={vec8403:#06x}",
                        u8::from(iff1_now)
                    );
                }
                BreakReason::Pc(0x8DF1) => {
                    fire_hits += 1;
                    if fire_hits <= 5 || fire_hits.is_multiple_of(500) {
                        eprintln!(
                            "FIRE $8DF1 #{fire_hits} +{frames}f $94C5={c5:#04x} $9595={s95:#04x} ($8403)={vec8403:#06x} stage={stage_now}"
                        );
                    }
                }
                BreakReason::Pc(0x8E2D) => {
                    if fire_hits <= 5 {
                        eprintln!(
                            "START-PATH $8E2D +{frames}f $94C5={c5:#04x} $9595={s95:#04x} ($8403)={vec8403:#06x}"
                        );
                    }
                }
                BreakReason::Pc(0x8230) => {
                    eprintln!(
                        "GATE-PASS $8230 +{frames}f (sp)={sp_word:#06x} (sp+4)={ret4:#06x} $94C5={c5:#04x} ($8403)={vec8403:#06x} stage={stage_now}"
                    );
                    gate_passed = true;
                    let m = s.machine_mut().expect("m");
                    // Finish the `$8403` patch then soak on frames for EI / IFF1.
                    m.debugger_mut().pc_breaks = vec![0xF2AC];
                    m.debugger_mut().continue_from_pc(0x8230);
                    m.debugger_mut().paused = false;
                    let _ = m.run_until_break(64);
                    eprintln!(
                        "  after patch ($8403)={:#06x} pc={:#06x}",
                        word(m, 0x8403),
                        m.cpu().regs.pc
                    );
                }
                _ => {}
            }

            let mut poll_pc: Option<(u16, &'static str)> = None;
            match reason {
                BreakReason::Port {
                    port, write: false, ..
                } if port & 0xFF == 0xFE => {
                    poll_pc = Some((pc_now, "port"));
                }
                BreakReason::Pc(pc) if matches!(pc, 0x8211 | 0x8229 | 0x8DC5) => {
                    poll_pc = Some((pc, "pc-break"));
                }
                BreakReason::Budget | BreakReason::None => {
                    let hit = {
                        let m = s.machine_mut().expect("m");
                        let _ = m.run_frame();
                        m.debugger()
                            .paused
                            .then_some((m.debugger().last_hit, m.cpu().regs.pc))
                    };
                    match hit {
                        Some((
                            BreakReason::Port {
                                port, write: false, ..
                            },
                            pc,
                        )) if port & 0xFF == 0xFE => {
                            poll_pc = Some((pc, "frame-port"));
                        }
                        Some((BreakReason::Pc(pc), _))
                            if matches!(pc, 0x8211 | 0x8229 | 0x8DC5) =>
                        {
                            poll_pc = Some((pc, "frame-pc"));
                        }
                        Some((BreakReason::Pc(0xF2AC), _)) => {
                            eprintln!("frame-EI $F2AC +{frames}f");
                        }
                        _ => {}
                    }
                }
                other => {
                    if i % 1000 == 0 {
                        eprintln!("other +{frames}f {other:?} pc={pc_now:#06x}");
                    }
                }
            }

            if let Some((pc, via)) = poll_pc {
                fe_events += 1;
                if let Some(e) = unique_fe.iter_mut().find(|(p, _)| *p == pc) {
                    e.1 += 1;
                } else {
                    unique_fe.push((pc, 1));
                }
                let in_gate = (0x820C..=0x822F).contains(&pc);
                let in_attract = pc == 0x8DC5;
                if fe_events <= max_fe || in_attract || (in_gate && fe_events <= 8) {
                    eprintln!(
                        "POLL#{fe_events} +{frames}f pc={pc:#06x} via={via} gate={in_gate} attract={in_attract} (sp)={sp_word:#06x} (sp+4)={ret4:#06x} $94C5={c5:#04x} $94EF={ef:#04x} $9595={s95:#04x} ($8403)={vec8403:#06x} stage={stage_now}"
                    );
                }
                // Fallback pulse if turbo miss (should be rare after #388 SP+4 fix).
                if in_gate && pc == 0x8229 && fe_events <= 4 && ret4 != 0xF3D4 {
                    let m = s.machine_mut().expect("m");
                    m.keyboard_mut().set_key(7, 0, true);
                    space_taps += 1;
                    eprintln!("gate Space pulse fallback (sp+4)={ret4:#06x}");
                    for _ in 0..16 {
                        if m.debugger().paused {
                            let p = m.cpu().regs.pc;
                            m.debugger_mut().continue_from_pc(p);
                            m.debugger_mut().paused = false;
                        }
                        let _ = m.run_until_break(64);
                        if m.cpu().regs.pc == 0x8230 || m.cpu().regs.iff1 {
                            break;
                        }
                    }
                    m.keyboard_mut().set_key(7, 0, false);
                }
                // When turbo owns the gate (sp+4 == $F3D4), run a Spectrum frame
                // so maybe_accelerate can inject Space across the IN.
                if in_gate && pc == 0x8229 && ret4 == 0xF3D4 && fe_events <= 8 {
                    let m = s.machine_mut().expect("m");
                    if m.debugger().paused {
                        let p = m.cpu().regs.pc;
                        m.debugger_mut().continue_from_pc(p);
                        m.debugger_mut().paused = false;
                    }
                    let _ = m.run_frame();
                    if m.cpu().regs.pc == 0x8230 || m.cpu().regs.iff1 {
                        eprintln!(
                            "turbo/frame left gate pc={:#06x} iff1={}",
                            m.cpu().regs.pc,
                            u8::from(m.cpu().regs.iff1)
                        );
                    }
                }
                // Fire pulse on attract `$8DC5` (shared `$8E2D` when `$94C5!=0`).
                if in_attract {
                    attract_polls += 1;
                    if attract_polls % pulse_every == 1 {
                        let m = s.machine_mut().expect("m");
                        m.keyboard_mut().set_key(7, 0, true);
                        m.keyboard_mut().set_key(4, 0, true);
                        space_taps += 1;
                        for _ in 0..24 {
                            if m.debugger().paused {
                                let p = m.cpu().regs.pc;
                                m.debugger_mut().continue_from_pc(p);
                                m.debugger_mut().paused = false;
                            }
                            let br = m.run_until_break(128);
                            if m.cpu().regs.iff1 {
                                eprintln!(
                                    "GAME ENTRY during fire pulse pc={:#06x}",
                                    m.cpu().regs.pc
                                );
                                break;
                            }
                            if matches!(br, BreakReason::Pc(0x8E2D | 0x8DF1 | 0xF2AC)) {
                                eprintln!("fire pulse hit {br:?} pc={:#06x}", m.cpu().regs.pc);
                                break;
                            }
                        }
                        m.keyboard_mut().set_key(7, 0, false);
                        m.keyboard_mut().set_key(4, 0, false);
                    }
                }
            }
            let _ = io::stderr().flush();
        }

        unique_fe.sort_by_key(|(pc, _)| *pc);
        eprintln!("--- unique key-poll PCs ---");
        for (pc, n) in &unique_fe {
            eprintln!("  {pc:#06x} ×{n}");
        }
        let m = s.machine().expect("m");
        eprintln!(
            "menu done frames={frames} fe={fe_events} attract={attract_polls} fire={fire_hits} taps={space_taps} $94C5_write={saw_94c5_write} pc={:#06x} iff1={} $94C5={:#04x} ($8403)={:#06x}",
            m.cpu().regs.pc,
            u8::from(m.cpu().regs.iff1),
            m.read_mem(0x94C5),
            word(m, 0x8403),
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
