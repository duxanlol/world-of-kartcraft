//! The course to drive: waypoints and the checkpoint gates on them, and the evenly resampled
//! path the driver steers along.

use glam::{Vec2, Vec3};

/// A checkpoint gate: passed on crossing the line through `position` square to the course,
/// within `half_width` of it, or on coming near its centre (the game's checkpoints, both in XZ).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gate {
    pub position: Vec3,
    /// The course's direction through the gate, XZ (x, z), unit.
    pub heading: Vec2,
    pub half_width: f32,
}

impl Gate {
    /// Whether moving from `from` to `to` passes the gate; `radius` is the near-the-centre pass.
    pub fn passes(&self, from: Vec3, to: Vec3, radius: f32) -> bool {
        let p = xz(self.position);
        let (f, t) = (xz(from), xz(to));
        if t.distance(p) <= radius {
            return true;
        }
        let h = self.heading;
        let along = |q: Vec2| (q - p).dot(h);
        let across = |q: Vec2| (q - p).perp_dot(h).abs();
        along(f) < 0.0 && along(t) >= 0.0 && across(t).min(across(f)) <= self.half_width
    }
}

/// A course: a polyline in lu, a closed loop driven `laps` times or (0 laps) an open sprint from
/// the first waypoint to the last, and its gates in order. The first gate is the start, and a
/// loop's finish line; a sprint finishes at its last gate.
#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    pub waypoints: Vec<Vec3>,
    pub laps: u32,
    pub gates: Vec<Gate>,
}

impl Route {
    /// A gate at every waypoint, square to the course there: halfway between the way in and the
    /// way out, as the game stands its checkpoints across a bend.
    pub fn new(waypoints: Vec<Vec3>, laps: u32, gate_half_width: f32) -> Self {
        let n = waypoints.len();
        let closed = laps > 0;
        let unit = |a: Vec3, b: Vec3| (xz(b) - xz(a)).try_normalize();
        let gates = (0..n)
            .map(|i| {
                let p = waypoints[i];
                let before = (closed || i > 0).then(|| waypoints[(i + n - 1) % n]);
                let after = (closed || i + 1 < n).then(|| waypoints[(i + 1) % n]);
                let into = before.and_then(|b| unit(b, p));
                let out = after.and_then(|a| unit(p, a));
                let heading = match (into, out) {
                    (Some(a), Some(b)) => (a + b).try_normalize().unwrap_or(b),
                    (Some(a), None) | (None, Some(a)) => a,
                    (None, None) => Vec2::Y,
                };
                Gate {
                    position: p,
                    heading,
                    half_width: gate_half_width,
                }
            })
            .collect();
        Self {
            waypoints,
            laps,
            gates,
        }
    }

    pub fn is_loop(&self) -> bool {
        self.laps > 0
    }
}

pub(crate) fn xz(p: Vec3) -> Vec2 {
    Vec2::new(p.x, p.z)
}

/// The signed angle from `a` to `b` in XZ, radians, + to the left (CTR's: a left turn raises the
/// heading, forward `(sin h, cos h)`, left `(cos h, −sin h)`).
pub(crate) fn left_angle(a: Vec2, b: Vec2) -> f32 {
    (a.y * b.x - a.x * b.y).atan2(a.dot(b))
}

/// Spacing of the resampled path, lu (the step is the nearest that divides the length evenly).
const SPACING: f32 = 40.0;
/// The chord each side the curvature is measured over, lu, and the span each side it is then
/// averaged over: a coarse polyline turns all at its corners, and the kart drives the bend between.
const CURVE_SPAN: f32 = 240.0;

/// The route resampled evenly, with its curvature.
pub(crate) struct Path {
    pub pts: Vec<Vec3>,
    pub step: f32,
    pub closed: bool,
    pub length: f32,
    /// Signed curvature at each point, 1/lu, + to the left.
    pub curv: Vec<f32>,
}

impl Path {
    pub fn new(waypoints: &[Vec3], closed: bool) -> Self {
        let mut legs: Vec<(Vec3, Vec3)> = waypoints.windows(2).map(|w| (w[0], w[1])).collect();
        if closed && waypoints.len() > 2 {
            legs.push((waypoints[waypoints.len() - 1], waypoints[0]));
        }
        legs.retain(|(a, b)| a.distance(*b) > 1e-3);
        let length: f32 = legs.iter().map(|(a, b)| a.distance(*b)).sum();
        let segs = ((length / SPACING).round() as usize).max(1);
        let step = length / segs as f32;
        let count = if closed { segs } else { segs + 1 };
        let mut pts = Vec::with_capacity(count);
        let (mut leg, mut leg_start) = (0, 0.0);
        for i in 0..count {
            let s = (i as f32 * step).min(length);
            while leg + 1 < legs.len() && s > leg_start + legs[leg].0.distance(legs[leg].1) {
                leg_start += legs[leg].0.distance(legs[leg].1);
                leg += 1;
            }
            let Some(&(a, b)) = legs.get(leg) else {
                pts.push(waypoints.first().copied().unwrap_or_default());
                continue;
            };
            let t = ((s - leg_start) / a.distance(b)).clamp(0.0, 1.0);
            pts.push(a.lerp(b, t));
        }
        let mut path = Self {
            pts,
            step,
            closed,
            length,
            curv: Vec::new(),
        };
        let k = ((CURVE_SPAN / step).round() as isize).max(1);
        let raw: Vec<f32> = (0..path.pts.len() as isize)
            .map(|i| {
                let p = xz(path.pts[path.idx(i)]);
                let a = p - xz(path.pts[path.idx(i - k)]);
                let b = xz(path.pts[path.idx(i + k)]) - p;
                if a.length() < 1e-3 || b.length() < 1e-3 {
                    0.0
                } else {
                    left_angle(a, b) / a.length().midpoint(b.length())
                }
            })
            .collect();
        path.curv = (0..raw.len() as isize)
            .map(|i| {
                let (sum, n) = (i - k..=i + k)
                    .filter(|&j| path.closed || (0..raw.len() as isize).contains(&j))
                    .fold((0.0, 0.0), |(sum, n), j| (sum + raw[path.idx(j)], n + 1.0));
                sum / n
            })
            .collect();
        path
    }

    pub fn len(&self) -> usize {
        self.pts.len()
    }

    /// Point index `i`, wrapped round a loop or clamped to a sprint's ends.
    pub fn idx(&self, i: isize) -> usize {
        let n = self.len() as isize;
        if self.closed {
            i.rem_euclid(n) as usize
        } else {
            i.clamp(0, n - 1) as usize
        }
    }

    fn segments(&self) -> usize {
        if self.closed {
            self.len()
        } else {
            self.len() - 1
        }
    }

    /// The point `s` lu along; a sprint runs on straight past its end.
    pub fn at(&self, s: f32) -> Vec3 {
        let n = self.len();
        if n < 2 {
            return self.pts.first().copied().unwrap_or_default();
        }
        if self.closed {
            let s = s.rem_euclid(self.length);
            let i = ((s / self.step) as usize).min(n - 1);
            let t = s / self.step - i as f32;
            return self.pts[i].lerp(self.pts[(i + 1) % n], t);
        }
        if s >= self.length {
            let (a, b) = (self.pts[n - 2], self.pts[n - 1]);
            return b + (b - a).normalize_or_zero() * (s - self.length);
        }
        let s = s.max(0.0);
        let i = ((s / self.step) as usize).min(n - 2);
        let t = s / self.step - i as f32;
        self.pts[i].lerp(self.pts[i + 1], t)
    }

    /// The nearest point of the path to `p` among the segments from `back` behind segment `hint`
    /// to `ahead` past it: (segment, lu along, distance).
    pub fn nearest(&self, p: Vec3, hint: usize, back: usize, ahead: usize) -> (usize, f32, f32) {
        let segs = self.segments();
        let mut best = (hint, hint as f32 * self.step, f32::INFINITY);
        if segs == 0 {
            return best;
        }
        let range: Vec<usize> = if self.closed {
            let span = (back + ahead + 1).min(segs);
            (0..span)
                .map(|k| (hint + segs * 2 + k - back.min(segs)) % segs)
                .collect()
        } else {
            (hint.saturating_sub(back)..=(hint + ahead).min(segs - 1)).collect()
        };
        for i in range {
            let a = self.pts[i];
            let b = self.pts[(i + 1) % self.len()];
            let ab = b - a;
            let t = ((p - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
            let d = p.distance(a + ab * t);
            if d < best.2 {
                best = (i, (i as f32 + t) * self.step, d);
            }
        }
        best
    }

    /// The signed heading change over the `n` steps from point `i`, radians, + to the left.
    pub fn turn(&self, i: usize, n: usize) -> f32 {
        (0..n)
            .map(|j| i + j)
            .take_while(|&j| self.closed || j < self.len())
            .map(|j| self.curv[self.idx(j as isize)] * self.step)
            .sum()
    }

    /// The largest curvature over the `n` steps from point `i`, 1/lu.
    pub fn sharpest(&self, i: usize, n: usize) -> f32 {
        (0..n)
            .map(|j| i + j)
            .take_while(|&j| self.closed || j < self.len())
            .map(|j| self.curv[self.idx(j as isize)].abs())
            .fold(0.0, f32::max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn circle(r: f32, n: usize) -> Vec<Vec3> {
        (0..n)
            .map(|i| {
                let a = i as f32 / n as f32 * std::f32::consts::TAU;
                Vec3::new(r * a.cos(), 0.0, r * a.sin())
            })
            .collect()
    }

    #[test]
    fn a_circle_resamples_evenly_with_its_curvature() {
        let path = Path::new(&circle(1000.0, 64), true);
        assert!((path.length - std::f32::consts::TAU * 1000.0).abs() < 20.0);
        // Counter-clockwise seen from above +Y in x, z is a right turn in CTR's frame.
        for &k in &path.curv {
            assert!((k + 1.0 / 1000.0).abs() < 2e-4, "{k}");
        }
        let total = path.turn(0, path.len());
        assert!((total + std::f32::consts::TAU).abs() < 0.05, "{total}");
    }

    #[test]
    fn the_nearest_point_wraps_round_a_loop() {
        let path = Path::new(&circle(1000.0, 64), true);
        let (_, s, d) = path.nearest(Vec3::new(1010.0, 0.0, -5.0), 2, 10, 10);
        assert!(d < 15.0);
        assert!(s > path.length - 20.0, "{s}");
    }

    #[test]
    fn a_gate_counts_when_crossed_not_when_beside_it() {
        let route = Route::new(
            vec![
                Vec3::ZERO,
                Vec3::new(0.0, 0.0, 1000.0),
                Vec3::new(0.0, 0.0, 2000.0),
            ],
            0,
            100.0,
        );
        let g = route.gates[1];
        assert!(g.passes(
            Vec3::new(20.0, 0.0, 990.0),
            Vec3::new(20.0, 0.0, 1010.0),
            0.0
        ));
        assert!(!g.passes(
            Vec3::new(300.0, 0.0, 990.0),
            Vec3::new(300.0, 0.0, 1010.0),
            0.0
        ));
        assert!(!g.passes(
            Vec3::new(20.0, 0.0, 1010.0),
            Vec3::new(20.0, 0.0, 990.0),
            0.0
        ));
    }
}
