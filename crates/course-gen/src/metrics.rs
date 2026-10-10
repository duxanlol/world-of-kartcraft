//! What a course line offers before anyone drives it: length, surface, bends by radius, hills,
//! crests that throw a kart in the air at speed, room either side, water, subzones.

use glam::Vec2;
use serde::Serialize;

use crate::cost::CostField;
use crate::generate::{Candidate, STEP};
use crate::geom;
use crate::terrain::ZoneTerrain;

/// lu per yard (kart scale 1), as `kart_driver::LU_PER_YARD`.
const LU_PER_YARD: f32 = 0.9144 / 0.025;
/// CTR gravity: 900 speed units a frame, 3164 lu/s², in yd/s².
pub const GRAVITY: f32 = 900.0 * 30.0 / 256.0 * 30.0 / LU_PER_YARD;

/// The Balanced class's top speed (yd/s) and the yaw rate (rad/s) the driver plans corners at.
pub fn kart_speeds() -> (f32, f32) {
    let s = kart_driver::Class::Balanced.stats();
    let top = s.speed as f32 * 30.0 / 256.0 / LU_PER_YARD;
    let yaw = s.turn_rate as f32 * std::f32::consts::TAU / 4096.0 * 30.0 * 0.85;
    (top, yaw)
}

#[derive(Clone, Debug, Serialize)]
pub struct Bend {
    /// Index of the bend's first point in the polyline, and of its apex.
    pub start: usize,
    pub at: usize,
    /// Signed total turn, degrees, + left.
    pub angle_deg: f32,
    pub min_radius: f32,
    pub length: f32,
}

impl Bend {
    /// A hairpin: 90° or more at under 20 yd radius.
    pub fn is_hairpin(&self) -> bool {
        self.min_radius < 20.0 && self.angle_deg.abs() >= 90.0
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Jump {
    pub at: usize,
    /// Yards along the course.
    pub s: f32,
    /// Estimated airtime at the speed the kart would carry there, seconds.
    pub airtime: f32,
    /// Ground distance flown, yards.
    pub distance: f32,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Metrics {
    pub length_yd: f32,
    pub length_m: f32,
    pub road_frac: f32,
    pub bends: Vec<Bend>,
    /// Bends of 30° or more by tightest radius: under 20 yd, 20–40, 40–80, 80–160, wider.
    pub bend_hist: [u32; 5],
    /// Bends a kart would powerslide: 60° or more at 16–120 yd radius.
    pub slide_bends: u32,
    pub hairpins: u32,
    pub longest_straight: f32,
    pub elev_min: f32,
    pub elev_max: f32,
    pub elev_range: f32,
    /// Height climbed over a lap (or the sprint), yards.
    pub climb: f32,
    pub max_grade_deg: f32,
    pub jumps: Vec<Jump>,
    pub min_clearance: f32,
    pub mean_clearance: f32,
    /// Share of the line with under 6 yd of room.
    pub narrow_frac: f32,
    /// Share of the line within 15 yd of water.
    pub water_frac: f32,
    pub subzones: Vec<String>,
    /// Estimated lap (or sprint) time from the curvature-limited speed, seconds.
    pub est_time: f32,
    pub static_score: f32,
}

/// The heights along a course, lightly smoothed.
pub fn heights(t: &ZoneTerrain, pts: &[Vec2], closed: bool) -> Vec<f32> {
    let raw: Vec<f32> = pts
        .iter()
        .map(|p| t.height_at(p.x, p.y).unwrap_or(0.0))
        .collect();
    geom::smooth(&raw, closed, 1)
}

/// The speed (yd/s) the kart could carry at each point: curvature-limited, then braking-limited.
pub fn speed_profile(curv: &[f32], closed: bool) -> Vec<f32> {
    let (top, yaw) = kart_speeds();
    let mut v: Vec<f32> = curv
        .iter()
        .map(|k| (yaw / k.abs().max(1e-6)).min(top))
        .collect();
    // ~650 lu/s² of braking, as the driver plans.
    let reach = 2.0 * 650.0 / LU_PER_YARD * STEP;
    let n = v.len();
    for _ in 0..if closed { 2 } else { 1 } {
        for i in (0..n).rev() {
            let j = if i + 1 < n {
                i + 1
            } else if closed {
                0
            } else {
                continue;
            };
            v[i] = v[i].min((v[j] * v[j] + reach).sqrt());
        }
    }
    v
}

/// The bends of a line, runs of one sign tighter than 150 yd turning 30° or more, and its
/// longest straight (yards).
pub fn bends(pts: &[Vec2], closed: bool) -> (Vec<Bend>, f32) {
    let n = pts.len();
    let mut out = Vec::new();
    let curv = geom::smooth(&geom::curvature(pts, closed, 4), closed, 2);
    let mut i = 0;
    let (mut straight, mut longest) = (0.0f32, 0.0f32);
    while i < n {
        let k = curv[i];
        if k.abs() < 1.0 / 250.0 {
            straight += STEP;
            longest = longest.max(straight);
        } else {
            straight = 0.0;
        }
        if k.abs() < 1.0 / 150.0 {
            i += 1;
            continue;
        }
        let sign = k.signum();
        let (mut j, mut angle, mut peak, mut at) = (i, 0.0f32, 0.0f32, i);
        while j < n && curv[j].signum() == sign && curv[j].abs() >= 1.0 / 150.0 {
            angle += curv[j] * STEP;
            if curv[j].abs() > peak {
                peak = curv[j].abs();
                at = j;
            }
            j += 1;
        }
        let deg = angle.to_degrees();
        if deg.abs() >= 30.0 {
            out.push(Bend {
                start: i,
                at,
                angle_deg: deg,
                min_radius: 1.0 / peak,
                length: (j - i) as f32 * STEP,
            });
        }
        i = j.max(i + 1);
    }
    (out, longest)
}

/// Each line point inside a hairpin, or within `margin` yards either side of one.
pub fn hairpin_zone(pts: &[Vec2], closed: bool, margin: f32) -> Vec<bool> {
    let n = pts.len();
    let mut zone = vec![false; n];
    let pad = (margin / STEP).ceil() as isize;
    for b in bends(pts, closed).0.iter().filter(|b| b.is_hairpin()) {
        let end = b.start as isize + (b.length / STEP).round() as isize;
        for i in b.start as isize - pad..=end + pad {
            if closed || (0..n as isize).contains(&i) {
                zone[geom::wrap(i, n, closed)] = true;
            }
        }
    }
    zone
}

pub fn measure(c: &Candidate, t: &ZoneTerrain, field: &CostField) -> Metrics {
    let pts = &c.pts;
    let n = pts.len();
    let closed = c.closed;
    let mut m = Metrics {
        length_yd: geom::length(pts, closed),
        ..Default::default()
    };
    m.length_m = m.length_yd * 0.9144;
    m.road_frac = pts
        .iter()
        .filter(|p| t.road_at(p.x, p.y).unwrap_or(0.0) > 0.4)
        .count() as f32
        / n as f32;

    let curv = geom::smooth(&geom::curvature(pts, closed, 4), closed, 2);
    let (bends, longest) = bends(pts, closed);
    for b in &bends {
        let r = b.min_radius;
        let deg = b.angle_deg;
        let k = match r {
            r if r < 20.0 => 0,
            r if r < 40.0 => 1,
            r if r < 80.0 => 2,
            r if r < 160.0 => 3,
            _ => 4,
        };
        m.bend_hist[k] += 1;
        if deg.abs() >= 60.0 && (16.0..=120.0).contains(&r) {
            m.slide_bends += 1;
        }
        if b.is_hairpin() {
            m.hairpins += 1;
        }
    }
    m.bends = bends;
    m.longest_straight = longest;

    let z = heights(t, pts, closed);
    m.elev_min = z.iter().copied().fold(f32::INFINITY, f32::min);
    m.elev_max = z.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    m.elev_range = m.elev_max - m.elev_min;
    let segs = if closed { n } else { n - 1 };
    for i in 0..segs {
        let dz = z[(i + 1) % n] - z[i];
        m.climb += dz.max(0.0);
        m.max_grade_deg = m.max_grade_deg.max((dz.abs() / STEP).atan().to_degrees());
    }

    let speed = speed_profile(&curv, closed);
    m.est_time = speed.iter().map(|v| STEP / v.max(1.0)).sum();
    m.jumps = jumps(&z, &speed, closed);

    let clr: Vec<f32> = pts.iter().map(|p| field.clearance_at(p.x, p.y)).collect();
    m.min_clearance = clr.iter().copied().fold(f32::INFINITY, f32::min);
    m.mean_clearance = clr.iter().sum::<f32>() / n as f32;
    m.narrow_frac = clr.iter().filter(|&&c| c < 6.0).count() as f32 / n as f32;
    m.water_frac = pts
        .iter()
        .filter(|p| field.water_dist_at(p.x, p.y) < 15.0)
        .count() as f32
        / n as f32;

    // Subzones with at least 30 yd of the course, most covered first.
    let mut seen: Vec<(u32, usize)> = Vec::new();
    for p in pts {
        let a = t.area_at(p.x, p.y).unwrap_or(0);
        match seen.iter_mut().find(|(id, _)| *id == a) {
            Some(e) => e.1 += 1,
            None => seen.push((a, 1)),
        }
    }
    seen.sort_by_key(|a| std::cmp::Reverse(a.1));
    m.subzones = seen
        .into_iter()
        .filter(|&(a, k)| a != 0 && k as f32 * STEP >= 30.0)
        .filter_map(|(a, _)| t.area_names.get(&a).cloned())
        .collect();
    m.static_score = static_score(&m, c.closed);
    m
}

/// Airtime (seconds) a crest must give to count as a jump.
pub const MIN_AIR: f32 = 0.2;

/// Crests that would throw the kart: fly a point mass from each point at the profile speed and
/// its approach slope until it meets the ground again.
pub fn jumps(z: &[f32], speed: &[f32], closed: bool) -> Vec<Jump> {
    let n = z.len();
    let mut out: Vec<Jump> = Vec::new();
    for i in 0..n {
        let prev = z[geom::wrap(i as isize - 2, n, closed)];
        let theta = ((z[i] - prev) / (2.0 * STEP)).atan();
        let v = speed[i];
        let (vx, vz) = (v * theta.cos(), v * theta.sin());
        let mut t_air = 0.0;
        let mut flown = 0.0;
        for j in 1..(n.min(200)) {
            if !closed && i + j >= n {
                break;
            }
            let s = j as f32 * STEP;
            let t = s / vx.max(1.0);
            let h = z[i] + vz * t - 0.5 * GRAVITY * t * t;
            if h <= z[(i + j) % n] + 0.05 {
                if j > 1 {
                    t_air = t;
                    flown = s;
                }
                break;
            }
        }
        if t_air < MIN_AIR {
            continue;
        }
        let s = i as f32 * STEP;
        match out.last_mut() {
            Some(last) if s - last.s < 30.0 => {
                if t_air > last.airtime {
                    *last = Jump {
                        at: i,
                        s,
                        airtime: t_air,
                        distance: flown,
                    };
                }
            }
            _ => out.push(Jump {
                at: i,
                s,
                airtime: t_air,
                distance: flown,
            }),
        }
    }
    out
}

/// The fun a course line promises, before driving it.
pub fn static_score(m: &Metrics, closed: bool) -> f32 {
    let mut s = 0.0;
    s += 1.2 * (m.slide_bends.min(10) as f32);
    s += 0.4 * ((m.bend_hist[1] + m.bend_hist[2] + m.bend_hist[3]).min(14) as f32);
    // A hairpin or two is a feature; more, and other bends under 20 yd, cost.
    s += 0.5 * m.hairpins.min(2) as f32;
    s -= 1.5 * m.hairpins.saturating_sub(2) as f32;
    s -= 0.5 * m.bend_hist[0].saturating_sub(m.hairpins) as f32;
    s += 1.2 * (m.jumps.len().min(5) as f32);
    s += 3.0
        * m.jumps
            .iter()
            .map(|j| j.airtime)
            .fold(0.0, f32::max)
            .min(0.8);
    s += 0.12 * m.elev_range.min(45.0);
    // A mix of road and open ground, best around a third paved.
    s += 3.0 * (1.0 - (m.road_frac - 0.35).abs() / 0.65).max(0.0);
    s += 0.5 * (m.subzones.len().min(4) as f32);
    s -= 10.0 * m.water_frac;
    s -= 6.0 * m.narrow_frac;
    s -= 0.008 * (m.longest_straight - 350.0).max(0.0);
    let (lo, hi) = if closed { (50.0, 110.0) } else { (45.0, 120.0) };
    if m.est_time < lo {
        s -= 0.15 * (lo - m.est_time);
    } else if m.est_time > hi {
        s -= 0.15 * (m.est_time - hi);
    }
    // Bends a minute, so a long course does not win on length alone.
    let per_min = (m.slide_bends + m.bend_hist[1] + m.bend_hist[2]) as f32 / (m.est_time / 60.0);
    s += 0.6 * per_min.min(12.0);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_u_turn_is_a_hairpin_zone() {
        // 150 yd out, a 10 yd radius half circle, 150 yd back.
        let mut raw = vec![Vec2::new(0.0, 0.0), Vec2::new(150.0, 0.0)];
        for k in 1..=18 {
            let a = std::f32::consts::PI * k as f32 / 18.0 - std::f32::consts::FRAC_PI_2;
            raw.push(Vec2::new(150.0 + 10.0 * a.cos(), 10.0 + 10.0 * a.sin()));
        }
        raw.push(Vec2::new(0.0, 20.0));
        let pts = geom::resample(&raw, false, STEP);
        let (b, _) = bends(&pts, false);
        assert_eq!(b.iter().filter(|b| b.is_hairpin()).count(), 1, "{b:?}");
        let zone = hairpin_zone(&pts, false, 20.0);
        let apex = pts
            .iter()
            .position(|p| p.x > 158.0)
            .expect("the turn's far side");
        assert!(zone[apex]);
        assert!(!zone[0] && !zone[pts.len() - 1]);
    }
}
