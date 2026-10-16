//! The walkable floor of a map built from WMOs, for an instance such as Stratholme, whose streets
//! are the faces of one city WMO (`stratholme_b.wmo`) over flat filler terrain. The terrain
//! rasters alone would lay a course through the houses; this samples the walking collision the
//! client stands on instead: every WMO face but `DETAIL` (`accumulate_wmo_group_collision`, the
//! faces the body's box query keeps), placed by its MODF, plus the ADT ground.
//!
//! Each pixel gets the floors over it (faces facing up, at most [`MAX_FLOOR_SLOPE`]) and the
//! spans of everything else (walls, ceilings). A floor is open when nothing stands in the kart's
//! height band above it; open floors a step apart join, and the joined piece with the most WMO
//! floor is the ground (the streets, not the hills round the walls). Group flags are no guide:
//! Stratholme's streets are interior groups, lit by the city's own ambience. A pixel walled off
//! beside the ground stays as ground under a solid pixel, so the course keeps off it.

use std::collections::HashMap;

use benilla_formats::{accumulate_wmo_group_collision, parse_wmo_root, Chain, WmoInstance};
use glam::{Quat, Vec3};

use crate::terrain::Grid;

/// The steepest face (degrees) that is a floor; WoW walks up 50°.
pub const MAX_FLOOR_SLOPE: f32 = 50.0;
/// The most two neighbouring floors (one pixel apart) may differ in height and still join, yards.
pub const STEP: f32 = 1.2;
/// Floors closer together than this (yards) on one pixel are one floor.
const MERGE: f32 = 0.6;
/// The kart's height band over a floor that must be clear, yards: a kerb under its lower edge
/// is driven over, anything in the band is a wall.
const BAND_LO: f32 = 0.6;
const BAND_HI: f32 = 2.6;
/// A floor this far over another on its pixel is a separate storey, not the same ramp, yards.
const FLOOR_ABOVE: f32 = 1.5;

/// The ground a WMO-built map is driven on.
pub struct Floor {
    /// The ground's height, NaN where there is none.
    pub height: Vec<f32>,
    /// A wall or an obstruction stands on the pixel's ground, or the pixel is off the ground
    /// beside it (an instance's walls).
    pub solid: Vec<bool>,
    /// A WMO face stands in the kart's height band over the pixel's ground (or, off the ground,
    /// over the terrain): a wall, and only a wall. On a continent, where the ground is the
    /// terrain with WMO floors laid over it, the edges of the ground piece are no walls.
    pub walls: Vec<bool>,
    /// The ground is a WMO face (a street), not the ADT terrain.
    pub paved: Vec<bool>,
    /// WMO doodads (MODD) of the placed sets, to stamp as obstacles.
    pub doodads: Vec<PlacedDoodad>,
    /// Triangles read, for the log.
    pub triangles: usize,
}

/// A WMO's doodad placed in the world: its M2, position, rotation and scale.
pub struct PlacedDoodad {
    pub model: String,
    pub at: Vec3,
    pub rotation: Quat,
    pub scale: f32,
}

/// One placed face, world yards.
struct Face {
    tri: [Vec3; 3],
}

/// The faces of every WMO placed on the tiles, deduplicated by uniqueId, and their doodads.
fn placed_faces(
    chain: &mut Chain,
    wmos: &[WmoInstance],
    rotation: impl Fn([f32; 3]) -> Quat,
) -> (Vec<Face>, Vec<PlacedDoodad>) {
    let mut faces = Vec::new();
    let mut doodads = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for w in wmos {
        if !seen.insert(w.unique_id) {
            continue;
        }
        let Ok(bytes) = chain.read_file(&w.model) else {
            continue;
        };
        let Ok(root) = parse_wmo_root(&bytes) else {
            continue;
        };
        let q = rotation(w.rotation);
        let origin = Vec3::from_array(w.position);
        let place = |p: [f32; 3]| origin + q * Vec3::from_array(p);
        let lower = w.model.to_ascii_lowercase();
        let stem = lower.strip_suffix(".wmo").unwrap_or(&lower);
        for gi in 0..root.group_count() {
            let Ok(g) = chain.read_file(&format!("{stem}_{gi:03}.wmo")) else {
                continue;
            };
            let (mut pos, mut idx) = (Vec::new(), Vec::new());
            accumulate_wmo_group_collision(&g, &mut pos, &mut idx);
            for t in idx.as_chunks::<3>().0 {
                let (Some(&a), Some(&b), Some(&c)) = (
                    pos.get(t[0] as usize),
                    pos.get(t[1] as usize),
                    pos.get(t[2] as usize),
                ) else {
                    continue;
                };
                faces.push(Face {
                    tri: [place(a), place(b), place(c)],
                });
            }
        }
        // Set 0 is always shown, beside the placement's own set.
        let sets = root.doodad_sets();
        let own = usize::from(w.doodad_set);
        for s in std::iter::once(0).chain((own != 0).then_some(own)) {
            let Some(set) = sets.get(s) else { continue };
            let (from, n) = (set.start as usize, set.count as usize);
            for d in root.doodads().iter().skip(from).take(n) {
                if d.model.is_empty() {
                    continue;
                }
                let [x, y, z, qw] = d.orientation;
                let local = Quat::from_xyzw(x, y, z, qw).normalize();
                doodads.push(PlacedDoodad {
                    model: d.model.clone(),
                    at: place(d.position),
                    rotation: q * local,
                    scale: d.scale,
                });
            }
        }
    }
    (faces, doodads)
}

/// The part of a polygon (pixel `col`, `row`, height) inside the unit pixel square at `(c, r)`.
fn clip_to_pixel(poly: &[Vec3], c: f32, r: f32) -> Vec<Vec3> {
    let mut out: Vec<Vec3> = poly.to_vec();
    // (axis 0 = col / 1 = row, bound, keep >= bound)
    for (axis, bound, above) in [
        (0, c, true),
        (0, c + 1.0, false),
        (1, r, true),
        (1, r + 1.0, false),
    ] {
        if out.is_empty() {
            break;
        }
        let d = |p: &Vec3| {
            let v = if axis == 0 { p.x } else { p.y };
            if above {
                v - bound
            } else {
                bound - v
            }
        };
        let mut next = Vec::with_capacity(out.len() + 2);
        for i in 0..out.len() {
            let (a, b) = (out[i], out[(i + 1) % out.len()]);
            let (da, db) = (d(&a), d(&b));
            if da >= 0.0 {
                next.push(a);
            }
            if (da >= 0.0) != (db >= 0.0) {
                next.push(a + (b - a) * (da / (da - db)));
            }
        }
        out = next;
    }
    out
}

/// One floor on a pixel.
#[derive(Clone, Copy, Debug)]
struct FloorAt {
    z: f32,
    /// On a WMO face, not the terrain.
    paved: bool,
}

/// Per pixel: floors and blocking spans `(lo, hi)`.
struct Columns {
    floors: Vec<Vec<FloorAt>>,
    spans: Vec<Vec<(f32, f32)>>,
}

fn rasterize(grid: &Grid, faces: &[Face], terrain: &[f32]) -> Columns {
    let n = grid.len();
    let mut floors: Vec<Vec<FloorAt>> = vec![Vec::new(); n];
    let mut spans: Vec<Vec<(f32, f32)>> = vec![Vec::new(); n];
    let min_nz = MAX_FLOOR_SLOPE.to_radians().cos();
    for f in faces {
        let normal = (f.tri[1] - f.tri[0]).cross(f.tri[2] - f.tri[0]);
        let len = normal.length();
        if len < 1e-6 {
            continue;
        }
        let nz = normal.z / len;
        // In pixel space: (col, row, height).
        let px: Vec<Vec3> = f
            .tri
            .iter()
            .map(|p| {
                let (c, r) = grid.pixel_of(p.x, p.y);
                Vec3::new(c, r, p.z)
            })
            .collect();
        let lo = px.iter().fold(Vec3::MAX, |m, p| m.min(*p));
        let hi = px.iter().fold(Vec3::MIN, |m, p| m.max(*p));
        let (c0, c1) = (lo.x.floor() as i64, hi.x.floor() as i64);
        let (r0, r1) = (lo.y.floor() as i64, hi.y.floor() as i64);
        for r in r0..=r1 {
            for c in c0..=c1 {
                let Some(i) = grid.index(c, r) else { continue };
                let part = clip_to_pixel(&px, c as f32, r as f32);
                if part.len() < 3 && !(part.len() == 2 && nz.abs() < min_nz) {
                    continue;
                }
                let zlo = part.iter().map(|p| p.z).fold(f32::INFINITY, f32::min);
                let zhi = part.iter().map(|p| p.z).fold(f32::NEG_INFINITY, f32::max);
                if nz >= min_nz {
                    let mean = part.iter().map(|p| p.z).sum::<f32>() / part.len() as f32;
                    floors[i].push(FloorAt {
                        z: mean,
                        paved: true,
                    });
                } else {
                    spans[i].push((zlo, zhi));
                }
            }
        }
    }
    for (i, &h) in terrain.iter().enumerate() {
        if h.is_finite() {
            floors[i].push(FloorAt { z: h, paved: false });
        }
    }
    for f in &mut floors {
        f.sort_by(|a, b| a.z.total_cmp(&b.z));
        // One floor per MERGE: the highest of a close pair, paved if either is.
        let mut out: Vec<FloorAt> = Vec::with_capacity(f.len());
        for &at in f.iter() {
            match out.last_mut() {
                Some(last) if at.z - last.z < MERGE => {
                    last.z = at.z;
                    last.paved |= at.paved;
                }
                _ => out.push(at),
            }
        }
        *f = out;
    }
    Columns { floors, spans }
}

fn find(parent: &mut [u32], mut a: u32) -> u32 {
    while parent[a as usize] != a {
        parent[a as usize] = parent[parent[a as usize] as usize];
        a = parent[a as usize];
    }
    a
}

/// The ground of a map whose walkable surfaces are WMOs over `terrain` (heights per pixel, NaN in
/// a hole). `rotation` is the MODF placement rotation (degrees to a WoW-frame quaternion). With
/// `prefer_paved` a pixel's ground is its lowest WMO floor of the ground where it has one (a
/// bridge's deck over its gorge), not its lowest floor of any kind.
pub fn walkable_floor(
    chain: &mut Chain,
    grid: &Grid,
    wmos: &[WmoInstance],
    terrain: &[f32],
    rotation: impl Fn([f32; 3]) -> Quat,
    prefer_paved: bool,
) -> Floor {
    let (faces, doodads) = placed_faces(chain, wmos, rotation);
    let triangles = faces.len();
    let cols = rasterize(grid, &faces, terrain);
    drop(faces);

    // Nodes: every floor of every pixel; open when its band is clear.
    let mut first = Vec::with_capacity(grid.len() + 1);
    let mut nodes: Vec<(f32, bool)> = Vec::new(); // (height, open)
    let mut paved_node: Vec<bool> = Vec::new();
    for (i, fl) in cols.floors.iter().enumerate() {
        first.push(nodes.len() as u32);
        for &FloorAt { z, paved } in fl {
            // A wall in the band, or a floor above low enough to be the band's ceiling (two faces
            // of one ramp on a pixel sit closer than FLOOR_ABOVE).
            let blocked = cols.spans[i]
                .iter()
                .any(|&(lo, hi)| hi > z + BAND_LO && lo < z + BAND_HI)
                || fl
                    .iter()
                    .any(|a| a.z > z + FLOOR_ABOVE && a.z < z + BAND_HI);
            nodes.push((z, !blocked));
            paved_node.push(paved);
        }
    }
    first.push(nodes.len() as u32);
    let of = |i: usize| first[i] as usize..first[i + 1] as usize;

    let mut parent: Vec<u32> = (0..nodes.len() as u32).collect();
    let (w, h) = (grid.width, grid.height);
    for r in 0..h {
        for c in 0..w {
            let i = r * w + c;
            for j in [(c + 1 < w).then(|| i + 1), (r + 1 < h).then(|| i + w)]
                .into_iter()
                .flatten()
            {
                for a in of(i) {
                    if !nodes[a].1 {
                        continue;
                    }
                    for b in of(j) {
                        if nodes[b].1 && (nodes[a].0 - nodes[b].0).abs() <= STEP {
                            let (ra, rb) =
                                (find(&mut parent, a as u32), find(&mut parent, b as u32));
                            if ra != rb {
                                parent[ra.max(rb) as usize] = ra.min(rb);
                            }
                        }
                    }
                }
            }
        }
    }
    // The open piece with the most paved floor is the ground: the streets, not the hills round
    // the walls.
    let mut size: HashMap<u32, usize> = HashMap::new();
    for k in 0..nodes.len() {
        if nodes[k].1 && paved_node[k] {
            *size.entry(find(&mut parent, k as u32)).or_default() += 1;
        }
    }
    let ground = size
        .iter()
        .max_by_key(|&(root, n)| (*n, std::cmp::Reverse(*root)))
        .map(|(root, _)| *root);

    // With `prefer_paved` (a continent), a WMO floor joined to the terrain anywhere is driven
    // on, not only the ground piece's: mountains split a continent's terrain into many pieces,
    // and a tunnel joins only the slopes at its mouths. A roof joins no terrain.
    let mut on_terrain: std::collections::HashSet<u32> = std::collections::HashSet::new();
    if prefer_paved {
        for k in 0..nodes.len() {
            if nodes[k].1 && !paved_node[k] {
                on_terrain.insert(find(&mut parent, k as u32));
            }
        }
    }

    let mut height = vec![f32::NAN; grid.len()];
    let mut solid = vec![false; grid.len()];
    let mut paved = vec![false; grid.len()];
    for i in 0..grid.len() {
        // The lowest floor of the ground on the pixel, a WMO's first when preferred.
        let paved_k = prefer_paved
            .then(|| {
                of(i).find(|&k| {
                    let root = find(&mut parent, k as u32);
                    paved_node[k]
                        && nodes[k].1
                        && (Some(root) == ground || on_terrain.contains(&root))
                })
            })
            .flatten();
        if let Some(k) =
            paved_k.or_else(|| of(i).find(|&k| Some(find(&mut parent, k as u32)) == ground))
        {
            height[i] = nodes[k].0;
            paved[i] = paved_node[k];
        }
    }
    // A pixel off the ground beside it is a wall: the ground's height there, solid. Taken from
    // the neighbour, so the drive test's block stands on the street, not on the roof.
    let ground_height = height.clone();
    for r in 0..h {
        for c in 0..w {
            let i = r * w + c;
            if ground_height[i].is_finite() {
                continue;
            }
            // Eight neighbours: a ground quad needs all four corners, so a diagonal gap would
            // leave a hole in the drive test's floor at a building's corner.
            let near = [
                (-1i64, 0i64),
                (1, 0),
                (0, -1),
                (0, 1),
                (-1, -1),
                (-1, 1),
                (1, -1),
                (1, 1),
            ]
            .iter()
            .filter_map(|&(dc, dr)| grid.index(c as i64 + dc, r as i64 + dr))
            .map(|j| ground_height[j])
            .find(|z| z.is_finite());
            if let Some(z) = near {
                height[i] = z;
                solid[i] = true;
            }
        }
    }
    let walls = (0..grid.len())
        .map(|i| {
            let z = if height[i].is_finite() && !solid[i] {
                height[i]
            } else {
                terrain[i]
            };
            z.is_finite()
                && cols.spans[i]
                    .iter()
                    .any(|&(lo, hi)| hi > z + BAND_LO && lo < z + BAND_HI)
        })
        .collect();
    Floor {
        height,
        solid,
        walls,
        paved,
        doodads,
        triangles,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(n: usize) -> Grid {
        Grid {
            north_x: n as f32,
            west_y: n as f32,
            yards_per_px: 1.0,
            width: n,
            height: n,
        }
    }

    fn quad(x0: f32, y0: f32, x1: f32, y1: f32, z: f32) -> [Face; 2] {
        let p = |x, y| Vec3::new(x, y, z);
        [
            Face {
                tri: [p(x0, y0), p(x1, y0), p(x1, y1)],
            },
            Face {
                tri: [p(x0, y0), p(x1, y1), p(x0, y1)],
            },
        ]
    }

    #[test]
    fn a_pixel_clip_keeps_the_part_inside() {
        let tri = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(4.0, 0.0, 4.0),
            Vec3::new(0.0, 4.0, 0.0),
        ];
        let part = clip_to_pixel(&tri, 1.0, 1.0);
        assert!(part.len() >= 3);
        for p in &part {
            assert!((1.0..=2.0).contains(&p.x) && (1.0..=2.0).contains(&p.y));
        }
        assert!(clip_to_pixel(&tri, 5.0, 5.0).is_empty());
    }

    #[test]
    fn a_floor_under_a_roof_is_one_ground_and_the_roof_is_not() {
        let g = grid(10);
        // A street at z 0 over the whole raster and a roof at z 8 over its north half.
        let mut faces: Vec<Face> = quad(0.0, 0.0, 10.0, 10.0, 0.0).into_iter().collect();
        faces.extend(quad(5.0, 0.0, 10.0, 10.0, 8.0));
        let cols = rasterize(&g, &faces, &vec![f32::NAN; g.len()]);
        let (c, r) = g.pixel_of(7.5, 5.0);
        let i = g.index(c as i64, r as i64).unwrap();
        let zs: Vec<f32> = cols.floors[i].iter().map(|f| f.z).collect();
        assert_eq!(zs.len(), 2, "{zs:?}");
        assert!(zs[0].abs() < 1e-3 && (zs[1] - 8.0).abs() < 1e-3, "{zs:?}");
    }
}
