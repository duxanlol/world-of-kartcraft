//! The caches (World of Kartcraft, `Kartcraft.cpp` "Caches"): the Astral and Wumpa Caches, chests
//! the server keeps on spots along the drivable roads. A driver claims one by driving through it:
//! the kart within [`TOUCH`] of one tells the server (`R\tAC\t<guid>`), which checks the distance
//! and that it is still there, pays, and takes the chest away (`R\tACGot`, the layer's toast). On
//! foot the chest opens as any does.

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
