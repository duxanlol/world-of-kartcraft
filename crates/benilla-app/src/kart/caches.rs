//! The caches (World of Kartcraft, `Kartcraft.cpp` "Caches"): the Astral and Wumpa Caches, chests
//! the server keeps on spots along the drivable roads. A driver claims one by driving through it:
//! the kart within [`TOUCH`] of one tells the server (`R\tAC\t<guid>`), which checks the distance
//! and that it is still there, pays, and takes the chest away (`R\tACGot`, the layer's toast). On
//! foot the chest opens as any does. An Astral Cache stands in a column of light, seen from afar.

use std::collections::HashMap;

use benilla_protocol::EntityKind;
use bevy::prelude::*;
use kartcraft::KartView;

use crate::net::{ClientCommand, Guid, NetCommands, NetEntity};

/// The caches' gameobject entries (`pack.py` `CACHE_ASTRAL`, `CACHE_WUMPA`).
const ENTRIES: [u32; 2] = [190_200, 190_201];
/// How near (yards) the kart passes for a touch: into the chest, well inside the server's 12.
const TOUCH: f32 = 6.0;
/// A touched cache is told of again no sooner than this (s), the first touch's answer under way.
const AGAIN: f32 = 30.0;

/// The Astral Cache's entry: the rare one, under a beam.
const ASTRAL: u32 = 190_200;
/// The beam: its height and radius (yards), and its colour (linear, additive).
const BEAM_HEIGHT: f32 = 90.0;
const BEAM_RADIUS: f32 = 0.7;
const BEAM_COLOUR: [f32; 3] = [0.9, 0.7, 0.25];

/// An Astral Cache's beam is up.
#[derive(Component)]
pub(crate) struct Beamed;

/// The beam's mesh and material, made once.
#[derive(Resource, Default)]
pub(crate) struct Beam(Option<(Handle<Mesh>, Handle<StandardMaterial>)>);

/// Puts a beam over each Astral Cache in sight; it goes with the chest.
pub(crate) fn beam(
    mut commands: Commands,
    mut beam: ResMut<Beam>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    chests: Query<(Entity, &Guid, &NetEntity, &GlobalTransform), Without<Beamed>>,
) {
    for (entity, guid, net, at) in &chests {
        if net.kind != EntityKind::GameObject
            || benilla_protocol::guid::entry(guid.0) != Some(ASTRAL)
        {
            continue;
        }
        let (mesh, material) = beam
            .0
            .get_or_insert_with(|| {
                let [r, g, b] = BEAM_COLOUR;
                (
                    meshes.add(Cylinder::new(BEAM_RADIUS, BEAM_HEIGHT)),
                    materials.add(StandardMaterial {
                        base_color: Color::linear_rgba(r, g, b, 0.35),
                        emissive: LinearRgba::rgb(r, g, b),
                        alpha_mode: AlphaMode::Add,
                        unlit: true,
                        double_sided: true,
                        cull_mode: None,
                        ..default()
                    }),
                )
            })
            .clone();
        // Upright over the chest whatever its turn and size, its foot in it.
        let (scale, rotation, _) = at.to_scale_rotation_translation();
        let unscale = Vec3::ONE / scale.max(Vec3::splat(0.01));
        let up = rotation.inverse() * (Vec3::Y * (BEAM_HEIGHT / 2.0)) * unscale;
        let column = commands
            .spawn((
                Mesh3d(mesh),
                MeshMaterial3d(material),
                Transform::from_translation(up)
                    .with_rotation(rotation.inverse())
                    .with_scale(unscale),
                Name::new("astral cache beam"),
            ))
            .id();
        commands.entity(entity).add_child(column).insert(Beamed);
    }
}

/// The caches told of, and when.
#[derive(Resource, Default)]
pub(crate) struct Touched(HashMap<u64, f32>);

/// The kart through a cache: the server told, once.
pub(crate) fn touch(
    view: Res<KartView>,
    time: Res<Time>,
    mut touched: ResMut<Touched>,
    net: Option<Res<NetCommands>>,
    chests: Query<(&Guid, &NetEntity, &GlobalTransform)>,
) {
    if !view.active || view.hidden {
        return;
    }
    let Some(net) = net else {
        return;
    };
    let now = time.elapsed_secs();
    touched.0.retain(|_, at| now - *at < AGAIN);
    let kart = view.transform.translation;
    for (guid, entity, at) in &chests {
        if entity.kind != EntityKind::GameObject
            || !benilla_protocol::guid::entry(guid.0).is_some_and(|e| ENTRIES.contains(&e))
            || at.translation().distance(kart) > TOUCH
            || touched.0.contains_key(&guid.0)
        {
            continue;
        }
        touched.0.insert(guid.0, now);
        debug!("kart caches: touched {:#x}", guid.0);
        let _ = net.0.send(ClientCommand::AddonMessage {
            distribution: benilla_ui::script::AddonDistribution::Party,
            text: format!("KARTCRAFT\tR\tAC\t{}", guid.0),
        });
    }
}
