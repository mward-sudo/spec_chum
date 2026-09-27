//! Blender-baked static room for the temporary #149 Current/New visual comparison.
//!
//! The bake contains only static room geometry. The procedural television,
//! cabinet, phosphor and framebuffer-driven spill remain live in both variants.

use bevy::gltf::GltfMeshName;
use bevy::pbr::Lightmap;
use bevy::prelude::*;

use crate::camera::OpeningSequence;
use crate::hybrid::RoomStatic;
use crate::scene_variant::{SceneVariant, SceneVariantOnly};

const SCENE: &str = "lightmaps/room_static.gltf#Scene0";
const IMAGE: &str = "lightmaps/room_static_lightmap.png";
const BAKED_MESH_NAME: &str = "spec_chum_room_static_baked";

/// Present only when a complete local bake is available for the procedural room.
#[derive(Resource, Debug)]
pub struct BakedRoomEnabled {
    lightmap: Handle<Image>,
}

#[derive(Component, Debug)]
struct BakedRoomSurface {
    material: Handle<StandardMaterial>,
    base_lightmap_exposure: f32,
    base_emissive: LinearRgba,
}

#[derive(Debug, Default)]
pub struct BakedRoomPlugin;

impl Plugin for BakedRoomPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_baked_room)
            .add_systems(PreUpdate, mark_procedural_static_for_current)
            .add_systems(
                Update,
                (bind_baked_lightmap, animate_baked_lighting).chain(),
            );
    }
}

fn spawn_baked_room(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    variant: Res<SceneVariant>,
    #[cfg(feature = "skein")] skein_mode: Option<Res<crate::skein::SkeinRoomMode>>,
) {
    #[cfg(feature = "skein")]
    if skein_mode
        .as_deref()
        .is_some_and(crate::skein::SkeinRoomMode::replaces_procedural)
    {
        return;
    }

    let asset_root = crate::resolve_asset_root();
    let baked = asset_root.join("lightmaps/room_static.gltf");
    let atlas = asset_root.join(IMAGE);
    if !baked.is_file() || !atlas.is_file() {
        bevy::log::info!("#149 Blender bake absent; New retains the comparison stub");
        return;
    }

    commands.insert_resource(BakedRoomEnabled {
        lightmap: asset_server.load(IMAGE),
    });
    commands.spawn((
        WorldAssetRoot(asset_server.load(SCENE)),
        SceneVariantOnly(SceneVariant::New),
        RoomStatic,
        if *variant == SceneVariant::New {
            Visibility::Visible
        } else {
            Visibility::Hidden
        },
        Name::new("spec_chum_baked_static_room"),
    ));
    bevy::log::info!("#149 Blender lightmap enabled for New room variant");
}

/// The procedural room's static roots are hidden only when a bake is available.
fn mark_procedural_static_for_current(
    mut commands: Commands,
    baked: Option<Res<BakedRoomEnabled>>,
    static_roots: Query<Entity, (With<RoomStatic>, Without<SceneVariantOnly>)>,
) {
    if baked.is_none() {
        return;
    }
    for entity in &static_roots {
        commands
            .entity(entity)
            .insert(SceneVariantOnly(SceneVariant::Current));
    }
}

/// Blender exports the atlas UVs as `TEXCOORD_1` on each material primitive.
/// Bevy's glTF loader creates one entity per primitive, so attach the same atlas
/// to each child with the baked mesh name.
fn bind_baked_lightmap(
    mut commands: Commands,
    baked: Option<Res<BakedRoomEnabled>>,
    meshes: Res<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    loaded: Query<
        (
            Entity,
            &GltfMeshName,
            &Mesh3d,
            &MeshMaterial3d<StandardMaterial>,
        ),
        Without<Lightmap>,
    >,
) {
    let Some(baked) = baked else {
        return;
    };
    for (entity, name, mesh, source_material) in &loaded {
        if !name.starts_with(BAKED_MESH_NAME) {
            continue;
        }
        if meshes
            .get(&mesh.0)
            .is_none_or(|mesh| mesh.attribute(Mesh::ATTRIBUTE_UV_1).is_none())
        {
            bevy::log::error!("#149 baked mesh {name:?} has no TEXCOORD_1; regenerate lightmaps");
            continue;
        }
        let Some(mut material) = materials.get(&source_material.0).cloned() else {
            continue;
        };
        let base_lightmap_exposure = material.lightmap_exposure;
        let base_emissive = material.emissive;
        // The baked atlas and exported bulb glow otherwise appear at full
        // strength, bypassing the room's opening light animation.
        material.lightmap_exposure = 0.0;
        material.emissive = LinearRgba::BLACK;
        let material = materials.add(material);
        commands.entity(entity).insert((
            Lightmap {
                image: baked.lightmap.clone(),
                ..default()
            },
            MeshMaterial3d(material.clone()),
            BakedRoomSurface {
                material,
                base_lightmap_exposure,
                base_emissive,
            },
        ));
    }
}

fn animate_baked_lighting(
    opening: Res<OpeningSequence>,
    surfaces: Query<&BakedRoomSurface>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let room_gain = opening.light_gain();
    let practical_gain = opening.practical_light_gain();
    for surface in &surfaces {
        let Some(mut material) = materials.get_mut(&surface.material) else {
            continue;
        };
        material.lightmap_exposure = surface.base_lightmap_exposure * room_gain;
        material.emissive = surface.base_emissive * practical_gain;
    }
}
