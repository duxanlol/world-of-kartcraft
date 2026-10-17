//! Every course of `courses/courses.json` measured, driven and drawn, to see which are good and
//! which are not. A course is its gates; a player drives a line of their own through them, so its
//! line here is a smooth one through its gates, as the generator laid them, each leg that runs
//! into anything routed round it instead; a course it does not drive is tried on other lines
//! through its gates ([`crate::generate::gate_line`]) before it is called broken. On that line: the
//! generator's own measures ([`crate::metrics`]: bends, straights, hills, crests that jump, water,
//! room either side, the estimated lap), the shape (how convex, how near it comes to itself, the
//! gaps between gates, the start's slope), and a drive by the CTR-physics bot over the world's
//! real collision as the client builds it ([`crate::soup`]: every doodad's hull, every WMO), with
//! the room it has at kart height measured by rays to either side. Each course's map and
//! elevation profile are drawn as the generator draws its finalists ([`crate::preview`]).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result};
use glam::{Vec2, Vec3};
use kart_driver::{drive, Collision, DriveOpts, Route, CELL, LU_PER_YARD};
use serde::{Deserialize, Serialize};

use crate::cost::{CostField, CostParams};
use crate::drivetest::DriveResult;
use crate::generate::{Candidate, GenParams, Kind, STEP};
use crate::metrics::{self, Metrics};
use crate::select::{Checkpoint, Course};
use crate::soup::{self, bevy_to_wow, wow_to_bevy, Region};
use crate::{load_zone, open_install, TerrainParams};

/// Terrain kept round a zone's chunks: a course leaves its zone a little now and then.
const MARGIN: f32 = 400.0;
/// The world gathered round a course's box, yards flat, and up and down.
const AROUND: f32 = 60.0;
const UP: f32 = 120.0;
/// A waypoint handed to the bot every this many yards.
const WAYPOINT_EVERY: f32 = 15.0;
/// A gate's half-width where the course names none (the client's `GATE_HALF_WIDTH`).
const GATE: f32 = 20.0;
/// Room measured every this many yards, by rays this long at this height over the ground.
const ROOM_EVERY: f32 = 9.0;
const ROOM_REACH: f32 = 25.0;
const ROOM_HEIGHT: f32 = 1.0;
/// Under this much room (yards, wall to wall) a point is cramped.
const CRAMPED: f32 = 10.0;
/// The start's ground looked at within this many yards: a grid of karts stands there.
const START_AREA: f32 = 15.0;

#[derive(Deserialize)]
struct File {
    courses: Vec<FileCourse>,
}

#[derive(Deserialize)]
struct FileCourse {
    key: String,
    name: String,
    #[serde(default)]
    zone: String,
    #[serde(default)]
    series: String,
    map: u32,
    laps: u32,
    #[serde(default)]
    pads: Vec<serde_json::Value>,
    checkpoints: Vec<FileCheckpoint>,
}

#[derive(Deserialize)]
struct FileCheckpoint {
    x: f32,
    y: f32,
    #[serde(default)]
    gate: f32,
}

/// The bot's drive over the real collision.
#[derive(Clone, Debug, Default, Serialize)]
pub struct RealDrive {
    pub finished: bool,
    pub fell: bool,
    pub gave_up: bool,
    pub time: f32,
    pub avg_speed_yds: f32,
    pub stuck_recoveries: u32,
    pub wall_contacts: u32,
    pub slams: u32,
    pub off_track_time: f32,
    pub powerslides: u32,
    pub boosts: u32,
    pub hops: u32,
    pub airtime: f32,
    pub longest_air: f32,
    pub max_slope_deg: f32,
    pub gates_passed: u32,
    pub gates: u32,
    /// Where it stalled (under 3 yd/s for a second), and gave up, WoW yards.
    pub stalls: Vec<[f32; 3]>,
    pub gave_up_at: Option<[f32; 3]>,
    /// The room at kart height, wall to wall (yards, capped at twice [`ROOM_REACH`]): the mean,
    /// and the share of the line under [`CRAMPED`].
    pub room_mean: f32,
    pub cramped_frac: f32,
    /// The share of the line under water.
    pub underwater_frac: f32,
    /// Solid faces within [`AROUND`] of the line, a thousand: how cluttered its world is.
    pub faces_k: f32,
}

/// One course, measured.
#[derive(Clone, Debug, Serialize)]
pub struct Audit {
    pub key: String,
    pub name: String,
    pub zone: String,
    pub series: String,
    pub map: u32,
    pub laps: u32,
    pub closed: bool,
    pub checkpoints: usize,
    pub pads: usize,
    /// The gaps between neighbouring gates, yards.
    pub gate_gap_min: f32,
    pub gate_gap_max: f32,
    /// The area the line encloses over its convex hull's (circuits; 1 a convex blob).
    pub convexity: f32,
    /// How near the line comes to itself away from where it is (yards; 0 when it crosses).
    pub min_separation: f32,
    pub crosses_itself: bool,
    /// The steepest ground (degrees) within [`START_AREA`] of the start.
    pub start_slope: f32,
    /// Legs between gates no route takes round what stands between, driven straight.
    pub straight_legs: usize,
    /// How far the line passes from its farthest gate, yards (a gate is 20 either side).
    pub gate_miss_max: f32,
    pub metrics: Metrics,
    pub drive: Option<RealDrive>,
    /// Where the Balanced bot did not finish the bent line, the first other line and kart that
    /// did (the routed line, the taut one where it keeps to its gates, by Balanced then Turn),
    /// else the last tried: a course none finishes is broken, one only the first fails awkward.
    pub retry: Option<RealDrive>,
    pub retry_line: String,
    pub error: Option<String>,
    /// The line, WoW x, y, z, every few yards.
    pub line: Vec<[f32; 3]>,
}

/// A smooth line through `gates` (centripetal Catmull-Rom; back to the first on a circuit), as the
/// generator laid its gates on a smooth line: a leg of it that runs into anything is routed round
/// instead ([`crate::generate::Planner::route`], its staircase rounded off).
fn spline_line(
    planner: &crate::generate::Planner,
    field: &CostField,
    gates: &[Vec2],
    closed: bool,
) -> Vec<Vec2> {
    let n = gates.len();
    if n < 2 {
        return gates.to_vec();
    }
    let at = |i: isize| -> Vec2 {
        if closed {
            gates[i.rem_euclid(n as isize) as usize]
        } else {
            gates[i.clamp(0, n as isize - 1) as usize]
        }
    };
    let segs = if closed { n } else { n - 1 };
    let mut out: Vec<Vec2> = Vec::new();
    let extra = std::collections::HashMap::new();
    for s in 0..segs as isize {
        let (p0, p1, p2, p3) = (at(s - 1), at(s), at(s + 1), at(s + 2));
        let t = |a: Vec2, b: Vec2| a.distance(b).max(1e-3).sqrt();
        let (t1, t2, t3) = (t(p0, p1), t(p1, p2), t(p2, p3));
        let (k0, k1, k2, k3) = (0.0, t1, t1 + t2, t1 + t2 + t3);
        let k = (p1.distance(p2) / (STEP * 0.5)).ceil().max(1.0) as usize;
        let leg: Vec<Vec2> = (0..k)
            .map(|j| {
                let u = k1 + (k2 - k1) * j as f32 / k as f32;
                let a1 = p0 * ((k1 - u) / (k1 - k0)) + p1 * ((u - k0) / (k1 - k0));
                let a2 = p1 * ((k2 - u) / (k2 - k1)) + p2 * ((u - k1) / (k2 - k1));
                let a3 = p2 * ((k3 - u) / (k3 - k2)) + p3 * ((u - k2) / (k3 - k2));
                let b1 = a1 * ((k2 - u) / (k2 - k0)) + a2 * ((u - k0) / (k2 - k0));
                let b2 = a2 * ((k3 - u) / (k3 - k1)) + a3 * ((u - k1) / (k3 - k1));
                b1 * ((k2 - u) / (k2 - k1)) + b2 * ((u - k1) / (k2 - k1))
            })
            .collect();
        let blocked = leg
            .iter()
            .any(|q| field.clearance_at(q.x, q.y) < SPLINE_ROOM);
        let routed = blocked
            .then(|| {
                let (a, b) = (
                    planner.nearest_open(p1, 8.0).unwrap_or(p1),
                    planner.nearest_open(p2, 8.0).unwrap_or(p2),
                );
                planner.route(a, b, &extra, None, None)
            })
            .flatten();
        match routed {
            Some(r) => {
                let pts = &r.points[..r.points.len() - 1];
                out.extend(crate::geom::chaikin(
                    &crate::geom::resample(pts, false, STEP),
                    false,
                    2,
                ));
            }
            None => out.extend(leg),
        }
    }
    if !closed {
        out.push(gates[n - 1]);
    }
    crate::geom::resample(&out, closed, STEP)
}

/// A spline leg with less room than this (yards) anywhere is routed round instead.
const SPLINE_ROOM: f32 = 1.5;

/// The polygon's area (shoelace).
fn area(p: &[Vec2]) -> f32 {
    let n = p.len();
    (0..n)
        .map(|i| {
            let (a, b) = (p[i], p[(i + 1) % n]);
            a.x * b.y - b.x * a.y
        })
        .sum::<f32>()
        .abs()
        * 0.5
}

/// The convex hull (monotone chain).
fn hull(p: &[Vec2]) -> Vec<Vec2> {
    let mut pts = p.to_vec();
    pts.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    let cross = |o: Vec2, a: Vec2, b: Vec2| (a - o).perp_dot(b - o);
    let mut lower: Vec<Vec2> = Vec::new();
    for &q in &pts {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], q) <= 0.0 {
            lower.pop();
        }
        lower.push(q);
    }
    let mut upper: Vec<Vec2> = Vec::new();
    for &q in pts.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], q) <= 0.0 {
            upper.pop();
        }
        upper.push(q);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// The bot drives `line` (WoW x, y and a height to search the ground from) once round, or
/// start to finish, over the world's real collision round it.
pub fn drive_real(
    chain: &mut benilla_formats::Chain,
    map: u32,
    line: &[[f32; 3]],
    closed: bool,
    class: kart_driver::Class,
) -> Result<RealDrive> {
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for q in line {
        let b = wow_to_bevy(*q);
        lo = lo.min(b);
        hi = hi.max(b);
    }
    let region = Region {
        min: lo - Vec3::new(AROUND, UP, AROUND),
        max: hi + Vec3::new(AROUND, UP, AROUND),
    };
    let faces = soup::gather_world(chain, map, region)?;
    let origin = wow_to_bevy(line[0]);
    let world = Collision::new(soup::to_lu(&faces.solid, origin), CELL);
    let water = Collision::new(soup::to_lu(&faces.liquid, origin), CELL);
    let lu = |v: Vec3| (v - origin) * LU_PER_YARD;
    let y = LU_PER_YARD;
    let ground = |q: [f32; 3]| {
        let at = lu(wow_to_bevy([q[0], q[1], q[2] + 30.0]));
        world
            .ground(at, 90.0 * y)
            .map_or(lu(wow_to_bevy(q)), |h| h.point)
    };
    let flat: Vec<Vec2> = line.iter().map(|q| Vec2::new(q[0], q[1])).collect();
    let near_z = |q: Vec2| {
        line.iter()
            .min_by(|a, b| {
                let d = |r: &[f32; 3]| (r[0] - q.x).hypot(r[1] - q.y);
                d(a).total_cmp(&d(b))
            })
            .map_or(0.0, |r| r[2])
    };
    let even = crate::geom::resample(&flat, closed, WAYPOINT_EVERY);
    let waypoints: Vec<Vec3> = even
        .iter()
        .map(|q| ground([q.x, q.y, near_z(*q)]))
        .collect();
    // The room either side at kart height, and the water over the line.
    let rooms = crate::geom::resample(&flat, closed, ROOM_EVERY);
    let (mut room_sum, mut cramped, mut wet) = (0.0, 0usize, 0usize);
    for (k, q) in rooms.iter().enumerate() {
        let g = ground([q.x, q.y, near_z(*q)]);
        let next = rooms[(k + 1).min(rooms.len() - 1)];
        let prev = rooms[k.saturating_sub(1)];
        let d = (next - prev).normalize_or_zero();
        // Across the line: WoW's (x, y) along it is Bevy's (−y, −x) (bevy = (−y, z, −x)), and
        // a quarter turn of that is (−x, y).
        let side = Vec3::new(-d.x, 0.0, d.y).normalize_or_zero();
        let from = g + Vec3::Y * ROOM_HEIGHT * y;
        let reach = |dir: Vec3| {
            world
                .ray(from, dir, ROOM_REACH * y)
                .map_or(ROOM_REACH, |h| h.distance / y)
        };
        let room = reach(side) + reach(-side);
        room_sum += room;
        cramped += usize::from(room < CRAMPED);
        wet += usize::from(
            water
                .ray(g + Vec3::Y * 40.0 * y, Vec3::NEG_Y, 39.5 * y)
                .is_some(),
        );
    }
    let n_room = rooms.len().max(1) as f32;
    let len = crate::geom::length(&flat, closed);
    let route = Route::new(waypoints, u32::from(closed), GATE * y);
    let opts = DriveOpts {
        time_limit: len / 12.0 + 30.0,
        off_track_yards: 20.0,
        class,
        ..Default::default()
    };
    let rep = drive(&world, &route, &opts);
    let wow = |v: Vec3| bevy_to_wow(v / y + origin);
    let mut stalls = Vec::new();
    let mut slow = 0.0;
    for tp in &rep.trace {
        if tp.t > 3.0 && tp.speed < 3.0 * y {
            slow += 0.25;
            if (slow - 1.0f32).abs() < 1e-3 {
                stalls.push(wow(tp.position));
            }
        } else {
            slow = 0.0;
        }
    }
    Ok(RealDrive {
        finished: rep.finished,
        fell: rep.fell,
        gave_up: rep.gave_up,
        time: rep.time,
        avg_speed_yds: rep.avg_speed / y,
        stuck_recoveries: rep.stuck_recoveries,
        wall_contacts: rep.wall_contacts,
        slams: rep.slams,
        off_track_time: rep.off_track_time,
        powerslides: rep.powerslides,
        boosts: rep.boosts,
        hops: rep.hops,
        airtime: rep.airtime,
        longest_air: rep.longest_air,
        max_slope_deg: rep.max_slope_deg,
        gates_passed: rep.gates_passed,
        gates: route.gates.len() as u32,
        gave_up_at: (!rep.finished)
            .then(|| rep.trace.last().map(|tp| wow(tp.position)))
            .flatten(),
        stalls,
        room_mean: room_sum / n_room,
        cramped_frac: cramped as f32 / n_room,
        underwater_frac: wet as f32 / n_room,
        faces_k: faces.solid.len() as f32 / 1000.0,
    })
}

/// Every course of `courses` (those whose key holds `only`, if given), its zones `jobs` at a
/// time: the measures as JSON in `out/audit.json`, each course's map and profile beside it.
pub fn run(courses: &Path, out: &Path, jobs: usize, only: Option<&str>) -> Result<Vec<Audit>> {
    let file: File = serde_json::from_str(
        &std::fs::read_to_string(courses)
            .with_context(|| format!("reading {}", courses.display()))?,
    )?;
    std::fs::create_dir_all(out)?;
    let mut by_zone: BTreeMap<String, Vec<&FileCourse>> = BTreeMap::new();
    for c in &file.courses {
        if only.is_none_or(|o| c.key.contains(o)) {
            by_zone.entry(c.zone.clone()).or_default().push(c);
        }
    }
    let zones: Vec<(&String, &Vec<&FileCourse>)> = by_zone.iter().collect();
    let done = Mutex::new(Vec::new());
    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..jobs.max(1) {
            s.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some((zone, list)) = zones.get(i) else {
                    break;
                };
                let started = std::time::Instant::now();
                let results = audit_zone(zone, list, out);
                eprintln!(
                    "{zone}: {} courses in {:.0}s",
                    results.len(),
                    started.elapsed().as_secs_f32()
                );
                done.lock().unwrap().extend(results);
            });
        }
    });
    let mut all = done.into_inner().unwrap();
    all.sort_by(|a, b| a.key.cmp(&b.key));
    std::fs::write(out.join("audit.json"), serde_json::to_string(&all)?)?;
    Ok(all)
}

fn audit_zone(zone: &str, list: &[&FileCourse], out: &Path) -> Vec<Audit> {
    let failed = |c: &FileCourse, e: String| Audit {
        key: c.key.clone(),
        name: c.name.clone(),
        zone: c.zone.clone(),
        series: c.series.clone(),
        map: c.map,
        laps: c.laps,
        closed: c.laps > 0,
        checkpoints: c.checkpoints.len(),
        pads: c.pads.len(),
        gate_gap_min: 0.0,
        gate_gap_max: 0.0,
        convexity: 0.0,
        min_separation: 0.0,
        crosses_itself: false,
        start_slope: 0.0,
        straight_legs: 0,
        gate_miss_max: 0.0,
        metrics: Metrics::default(),
        drive: None,
        retry: None,
        retry_line: String::new(),
        error: Some(e),
        line: Vec::new(),
    };
    // WMO floors laid: a bridge's deck is where the line runs, not the gorge under it.
    let params = TerrainParams {
        margin: MARGIN,
        wmo_floors: true,
        ..TerrainParams::default()
    };
    let mut chain = match open_install() {
        Ok(c) => c,
        Err(e) => return list.iter().map(|c| failed(c, format!("{e:#}"))).collect(),
    };
    let t = match load_zone(&mut chain, zone, &params) {
        Ok(t) => t,
        Err(e) => return list.iter().map(|c| failed(c, format!("{e:#}"))).collect(),
    };
    let field = CostField::build(&t, &CostParams::default());
    let planner = crate::generate::Planner::new(&field, 2);
    let mut results = Vec::new();
    for c in list {
        let closed = c.laps > 0;
        let gates: Vec<Vec2> = c.checkpoints.iter().map(|p| Vec2::new(p.x, p.y)).collect();
        use crate::generate::LineStyle;
        let line_of = |style| {
            crate::generate::gate_line(&planner, &gates, closed, &GenParams::default(), style)
        };
        // Its shape is measured on the taut line bent back to its gates, which turns as a kart
        // does; the bot drives the spline first, which keeps nearest its gates.
        let (shape, straight_legs) = line_of(LineStyle::Bent);
        let pts = spline_line(&planner, &field, &gates, closed);
        let cand = Candidate {
            kind: if closed { Kind::Circuit } else { Kind::Sprint },
            name: c.name.clone(),
            pts: shape.clone(),
            closed,
            seed: 0,
        };
        let m = metrics::measure(&cand, &t, &field);
        let with_z = |p: &[Vec2]| -> Vec<[f32; 3]> {
            let h = metrics::heights(&t, p, closed);
            p.iter().zip(&h).map(|(p, &z)| [p.x, p.y, z]).collect()
        };
        let line = with_z(&pts);
        let shape_line = with_z(&shape);
        let gaps: Vec<f32> = (0..gates.len())
            .filter(|&i| closed || i + 1 < gates.len())
            .map(|i| gates[i].distance(gates[(i + 1) % gates.len()]))
            .collect();
        let convexity = if closed {
            let h = hull(&shape);
            area(&shape) / area(&h).max(1.0)
        } else {
            0.0
        };
        let start_slope = {
            let s = gates[0];
            let mut worst = 0.0f32;
            for dx in -7..=7 {
                for dy in -7..=7 {
                    let q = s + Vec2::new(dx as f32, dy as f32) * (START_AREA / 7.0);
                    if q.distance(s) <= START_AREA {
                        if let Some(i) = t.nearest(q.x, q.y) {
                            worst = worst.max(t.slope[i]);
                        }
                    }
                }
            }
            worst
        };
        let mut go = |line: &[[f32; 3]], class| {
            if c.map > 1 {
                return None;
            }
            drive_real(&mut chain, c.map, line, closed, class)
                .inspect_err(|e| eprintln!("{}: {e:#}", c.key))
                .ok()
        };
        let drive = go(&line, kart_driver::Class::Balanced);
        // Not finished: does any line through the gates drive? The routed one, then the taut one
        // where it keeps to its gates, each by both karts.
        let mut retry = None;
        let mut retry_line = String::new();
        if drive.as_ref().is_some_and(|d| !d.finished) {
            let mut lines: Vec<(&str, Vec<[f32; 3]>)> = vec![("spline", line.clone())];
            for (name, style) in [
                ("bent", LineStyle::Bent),
                ("routed", LineStyle::Routed),
                ("taut", LineStyle::Taut),
            ] {
                let (p2, _) = line_of(style);
                let miss = gates
                    .iter()
                    .map(|g| {
                        p2.iter()
                            .map(|q| q.distance(*g))
                            .fold(f32::INFINITY, f32::min)
                    })
                    .fold(0.0, f32::max);
                if miss <= GATE {
                    let h = metrics::heights(&t, &p2, closed);
                    lines.push((
                        name,
                        p2.iter().zip(&h).map(|(p, &z)| [p.x, p.y, z]).collect(),
                    ));
                }
            }
            'tries: for class in [kart_driver::Class::Balanced, kart_driver::Class::Turn] {
                for (name, l) in &lines {
                    if *name == "spline" && class == kart_driver::Class::Balanced {
                        continue;
                    }
                    let d = go(l, class);
                    let done = d.as_ref().is_some_and(|d| d.finished);
                    if done || retry.is_none() {
                        retry = d;
                        retry_line = format!("{name}, {class:?}");
                    }
                    if done {
                        break 'tries;
                    }
                }
            }
        }
        let a = Audit {
            key: c.key.clone(),
            name: c.name.clone(),
            zone: c.zone.clone(),
            series: c.series.clone(),
            map: c.map,
            laps: c.laps,
            closed,
            checkpoints: c.checkpoints.len(),
            pads: c.pads.len(),
            gate_gap_min: gaps.iter().copied().fold(f32::INFINITY, f32::min),
            gate_gap_max: gaps.iter().copied().fold(0.0, f32::max),
            convexity,
            min_separation: crate::geom::min_separation(&shape, closed, STEP, 120.0),
            crosses_itself: crate::geom::self_intersects(&shape, closed),
            start_slope,
            straight_legs,
            gate_miss_max: gates
                .iter()
                .map(|g| {
                    shape
                        .iter()
                        .map(|q| q.distance(*g))
                        .fold(f32::INFINITY, f32::min)
                })
                .fold(0.0, f32::max),
            metrics: m,
            drive,
            retry,
            retry_line,
            error: None,
            line: shape_line.iter().step_by(3).copied().collect(),
        };
        draw(&t, &field, c, &a, &shape_line, &gates, out);
        results.push(a);
    }
    results
}

/// The course's map and profile, as the generator draws its finalists.
fn draw(
    t: &crate::ZoneTerrain,
    field: &CostField,
    c: &FileCourse,
    a: &Audit,
    line: &[[f32; 3]],
    gates: &[Vec2],
    out: &Path,
) {
    let pts: Vec<Vec2> = line.iter().map(|p| Vec2::new(p[0], p[1])).collect();
    let checkpoints: Vec<Checkpoint> = gates
        .iter()
        .enumerate()
        .map(|(k, g)| {
            let (index, _) = pts
                .iter()
                .enumerate()
                .min_by(|x, y| x.1.distance(*g).total_cmp(&y.1.distance(*g)))
                .unwrap_or((0, g));
            let next = pts[(index + 1).min(pts.len() - 1)];
            Checkpoint {
                index,
                s: index as f32 * STEP,
                x: g.x,
                y: g.y,
                z: line[index][2],
                heading: (next - pts[index]).normalize_or_zero().to_array(),
                half_width: if c.checkpoints[k].gate > 0.0 {
                    c.checkpoints[k].gate
                } else {
                    GATE
                },
                kind: if k == 0 { "start" } else { "spacing" },
            }
        })
        .collect();
    let drive = a.drive.as_ref().map(|d| DriveResult {
        ok: d.finished && !d.fell && !d.gave_up,
        finished: d.finished,
        time: d.time,
        best_lap: d.time,
        stalls: d.stalls.clone(),
        wall_contacts: d.wall_contacts,
        ..Default::default()
    });
    let course = Course {
        rank: 0,
        name: a.name.clone(),
        kind: if a.closed {
            Kind::Circuit
        } else {
            Kind::Sprint
        },
        closed: a.closed,
        laps: a.laps,
        seed: 0,
        score: a.metrics.static_score,
        metrics: a.metrics.clone(),
        drive,
        checkpoints,
        route: line.to_vec(),
    };
    let _ = crate::preview::map(t, field, &course).save(out.join(format!("{}.png", a.key)));
    let _ = crate::preview::profile(&course).save(out.join(format!("{}-profile.png", a.key)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_square_fills_its_hull_and_an_l_does_not() {
        let square = [
            Vec2::new(0.0, 0.0),
            Vec2::new(10.0, 0.0),
            Vec2::new(10.0, 10.0),
            Vec2::new(0.0, 10.0),
        ];
        assert!((area(&square) / area(&hull(&square)) - 1.0).abs() < 1e-5);
        let l = [
            Vec2::new(0.0, 0.0),
            Vec2::new(10.0, 0.0),
            Vec2::new(10.0, 4.0),
            Vec2::new(4.0, 4.0),
            Vec2::new(4.0, 10.0),
            Vec2::new(0.0, 10.0),
        ];
        let convexity = area(&l) / area(&hull(&l));
        // 64 of the hull's 82.
        assert!((convexity - 64.0 / 82.0).abs() < 1e-3, "{convexity}");
    }
}
