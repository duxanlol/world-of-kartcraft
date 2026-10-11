//! Race Marshals: one NPC per distinct course start, standing beside the start line, where a
//! player hosts, joins and starts races from a gossip menu. [`place`] writes each course's
//! `"marshal": {x, y, z, o}` into `courses/courses.json`; `setup/vmangos/pack.py` turns those
//! into the creature spawns of `setup/vmangos/sql/kartcraft.sql` and the server matches a
//! marshal to its courses by that point.

use std::collections::HashMap;

use anyhow::{bail, Context, Result};
use benilla_formats::{adt_to_tile_mesh, load_map_catalog, Chain, TileMesh, TILE_SIZE};
use serde_json::{Map, Value};

/// Starts closer together than this (yards) share one marshal.
pub const MERGE_RADIUS: f32 = 40.0;
/// How far left of the start line (yards, across the direction from checkpoint 0 to 1) the
/// marshal stands: inside the server's 60-yard start radius, clear of the grid.
pub const SIDE_OFFSET: f32 = 10.0;

/// World yards from the map corner to its centre: 32 tiles.
const MAP_CENTER: f32 = 32.0 * TILE_SIZE;

/// Terrain height at single points, reading only the ADT tiles asked about.
pub struct Ground {
    chain: Chain,
    dirs: HashMap<u32, String>,
    tiles: HashMap<(u32, i64, i64), Option<TileMesh>>,
}

impl Ground {
    pub fn new(mut chain: Chain) -> Result<Self> {
        let maps = load_map_catalog(&mut chain)?;
        let dirs = [0u32, 1]
            .into_iter()
            .filter_map(|m| maps.directory(m).map(|d| (m, d.to_string())))
            .collect();
        Ok(Self {
            chain,
            dirs,
            tiles: HashMap::new(),
        })
    }

    /// The terrain height under `(x, y)` on `map`; `None` off the tiles or in a hole.
    pub fn height(&mut self, map: u32, x: f32, y: f32) -> Option<f32> {
        let tx = ((MAP_CENTER - y) / TILE_SIZE).floor() as i64;
        let ty = ((MAP_CENTER - x) / TILE_SIZE).floor() as i64;
        if !(0..64).contains(&tx) || !(0..64).contains(&ty) {
            return None;
        }
        if !self.tiles.contains_key(&(map, tx, ty)) {
            let mesh = self.dirs.get(&map).and_then(|dir| {
                let path = format!("World\\Maps\\{dir}\\{dir}_{tx}_{ty}.adt");
                let bytes = self.chain.read_file(&path).ok()?;
                adt_to_tile_mesh(&bytes).ok()
            });
            self.tiles.insert((map, tx, ty), mesh);
        }
        let tile = self.tiles.get(&(map, tx, ty))?.as_ref()?;
        tile.chunks.iter().find_map(|c| c.height_at([x, y, 0.0]))
    }
}

fn round(v: f64, places: i32) -> Value {
    let k = 10f64.powi(places);
    let r = (v * k).round() / k;
    if r == r.trunc() && r.abs() < 1e15 {
        Value::from(r as i64)
    } else {
        Value::from(r)
    }
}

fn point(c: &Value, i: usize) -> Result<(f32, f32)> {
    let p = &c["checkpoints"][i];
    Ok((
        p["x"].as_f64().context("checkpoint x")? as f32,
        p["y"].as_f64().context("checkpoint y")? as f32,
    ))
}

/// Writes `"marshal"` into every course of `file`: courses whose starts lie within
/// [`MERGE_RADIUS`] of an earlier course's start (same map) share that course's marshal, which
/// stands [`SIDE_OFFSET`] to the left of its start line facing the line, at `ground`'s height
/// there. Returns the number of distinct marshals.
pub fn place(
    file: &mut Value,
    mut ground: impl FnMut(u32, f32, f32) -> Option<f32>,
) -> Result<usize> {
    let courses = file
        .get_mut("courses")
        .and_then(Value::as_array_mut)
        .context("courses.json: no \"courses\" list")?;
    // (map, start x, start y, marshal) per distinct start, in file order.
    let mut placed: Vec<(u32, f32, f32, Value)> = Vec::new();
    for c in courses.iter_mut() {
        let key = c["key"].as_str().unwrap_or("?").to_string();
        let map = c["map"].as_u64().context("course map")? as u32;
        let (sx, sy) = point(c, 0).with_context(|| format!("course {key}"))?;
        let near = placed.iter().find(|(m, x, y, _)| {
            *m == map && ((x - sx).powi(2) + (y - sy).powi(2)).sqrt() <= MERGE_RADIUS
        });
        let marshal = match near {
            Some((.., m)) => m.clone(),
            None => {
                let (nx, ny) = point(c, 1).with_context(|| format!("course {key}"))?;
                let (dx, dy) = (nx - sx, ny - sy);
                let len = (dx * dx + dy * dy).sqrt().max(1.0);
                let (dx, dy) = (dx / len, dy / len);
                // Left of the heading (x north, y west): the heading turned a quarter anticlockwise.
                let (mx, my) = (sx - dy * SIDE_OFFSET, sy + dx * SIDE_OFFSET);
                // Facing the start line: the heading turned a quarter clockwise.
                let o = dy.atan2(dx) - std::f32::consts::FRAC_PI_2;
                let o = o.rem_euclid(std::f32::consts::TAU);
                // A hole under the exact spot (a cave mouth): the nearest ground a yard or two off.
                let z = [(0.0, 0.0), (1.5, 0.0), (-1.5, 0.0), (0.0, 1.5), (0.0, -1.5)]
                    .iter()
                    .find_map(|(ox, oy)| ground(map, mx + ox, my + oy))
                    .or_else(|| c["checkpoints"][0]["z"].as_f64().map(|z| z as f32));
                let Some(z) = z else {
                    bail!("course {key}: no ground under its marshal at {mx:.1}, {my:.1}");
                };
                let mut m = Map::new();
                m.insert("x".into(), round(f64::from(mx), 1));
                m.insert("y".into(), round(f64::from(my), 1));
                m.insert("z".into(), round(f64::from(z), 1));
                m.insert("o".into(), round(f64::from(o), 3));
                let m = Value::Object(m);
                placed.push((map, sx, sy, m.clone()));
                m
            }
        };
        c["marshal"] = marshal;
    }
    Ok(placed.len())
}

/// Reads `courses`, places the marshals over the install's terrain, validates and writes it.
pub fn run(courses: &std::path::Path, chain: Chain) -> Result<usize> {
    let mut file: Value = serde_json::from_str(
        &std::fs::read_to_string(courses)
            .with_context(|| format!("reading {}", courses.display()))?,
    )
    .with_context(|| format!("parsing {}", courses.display()))?;
    let mut ground = Ground::new(chain)?;
    let n = place(&mut file, |map, x, y| ground.height(map, x, y))?;
    crate::export::validate(&file)
        .with_context(|| format!("{} would be invalid", courses.display()))?;
    let text = crate::export::format(&file);
    let back: Value = serde_json::from_str(&text).context("re-reading the formatted file")?;
    anyhow::ensure!(back == file, "formatting changed the file");
    std::fs::write(courses, text).with_context(|| format!("writing {}", courses.display()))?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marshals_stand_left_of_the_start_and_merge_near_starts() {
        let mut file = serde_json::json!({"courses": [
            {"key": "a", "name": "A", "map": 0, "laps": 0,
             "checkpoints": [{"x": 0, "y": 0}, {"x": 100, "y": 0}]},
            {"key": "b", "name": "B", "map": 0, "laps": 3,
             "checkpoints": [{"x": 20, "y": 10}, {"x": 20, "y": 200}]},
            {"key": "c", "name": "C", "map": 1, "laps": 0,
             "checkpoints": [{"x": 0, "y": 0}, {"x": 0, "y": -50}]}
        ]});
        let n = place(&mut file, |_, x, _| Some(x * 0.5)).unwrap();
        assert_eq!(n, 2);
        let a = &file["courses"][0]["marshal"];
        // Heading north (+x): left is west (+y); facing the line is east, a quarter clockwise.
        assert_eq!((a["x"].as_f64(), a["y"].as_f64()), (Some(0.0), Some(10.0)));
        assert_eq!(a["z"].as_f64(), Some(0.0));
        let o = a["o"].as_f64().unwrap();
        assert!((o - 3.0 * std::f64::consts::FRAC_PI_2).abs() < 1e-3, "{o}");
        assert_eq!(file["courses"][1]["marshal"], *a);
        // Heading east (−y) on another map: left is north (+x).
        let c = &file["courses"][2]["marshal"];
        assert_eq!((c["x"].as_f64(), c["y"].as_f64()), (Some(10.0), Some(0.0)));
        assert_eq!(c["z"].as_f64(), Some(5.0));
        crate::export::validate(&file).unwrap();
    }
}
