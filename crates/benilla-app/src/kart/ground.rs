//! The ground under our kart, WoW's surface read as one of CTR's grounds ([`Ground`], CTR's
//! terrain table). WoW's own answer is the footsteps' (`world.terrain_type`: the ground-effect
//! layer under the feet → `TerrainType` → its `SoundID` class); CTR's is a quadblock's
//! `terrain_type`. The mapping, by `TerrainType.SoundID`:
//!
//! | WoW sound class | CTR ground | top speed | engine |
//! |---|---|---|---|
//! | 0 None (a building's default), 2 Metallic, 3 Stone, 5 Wood | asphalt, metal, stone, wood | 256 | 256 |
//! | 1 Dirt, 8 Sand | dirt | 140 | 180 |
//! | 6 Grass, 7 Leaves | grass | 140 | 180 |
//! | 4 Snow | snow (and half the grip) | 140 | 180 |
//! | 9 Soggy (marsh) | mud (twice the drag, mud's floors) | 140 | 180 |
//! | under a water surface | water | 180 | 200 |
//! | above or below the terrain's heightfield (a building, a bridge, a doodad, a cave) | asphalt | 256 | 256 |
//!
//! WoW has no ice ground (Winterspring's ice is Snow), so CTR's ice is never read. The ground is
//! read every frame whatever `KartTerrain` says, for the skid and rolling sounds; the setting
//! decides only whether the kart drives on it ([`kartcraft::Session::set_ground`]).

use benilla_assets::coords::bevy_to_wow;
use benilla_world::world_point::{Subject, WorldPoint};
use bevy::prelude::*;
use kartcraft::{Ground, KartView};

use crate::sound::footsteps::Footsteps;

/// How far (yards) the kart's origin may stand above the terrain's heightfield and still be on
/// it: higher, it stands on a building, a bridge or a doodad.
const ABOVE_TERRAIN: f32 = 0.6;
/// How far below it: lower, it is in a cave or a cellar.
const BELOW_TERRAIN: f32 = 1.5;

/// A `TerrainType.SoundID` class as CTR's ground.
pub(crate) fn ground_of(sound_class: u32) -> Ground {
    match sound_class {
        1 | 8 => Ground::Dirt,
        2 => Ground::Metal,
        3 => Ground::Stone,
        4 => Ground::Snow,
        5 => Ground::Wood,
        6 | 7 => Ground::Grass,
        9 => Ground::Mud,
        _ => Ground::Asphalt,
    }
}

/// The ground under the kart: water when a water surface is over it, asphalt off the terrain
/// (or with no layer there), else the layer's.
pub(crate) fn ground_under(in_water: bool, on_terrain: bool, sound_class: Option<u32>) -> Ground {
    if in_water {
        Ground::Water
    } else if !on_terrain {
        Ground::Asphalt
    } else {
        sound_class.map_or(Ground::Asphalt, ground_of)
    }
}

/// Reads the ground under our kart where it was last drawn and hands it to the session, which
/// drives on it with `KartTerrain` on. Through a hop it keeps the ground it left.
pub(super) fn sample(
    view: Res<KartView>,
    settings: Res<super::settings::KartSettings>,
    world: WorldPoint,
    footsteps: Option<Res<Footsteps>>,
    mut host: ResMut<super::Host>,
) {
    if !view.active || !view.grounded {
        let ground = view.ground;
        host.session.set_ground(ground, settings.terrain);
        return;
    }
    let at = view.transform.translation;
    let wow = bevy_to_wow(at);
    let in_water = world
        .water_surface_at(Subject::Player, wow)
        .is_some_and(|surface| surface > at.y + 0.1);
    let on_terrain = world
        .terrain_height_under(at)
        .is_some_and(|h| (h - BELOW_TERRAIN..=h + ABOVE_TERRAIN).contains(&at.y));
    let class = footsteps.as_deref().and_then(|f| {
        let terrain = world.terrain_type(&f.0, Subject::Player, at)?;
        f.0.sound_class_of(terrain)
    });
    let ground = ground_under(in_water, on_terrain, class);
    host.session.set_ground(ground, settings.terrain);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wow_s_grounds_map_to_ctr_s() {
        let speed = |class: u32| ground_of(class).terrain().speed;
        // Hard surfaces drive as asphalt.
        for class in [0, 2, 3, 5] {
            assert_eq!(speed(class), 256, "class {class}");
        }
        // Off-road is CTR's off-road: 140/256 of the top speed.
        for class in [1, 4, 6, 7, 8, 9] {
            assert_eq!(speed(class), 140, "class {class}");
        }
        assert_eq!(ground_of(6), Ground::Grass);
        assert_eq!(ground_of(8), Ground::Dirt);
        assert_eq!(ground_of(4), Ground::Snow);
        assert_eq!(ground_of(9), Ground::Mud);
        assert_eq!(ground_of(42), Ground::Asphalt, "an unknown class");
        // Water over the kart wins; off the terrain (a building, a bridge) is asphalt; no layer
        // is asphalt.
        assert_eq!(ground_under(true, true, Some(6)), Ground::Water);
        assert_eq!(ground_under(true, false, None), Ground::Water);
        assert_eq!(ground_under(false, false, Some(6)), Ground::Asphalt);
        assert_eq!(ground_under(false, true, None), Ground::Asphalt);
        assert_eq!(ground_under(false, true, Some(6)), Ground::Grass);
    }
}
