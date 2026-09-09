//! Arkanoid paddle visibility probe (#379 play-smoke / bat redraw).
//!
//! User report: bat is briefly visible after game start, then vanishes once the
//! ball moves, while collision still works. Hypothesis: first ball XOR/blit or
//! background restore erases the bat without a correct redraw.
//!
//! ```bash
//! cargo run -p host_api --release \
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

/// Set bits per pixel row inside the playfield walls (cols 1..=22) for `y` in
/// `140..192` — the bat lives somewhere in this band and shows up as a wide run.
fn playfield_row_bits(m: &machine::Machine) -> Vec<(u8, u32)> {
    (140u8..192)
        .map(|y| {
            let bits = (1u8..=22)
                .map(|col| m.read_mem(screen_addr(col * 8, y)).count_ones())
                .sum();
            (y, bits)
        })
        .collect()
}

/// Longest horizontal run of non-zero bitmap bytes inside the walls, per row.
///
/// The bat is the only wide solid object near the floor, so `(y, col, len)` with
/// `len >= 3` is a good "bat is on screen" signal; the ball is 1–2 bytes wide.
fn widest_run(m: &machine::Machine) -> Option<(u8, u8, u8)> {
    let mut best: Option<(u8, u8, u8)> = None;
    for y in 150u8..192 {
        let mut run = 0u8;
        let mut start = 0u8;
        for col in 1u8..=22 {
            if m.read_mem(screen_addr(col * 8, y)) == 0 {
                run = 0;
                continue;
            }
            if run == 0 {
                start = col;
            }
            run += 1;
            if best.is_none_or(|(_, _, len)| run > len) {
                best = Some((y, start, run));
            }
        }
    }
    best.filter(|&(_, _, len)| len >= 3)
}

fn row_bits_summary(rows: &[(u8, u32)]) -> String {
    let mut s = String::new();
    for &(y, bits) in rows {
        if bits > 0 {
            let _ = write!(s, " y{y}={bits}");
        }
    }
    if s.is_empty() {
        s.push_str(" (empty)");
    }
    s
}

/// Longest run of rendered ink pixels on the bat's scanlines.
///
/// The playfield is white ink on bright-blue paper, so bat pixels have a strong
/// red/green component while the paper does not. The bat is the only ~32px solid
/// horizontal run down there, which separates it from band text. This measures
/// what the host actually displays, not what happens to be in memory at the frame
/// boundary (#379).
fn bat_ink_run(fb: &[u8], w: usize, h: usize, with_border: bool) -> u32 {
    let (ox, oy) = if with_border { (48usize, 48) } else { (0, 0) };
    let mut best = 0u32;
    for py in 182..191 {
        let y = oy + py;
        if y >= h {
            continue;
        }
        let mut run = 0u32;
        for px in 16..176 {
            let x = ox + px;
            if x >= w {
                continue;
            }
            let i = (y * w + x) * 4;
            if i + 2 < fb.len() && (fb[i] > 96 || fb[i + 1] > 96) {
                run += 1;
                best = best.max(run);
            } else {
                run = 0;
            }
        }
    }
    best
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

    // Phase 2: pure `run_frame` soak — the probe taps keys itself for the
    // `$8224` gate. Debugger port watches break that path.
    {
        let m = s.machine_mut().expect("m");
        m.debugger_mut().clear_breaks();
        m.debugger_mut().paused = false;
        // Start from a clean matrix; the loop below taps the gate.
        for row in 0..8usize {
            for bit in 0..5u8 {
                m.keyboard_mut().set_key(row, bit, false);
            }
        }
    }

    let mut saw_8df1 = false;
    let screen_act = env::var_os("SPEC_CHUM_SCREEN_ACT").is_some();
    let mut prev_screen = vec![0u8; 0x1B00];
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
            eprintln!("vec $8403=$8DF1 +{frames}f pc={pc:#06x} $94C4={c4:#06x}");
            // The bat is drawn in the pre-ball / serve-ready state and vanishes
            // once the ball is live, so hand over to the play loop straight
            // away (#379).
            if env::var_os("SPEC_CHUM_ARKANOID_AT_VEC").is_some() {
                let mut opts = m.tape_load_options();
                opts.speed = 1;
                m.set_tape_load_options(opts);
                return true;
            }
        }
        if arkanoid_game_entry(pc, v8403, c4, dd) {
            eprintln!(
                "GAME ENTRY +{frames}f pc={pc:#06x} $94C4={c4:#06x} $94DD={dd:#04x} ($8403)={v8403:#06x} iff1={}",
                u8::from(m.cpu().regs.iff1)
            );
            return true;
        }
        // `SPEC_CHUM_SCREEN_ACT=1`: is "the display file is static" a generic
        // stand-in for "still in the Speedlock delay"? Measured answer is no —
        // delay and game code interleave, and both show quiet frames (#379).
        let mut changed = 0usize;
        if screen_act {
            let screen: Vec<u8> = (0x4000u16..0x5B00).map(|a| m.read_mem(a)).collect();
            changed = prev_screen
                .iter()
                .zip(&screen)
                .filter(|(a, b)| a != b)
                .count();
            prev_screen = screen;
        }
        if frames.is_multiple_of(100) {
            eprintln!(
                "entry soak +{frames}f pc={pc:#06x} changed={changed} ($8403)={v8403:#06x} $94C4={c4:#06x}"
            );
        }
        let _ = io::stderr().flush();
    }
    eprintln!("no GAME ENTRY after {frames}f (saw_8df1={saw_8df1})");
    false
}

fn main() {
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
        load_tape(&mut s, &arkanoid_path());
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

    let mut rendered_bat_frames = 0u32;
    let mut min_ink = u32::MAX;
    let mut max_ink = 0u32;
    let mut prev_bat: Option<(u8, u8, u8)> = None;
    let mut prev_hash = 0u64;
    let mut prev_rgba = 0u32;
    let mut prev_bitmap = 0u32;
    let mut first_drop: Option<u32> = None;
    let mut peak_rgba = 0u32;

    // Who writes the bat char row? Arm write watches across `y = 184..191` and
    // report the PCs that hit, so the draw/erase routines can be disassembled.
    if env::var_os("SPEC_CHUM_BATWATCH").is_some() {
        {
            let m = s.machine_mut().expect("m");
            let mut watches = Vec::new();
            for y in 184u8..192 {
                for col in 0u8..32 {
                    watches.push(machine::Watch {
                        addr: screen_addr(col * 8, y),
                        read: false,
                        write: true,
                    });
                }
            }
            m.debugger_mut().mem_watches = watches;
            m.debugger_mut().paused = false;
        }
        let mut hits: std::collections::BTreeMap<u16, (u32, u16, u16)> =
            std::collections::BTreeMap::new();
        let mut frame = 0u32;
        let mut elapsed = 0u64;
        while frame < play_frames {
            let m = s.machine_mut().expect("m");
            let t0 = m.cpu().t;
            let _ = m.run_frame();
            if m.debugger().paused {
                if let machine::BreakReason::Mem { addr, value, .. } = m.debugger().last_hit {
                    let pc = m.cpu().regs.pc;
                    let e = hits.entry(pc).or_insert((0, addr, u16::from(value)));
                    e.0 += 1;
                }
                m.debugger_mut().paused = false;
                m.debugger_mut().last_hit = machine::BreakReason::None;
            }
            elapsed += m.cpu().t.saturating_sub(t0);
            frame = u32::try_from(elapsed / 69_888).unwrap_or(u32::MAX);
        }
        eprintln!("bat-row writers over {play_frames} frames:");
        for (pc, (n, addr, value)) in hits {
            eprintln!("  pc={pc:#06x} hits={n} last_addr={addr:#06x} last_value={value:#04x}");
        }
        return;
    }

    // Sub-frame sweep: step the CPU and sample the bat band far more often than
    // once per host frame, so a draw/erase pair that never survives to the frame
    // boundary still shows up (#379).
    if env::var_os("SPEC_CHUM_SUBFRAME").is_some() {
        // The beam paints paper line `y` at `14335 + y * 224`; row 184 (the bat)
        // is around frame T 55551. Record which frame-T windows actually hold the
        // bat so we can compare "on screen when scanned" against "on screen at
        // the frame-end render point" (#379).
        let bat_line_t = 14_335 + 184 * 224;
        for f in 0..play_frames {
            let mut on: Vec<u32> = Vec::new();
            let mut samples = 0u32;
            let m = s.machine_mut().expect("m");
            let t_end = m.cpu().t + 69_888;
            let mut at_bat_line: Option<bool> = None;
            while m.cpu().t < t_end {
                for _ in 0..16 {
                    m.step_once();
                }
                samples += 1;
                let frame_t = m.inspect().frame_t;
                let bat = widest_run(m).is_some();
                if bat {
                    on.push(frame_t);
                }
                if at_bat_line.is_none() && (bat_line_t..bat_line_t + 400).contains(&frame_t) {
                    at_bat_line = Some(bat);
                }
            }
            let span = on
                .iter()
                .fold((u32::MAX, 0u32), |(lo, hi), &t| (lo.min(t), hi.max(t)));
            eprintln!(
                "sub +{f}f samples={samples} bat_on={} span_t={}..{} at_bat_line={at_bat_line:?} end_bat={}",
                on.len(),
                if on.is_empty() { 0 } else { span.0 },
                span.1,
                widest_run(m).is_some(),
            );
        }
        return;
    }

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

        let mut bat_flip = false;
        {
            let m = s.machine().expect("m");
            let bat = widest_run(m);
            if bat.is_some() != prev_bat.is_some() {
                let rows = playfield_row_bits(m);
                eprintln!(
                    "BAT {} +{f}f pc={:#06x} run={bat:?} prev={prev_bat:?}{}",
                    if bat.is_some() { "ON " } else { "OFF" },
                    m.cpu().regs.pc,
                    row_bits_summary(&rows)
                );
                bat_flip = true;
            }
            prev_bat = bat;
        }

        let (bitmap_nz, attr_int, hash) = {
            let m = s.machine().expect("m");
            dump_bottom_band(m)
        };
        let fb = s.framebuffer();
        let (w, h) = (s.width(), s.height());
        let rgba_nz = count_bat_rgba(fb, w, h, true);
        peak_rgba = peak_rgba.max(rgba_nz);
        let ink = bat_ink_run(fb, w, h, true);
        if ink >= 20 {
            rendered_bat_frames += 1;
        }
        min_ink = min_ink.min(ink);
        max_ink = max_ink.max(ink);

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
            || bat_flip
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
            if f <= 10 || big_drop || bat_flip {
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

    eprintln!(
        "RENDERED BAT: {rendered_bat_frames}/{play_frames} frames render a >=20px bat run (run {}..{})",
        if min_ink == u32::MAX { 0 } else { min_ink },
        max_ink
    );
    if let Some(f) = first_drop {
        eprintln!("RESULT: first bat-band drop around play frame {f} (peak_rgba={peak_rgba})");
    } else {
        eprintln!(
            "RESULT: no clear bat-band drop in {play_frames} frames (rgba_nz={prev_rgba} peak={peak_rgba})"
        );
    }
}
