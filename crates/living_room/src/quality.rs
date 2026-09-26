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

/// Primary CRT halation path. Defaults to the established full-screen Bloom look.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HalationMode {
    #[default]
    Bloom,
    Material,
}

/// `SPEC_CHUM_ROOM_HALATION=bloom|material` — default **bloom** until visual acceptance.
/// For issue #458, capture both with `SPEC_CHUM_ROOM_SCENE=current`, unchanged window
/// size/camera preset and exposure: once with `...HALATION=bloom`, then `...=material`.
pub fn halation_mode() -> HalationMode {
    halation_mode_from(std::env::var("SPEC_CHUM_ROOM_HALATION").ok().as_deref())
}

fn halation_mode_from(value: Option<&str>) -> HalationMode {
    match value
        .unwrap_or("bloom")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "material" => HalationMode::Material,
        _ => HalationMode::Bloom,
    }
}

pub fn material_halation_enabled() -> bool {
    halation_mode() == HalationMode::Material
}

/// Bloom stays on for the baseline; material mode replaces global Bloom.
pub fn bloom_enabled() -> bool {
    !material_halation_enabled() && env_truthy("SPEC_CHUM_ROOM_BLOOM").unwrap_or(true)
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

/// Temporary #149 A/B: `SPEC_CHUM_ROOM_SCENE=current|new` (default **current**).
///
/// Prefer the SpecChumMac toolbar toggle when verifying visually. Env is for automation.
/// Remove with the A/B harness once lightmaps ship.
pub fn scene_variant() -> crate::scene_variant::SceneVariant {
    match std::env::var("SPEC_CHUM_ROOM_SCENE")
        .unwrap_or_else(|_| "current".into())
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "new" | "1" | "wip" | "lightmap" => crate::scene_variant::SceneVariant::New,
        _ => crate::scene_variant::SceneVariant::Current,
    }
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
        "scene={} hybrid={} halation={:?} bloom={} mips={} msaa={:?} fxaa={} lights={:?}",
        scene_variant().label(),
        hybrid_enabled(),
        halation_mode(),
        bloom_enabled(),
        bloom_max_mip_dimension(),
        msaa_samples(),
        fxaa_enabled(),
        light_preset()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halation_mode_defaults_to_bloom_and_accepts_material_case_insensitively() {
        assert_eq!(halation_mode_from(None), HalationMode::Bloom);
        assert_eq!(halation_mode_from(Some("BLOOM")), HalationMode::Bloom);
        assert_eq!(
            halation_mode_from(Some(" Material ")),
            HalationMode::Material
        );
        assert_eq!(halation_mode_from(Some("other")), HalationMode::Bloom);
    }
}
