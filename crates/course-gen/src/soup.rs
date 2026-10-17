//! The world's solid faces around a spot, read from the install the way the client builds its
//! colliders (`benilla-world`'s `terrain_stream`): every ADT chunk's hole-masked terrain, the
//! impassable chunks' fences, each placed doodad's M2 collision hull, each placed WMO's walking
//! faces (every face but `DETAIL`) and its doodad sets' hulls. The faces come out in the kart's
//! frame as the session gathers them (`kartcraft::session`): Bevy axes (`bevy = (−y, z, −x)` of
//! WoW's), in lu from an origin. Server-spawned GameObjects (doors, the Ahn'Qiraj gate) are not in
//! the map files and are not here.

use std::collections::{HashMap, HashSet};

use anyhow::{bail, Context, Result};
use benilla_formats::{
    load_m2_collision_hull, load_tiles_around, load_wmo_collision_tris, parse_wmo_root,
    wmo_group_liquid_mesh, Chain, CollisionMesh,
};
use glam::{Affine3A, Mat3, Quat, Vec3};
use kart_driver::LU_PER_YARD;

/// A triangle in lu, as `ctr_kart::Collision` takes it.
pub type Tri = [[f32; 3]; 3];

/// WoW `[x, y, z]` to Bevy (`benilla_assets::coords::wow_to_bevy`).
pub fn wow_to_bevy(p: [f32; 3]) -> Vec3 {
    Vec3::new(-p[1], p[2], -p[0])
}

/// Inverse of [`wow_to_bevy`].
pub fn bevy_to_wow(b: Vec3) -> [f32; 3] {
    [-b.z, -b.x, b.y]
}

/// The rotation of [`wow_to_bevy`].
fn wow_to_bevy_quat() -> Quat {
    Quat::from_mat3(&Mat3::from_cols(
        Vec3::new(0.0, 0.0, -1.0),
        Vec3::new(-1.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
    ))
}

/// An MDDF/MODF Euler rotation (degrees) in Bevy space (`benilla_assets::coords::placement_rotation`).
fn placement_rotation(rotation_deg: [f32; 3]) -> Quat {
    use std::f32::consts::{FRAC_PI_2, PI};
    let [rx, ry, rz] = rotation_deg.map(f32::to_radians);
    let in_wow = Quat::from_rotation_x(FRAC_PI_2)
        * Quat::from_rotation_y(ry - PI)
        * Quat::from_rotation_z(-rx)
        * Quat::from_rotation_x(rz - FRAC_PI_2);
    let to_bevy = wow_to_bevy_quat();
    to_bevy * in_wow * to_bevy.inverse()
}

/// An MODD quaternion in Bevy space (`benilla_assets::coords::wow_rotation_to_bevy`).
fn wow_rotation_to_bevy(q: [f32; 4]) -> Quat {
    let to_bevy = wow_to_bevy_quat();
    (to_bevy * Quat::from_xyzw(q[0], q[1], q[2], q[3]) * to_bevy.inverse()).normalize()
}

/// The ADT map directory of a map id.
pub fn map_dir(map: u32) -> Result<&'static str> {
    Ok(match map {
        0 => "Azeroth",
        1 => "Kalimdor",
        _ => bail!("map {map} is not a continent"),
    })
}

/// An ADT tile's side, yards.
const TILE_YARDS: f32 = 1600.0 / 3.0;

/// How far an impassable chunk's fence rises (`benilla-world`'s `FENCE_REACH`).
const FENCE_REACH: f32 = 32000.0;

/// The box the faces are gathered in: Bevy world, yards.
#[derive(Clone, Copy, Debug)]
pub struct Region {
    pub min: Vec3,
    pub max: Vec3,
}

impl Region {
    /// A box `half` (Bevy axes, yards) around WoW `centre`.
    pub fn around(centre: [f32; 3], half: Vec3) -> Self {
        let c = wow_to_bevy(centre);
        Self {
            min: c - half,
            max: c + half,
        }
    }

    fn overlaps(&self, t: &[Vec3; 3]) -> bool {
        let lo = t[0].min(t[1]).min(t[2]);
        let hi = t[0].max(t[1]).max(t[2]);
        lo.cmple(self.max).all() && hi.cmpge(self.min).all()
    }
}

/// The faces gathered: the solid ones at their authored winding, and the ADT liquid surfaces
/// (never solid; a kart would drive the bed beneath), Bevy world yards.
#[derive(Default)]
pub struct Faces {
    pub solid: Vec<[Vec3; 3]>,
    pub liquid: Vec<[Vec3; 3]>,
}

/// The faces in `region` on `map`.
pub fn gather_world(chain: &mut Chain, map: u32, region: Region) -> Result<Faces> {
    let centre = bevy_to_wow((region.min + region.max) * 0.5);
    // The tiles round the centre the region reaches: one each way at least.
    let half = ((region.max - region.min) * 0.5).max_element();
    let reach = ((half / TILE_YARDS).ceil() as u32).max(1);
    let tiles = load_tiles_around(chain, map_dir(map)?, centre[0], centre[1], reach)
        .context("loading the tiles")?;
    let mut out: Vec<[Vec3; 3]> = Vec::new();
    let mut liquid: Vec<[Vec3; 3]> = Vec::new();
    for (_, tile) in &tiles {
        for lq in tile.chunks.iter().flat_map(|c| &c.liquids) {
            for c in lq.indices.as_chunks::<3>().0 {
                let v = |i: u32| lq.positions.get(i as usize).copied().map(wow_to_bevy);
                if let (Some(a), Some(b), Some(d)) = (v(c[0]), v(c[1]), v(c[2])) {
                    let t = [a, b, d];
                    if region.overlaps(&t) {
                        liquid.push(t);
                    }
                }
            }
        }
    }
    let mut keep = |t: [Vec3; 3]| {
        if region.overlaps(&t) {
            out.push(t);
        }
    };

    // Terrain and the impassable fences.
    for (_, tile) in &tiles {
        for chunk in &tile.chunks {
            for c in chunk.indices.as_chunks::<3>().0 {
                let v = |i: u32| chunk.positions.get(i as usize).copied().map(wow_to_bevy);
                if let (Some(a), Some(b), Some(d)) = (v(c[0]), v(c[1]), v(c[2])) {
                    keep([a, b, d]);
                }
            }
            if !chunk.impassable {
                continue;
            }
            let Some(floor) = chunk.positions.iter().map(|p| p[2]).reduce(f32::min) else {
                continue;
            };
            for [from, to] in [[8, 0], [136, 144], [0, 136], [144, 8]] {
                let (Some(a), Some(b)) = (chunk.positions.get(from), chunk.positions.get(to))
                else {
                    continue;
                };
                let q = [
                    wow_to_bevy([a[0], a[1], floor]),
                    wow_to_bevy([b[0], b[1], floor]),
                    wow_to_bevy([b[0], b[1], floor + FENCE_REACH]),
                    wow_to_bevy([a[0], a[1], floor + FENCE_REACH]),
                ];
                keep([q[0], q[1], q[2]]);
                keep([q[0], q[2], q[3]]);
            }
        }
    }

    // Placed models, once each however many tiles list them.
    let mut hulls: HashMap<String, Option<CollisionMesh>> = HashMap::new();
    let mut hull = |chain: &mut Chain, path: &str| -> Option<CollisionMesh> {
        hulls
            .entry(path.to_ascii_lowercase())
            .or_insert_with(|| {
                load_m2_collision_hull(chain, path)
                    .ok()
                    .filter(|c| !c.is_empty())
            })
            .clone()
    };
    let place = |mesh: &CollisionMesh, at: Affine3A, keep: &mut dyn FnMut([Vec3; 3])| {
        for c in mesh.indices.as_chunks::<3>().0 {
            let v = |i: u32| {
                mesh.positions
                    .get(i as usize)
                    .map(|p| at.transform_point3(wow_to_bevy(*p)))
            };
            if let (Some(a), Some(b), Some(d)) = (v(c[0]), v(c[1]), v(c[2])) {
                keep([a, b, d]);
            }
        }
    };
    let mut seen = HashSet::new();
    for (_, tile) in &tiles {
        for d in &tile.doodads {
            if !seen.insert(d.unique_id) {
                continue;
            }
            // A placement far outside the box cannot reach into it (no hull is 200 yd across).
            if !near(&region, wow_to_bevy(d.position), 200.0) {
                continue;
            }
            if let Some(mesh) = hull(chain, &d.model) {
                let at = Affine3A::from_scale_rotation_translation(
                    Vec3::splat(d.scale),
                    placement_rotation(d.rotation),
                    wow_to_bevy(d.position),
                );
                place(&mesh, at, &mut keep);
            }
        }
    }
    let mut seen = HashSet::new();
    for (_, tile) in &tiles {
        for w in &tile.wmos {
            if !seen.insert(w.unique_id) {
                continue;
            }
            let at = Affine3A::from_rotation_translation(
                placement_rotation(w.rotation),
                wow_to_bevy(w.position),
            );
            let root_path = w.model.to_ascii_lowercase();
            let Ok(mesh) = load_wmo_collision_tris(chain, &root_path) else {
                continue;
            };
            place(&mesh, at, &mut keep);
            let Ok(root) = chain.read_file(&root_path).and_then(|b| parse_wmo_root(&b)) else {
                continue;
            };
            // The building's own water (a city's canals).
            let stem = root_path
                .strip_suffix(".wmo")
                .unwrap_or(&root_path)
                .to_string();
            for gi in 0..root.group_count() {
                let Ok(group) = chain.read_file(&format!("{stem}_{gi:03}.wmo")) else {
                    continue;
                };
                let Some(lq) = wmo_group_liquid_mesh(&group) else {
                    continue;
                };
                for c in lq.indices.as_chunks::<3>().0 {
                    let v = |i: u32| {
                        lq.positions
                            .get(i as usize)
                            .map(|p| at.transform_point3(wow_to_bevy(*p)))
                    };
                    if let (Some(a), Some(b), Some(d)) = (v(c[0]), v(c[1]), v(c[2])) {
                        let t = [a, b, d];
                        if region.overlaps(&t) {
                            liquid.push(t);
                        }
                    }
                }
            }
            let sets = root.doodad_sets();
            let mut ranges: Vec<(u32, u32)> = sets
                .first()
                .map(|s| (s.start, s.count))
                .into_iter()
                .collect();
            if w.doodad_set != 0 {
                if let Some(s) = sets.get(w.doodad_set as usize) {
                    ranges.push((s.start, s.count));
                }
            }
            for (start, count) in ranges {
                for d in root
                    .doodads()
                    .iter()
                    .skip(start as usize)
                    .take(count as usize)
                {
                    if d.model.is_empty() {
                        continue;
                    }
                    let local = Affine3A::from_scale_rotation_translation(
                        Vec3::splat(d.scale),
                        wow_rotation_to_bevy(d.orientation),
                        wow_to_bevy(d.position),
                    );
                    let world = at * local;
                    if !near(&region, world.translation.into(), 200.0) {
                        continue;
                    }
                    if let Some(mesh) = hull(chain, &d.model) {
                        place(&mesh, world, &mut keep);
                    }
                }
            }
        }
    }
    Ok(Faces { solid: out, liquid })
}

fn near(region: &Region, p: Vec3, margin: f32) -> bool {
    p.cmpge(region.min - Vec3::splat(margin)).all()
        && p.cmple(region.max + Vec3::splat(margin)).all()
}

/// The faces in lu from `origin` (Bevy world), as the session gathers them: zero-area and
/// non-finite faces left out.
pub fn to_lu(faces: &[[Vec3; 3]], origin: Vec3) -> Vec<Tri> {
    faces
        .iter()
        .filter_map(|f| {
            let t = f.map(|v| (v - origin) * LU_PER_YARD);
            let solid = t.iter().all(|v| v.is_finite())
                && (t[1] - t[0]).cross(t[2] - t[0]).length_squared() > 1e-4;
            solid.then(|| t.map(|v| v.to_array()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wow_and_bevy_round_trip_and_keep_handedness() {
        let p = [-6221.4, -3927.6, -58.7];
        let back = bevy_to_wow(wow_to_bevy(p));
        assert!((0..3).all(|i| (back[i] - p[i]).abs() < 1e-3));
        // A heading of 0 in WoW (north, +X) is Bevy −Z, the yaw-0 forward.
        assert_eq!(wow_to_bevy([1.0, 0.0, 0.0]), Vec3::new(0.0, 0.0, -1.0));
        // A placement turned by `ry` turns about up.
        let q = placement_rotation([0.0, 90.0, 0.0]);
        let up = q * Vec3::Y;
        assert!((up - Vec3::Y).length() < 1e-4, "{up:?}");
    }

    #[test]
    fn the_region_takes_a_face_that_reaches_into_it() {
        let r = Region {
            min: Vec3::splat(-1.0),
            max: Vec3::splat(1.0),
        };
        assert!(r.overlaps(&[
            Vec3::new(-5.0, 0.0, 0.0),
            Vec3::new(5.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 5.0)
        ]));
        assert!(!r.overlaps(&[
            Vec3::new(3.0, 0.0, 0.0),
            Vec3::new(5.0, 0.0, 0.0),
            Vec3::new(4.0, 0.0, 5.0)
        ]));
    }
}
