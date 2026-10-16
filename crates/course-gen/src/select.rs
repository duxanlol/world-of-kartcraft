//! The whole course run for a zone: generate, measure, keep the distinct, test-drive the best,
//! score, and place each finalist's checkpoints.

use std::collections::HashMap;

use glam::Vec2;
use serde::Serialize;

use crate::cost::{CostField, CostParams};
use crate::drivetest::{test_drive, DriveResult};
use crate::generate::{self, Candidate, GenParams, Hub, Kind, Planner, STEP};
use crate::geom;
use crate::metrics::{self, Metrics};
use crate::roads::RoadGraph;
use crate::terrain::ZoneTerrain;

/// Laps a circuit is raced over.
pub const LAPS: u32 = 3;
/// Fewer hubs than this on roomy ground, and smaller pieces of open ground take hubs too.
const MIN_HUBS: usize = 8;
/// The steepest open ground (degrees) for a zone with too few hubs otherwise.
const STEEP_FALLBACK: f32 = 38.0;
/// The most of its line a finalist may share with a better one.
const MAX_SHARED: f32 = 0.35;

#[derive(Clone, Debug)]
pub struct SelectParams {
    pub gen: GenParams,
    pub cost: CostParams,
    /// How many of the best by static score are driven.
    pub drive: usize,
    /// How many finalists are kept.
    pub keep: usize,
    /// The circuit's waypoints (WoW `x, y`), in driving order: on a map whose ground is its
    /// streets ([`generate::street_circuit`]), or a designed circuit on open ground
    /// ([`generate::designed_circuit`]), the one candidate.
    pub via: Vec<Vec2>,
}

impl Default for SelectParams {
    fn default() -> Self {
        Self {
            gen: GenParams::default(),
            cost: CostParams::default(),
            drive: 40,
            keep: 8,
            via: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Checkpoint {
    /// Index into the route polyline, and yards along it.
    pub index: usize,
    pub s: f32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// The course direction through the gate, world (x, y), unit.
    pub heading: [f32; 2],
    pub half_width: f32,
    /// `start`, `apex`, `spacing` or `finish`.
    pub kind: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub struct Course {
    pub rank: usize,
    pub name: String,
    pub kind: Kind,
    pub closed: bool,
    pub laps: u32,
    pub seed: u64,
    pub score: f32,
    pub metrics: Metrics,
    pub drive: Option<DriveResult>,
    pub checkpoints: Vec<Checkpoint>,
    /// The line in WoW world `(x, y, z)` yards, every [`STEP`] yards, from the start.
    pub route: Vec<[f32; 3]>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct RunStats {
    pub hubs: usize,
    /// The largest piece of connected open ground, km².
    pub largest_open_km2: f32,
    /// The steepest ground (degrees) counted open.
    pub max_slope: f32,
    pub attempts: usize,
    pub generated: usize,
    pub distinct: usize,
    pub driven: usize,
    pub drive_ok: usize,
    pub kept: usize,
    /// Wall contacts over every course driven, and of those in hairpins.
    pub wall_contacts: u32,
    pub hairpin_contacts: u32,
    pub rejects: Vec<String>,
    /// Generation attempts that failed, by reason.
    pub failures: Vec<(String, usize)>,
}

pub struct Run {
    pub field: CostField,
    pub hubs: Vec<Hub>,
    pub stats: RunStats,
    pub finalists: Vec<Course>,
    /// Driven and rejected, for inspection.
    pub rejected: Vec<Course>,
}

pub fn run(t: &ZoneTerrain, roads: &RoadGraph, p: &SelectParams) -> Run {
    // A hub needs room for a course: 0.5 km² of connected open ground, or, in a zone broken up
    // by slopes and trees where few places have that, 0.2 km², then 0.08; failing that, ground up to
    // [`STEEP_FALLBACK`] counts as open and trees keep a yard of margin, not two (the test drive
    // still has to climb it and thread them).
    let hubs_on = |field: &CostField| {
        let planner = Planner::new(field, 2);
        let mut hubs = Vec::new();
        for reach in [600_000.0, 240_000.0, 100_000.0] {
            hubs = generate::hubs(t, roads, &planner, reach);
            if hubs.len() >= MIN_HUBS {
                break;
            }
        }
        hubs
    };
    // A city's streets are lined with walls and barrels the margin would close a lane over: every
    // pixel keeps a paved lane's sliver of margin there.
    let street_cost = CostParams {
        obstacle_margin: 0.0,
        lane_margin: 0.0,
        ..p.cost.clone()
    };
    let mut field = CostField::build(t, if t.wmo_ground { &street_cost } else { &p.cost });
    let mut hubs = hubs_on(&field);
    let mut max_slope = p.cost.max_slope;
    if hubs.len() < MIN_HUBS && p.cost.max_slope < STEEP_FALLBACK {
        let cost = CostParams {
            max_slope: STEEP_FALLBACK,
            obstacle_margin: p.cost.obstacle_margin.min(1.0),
            ..p.cost.clone()
        };
        let steep = CostField::build(t, &cost);
        let more = hubs_on(&steep);
        if more.len() > hubs.len() {
            (field, hubs, max_slope) = (steep, more, STEEP_FALLBACK);
        }
    }
    let planner = Planner::new(&field, 2);
    // A city's streets are too narrow for hubs: its circuit goes by the streets `via` names.
    let (mut cands, why) = if t.wmo_ground {
        let street = Planner::with_clearance(&field, 1, generate::STREET_ROUTE_CLEARANCE);
        match generate::street_circuit(&street, &p.via, &p.gen) {
            Ok(c) => (vec![c], HashMap::new()),
            Err(e) => (Vec::new(), HashMap::from([(e, 1)])),
        }
    } else if !p.via.is_empty() {
        match generate::designed_circuit(&planner, &p.via, &p.gen) {
            Ok(c) => (vec![c], HashMap::new()),
            Err(e) => (Vec::new(), HashMap::from([(e, 1)])),
        }
    } else {
        generate::generate(&planner, &hubs, &p.gen)
    };
    let mut failures: Vec<(String, usize)> =
        why.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    failures.sort_by_key(|a| std::cmp::Reverse(a.1));
    let mut stats = RunStats {
        failures,
        hubs: hubs.len(),
        largest_open_km2: planner.largest_reach() * 0.836_127 / 1e6,
        max_slope,
        attempts: p.gen.attempts,
        generated: cands.len(),
        ..Default::default()
    };
    // A circuit by waypoints starts at its first, where it was asked to.
    if !t.wmo_ground && p.via.is_empty() {
        for c in &mut cands {
            generate::place_start(c, t, &field);
        }
    }
    let mut measured: Vec<(Candidate, Metrics)> = cands
        .into_iter()
        .map(|c| {
            let m = metrics::measure(&c, t, &field);
            (c, m)
        })
        .collect();
    measured.sort_by(|a, b| b.1.static_score.total_cmp(&a.1.static_score));
    let mut distinct: Vec<(Candidate, Metrics)> = Vec::new();
    for (c, m) in measured {
        if distinct.iter().all(|(d, _)| !generate::similar(&c, d)) {
            distinct.push((c, m));
        }
    }
    stats.distinct = distinct.len();

    let to_drive: Vec<(Candidate, Metrics)> = distinct.into_iter().take(p.drive).collect();
    stats.driven = to_drive.len();
    let threads = p.gen.threads.max(1);
    let driven: Vec<(Candidate, Metrics, DriveResult)> = std::thread::scope(|s| {
        let chunks: Vec<_> = to_drive
            .chunks(to_drive.len().div_ceil(threads).max(1))
            .map(|chunk| {
                s.spawn(move || {
                    chunk
                        .iter()
                        .map(|(c, m)| {
                            let (r, _) = test_drive(t, c, LAPS);
                            (c.clone(), m.clone(), r)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        chunks
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    });
    let mut scored: Vec<(f32, Candidate, Metrics, DriveResult)> = Vec::new();
    let mut rejected = Vec::new();
    for (c, m, r) in driven {
        stats.wall_contacts += r.wall_contacts;
        stats.hairpin_contacts += r.hairpin_contacts;
        if let Some(why) = &r.reject {
            stats.rejects.push(format!("{}: {why}", c.name));
            rejected.push(course(t, &field, 0, 0.0, &c, m, r));
            continue;
        }
        stats.drive_ok += 1;
        let s = final_score(&m, &r, c.closed);
        scored.push((s, c, m, r));
    }
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    // The finalists differ: each shares at most a third of its line with any above it.
    let mut picked: Vec<(f32, Candidate, Metrics, DriveResult)> = Vec::new();
    for e in scored {
        if picked.len() == p.keep {
            break;
        }
        let shared = |o: &(f32, Candidate, Metrics, DriveResult)| {
            (generate::overlap(&e.1.pts, &o.1.pts, 30.0)
                + generate::overlap(&o.1.pts, &e.1.pts, 30.0))
                / 2.0
        };
        if picked.iter().all(|o| shared(o) <= MAX_SHARED) {
            picked.push(e);
        }
    }
    let mut finalists: Vec<Course> = picked
        .into_iter()
        .enumerate()
        .map(|(rank, (score, c, m, r))| course(t, &field, rank + 1, score, &c, m, r))
        .collect();
    name_courses(t, &mut finalists);
    stats.kept = finalists.len();
    Run {
        field,
        hubs,
        stats,
        finalists,
        rejected,
    }
}

/// Name each course by the places it runs through: a circuit by the one or two subzones it
/// covers most, a sprint by the subzones at its ends; repeats get a numeral.
fn name_courses(t: &ZoneTerrain, courses: &mut [Course]) {
    let place = |x: f32, y: f32| {
        t.area_at(x, y)
            .filter(|&a| a != t.area_id)
            .and_then(|a| t.area_names.get(&a).cloned())
    };
    let mut used: Vec<String> = Vec::new();
    for c in courses.iter_mut() {
        let base = if c.closed {
            let own: Vec<&String> = c
                .metrics
                .subzones
                .iter()
                .filter(|s| **s != t.zone)
                .collect();
            match own.as_slice() {
                [] => format!("{} Fields Circuit", t.zone),
                [a] => format!("{a} Circuit"),
                [a, b, ..] => format!("{a} & {b} Circuit"),
            }
        } else {
            let (Some(a), Some(b)) = (c.route.first(), c.route.last()) else {
                continue;
            };
            let from = place(a[0], a[1]).unwrap_or_else(|| t.zone.clone());
            let to = place(b[0], b[1]).unwrap_or_else(|| t.zone.clone());
            if from == to {
                format!("{from} Sprint")
            } else {
                format!("{from} to {to} Sprint")
            }
        };
        let n = used.iter().filter(|u| **u == base).count();
        used.push(base.clone());
        c.name = match n {
            0 => base,
            1 => format!("{base} II"),
            2 => format!("{base} III"),
            k => format!("{base} {}", k + 1),
        };
    }
}

fn course(
    t: &ZoneTerrain,
    field: &CostField,
    rank: usize,
    score: f32,
    c: &Candidate,
    m: Metrics,
    r: DriveResult,
) -> Course {
    let z = metrics::heights(t, &c.pts, c.closed);
    let gap = if t.wmo_ground {
        STREET_GATE_GAP
    } else {
        GATE_GAP
    };
    let checkpoints = checkpoints(c, &m, &z, field, gap);
    Course {
        rank,
        name: c.name.clone(),
        kind: c.kind,
        closed: c.closed,
        laps: if c.closed { LAPS } else { 0 },
        seed: c.seed,
        score,
        metrics: m,
        drive: Some(r),
        checkpoints,
        route: c.pts.iter().zip(&z).map(|(p, &z)| [p.x, p.y, z]).collect(),
    }
}

/// The fun of a course once driven.
pub fn final_score(m: &Metrics, r: &DriveResult, closed: bool) -> f32 {
    let laps = if closed { LAPS as f32 } else { 1.0 };
    let mut s = m.static_score;
    s += 0.8 * (r.powerslides as f32 / laps).min(8.0);
    s += 0.3 * (r.boosts as f32 / laps).min(12.0);
    s += 5.0 * r.longest_air.min(1.0);
    s += 1.5 * (r.airtime / laps).min(2.0);
    let lap = r.best_lap;
    let (lo, hi) = if closed { (50.0, 110.0) } else { (45.0, 120.0) };
    if (lo..=hi).contains(&lap) {
        s += 3.0;
    } else {
        s -= 0.15 * (lo - lap).max(lap - hi);
    }
    let scrapes = r.hairpin_contacts as f32;
    s -= (1.0 * (r.wall_contacts as f32 - scrapes) + 0.25 * scrapes) / laps;
    s -= 3.0 * r.stuck_recoveries as f32;
    s -= 0.5 * r.off_track_time;
    s
}

/// Gates no further apart (yards) on open ground, and on a city's streets: there the line from
/// one gate to the next, which the client's arrow points along, must not cross a house.
pub const GATE_GAP: f32 = 180.0;
pub const STREET_GATE_GAP: f32 = 50.0;

/// Gates at the start, at the apex of every bend of 45° or more, and between them so no two are
/// more than about `max_gap` yd apart ([`GATE_GAP`], [`STREET_GATE_GAP`]); each as wide as the
/// ground allows, 15–25 yd each side.
pub fn checkpoints(
    c: &Candidate,
    m: &Metrics,
    z: &[f32],
    field: &CostField,
    max_gap: f32,
) -> Vec<Checkpoint> {
    let n = c.pts.len();
    let total = geom::length(&c.pts, c.closed);
    let mut keys: Vec<(usize, &'static str)> = vec![(0, "start")];
    let mut apexes: Vec<usize> = m
        .bends
        .iter()
        .filter(|b| b.angle_deg.abs() >= 45.0)
        .map(|b| b.at)
        .collect();
    apexes.sort();
    for a in apexes {
        let s = a as f32 * STEP;
        let last = keys.last().map_or(0.0, |k| k.0 as f32 * STEP);
        let to_end = if c.closed {
            total - s
        } else {
            (n - 1) as f32 * STEP - s
        };
        if s - last >= 60.0 && to_end >= 60.0 {
            keys.push((a, "apex"));
        }
    }
    let end = if c.closed { n } else { n - 1 };
    let mut out_idx: Vec<(usize, &'static str)> = Vec::new();
    for (k, &(i, kind)) in keys.iter().enumerate() {
        out_idx.push((i, kind));
        let next = keys.get(k + 1).map_or(end, |k| k.0);
        let gap = (next - i) as f32 * STEP;
        if gap > max_gap {
            let parts = (gap / (max_gap * 8.0 / 9.0)).ceil() as usize;
            for j in 1..parts {
                out_idx.push((i + (next - i) * j / parts, "spacing"));
            }
        }
    }
    if !c.closed {
        out_idx.push((n - 1, "finish"));
    }
    out_idx
        .into_iter()
        .map(|(i, kind)| {
            let p = c.pts[i];
            let a = c.pts[geom::wrap(i as isize - 2, n, c.closed)];
            let b = c.pts[geom::wrap(i as isize + 2, n, c.closed)];
            let h = (b - a).normalize_or(Vec2::X);
            let clr = field.clearance_at(p.x, p.y);
            Checkpoint {
                index: i,
                s: i as f32 * STEP,
                x: p.x,
                y: p.y,
                z: z[i],
                heading: [h.x, h.y],
                half_width: (clr * 1.5 + 5.0).clamp(15.0, 25.0),
                kind,
            }
        })
        .collect()
}
