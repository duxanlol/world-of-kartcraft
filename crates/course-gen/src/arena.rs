//! Arenas (`course-gen arena`): a pit of flat floor walled round, where the server sends waves of
//! creatures after its racers and they fight them with their weapons (`Kartcraft.cpp` "Arena",
//! `docs/courses.md`). The pit is measured from the real world: rays out from its middle every
//! [`RAY_STEP`] degrees until the floor leaves the middle's height, the nearest wall setting its
//! radius. Its two checkpoints stand across it, the start (the grid) on one side facing the other;
//! its crates stand in rings round its middle.

use anyhow::{anyhow, Result};
use benilla_formats::Chain;
use serde_json::{json, Value};

use crate::flight::Sky;

/// Degrees between the rays that find the pit's wall.
const RAY_STEP: usize = 10;
/// How far a ray looks for the wall (yards), and how far the floor may rise or fall from the
/// middle's before it is the wall.
const RAY_REACH: f32 = 90.0;
const WALL_RISE: f32 = 1.5;
/// Yards kept clear of the wall: the arena is the pit less this.
const WALL_CLEAR: f32 = 3.0;
/// The waves, in turn, where none are given: each a creature a letter (`R` a raptor, `B` a
/// bombot, `O` a brute, `A` an axe thrower, `K` a berserker, the boss).
pub const WAVES: [&str; 6] = ["RR", "RRB", "RBAR", "ORRA", "RRBBA", "KRBR"];

/// What the rays found: the pit's middle (the walls' centroid) and its floor, the nearest and the
/// mean wall.
pub struct Pit {
    pub centre: [f32; 3],
    pub nearest: f32,
    pub mean: f32,
}

/// The pit round WoW `at` (x, y) on `map`, its floor about `z`: measured twice, the second time
/// from the first's centroid.
pub fn measure(chain: &mut Chain, map: u32, at: [f32; 2], z: f32) -> Result<Pit> {
    let sky = Sky::gather_area(chain, map, at, RAY_REACH + 10.0, z)?;
    let mut centre = at;
    let mut pit = None;
    for _ in 0..2 {
        let floor = sky
            .floor(centre[0], centre[1])
            .ok_or_else(|| anyhow!("no floor at {centre:?}"))?;
        let walls: Vec<[f32; 2]> = (0..360)
            .step_by(RAY_STEP)
            .map(|deg| {
                let a = (deg as f32).to_radians();
                let (dx, dy) = (a.cos(), a.sin());
                let mut r = 1.0;
                while r < RAY_REACH {
                    let (x, y) = (centre[0] + dx * r, centre[1] + dy * r);
                    if sky
                        .floor(x, y)
                        .is_none_or(|f| (f - floor).abs() > WALL_RISE)
                    {
                        break;
                    }
                    r += 0.5;
                }
                [centre[0] + dx * r, centre[1] + dy * r]
            })
            .collect();
        let dist = |w: &[f32; 2]| (w[0] - centre[0]).hypot(w[1] - centre[1]);
        let nearest = walls.iter().map(dist).fold(f32::INFINITY, f32::min);
        let mean = walls.iter().map(dist).sum::<f32>() / walls.len() as f32;
        pit = Some(Pit {
            centre: [centre[0], centre[1], floor],
            nearest,
            mean,
        });
        let n = walls.len() as f32;
        centre = [
            walls.iter().map(|w| w[0]).sum::<f32>() / n,
            walls.iter().map(|w| w[1]).sum::<f32>() / n,
        ];
    }
    pit.ok_or_else(|| anyhow!("no pit"))
}

/// The arena course of the pit round `at` as a course entry, `key`, `name` and `zone` its own,
/// fought in `waves` ([`WAVES`] where empty).
pub fn make(
    chain: &mut Chain,
    map: u32,
    at: [f32; 2],
    z: f32,
    key: &str,
    name: &str,
    zone: &str,
    waves: &[String],
) -> Result<Value> {
    let pit = measure(chain, map, at, z)?;
    let radius = pit.nearest - WALL_CLEAR;
    if radius < 10.0 {
        return Err(anyhow!("a pit of {:.1} yd is too small", pit.nearest));
    }
    let [cx, cy, cz] = pit.centre;
    let tenth = |v: f32| (f64::from(v) * 10.0).round() / 10.0;
    let side = radius * 0.6;
    let point = |sign: f32| json!({ "x": tenth(cx + sign * side), "y": tenth(cy), "z": tenth(cz) });
    // Weapon crates in a ring halfway out, fruit crates and fruit between them further out.
    let ring = |n: usize, r: f32, turn: f32, kind: &str| -> Vec<Value> {
        (0..n)
            .map(|i| {
                let a = (i as f32 + turn) * std::f32::consts::TAU / n as f32;
                json!({ "kind": kind, "x": tenth(cx + r * a.cos()), "y": tenth(cy + r * a.sin()), "z": tenth(cz) })
            })
            .collect()
    };
    let mut crates = ring(8, radius * 0.5, 0.0, "Q");
    crates.extend(ring(6, radius * 0.8, 0.5, "F"));
    crates.extend(ring(6, radius * 0.25, 0.5, "W"));
    Ok(json!({
        "key": key,
        "name": name,
        "zone": zone,
        "map": map,
        "laps": 0,
        "note": format!(
            "arena made by course-gen arena: a pit of {:.1} yd (mean {:.1}), fought in {:.1} yd of it.",
            pit.nearest, pit.mean, radius
        ),
        "marshal": { "x": tenth(cx + side), "y": tenth(cy + 5.0), "z": tenth(cz), "o": (std::f64::consts::PI * 1000.0).round() / 1000.0 },
        "checkpoints": [point(1.0), point(-1.0)],
        "crates": crates,
        "arena": {
            "x": tenth(cx), "y": tenth(cy), "z": tenth(cz), "radius": tenth(radius),
            "waves": if waves.is_empty() { WAVES.map(String::from).to_vec() } else { waves.to_vec() },
        },
    }))
}
