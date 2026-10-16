//! Turbo pads on a course's straights. [`place`] writes each course's `"pads"`: `{x, y, z, o}`
//! per pad, WoW yards and the pad's facing, into `courses/courses.json`; the client lays them on
//! the ground and fires CTR's turbo pad over them (`kartcraft::pads`). The server ignores them.
//!
//! A straight is a leg between two checkpoints at least [`MIN_STRAIGHT`] long whose route turns
//! less than [`MAX_TURN`] at both its ends (the generator spaces checkpoints about 130 yards
//! apart, so the turns, not the legs' lengths, tell a straight from a bend). The straightest
//! legs get a pad, at most [`MAX_PADS`] a course and never on neighbouring legs, [`AT`] of the
//! way along, so the boost carries down the rest of the straight; a circuit's closing leg counts,
//! a sprint's first leg does not (the grid stands on it). A pad lies on clear ground: where a
//! tree, a rock, a building, water or a slope stands in its footprint ([`run`]'s check over the
//! zone's terrain) it slides along its leg to the nearest clear spot ([`SLIDE`]), and a leg with
//! none gives its place to the next straightest.

use anyhow::{Context, Result};
use serde_json::{Map, Value};

/// The shortest leg (yards) a pad goes on.
pub const MIN_STRAIGHT: f32 = 90.0;
/// The most the route may turn (degrees) at either end of a straight.
pub const MAX_TURN: f32 = 20.0;
/// Pads a course gets at most.
pub const MAX_PADS: usize = 3;
/// How far along its leg a pad lies.
pub const AT: f32 = 0.3;
/// Where along its leg a pad is tried, in turn, when its spot is not clear: nearest [`AT`]
/// first, the earlier side first, so the boost still carries down the straight.
pub const SLIDE: [f32; 12] = [
    0.3, 0.25, 0.35, 0.2, 0.4, 0.15, 0.45, 0.5, 0.55, 0.6, 0.65, 0.7,
];
/// A pad's side, yards: CTR's 384-lu quadblock (`kartcraft::pads::PAD_LENGTH`), and the clear
/// ground kept round it.
const PAD_SIDE: f32 = 384.0 * 0.025 / 0.9144;
const PAD_MARGIN: f32 = 1.0;
/// Ground steeper than this (degrees) under a pad is no ground for it.
const PAD_SLOPE: f32 = 25.0;

fn round(v: f64, places: i32) -> Value {
    let k = 10f64.powi(places);
    let r = (v * k).round() / k;
    if r == r.trunc() && r.abs() < 1e15 {
        Value::from(r as i64)
    } else {
        Value::from(r)
    }
}

/// The turn (degrees) at `b` going `a → b → c`.
fn turn(a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> f32 {
    let h1 = (b.1 - a.1).atan2(b.0 - a.0);
    let h2 = (c.1 - b.1).atan2(c.0 - b.0);
    let d = (h2 - h1).rem_euclid(std::f32::consts::TAU);
    d.min(std::f32::consts::TAU - d).to_degrees()
}

/// The legs a course's pads go on, by their first checkpoint's index, in route order.
pub fn straights(points: &[(f32, f32)], circuit: bool) -> Vec<usize> {
    let mut chosen: Vec<usize> = choose(points, circuit, |_| Some(AT))
        .into_iter()
        .map(|(i, _)| i)
        .collect();
    chosen.sort_unstable();
    chosen
}

/// The straightest legs that `spot` finds a place on (how far along), at most [`MAX_PADS`] and
/// never neighbours, best first.
fn choose(
    points: &[(f32, f32)],
    circuit: bool,
    mut spot: impl FnMut(usize) -> Option<f32>,
) -> Vec<(usize, f32)> {
    let n = points.len();
    let legs = if circuit { n } else { n.saturating_sub(1) };
    let at = |i: usize| points[i % n];
    // (worse end turn, leg) for every straight.
    let mut candidates: Vec<(f32, usize)> = (0..legs)
        .filter(|&i| circuit || i > 0)
        .filter_map(|i| {
            let (a, b) = (at(i), at(i + 1));
            let len = (b.0 - a.0).hypot(b.1 - a.1);
            if len < MIN_STRAIGHT {
                return None;
            }
            // A sprint's ends have no turn to measure: they count as straight.
            let before = (circuit || i > 0).then(|| turn(at(i + n - 1), a, b));
            let after = (circuit || i + 2 < n).then(|| turn(a, b, at(i + 2)));
            let worst = before.unwrap_or(0.0).max(after.unwrap_or(0.0));
            (worst < MAX_TURN).then_some((worst, i))
        })
        .collect();
    candidates.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)));
    let mut chosen: Vec<(usize, f32)> = Vec::new();
    for (_, i) in candidates {
        if chosen.len() == MAX_PADS {
            break;
        }
        let neighbour = chosen.iter().any(|&(j, _)| {
            let d = i.abs_diff(j);
            d <= 1 || (circuit && d == legs - 1)
        });
        if neighbour {
            continue;
        }
        if let Some(t) = spot(i) {
            chosen.push((i, t));
        }
    }
    chosen
}

/// Whether a pad at `(x, y)` facing `o` (radians) lies on clear ground: every yard of its
/// footprint, with [`PAD_MARGIN`] round it, `clear` lets through.
pub fn footprint_clear(x: f32, y: f32, o: f32, mut clear: impl FnMut(f32, f32) -> bool) -> bool {
    let half = PAD_SIDE * 0.5 + PAD_MARGIN;
    let (along, across) = ((o.cos(), o.sin()), (-o.sin(), o.cos()));
    let steps = (half * 2.0).ceil() as i32;
    (0..=steps).all(|a| {
        (0..=steps).all(|b| {
            let (u, v) = (-half + a as f32, -half + b as f32);
            clear(
                x + along.0 * u + across.0 * v,
                y + along.1 * u + across.1 * v,
            )
        })
    })
}

/// Writes `"pads"` into every course of `file` that has a straight, at `ground`'s height there
/// (the field is left out where it has none); a course with no straight gets none. Returns the
/// number of pads written.
pub fn place(
    file: &mut Value,
    mut ground: impl FnMut(u32, f32, f32) -> Option<f32>,
    mut clear: impl FnMut(u32, &str, f32, f32, f32) -> bool,
) -> Result<usize> {
    let courses = file
        .get_mut("courses")
        .and_then(Value::as_array_mut)
        .context("courses.json: no \"courses\" list")?;
    let mut total = 0;
    for c in courses.iter_mut() {
        let key = c["key"].as_str().unwrap_or("?").to_string();
        let zone = c["zone"].as_str().unwrap_or("").to_string();
        let map = c["map"].as_u64().context("course map")? as u32;
        let circuit = c["laps"].as_u64().unwrap_or(0) > 0;
        let points: Vec<(f32, f32)> = c["checkpoints"]
            .as_array()
            .with_context(|| format!("course {key}: checkpoints"))?
            .iter()
            .map(|p| {
                Ok((
                    p["x"].as_f64().context("checkpoint x")? as f32,
                    p["y"].as_f64().context("checkpoint y")? as f32,
                ))
            })
            .collect::<Result<_>>()
            .with_context(|| format!("course {key}"))?;
        let Some(obj) = c.as_object_mut() else {
            continue;
        };
        obj.remove("pads");
        let leg = |i: usize, t: f32| {
            let (a, b) = (points[i], points[(i + 1) % points.len()]);
            let o = (b.1 - a.1)
                .atan2(b.0 - a.0)
                .rem_euclid(std::f32::consts::TAU);
            (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, o)
        };
        let mut chosen = choose(&points, circuit, |i| {
            SLIDE.into_iter().find(|&t| {
                let (x, y, o) = leg(i, t);
                clear(map, &zone, x, y, o)
            })
        });
        chosen.sort_unstable_by_key(|&(i, _)| i);
        let pads: Vec<Value> = chosen
            .into_iter()
            .map(|(i, t)| {
                let (x, y, o) = leg(i, t);
                let mut m = Map::new();
                m.insert("x".into(), round(f64::from(x), 1));
                m.insert("y".into(), round(f64::from(y), 1));
                if let Some(z) = ground(map, x, y) {
                    m.insert("z".into(), round(f64::from(z), 1));
                }
                m.insert("o".into(), round(f64::from(o), 3));
                Value::Object(m)
            })
            .collect();
        total += pads.len();
        if !pads.is_empty() {
            obj.insert("pads".into(), Value::Array(pads));
        }
    }
    Ok(total)
}

/// Reads `courses`, places the pads over the install's terrain, validates and writes it.
pub fn run(courses: &std::path::Path, chain: benilla_formats::Chain) -> Result<usize> {
    let mut file: Value = serde_json::from_str(
        &std::fs::read_to_string(courses)
            .with_context(|| format!("reading {}", courses.display()))?,
    )
    .with_context(|| format!("parsing {}", courses.display()))?;
    let mut ground = crate::marshal::Ground::new(chain)?;
    // Each zone's terrain, loaded once for its courses: the trees, rocks, buildings, water and
    // slopes a pad may not lie on. A zone that will not load lets every spot through.
    let mut zones: std::collections::HashMap<String, Option<crate::ZoneTerrain>> =
        Default::default();
    let mut install = crate::open_install()?;
    let n = place(
        &mut file,
        |map, x, y| ground.height(map, x, y),
        |_, zone, x, y, o| {
            let terrain = zones.entry(zone.to_string()).or_insert_with(|| {
                match crate::load_zone(&mut install, zone, &crate::TerrainParams::default()) {
                    Ok(t) => Some(t),
                    Err(e) => {
                        eprintln!("pads: {zone}: {e:#}; its pads go unchecked");
                        None
                    }
                }
            });
            let Some(t) = terrain else {
                return true;
            };
            footprint_clear(x, y, o, |x, y| {
                t.height_at(x, y).is_some()
                    && t.solid_at(x, y) == Some(false)
                    && t.water_at(x, y) == Some(false)
                    && t.slope_at(x, y).is_some_and(|s| s <= PAD_SLOPE)
            })
        },
    )?;
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
    fn pads_go_on_the_straightest_legs_never_side_by_side() {
        // A square circuit, 200 on a side, with points every 100: the legs either side of a
        // corner turn 90°, the middle of each side is dead straight... and every leg touches a
        // corner. So a long rectangle: 4 points along each long side.
        let pts = vec![
            (0.0, 0.0),
            (100.0, 0.0),
            (200.0, 0.0),
            (300.0, 0.0),
            (300.0, 100.0),
            (200.0, 100.0),
            (100.0, 100.0),
            (0.0, 100.0),
        ];
        // Straights: legs 1 (100→200 on the bottom) and 5 (200→100 on the top).
        assert_eq!(straights(&pts, true), vec![1, 5]);
        // A sprint along a straight line: every leg but the first qualifies; neighbours never.
        let line: Vec<(f32, f32)> = (0..8).map(|i| (i as f32 * 100.0, 0.0)).collect();
        let s = straights(&line, false);
        assert_eq!(s.len(), MAX_PADS);
        assert!(!s.contains(&0), "the grid's leg");
        assert!(s.windows(2).all(|w| w[1] - w[0] >= 2), "{s:?}");
        // Short legs get none.
        let short: Vec<(f32, f32)> = (0..8).map(|i| (i as f32 * 50.0, 0.0)).collect();
        assert!(straights(&short, false).is_empty());
    }

    #[test]
    fn pads_are_written_along_their_legs_and_the_file_stays_valid() {
        let mut file = serde_json::json!({"courses": [
            {"key": "a", "name": "A", "map": 0, "laps": 0,
             "checkpoints": [{"x": 0, "y": 0}, {"x": 100, "y": 0}, {"x": 200, "y": 0},
                             {"x": 300, "y": 0}]},
            {"key": "b", "name": "B", "map": 1, "laps": 0,
             "checkpoints": [{"x": 0, "y": 0}, {"x": 10, "y": 0}], "pads": [{"x": 1, "y": 1, "o": 0}]}
        ]});
        let n = place(&mut file, |_, x, _| Some(x * 0.5), |_, _, _, _, _| true).unwrap();
        // Legs 1 and 2 are straight but neighbours: the first (by order) wins.
        assert_eq!(n, 1);
        let pad = &file["courses"][0]["pads"][0];
        assert_eq!(pad["x"].as_f64(), Some(130.0));
        assert_eq!(pad["y"].as_f64(), Some(0.0));
        assert_eq!(pad["z"].as_f64(), Some(65.0));
        assert_eq!(pad["o"].as_f64(), Some(0.0));
        // A course with no straight loses any old pads.
        assert!(file["courses"][1].get("pads").is_none());
        crate::export::validate(&file).unwrap();
        // Formatting keeps the pads, one per line.
        let text = crate::export::format(&file);
        assert!(text.contains("\"pads\": [\n        { \"o\": 0, \"x\": 130, \"y\": 0, \"z\": 65 }"));
    }

    #[test]
    fn a_pad_slides_off_a_tree_and_a_blocked_straight_gives_its_place_away() {
        // A sprint along x, points every 100: legs 1-6 straight; a tree across leg 1 at 30%.
        let course = |trees: &[(f32, f32)]| {
            let mut file = serde_json::json!({"courses": [
                {"key": "a", "name": "A", "map": 0, "laps": 0, "zone": "Testland",
                 "checkpoints": (0..8).map(|i| serde_json::json!({"x": i * 100, "y": 0}))
                    .collect::<Vec<_>>()}
            ]});
            let trees = trees.to_vec();
            place(
                &mut file,
                |_, _, _| Some(0.0),
                |_, _, x, y, o| {
                    footprint_clear(x, y, o, |x, y| {
                        trees.iter().all(|t| (t.0 - x).hypot(t.1 - y) > 3.0)
                    })
                },
            )
            .unwrap();
            file["courses"][0]["pads"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| p["x"].as_f64().unwrap() as f32)
                .collect::<Vec<f32>>()
        };
        let open = course(&[]);
        assert_eq!(open.len(), MAX_PADS);
        assert!(open.contains(&130.0), "leg 1 at 30%: {open:?}");
        // A tree at 30% of leg 1: its pad slides off it, still on leg 1.
        let slid = course(&[(130.0, 0.0)]);
        let on_leg_1: Vec<f32> = slid
            .iter()
            .copied()
            .filter(|x| (100.0..200.0).contains(x))
            .collect();
        assert_eq!(on_leg_1.len(), 1, "{slid:?}");
        assert!(
            (on_leg_1[0] - 130.0).abs() >= 3.0 + PAD_SIDE / 2.0,
            "{slid:?}"
        );
        // Trees all along leg 1: no pad there, and three pads all the same.
        let blocked: Vec<(f32, f32)> = (100..=200).step_by(4).map(|x| (x as f32, 0.0)).collect();
        let moved = course(&blocked);
        assert_eq!(moved.len(), MAX_PADS, "{moved:?}");
        assert!(
            moved.iter().all(|x| !(100.0..200.0).contains(x)),
            "{moved:?}"
        );
    }
}
