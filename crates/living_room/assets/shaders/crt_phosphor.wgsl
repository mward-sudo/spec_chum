#import bevy_pbr::forward_io::VertexOutput

// CRT phosphor pass — open crt-aperture / crt-easymode techniques approximating
// Retro Virtual Machine's UK-TV look on a 3D tube mesh (RVM = visual reference only).
// Curvature is mesh geometry; do not apply 2D barrel warp here.
// Halation: light in-shader feed only; main glow is Bevy Bloom.

struct CrtPhosphorMaterial {
    params0: vec4<f32>, // time, scan_str, grille_str, brightness
    params1: vec4<f32>, // gamma_in, gamma_out, soft_mix, black_lift
}

@group(#{MATERIAL_BIND_GROUP}) @binding(0)
var<uniform> material: CrtPhosphorMaterial;
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

fn to_linear(c: vec3<f32>, gamma_in: f32) -> vec3<f32> {
    return pow(max(c, vec3(0.0)), vec3(gamma_in));
}

fn to_display(c: vec3<f32>, gamma_out: f32) -> vec3<f32> {
    return pow(max(c, vec3(0.0)), vec3(1.0 / gamma_out));
}

// Soft horizontal 3-tap (composite-ish H blur); vertical stays sharp.
fn sample_nearest(uv: vec2<f32>) -> vec3<f32> {
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

// Luminance-adaptive scanline weight (crt-aperture beam min/max + shape).
// Floor the weight so thin Spectrum text rows are never fully extinguished.
fn scanline_weight(uv_y: f32, col: vec3<f32>, strength: f32) -> f32 {
    let luma = dot(col, vec3(0.2126, 0.7152, 0.0722));
    let bright = pow(clamp(luma, 0.0, 1.0), 1.0 / SCAN_SHAPE);
    let beam = mix(SCAN_BEAM_MIN, SCAN_BEAM_MAX, bright);
    // Distance from scan centre in line space (0..1 within a source line).
    let line_y = fract(uv_y * SRC_H);
    let x = abs(line_y - 0.5) * 2.0;
    let core = smoothstep(0.0, 1.0, 1.0 - min(1.0, x / max(beam * 0.5, 0.05)));
    let weight = mix(1.0 - strength, 1.0, core);
    let w = mix(1.0, weight, strength);
    // Floor keeps thin Spectrum glyph rows alive; 0.72 flattened scanlines into mush.
    return max(w, 0.58);
}

/// Soften aperture mask so a channel never drops below ~70% (glyph edges stay).
fn aperture_grille(uv_x: f32, strength: f32) -> vec3<f32> {
    // Vertical RGB triad — Trinitron / aperture-grille class (crt-aperture MASK_COLORS=3).
    let slot = floor(uv_x * SRC_W * 3.0) % 3.0;
    var mask = vec3(1.0);
    if slot < 1.0 {
        mask = vec3(1.12, 0.82, 0.82);
    } else if slot < 2.0 {
        mask = vec3(0.82, 1.12, 0.82);
    } else {
        mask = vec3(0.82, 0.82, 1.12);
    }
    return mix(vec3(1.0), mask, strength);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // Tube-space UV across the phosphor mesh; bezel occludes geometric overscan.
    let tube_uv = in.uv;
    let t = material.params0.x;
    let scan_str = material.params0.y;
    let grille_str = material.params0.z;
    let brightness = material.params0.w;
    let gamma_in = material.params1.x;
    let gamma_out = material.params1.y;
    let soft_mix = material.params1.z;
    // params1.w = mesh aspect (W/H of phosphor quad).
    let mesh_aspect = max(material.params1.w, 0.01);

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

    // Soft H / sharp V: nearest texel (sharp) + optional horizontal soft mix.
    let sharp = sample_nearest(uv);
    let soft_h = sample_soft_h(uv);
    // Geometric mean (crt-aperture sharp×soft) with soft_mix as blend toward soft-H path.
    let soft_sharp = sqrt(max(sharp * soft_h, vec3(0.0)));
    var color = mix(sharp, soft_sharp, soft_mix);

    color = max(color, vec3(BLACK_LIFT));
    color = to_linear(color, gamma_in);

    let soft_lin = to_linear(max(soft_h, vec3(BLACK_LIFT)), gamma_in);
    let scan = scanline_weight(uv.y, soft_lin, scan_str);
    color *= scan;
    color *= aperture_grille(uv.x, grille_str);

    // Tiny in-shader halation / diffusion (crt-aperture GLOW_*); Bevy Bloom does the rest.
    let glow = to_linear(max(sample_glow_h(uv), vec3(BLACK_LIFT)), gamma_in);
    let halo = max(glow - color, vec3(0.0));
    color += halo * halo * HALATION;
    color += glow * DIFFUSION;

    // Subtle 50 Hz brightness flicker (PAL); amp ≤ ~1%.
    let flicker = 1.0 + FLICKER_AMP * sin(t * 50.0 * 2.0 * PI);
    color *= flicker * vignette(tube_uv) * brightness;

    color = to_display(color, gamma_out);
    return vec4(color, 1.0);
}
