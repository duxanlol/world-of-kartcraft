//! The flight scout: flight courses found in the real world. It samples the surface a kart lands
//! on (the ground, a roof, a mesa's top) over an area, finds its tops (walkable spots well over
//! what is round them), chains tops each lower than the last by more than a glide comes down
//! (`kart_driver::fly`: about a yard in six) with the air between them clear, and hangs a ring
//! low over each top after the first: a course flown top to top, landing on one or skimming it.
//! Each chain is flown by the flight bot ([`crate::flight::fly_in`]) and kept to the rings it
//! passed.

use anyhow::Result;
use benilla_formats::Chain;
use serde_json::{json, Value};

use crate::flight::{fly_in, FlightCourse, Sky};

/// How the scout looks.
#[derive(Clone, Copy, Debug)]
pub struct ScoutOpts {
    /// The sampling step, yards.
    pub step: f32,
    /// How far over the lowest of what is round it (yards) a top stands, within `reach`.
    pub prominence: f32,
    pub reach: f32,
    /// The most a walkable spot rises to its neighbour, yards a step.
    pub flat: f32,
    /// The fewest samples a top holds.
    pub min_cells: usize,
    /// Yards across per yard down a hop may ask of a glide (the bot's is about 6).
    pub glide: f32,
    /// The most yards down per yard across a hop may ask of a dive.
    pub dive: f32,
    /// A hop's least and most length, yards.
    pub hop: (f32, f32),
    /// The sharpest turn between hops, degrees.
    pub turn: f32,
    /// The rings' radius and how far over the top their bottom hangs, yards.
    pub ring: f32,
    pub over: f32,
    /// The least clearance of the line between rings over the ground under it, yards.
    pub clear: f32,
    /// The fewest rings a course keeps.
    pub min_rings: usize,
}

impl Default for ScoutOpts {
    fn default() -> Self {
        Self {
            step: 3.0,
            prominence: 10.0,
            reach: 30.0,
            flat: 1.6,
            min_cells: 6,
            glide: 5.0,
            dive: 0.45,
            hop: (25.0, 220.0),
            turn: 70.0,
            ring: 10.0,
            over: 2.0,
            clear: 5.0,
            min_rings: 3,
        }
    }
}

/// A top: its middle (WoW x, y), its height and how many samples it holds.
#[derive(Clone, Copy, Debug)]
pub struct Top {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub cells: usize,
    /// Water's surface, not land.
    pub wet: bool,
}

/// A course found: its checkpoints (WoW x, y, z; the start, then the rings), the rings the bot
/// passed, its length (yards) and the bot's time (s).
#[derive(Clone, Debug)]
pub struct Found {
    pub points: Vec<[f32; 3]>,
    pub passed: usize,
    pub yards: f32,
    pub seconds: f32,
    /// The bot's flight, WoW x, y, z, a point every few ticks.
    pub flown: Vec<[f32; 3]>,
    /// Its map ([`picture`]): hillshade, rings, the flight and its profile.
    pub picture: Option<image::RgbImage>,
    /// Its rings' radius, yards, and what it is: tops hopped, or a glide run.
    pub ring: f32,
    pub run: bool,
}

/// The surface's height on a grid: `h[j * n + i]` at WoW x = x0 + i step, y = y0 + j step.
struct Field {
    x0: f32,
    y0: f32,
    step: f32,
    n: usize,
    h: Vec<f32>,
    /// Where the floor is water's surface.
    wet: Vec<bool>,
}

impl Field {
    fn sample(sky: &Sky, centre: [f32; 2], half: f32, step: f32) -> Self {
        let n = (2.0 * half / step) as usize + 1;
        let (x0, y0) = (centre[0] - half, centre[1] - half);
        let mut h = vec![f32::NAN; n * n];
        let mut wet = vec![false; n * n];
        for j in 0..n {
            for i in 0..n {
                let (x, y) = (x0 + i as f32 * step, y0 + j as f32 * step);
                let floor = sky.floor(x, y).unwrap_or(f32::NAN);
                h[j * n + i] = floor;
                wet[j * n + i] = sky.ground(x, y).is_none_or(|g| floor > g + 0.5);
            }
        }
        Self {
            x0,
            y0,
            step,
            n,
            h,
            wet,
        }
    }

    fn at(&self, i: usize, j: usize) -> f32 {
        self.h[j * self.n + i]
    }

    /// The height under WoW (x, y), the nearest sample's.
    fn under(&self, x: f32, y: f32) -> f32 {
        let i = ((x - self.x0) / self.step).round();
        let j = ((y - self.y0) / self.step).round();
        if i < 0.0 || j < 0.0 || i as usize >= self.n || j as usize >= self.n {
            return f32::NAN;
        }
        self.at(i as usize, j as usize)
    }

    /// The tops: walkable samples standing `prominence` over the lowest within `reach`, gathered
    /// into the flats they make.
    fn tops(&self, o: &ScoutOpts) -> Vec<Top> {
        let n = self.n;
        let r = (o.reach / self.step).ceil() as i64;
        let mut top = vec![false; n * n];
        for j in 0..n {
            for i in 0..n {
                let z = self.at(i, j);
                if !z.is_finite() {
                    continue;
                }
                let mut walk = true;
                for (di, dj) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
                    let (a, b) = (i as i64 + di, j as i64 + dj);
                    if a < 0 || b < 0 || a >= n as i64 || b >= n as i64 {
                        continue;
                    }
                    let w = self.at(a as usize, b as usize);
                    walk &= w.is_finite() && (w - z).abs() <= o.flat;
                }
                if !walk {
                    continue;
                }
                let mut low = z;
                for dj in -r..=r {
                    for di in -r..=r {
                        if di * di + dj * dj > r * r {
                            continue;
                        }
                        let (a, b) = (i as i64 + di, j as i64 + dj);
                        if a < 0 || b < 0 || a >= n as i64 || b >= n as i64 {
                            continue;
                        }
                        let w = self.at(a as usize, b as usize);
                        if w.is_finite() {
                            low = low.min(w);
                        }
                    }
                }
                top[j * n + i] = z - low >= o.prominence;
            }
        }
        // The flats: top samples next to one another and as high, filled out.
        let mut seen = vec![false; n * n];
        let mut out = Vec::new();
        for start in 0..n * n {
            if !top[start] || seen[start] {
                continue;
            }
            let mut stack = vec![start];
            seen[start] = true;
            let (mut sx, mut sy, mut zs, mut cells, mut wet) =
                (0.0, 0.0, Vec::new(), 0usize, 0usize);
            while let Some(c) = stack.pop() {
                let (i, j) = (c % n, c / n);
                sx += self.x0 + i as f32 * self.step;
                sy += self.y0 + j as f32 * self.step;
                zs.push(self.h[c]);
                cells += 1;
                wet += usize::from(self.wet[c]);
                for (di, dj) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
                    let (a, b) = (i as i64 + di, j as i64 + dj);
                    if a < 0 || b < 0 || a >= n as i64 || b >= n as i64 {
                        continue;
                    }
                    let d = b as usize * n + a as usize;
                    if top[d] && !seen[d] && (self.h[d] - self.h[c]).abs() <= o.flat {
                        seen[d] = true;
                        stack.push(d);
                    }
                }
            }
            if cells < o.min_cells {
                continue;
            }
            zs.sort_by(f32::total_cmp);
            out.push(Top {
                x: sx / cells as f32,
                y: sy / cells as f32,
                z: zs[zs.len() / 2],
                cells,
                wet: wet * 2 > cells,
            });
        }
        out
    }

    /// Whether the line from `a` to `b` (WoW x, y, z) keeps `clear` yards over the surface, but
    /// near its ends, which are over tops.
    fn clear(&self, a: [f32; 3], b: [f32; 3], clear: f32) -> bool {
        let d = (b[0] - a[0]).hypot(b[1] - a[1]);
        let steps = (d / self.step).ceil().max(1.0) as usize;
        for k in 0..=steps {
            let t = k as f32 / steps as f32;
            let along = t * d;
            if along < 8.0 || d - along < 8.0 {
                continue;
            }
            let p = [
                a[0] + (b[0] - a[0]) * t,
                a[1] + (b[1] - a[1]) * t,
                a[2] + (b[2] - a[2]) * t,
            ];
            let g = self.under(p[0], p[1]);
            if !g.is_finite() || g > p[2] - clear {
                return false;
            }
        }
        true
    }
}

/// The height (yards) a kart falls off its first edge before its wings hold it.
const TAKE_OFF: f32 = 10.0;

/// The ring over a top: its centre, its bottom `over` yards over the top.
fn ring_over(t: &Top, o: &ScoutOpts) -> [f32; 3] {
    [t.x, t.y, t.z + o.over + o.ring]
}

/// The chains of tops, longest first: each hop down more than a glide, its air clear, turning no
/// sharper than `turn`.
fn chains(tops: &[Top], field: &Field, o: &ScoutOpts) -> Vec<Vec<usize>> {
    let n = tops.len();
    let land: Vec<bool> = tops.iter().map(|t| !t.wet).collect();
    // The hops: i to j, j lower by more than the glide asks.
    let mut next: Vec<Vec<usize>> = vec![Vec::new(); n];
    for i in 0..n {
        for j in 0..n {
            let (a, b) = (&tops[i], &tops[j]);
            let d = (b.x - a.x).hypot(b.y - a.y);
            let drop = a.z - b.z;
            if d < o.hop.0 || d > o.hop.1 || drop < d / o.glide + 2.0 || drop > d * o.dive + 4.0 {
                continue;
            }
            // Water is no top: a kart skims it, but its hops are the land's.
            if !land[i] || !land[j] {
                continue;
            }
            let from = [a.x, a.y, a.z + o.over + o.ring];
            if field.clear(from, ring_over(b, o), o.clear) {
                next[i].push(j);
            }
        }
    }
    // The longest chain on from each hop, by the hops after it (highest last, so lower first).
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| tops[a].z.total_cmp(&tops[b].z));
    let turn_ok = |a: usize, b: usize, c: usize| {
        let (u, v) = (
            (tops[b].x - tops[a].x, tops[b].y - tops[a].y),
            (tops[c].x - tops[b].x, tops[c].y - tops[b].y),
        );
        let cos = (u.0 * v.0 + u.1 * v.1) / (u.0.hypot(u.1) * v.0.hypot(v.1)).max(1e-3);
        cos >= o.turn.to_radians().cos()
    };
    // best[(i, j)]: the chain from hop i->j on, as the tops after j.
    let mut best: std::collections::HashMap<(usize, usize), Vec<usize>> = Default::default();
    for &j in &order {
        for i in 0..n {
            if !next[i].contains(&j) {
                continue;
            }
            let mut tail: Vec<usize> = Vec::new();
            for &k in &next[j] {
                if !turn_ok(i, j, k) {
                    continue;
                }
                if let Some(t) = best.get(&(j, k)) {
                    if t.len() + 1 > tail.len() {
                        tail = std::iter::once(k).chain(t.iter().copied()).collect();
                    }
                }
            }
            best.insert((i, j), tail);
        }
    }
    eprintln!(
        "{} hops, the longest chain {} tops",
        next.iter().map(Vec::len).sum::<usize>(),
        best.values().map(|t| t.len() + 2).max().unwrap_or(0)
    );
    let mut out: Vec<Vec<usize>> = best
        .into_iter()
        .map(|((i, j), tail)| [i, j].into_iter().chain(tail).collect())
        .collect();
    out.sort_by_key(|c| std::cmp::Reverse(c.len()));
    out
}

/// The start on the first top: back from its middle away from the first hop, as far as the top
/// runs flat (at most 30 yards), for a run-up to its edge.
fn start_on(t: &Top, toward: &Top, field: &Field, o: &ScoutOpts) -> [f32; 3] {
    let (dx, dy) = (toward.x - t.x, toward.y - t.y);
    let l = dx.hypot(dy).max(1e-3);
    let (ux, uy) = (-dx / l, -dy / l);
    let mut back = 0.0;
    while back < 30.0 {
        let g = field.under(t.x + ux * (back + o.step), t.y + uy * (back + o.step));
        if !g.is_finite() || (g - t.z).abs() > 2.0 {
            break;
        }
        back += o.step;
    }
    let s = back * 0.8;
    let (x, y) = (t.x + ux * s, t.y + uy * s);
    [x, y, field.under(x, y)]
}

/// Scouts `half` yards round WoW `centre` on `map` (the ground near `z`): the courses the bot
/// flies, the most rings first, at most `keep`, no two from the same first top.
pub fn scout(
    chain: &mut Chain,
    map: u32,
    centre: [f32; 2],
    half: f32,
    z: f32,
    o: &ScoutOpts,
    keep: usize,
) -> Result<Vec<Found>> {
    // The chains run over the area; the world round it a little wider, for the flights.
    let sky = Sky::gather_area(chain, map, centre, half + 60.0, z)?;
    let field = Field::sample(&sky, centre, half, o.step);
    let tops = field.tops(o);
    eprintln!("{} tops", tops.len());
    let mut found: Vec<Found> = Vec::new();
    let mut starts: Vec<usize> = Vec::new();
    let mut tried = 0;
    for c in chains(&tops, &field, o) {
        if c.len() < o.min_rings + 1 || found.len() >= keep || tried >= keep * 6 {
            break;
        }
        if starts.contains(&c[0]) {
            continue;
        }
        let (a, b) = (&tops[c[0]], &tops[c[1]]);
        if a.z - b.z < (b.x - a.x).hypot(b.y - a.y) / o.glide + TAKE_OFF {
            continue;
        }
        tried += 1;
        let start = start_on(&tops[c[0]], &tops[c[1]], &field, o);
        if !start[2].is_finite() {
            continue;
        }
        let mut points = vec![start];
        points.extend(c[1..].iter().map(|&i| ring_over(&tops[i], o)));
        let course = FlightCourse {
            map,
            rings: (1..points.len()).collect(),
            radii: std::iter::once(25.0)
                .chain(std::iter::repeat(o.ring))
                .take(points.len())
                .collect(),
            points: points.clone(),
        };
        let Ok((lines, r)) = fly_in(&sky, &course) else {
            continue;
        };
        let passed = lines.iter().take_while(|l| l.passed).count();
        eprintln!(
            "  chain of {} from {:.0},{:.0},{:.0}: {passed} passed{}",
            c.len(),
            start[0],
            start[1],
            start[2],
            lines
                .iter()
                .find(|l| !l.passed)
                .map_or(String::new(), |l| format!(
                    ", ring {} missed at z {:.0} (ring {:.0}, {:.0} off)",
                    l.ring, l.z, l.ring_z, l.off
                ))
        );
        if passed < o.min_rings {
            continue;
        }
        let points: Vec<[f32; 3]> = points[..=passed].to_vec();
        let yards = points
            .windows(2)
            .map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]))
            .sum();
        let seconds = r
            .passes
            .get(passed - 1)
            .map_or(0.0, |p| p.tick as f32 / 30.0);
        starts.push(c[0]);
        let flown: Vec<[f32; 3]> = r.path.iter().map(|&p| sky.wow(p)).collect();
        let picture = Some(picture(&field, &points, &flown));
        found.push(Found {
            points,
            passed,
            yards,
            seconds,
            flown,
            picture,
            ring: o.ring,
            run: false,
        });
    }
    found.sort_by(|a, b| b.passed.cmp(&a.passed).then(b.yards.total_cmp(&a.yards)));
    Ok(found)
}

/// A found course as the course file's entry (its marshal for `course-gen marshals` to write).
pub fn course_entry(f: &Found, key: &str, name: &str, zone: &str, map: u32) -> Value {
    let ring = f.ring;
    let tenth = |v: f32| (f64::from(v) * 10.0).round() / 10.0;
    let mut cps = vec![
        json!({ "gate": 25, "x": tenth(f.points[0][0]), "y": tenth(f.points[0][1]),
        "z": tenth(f.points[0][2]) }),
    ];
    for p in &f.points[1..] {
        cps.push(
            json!({ "gate": ring, "ring": [0, 0], "x": tenth(p[0]), "y": tenth(p[1]),
            "z": tenth(p[2]) }),
        );
    }
    json!({
        "key": key,
        "name": name,
        "zone": zone,
        "map": map,
        "laps": 0,
        "flight": true,
        "note": if f.run {
            format!("flight course found by the flight scout (course-gen scout --runs): a glide run of {} rings low over the ground and between higher ground, {:.0} yd, the bot's {:.1} s.", f.passed, f.yards, f.seconds)
        } else {
            format!("flight course found by the flight scout (course-gen scout): {} rings low over the tops it hops, {:.0} yd, the bot's {:.1} s.", f.passed, f.yards, f.seconds)
        },
        "checkpoints": cps,
    })
}

/// A glide run's options ([`glide_runs`]).
#[derive(Clone, Copy, Debug)]
pub struct RunOpts {
    /// Yards across per yard down the path keeps (the bot glides about 6).
    pub glide: f32,
    /// A step's length, yards, and the turns a step may take, degrees.
    pub step: f32,
    pub turn: f32,
    /// The least clearance over the floor, yards, past the first `runup` yards.
    pub clear: f32,
    pub runup: f32,
    /// How near the floor (yards over the clearance) a step counts as low.
    pub low: f32,
    /// How far to each side (yards) higher ground counts as a wall beside the path.
    pub wall: f32,
    /// The paths kept at each step.
    pub beam: usize,
    /// The longest path, yards.
    pub longest: f32,
    /// A ring every this many yards of path, and its radius.
    pub every: f32,
    pub ring: f32,
}

impl Default for RunOpts {
    fn default() -> Self {
        Self {
            glide: 5.5,
            step: 15.0,
            turn: 20.0,
            clear: 7.0,
            runup: 45.0,
            low: 25.0,
            wall: 35.0,
            beam: 300,
            longest: 1600.0,
            every: 90.0,
            ring: 12.0,
        }
    }
}

#[derive(Clone)]
struct Path {
    pts: Vec<[f32; 3]>,
    heading: f32,
    yards: f32,
    score: f32,
}

impl Field {
    /// How interesting a point of a path is: low over the floor, and beside higher ground.
    fn interest(&self, p: [f32; 3], heading: f32, o: &RunOpts) -> f32 {
        let floor = self.under(p[0], p[1]);
        let over = p[2] - floor - o.clear;
        let low = (1.0 - over / o.low).clamp(0.0, 1.0);
        let (sx, sy) = (-heading.sin(), heading.cos());
        let mut walls = 0.0;
        for side in [-1.0f32, 1.0] {
            let mut d = 8.0;
            while d <= o.wall {
                let g = self.under(p[0] + sx * side * d, p[1] + sy * side * d);
                if g.is_finite() && g > p[2] - 2.0 {
                    walls += 1.0;
                    break;
                }
                d += 6.0;
            }
        }
        low + 0.8 * walls + 0.6 * low * walls
    }

    /// Whether the step from `a` to `b` keeps its clearance (none asked within `free` yards of the
    /// path's start).
    fn step_clear(&self, a: [f32; 3], b: [f32; 3], walked: f32, o: &RunOpts) -> bool {
        let d = (b[0] - a[0]).hypot(b[1] - a[1]);
        let n = (d / 3.0).ceil().max(1.0) as usize;
        for k in 1..=n {
            let t = k as f32 / n as f32;
            if walked + t * d < o.runup {
                continue;
            }
            let p = [
                a[0] + (b[0] - a[0]) * t,
                a[1] + (b[1] - a[1]) * t,
                a[2] + (b[2] - a[2]) * t,
            ];
            let g = self.under(p[0], p[1]);
            if !g.is_finite() || g > p[2] - o.clear {
                return false;
            }
        }
        true
    }
}

/// The glide runs off `from` (a start on a top, WoW x, y, z) heading out every way: paths down
/// a glide's slope, as long and as low and walled as the search finds, the best first.
fn glide_runs(field: &Field, from: [f32; 3], o: &RunOpts) -> Vec<Path> {
    let mut beam: Vec<Path> = (0..16)
        .map(|k| Path {
            pts: vec![from],
            heading: k as f32 * std::f32::consts::TAU / 16.0,
            yards: 0.0,
            score: 0.0,
        })
        .collect();
    let mut done: Vec<Path> = Vec::new();
    let turns = [-o.turn, 0.0, o.turn].map(f32::to_radians);
    while !beam.is_empty() {
        let mut next = Vec::new();
        for p in &beam {
            let last = *p.pts.last().unwrap();
            let mut went = false;
            for t in turns {
                let h = p.heading + t;
                // The take-off's fall at the run-up's end, then the glide.
                let fall = if p.yards < o.runup && p.yards + o.step >= o.runup {
                    10.0
                } else {
                    0.0
                };
                let b = [
                    last[0] + h.cos() * o.step,
                    last[1] + h.sin() * o.step,
                    last[2] - o.step / o.glide - fall,
                ];
                if !field.step_clear(last, b, p.yards, o) {
                    continue;
                }
                went = true;
                let mut q = p.clone();
                q.pts.push(b);
                q.heading = h;
                q.yards += o.step;
                if q.yards > o.runup {
                    q.score += field.interest(b, h, o);
                }
                if q.yards >= o.longest {
                    done.push(q);
                } else {
                    next.push(q);
                }
            }
            if !went && p.yards > o.runup {
                done.push(p.clone());
            }
        }
        // The best by score with length: a long run of interest.
        next.sort_by(|a, b| (b.score + b.yards * 0.01).total_cmp(&(a.score + a.yards * 0.01)));
        let mut kept: Vec<Path> = Vec::new();
        for q in next {
            let e = *q.pts.last().unwrap();
            if kept.iter().any(|k| {
                let f = *k.pts.last().unwrap();
                (e[0] - f[0]).hypot(e[1] - f[1]) < 6.0
            }) {
                continue;
            }
            kept.push(q);
            if kept.len() == o.beam {
                break;
            }
        }
        beam = kept;
    }
    done.sort_by(|a, b| b.score.total_cmp(&a.score));
    done
}

/// Scouts glide runs `half` yards round WoW `centre` on `map`: off the highest tops, the runs
/// the bot flies, rings every [`RunOpts::every`] yards of path, the most interest first.
#[allow(clippy::too_many_arguments)]
pub fn scout_runs(
    chain: &mut Chain,
    map: u32,
    centre: [f32; 2],
    half: f32,
    z: f32,
    o: &ScoutOpts,
    r: &RunOpts,
    keep: usize,
) -> Result<Vec<Found>> {
    let sky = Sky::gather_area(chain, map, centre, half + 60.0, z)?;
    let field = Field::sample(&sky, centre, half, o.step);
    let mut tops: Vec<Top> = field.tops(o).into_iter().filter(|t| !t.wet).collect();
    tops.sort_by(|a, b| b.z.total_cmp(&a.z));
    eprintln!("{} tops", tops.len());
    let mut found: Vec<Found> = Vec::new();
    for t in tops.iter().take(keep * 3) {
        if found
            .iter()
            .any(|f| (f.points[0][0] - t.x).hypot(f.points[0][1] - t.y) < 60.0)
        {
            continue;
        }
        let runs = glide_runs(&field, [t.x, t.y, t.z], r);
        let Some(best) = runs.into_iter().next() else {
            continue;
        };
        // Rings along the path past the run-up, every `every` yards, the last at its end.
        let per = (r.every / r.step).round().max(1.0) as usize;
        let first = (r.runup / r.step).ceil() as usize + per / 2;
        let mut points = vec![[t.x, t.y, t.z]];
        let mut k = first;
        while k < best.pts.len() {
            points.push(best.pts[k]);
            k += per;
        }
        if points.len() < o.min_rings + 1 {
            continue;
        }
        let course = FlightCourse {
            map,
            rings: (1..points.len()).collect(),
            radii: std::iter::once(25.0)
                .chain(std::iter::repeat(r.ring))
                .take(points.len())
                .collect(),
            points: points.clone(),
        };
        let Ok((lines, rep)) = fly_in(&sky, &course) else {
            continue;
        };
        let passed = lines.iter().take_while(|l| l.passed).count();
        eprintln!(
            "  run of {:.0} yd (interest {:.0}) from {:.0},{:.0},{:.0}: {passed} of {} rings{}",
            best.yards,
            best.score,
            t.x,
            t.y,
            t.z,
            points.len() - 1,
            lines
                .iter()
                .find(|l| !l.passed)
                .map_or(String::new(), |l| format!(
                    ", ring {} missed at z {:.0} (ring {:.0}, {:.0} off)",
                    l.ring, l.z, l.ring_z, l.off
                ))
        );
        if passed < o.min_rings {
            continue;
        }
        let points: Vec<[f32; 3]> = points[..=passed].to_vec();
        let yards = points
            .windows(2)
            .map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]))
            .sum();
        let seconds = rep
            .passes
            .get(passed - 1)
            .map_or(0.0, |p| p.tick as f32 / 30.0);
        let flown: Vec<[f32; 3]> = rep.path.iter().map(|&p| sky.wow(p)).collect();
        let picture = Some(picture(&field, &points, &flown));
        found.push(Found {
            points,
            passed,
            yards,
            seconds,
            flown,
            picture,
            ring: r.ring,
            run: true,
        });
        if found.len() >= keep {
            break;
        }
    }
    Ok(found)
}

/// A found course's map: the surface hillshaded north up round its rings (darker lower, water
/// blue), the rings yellow, the start green, the bot's flight red; under it the profile, the floor
/// under the flight grey and the flight red, the rings' heights yellow.
fn picture(field: &Field, points: &[[f32; 3]], flown: &[[f32; 3]]) -> image::RgbImage {
    use image::{Rgb, RgbImage};
    const PX: f32 = 2.0; // yards a pixel
    let all = points.iter().chain(flown);
    let (mut x0, mut x1, mut y0, mut y1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for p in all {
        (x0, x1, y0, y1) = (x0.min(p[0]), x1.max(p[0]), y0.min(p[1]), y1.max(p[1]));
    }
    let m = 60.0;
    let (x0, x1, y0, y1) = (x0 - m, x1 + m, y0 - m, y1 + m);
    // North (+x) up, west (+y) left.
    let (w, h) = (((y1 - y0) / PX) as u32, ((x1 - x0) / PX) as u32);
    let ph = 140u32;
    let mut img = RgbImage::from_pixel(w.max(200), h + ph, Rgb([24, 24, 28]));
    let at = |x: f32, y: f32| -> (i64, i64) { (((y1 - y) / PX) as i64, ((x1 - x) / PX) as i64) };
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for py in 0..h {
        for px in 0..w {
            let (x, y) = (x1 - py as f32 * PX, y1 - px as f32 * PX);
            let z = field.under(x, y);
            if z.is_finite() {
                (lo, hi) = (lo.min(z), hi.max(z));
            }
        }
    }
    let span = (hi - lo).max(1.0);
    for py in 0..h {
        for px in 0..w {
            let (x, y) = (x1 - py as f32 * PX, y1 - px as f32 * PX);
            let z = field.under(x, y);
            if !z.is_finite() {
                continue;
            }
            // Lit from the north-west.
            let zn = field.under(x + PX, y + PX);
            let shade = if zn.is_finite() {
                ((zn - z) * 0.5).clamp(-0.4, 0.4)
            } else {
                0.0
            };
            let v = (0.25 + 0.6 * (z - lo) / span + shade).clamp(0.05, 1.0);
            let wet = {
                let i = ((x - field.x0) / field.step).round();
                let j = ((y - field.y0) / field.step).round();
                i >= 0.0
                    && j >= 0.0
                    && (i as usize) < field.n
                    && (j as usize) < field.n
                    && field.wet[j as usize * field.n + i as usize]
            };
            let c = if wet {
                [(40.0 * v) as u8, (80.0 * v) as u8, (200.0 * v) as u8]
            } else {
                [(200.0 * v) as u8, (180.0 * v) as u8, (150.0 * v) as u8]
            };
            img.put_pixel(px, py, Rgb(c));
        }
    }
    let dot = |img: &mut RgbImage, x: i64, y: i64, r: i64, c: [u8; 3]| {
        for dy in -r..=r {
            for dx in -r..=r {
                let (px, py) = (x + dx, y + dy);
                if dx * dx + dy * dy <= r * r
                    && px >= 0
                    && py >= 0
                    && (px as u32) < img.width()
                    && (py as u32) < h
                {
                    img.put_pixel(px as u32, py as u32, Rgb(c));
                }
            }
        }
    };
    for p in flown {
        let (px, py) = at(p[0], p[1]);
        dot(&mut img, px, py, 1, [230, 40, 40]);
    }
    for (k, p) in points.iter().enumerate() {
        let (px, py) = at(p[0], p[1]);
        let c = if k == 0 {
            [40, 220, 60]
        } else {
            [250, 210, 30]
        };
        dot(&mut img, px, py, if k == 0 { 6 } else { 4 }, c);
    }
    // The profile: along the flight, the floor under it and its height.
    if flown.len() >= 2 {
        let mut along = vec![0.0f32];
        for w in flown.windows(2) {
            along.push(along.last().unwrap() + (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]));
        }
        let total = along.last().copied().unwrap_or(1.0).max(1.0);
        let zs: Vec<f32> = flown.iter().map(|p| p[2]).collect();
        let floors: Vec<f32> = flown.iter().map(|p| field.under(p[0], p[1])).collect();
        let zmax = zs.iter().copied().fold(f32::MIN, f32::max);
        let zmin = floors
            .iter()
            .copied()
            .filter(|z| z.is_finite())
            .fold(f32::MAX, f32::min)
            .min(zmax - 10.0);
        let iw = img.width() as f32;
        let ty = |z: f32| {
            h as i64 + 10 + ((zmax - z) / (zmax - zmin).max(1.0) * (ph as f32 - 20.0)) as i64
        };
        for (k, a) in along.iter().enumerate() {
            let px = (a / total * (iw - 1.0)) as i64;
            if floors[k].is_finite() {
                for y in ty(floors[k])..(h + ph) as i64 {
                    if y >= 0 && (y as u32) < img.height() && px >= 0 && (px as u32) < img.width() {
                        img.put_pixel(px as u32, y as u32, Rgb([90, 90, 96]));
                    }
                }
            }
        }
        for (k, a) in along.iter().enumerate() {
            let px = (a / total * (iw - 1.0)) as i64;
            let y = ty(zs[k]);
            if y >= 0 && (y as u32) < img.height() && px >= 0 && (px as u32) < img.width() {
                img.put_pixel(px as u32, y as u32, Rgb([230, 40, 40]));
            }
        }
        // The rings, where the flight came nearest each.
        for p in &points[1..] {
            let (k, _) = flown
                .iter()
                .enumerate()
                .map(|(k, f)| (k, (f[0] - p[0]).hypot(f[1] - p[1])))
                .fold((0, f32::MAX), |a, b| if b.1 < a.1 { b } else { a });
            let px = (along[k] / total * (iw - 1.0)) as i64;
            let y = ty(p[2]);
            for dy in -3..=3i64 {
                for dx in -1..=1i64 {
                    let (x, yy) = (px + dx, y + dy);
                    if x >= 0 && yy >= 0 && (x as u32) < img.width() && (yy as u32) < img.height() {
                        img.put_pixel(x as u32, yy as u32, Rgb([250, 210, 30]));
                    }
                }
            }
        }
    }
    img
}
