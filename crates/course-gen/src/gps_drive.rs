//! The GPS roads (`roads/roads.json`) driven by the CTR-physics bot: every road between two
//! junctions cut into stretches of at most [`PIECE`] yards, each stretch driven as a sprint over
//! the world's real collision round it, as the client builds it ([`crate::soup`]: the terrain,
//! every doodad's collision hull, every WMO's faces). A stretch the bot cannot drive (it gives up
//! stuck, falls, runs out of time or keeps leaving the line) is a way the GPS shows that a kart
//! cannot take.

use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::Result;
use glam::{Vec2, Vec3};
use kart_driver::{drive, Collision, DriveOpts, Route, CELL, LU_PER_YARD};
use kart_gps::{RoadFile, RoadMap};
use serde::Serialize;

use crate::gps::{continent_zones, Zone};
use crate::open_install;
use crate::soup::{self, bevy_to_wow, wow_to_bevy, Region};

/// The longest stretch driven at once, yards.
const PIECE: f32 = 400.0;
/// The shortest worth driving, yards: a junction's stub.
const SHORTEST: f32 = 30.0;
/// The world gathered round a stretch's box, yards (flat), and up and down from its heights.
const AROUND: f32 = 60.0;
const UP: f32 = 80.0;
/// A waypoint handed to the bot every this many yards; each is a gate this wide either side.
const WAYPOINT_EVERY: f32 = 15.0;
const GATE: f32 = 20.0;
/// The bot's ground search above a road point, yards: the GPS's heights are a cell's mean.
const GROUND_ABOVE: f32 = 12.0;

/// One stretch driven.
#[derive(Clone, Debug, Serialize)]
pub struct Stretch {
    pub map: u32,
    /// The smallest zone whose world map holds it, for the report.
    pub zone: String,
    pub length: f32,
    /// Its first and last points, WoW x, y, z.
    pub from: [f32; 3],
    pub to: [f32; 3],
    pub ok: bool,
    pub reject: Option<String>,
    /// Where the bot stalled (under 3 yd/s for a second), and where it gave up, WoW yards.
    pub stalls: Vec<[f32; 3]>,
    pub gave_up_at: Option<[f32; 3]>,
    pub max_slope_deg: f32,
    pub time: f32,
    pub stuck_recoveries: u32,
    pub points: Vec<[f32; 3]>,
}

/// The roads between junctions (points with other than two links), each as its points in order.
fn chains(points: &[i32], links: &[u32]) -> Vec<Vec<[f32; 3]>> {
    let n = points.len() / 3;
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for l in links.as_chunks::<2>().0 {
        let (a, b) = (l[0] as usize, l[1] as usize);
        if a != b && a < n && b < n {
            adj[a].push(b);
            adj[b].push(a);
        }
    }
    let at = |i: usize| {
        [
            points[3 * i] as f32,
            points[3 * i + 1] as f32,
            points[3 * i + 2] as f32,
        ]
    };
    let mut used: HashMap<(usize, usize), ()> = HashMap::new();
    let mut seen = |a: usize, b: usize| used.insert((a.min(b), a.max(b)), ()).is_some();
    let mut out = Vec::new();
    let walk = |start: usize, first: usize, seen: &mut dyn FnMut(usize, usize) -> bool| {
        if seen(start, first) {
            return None;
        }
        let mut chain = vec![at(start), at(first)];
        let (mut prev, mut cur) = (start, first);
        while adj[cur].len() == 2 {
            let next = if adj[cur][0] == prev {
                adj[cur][1]
            } else {
                adj[cur][0]
            };
            if seen(cur, next) {
                break;
            }
            chain.push(at(next));
            (prev, cur) = (cur, next);
        }
        Some(chain)
    };
    // From every junction and end first, then whatever loops have none.
    let ends = (0..n).filter(|&i| adj[i].len() != 2);
    for s in ends.chain(0..n) {
        for &f in &adj[s] {
            if let Some(c) = walk(s, f, &mut seen) {
                out.push(c);
            }
        }
    }
    out
}

/// A road cut into stretches of at most [`PIECE`] yards, sharing their ends.
fn pieces(chain: &[[f32; 3]]) -> Vec<Vec<[f32; 3]>> {
    let mut out = Vec::new();
    let mut cur = vec![chain[0]];
    let mut len = 0.0;
    for w in chain.windows(2) {
        len += (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
        cur.push(w[1]);
        if len >= PIECE {
            out.push(std::mem::replace(&mut cur, vec![w[1]]));
            len = 0.0;
        }
    }
    if cur.len() > 1 {
        out.push(cur);
    }
    out
}

fn length(p: &[[f32; 3]]) -> f32 {
    p.windows(2)
        .map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]))
        .sum()
}

/// The smallest zone whose world-map rect holds the stretch's middle.
fn zone_of(zones: &[Zone], map: u32, p: &[[f32; 3]]) -> String {
    let m = p[p.len() / 2];
    zones
        .iter()
        .filter(|z| z.map == map)
        .filter(|z| m[0] >= z.rect.0 && m[0] <= z.rect.1 && m[1] >= z.rect.2 && m[1] <= z.rect.3)
        .min_by(|a, b| {
            let area = |z: &Zone| (z.rect.1 - z.rect.0) * (z.rect.3 - z.rect.2);
            area(a).total_cmp(&area(b))
        })
        .map_or_else(String::new, |z| z.name.clone())
}

/// Drive every stretch of `roads`' maps (`only` one map, if given), `jobs` at a time.
pub fn run(roads: &RoadFile, only: Option<u32>, jobs: usize) -> Result<Vec<Stretch>> {
    let mut out = Vec::new();
    for m in &roads.maps {
        if only.is_none_or(|o| o == m.map) {
            out.extend(drive_map(m, jobs));
        }
    }
    Ok(out)
}

/// Every stretch of one map's roads driven, `jobs` at a time.
pub fn drive_map(m: &RoadMap, jobs: usize) -> Vec<Stretch> {
    let zones = open_install()
        .and_then(|mut c| continent_zones(&mut c))
        .unwrap_or_default();
    let mut work: Vec<(u32, Vec<[f32; 3]>)> = Vec::new();
    for chain in chains(&m.points, &m.links) {
        work.extend(
            pieces(&chain)
                .into_iter()
                .filter(|p| length(p) >= SHORTEST)
                .map(|p| (m.map, p)),
        );
    }
    let out = Mutex::new(Vec::new());
    let next = std::sync::atomic::AtomicUsize::new(0);
    let total = work.len();
    std::thread::scope(|s| {
        for _ in 0..jobs.max(1) {
            s.spawn(|| {
                let Ok(mut chain) = open_install() else {
                    return;
                };
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some((map, p)) = work.get(i) else {
                        break;
                    };
                    let mut s = match drive_stretch(&mut chain, *map, p) {
                        Ok(s) => s,
                        Err(e) => {
                            eprintln!("stretch at {:?}: {e:#}", p[0]);
                            continue;
                        }
                    };
                    s.zone = zone_of(&zones, *map, p);
                    if i.is_multiple_of(50) {
                        eprintln!("{i} of {total} stretches");
                    }
                    out.lock().unwrap().push(s);
                }
            });
        }
    });
    let mut out = out.into_inner().unwrap();
    out.sort_by(|a, b| (a.map, &a.zone).cmp(&(b.map, &b.zone)));
    out
}

impl Stretch {
    /// The road's point nearest where the bot could not get on (where it gave up, else its
    /// first stall), WoW yards; `None` where it names no spot.
    pub fn blocked_at(&self) -> Option<[f32; 3]> {
        if self.ok {
            return None;
        }
        let at = self.gave_up_at.or_else(|| self.stalls.first().copied())?;
        self.points
            .iter()
            .min_by(|a, b| {
                let d = |r: &[f32; 3]| (r[0] - at[0]).hypot(r[1] - at[1]);
                d(a).total_cmp(&d(b))
            })
            .copied()
    }
}

/// One stretch driven over the world's faces round it.
fn drive_stretch(chain: &mut benilla_formats::Chain, map: u32, p: &[[f32; 3]]) -> Result<Stretch> {
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for q in p {
        let b = wow_to_bevy(*q);
        lo = lo.min(b);
        hi = hi.max(b);
    }
    let region = Region {
        min: lo - Vec3::new(AROUND, UP, AROUND),
        max: hi + Vec3::new(AROUND, UP, AROUND),
    };
    let faces = soup::gather_world(chain, map, region)?;
    let origin = wow_to_bevy(p[0]);
    let world = Collision::new(soup::to_lu(&faces.solid, origin), CELL);
    let lu = |v: Vec3| (v - origin) * LU_PER_YARD;
    let flat: Vec<Vec2> = p.iter().map(|q| Vec2::new(q[0], q[1])).collect();
    let even = crate::geom::resample(&flat, false, WAYPOINT_EVERY);
    // Each waypoint on the ground under the road's height: the road's own z is a cell's mean.
    let mut waypoints: Vec<Vec3> = even
        .iter()
        .map(|q| {
            let z = p
                .iter()
                .min_by(|a, b| {
                    let d = |r: &[f32; 3]| (r[0] - q.x).hypot(r[1] - q.y);
                    d(a).total_cmp(&d(b))
                })
                .map_or(0.0, |r| r[2]);
            let at = lu(wow_to_bevy([q.x, q.y, z + GROUND_ABOVE]));
            world
                .ground(at, (GROUND_ABOVE + 30.0) * LU_PER_YARD)
                .map_or(lu(wow_to_bevy([q.x, q.y, z])), |h| h.point)
        })
        .collect();
    let last = p[p.len() - 1];
    if let Some(&end) = waypoints.last() {
        let tip = lu(wow_to_bevy(last));
        if (end - tip).length() > 3.0 * LU_PER_YARD {
            waypoints.push(tip);
        }
    }
    let len = length(p);
    let route = Route::new(waypoints, 0, GATE * LU_PER_YARD);
    let opts = DriveOpts {
        time_limit: len / 14.0 + 20.0,
        off_track_yards: 15.0,
        ..Default::default()
    };
    let rep = drive(&world, &route, &opts);
    let wow = |v: Vec3| bevy_to_wow(v / LU_PER_YARD + origin);
    let mut stalls = Vec::new();
    let mut slow = 0.0;
    for tp in &rep.trace {
        if tp.t > 3.0 && tp.speed < 3.0 * LU_PER_YARD {
            slow += 0.25;
            if (slow - 1.0f32).abs() < 1e-3 {
                stalls.push(wow(tp.position));
            }
        } else {
            slow = 0.0;
        }
    }
    let reject = if rep.fell {
        Some("fell off the world".to_string())
    } else if rep.gave_up {
        Some("gave up stuck".into())
    } else if !rep.finished {
        Some("did not finish in time".into())
    } else if rep.stuck_recoveries > 1 {
        Some(format!("{} stuck recoveries", rep.stuck_recoveries))
    } else if rep.off_track_time > 0.1 * rep.time {
        Some("off the line".into())
    } else {
        None
    };
    Ok(Stretch {
        map,
        zone: String::new(),
        length: len,
        from: p[0],
        to: last,
        ok: reject.is_none(),
        gave_up_at: (rep.gave_up || !rep.finished)
            .then(|| rep.trace.last().map(|tp| wow(tp.position)))
            .flatten(),
        reject,
        stalls,
        max_slope_deg: rep.max_slope_deg,
        time: rep.time,
        stuck_recoveries: rep.stuck_recoveries,
        points: p.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_road_splits_at_its_junctions_and_into_pieces() {
        // A T: 0-1-2-3-4 east, 200 yd apart, and 1-5 north; 1 is the junction.
        let points = [
            0, 0, 0, 0, 200, 0, 0, 400, 0, 0, 600, 0, 0, 800, 0, 300, 200, 0,
        ];
        let links = [0, 1, 1, 2, 2, 3, 3, 4, 1, 5];
        let mut c = chains(&points, &links);
        c.sort_by_key(|c| c.len());
        assert_eq!(c.len(), 3, "0-1, 1-5 and 1-2-3-4");
        assert_eq!(c[2].len(), 4);
        let p = pieces(&c[2]);
        assert_eq!(p.len(), 2, "600 yd in 400 yd pieces");
        assert_eq!(p[0].last(), p[1].first(), "pieces share their ends");
    }
}
