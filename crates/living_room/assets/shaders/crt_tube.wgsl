#import bevy_sprite::mesh2d_vertex_output::VertexOutput

// CRT phosphor reconstruction in a fixed 1280x960 tube image, before the
// room camera samples the curved 3D mesh (RVM = visual reference only).
// Curvature remains mesh geometry; do not apply 2D barrel warp here.
// Halation: either the established camera Bloom path or selectable local material scatter.

struct CrtTubeMaterial {
    params0: vec4<f32>, // time, scan_str, grille_str, brightness
    params1: vec4<f32>, // soft_mix, mesh_aspect, reserved, reserved
    params2: vec4<f32>, // material_halation (0 = Bloom baseline, 1 = local halo)
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0)
var<uniform> material: CrtTubeMaterial;
@group(#{MATERIAL_BIND_GROUP}) @binding(2)
var screen_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(3)
var screen_sampler: sampler;

const SRC_W: f32 = 352.0;
const SRC_H: f32 = 296.0;
const PI: f32 = 3.14159265;
const BLACK_LIFT: f32 = 0.012;
// Classic Spectrum-on-TV picture aspect (256×192 / 4:3).
const CONTENT_ASPECT: f32 = 4.0 / 3.0;
// Trinitron-style grille frequency across the visible picture, independent of
// the Spectrum source raster and the tube render-target dimensions.
const APERTURE_TRIADS: f32 = 280.0;
// Slight UV zoom inside the 4:3 rect (emulated overscan; not geometric spill).
const TEX_OVERSCAN: f32 = 1.012;

// Mesh fills a 4:3 rect in the punched opening (nearly flush). Spectrum FB stretches
// into that 4:3 (classic non-square pixels). If mesh ≠ 4:3, letterbox/pillar first.

// crt-aperture / easymode beam model (constants — sofa-distance Trinitron look).
const SCAN_BEAM_MIN: f32 = 0.65;
const SCAN_BEAM_MAX: f32 = 1.35;
const SCAN_SHAPE: f32 = 2.5;
const HALATION: f32 = 0.04;
const DIFFUSION: f32 = 0.015;
const FLICKER_AMP: f32 = 0.006;

fn vignette(uv: vec2<f32>) -> f32 {
    // Essentially off — corner darkening read as a pinched / floating screen.
    // Keep `uv` referenced so WGSL accepts the signature (no `_` discard).
    return 1.0 + 0.0 * uv.x;
}

// Soft horizontal 3-tap (composite-ish H blur); vertical stays sharp.
fn sample_nearest(uv: vec2<f32>) -> vec3<f32> {
    // The Rgba8UnormSrgb source is decoded to linear values by the texture format.
    // Snap to texel centres so Spectrum 8×8 glyphs don't drop columns/rows when
    // the tube fills the view (interpolated sample + scanlines ate thin strokes).
    let px = clamp(i32(floor(uv.x * SRC_W + 0.0)), 0, i32(SRC_W) - 1);
    let py = clamp(i32(floor(uv.y * SRC_H + 0.0)), 0, i32(SRC_H) - 1);
    return textureLoad(screen_texture, vec2(px, py), 0).rgb;
}

fn sample_soft_h(uv: vec2<f32>) -> vec3<f32> {
    let dx = 1.0 / SRC_W;
    var acc = sample_nearest(uv) * 0.50;
    acc += sample_nearest(uv + vec2(dx, 0.0)) * 0.25;
    acc += sample_nearest(uv - vec2(dx, 0.0)) * 0.25;
    return acc;
}

// Wider H taps for a light phosphor-diffusion / Bloom feed (not main glow).
fn sample_glow_h(uv: vec2<f32>) -> vec3<f32> {
    let dx = 1.5 / SRC_W;
    var acc = sample_nearest(uv) * 0.40;
    acc += sample_nearest(uv + vec2(dx, 0.0)) * 0.30;
    acc += sample_nearest(uv - vec2(dx, 0.0)) * 0.30;
    return acc;
}

// Broad horizontal phosphor scatter used only by the opt-in material-halation path.
fn sample_halation_h(uv: vec2<f32>) -> vec3<f32> {
    let dx = 5.0 / SRC_W;
    var acc = sample_nearest(uv) * 0.50;
    acc += sample_nearest(uv + vec2(dx, 0.0)) * 0.25;
    acc += sample_nearest(uv - vec2(dx, 0.0)) * 0.25;
    return acc;
}

// Integral of the shaped beam core from its centre to `distance`. The core is
// 1 - smoothstep(0, 1, distance / radius), whose full-line integral is radius.
fn beam_integral(distance: f32, radius: f32) -> f32 {
    let x = clamp(distance, 0.0, radius);
    let x2 = x * x;
    let r2 = radius * radius;
    return x - (x2 * x) / r2 + (x2 * x2) / (2.0 * r2 * radius);
}

fn beam_core(distance: f32, radius: f32) -> f32 {
    if distance >= radius {
        return 0.0;
    }
    return 1.0 - smoothstep(0.0, 1.0, distance / radius);
}

fn raster_color(uv: vec2<f32>, soft_mix: f32) -> vec3<f32> {
    let sharp = sample_nearest(uv);
    let soft_h = sample_soft_h(uv);
    // Both inputs are linear-light signals with preserved average energy;
    // arithmetic interpolation retains that invariant for every blend value.
    return mix(sharp, soft_h, soft_mix);
}

// Reconstruct each source row as an emitted beam. Normalizing each shaped beam
// by its analytic area preserves that row's integrated linear-light energy;
// clipping the normalization at the top/bottom edge avoids losing edge rows.
// Three rows suffice because the widest beam has a radius below one line pitch.
fn scanline_reconstruction(uv: vec2<f32>, soft_mix: f32, strength: f32) -> vec3<f32> {
    let line_y = uv.y * SRC_H;
    let base_row = min(i32(floor(line_y)), i32(SRC_H) - 1);
    let unfiltered = raster_color(uv, soft_mix);
    var beam_color = vec3(0.0);
    for (var offset = -1; offset <= 1; offset += 1) {
        let row = base_row + offset;
        if row >= 0 && row < i32(SRC_H) {
            let row_y = f32(row) + 0.5;
            let distance = abs(line_y - row_y);
            if distance < SCAN_BEAM_MAX * 0.5 {
                var row_color = unfiltered;
                if row != base_row {
                    row_color = raster_color(vec2(uv.x, row_y / SRC_H), soft_mix);
                }
                let luma = dot(row_color, vec3(0.2126, 0.7152, 0.0722));
                let bright = pow(clamp(luma, 0.0, 1.0), 1.0 / SCAN_SHAPE);
                let beam = mix(SCAN_BEAM_MIN, SCAN_BEAM_MAX, bright);
                let radius = max(beam * 0.5, 0.05);
                let area = beam_integral(row_y, radius)
                    + beam_integral(SRC_H - row_y, radius);
                let density = beam_core(distance, radius) / area;
                beam_color += row_color * density;
            }
        }
    }

    // The former scanline control was quadratic in `strength`; retaining that
    // response keeps the existing near/far look ramp gentle and predictable.
    let amount = pow(clamp(strength, 0.0, 1.0), 2.0);
    return mix(unfiltered, beam_color, amount);
}

// Antiderivative of a periodic, unit-width stripe with period three. Integrating
// over the fragment footprint gives stable stripe coverage at tube resolution
// and naturally fades the pattern as the downstream curved-mesh mips minify it.
fn grille_stripe_integral(stripe_x: f32, channel: f32) -> f32 {
    let shifted = stripe_x - channel;
    let periods = floor(shifted / 3.0);
    let within_period = shifted - periods * 3.0;
    return periods + clamp(within_period, 0.0, 1.0);
}

fn grille_channel_coverage(stripe_x: f32, footprint: f32, channel: f32) -> f32 {
    let half_footprint = footprint * 0.5;
    let covered = grille_stripe_integral(stripe_x + half_footprint, channel)
        - grille_stripe_integral(stripe_x - half_footprint, channel);
    return clamp(covered / footprint, 0.0, 1.0);
}

/// Analytic RGB aperture grille in visible-picture coordinates. The 0.82 / 1.12
/// levels retain the previous mask's strength and mean luminance.
fn aperture_grille(content_x: f32, footprint: f32, strength: f32) -> vec3<f32> {
    let stripe_x = content_x * APERTURE_TRIADS * 3.0;
    let coverage = vec3(
        grille_channel_coverage(stripe_x, footprint, 0.0),
        grille_channel_coverage(stripe_x, footprint, 1.0),
        grille_channel_coverage(stripe_x, footprint, 2.0),
    );
    let mask = mix(vec3(0.82), vec3(1.12), coverage);
    return mix(vec3(1.0), mask, strength);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // Evaluate derivatives before the non-uniform content-rectangle returns.
    let grille_footprint = max(fwidth(in.uv.x * APERTURE_TRIADS * 3.0), 0.0001);
    // Tube-space UV across the flat reconstruction quad. The curved mesh and
    // physical bezel apply their geometry after this pass.
    let tube_uv = in.uv;
    let t = material.params0.x;
    let scan_str = material.params0.y;
    let grille_str = material.params0.z;
    let brightness = material.params0.w;
    let soft_mix = material.params1.x;
    let material_halation = material.params2.x > 0.5;
    let power = clamp(material.params2.y, 0.0, 1.0);
    // params1.w = mesh aspect (W/H of phosphor quad).
    let mesh_aspect = max(material.params1.y, 0.01);

    // Fit a 4:3 content rect into the mesh (pillar/letter if mesh aspect differs).
    var content_uv = tube_uv;
    if mesh_aspect > CONTENT_ASPECT {
        let content_w = CONTENT_ASPECT / mesh_aspect;
        let u0 = 0.5 - 0.5 * content_w;
        let u_c = (tube_uv.x - u0) / content_w;
        if u_c < 0.0 || u_c > 1.0 {
            return vec4(vec3(BLACK_LIFT), 1.0);
        }
        content_uv = vec2(u_c, tube_uv.y);
    } else if mesh_aspect < CONTENT_ASPECT {
        let content_h = mesh_aspect / CONTENT_ASPECT;
        let v0 = 0.5 - 0.5 * content_h;
        let v_c = (tube_uv.y - v0) / content_h;
        if v_c < 0.0 || v_c > 1.0 {
            return vec4(vec3(BLACK_LIFT), 1.0);
        }
        content_uv = vec2(tube_uv.x, v_c);
    }

    // Stretch Spectrum FB into the 4:3 rect + slight UV overscan.
    var uv = (content_uv - vec2(0.5)) / TEX_OVERSCAN + vec2(0.5);
    uv = clamp(uv, vec2(0.0), vec2(1.0));

    // Soft H / beam-reconstructed V: horizontal diffusion remains independent
    // of the energy-normalized vertical electron-beam profile.
    var color = scanline_reconstruction(uv, soft_mix, scan_str);

    color = max(color, vec3(BLACK_LIFT));
    color *= aperture_grille(content_uv.x, grille_footprint, grille_str);

    // Retain the historical feed for the selectable Bloom baseline. In material mode,
    // broaden the source on the phosphor and put the primary halo into the CRT surface.
    let glow = max(sample_glow_h(uv), vec3(BLACK_LIFT));
    let halo = max(glow - color, vec3(0.0));
    if material_halation {
        let wide = max(sample_halation_h(uv), vec3(BLACK_LIFT));
        color += halo * 0.28;
        color += max(wide - color, vec3(0.0)) * 0.18;
        color += glow * 0.035;
    } else {
        color += halo * halo * HALATION;
        color += glow * DIFFUSION;
    }

    // CRT power-up: let a broad central phosphor glow bloom over the raster.
    // Add a warm central phosphor glow during startup; CRT power still gates
    // the raster below, so zero power intentionally produces a black screen.
    let powered = smoothstep(0.0, 1.0, power);
    let radial = length((tube_uv - vec2(0.5)) * vec2(1.0, 0.75));
    let glow_radius = mix(0.32, 0.90, powered);
    let glow_distance = radial / glow_radius;
    let startup_glow = exp(-glow_distance * glow_distance);
    let central_warmth = smoothstep(0.0, 0.22, power) * (1.0 - powered) * startup_glow * 0.12;
    let settle_distance = (powered - 0.86) / 0.13;
    let phosphor_settle = 1.0 + 0.025 * exp(-settle_distance * settle_distance);
    color *= powered;
    color += vec3(1.0, 0.62, 0.34) * central_warmth;
    color *= phosphor_settle;

    // Subtle 50 Hz brightness flicker (PAL); amp ≤ ~1%.
    let flicker = 1.0 + FLICKER_AMP * sin(t * 50.0 * 2.0 * PI);
    color *= flicker * vignette(tube_uv) * brightness;

    // The sRGB source is decoded by textureLoad. Store linear HDR here; the
    // room camera performs tonemapping after sampling the completed mip chain.
    return vec4(color, 1.0);
}
