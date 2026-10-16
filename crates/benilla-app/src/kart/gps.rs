//! The courier delivery's GPS. While a delivery is carried (the server's `Del <town> <par>
//! <gone>`, until `Del-`), the way by road from the kart to the town's innkeeper over the GPS
//! roads (`roads/roads.json`, [`kart_gps`]), found again whenever the kart strays from it:
//! - the race guide arrow points along it, a stretch ahead, and the minimap marks the town
//!   ([`super::race::guide`]);
//! - chevrons lie on the way ahead ([`chevrons`]);
//! - the delivery clock says how far is left by road (`BENILLA_KART_GPS`).

use std::collections::HashMap;
use std::sync::LazyLock;

use benilla_assets::coords::{bevy_to_wow, wow_to_bevy};
use benilla_protocol::SessionEvent;
use benilla_world::collision::WorldCollision;
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use kart_gps::{RoadFile, Router};

/// Farther than this (yards) from the way, it is found again.
const STRAY: f32 = 30.0;
/// It is found again at most this often, seconds.
const REROUTE_EVERY: f32 = 1.0;
/// The courier deliveries' quests: `190200 + 64 × from + to` (`setup/vmangos/pack.py`).
const DELIVERY_QUESTS: std::ops::RangeInclusive<u32> = 190_201..=190_200 + 64 * 64;
/// How long (seconds) a delivery's quest may be missing from the log before the delivery is
/// dropped: abandoned, it is gone, but at login the log can come a moment after the line.
const QUEST_GONE: f32 = 1.0;
/// Closer than this (yards) to the road's end, the arrow points at the town's innkeeper.
const ARRIVE: f32 = 15.0;
/// How far along the way (yards) the arrow points.
const LOOKAHEAD: f32 = 40.0;
/// A chevron every this many yards along the way, this far ahead of the kart.
const CHEVRON_EVERY: f32 = 12.0;
const CHEVRONS_AHEAD: f32 = 180.0;
/// How far over the ground (yards) a chevron floats, and how far above the way's own height the
/// ground under it is looked for (a bridge's deck, not the gorge; a tunnel's floor, not the hill).
const CHEVRON_LIFT: f32 = 0.15;
const GROUND_PROBE: f32 = 6.0;

/// Each map's GPS roads, ready to route on.
static ROADS: LazyLock<HashMap<u32, Router>> = LazyLock::new(|| {
    let file =
        RoadFile::parse(include_str!("../../../../roads/roads.json")).expect("roads/roads.json");
    file.maps.iter().map(|m| (m.map, Router::new(m))).collect()
});

/// Parses the GPS roads and the towns off the main thread at startup, so the first delivery
/// does not wait on them.
pub(crate) fn preload() {
    std::thread::spawn(|| {
        LazyLock::force(&ROADS);
        LazyLock::force(&TOWNS);
    });
}

/// A delivery town: the name the server says, its map, and its innkeeper's place (WoW yards).
struct Town {
    name: String,
    map: u32,
    at: [f32; 3],
}

static TOWNS: LazyLock<Vec<Town>> = LazyLock::new(|| {
    #[derive(serde::Deserialize)]
    struct File {
        towns: Vec<Row>,
    }
    #[derive(serde::Deserialize)]
    struct Row {
        name: String,
        map: u32,
        x: f32,
        y: f32,
        z: f32,
    }
    let file: File = serde_json::from_str(include_str!("../../../../deliveries/deliveries.json"))
        .expect("deliveries/deliveries.json");
    file.towns
        .into_iter()
        .map(|r| Town {
            name: r.name,
            map: r.map,
            at: [r.x, r.y, r.z],
        })
        .collect()
});

/// The delivery being carried and the way to it.
#[derive(Resource, Default)]
pub(crate) struct Gps {
    /// Lines from the server not yet handled.
    inbox: Vec<Vec<String>>,
    /// Since when the delivery's quest has been missing from the log (`QUEST_GONE`).
    quest_gone_since: Option<f32>,
    /// The town delivered to, an index into [`TOWNS`].
    target: Option<usize>,
    /// The way, WoW yards, from where it was found to the innkeeper, and the yards along it to
    /// each of its points.
    way: Vec<[f32; 3]>,
    along: Vec<f32>,
    /// When the way was last found, seconds; a new way bumps the generation.
    routed_at: Option<f32>,
    generation: u32,
    /// Yards along the way to the kart's nearest point on it.
    progress: f32,
    /// Where the arrow points, WoW yards: [`LOOKAHEAD`] along the way, or the town with no way.
    pub(crate) ahead: Option<[f32; 3]>,
    /// The town and the yards left to it by road, while the kart is on its map.
    pub(crate) left: Option<(String, u32)>,
    /// The way on the minimap, in this colour (sRGB), unless switched off (`KartNavGpsMinimap`).
    pub(crate) minimap: Option<[f32; 3]>,
}

impl Gps {
    /// The town delivered to, while the kart is on its map: its name, map and place.
    pub(crate) fn town(&self) -> Option<(&str, u32, [f32; 2])> {
        self.left.as_ref()?;
        let t = &TOWNS[self.target?];
        Some((t.name.as_str(), t.map, [t.at[0], t.at[1]]))
    }
}

impl Gps {
    /// The way ahead of the kart, a point every `step` yards from where it is to the town,
    /// while a delivery shows: what the minimap draws.
    pub(crate) fn ahead_of_kart(&self, step: f32) -> impl Iterator<Item = [f32; 3]> + '_ {
        let total = self.along.last().copied().unwrap_or(0.0);
        let shown = self.left.is_some() && self.minimap.is_some() && self.way.len() >= 2;
        let first = (self.progress / step).ceil() as i32;
        let last = if shown {
            (total / step).floor() as i32
        } else {
            first - 1
        };
        (first..=last).map(move |k| point_at(&self.way, &self.along, k as f32 * step))
    }
}

pub(crate) fn on_chat(In(ev): In<SessionEvent>, mut gps: ResMut<Gps>) {
    let SessionEvent::Chat(m) = ev else {
        return;
    };
    if !m.is_addon() {
        return;
    }
    if let Some(body) = m.text.strip_prefix("KARTCRAFT\tR\t") {
        if body.starts_with("Del") {
            gps.inbox
                .push(body.split('\t').map(str::to_string).collect());
        }
    }
}

/// Where `p` comes nearest the way (level): the yards along it there, and how far off it `p` is.
fn project(way: &[[f32; 3]], along: &[f32], p: [f32; 3]) -> (f32, f32) {
    let mut best = (0.0, f32::INFINITY);
    for (i, w) in way.windows(2).enumerate() {
        let (a, b) = (Vec2::new(w[0][0], w[0][1]), Vec2::new(w[1][0], w[1][1]));
        let ab = b - a;
        let t =
            ((Vec2::new(p[0], p[1]) - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
        let d = (a + ab * t).distance(Vec2::new(p[0], p[1]));
        if d < best.1 {
            best = (along[i] + ab.length() * t, d);
        }
    }
    best
}

/// The point `s` yards along the way, held to its ends.
fn point_at(way: &[[f32; 3]], along: &[f32], s: f32) -> [f32; 3] {
    let i = along
        .partition_point(|&a| a <= s)
        .clamp(1, way.len().max(2) - 1);
    let (a, b) = (way[i - 1], way[i]);
    let span = (along[i] - along[i - 1]).max(1e-6);
    let t = ((s - along[i - 1]) / span).clamp(0.0, 1.0);
    std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t)
}

fn lengths(way: &[[f32; 3]]) -> Vec<f32> {
    let mut along = vec![0.0];
    for w in way.windows(2) {
        let last = *along.last().unwrap();
        along.push(last + (w[0][0] - w[1][0]).hypot(w[0][1] - w[1][1]));
    }
    along
}

/// The delivery's lines, then the way from the kart: found when there is none, or the kart has
/// strayed from it; then how far along it the kart is, where the arrow points and what is left.
pub(crate) fn update(
    time: Res<Time>,
    drive: Res<super::KartDrive>,
    map: Option<Res<benilla_world::world_map::CurrentMap>>,
    settings: Res<super::settings::KartSettings>,
    quest_log: Option<Res<crate::ui_quest_log::QuestLog>>,
    mut gps: ResMut<Gps>,
) {
    let minimap = settings.nav.gps_minimap.then_some(settings.nav.gps_color);
    if gps.minimap != minimap {
        gps.minimap = minimap;
    }
    for line in std::mem::take(&mut gps.inbox) {
        match line.first().map(String::as_str) {
            Some("Del") => {
                let town = line.get(1).map_or("", String::as_str);
                gps.target = TOWNS.iter().position(|t| t.name.eq_ignore_ascii_case(town));
                gps.way.clear();
                gps.routed_at = None;
            }
            Some("Del-") => {
                gps.target = None;
                gps.way.clear();
            }
            _ => {}
        }
    }
    let now = time.elapsed_secs();
    // An abandoned delivery (its quest gone from the log) ends as a delivered one does: the
    // server sends no line for it.
    if gps.target.is_some() {
        let held = quest_log
            .as_ref()
            .is_none_or(|q| q.holds_any(DELIVERY_QUESTS.clone()));
        match (held, gps.quest_gone_since) {
            (true, _) => gps.quest_gone_since = None,
            (false, None) => gps.quest_gone_since = Some(now),
            (false, Some(since)) if now - since >= QUEST_GONE => {
                gps.target = None;
                gps.way.clear();
                gps.quest_gone_since = None;
            }
            (false, Some(_)) => {}
        }
    }
    let on_map = map.as_ref().map(|m| m.0);
    let Some(town) = gps
        .target
        .map(|i| &TOWNS[i])
        .filter(|t| drive.active && on_map == Some(t.map))
    else {
        gps.ahead = None;
        gps.left = None;
        return;
    };
    let at = bevy_to_wow(drive.pos);
    let strayed = gps.way.is_empty() || project(&gps.way, &gps.along, at).1 > STRAY;
    let may = gps.routed_at.is_none_or(|t| now - t >= REROUTE_EVERY);
    if strayed && may {
        gps.routed_at = Some(now);
        // The way ends on the road: the last step, into the town's inn, crosses its wall.
        let mut way = ROADS
            .get(&town.map)
            .and_then(|r| r.route(at, town.at))
            .map(|r| r.points)
            .unwrap_or_default();
        way.pop();
        gps.along = lengths(&way);
        gps.way = way;
        gps.generation = gps.generation.wrapping_add(1);
    }
    let name = town.name.clone();
    if gps.way.len() < 2 {
        let left = (at[0] - town.at[0]).hypot(at[1] - town.at[1]);
        gps.ahead = Some(town.at);
        gps.left = Some((name, left.round() as u32));
        return;
    }
    let (s, off) = project(&gps.way, &gps.along, at);
    let total = *gps.along.last().unwrap();
    let end = gps.way[gps.way.len() - 1];
    let walk_in = (end[0] - town.at[0]).hypot(end[1] - town.at[1]);
    gps.progress = s;
    // Near the road's end, the arrow points at the town itself.
    gps.ahead = Some(if total - s < ARRIVE {
        town.at
    } else {
        point_at(&gps.way, &gps.along, s + LOOKAHEAD)
    });
    gps.left = Some((name, (total - s + off + walk_in).round() as u32));
}

/// A chevron on the way.
#[derive(Component)]
pub(crate) struct GpsChevron;

/// A chevron: flat, pointing −Z, a yard and a half deep and three wide, two arms meeting at
/// its tip.
fn chevron_mesh() -> Mesh {
    let (half, depth, arm) = (1.5, 1.5, 0.55);
    let positions: Vec<[f32; 3]> = vec![
        // Left arm: tail to tip.
        [-half, 0.0, depth * 0.5],
        [0.0, 0.0, -depth * 0.5],
        [0.0, 0.0, -depth * 0.5 + arm],
        [-half, 0.0, depth * 0.5 + arm],
        // Right arm.
        [0.0, 0.0, -depth * 0.5],
        [half, 0.0, depth * 0.5],
        [half, 0.0, depth * 0.5 + arm],
        [0.0, 0.0, -depth * 0.5 + arm],
    ];
    let normals = vec![[0.0, 1.0, 0.0]; positions.len()];
    let indices = vec![0, 2, 1, 0, 3, 2, 4, 6, 5, 4, 7, 6];
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_indices(Indices::U32(indices))
}

/// The chevrons' mesh and material, and the colour the material was last given.
type ChevronLook = (Handle<Mesh>, Handle<StandardMaterial>, [f32; 3]);

/// The chevrons' look: unlit, a little see-through, in the GPS's colour (sRGB).
fn chevron_material(c: [f32; 3]) -> StandardMaterial {
    StandardMaterial {
        base_color: Color::srgba(c[0], c[1], c[2], 0.85),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        double_sided: true,
        cull_mode: None,
        ..default()
    }
}

/// The chevrons on the way ahead: one every [`CHEVRON_EVERY`] yards for [`CHEVRONS_AHEAD`],
/// fixed to the way, each laid once on the ground under it and taken up once passed; a new way
/// takes them all up.
pub(crate) fn chevrons(
    mut commands: Commands,
    gps: Res<Gps>,
    settings: Res<super::settings::KartSettings>,
    collide: WorldCollision,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    // The chevrons' mesh and material, and the colour the material was last given.
    mut look: Local<Option<ChevronLook>>,
    // The way's generation they were laid on, and each laid chevron by its slot along it.
    mut laid: Local<(u32, HashMap<i32, Entity>)>,
) {
    // Switched off (`KartNavGpsChevrons`): none wanted, all taken up.
    let wanted = if settings.nav.gps_chevrons && gps.left.is_some() && gps.way.len() >= 2 {
        let total = *gps.along.last().unwrap();
        let first = (gps.progress / CHEVRON_EVERY).ceil() as i32;
        let last =
            ((gps.progress + CHEVRONS_AHEAD).min(total - 1.0) / CHEVRON_EVERY).floor() as i32;
        first.max(1)..=last
    } else {
        #[allow(clippy::reversed_empty_ranges)]
        {
            1..=0
        }
    };
    if laid.0 != gps.generation {
        for (_, e) in laid.1.drain() {
            commands.entity(e).despawn();
        }
        laid.0 = gps.generation;
    }
    laid.1.retain(|k, e| {
        let keep = wanted.contains(k);
        if !keep {
            commands.entity(*e).despawn();
        }
        keep
    });
    // One mesh and material for every chevron, the material recoloured when `KartNavGpsColor`
    // moves.
    let color = settings.nav.gps_color;
    let (mesh, material, made) = look.get_or_insert_with(|| {
        (
            meshes.add(chevron_mesh()),
            materials.add(chevron_material(color)),
            color,
        )
    });
    if *made != color {
        if let Some(m) = materials.get_mut(&*material) {
            *m = chevron_material(color);
        }
        *made = color;
    }
    for k in wanted {
        if laid.1.contains_key(&k) {
            continue;
        }
        let s = k as f32 * CHEVRON_EVERY;
        let p = point_at(&gps.way, &gps.along, s);
        let (a, b) = (
            point_at(&gps.way, &gps.along, s - 2.0),
            point_at(&gps.way, &gps.along, s + 2.0),
        );
        let column = wow_to_bevy(p);
        let y = collide
            .ray_body(
                column + Vec3::Y * GROUND_PROBE,
                Dir3::NEG_Y,
                GROUND_PROBE * 3.0,
            )
            .map_or(column.y, |hit| column.y + GROUND_PROBE - hit.distance);
        let towards = (wow_to_bevy(b) - wow_to_bevy(a)).with_y(0.0);
        let place = Transform::from_translation(Vec3::new(column.x, y + CHEVRON_LIFT, column.z))
            .looking_to(towards.try_normalize().unwrap_or(Vec3::NEG_Z), Vec3::Y);
        let e = commands
            .spawn((
                GpsChevron,
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material.clone()),
                place,
                Name::new("delivery GPS chevron"),
            ))
            .id();
        laid.1.insert(k, e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_delivery_town_is_on_the_gps_roads_of_its_map() {
        for t in TOWNS.iter() {
            let router = ROADS.get(&t.map).expect("the town's map has roads");
            let other = TOWNS
                .iter()
                .find(|o| o.map == t.map && o.name != t.name)
                .unwrap();
            let route = router
                .route(t.at, other.at)
                .unwrap_or_else(|| panic!("{} to {}", t.name, other.name));
            assert!(route.off_road < 400.0, "{} joins the road", t.name);
        }
    }

    #[test]
    fn the_kart_s_place_on_the_way_and_the_points_along_it() {
        let way = vec![[0.0, 0.0, 0.0], [100.0, 0.0, 10.0], [100.0, 50.0, 10.0]];
        let along = lengths(&way);
        assert_eq!(along, [0.0, 100.0, 150.0]);
        let (s, off) = project(&way, &along, [50.0, 4.0, 0.0]);
        assert!((s - 50.0).abs() < 1e-3 && (off - 4.0).abs() < 1e-3);
        let (s, _) = project(&way, &along, [104.0, 20.0, 0.0]);
        assert!((s - 120.0).abs() < 1e-3, "round the corner: {s}");
        assert_eq!(point_at(&way, &along, 50.0), [50.0, 0.0, 5.0]);
        assert_eq!(point_at(&way, &along, 125.0), [100.0, 25.0, 10.0]);
        assert_eq!(
            point_at(&way, &along, 999.0),
            [100.0, 50.0, 10.0],
            "held to its end"
        );
    }

    #[test]
    fn the_minimap_gets_the_way_ahead_of_the_kart_only_while_it_shows() {
        let way = vec![[0.0, 0.0, 0.0], [100.0, 0.0, 0.0]];
        let mut gps = Gps {
            along: lengths(&way),
            way,
            progress: 30.0,
            ..default()
        };
        assert_eq!(gps.ahead_of_kart(8.0).count(), 0, "no delivery shown");
        gps.left = Some(("Goldshire".into(), 70));
        assert_eq!(
            gps.ahead_of_kart(8.0).count(),
            0,
            "the minimap way switched off"
        );
        gps.minimap = Some(super::super::settings::GPS_COLOR);
        let dots: Vec<f32> = gps.ahead_of_kart(8.0).map(|p| p[0]).collect();
        assert_eq!(dots.first(), Some(&32.0), "from the kart on");
        assert_eq!(dots.last(), Some(&96.0), "to the town");
        assert_eq!(dots.len(), 9);
    }

    #[test]
    fn an_abandoned_delivery_is_dropped_once_its_quest_has_left_the_log() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(400),
            ))
            .init_resource::<Gps>()
            .init_resource::<super::super::KartDrive>()
            .init_resource::<super::super::settings::KartSettings>()
            // No courier quest in the log: abandoned.
            .init_resource::<crate::ui_quest_log::QuestLog>()
            .add_systems(Update, update);
        app.world_mut().resource_mut::<Gps>().inbox.push(vec![
            "Del".into(),
            TOWNS[0].name.clone(),
            "300".into(),
            "0".into(),
        ]);
        app.update();
        app.update();
        assert_eq!(
            app.world().resource::<Gps>().target,
            Some(0),
            "a moment's absence is waited out: at login the log can come late"
        );
        for _ in 0..4 {
            app.update();
        }
        assert_eq!(
            app.world().resource::<Gps>().target,
            None,
            "gone a second: dropped"
        );
    }

    #[test]
    fn a_delivery_line_names_its_town_and_its_end_clears_it() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Gps>()
            .init_resource::<super::super::KartDrive>()
            .init_resource::<super::super::settings::KartSettings>()
            .add_systems(Update, update);
        let town = &TOWNS[0];
        app.world_mut().resource_mut::<Gps>().inbox.push(vec![
            "Del".into(),
            town.name.to_uppercase(),
            "300".into(),
            "0".into(),
        ]);
        app.update();
        assert_eq!(app.world().resource::<Gps>().target, Some(0));
        // Out of the kart, nothing is shown.
        assert!(app.world().resource::<Gps>().left.is_none());
        app.world_mut()
            .resource_mut::<Gps>()
            .inbox
            .push(vec!["Del-".into()]);
        app.update();
        assert_eq!(app.world().resource::<Gps>().target, None);
    }
}
