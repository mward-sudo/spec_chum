//! Blender-baked static room for the temporary #149 Current/New visual comparison.
//!
//! The bake contains only static room geometry. The procedural television,
//! cabinet, phosphor and framebuffer-driven spill remain live in both variants.

use bevy::gltf::GltfMeshName;
use bevy::pbr::Lightmap;
use bevy::prelude::*;
use bevy::world_serialization::{WorldInstance, WorldInstanceSpawner};

use crate::camera::OpeningSequence;
use crate::hybrid::RoomStatic;
use crate::scene_variant::{SceneVariant, SceneVariantOnly};

const SCENE: &str = "lightmaps/room_static.gltf#Scene0";
const IMAGE: &str = "lightmaps/room_static_lightmap.png";
const BAKED_MESH_NAME: &str = "spec_chum_room_static_baked";

fn should_spawn_baked_room(presentation: crate::cabinet_room::RoomPresentation) -> bool {
    presentation != crate::cabinet_room::RoomPresentation::FixedCabinet
}

fn all_baked_meshes_bound(mesh_count: usize, bound_count: usize) -> bool {
    mesh_count > 0 && mesh_count == bound_count
}

/// Present only when a complete local bake is available for the procedural room.
#[derive(Resource, Debug)]
pub struct BakedRoomEnabled;

#[derive(Component, Debug)]
struct PendingBakedRoom {
    lightmap: Handle<Image>,
}

type ProceduralStaticRootFilter = (
    With<RoomStatic>,
    Without<SceneVariantOnly>,
    Without<PendingBakedRoom>,
);
type BakedMeshPrimitiveQuery<'a> = (
    &'a GltfMeshName,
    Option<&'a Mesh3d>,
    Option<&'a MeshMaterial3d<StandardMaterial>>,
    Option<&'a BakedRoomSurface>,
    Option<&'a BakedMeshInvalid>,
);

#[derive(Component, Debug)]
struct BakedRoomSurface {
    material: Handle<StandardMaterial>,
    base_lightmap_exposure: f32,
    base_emissive: LinearRgba,
}

#[derive(Component, Debug)]
struct BakedMeshInvalid;

#[derive(Debug, Default)]
pub struct BakedRoomPlugin;

impl Plugin for BakedRoomPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_baked_room).add_systems(
            Update,
            (bind_baked_lightmap, animate_baked_lighting).chain(),
        );
    }
}

fn spawn_baked_room(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    presentation: Res<crate::cabinet_room::RoomPresentation>,
    #[cfg(feature = "skein")] skein_mode: Option<Res<crate::skein::SkeinRoomMode>>,
) {
    if !should_spawn_baked_room(*presentation) {
        return;
    }

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

    commands.spawn((
        WorldAssetRoot(asset_server.load(SCENE)),
        PendingBakedRoom {
            lightmap: asset_server.load(IMAGE),
        },
        RoomStatic,
        Visibility::Hidden,
        Name::new("spec_chum_baked_static_room"),
    ));
}

/// Blender exports the atlas UVs as `TEXCOORD_1` on each material primitive.
/// Bevy's glTF loader creates one entity per primitive, so attach the same atlas
/// to each primitive. Keep the procedural room visible until every primitive
/// in the ready world instance has received its lightmap material.
fn bind_baked_lightmap(
    mut commands: Commands,
    variant: Res<SceneVariant>,
    mut world_instances: ResMut<WorldInstanceSpawner>,
    meshes: Res<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    pending_roots: Query<(Entity, &PendingBakedRoom, &WorldInstance)>,
    loaded: Query<BakedMeshPrimitiveQuery<'_>>,
    static_roots: Query<Entity, ProceduralStaticRootFilter>,
) {
    for (root, pending, instance) in &pending_roots {
        if !world_instances.instance_is_ready(**instance) {
            continue;
        }

        let mut mesh_count = 0;
        let mut bound_count = 0;
        let mut invalid_mesh = false;
        for entity in world_instances.iter_instance_entities(**instance) {
            let Ok((name, mesh, source_material, surface, invalid)) = loaded.get(entity) else {
                continue;
            };
            if !name.starts_with(BAKED_MESH_NAME) {
                continue;
            }
            mesh_count += 1;
            if surface.is_some() {
                bound_count += 1;
                continue;
            }
            if invalid.is_some() {
                invalid_mesh = true;
                continue;
            }

            let (Some(mesh), Some(source_material)) = (mesh, source_material) else {
                continue;
            };
            let Some(baked_mesh) = meshes.get(&mesh.0) else {
                continue;
            };
            if baked_mesh.attribute(Mesh::ATTRIBUTE_UV_1).is_none() {
                bevy::log::error!(
                    "#149 baked mesh {name:?} has no TEXCOORD_1; regenerate lightmaps"
                );
                commands.entity(entity).insert(BakedMeshInvalid);
                invalid_mesh = true;
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
                    image: pending.lightmap.clone(),
                    ..default()
                },
                MeshMaterial3d(material.clone()),
                BakedRoomSurface {
                    material,
                    base_lightmap_exposure,
                    base_emissive,
                },
            ));
            bound_count += 1;
        }

        if invalid_mesh {
            world_instances.despawn_instance(**instance);
            commands.entity(root).despawn();
            bevy::log::warn!("#149 Blender bake rejected; keeping the procedural room visible");
            continue;
        }

        if !all_baked_meshes_bound(mesh_count, bound_count) {
            continue;
        }

        commands.entity(root).insert((
            SceneVariantOnly(SceneVariant::New),
            if *variant == SceneVariant::New {
                Visibility::Visible
            } else {
                Visibility::Hidden
            },
        ));
        commands.entity(root).remove::<PendingBakedRoom>();
        commands.insert_resource(BakedRoomEnabled);
        for static_root in &static_roots {
            commands.entity(static_root).insert((
                SceneVariantOnly(SceneVariant::Current),
                if *variant == SceneVariant::New {
                    Visibility::Hidden
                } else {
                    Visibility::Visible
                },
            ));
        }
        bevy::log::info!("#149 Blender lightmap enabled after every baked mesh was bound");
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

#[cfg(test)]
mod tests {
    use super::{all_baked_meshes_bound, should_spawn_baked_room};
    use crate::cabinet_room::RoomPresentation;

    #[test]
    fn fixed_cabinet_does_not_spawn_the_baked_three_dimensional_room() {
        assert!(!should_spawn_baked_room(RoomPresentation::FixedCabinet));
        assert!(should_spawn_baked_room(RoomPresentation::ThreeDimensional));
    }

    #[test]
    fn baked_room_waits_for_every_mesh_lightmap() {
        assert!(!all_baked_meshes_bound(0, 0));
        assert!(!all_baked_meshes_bound(3, 2));
        assert!(all_baked_meshes_bound(3, 3));
    }
}
