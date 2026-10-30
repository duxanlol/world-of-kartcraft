//! Gauntlets made from courses (`course-gen gauntlet`): a course's copy, one lap of it, with
//! hazards along its route for the server to fire at its racers (`Kartcraft.cpp` "Gauntlet",
//! `docs/courses.md`): every [`SPACING`] yards past the start, the next of [`PATTERN`]. A
//! missile battery or a bomb shredder stands beside the road, the ground there flat and land; a
//! TNT, a Nitro or a red beaker is dropped on the road; a storm or a clock waits on it.

use anyhow::{anyhow, Context, Result};
use benilla_formats::Chain;
use serde_json::{json, Value};

use crate::flight::{slope, Sky};

/// Yards of route between hazards, and none within [`CLEAR_START`] of the start or
/// [`CLEAR_END`] of the finish.
pub const SPACING: f32 = 95.0;
const CLEAR_START: f32 = 130.0;
const CLEAR_END: f32 = 40.0;
/// What goes where, in turn: the kind, and for an emplacement its side (1 left, -1 right).
const PATTERN: [(&str, f32); 10] = [
    ("missile", 1.0),
    ("tnt", 0.0),
    ("bomb", -1.0),
    ("storm", 0.0),
    ("missile", -1.0),
    ("nitro", 0.0),
    ("bomb", 1.0),
    ("beaker", 0.0),
    ("clock", 0.0),
    ("ice", 0.0),
];
/// How far beside the road an emplacement stands (yards), tried out to the last.
const BESIDE: [f32; 4] = [22.0, 26.0, 18.0, 30.0];

/// A hazard's reach (yards) and how often it goes off (ms), by kind.
fn timing(kind: &str) -> (f32, u32) {
    match kind {
        "missile" => (75.0, 4000),
        "bomb" => (55.0, 5000),
        "tnt" | "nitro" | "beaker" => (70.0, 12000),
        "storm" | "ice" => (18.0, 12000),
        _ => (14.0, 15000),
    }
}

/// The gauntlet of the course `from` as a course entry: `key` and `name` its own, one lap of a
/// circuit, its hazards laid along the route over the real world.
pub fn make(chain: &mut Chain, file: &Value, from: &str, key: &str, name: &str) -> Result<Value> {
    let src = file["courses"]
        .as_array()
        .and_then(|cs| cs.iter().find(|c| c["key"] == from))
        .ok_or_else(|| anyhow!("no course {from}"))?;
    let map = src["map"].as_u64().context("map")? as u32;
    let laps = src["laps"].as_u64().unwrap_or(0);
    let cps = src["checkpoints"].as_array().context("checkpoints")?;
    let mut route: Vec<[f32; 3]> = cps
        .iter()
        .map(|c| {
            let n = |k: &str| c[k].as_f64().unwrap_or(0.0) as f32;
            [n("x"), n("y"), n("z")]
        })
        .collect();
    if laps > 0 {
        route.push(route[0]);
    }
    let sky = Sky::gather(chain, map, &route)?;
    let total: f32 = route
        .windows(2)
        .map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]))
        .sum();
    let mut hazards = Vec::new();
    let mut along = CLEAR_START;
    let mut turn = 0;
    while along < total - CLEAR_END {
        let (p, dir) = at(&route, along);
        let (kind, side) = PATTERN[turn % PATTERN.len()];
        turn += 1;
        along += SPACING;
        let (reach, every) = timing(kind);
        let spot = if side == 0.0 {
            sky.floor(p[0], p[1]).map(|z| [p[0], p[1], z])
        } else {
            // Beside the road, on flat land, facing it.
            let normal = [-dir[1] * side, dir[0] * side];
            BESIDE.iter().find_map(|&d| {
                let (x, y) = (p[0] + normal[0] * d, p[1] + normal[1] * d);
                let z = sky.floor(x, y)?;
                let land = sky.ground(x, y).is_some_and(|g| z <= g + 0.5);
                (land && !sky.on_doodad(x, y) && slope(&sky, x, y, 2.0) <= 0.5).then_some([x, y, z])
            })
        };
        let Some(s) = spot else {
            continue;
        };
        let o = if side == 0.0 {
            dir[1].atan2(dir[0])
        } else {
            (p[1] - s[1]).atan2(p[0] - s[0])
        };
        let tenth = |v: f32| (f64::from(v) * 10.0).round() / 10.0;
        hazards.push(
            json!({ "kind": kind, "x": tenth(s[0]), "y": tenth(s[1]), "z": tenth(s[2]),
            "o": (f64::from(o.rem_euclid(std::f32::consts::TAU)) * 1000.0).round() / 1000.0,
            "reach": reach, "every": every }),
        );
    }
    let mut c = src.clone();
    let o = c.as_object_mut().context("course")?;
    o.insert("key".into(), json!(key));
    o.insert("name".into(), json!(name));
    o.remove("series");
    if laps > 0 {
        o.insert("laps".into(), json!(1));
    }
    o.insert("gauntlet".into(), json!(true));
    o.insert(
        "note".into(),
        json!(format!(
            "gauntlet made from {from} by course-gen gauntlet: {} hazards along one run of its route, {:.0} yd.",
            hazards.len(),
            total
        )),
    );
    o.insert("hazards".into(), Value::from(hazards));
    Ok(c)
}

/// The point `along` yards down `route` and the way it runs there (unit, WoW x, y).
fn at(route: &[[f32; 3]], along: f32) -> ([f32; 2], [f32; 2]) {
    let mut left = along;
    for w in route.windows(2) {
        let (dx, dy) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
        let l = dx.hypot(dy).max(1e-3);
        if left <= l {
            let t = left / l;
            return ([w[0][0] + dx * t, w[0][1] + dy * t], [dx / l, dy / l]);
        }
        left -= l;
    }
    let w = &route[route.len() - 2..];
    let (dx, dy) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
    let l = dx.hypot(dy).max(1e-3);
    ([w[1][0], w[1][1]], [dx / l, dy / l])
}
