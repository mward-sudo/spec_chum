//! Headless room tick budget check.
//!
//! SpecChumMac presents via IOSurface (no CPU map), so the gate times ticks with the
//! readback path disabled via [`HeadlessRoom::set_simulate_present_path`]. Timing the
//! readback instead adds a blocking `map_async` plus two multi-megabyte copies per
//! frame — that mismeasurement is why bloom and MSAA once looked ruinously expensive.
//! A short readback phase still runs first, to prove the render produced real pixels.
//!
//! Usage: `room_perf [width] [height]` — defaults to the shipped embed size.

use std::env;
use std::time::Instant;

use spec_chum_room::crt::{SCREEN_H, SCREEN_W};
use spec_chum_room::HeadlessRoom;

fn percentile_ms(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn summarize(label: &str, times: &[std::time::Duration]) -> (f64, f64, f64) {
    let mut ms: Vec<f64> = times.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
    let avg = ms.iter().sum::<f64>() / ms.len() as f64;
    let max = ms.iter().copied().fold(0.0_f64, f64::max);
    let min = ms.iter().copied().fold(f64::INFINITY, f64::min);
    ms.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let p50 = percentile_ms(&ms, 0.50);
    let p95 = percentile_ms(&ms, 0.95);
    eprintln!(
        "  {label}: n={} avg={avg:.2} ms  p50={p50:.2} ms  p95={p95:.2} ms  min={min:.2} ms  max={max:.2} ms",
        times.len()
    );
    (avg, p50, p95)
}

/// Vary the Spectrum framebuffer per frame so nothing can cache a static result.
fn varying_frame(base: &[u8], i: usize) -> Vec<u8> {
    let mut frame = base.to_vec();
    let c = (i as u8).wrapping_mul(17).saturating_add(80);
    for px in frame.as_chunks_mut::<4>().0 {
        px[0] = c;
        px[1] = c.wrapping_add(20);
        px[2] = c.wrapping_add(40);
        px[3] = 255;
    }
    frame
}

fn main() {
    let mut args = env::args().skip(1);
    let w: u32 = args
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(spec_chum_room::headless::DEFAULT_ROOM_W)
        .max(64);
    let h: u32 = args
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(spec_chum_room::headless::DEFAULT_ROOM_H)
        .max(64);

    eprintln!("living_room perf: creating HeadlessRoom {w}×{h}…");
    eprintln!("  quality: {}", spec_chum_room::quality::preset_label());
    let t0 = Instant::now();
    let mut room = HeadlessRoom::new(w, h);
    room.request_skip_intro();
    let create_ms = t0.elapsed().as_secs_f64() * 1000.0;
    eprintln!("  create: {create_ms:.1} ms");

    let fb = vec![40u8; (SCREEN_W * SCREEN_H * 4) as usize];
    let mut out = vec![0u8; (w * h * 4) as usize];

    // Warm until painted, then settle on tick-only timing.
    let mut warmed = false;
    let mut settle: Vec<f64> = Vec::new();
    for i in 0..240 {
        room.set_framebuffer(&fb);
        let start = Instant::now();
        room.tick();
        let tick_ms = start.elapsed().as_secs_f64() * 1000.0;
        let n = room.copy_frame_rgba(&mut out);
        if !warmed && n == out.len() && out.iter().any(|&b| b > 0) {
            warmed = true;
            eprintln!("  warm frame {i}: non-black readback ({n} bytes, tick {tick_ms:.1} ms)");
        }
        if warmed {
            settle.push(tick_ms);
            if settle.len() >= 20 {
                let window = &settle[settle.len() - 20..];
                let avg = window.iter().sum::<f64>() / window.len() as f64;
                let max = window.iter().copied().fold(0.0_f64, f64::max);
                if avg < 28.0 && max < 60.0 {
                    eprintln!(
                        "  settled after {i} frames (tick last-20 avg={avg:.2} ms max={max:.2} ms)"
                    );
                    break;
                }
            }
        }
    }
    if !warmed {
        eprintln!("FAIL: never received a non-black room frame (render path broken)");
        std::process::exit(1);
    }

    // Warmup above verified real pixels via CPU readback. Time the *shipping* path:
    // no readback, non-blocking poll. Otherwise every sample includes a blocking
    // `map_async` plus two multi-megabyte copies and the render cost is unreadable.
    let mut readback_times = Vec::with_capacity(100);
    let mut last_checksum = 0u64;
    let mut checksum_changed = false;
    for i in 0..20 {
        room.set_framebuffer(&varying_frame(&fb, i));
        let t = Instant::now();
        room.tick();
        let n = room.copy_frame_rgba(&mut out);
        readback_times.push(t.elapsed());
        let sum: u64 = out.iter().map(|&b| u64::from(b)).sum();
        if i == 0 {
            eprintln!("  first timed copy bytes={n} checksum={sum}");
            last_checksum = sum;
        } else if sum != last_checksum {
            checksum_changed = true;
            last_checksum = sum;
        }
    }

    room.set_simulate_present_path(true);
    let frames = 100usize;
    let mut tick_times = Vec::with_capacity(frames);
    for i in 0..frames {
        room.set_framebuffer(&varying_frame(&fb, i));
        let t_tick = Instant::now();
        room.tick();
        tick_times.push(t_tick.elapsed());
    }

    let (tick_avg, _tick_p50, tick_p95) = summarize("tick-only (present path)", &tick_times);
    let (_rb_avg, _, rb_p95) = summarize("tick+CPU readback (diagnostic only)", &readback_times);
    let _ = rb_p95;

    // Gate on tick-only (IOSurface-like). Floor: 60 Hz (≤16 ms avg). ProMotion 120 Hz
    // needs ≤8 ms avg in-app — reported here; SpecChumMac HUD is the source of truth.
    const P95_BUDGET_MS: f64 = 16.0;
    const AVG_BUDGET_MS: f64 = 12.0;
    const MIN_PLAUSIBLE_MS: f64 = 0.5;
    let soft = std::env::var("SPEC_CHUM_ROOM_PERF_SOFT")
        .map(|v| v != "0" && !v.is_empty())
        .unwrap_or(false);
    if tick_p95 > P95_BUDGET_MS {
        eprintln!("FAIL: tick-only p95 {tick_p95:.2} ms exceeds {P95_BUDGET_MS} ms (60 Hz floor)");
        if !soft {
            std::process::exit(1);
        }
    }
    if tick_avg > AVG_BUDGET_MS {
        eprintln!(
            "FAIL: tick-only average {tick_avg:.2} ms exceeds {AVG_BUDGET_MS} ms (60 Hz floor)"
        );
        if !soft {
            std::process::exit(1);
        }
    }
    if tick_avg > 8.0 {
        eprintln!(
            "WARN: tick-only avg {tick_avg:.2} ms > 8 ms — may not sustain 120 Hz on ProMotion"
        );
    }
    if tick_avg < MIN_PLAUSIBLE_MS {
        eprintln!(
            "FAIL: tick-only average {tick_avg:.2} ms is unrealistically fast (render likely skipped)"
        );
        std::process::exit(1);
    }
    if create_ms > 30_000.0 {
        eprintln!("FAIL: create took {create_ms:.0} ms (too slow)");
        std::process::exit(1);
    }
    if !checksum_changed {
        eprintln!("WARN: room frame checksum never changed across timed ticks");
    }
    eprintln!("OK: headless tick-only performance within budget at {w}×{h}");
}
