//! Headless visual probe — render the room and dump a PPM for eyeballing.
//!
//! Catches "present path renders, but the room is missing" regressions that a
//! non-black pixel check in `room_perf` happily passes.
//!
//! **Note:** this path uses CPU `copy_frame_rgba` readback (blocking `map_async` +
//! BGRA copy), not the SpecChumMac IOSurface present. Readback frames can look
//! **darker or harsher** than the live embed — that is a probe artifact, not a live bug.
//!
//! Usage: `room_probe [width] [height] [out.ppm] [zoom_steps]`
//! (default 1280×720, `/tmp/room_probe.ppm`, zoom preset 0).

use std::env;
use std::io::Write;

use spec_chum_room::crt::{SCREEN_H, SCREEN_W};
use spec_chum_room::HeadlessRoom;

fn main() {
    let mut args = env::args().skip(1);
    let w: u32 = args
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1280)
        .max(64);
    let h: u32 = args
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(720)
        .max(64);
    let out_path = args.next().unwrap_or_else(|| "/tmp/room_probe.ppm".into());
    let zoom_steps: i32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(0);

    let mut room = HeadlessRoom::new(w, h);
    room.request_skip_intro();

    // Mid-grey Spectrum framebuffer so the CRT is clearly distinguishable.
    let fb = vec![90u8; (SCREEN_W * SCREEN_H * 4) as usize];
    let mut buf = vec![0u8; (w * h * 4) as usize];
    // Intro skip resets zoom, so settle first, then nudge and let plates re-settle.
    for _ in 0..90 {
        room.set_framebuffer(&fb);
        room.tick();
    }
    if zoom_steps != 0 {
        room.nudge_zoom(zoom_steps);
    }
    for _ in 0..120 {
        room.set_framebuffer(&fb);
        room.tick();
    }
    let n = room.copy_frame_rgba(&mut buf);
    assert_eq!(n, buf.len(), "short frame readback");

    // Present target is BGRA; PPM wants RGB.
    let mut ppm = Vec::with_capacity(buf.len() / 4 * 3 + 32);
    ppm.extend_from_slice(format!("P6\n{w} {h}\n255\n").as_bytes());
    for px in buf.as_chunks::<4>().0 {
        ppm.extend_from_slice(&[px[2], px[1], px[0]]);
    }
    let mut f = std::fs::File::create(&out_path).expect("create probe output");
    f.write_all(&ppm).expect("write probe output");

    // Rough content report: unique-ish colours and mean luma per third of the frame.
    let third = (h / 3).max(1) as usize;
    for (label, row0) in [("top", 0usize), ("middle", third), ("bottom", third * 2)] {
        let start = row0 * w as usize * 4;
        let end = (start + third * w as usize * 4).min(buf.len());
        let rows = &buf[start..end];
        let mut sum = 0f64;
        for px in rows.as_chunks::<4>().0 {
            sum +=
                f64::from(px[2]) * 0.2126 + f64::from(px[1]) * 0.7152 + f64::from(px[0]) * 0.0722;
        }
        let mean = sum / (rows.len() / 4) as f64;
        eprintln!("  {label} third: mean luma {mean:.1}");
    }
    eprintln!("  zoom preset {}", room.zoom_preset());
    eprintln!("wrote {out_path}");
}
