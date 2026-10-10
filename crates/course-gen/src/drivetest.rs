//! Test-drive a course with the CTR-physics bot: the terrain in a corridor round the line as 2-yd
//! tiles, the solid footprint pixels there as blocks, all in a local kart frame.
//!
//! The frame keeps handedness: CTR `(x, y, z)` = WoW `(y, z, x)` about an origin, in lu (the
//! physics spec's `wow = (ctr.z, ctr.x, ctr.y)`), so +Y is up.

use glam::Vec3;
use kart_driver::{drive, Collision, DriveOpts, DriveReport, Route, CELL, LU_PER_YARD};
use serde::Serialize;

use crate::generate::Candidate;
use crate::terrain::ZoneTerrain;

/// Half-width of the terrain corridor, yards.
pub const CORRIDOR: f32 = 70.0;
/// Height of a solid footprint block above the ground, yards.
const BLOCK_HEIGHT: f32 = 6.0;
/// A wall contact this near a hairpin (yards along the line) is a hairpin scrape.
pub const HAIRPIN_MARGIN: f32 = 20.0;
/// Hairpin scrapes allowed a lap: this many per hairpin, plus [`HAIRPIN_SCRAPES_BASE`].
const HAIRPIN_SCRAPES_EACH: f32 = 3.0;
const HAIRPIN_SCRAPES_BASE: f32 = 2.0;
/// Waypoint spacing handed to the driver, yards; each is a gate.
const WAYPOINT_EVERY: f32 = 15.0;

/// The local frame: origin in WoW yards.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub origin: Vec3,
}

impl Frame {
    pub fn to_lu(&self, x: f32, y: f32, z: f32) -> Vec3 {
        let o = self.origin;
        Vec3::new(y - o.y, z - o.z, x - o.x) * LU_PER_YARD
    }

    /// WoW `(x, y, z)` yards of a kart-frame point.
    pub fn to_wow(&self, p: Vec3) -> Vec3 {
        let q = p / LU_PER_YARD;
        Vec3::new(q.z, q.x, q.y) + self.origin
    }
}

type Tri = [[f32; 3]; 3];

/// The collision soup for a course: tiled terrain within [`CORRIDOR`] of the line plus prisms.
pub fn build_world(t: &ZoneTerrain, c: &Candidate, frame: &Frame) -> Vec<Tri> {
    let g = t.grid;
    let (w, h) = (g.width, g.height);
    let res = g.yards_per_px;
    let mut mask = vec![false; w * h];
    let k = (CORRIDOR / res).ceil() as i64;
    for (n, p) in c.pts.iter().enumerate() {
        if n % 4 != 0 {
            continue;
        }
        let (pc, pr) = g.pixel_of(p.x, p.y);
        let (pc, pr) = (pc.floor() as i64, pr.floor() as i64);
        for dr in -k..=k {
            for dc in -k..=k {
                if dr * dr + dc * dc > k * k {
                    continue;
                }
                if let Some(i) = g.index(pc + dc, pr + dr) {
                    mask[i] = true;
                }
            }
        }
    }
    let vert = |c: usize, r: usize| -> Option<Vec3> {
        let z = t.height[r * w + c];
        z.is_finite().then(|| {
            let (x, y) = g.world_of(c as f32 + 0.5, r as f32 + 0.5);
            frame.to_lu(x, y, z)
        })
    };
    let mut tris = Vec::new();
    let push_up = |a: Vec3, b: Vec3, c: Vec3, tris: &mut Vec<Tri>| {
        // Ground faces wind so their normal points up.
        if (b - a).cross(c - a).y >= 0.0 {
            tris.push([a.to_array(), b.to_array(), c.to_array()]);
        } else {
            tris.push([a.to_array(), c.to_array(), b.to_array()]);
        }
    };
    for r in 0..h.saturating_sub(1) {
        for c in 0..w.saturating_sub(1) {
            if !mask[r * w + c] {
                continue;
            }
            let (Some(a), Some(b), Some(d), Some(e)) = (
                vert(c, r),
                vert(c + 1, r),
                vert(c + 1, r + 1),
                vert(c, r + 1),
            ) else {
                continue;
            };
            push_up(a, b, d, &mut tris);
            push_up(a, d, e, &mut tris);
        }
    }
    // Solid footprint pixels as blocks, walled where the neighbour is open.
    let solid = |c: i64, r: i64| g.index(c, r).is_some_and(|i| t.solid[i]);
    for r in 0..h {
        for c in 0..w {
            let i = r * w + c;
            if !mask[i] || !t.solid[i] {
                continue;
            }
            let z = t.height[i];
            if !z.is_finite() {
                continue;
            }
            let (lo, hi) = (z - 2.0, z + BLOCK_HEIGHT);
            let corner = |dc: f32, dr: f32| g.world_of(c as f32 + dc, r as f32 + dr);
            // Corners clockwise from the north-west on the image: (0,0) (1,0) (1,1) (0,1).
            let k = [
                corner(0.0, 0.0),
                corner(1.0, 0.0),
                corner(1.0, 1.0),
                corner(0.0, 1.0),
            ];
            let (ci, ri) = (c as i64, r as i64);
            let sides = [
                (0, 1, solid(ci, ri - 1)),
                (1, 2, solid(ci + 1, ri)),
                (2, 3, solid(ci, ri + 1)),
                (3, 0, solid(ci - 1, ri)),
            ];
            for (a, b, shut) in sides {
                if shut {
                    continue;
                }
                let (a0, b0) = (
                    frame.to_lu(k[a].0, k[a].1, lo),
                    frame.to_lu(k[b].0, k[b].1, lo),
                );
                let (a1, b1) = (
                    frame.to_lu(k[a].0, k[a].1, hi),
                    frame.to_lu(k[b].0, k[b].1, hi),
                );
                tris.push([a0.to_array(), b0.to_array(), b1.to_array()]);
                tris.push([a0.to_array(), b1.to_array(), a1.to_array()]);
            }
            let top = k.map(|(x, y)| frame.to_lu(x, y, hi));
            push_up(top[0], top[1], top[2], &mut tris);
            push_up(top[0], top[2], top[3], &mut tris);
        }
    }
    tris
}

/// The driven result, in yards and seconds.
#[derive(Clone, Debug, Default, Serialize)]
pub struct DriveResult {
    pub ok: bool,
    pub reject: Option<String>,
    pub finished: bool,
    pub time: f32,
    pub lap_times: Vec<f32>,
    pub best_lap: f32,
    pub avg_speed_yds: f32,
    pub max_speed_yds: f32,
    pub powerslides: u32,
    pub boosts: u32,
    pub airtime: f32,
    pub longest_air: f32,
    pub wall_contacts: u32,
    /// Of those, the ones in or just after a hairpin ([`HAIRPIN_MARGIN`]).
    pub hairpin_contacts: u32,
    /// Of the stalls, those in or just after a hairpin.
    pub hairpin_stalls: u32,
    pub slams: u32,
    pub stuck_recoveries: u32,
    pub off_track_events: u32,
    pub off_track_time: f32,
    pub max_slope_deg: f32,
    pub triangles: usize,
    /// Where the kart stalled (under 3 yd/s for a second or more), WoW yards.
    pub stalls: Vec<[f32; 3]>,
    /// The driven path, WoW yards, every quarter second (not written out).
    #[serde(skip)]
    pub trace: Vec<[f32; 3]>,
}

pub fn test_drive(t: &ZoneTerrain, c: &Candidate, laps: u32) -> (DriveResult, DriveReport) {
    let p0 = c.pts[0];
    let z0 = t.height_at(p0.x, p0.y).unwrap_or(0.0);
    let frame = Frame {
        origin: Vec3::new(p0.x, p0.y, z0),
    };
    let tris = build_world(t, c, &frame);
    let world = Collision::new(tris.iter().copied(), CELL);
    let every = (WAYPOINT_EVERY / crate::generate::STEP).round().max(1.0) as usize;
    let mut waypoints: Vec<Vec3> = c
        .pts
        .iter()
        .step_by(every)
        .map(|p| {
            let z = t.height_at(p.x, p.y).unwrap_or(z0);
            frame.to_lu(p.x, p.y, z)
        })
        .collect();
    if !c.closed {
        let p = *c.pts.last().unwrap_or(&p0);
        let z = t.height_at(p.x, p.y).unwrap_or(z0);
        waypoints.push(frame.to_lu(p.x, p.y, z));
    }
    let laps = if c.closed { laps } else { 0 };
    let route = Route::new(waypoints, laps, 20.0 * LU_PER_YARD);
    let length = crate::geom::length(&c.pts, c.closed);
    let opts = DriveOpts {
        // About three times the estimated time: a bot that slow is lost.
        time_limit: laps.max(1) as f32 * length / 14.0 + 20.0,
        off_track_yards: 15.0,
        ..Default::default()
    };
    let rep = drive(&world, &route, &opts);
    let y = LU_PER_YARD;
    let mut r = DriveResult {
        finished: rep.finished,
        time: rep.time,
        lap_times: rep.lap_times.clone(),
        best_lap: rep.lap_times.iter().copied().fold(f32::INFINITY, f32::min),
        avg_speed_yds: rep.avg_speed / y,
        max_speed_yds: rep.max_speed / y,
        powerslides: rep.powerslides,
        boosts: rep.boosts,
        airtime: rep.airtime,
        longest_air: rep.longest_air,
        wall_contacts: rep.wall_contacts,
        slams: rep.slams,
        stuck_recoveries: rep.stuck_recoveries,
        off_track_events: rep.off_track_events,
        off_track_time: rep.off_track_time,
        max_slope_deg: rep.max_slope_deg,
        triangles: tris.len(),
        ..Default::default()
    };
    if !c.closed {
        r.best_lap = rep.time;
    }
    r.trace = rep
        .trace
        .iter()
        .map(|tp| frame.to_wow(tp.position).to_array())
        .collect();
    let mut slow = 0.0;
    for tp in &rep.trace {
        if tp.t > 3.0 && tp.speed < 3.0 * y {
            slow += 0.25;
            if (slow - 1.0f32).abs() < 1e-3 {
                let q = frame.to_wow(tp.position);
                r.stalls.push([q.x, q.y, q.z]);
            }
        } else {
            slow = 0.0;
        }
    }
    let zone = crate::metrics::hairpin_zone(&c.pts, c.closed, HAIRPIN_MARGIN);
    let hairpins = crate::metrics::bends(&c.pts, c.closed)
        .0
        .iter()
        .filter(|b| b.is_hairpin())
        .count();
    for p in &rep.wall_contact_points {
        let q = frame.to_wow(*p);
        let q = glam::Vec2::new(q.x, q.y);
        let near = c
            .pts
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.distance_squared(q).total_cmp(&b.1.distance_squared(q)))
            .map(|(i, _)| i);
        if near.is_some_and(|i| zone[i]) {
            r.hairpin_contacts += 1;
        }
    }
    let in_zone = |q: [f32; 3]| {
        let q = glam::Vec2::new(q[0], q[1]);
        c.pts
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.distance_squared(q).total_cmp(&b.1.distance_squared(q)))
            .is_some_and(|(i, _)| zone[i])
    };
    r.hairpin_stalls = r.stalls.iter().filter(|q| in_zone(**q)).count() as u32;
    let laps_f = laps.max(1) as f32;
    let other = rep.wall_contacts - r.hairpin_contacts;
    let scrapes = (HAIRPIN_SCRAPES_EACH * hairpins as f32 + HAIRPIN_SCRAPES_BASE) * laps_f;
    r.reject = if rep.fell {
        Some("fell off the world".into())
    } else if rep.gave_up {
        Some("gave up stuck".into())
    } else if !rep.finished {
        Some("did not finish in time".into())
    } else if rep.stuck_recoveries as f32 > laps_f {
        Some(format!("{} stuck recoveries", rep.stuck_recoveries))
    } else if other as f32 > 4.0 * laps_f {
        Some(format!("{other} wall contacts outside hairpins"))
    } else if hairpins > 0 && r.hairpin_contacts as f32 > scrapes {
        Some(format!("{} hairpin scrapes", r.hairpin_contacts))
    } else if rep.off_track_time > 0.05 * rep.time {
        Some(format!("{:.1}s off the line", rep.off_track_time))
    } else {
        None
    };
    if let Some(why) = &mut r.reject {
        if !r.stalls.is_empty() {
            why.push_str(&format!(
                " ({} of {} stalls at hairpins)",
                r.hairpin_stalls,
                r.stalls.len()
            ));
        }
    }
    r.ok = r.reject.is_none();
    (r, rep)
}
