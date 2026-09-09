//! Arkanoid Speedlock outer-stub tracer (#379).
//!
//! Breaks on the `$F3CE` outer stub / `$8224` key gate so the *decision* points
//! are observed exactly (frame sampling only ever caught the delay nest). Dumps
//! live disassembly of the stub, gate, and `$93xx` decrypt so the loop can be
//! reasoned about from real decrypted bytes rather than guesses.
//!
//! ```bash
//! cargo run -p host_api --release --example arkanoid_stub_trace -- [tzx] [max_breaks]
//! ```

use machine::TapeLoadOptions;
use spec_chum_host::{HostSession, ModelId};
use std::env;
use std::io::{self, Write};
use std::path::PathBuf;

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

fn dump(s: &HostSession, label: &str, addr: u16, count: usize) {
    match s.disasm(Some(addr), count) {
        Ok(text) => eprintln!("--- {label} @ {addr:#06x} ---\n{text}"),
        Err(e) => eprintln!("--- {label} @ {addr:#06x} FAILED: {e} ---"),
    }
}

fn main() {
    let tape = arkanoid_path();
    let max_breaks: u32 = env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(40);

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
    for i in 0..40_000u32 {
        let _ = s.machine_mut().expect("m").run_frame();
        let m = s.machine().expect("m");
        if m.tape_finished() || !m.tape_playing() {
            eprintln!("tape end +{i}");
            break;
        }
    }

    // Decision points around the outer stub: post-delay RET, post-gate RET,
    // the `INC E` / `JR Z` re-loop test, and the gate abort/continue split.
    let breaks = [
        0xF3CEu16, 0xF3D1, 0xF3D4, 0xF3D5, 0xF3D7, 0xF3D8, 0x8224, 0x8228, 0x822A, 0x8230,
    ];
    {
        let m = s.machine_mut().expect("m");
        m.debugger_mut().pc_breaks = breaks.to_vec();
    }

    dump(&s, "outer stub", 0xF3C8, 20);
    dump(&s, "gate entry", 0x8210, 16);
    dump(&s, "key gate", 0x8224, 30);
    dump(&s, "decrypt93", 0x9360, 24);
    dump(&s, "sig-miss path", 0x8280, 26);
    dump(&s, "sig-hit helper", 0x8368, 20);
    dump(&s, "outer caller", 0x8138, 12);
    {
        let m = s.machine().expect("m");
        let word = |a: u16| u16::from(m.read_mem(a)) | (u16::from(m.read_mem(a + 1)) << 8);
        let sig_ptr = word(0x94F6);
        let hex = |base: u16, n: u16| {
            (0..n)
                .map(|i| format!("{:02x}", m.read_mem(base.wrapping_add(i))))
                .collect::<Vec<_>>()
                .join(" ")
        };
        eprintln!("--- $8230 signature check ---");
        eprintln!(
            "  ($94F6)={sig_ptr:#06x} → HL={:#06x}",
            sig_ptr.wrapping_add(1)
        );
        eprintln!("  expected $9570: {}", hex(0x9570, 6));
        eprintln!(
            "  actual  {:#06x}: {}",
            sig_ptr.wrapping_add(1),
            hex(sig_ptr.wrapping_add(1), 6)
        );
        eprintln!(
            "  flag $956F={:#04x}  SP src ($94C8)={:#06x}",
            m.read_mem(0x956F),
            word(0x94C8)
        );
        eprintln!(
            "  around {:#06x}: {}",
            sig_ptr.wrapping_sub(8),
            hex(sig_ptr.wrapping_sub(8), 24)
        );
        let sig: [u8; 6] = [0x50, 0x42, 0x52, 0x41, 0x49, 0x4E];
        let mut found = Vec::new();
        for a in 0x4000u32..=0xFFFA {
            let a = a as u16;
            if (0..6).all(|i| m.read_mem(a.wrapping_add(i)) == sig[usize::from(i)]) {
                found.push(format!("{a:#06x}"));
            }
        }
        eprintln!(
            "  \"PBRAIN\" found at: {}",
            if found.is_empty() {
                "<nowhere>".into()
            } else {
                found.join(", ")
            }
        );
        // How much of the $4000-$FFFF image is still zero (never written)?
        let zeros = (0x4000u32..=0xFFFF)
            .filter(|&a| m.read_mem(a as u16) == 0)
            .count();
        eprintln!("  zero bytes in $4000-$FFFF: {zeros}");
        eprintln!("--- all-zero 256-byte pages ---");
        let mut run: Option<u16> = None;
        for p in 0x40u16..=0x100 {
            let all_zero =
                p < 0x100 && (0..256u16).all(|o| m.read_mem((p << 8).wrapping_add(o)) == 0);
            match (all_zero, run) {
                (true, None) => run = Some(p),
                (false, Some(start)) => {
                    eprintln!(
                        "  {:#06x}-{:#06x} zero",
                        start << 8,
                        (p << 8).wrapping_sub(1)
                    );
                    run = None;
                }
                _ => {}
            }
        }
    }

    // `max_breaks == 0` → cursor mode: no breakpoints, just watch how the
    // `$8230` compare pointer at `($94F6)` moves relative to the `"PBRAIN"`
    // table at `$9570`.
    if max_breaks == 0 {
        s.machine_mut().expect("m").debugger_mut().clear_breaks();
        let track: u32 = env::var("SPEC_CHUM_TRACK_FRAMES")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(20_000);
        let mut prev = 0u16;
        for i in 0..track {
            let _ = s.machine_mut().expect("m").run_frame();
            let m = s.machine().expect("m");
            let ptr = u16::from(m.read_mem(0x94F6)) | (u16::from(m.read_mem(0x94F7)) << 8);
            if i % 25 == 0 || m.cpu().regs.iff1 {
                eprintln!(
                    "+{i}f ($94F6)={ptr:#06x} delta={} to_9570={} pc={:#06x} iff1={}",
                    ptr.wrapping_sub(prev) as i16,
                    0x956F_i32 - i32::from(ptr),
                    m.cpu().regs.pc,
                    u8::from(m.cpu().regs.iff1),
                );
                prev = ptr;
                let _ = io::stderr().flush();
            }
            if m.cpu().regs.iff1 {
                eprintln!("GAME ENTRY at +{i}f");
                break;
            }
        }
        return;
    }

    let mut hits = 0u32;
    let mut frames = 0u32;
    while hits < max_breaks && frames < 60_000 {
        {
            let m = s.machine_mut().expect("m");
            let at = m.cpu().regs.pc;
            m.debugger_mut().continue_from_pc(at);
            let _ = m.run_frame();
            frames += 1;
        }
        let m = s.machine().expect("m");
        if !m.debugger().paused {
            continue;
        }
        hits += 1;
        let r = &m.cpu().regs;
        let sp = r.sp;
        let ret = u16::from(m.read_mem(sp)) | (u16::from(m.read_mem(sp.wrapping_add(1))) << 8);
        eprintln!(
            "hit#{hits} +{frames}f pc={:#06x} af={:#06x} bc={:#06x} de={:#06x} hl={:#06x} sp={sp:#06x} ret={ret:#06x} iff1={} r={:#04x}",
            r.pc,
            r.af(),
            r.bc(),
            r.de(),
            r.hl(),
            u8::from(r.iff1),
            r.r,
        );
        let _ = io::stderr().flush();
    }

    let m = s.machine().expect("m");
    eprintln!(
        "summary: hits={hits} frames={frames} pc={:#06x} iff1={}",
        m.cpu().regs.pc,
        u8::from(m.cpu().regs.iff1)
    );
}
