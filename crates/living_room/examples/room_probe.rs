//! Headless visual probe — render a CRT test pattern in the room and dump a PPM.
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
use std::thread;
use std::time::Duration;

use spec_chum_room::camera::ZOOM_PRESET_COUNT;
use spec_chum_room::crt::{SCREEN_H, SCREEN_W};
use spec_chum_room::HeadlessRoom;

/// Render one headless room frame at a requested size and relative zoom stop.
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

    let initial_fb = crt_test_pattern(false);
    let updated_fb = crt_test_pattern(true);
    let mut buf = vec![0u8; (w * h * 4) as usize];
    let mut initial_frame = vec![0u8; buf.len()];
    // Intro skip resets zoom, so settle first, then nudge and let plates re-settle.
    for _ in 0..90 {
        room.set_framebuffer(&initial_fb);
        room.tick();
    }
    let initial_len = room.copy_frame_rgba(&mut initial_frame);
    assert_eq!(
        initial_len,
        initial_frame.len(),
        "short initial frame readback"
    );
    if zoom_steps != 0 {
        let direction = zoom_steps.signum();
        let step_count = zoom_steps
            .unsigned_abs()
            .min(u32::from(ZOOM_PRESET_COUNT.saturating_sub(1)));
        for _ in 0..step_count {
            room.nudge_zoom(direction);
            thread::sleep(Duration::from_millis(110));
            for _ in 0..12 {
                room.set_framebuffer(&initial_fb);
                room.tick();
            }
        }
    }
    for _ in 0..120 {
        room.set_framebuffer(&updated_fb);
        room.tick();
    }
    let n = room.copy_frame_rgba(&mut buf);
    assert_eq!(n, buf.len(), "short frame readback");
    let changed_pixels = initial_frame
        .as_chunks::<4>()
        .0
        .iter()
        .zip(buf.as_chunks::<4>().0.iter())
        .filter(|(before, after)| before != after)
        .count();
    assert!(
        changed_pixels > 0,
        "framebuffer update changed no rendered pixels"
    );

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
    eprintln!("  framebuffer update changed {changed_pixels} rendered pixels");
    eprintln!("wrote {out_path}");
}

/// Color bars and fine horizontal/vertical edges make phosphor filtering visible.
fn crt_test_pattern(alternate: bool) -> Vec<u8> {
    const BARS: [[u8; 3]; 8] = [
        [235, 235, 235],
        [235, 220, 32],
        [32, 220, 220],
        [32, 220, 48],
        [235, 32, 220],
        [220, 32, 32],
        [32, 48, 220],
        [12, 12, 12],
    ];
    let mut framebuffer = vec![0u8; (SCREEN_W * SCREEN_H * 4) as usize];
    for y in 0..SCREEN_H {
        for x in 0..SCREEN_W {
            let rgb = if y < SCREEN_H / 2 {
                let bar = (x * BARS.len() as u32 / SCREEN_W) as usize;
                BARS[if alternate { BARS.len() - bar - 1 } else { bar }]
            } else if y < SCREEN_H * 3 / 4 {
                if (x / if alternate { 7 } else { 4 }) % 2 == 0 {
                    [240, 240, 240]
                } else {
                    [8, 8, 8]
                }
            } else if ((x / if alternate { 5 } else { 8 }) + (y / if alternate { 5 } else { 8 }))
                % 2
                == 0
            {
                [240, 240, 240]
            } else {
                [8, 8, 8]
            };
            let offset = ((y * SCREEN_W + x) * 4) as usize;
            framebuffer[offset..offset + 3].copy_from_slice(&rgb);
            framebuffer[offset + 3] = 255;
        }
    }
    framebuffer
}
