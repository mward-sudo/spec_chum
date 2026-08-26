//! Runtime quality / perf knobs for living-room A/B (`SPEC_CHUM_ROOM_*` env).
//!
//! Defaults favour the look the CRT scene was designed for. Dial down for matrix runs:
//! ```text
//! SPEC_CHUM_ROOM_BLOOM=0 SPEC_CHUM_ROOM_MSAA=0 SPEC_CHUM_ROOM_LIGHTS=min
//! ```

use bevy::prelude::*;
use bevy::render::view::Msaa;

fn env_truthy(key: &str) -> Option<bool> {
    let v = std::env::var(key).ok()?;
    let t = v.trim();
    if t.is_empty() || t == "0" || t.eq_ignore_ascii_case("false") || t.eq_ignore_ascii_case("off")
    {
        return Some(false);
    }
    Some(true)
}

/// Bloom on by default (CRT halation). `SPEC_CHUM_ROOM_BLOOM=0` disables.
pub fn bloom_enabled() -> bool {
    env_truthy("SPEC_CHUM_ROOM_BLOOM").unwrap_or(true)
}

/// Cap bloom mip dimension. Default **512** (full CRT halation).
///
/// The earlier 256 default was chosen against a benchmark that timed a blocking CPU
/// readback; on the real present path 512 costs well under a millisecond here.
pub fn bloom_max_mip_dimension() -> u32 {
    std::env::var("SPEC_CHUM_ROOM_BLOOM_MIPS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(512)
        .clamp(64, 1024)
}

/// `SPEC_CHUM_ROOM_MSAA=0|2|4` — default **4** (bezel / furniture edges).
///
/// 8x is deliberately absent: Metal rejects it here and the render target goes black.
pub fn msaa_samples() -> Msaa {
    match std::env::var("SPEC_CHUM_ROOM_MSAA")
        .unwrap_or_else(|_| "4".into())
        .trim()
    {
        "0" | "off" | "Off" => Msaa::Off,
        "2" => Msaa::Sample2,
        _ => Msaa::Sample4,
    }
}

/// Post-process FXAA (TV bezel edges). Default **on**; `SPEC_CHUM_ROOM_FXAA=0` disables.
pub fn fxaa_enabled() -> bool {
    env_truthy("SPEC_CHUM_ROOM_FXAA").unwrap_or(true)
}

/// Bake room to camera-space plates; live TV/cabinet/CRT.
///
/// Default **off**: the plate is camera-parented, so the room does not parallax while
/// zooming, and bake frames blank the background. `SPEC_CHUM_ROOM_HYBRID=1` for experiments.
pub fn hybrid_enabled() -> bool {
    env_truthy("SPEC_CHUM_ROOM_HYBRID").unwrap_or(false)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightPreset {
    /// CRT fill + wall bounce + 3 TV-wall sconces (side fixtures mesh-only).
    Full,
    /// One CRT fill + one centre sconce only.
    Min,
}

/// `SPEC_CHUM_ROOM_LIGHTS=full|min` — default **full**.
pub fn light_preset() -> LightPreset {
    match std::env::var("SPEC_CHUM_ROOM_LIGHTS")
        .unwrap_or_else(|_| "full".into())
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "min" | "minimal" | "low" => LightPreset::Min,
        _ => LightPreset::Full,
    }
}

/// One-line label for perf logs.
pub fn preset_label() -> String {
    format!(
        "hybrid={} bloom={} mips={} msaa={:?} fxaa={} lights={:?}",
        hybrid_enabled(),
        bloom_enabled(),
        bloom_max_mip_dimension(),
        msaa_samples(),
        fxaa_enabled(),
        light_preset()
    )
}
