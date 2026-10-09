//! Bulging phosphor mesh + CRT material + glass, seated in the punched
//! `television_02_aperture` mesh.
//!
//! Poly Haven `television_02` has two bezels that stay as real geometry:
//!   1. Outer cabinet / frame bevel
//!   2. Inner screen bezel / glass rim (clips the picture)
//!
//! The punch removes the painted glass UV island. Phosphor matches the user
//! reference magenta footprint: fills the visible glass, slight underlap under the
//! inner dark bezel, soft barrel; never covers the outer cabinet. Depth matters —
//! a plane too far forward parallax-shifts up onto the outer bevel.
//!
//! Phosphor fills the punched glass opening (AABB + slight geometric overscan).
//! Depth is nearly flush with the rim — overscan is mostly UV/texture bleed, not
//! deep Z-occlusion under the bezel. Bottom gets an extra downward extend so the
//! short bottom edge matches top/left/right without moving those edges.
//!
//! Debug (bake into SpecChumMac wrapper when set at launch):
//! - `SPEC_CHUM_ROOM_APERTURE_DEBUG=1` — bright magenta = phosphor-sized aperture
//!   (same mesh as CRT, no Spectrum FB); hides phosphor; also brightens the room.
//! - `SPEC_CHUM_ROOM_BRIGHT_DEBUG=1` — brighten lights/ambient without magenta.
//! - `SPEC_CHUM_ROOM_HIDE_CRT=1` — hide phosphor / glass only.

use bevy::asset::RenderAssetUsages;
use bevy::camera::{visibility::RenderLayers, Hdr, NormalizedRenderTarget, RenderTarget};
use bevy::core_pipeline::{
    mip_generation::{
        generate_mips_for_phase, MipGenerationJobs, MipGenerationPhaseId, MipGenerationPipelines,
    },
    schedule::Core2d,
    upscaling::upscaling,
};
use bevy::image::{ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::{
    camera::ExtractedCamera,
    render_asset::RenderAssets,
    render_resource::{
        AsBindGroup, Extent3d, PipelineCache, TextureDimension, TextureFormat, TextureUsages,
    },
    renderer::{CurrentView, RenderContext},
    texture::GpuImage,
    Extract, ExtractSchedule, RenderApp,
};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{Material2d, Material2dPlugin};

use crate::room::{TelevisionCabinet, TV_CABINET_POS};

pub const SCREEN_W: u32 = 352;
pub const SCREEN_H: u32 = 296;
pub const TUBE_W: u32 = 1280;
pub const TUBE_H: u32 = 960;
const TUBE_MIP_COUNT: u32 = 32 - TUBE_W.leading_zeros();
const TUBE_LAYER: RenderLayers = RenderLayers::layer(31);
// "CRT1" in ASCII; phase IDs are shared by all render plugins.
const TUBE_MIP_PHASE: MipGenerationPhaseId = MipGenerationPhaseId(0x4352_5401);

/// Visible glass opening = punched painted-glass XY AABB (matches hole in mesh).
pub const APERTURE_W: f32 = 0.3165;
pub const APERTURE_H: f32 = 0.2372;

/// Same as [`APERTURE_W`] / [`APERTURE_H`] (kept for diagnostics / older call sites).
pub const GLASS_AABB_W: f32 = APERTURE_W;
pub const GLASS_AABB_H: f32 = APERTURE_H;

/// Classic Spectrum picture aspect (256×192 active area / 4:3 TV).
pub const CONTENT_ASPECT: f32 = 4.0 / 3.0;

/// Narrower rim-plane slice (z-filtered) — under-bezel wrap excluded.
pub const RIM_PLANE_W: f32 = 0.2374;
pub const RIM_PLANE_H: f32 = 0.2372;

/// Local-space centre of the punched glass opening.
pub const APERTURE_CENTER_LOCAL: Vec3 = Vec3::new(-0.0018, 0.2588, 0.1077);

/// Inner-rim front Z (local). Phosphor peak must stay near this (flush policy).
pub const APERTURE_Z_FRONT: f32 = 0.1100;

/// Slight uniform geometric overscan (keep small — bottom must stay inside bevel).
pub const OVERSCAN_FRAC: f32 = 0.012;

/// 4:3 phosphor footprint fitted to aperture width (opening is already ≈4:3).
pub const PHOSPHOR_BASE_W: f32 = APERTURE_W;
pub const PHOSPHOR_BASE_H: f32 = APERTURE_W / CONTENT_ASPECT;

pub const PHOSPHOR_W: f32 = PHOSPHOR_BASE_W * (1.0 + 2.0 * OVERSCAN_FRAC);
pub const PHOSPHOR_H: f32 = PHOSPHOR_BASE_H * (1.0 + 2.0 * OVERSCAN_FRAC);

/// Bottom Y adjust in metres: **negative pulls bottom up** (inside bevel), positive spills.
/// Kept at 0 — spill was from a positive extend; 4:3 mesh on aperture centre stays inside.
pub const BOTTOM_Y_ADJUST: f32 = 0.0;

/// Mesh height after bottom adjust (equals [`PHOSPHOR_H`] when adjust is 0).
pub const PHOSPHOR_MESH_H: f32 = PHOSPHOR_H + BOTTOM_Y_ADJUST;

/// Soft convex faceplate bulge (metres) — mild barrel; plane sits nearly flush.
pub const PHOSPHOR_BULGE: f32 = 0.005;

/// How far behind [`APERTURE_Z_FRONT`] the phosphor mesh origin sits.
const PHOSPHOR_Z_BEHIND: f32 = 0.005;

/// Spectrum FB UV zoom (>1 = slight emulated overscan within the 4:3 rect).
/// Keep mild — larger values crop edge glyphs at mid zoom fills.
pub const TEX_OVERSCAN: f32 = 1.012;

const SHADER_PATH: &str = "shaders/crt_phosphor.wgsl";
const TUBE_SHADER_PATH: &str = "shaders/crt_tube.wgsl";

#[derive(Resource, Clone, Debug)]
pub struct CrtScreenTexture(pub Handle<Image>);

/// Fixed-resolution, linear HDR phosphor image. The curved screen samples its
/// generated mip chain while the window-sized room target remains independent.
#[derive(Resource, Clone, Debug)]
pub struct CrtTubeTexture(pub Handle<Image>);

#[derive(Resource, Clone, Debug)]
struct CrtTubeCanvas(Handle<Image>);

#[derive(Resource)]
struct ExtractedTubeImages {
    canvas: Handle<Image>,
    tube: Handle<Image>,
}

#[derive(Component, Debug)]
pub struct CrtPhosphor;

#[derive(Component, Debug)]
pub struct CrtGlass;

#[derive(Component, Debug)]
pub struct ApertureDebugMarker;

#[derive(Component, Debug)]
struct CrtAttachedToTv;

/// Samples the completed tube image on the curved CRT surface.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct CrtPhosphorMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub tube: Handle<Image>,
}

impl Material for CrtPhosphorMaterial {
    fn fragment_shader() -> ShaderRef {
        SHADER_PATH.into()
    }
}

/// The raster is reconstructed once in tube coordinates before the 3D room
/// camera renders the curved mesh. Uniform packing matches `crt_tube.wgsl`.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct CrtTubeMaterial {
    #[uniform(0)]
    pub params0: Vec4,
    #[uniform(0)]
    pub params1: Vec4,
    #[uniform(0)]
    pub params2: Vec4,
    #[texture(2)]
    #[sampler(3)]
    pub screen: Handle<Image>,
}

impl Material2d for CrtTubeMaterial {
    fn fragment_shader() -> ShaderRef {
        TUBE_SHADER_PATH.into()
    }
}

#[derive(Component, Debug)]
struct CrtTubeQuad;

#[derive(Debug, Default)]
pub struct CrtPlugin;

impl Plugin for CrtPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            MaterialPlugin::<CrtPhosphorMaterial>::default(),
            Material2dPlugin::<CrtTubeMaterial>::default(),
        ))
        .add_systems(Startup, setup_crt_resources)
        .add_systems(Update, (attach_crt_to_television, animate_crt_params));

        let render_app = app.sub_app_mut(RenderApp);
        render_app
            .add_systems(ExtractSchedule, schedule_tube_mips)
            // The tube camera has order -1. Its Core2d output must be complete
            // before mips are generated and the order-0 room camera samples them.
            .add_systems(
                Core2d,
                (copy_tube_canvas, generate_tube_mips)
                    .chain()
                    .after(upscaling),
            );
    }
}

fn schedule_tube_mips(
    mut commands: Commands,
    canvas: Extract<Res<CrtTubeCanvas>>,
    tube: Extract<Res<CrtTubeTexture>>,
    mut jobs: ResMut<MipGenerationJobs>,
) {
    commands.insert_resource(ExtractedTubeImages {
        canvas: canvas.0.clone(),
        tube: tube.0.clone(),
    });
    jobs.add(TUBE_MIP_PHASE, tube.0.id());
}

fn tube_view_is_current(
    current_view: CurrentView,
    cameras: &Query<&ExtractedCamera>,
    images: &ExtractedTubeImages,
) -> bool {
    cameras
        .get(*current_view)
        .ok()
        .and_then(|camera| camera.target.as_ref())
        .is_some_and(|target| {
            matches!(target, NormalizedRenderTarget::Image(image) if image.handle == images.canvas)
        })
}

fn copy_tube_canvas(
    images: Option<Res<ExtractedTubeImages>>,
    current_view: Res<CurrentView>,
    cameras: Query<&ExtractedCamera>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    mut ctx: RenderContext,
) {
    let Some(images) = images else {
        return;
    };
    if !tube_view_is_current(*current_view, &cameras, &images) {
        return;
    }
    let (Some(canvas), Some(tube)) = (gpu_images.get(&images.canvas), gpu_images.get(&images.tube))
    else {
        return;
    };
    ctx.command_encoder().copy_texture_to_texture(
        canvas.texture.as_image_copy(),
        tube.texture.as_image_copy(),
        Extent3d {
            width: TUBE_W,
            height: TUBE_H,
            depth_or_array_layers: 1,
        },
    );
}

fn generate_tube_mips(
    images: Option<Res<ExtractedTubeImages>>,
    current_view: Res<CurrentView>,
    cameras: Query<&ExtractedCamera>,
    jobs: Res<MipGenerationJobs>,
    pipeline_cache: Res<PipelineCache>,
    pipelines: Option<Res<MipGenerationPipelines>>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    mut ctx: RenderContext,
) {
    let Some(images) = images else {
        return;
    };
    if !tube_view_is_current(*current_view, &cameras, &images) {
        return;
    }
    let Some(pipelines) = pipelines else {
        return;
    };
    generate_mips_for_phase(
        TUBE_MIP_PHASE,
        &jobs,
        &pipeline_cache,
        &pipelines,
        &gpu_images,
        &mut ctx,
    );
}

#[must_use]
pub fn aperture_debug_enabled() -> bool {
    // Opt-in via env (baked into SpecChumMac wrapper). Default off so Spectrum shows.
    env_flag("SPEC_CHUM_ROOM_APERTURE_DEBUG")
}

/// Bright room lighting for aperture / CRT screenshot debugging.
#[must_use]
pub fn bright_debug_enabled() -> bool {
    aperture_debug_enabled() || env_flag("SPEC_CHUM_ROOM_BRIGHT_DEBUG")
}

/// Hide phosphor / glass so aperture debug colour is fully visible.
#[must_use]
pub fn hide_crt_enabled() -> bool {
    env_flag("SPEC_CHUM_ROOM_HIDE_CRT") || aperture_debug_enabled()
}

fn env_flag(name: &str) -> bool {
    matches!(
        std::env::var(name).as_deref(),
        Ok("1" | "true" | "TRUE" | "yes" | "YES")
    )
}

/// Local-space offset of the phosphor origin on the TV cabinet.
/// Y shifts by `-BOTTOM_Y_ADJUST/2` so the top edge stays fixed when the bottom moves.
#[must_use]
pub fn crt_phosphor_local() -> Vec3 {
    Vec3::new(
        APERTURE_CENTER_LOCAL.x,
        APERTURE_CENTER_LOCAL.y - BOTTOM_Y_ADJUST * 0.5,
        APERTURE_Z_FRONT - PHOSPHOR_Z_BEHIND,
    )
}

/// World-space CRT pose shared by camera look-at (TV spawn + local phosphor).
#[must_use]
pub fn crt_screen_world_center() -> Vec3 {
    TV_CABINET_POS + crt_phosphor_local()
}

#[derive(Resource)]
struct CrtSpawnKit {
    phosphor_mesh: Handle<Mesh>,
    phosphor_mat: Handle<CrtPhosphorMaterial>,
    glass_mesh: Handle<Mesh>,
    glass_mat: Handle<StandardMaterial>,
    debug_magenta_mesh: Handle<Mesh>,
    debug_magenta_mat: Handle<StandardMaterial>,
}

fn setup_crt_resources(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut phosphor_mats: ResMut<Assets<CrtPhosphorMaterial>>,
    mut tube_mats: ResMut<Assets<CrtTubeMaterial>>,
    mut std_mats: ResMut<Assets<StandardMaterial>>,
) {
    let mut image = Image::new_fill(
        Extent3d {
            width: SCREEN_W,
            height: SCREEN_H,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[0, 0, 0, 255],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::all(),
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::nearest());
    let handle = images.add(image);
    commands.insert_resource(CrtScreenTexture(handle.clone()));

    let mut canvas = Image::new_uninit(
        Extent3d {
            width: TUBE_W,
            height: TUBE_H,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        TextureFormat::Rgba16Float,
        RenderAssetUsages::all(),
    );
    canvas.texture_descriptor.usage |= TextureUsages::COPY_SRC | TextureUsages::RENDER_ATTACHMENT;
    let canvas_handle = images.add(canvas);
    commands.insert_resource(CrtTubeCanvas(canvas_handle.clone()));

    let mut tube = Image::new_uninit(
        Extent3d {
            width: TUBE_W,
            height: TUBE_H,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        TextureFormat::Rgba16Float,
        RenderAssetUsages::all(),
    );
    tube.texture_descriptor.mip_level_count = TUBE_MIP_COUNT;
    tube.texture_descriptor.usage |= TextureUsages::STORAGE_BINDING;
    tube.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
    let tube_handle = images.add(tube);
    commands.insert_resource(CrtTubeTexture(tube_handle.clone()));

    let tube_mat = tube_mats.add(CrtTubeMaterial {
        params0: Vec4::new(0.0, 0.18, 0.10, 1.85),
        params1: Vec4::new(0.08, PHOSPHOR_W / PHOSPHOR_MESH_H, 0.0, 0.0),
        params2: Vec4::new(
            if crate::quality::material_halation_enabled() {
                1.0
            } else {
                0.0
            },
            0.0,
            0.0,
            0.0,
        ),
        screen: handle,
    });
    commands.spawn((
        Camera2d,
        Hdr,
        Msaa::Off,
        Camera {
            order: -1,
            clear_color: ClearColorConfig::Custom(Color::BLACK),
            ..default()
        },
        RenderTarget::Image(canvas_handle.into()),
        TUBE_LAYER,
        Name::new("crt_tube_camera"),
    ));
    commands.spawn((
        Mesh2d(meshes.add(Rectangle::new(TUBE_W as f32, TUBE_H as f32))),
        MeshMaterial2d(tube_mat),
        Transform::default(),
        TUBE_LAYER,
        CrtTubeQuad,
        Name::new("crt_tube_quad"),
    ));

    let phosphor_mesh = meshes.add(bulging_screen_mesh(
        PHOSPHOR_W,
        PHOSPHOR_MESH_H,
        PHOSPHOR_BULGE,
        176,
        148,
    ));
    let phosphor_mat = phosphor_mats.add(CrtPhosphorMaterial { tube: tube_handle });
    let glass_mesh = meshes.add(bulging_screen_mesh(
        PHOSPHOR_W * 1.002,
        PHOSPHOR_MESH_H * 1.002,
        PHOSPHOR_BULGE * 1.04,
        48,
        36,
    ));
    // Keep just a soft glass sheen. Strong specular reflections moved across the
    // curved screen during the opening and vanished at the frontal settle.
    let glass_mat = std_mats.add(StandardMaterial {
        base_color: Color::srgba(0.55, 0.65, 0.75, 0.06),
        perceptual_roughness: 0.28,
        reflectance: 0.32,
        metallic: 0.0,
        alpha_mode: AlphaMode::Blend,
        ..default()
    });

    // Magenta debug = exact phosphor size/bulge (reference aperture visualisation).
    let debug_magenta_mesh = meshes.add(bulging_screen_mesh(
        PHOSPHOR_W,
        PHOSPHOR_MESH_H,
        PHOSPHOR_BULGE,
        96,
        72,
    ));
    let debug_magenta_mat = std_mats.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.0, 1.0),
        emissive: LinearRgba::rgb(5.0, 0.0, 5.0),
        unlit: true,
        alpha_mode: AlphaMode::Opaque,
        ..default()
    });

    commands.insert_resource(CrtSpawnKit {
        phosphor_mesh,
        phosphor_mat,
        glass_mesh,
        glass_mat,
        debug_magenta_mesh,
        debug_magenta_mat,
    });
}

fn attach_crt_to_television(
    mut commands: Commands,
    kit: Option<Res<CrtSpawnKit>>,
    tv: Query<Entity, (With<TelevisionCabinet>, Without<CrtAttachedToTv>)>,
) {
    let Some(kit) = kit else {
        return;
    };
    let Ok(tv_entity) = tv.single() else {
        return;
    };

    let local = crt_phosphor_local();
    let hide_crt = hide_crt_enabled();
    let debug = aperture_debug_enabled();

    if debug {
        // Same pose/size as phosphor — bright magenta so underlap + barrel are obvious.
        commands.entity(tv_entity).with_children(|parent| {
            parent.spawn((
                Mesh3d(kit.debug_magenta_mesh.clone()),
                MeshMaterial3d(kit.debug_magenta_mat.clone()),
                Transform::from_translation(local),
                ApertureDebugMarker,
                Name::new("aperture_debug_magenta"),
            ));
        });
        bevy::log::info!(
            "SPEC_CHUM_ROOM_APERTURE_DEBUG: magenta phosphor {PHOSPHOR_W:.4}×{PHOSPHOR_MESH_H:.4} \
             aspect={:.3} (4:3) bottom_adj={BOTTOM_Y_ADJUST:.4} bulge={PHOSPHOR_BULGE:.3} \
             overscan={OVERSCAN_FRAC:.3} z_behind={PHOSPHOR_Z_BEHIND:.3}",
            PHOSPHOR_W / PHOSPHOR_MESH_H
        );
    }

    if hide_crt {
        bevy::log::info!("SPEC_CHUM_ROOM_HIDE_CRT: phosphor/glass not spawned");
    } else {
        commands.entity(tv_entity).with_children(|parent| {
            parent.spawn((
                Mesh3d(kit.phosphor_mesh.clone()),
                MeshMaterial3d(kit.phosphor_mat.clone()),
                Transform::from_translation(local),
                CrtPhosphor,
                Name::new("crt_phosphor"),
            ));
            parent.spawn((
                Mesh3d(kit.glass_mesh.clone()),
                MeshMaterial3d(kit.glass_mat.clone()),
                Transform::from_translation(local + Vec3::new(0.0, 0.0, 0.002)),
                CrtGlass,
                Name::new("crt_glass"),
            ));
        });
    }

    commands.entity(tv_entity).insert(CrtAttachedToTv);
}

fn animate_crt_params(
    time: Res<Time>,
    opening: Option<Res<crate::camera::OpeningSequence>>,
    mut materials: ResMut<Assets<CrtTubeMaterial>>,
    query: Query<&MeshMaterial2d<CrtTubeMaterial>, With<CrtTubeQuad>>,
) {
    // Keep phosphor reconstruction independent of camera zoom. Geometry and
    // camera bloom provide the room-scale change in appearance.
    const SCAN_STRENGTH: f32 = 0.48;
    const GRILLE_STRENGTH: f32 = 0.28;
    const HORIZONTAL_SOFT_MIX: f32 = 0.38;
    const BRIGHTNESS: f32 = 2.30;
    let power = opening.map_or(1.0, |sequence| sequence.crt_power());
    for handle in &query {
        if let Some(mut mat) = materials.get_mut(handle) {
            mat.params0.x = time.elapsed_secs();
            mat.params0.y = SCAN_STRENGTH;
            mat.params0.z = GRILLE_STRENGTH;
            mat.params0.w = BRIGHTNESS;
            mat.params1.x = HORIZONTAL_SOFT_MIX;
            mat.params1.y = PHOSPHOR_W / PHOSPHOR_MESH_H;
            mat.params2.y = power;
        }
    }
}

#[must_use]
pub fn overscan_sample_uv(uv: Vec2) -> Vec2 {
    uv
}

/// Soft convex CRT face (barrel toward viewer), UV 0..1, bulge toward +Z.
#[must_use]
pub fn bulging_screen_mesh(width: f32, height: f32, bulge: f32, seg_x: u32, seg_y: u32) -> Mesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();

    // Mild spherical section — stronger H than V; soft falloff (reference CRT barrel).
    const H_CURVE: f32 = 0.28;
    const V_CURVE: f32 = 0.16;

    for y in 0..=seg_y {
        let v = y as f32 / seg_y as f32;
        let ny = v * 2.0 - 1.0;
        for x in 0..=seg_x {
            let u = x as f32 / seg_x as f32;
            let nx = u * 2.0 - 1.0;
            let z = bulge * (1.0 - H_CURVE * nx * nx) * (1.0 - V_CURVE * ny * ny);
            positions.push([nx * width * 0.5, -ny * height * 0.5, z]);
            let hx = 1.0 - H_CURVE * nx * nx;
            let vy = 1.0 - V_CURVE * ny * ny;
            let z_grad_x = bulge * (-2.0 * H_CURVE * nx) * vy;
            let z_grad_y = bulge * hx * (-2.0 * V_CURVE * ny);
            let n =
                Vec3::new(-z_grad_x / (width * 0.5), z_grad_y / (height * 0.5), 1.0).normalize();
            normals.push(n.to_array());
            uvs.push([u, v]);
        }
    }

    let stride = seg_x + 1;
    for y in 0..seg_y {
        for x in 0..seg_x {
            let i0 = y * stride + x;
            let i1 = i0 + 1;
            let i2 = i0 + stride;
            let i3 = i2 + 1;
            indices.extend_from_slice(&[i0, i2, i1, i1, i2, i3]);
        }
    }

    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bulge_mesh_has_expected_vertex_count() {
        let mesh = bulging_screen_mesh(1.0, 1.0, 0.1, 4, 3);
        assert_eq!(mesh.count_vertices(), 5 * 4);
    }

    #[test]
    fn phosphor_overscans_geometric_aperture() {
        const {
            assert!(PHOSPHOR_W > PHOSPHOR_BASE_W);
            assert!(PHOSPHOR_H > PHOSPHOR_BASE_H);
        }
        let edge = (PHOSPHOR_W - PHOSPHOR_BASE_W) * 0.5;
        assert!((edge - PHOSPHOR_BASE_W * OVERSCAN_FRAC).abs() < 1e-4);
    }

    #[test]
    fn phosphor_mesh_is_four_by_three() {
        let aspect = PHOSPHOR_W / PHOSPHOR_H;
        assert!(
            (aspect - CONTENT_ASPECT).abs() < 0.002,
            "base mesh aspect={aspect} (before bottom adjust)"
        );
        // After bottom inset, aspect rises slightly above 4:3 — still near classic.
        let mesh_aspect = PHOSPHOR_W / PHOSPHOR_MESH_H;
        assert!(
            mesh_aspect >= CONTENT_ASPECT - 0.002,
            "mesh_aspect={mesh_aspect}"
        );
        const {
            assert!(BOTTOM_Y_ADJUST <= 0.0);
        }
    }

    #[test]
    fn bottom_adjust_keeps_top_edge() {
        let top_nominal = APERTURE_CENTER_LOCAL.y + PHOSPHOR_H * 0.5;
        let top_mesh = crt_phosphor_local().y + PHOSPHOR_MESH_H * 0.5;
        assert!((top_nominal - top_mesh).abs() < 1e-5);
        let bot_nominal = APERTURE_CENTER_LOCAL.y - PHOSPHOR_H * 0.5;
        let bot_mesh = crt_phosphor_local().y - PHOSPHOR_MESH_H * 0.5;
        // Negative adjust → bottom moves up (bot_mesh > bot_nominal).
        assert!((bot_mesh - bot_nominal - (-BOTTOM_Y_ADJUST)).abs() < 1e-5);
    }

    #[test]
    fn aperture_matches_punched_glass_aabb() {
        const {
            assert!(APERTURE_W > APERTURE_H);
        }
        // Full bottom punch → opening aspect ~1.33 (was 1.525 before bottom lip removed).
        let aspect = APERTURE_W / APERTURE_H;
        assert!((aspect - 1.335).abs() < 0.03, "aspect={aspect}");
        assert!((APERTURE_W - GLASS_AABB_W).abs() < 1e-4);
        assert!((APERTURE_H - GLASS_AABB_H).abs() < 1e-4);
    }

    #[test]
    fn phosphor_peak_stays_near_rim() {
        let peak = (APERTURE_Z_FRONT - PHOSPHOR_Z_BEHIND) + PHOSPHOR_BULGE;
        // Flush policy: peak may sit at/near the rim; keep a tiny epsilon behind.
        assert!(
            peak <= APERTURE_Z_FRONT + 0.002,
            "peak={peak} rim={APERTURE_Z_FRONT}"
        );
        const {
            assert!(PHOSPHOR_Z_BEHIND < 0.01);
        }
    }

    #[test]
    fn screen_world_center_uses_the_spawned_television_pose() {
        let expected = crate::room::TV_CABINET_POS + crt_phosphor_local();
        let actual = crt_screen_world_center();
        assert!((actual - expected).length() < 1e-6);
        assert!((actual.y - 0.8368).abs() < 1e-4);
    }

    #[test]
    fn rim_plane_narrower_than_aperture() {
        const {
            assert!(RIM_PLANE_W < APERTURE_W);
            assert!(RIM_PLANE_H <= APERTURE_H);
        }
    }

    #[test]
    fn bulge_centre_is_closer_than_edges() {
        const H: f32 = 0.28;
        const V: f32 = 0.16;
        let z_c = PHOSPHOR_BULGE;
        let z_e = PHOSPHOR_BULGE * (1.0 - H);
        let z_corner = PHOSPHOR_BULGE * (1.0 - H) * (1.0 - V);
        assert!(z_c > z_e && z_e > z_corner);
        assert!(z_e / z_c > 0.65);
    }
}
