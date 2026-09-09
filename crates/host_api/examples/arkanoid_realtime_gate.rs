//! Is the post-tape Speedlock delay genuinely long, or is it a key wait? (#390)
//!
//! Loads Arkanoid over the EAR path with tape turbo (that part is legitimate —
//! the deck is playing), then drops to a **true 1x** (`speed = 1`, so no
//! post-tape multiplier can apply) and steps instructions, measuring Spectrum
//! time in T-states between protection landmarks.
//!
//! A `--tap-ms` cadence simulates a human tapping Space so the `$8224` any-key
//! gate can be answered the way it is on real hardware, instead of relying on
//! the emulator's turbo auto-ack.
//!
//! ```bash
//! cargo run -p host_api --release --example arkanoid_realtime_gate -- \
//!   ~/Downloads/Arkanoid.tzx
//! ```

use machine::TapeLoadOptions;
use spec_chum_host::{HostSession, ModelId};
use std::collections::BTreeMap;
use std::env;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const T_PER_FRAME: u64 = 69_888;

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

fn env_u64(key: &str, default: u64) -> u64 {
    env::var(key)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

fn secs(t: Option<u64>) -> String {
    t.map_or_else(
        || "never".to_string(),
        |t| format!("{:.2}s", t as f64 / (T_PER_FRAME * 50) as f64),
    )
}

fn load_tape(s: &mut HostSession, tape: &Path) {
    s.open_tape(tape).expect("open tape");
    {
        let m = s.machine_mut().expect("machine");
        m.set_tape_load_options(TapeLoadOptions {
            flash_load: false,
            speed: 64,
            experience_load: false,
        });
        m.set_tape_playing(false);
    }
    for _ in 0..200 {
        let _ = s.machine_mut().expect("machine").run_frame();
    }
    {
        let m = s.machine_mut().expect("machine");
        m.type_load_quotes(false);
        m.set_tape_playing(true);
    }
    for i in 0..40_000u32 {
        let _ = s.machine_mut().expect("machine").run_frame();
        let m = s.machine().expect("machine");
        if m.tape_finished() || !m.tape_playing() {
            eprintln!("tape end after {i} host frames");
            return;
        }
    }
    panic!("tape never finished");
}

/// Protection landmarks worth timestamping (decoded in #379/#385).
fn landmark(pc: u16) -> Option<&'static str> {
    match pc {
        0xF3CD => Some("F3CD outer stub DI"),
        0xF408 => Some("F408 delay entry"),
        0xF448 => Some("F448 delay inner"),
        0xF476 => Some("F476 delay RET"),
        0x8224 => Some("8224 any-key gate"),
        0x8230 => Some("8230 gate passed"),
        0x824A => Some("824A stack switch"),
        0x8280 => Some("8280 miss / wipe"),
        0x808E => Some("808E title draw"),
        0x83DF => Some("83DF main loop"),
        _ => None,
    }
}

fn main() {
    let tape = arkanoid_path();
    // Spectrum seconds to observe after the deck finishes.
    let budget_frames = env_u64("SPEC_CHUM_GATE_FRAMES", 2_500);
    // Human-ish tap: press Space for `hold` ms every `period` ms of Spectrum time.
    let tap_period_ms = env_u64("SPEC_CHUM_GATE_TAP_MS", 700);
    let tap_hold_ms = env_u64("SPEC_CHUM_GATE_HOLD_MS", 120);
    let taps_enabled = tap_period_ms > 0;

    let rom = std::fs::read(workspace_root().join("roms/spec48.rom")).expect("48K ROM");
    let mut s = HostSession::new(ModelId::Spectrum48, true);
    s.load_rom_bytes(&rom).expect("rom");
    load_tape(&mut s, &tape);

    // `speed = 1` is a true 1x (no EAR / post-tape multiplier can apply); any
    // higher value keeps the post-tape turbo gate live for an A/B.
    let post_speed = env_u64("SPEC_CHUM_GATE_POST_SPEED", 1).clamp(1, 64) as u32;
    let m = s.machine_mut().expect("machine");
    m.set_tape_load_options(TapeLoadOptions {
        flash_load: false,
        speed: post_speed,
        experience_load: false,
    });

    let t0 = m.cpu().t;
    let budget_t = budget_frames * T_PER_FRAME;
    let mut first_seen: BTreeMap<u16, u64> = BTreeMap::new();
    let mut hits: BTreeMap<u16, u64> = BTreeMap::new();
    let mut space_held = false;
    let mut key_presses = 0u64;
    let mut iff1_at: Option<u64> = None;
    let mut vec_8df1_at: Option<u64> = None;
    let mut main_loop_at: Option<u64> = None;

    eprintln!(
        "soak at {post_speed}x: {budget_frames} Spectrum frames ({:.1}s), tap {tap_hold_ms}ms every {tap_period_ms}ms",
        budget_frames as f64 / 50.0
    );

    loop {
        let m = s.machine_mut().expect("machine");
        let elapsed = m.cpu().t.saturating_sub(t0);
        if elapsed >= budget_t {
            break;
        }

        if taps_enabled {
            let ms = elapsed * 1000 / (T_PER_FRAME * 50);
            let want = ms % tap_period_ms < tap_hold_ms;
            if want != space_held {
                m.keyboard_mut().set_key(7, 0, want);
                space_held = want;
                if want {
                    key_presses += 1;
                }
            }
        }

        if post_speed > 1 {
            // Turbo only applies through `run_frame`, so the A/B has to use it
            // (PC is then sampled per Spectrum frame, not per instruction).
            let _ = m.run_frame();
        } else {
            m.step_once();
        }
        let pc = m.cpu().regs.pc;
        if landmark(pc).is_some() {
            *hits.entry(pc).or_default() += 1;
            first_seen.entry(pc).or_insert(elapsed);
        }
        if m.cpu().regs.iff1 && iff1_at.is_none() {
            iff1_at = Some(elapsed);
            eprintln!(
                "IFF1 set at +{:.2}s (pc={pc:#06x})",
                elapsed as f64 / (T_PER_FRAME * 50) as f64
            );
        }
        // Game-entry markers from #386: post-gate vector + main loop PC.
        if vec_8df1_at.is_none()
            && u16::from(m.read_mem(0x8403)) | (u16::from(m.read_mem(0x8404)) << 8) == 0x8DF1
        {
            vec_8df1_at = Some(elapsed);
            eprintln!(
                "($8403)=$8DF1 at +{:.2}s (pc={pc:#06x})",
                elapsed as f64 / (T_PER_FRAME * 50) as f64
            );
        }
        if main_loop_at.is_none()
            && ((0x83DF..=0x83FF).contains(&pc) || (0x8410..=0x8428).contains(&pc))
        {
            main_loop_at = Some(elapsed);
            eprintln!(
                "main loop at +{:.2}s (pc={pc:#06x})",
                elapsed as f64 / (T_PER_FRAME * 50) as f64
            );
        }
    }

    let m = s.machine().expect("machine");
    let elapsed = m.cpu().t.saturating_sub(t0);
    eprintln!(
        "\nafter {:.2}s Spectrum time ({} taps): pc={:#06x} iff1={} iff1_at={}",
        elapsed as f64 / (T_PER_FRAME * 50) as f64,
        key_presses,
        m.cpu().regs.pc,
        u8::from(m.cpu().regs.iff1),
        secs(iff1_at),
    );
    eprintln!(
        "markers: ($8403)=$8DF1 {} / main loop {}",
        secs(vec_8df1_at),
        secs(main_loop_at),
    );
    eprintln!("landmark            first-seen(s)   hits");
    for (pc, first) in &first_seen {
        eprintln!(
            "  {:<22} {:>9.3}   {}",
            landmark(*pc).unwrap_or("?"),
            *first as f64 / (T_PER_FRAME * 50) as f64,
            hits.get(pc).copied().unwrap_or(0),
        );
    }
    let _ = io::stderr().flush();
}
