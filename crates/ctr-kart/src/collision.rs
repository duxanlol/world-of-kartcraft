//! The world the kart drives on: a triangle soup (+Y up, in the kart's level units) bucketed in a
//! uniform XZ grid. CTR collides against its tracks' quadblocks through a BSP and tells ground from
//! wall by a per-quad flag; here any mesh will do, and the slope decides (spec section 9).

use std::collections::HashMap;

use glam::Vec3;

/// A face whose normal is this upright or more is ground; steeper is wall (≈ 50°, spec 9.1).
pub const GROUND_NORMAL_Y: f32 = 0.643;

#[derive(Clone, Copy, Debug)]
pub struct Triangle {
    pub a: Vec3,
    pub b: Vec3,
    pub c: Vec3,
    /// Unit normal by the winding, `(b − a) × (c − a)`.
    pub normal: Vec3,
}

impl Triangle {
    pub fn new(a: Vec3, b: Vec3, c: Vec3) -> Option<Self> {
        let normal = (b - a).cross(c - a).try_normalize()?;
        (a.is_finite() && b.is_finite() && c.is_finite()).then_some(Self { a, b, c, normal })
    }

    /// The point of the triangle closest to `p` (Ericson, Real-Time Collision Detection 5.1.5).
    pub fn closest_point(&self, p: Vec3) -> Vec3 {
        let (a, b, c) = (self.a, self.b, self.c);
        let ab = b - a;
        let ac = c - a;
        let ap = p - a;
        let d1 = ab.dot(ap);
        let d2 = ac.dot(ap);
        if d1 <= 0.0 && d2 <= 0.0 {
            return a;
        }
        let bp = p - b;
        let d3 = ab.dot(bp);
        let d4 = ac.dot(bp);
        if d3 >= 0.0 && d4 <= d3 {
            return b;
        }
        let vc = d1 * d4 - d3 * d2;
        if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
            return a + ab * (d1 / (d1 - d3));
        }
        let cp = p - c;
        let d5 = ab.dot(cp);
        let d6 = ac.dot(cp);
        if d6 >= 0.0 && d5 <= d6 {
            return c;
        }
        let vb = d5 * d2 - d1 * d6;
        if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
            return a + ac * (d2 / (d2 - d6));
        }
        let va = d3 * d6 - d5 * d4;
        if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
            return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
        }
        let denom = 1.0 / (va + vb + vc);
        a + ab * (vb * denom) + ac * (vc * denom)
    }

    /// Möller–Trumbore, both faces: the distance along `dir` (unit) to the hit.
    pub fn ray(&self, origin: Vec3, dir: Vec3) -> Option<f32> {
        let e1 = self.b - self.a;
        let e2 = self.c - self.a;
        let p = dir.cross(e2);
        let det = e1.dot(p);
        if det.abs() < 1e-9 {
            return None;
        }
        let inv = 1.0 / det;
        let s = origin - self.a;
        let u = s.dot(p) * inv;
        if !(0.0..=1.0).contains(&u) {
            return None;
        }
        let q = s.cross(e1);
        let v = dir.dot(q) * inv;
        if v < 0.0 || u + v > 1.0 {
            return None;
        }
        let t = e2.dot(q) * inv;
        (t >= 0.0).then_some(t)
    }

    /// The normal turned to face up (WoW's collision winding is not reliable for our purposes).
    pub fn up_normal(&self) -> Vec3 {
        if self.normal.y < 0.0 {
            -self.normal
        } else {
            self.normal
        }
    }
}

/// A ray or probe hit: where, the face normal turned toward the probe, and how far.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub point: Vec3,
    pub normal: Vec3,
    pub distance: f32,
}

/// A sphere overlapping a face: how deep, and the direction out of it (face to centre).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Contact {
    pub depth: f32,
    pub normal: Vec3,
    pub point: Vec3,
}

pub struct Collision {
    cell: f32,
    tris: Vec<Triangle>,
    cells: HashMap<(i32, i32), Vec<u32>>,
}

impl Collision {
    /// Buckets the faces in `cell`-sized XZ squares; degenerate and non-finite faces are dropped.
    pub fn new(triangles: impl IntoIterator<Item = [[f32; 3]; 3]>, cell: f32) -> Self {
        let mut out = Self {
            cell,
            tris: Vec::new(),
            cells: HashMap::new(),
        };
        for t in triangles {
            let [a, b, c] = t.map(Vec3::from_array);
            let Some(tri) = Triangle::new(a, b, c) else {
                continue;
            };
            let index = out.tris.len() as u32;
            let min = a.min(b).min(c);
            let max = a.max(b).max(c);
            let (x0, z0) = out.cell_of(min);
            let (x1, z1) = out.cell_of(max);
            // A sliver spanning a huge area would flood the grid; such faces are not drivable.
            if i64::from(x1 - x0 + 1) * i64::from(z1 - z0 + 1) > 4096 {
                continue;
            }
            for x in x0..=x1 {
                for z in z0..=z1 {
                    out.cells.entry((x, z)).or_default().push(index);
                }
            }
            out.tris.push(tri);
        }
        out
    }

    fn cell_of(&self, p: Vec3) -> (i32, i32) {
        (
            (p.x / self.cell).floor() as i32,
            (p.z / self.cell).floor() as i32,
        )
    }

    pub fn len(&self) -> usize {
        self.tris.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tris.is_empty()
    }

    /// Every face whose cell overlaps the XZ box, each once.
    fn faces_in(&self, min: Vec3, max: Vec3, mut f: impl FnMut(&Triangle)) {
        let (x0, z0) = self.cell_of(min);
        let (x1, z1) = self.cell_of(max);
        if x0 == x1 && z0 == z1 {
            for &i in self.cells.get(&(x0, z0)).into_iter().flatten() {
                f(&self.tris[i as usize]);
            }
            return;
        }
        let mut seen = std::collections::HashSet::new();
        for x in x0..=x1 {
            for z in z0..=z1 {
                for &i in self.cells.get(&(x, z)).into_iter().flatten() {
                    if seen.insert(i) {
                        f(&self.tris[i as usize]);
                    }
                }
            }
        }
    }

    /// The nearest face hit by a ray from `origin` along `dir` within `max`. The normal is turned
    /// to face back along the ray.
    pub fn ray(&self, origin: Vec3, dir: Vec3, max: f32) -> Option<Hit> {
        let dir = dir.try_normalize()?;
        let end = origin + dir * max;
        let mut best: Option<Hit> = None;
        self.faces_in(origin.min(end), origin.max(end), |t| {
            if let Some(d) = t.ray(origin, dir) {
                if d <= max && best.is_none_or(|b| d < b.distance) {
                    let normal = if t.normal.dot(dir) > 0.0 {
                        -t.normal
                    } else {
                        t.normal
                    };
                    best = Some(Hit {
                        point: origin + dir * d,
                        normal,
                        distance: d,
                    });
                }
            }
        });
        best
    }

    /// The ground under `from`: the first ground face straight down within `max` (spec 9.3).
    pub fn ground(&self, from: Vec3, max: f32) -> Option<Hit> {
        let mut best: Option<Hit> = None;
        self.faces_in(from, from, |t| {
            let up = t.up_normal();
            if up.y < GROUND_NORMAL_Y {
                return;
            }
            if let Some(d) = t.ray(from, Vec3::NEG_Y) {
                if d <= max && best.is_none_or(|b| d < b.distance) {
                    best = Some(Hit {
                        point: from - Vec3::Y * d,
                        normal: up,
                        distance: d,
                    });
                }
            }
        });
        best
    }

    /// The deepest face a sphere overlaps, any face.
    pub fn deepest(&self, centre: Vec3, radius: f32) -> Option<Contact> {
        let r = Vec3::splat(radius);
        let mut best: Option<Contact> = None;
        self.faces_in(centre - r, centre + r, |t| {
            let q = t.closest_point(centre);
            let d = centre - q;
            let dist_sq = d.length_squared();
            if dist_sq >= radius * radius {
                return;
            }
            let dist = dist_sq.sqrt();
            // The centre on the face itself: push along the face normal, toward the side it
            // came from is unknown, so up-facing for ground and the winding for walls.
            let normal = if dist > 1e-4 {
                d / dist
            } else if t.up_normal().y >= GROUND_NORMAL_Y {
                t.up_normal()
            } else {
                t.normal
            };
            let depth = radius - dist;
            if best.is_none_or(|b| depth > b.depth) {
                best = Some(Contact {
                    depth,
                    normal,
                    point: q,
                });
            }
        });
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat floor at y = 0 and a wall at x = 5 facing −X, over x, z in [−20, 20].
    fn arena() -> Collision {
        let s = 20.0;
        Collision::new(
            [
                [[-s, 0.0, -s], [-s, 0.0, s], [s, 0.0, s]],
                [[-s, 0.0, -s], [s, 0.0, s], [s, 0.0, -s]],
                [[5.0, 0.0, -s], [5.0, 5.0, s], [5.0, 0.0, s]],
                [[5.0, 0.0, -s], [5.0, 5.0, -s], [5.0, 5.0, s]],
            ],
            4.0,
        )
    }

    #[test]
    fn ground_under_a_point() {
        let c = arena();
        let hit = c.ground(Vec3::new(1.0, 2.0, 3.0), 10.0).unwrap();
        assert!(hit.point.y.abs() < 1e-5);
        assert!((hit.normal.y - 1.0).abs() < 1e-5);
        assert!((hit.distance - 2.0).abs() < 1e-5);
        assert!(c.ground(Vec3::new(1.0, 2.0, 3.0), 1.0).is_none());
        assert!(c.ground(Vec3::new(30.0, 2.0, 3.0), 10.0).is_none());
    }

    #[test]
    fn the_deepest_contact_is_the_wall() {
        let c = arena();
        let hit = c.deepest(Vec3::new(4.7, 0.6, 0.0), 0.5).unwrap();
        assert!((hit.depth - 0.2).abs() < 1e-4, "{hit:?}");
        assert!(hit.normal.x < -0.99);
        let floor = c.deepest(Vec3::new(0.0, 0.4, 0.0), 0.5).unwrap();
        assert!((floor.normal.y - 1.0).abs() < 1e-5);
    }

    #[test]
    fn ray_hits_the_wall() {
        let c = arena();
        let hit = c.ray(Vec3::new(0.0, 1.0, 0.0), Vec3::X, 10.0).unwrap();
        assert!((hit.distance - 5.0).abs() < 1e-4);
        assert!(hit.normal.x < -0.99);
    }

    #[test]
    fn degenerate_faces_are_dropped() {
        let c = Collision::new([[[0.0; 3], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]]], 4.0);
        assert!(c.is_empty());
    }
}
