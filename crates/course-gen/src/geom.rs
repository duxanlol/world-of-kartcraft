//! Polyline geometry in WoW world yards `(x, y)`: resampling, smoothing, curvature, crossings.
//! A closed polyline lists each point once; its last point joins back to its first.

use glam::Vec2;

/// Length of a polyline, closing it if `closed`.
/// How much of its convex hull a closed line's area fills: 1 for a convex blob, lower the more
/// it bends inward.
pub fn convexity(pts: &[Vec2]) -> f32 {
    area(pts) / area(&hull(pts)).max(1.0)
}

/// A polygon's area (shoelace).
pub fn area(p: &[Vec2]) -> f32 {
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
pub fn hull(p: &[Vec2]) -> Vec<Vec2> {
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

pub fn length(pts: &[Vec2], closed: bool) -> f32 {
    let open: f32 = pts.windows(2).map(|w| w[0].distance(w[1])).sum();
    match (closed, pts.first(), pts.last()) {
        (true, Some(a), Some(b)) => open + a.distance(*b),
        _ => open,
    }
}

/// The polyline resampled at an even step as near `step` as divides its length.
pub fn resample(pts: &[Vec2], closed: bool, step: f32) -> Vec<Vec2> {
    if pts.len() < 2 {
        return pts.to_vec();
    }
    let mut legs: Vec<(Vec2, Vec2)> = pts.windows(2).map(|w| (w[0], w[1])).collect();
    if closed {
        legs.push((pts[pts.len() - 1], pts[0]));
    }
    legs.retain(|(a, b)| a.distance(*b) > 1e-4);
    let total: f32 = legs.iter().map(|(a, b)| a.distance(*b)).sum();
    if legs.is_empty() || total <= 0.0 {
        return pts[..1].to_vec();
    }
    let segs = ((total / step).round() as usize).max(1);
    let d = total / segs as f32;
    let count = if closed { segs } else { segs + 1 };
    let mut out = Vec::with_capacity(count);
    let (mut leg, mut start) = (0usize, 0.0f32);
    for i in 0..count {
        let s = (i as f32 * d).min(total);
        while leg + 1 < legs.len() && s > start + legs[leg].0.distance(legs[leg].1) {
            start += legs[leg].0.distance(legs[leg].1);
            leg += 1;
        }
        let (a, b) = legs[leg];
        let t = ((s - start) / a.distance(b)).clamp(0.0, 1.0);
        out.push(a.lerp(b, t));
    }
    out
}

/// Douglas–Peucker simplification of an open polyline to within `tol`.
pub fn simplify(pts: &[Vec2], tol: f32) -> Vec<Vec2> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let mut keep = vec![false; pts.len()];
    keep[0] = true;
    keep[pts.len() - 1] = true;
    let mut stack = vec![(0usize, pts.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        if b <= a + 1 {
            continue;
        }
        let (pa, pb) = (pts[a], pts[b]);
        let (mut best, mut at) = (0.0f32, a);
        for (i, &p) in pts.iter().enumerate().take(b).skip(a + 1) {
            let d = point_segment(p, pa, pb);
            if d > best {
                best = d;
                at = i;
            }
        }
        if best > tol {
            keep[at] = true;
            stack.push((a, at));
            stack.push((at, b));
        }
    }
    pts.iter()
        .zip(keep)
        .filter_map(|(&p, k)| k.then_some(p))
        .collect()
}

/// Chaikin corner cutting, `iters` rounds; an open polyline keeps its ends.
pub fn chaikin(pts: &[Vec2], closed: bool, iters: usize) -> Vec<Vec2> {
    let mut cur = pts.to_vec();
    for _ in 0..iters {
        if cur.len() < 3 {
            return cur;
        }
        let n = cur.len();
        let mut next = Vec::with_capacity(n * 2 + 2);
        if !closed {
            next.push(cur[0]);
        }
        let segs = if closed { n } else { n - 1 };
        for i in 0..segs {
            let (a, b) = (cur[i], cur[(i + 1) % n]);
            next.push(a.lerp(b, 0.25));
            next.push(a.lerp(b, 0.75));
        }
        if !closed {
            next.push(cur[n - 1]);
        }
        cur = next;
    }
    cur
}

/// Distance from `p` to segment `ab`.
pub fn point_segment(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared().max(1e-9)).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

/// Index `i` wrapped round a closed polyline of `n` points or clamped to an open one.
pub fn wrap(i: isize, n: usize, closed: bool) -> usize {
    if closed {
        i.rem_euclid(n as isize) as usize
    } else {
        i.clamp(0, n as isize - 1) as usize
    }
}

/// Signed curvature (1/yd, + to the left looking down on WoW's x north, y west) at each point of
/// an evenly sampled polyline, from the circle through the points `k` either side.
pub fn curvature(pts: &[Vec2], closed: bool, k: usize) -> Vec<f32> {
    let n = pts.len();
    (0..n)
        .map(|i| {
            let a = pts[wrap(i as isize - k as isize, n, closed)];
            let b = pts[i];
            let c = pts[wrap(i as isize + k as isize, n, closed)];
            signed_curvature(a, b, c)
        })
        .collect()
}

/// The signed curvature of the circle through `a`, `b`, `c` (+ when `c` is left of `a`→`b`
/// in a right-handed x, y plane).
pub fn signed_curvature(a: Vec2, b: Vec2, c: Vec2) -> f32 {
    let (ab, bc, ca) = (a.distance(b), b.distance(c), c.distance(a));
    let denom = ab * bc * ca;
    if denom < 1e-6 {
        return 0.0;
    }
    2.0 * (b - a).perp_dot(c - a) / denom
}

/// A moving average over `±k` points.
pub fn smooth(v: &[f32], closed: bool, k: usize) -> Vec<f32> {
    let n = v.len();
    (0..n)
        .map(|i| {
            let mut s = 0.0;
            let mut c = 0.0;
            for j in -(k as isize)..=k as isize {
                let at = i as isize + j;
                if !closed && (at < 0 || at >= n as isize) {
                    continue;
                }
                s += v[wrap(at, n, closed)];
                c += 1.0;
            }
            s / c
        })
        .collect()
}

/// Whether segments `ab` and `cd` cross properly.
pub fn segments_cross(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> bool {
    let o = |p: Vec2, q: Vec2, r: Vec2| (q - p).perp_dot(r - p);
    let (d1, d2) = (o(a, b, c), o(a, b, d));
    let (d3, d4) = (o(c, d, a), o(c, d, b));
    d1 * d2 < 0.0 && d3 * d4 < 0.0
}

/// Whether the polyline crosses itself (segments at least two apart).
pub fn self_intersects(pts: &[Vec2], closed: bool) -> bool {
    let n = pts.len();
    let segs = if closed { n } else { n.saturating_sub(1) };
    // Bucket the segments in a coarse grid so long courses stay fast.
    let cell = 40.0;
    let mut buckets: std::collections::HashMap<(i32, i32), Vec<usize>> = Default::default();
    for i in 0..segs {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        let (lo, hi) = (a.min(b), a.max(b));
        for cx in (lo.x / cell).floor() as i32..=(hi.x / cell).floor() as i32 {
            for cy in (lo.y / cell).floor() as i32..=(hi.y / cell).floor() as i32 {
                buckets.entry((cx, cy)).or_default().push(i);
            }
        }
    }
    for list in buckets.values() {
        for (k, &i) in list.iter().enumerate() {
            for &j in &list[k + 1..] {
                let gap = i.abs_diff(j);
                if gap < 2 || (closed && gap > segs - 2) {
                    continue;
                }
                if segments_cross(pts[i], pts[(i + 1) % n], pts[j], pts[(j + 1) % n]) {
                    return true;
                }
            }
        }
    }
    false
}

/// The closest approach (yd) between two parts of the polyline at least `apart` yd apart along
/// it: small when the course runs back alongside itself.
pub fn min_separation(pts: &[Vec2], closed: bool, step: f32, apart: f32) -> f32 {
    let n = pts.len();
    let gap = (apart / step).ceil() as usize;
    let cell = 30.0;
    let mut buckets: std::collections::HashMap<(i32, i32), Vec<usize>> = Default::default();
    for (i, p) in pts.iter().enumerate() {
        buckets
            .entry(((p.x / cell).floor() as i32, (p.y / cell).floor() as i32))
            .or_default()
            .push(i);
    }
    let mut best = f32::INFINITY;
    for (i, p) in pts.iter().enumerate() {
        let (cx, cy) = ((p.x / cell).floor() as i32, (p.y / cell).floor() as i32);
        for dx in -1..=1 {
            for dy in -1..=1 {
                for &j in buckets.get(&(cx + dx, cy + dy)).into_iter().flatten() {
                    let mut d = i.abs_diff(j);
                    if closed {
                        d = d.min(n - d);
                    }
                    if d < gap {
                        continue;
                    }
                    best = best.min(p.distance(pts[j]));
                }
            }
        }
    }
    best
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
        assert!((convexity(&square) - 1.0).abs() < 1e-5);
        let l = [
            Vec2::new(0.0, 0.0),
            Vec2::new(10.0, 0.0),
            Vec2::new(10.0, 4.0),
            Vec2::new(4.0, 4.0),
            Vec2::new(4.0, 10.0),
            Vec2::new(0.0, 10.0),
        ];
        // 64 of the hull's 82.
        assert!((convexity(&l) - 64.0 / 82.0).abs() < 1e-3);
    }

    fn circle(r: f32, n: usize) -> Vec<Vec2> {
        (0..n)
            .map(|i| {
                let a = i as f32 / n as f32 * std::f32::consts::TAU;
                Vec2::new(r * a.cos(), r * a.sin())
            })
            .collect()
    }

    #[test]
    fn a_counter_clockwise_circle_curves_left_at_one_over_its_radius() {
        let pts = resample(&circle(50.0, 64), true, 3.0);
        for k in curvature(&pts, true, 4) {
            assert!((k - 1.0 / 50.0).abs() < 1e-3, "{k}");
        }
        assert!((length(&pts, true) - std::f32::consts::TAU * 50.0).abs() < 1.0);
    }

    #[test]
    fn a_figure_eight_crosses_itself_and_a_circle_does_not() {
        assert!(!self_intersects(&circle(50.0, 40), true));
        let eight: Vec<Vec2> = (0..80)
            .map(|i| {
                let t = (i as f32 + 0.5) / 80.0 * std::f32::consts::TAU;
                Vec2::new(100.0 * t.sin(), 50.0 * (2.0 * t).sin())
            })
            .collect();
        assert!(self_intersects(&eight, true));
    }

    #[test]
    fn an_out_and_back_runs_close_to_itself() {
        let mut pts: Vec<Vec2> = (0..100).map(|i| Vec2::new(i as f32 * 3.0, 0.0)).collect();
        pts.extend((0..100).rev().map(|i| Vec2::new(i as f32 * 3.0, 10.0)));
        assert!(min_separation(&pts, true, 3.0, 60.0) <= 10.0 + 1e-3);
        let ring = resample(&circle(100.0, 64), true, 3.0);
        assert!(min_separation(&ring, true, 3.0, 60.0) > 50.0);
    }

    #[test]
    fn simplify_keeps_corners_and_drops_collinear_points() {
        let pts: Vec<Vec2> = (0..=10)
            .map(|i| Vec2::new(i as f32, 0.0))
            .chain((1..=10).map(|i| Vec2::new(10.0, i as f32)))
            .collect();
        let s = simplify(&pts, 0.1);
        assert_eq!(s.len(), 3);
    }
}
