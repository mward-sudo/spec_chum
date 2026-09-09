//! Arkanoid paddle visibility probe (#379 play-smoke / bat redraw).
//!
//! User report: bat is briefly visible after game start, then vanishes once the
//! ball moves, while collision still works. Hypothesis: first ball XOR/blit or
//! background restore erases the bat without a correct redraw.
//!
//! ```bash
//! SPEC_CHUM_SPEEDLOCK_BOOST=16 cargo run -p host_api --release \
//!   --example arkanoid_paddle_probe -- ~/Downloads/Arkanoid.tzx
//! ```
//!
//! Writes PPM frames under `tmp/arkanoid_paddle/` when bat-band pixel counts jump.
//!
//! Reaching game entry costs minutes of Speedlock delay, so the probe can cache
//! the post-gate machine as a 48K `.sna` and replay from it:
//!
//! ```bash
//! # capture once (writes tmp/arkanoid_play.sna)
//! SPEC_CHUM_ARKANOID_SNA=tmp/arkanoid_play.sna cargo run -p host_api --release \
//!   --example arkanoid_paddle_probe
//! # then iterate in seconds
//! SPEC_CHUM_ARKANOID_FROM_SNA=tmp/arkanoid_play.sna cargo run -p host_api --release \
//!   --example arkanoid_paddle_probe
//! ```
//!
//! Snapshots of a commercial tape are debug artefacts — keep them out of git.

use machine::TapeLoadOptions;
use spec_chum_host::{HostSession, ModelId};
use std::env;
use std::fmt::Write as _;
use std::fs;
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

fn arkanoid_game_entry(pc: u16, vec8403: u16, c4: u16, dd: u8) -> bool {
    let in_main = (0x83DF..=0x83FF).contains(&pc) || (0x8410..=0x8428).contains(&pc);
    vec8403 == 0x8DF1 && in_main && (c4 != 0 || dd != 0)
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

/// Spectrum screen address for pixel (x,y) in the 256×192 paper.
fn screen_addr(x: u8, y: u8) -> u16 {
    let y = u16::from(y);
    let third = (y / 64) * 0x0800;
    let line = (y % 8) * 0x0100;
    let row = ((y % 64) / 8) * 0x0020;
    0x4000 + third + line + row + u16::from(x / 8)
}

fn dump_bottom_band(m: &machine::Machine) -> (u32, u32, u64) {
    // Bat sits near the playfield floor — scan last ~24 pixel rows + attrs.
    let mut bitmap_nz = 0u32;
    let mut attr_interesting = 0u32;
    let mut hash = 0u64;
    for y in 168u8..=191 {
        for col in 0u8..32 {
            let a = screen_addr(col * 8, y);
            let b = m.read_mem(a);
            hash = hash.wrapping_mul(131).wrapping_add(u64::from(b));
            if b != 0 {
                bitmap_nz += b.count_ones();
            }
        }
    }
    // Attr rows 21–23 (y 168–191).
    for row in 21u16..=23 {
        for col in 0u16..32 {
            let a = 0x5800 + row * 32 + col;
            let b = m.read_mem(a);
            hash = hash.wrapping_mul(131).wrapping_add(u64::from(b));
            let ink = b & 7;
            let paper = (b >> 3) & 7;
            if ink != paper {
                attr_interesting += 1;
            }
        }
    }
    (bitmap_nz, attr_interesting, hash)
}

fn count_bat_rgba(fb: &[u8], w: usize, h: usize, with_border: bool) -> u32 {
    // Paper-relative band: y 168..191. With border, paper origin is (48,48).
    let (ox, oy) = if with_border { (48usize, 48) } else { (0, 0) };
    let mut n = 0u32;
    for py in 168..192 {
        let y = oy + py;
        if y >= h {
            continue;
        }
        for px in 0..256 {
            let x = ox + px;
            if x >= w {
                continue;
            }
            let i = (y * w + x) * 4;
            if i + 2 >= fb.len() {
                continue;
            }
            // Non-black-ish pixel (any channel above near-black).
            if fb[i] > 16 || fb[i + 1] > 16 || fb[i + 2] > 16 {
                n += 1;
            }
        }
    }
    n
}

fn write_ppm(path: &Path, fb: &[u8], w: usize, h: usize) -> io::Result<()> {
    let mut out = Vec::with_capacity(64 + w * h * 3);
    out.extend_from_slice(format!("P6\n{w} {h}\n255\n").as_bytes());
    for px in fb.as_chunks::<4>().0 {
        out.extend_from_slice(&px[..3]);
    }
    fs::write(path, out)
}

/// Serialise the live 48K machine as a `.sna` so later runs can skip Speedlock.
///
/// SNA takes PC from the stack, so this pushes PC below SP (clobbers 2 bytes of
/// guest RAM — acceptable for a debug capture).
fn write_sna(m: &mut machine::Machine, path: &Path) -> io::Result<()> {
    let sp = m.cpu().regs.sp.wrapping_sub(2);
    let pc = m.cpu().regs.pc;
    m.write_mem(sp, (pc & 0xff) as u8);
    m.write_mem(sp.wrapping_add(1), (pc >> 8) as u8);
    let r = m.cpu().regs;
    let mut out = Vec::with_capacity(49179);
    out.push(r.i);
    out.extend_from_slice(&u16::from_le_bytes([r.l_, r.h_]).to_le_bytes());
    out.extend_from_slice(&u16::from_le_bytes([r.e_, r.d_]).to_le_bytes());
    out.extend_from_slice(&u16::from_le_bytes([r.c_, r.b_]).to_le_bytes());
    out.push(r.a_);
    out.push(r.f_);
    out.extend_from_slice(&u16::from_le_bytes([r.l, r.h]).to_le_bytes());
    out.extend_from_slice(&u16::from_le_bytes([r.e, r.d]).to_le_bytes());
    out.extend_from_slice(&u16::from_le_bytes([r.c, r.b]).to_le_bytes());
    out.extend_from_slice(&u16::from_le_bytes([r.iyl, r.iyh]).to_le_bytes());
    out.extend_from_slice(&u16::from_le_bytes([r.ixl, r.ixh]).to_le_bytes());
    out.push(if r.iff2 { 0x04 } else { 0 });
    out.push(r.r);
    out.push(r.a);
    out.push(r.f);
    out.extend_from_slice(&sp.to_le_bytes());
    out.push(r.im);
    out.push(0); // border (cosmetic)
    for a in 0x4000..=0xffffu32 {
        out.push(m.read_mem(a as u16));
    }
    fs::write(path, out)
}

fn row_hex(m: &machine::Machine, y: u8) -> String {
    let mut s = String::with_capacity(64);
    for col in 0u8..32 {
        let _ = write!(s, "{:02x}", m.read_mem(screen_addr(col * 8, y)));
    }
    s
}

fn attr_row_hex(m: &machine::Machine, row: u16) -> String {
    let mut s = String::with_capacity(96);
    for col in 0u16..32 {
        if col > 0 {
            s.push(' ');
        }
        let _ = write!(s, "{:02x}", m.read_mem(0x5800 + row * 32 + col));
    }
    s
}

fn reach_game_entry(s: &mut HostSession) -> bool {
    let track: u32 = env::var("SPEC_CHUM_TRACK_FRAMES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8_000);

    // Phase 1: title draw (PC break only — no port watches).
    {
        let m = s.machine_mut().expect("m");
        m.debugger_mut().pc_breaks = vec![0x808E];
    }
    let mut frames = 0u32;
    let mut saw_title = false;
    while frames < track && !saw_title {
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
        frames += 1;
    }
    if !saw_title {
        eprintln!("never hit $808E in {frames}f");
        return false;
    }

    // Phase 2: pure `run_frame` soak so Speedlock turbo can inject Space across
    // the `$8224` IN (#388). Debugger port watches break that path.
    {
        let m = s.machine_mut().expect("m");
        m.debugger_mut().clear_breaks();
        m.debugger_mut().paused = false;
        // Never hold keys through `$F448` — release everything; turbo owns the gate.
        for row in 0..8usize {
            for bit in 0..5u8 {
                m.keyboard_mut().set_key(row, bit, false);
            }
        }
    }

    let mut saw_8df1 = false;
    let phase2 = track.saturating_sub(frames).max(4_000);
    for i in 0..phase2 {
        let m = s.machine_mut().expect("m");
        // Brief fire pulse only when already past the title vector (not in delay).
        let pc_before = m.cpu().regs.pc;
        let in_delay = (0xF400..=0xF4FF).contains(&pc_before);
        if !in_delay && i.is_multiple_of(48) {
            m.keyboard_mut().set_key(7, 0, true);
            m.keyboard_mut().set_key(4, 0, true);
        } else if in_delay || i % 48 == 6 {
            m.keyboard_mut().set_key(7, 0, false);
            m.keyboard_mut().set_key(4, 0, false);
        }
        let _ = m.run_frame();
        frames += 1;
        let pc = m.cpu().regs.pc;
        let v8403 = word(m, 0x8403);
        let c4 = word(m, 0x94C4);
        let dd = m.read_mem(0x94DD);
        if v8403 == 0x8DF1 && !saw_8df1 {
            saw_8df1 = true;
            eprintln!(
                "vec $8403=$8DF1 +{frames}f pc={pc:#06x} $94C4={c4:#06x} stage={}",
                m.speedlock_stage().as_str()
            );
        }
        if arkanoid_game_entry(pc, v8403, c4, dd) {
            eprintln!(
                "GAME ENTRY +{frames}f pc={pc:#06x} $94C4={c4:#06x} $94DD={dd:#04x} ($8403)={v8403:#06x} iff1={}",
                u8::from(m.cpu().regs.iff1)
            );
            return true;
        }
        if frames.is_multiple_of(500) {
            eprintln!(
                "entry soak +{frames}f pc={pc:#06x} ($8403)={v8403:#06x} $94C4={c4:#06x} $94DD={dd:#04x} stage={}",
                m.speedlock_stage().as_str()
            );
        }
        let _ = io::stderr().flush();
    }
    eprintln!("no GAME ENTRY after {frames}f (saw_8df1={saw_8df1})");
    false
}

fn main() {
    let tape = arkanoid_path();
    let out_dir = workspace_root().join("tmp/arkanoid_paddle");
    fs::create_dir_all(&out_dir).expect("outdir");

    let rom = fs::read(workspace_root().join("roms/spec48.rom")).expect("rom");
    let mut s = HostSession::new(ModelId::Spectrum48, true);
    s.load_rom_bytes(&rom).expect("rom");

    if let Some(snap) = env::var_os("SPEC_CHUM_ARKANOID_FROM_SNA") {
        let snap = PathBuf::from(snap);
        s.load_snapshot(&snap).expect("snapshot");
        let m = s.machine().expect("m");
        eprintln!(
            "replay from {} pc={:#06x} iff1={} ($8403)={:#06x}",
            snap.display(),
            m.cpu().regs.pc,
            u8::from(m.cpu().regs.iff1),
            word(m, 0x8403)
        );
    } else {
        load_tape(&mut s, &tape);
        if !reach_game_entry(&mut s) {
            return;
        }
        if let Some(dst) = env::var_os("SPEC_CHUM_ARKANOID_SNA") {
            let dst = PathBuf::from(dst);
            let m = s.machine_mut().expect("m");
            write_sna(m, &dst).expect("sna");
            eprintln!("wrote {}", dst.display());
        }
    }

    {
        let m = s.machine_mut().expect("m");
        m.debugger_mut().clear_breaks();
        m.debugger_mut().paused = false;
        for row in 0..8usize {
            for bit in 0..5u8 {
                m.keyboard_mut().set_key(row, bit, false);
            }
        }
    }

    let play_frames: u32 = env::var("SPEC_CHUM_PLAY_FRAMES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(400);

    let mut prev_hash = 0u64;
    let mut prev_rgba = 0u32;
    let mut prev_bitmap = 0u32;
    let mut first_drop: Option<u32> = None;
    let mut peak_rgba = 0u32;

    for f in 0..play_frames {
        {
            let m = s.machine_mut().expect("m");
            // Nudge bat left/right occasionally so collision path stays live.
            if f.is_multiple_of(30) {
                m.keyboard_mut().set_key(3, 4, true); // 5 / left-ish
            } else if f % 30 == 8 {
                m.keyboard_mut().set_key(3, 4, false);
            }
            if f % 30 == 15 {
                m.keyboard_mut().set_key(4, 2, true); // 8 / right-ish — may be wrong mapping
            } else if f % 30 == 22 {
                m.keyboard_mut().set_key(4, 2, false);
            }
            let _ = m.run_frame();
        }
        s.refresh_framebuffer();

        let (bitmap_nz, attr_int, hash) = {
            let m = s.machine().expect("m");
            dump_bottom_band(m)
        };
        let fb = s.framebuffer();
        let (w, h) = (s.width(), s.height());
        let rgba_nz = count_bat_rgba(fb, w, h, true);
        peak_rgba = peak_rgba.max(rgba_nz);

        let big_drop = prev_rgba > 80 && rgba_nz + 40 < prev_rgba
            || prev_bitmap > 80 && bitmap_nz + 40 < prev_bitmap;
        let changed = hash != prev_hash || rgba_nz != prev_rgba;
        if f == 0
            || f == 1
            || f == 2
            || f == 5
            || f == 10
            || f % 25 == 0
            || changed && f < 80
            || big_drop
        {
            let m = s.machine().expect("m");
            eprintln!(
                "play +{f}f pc={:#06x} iff1={} bitmap_nz={bitmap_nz} attr_int={attr_int} rgba_nz={rgba_nz} $94C4={:#06x} $94DD={:#04x} $94DB={:#04x} $94DC={:#04x} $94D8={:#04x} $94D9={:#04x}",
                m.cpu().regs.pc,
                u8::from(m.cpu().regs.iff1),
                word(m, 0x94C4),
                m.read_mem(0x94DD),
                m.read_mem(0x94DB),
                m.read_mem(0x94DC),
                m.read_mem(0x94D8),
                m.read_mem(0x94D9),
            );
            if f <= 10 || big_drop {
                eprintln!("  y176: {}", row_hex(m, 176));
                eprintln!("  y184: {}", row_hex(m, 184));
                eprintln!("  y188: {}", row_hex(m, 188));
                eprintln!("  attr21: {}", attr_row_hex(m, 21));
                eprintln!("  attr22: {}", attr_row_hex(m, 22));
                eprintln!("  attr23: {}", attr_row_hex(m, 23));
                let path = out_dir.join(format!("play_{f:04}.ppm"));
                write_ppm(&path, fb, w, h).expect("ppm");
                eprintln!("  wrote {}", path.display());
            }
        }

        if first_drop.is_none() && big_drop {
            first_drop = Some(f);
            eprintln!(
                "BAT BAND DROP at +{f}f rgba {prev_rgba}->{rgba_nz} bitmap {prev_bitmap}->{bitmap_nz}"
            );
        }

        prev_hash = hash;
        prev_rgba = rgba_nz;
        prev_bitmap = bitmap_nz;
    }

    if let Some(f) = first_drop {
        eprintln!("RESULT: first bat-band drop around play frame {f} (peak_rgba={peak_rgba})");
    } else {
        eprintln!(
            "RESULT: no clear bat-band drop in {play_frames} frames (rgba_nz={prev_rgba} peak={peak_rgba})"
        );
    }
}
