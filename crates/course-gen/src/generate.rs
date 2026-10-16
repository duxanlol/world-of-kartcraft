//! Course candidates on the cost field: circuits round a hub through anchors at varied radii, and
//! sprints between two hubs, each leg found by A* on a coarse copy of the field, then smoothed to
//! a kart's turning circle and checked for crossings, overlaps and blocked ground.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

use glam::Vec2;
use serde::Serialize;

use crate::cost::{Block, CostField};
use crate::geom;
use crate::roads::RoadGraph;
use crate::terrain::ZoneTerrain;

/// Spacing of a candidate's polyline, yards.
pub const STEP: f32 = 3.0;

/// A small deterministic generator (SplitMix64), so a seed reproduces a run.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[lo, hi)`.
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * ((self.next_u64() >> 40) as f32 / (1u64 << 24) as f32)
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n.max(1) as u64) as usize
    }
}

/// A named place a course can start from or run to.
#[derive(Clone, Debug, Serialize)]
pub struct Hub {
    pub name: String,
    pub x: f32,
    pub y: f32,
}

/// The cost field at `factor`× coarser pixels, for routing.
pub struct Planner<'a> {
    pub field: &'a CostField,
    factor: usize,
    w: usize,
    h: usize,
    cost: Vec<f32>,
    /// The fine clearance at each coarse cell (the least of its pixels), yards.
    clear: Vec<f32>,
    /// Connected component of each passable cell (8-connected), `u32::MAX` if impassable.
    comp: Vec<u32>,
    /// Cells in each component.
    comp_size: Vec<usize>,
}

/// Neighbour steps: the 8 kings' and the 8 knights' moves, for straighter paths on a grid.
const STEPS: [(i32, i32); 16] = [
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
    (1, 1),
    (1, -1),
    (-1, 1),
    (-1, -1),
    (2, 1),
    (2, -1),
    (-2, 1),
    (-2, -1),
    (1, 2),
    (1, -2),
    (-1, 2),
    (-1, -2),
];

/// The most a route may turn in one step, radians, and the cost of turning (per radian²).
const MAX_STEP_TURN: f32 = 0.8;
const TURN_WEIGHT: f32 = 15.0;

/// The turn cost from move `a` to move `b`, `None` past [`MAX_STEP_TURN`].
fn turn_table() -> [[Option<f32>; 16]; 16] {
    let mut t = [[None; 16]; 16];
    for (i, &(ac, ar)) in STEPS.iter().enumerate() {
        for (j, &(bc, br)) in STEPS.iter().enumerate() {
            let (a, b) = (
                Vec2::new(ac as f32, ar as f32).normalize(),
                Vec2::new(bc as f32, br as f32).normalize(),
            );
            let d = a.angle_to(b).abs();
            if d <= MAX_STEP_TURN + 1e-3 {
                t[i][j] = Some(TURN_WEIGHT * d * d);
            }
        }
    }
    t
}

/// A routed leg: its points and the headings (move indices) it leaves and arrives on.
pub struct Leg {
    pub points: Vec<Vec2>,
    pub first_dir: usize,
    pub last_dir: usize,
}

/// Coarse cells nearer impassable ground than this (yards, fine clearance) are not routed on.
const MIN_ROUTE_CLEARANCE: f32 = 2.5;

/// The same on a city's streets, routed at the field's own resolution: an alley between crates
/// is a kart and a yard either side.
pub const STREET_ROUTE_CLEARANCE: f32 = 1.0;

impl<'a> Planner<'a> {
    pub fn new(field: &'a CostField, factor: usize) -> Self {
        Self::with_clearance(field, factor, MIN_ROUTE_CLEARANCE)
    }

    /// A planner that routes on cells with `min_clearance` yards of room.
    pub fn with_clearance(field: &'a CostField, factor: usize, min_clearance: f32) -> Self {
        let g = field.grid;
        let (w, h) = (g.width / factor, g.height / factor);
        let mut cost = vec![f32::INFINITY; w * h];
        let mut clear = vec![0.0; w * h];
        for r in 0..h {
            for c in 0..w {
                let (mut sum, mut lo) = (0.0, f32::INFINITY);
                for dr in 0..factor {
                    for dc in 0..factor {
                        let i = (r * factor + dr) * g.width + c * factor + dc;
                        sum += field.cost[i];
                        lo = lo.min(field.clearance[i]);
                    }
                }
                clear[r * w + c] = lo;
                if lo >= min_clearance && sum.is_finite() {
                    cost[r * w + c] = sum / (factor * factor) as f32;
                }
            }
        }
        let comp = components(&cost, w, h);
        let mut comp_size = Vec::new();
        for &k in comp.iter().filter(|&&k| k != u32::MAX) {
            if comp_size.len() <= k as usize {
                comp_size.resize(k as usize + 1, 0);
            }
            comp_size[k as usize] += 1;
        }
        Self {
            field,
            factor,
            w,
            h,
            cost,
            clear,
            comp,
            comp_size,
        }
    }

    /// Square yards of the largest piece of connected open ground.
    pub fn largest_reach(&self) -> f32 {
        self.comp_size.iter().copied().max().unwrap_or(0) as f32 * self.yards() * self.yards()
    }

    /// Square yards of open ground connected to `p`.
    pub fn reach(&self, p: Vec2) -> f32 {
        self.component(p).map_or(0.0, |k| {
            self.comp_size[k as usize] as f32 * self.yards() * self.yards()
        })
    }

    /// The connected component under `p`, if passable.
    pub fn component(&self, p: Vec2) -> Option<u32> {
        let (c, r) = self.cell_of(p)?;
        let k = self.comp[r * self.w + c];
        (k != u32::MAX).then_some(k)
    }

    pub fn len(&self) -> usize {
        self.w * self.h
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn yards(&self) -> f32 {
        self.field.grid.yards_per_px * self.factor as f32
    }

    /// The coarse cell of world `(x, y)`.
    pub fn cell_of(&self, p: Vec2) -> Option<(usize, usize)> {
        let (c, r) = self.field.grid.pixel_of(p.x, p.y);
        let (c, r) = (
            (c / self.factor as f32).floor(),
            (r / self.factor as f32).floor(),
        );
        (c >= 0.0 && r >= 0.0 && (c as usize) < self.w && (r as usize) < self.h)
            .then_some((c as usize, r as usize))
    }

    /// World `(x, y)` of a coarse cell's centre.
    pub fn world_of(&self, c: usize, r: usize) -> Vec2 {
        let f = self.factor as f32;
        let (x, y) = self
            .field
            .grid
            .world_of(c as f32 * f + f / 2.0, r as f32 * f + f / 2.0);
        Vec2::new(x, y)
    }

    fn passable(&self, c: i64, r: i64) -> bool {
        c >= 0
            && r >= 0
            && (c as usize) < self.w
            && (r as usize) < self.h
            && self.cost[r as usize * self.w + c as usize].is_finite()
    }

    /// The best cell within `radius` yards of `p` to anchor a course on: wide open, flat, near
    /// `p`; `None` if nothing there has `min_clear` yards of clearance.
    pub fn snap(&self, p: Vec2, radius: f32, min_clear: f32, comp: Option<u32>) -> Option<Vec2> {
        let (c0, r0) = self.cell_of(p)?;
        let k = (radius / self.yards()).ceil() as i64;
        let mut best: Option<(f32, Vec2)> = None;
        for dr in -k..=k {
            for dc in -k..=k {
                let (c, r) = (c0 as i64 + dc, r0 as i64 + dr);
                if !self.passable(c, r) {
                    continue;
                }
                let i = r as usize * self.w + c as usize;
                let clr = self.clear[i];
                if clr < min_clear || comp.is_some_and(|k| self.comp[i] != k) {
                    continue;
                }
                let q = self.world_of(c as usize, r as usize);
                let d = q.distance(p);
                if d > radius {
                    continue;
                }
                let score = clr.min(30.0) - 0.15 * d - 4.0 * self.cost[i];
                if best.is_none_or(|(b, _)| score > b) {
                    best = Some((score, q));
                }
            }
        }
        best.map(|(_, q)| q)
    }

    /// The nearest routable cell to `p` within `radius` yards, `p`'s own if it is one.
    pub fn nearest_open(&self, p: Vec2, radius: f32) -> Option<Vec2> {
        let (c0, r0) = self.cell_of(p)?;
        let k = (radius / self.yards()).ceil() as i64;
        let mut best: Option<(f32, Vec2)> = None;
        for dr in -k..=k {
            for dc in -k..=k {
                let (c, r) = (c0 as i64 + dc, r0 as i64 + dr);
                if !self.passable(c, r) {
                    continue;
                }
                let q = self.world_of(c as usize, r as usize);
                let d = q.distance(p);
                if d <= radius && best.is_none_or(|(b, _)| d < b) {
                    best = Some((d, q));
                }
            }
        }
        best.map(|(_, q)| q)
    }

    /// The nearest paved cell to `p` within `radius` yards with `min_clear` yards of room, in
    /// component `comp`.
    pub fn snap_road(
        &self,
        p: Vec2,
        radius: f32,
        min_clear: f32,
        comp: Option<u32>,
    ) -> Option<Vec2> {
        let (c0, r0) = self.cell_of(p)?;
        let k = (radius / self.yards()).ceil() as i64;
        let g = self.field.grid;
        let mut best: Option<(f32, Vec2)> = None;
        for dr in -k..=k {
            for dc in -k..=k {
                let (c, r) = (c0 as i64 + dc, r0 as i64 + dr);
                if !self.passable(c, r) {
                    continue;
                }
                let i = r as usize * self.w + c as usize;
                if self.clear[i] < min_clear || comp.is_some_and(|k| self.comp[i] != k) {
                    continue;
                }
                let q = self.world_of(c as usize, r as usize);
                let (pc, pr) = g.pixel_of(q.x, q.y);
                let paved = g
                    .index(pc.floor() as i64, pr.floor() as i64)
                    .is_some_and(|fi| self.field.road[fi] >= 0.5);
                let d = q.distance(p);
                if !paved || d > radius {
                    continue;
                }
                if best.is_none_or(|(b, _)| d < b) {
                    best = Some((d, q));
                }
            }
        }
        best.map(|(_, q)| q)
    }

    /// The cheapest 16-connected path from `a` to `b`, steering: the search state is the cell
    /// and the heading it was entered on, a step may turn at most [`MAX_STEP_TURN`] and pays
    /// [`TURN_WEIGHT`]·Δθ² for it. `start_dir` and `end_dir` (indices into the moves) continue
    /// the legs either side. Each cell's cost is scaled by `extra` (1.0 = as is). Returns the
    /// world points and the first and last move's heading; `None` if `b` cannot be reached.
    pub fn route(
        &self,
        a: Vec2,
        b: Vec2,
        extra: &HashMap<usize, f32>,
        start_dir: Option<usize>,
        end_dir: Option<usize>,
    ) -> Option<Leg> {
        let (ac, ar) = self.cell_of(a)?;
        let (bc, br) = self.cell_of(b)?;
        if !self.passable(ac as i64, ar as i64) || !self.passable(bc as i64, br as i64) {
            return None;
        }
        let nd = STEPS.len();
        let turn = turn_table();
        let (w, n) = (self.w, self.len());
        let start = ar * w + ac;
        let goal = br * w + bc;
        if start == goal {
            return None;
        }
        let mut g = vec![f32::INFINITY; n * nd];
        let mut from = vec![u32::MAX; n * nd];
        let mut heap = BinaryHeap::new();
        let hw = 0.55f32;
        let heur = |i: usize| {
            let (c, r) = ((i % w) as f32, (i / w) as f32);
            hw * (c - bc as f32).hypot(r - br as f32)
        };
        for d in 0..nd {
            if start_dir.is_none_or(|s| s == d) {
                g[start * nd + d] = 0.0;
                heap.push(Reverse((heur(start).to_bits(), start * nd + d)));
            }
        }
        let cell_cost = |i: usize| self.cost[i] * extra.get(&i).copied().unwrap_or(1.0);
        let mut found = None;
        while let Some(Reverse((fbits, st))) = heap.pop() {
            let (i, d) = (st / nd, st % nd);
            if i == goal {
                found = Some(st);
                break;
            }
            let gi = g[st];
            if f32::from_bits(fbits) > gi + heur(i) + 1e-3 {
                continue;
            }
            let (c, r) = ((i % w) as i64, (i / w) as i64);
            let ci = cell_cost(i);
            for (nd2, &(dc, dr)) in STEPS.iter().enumerate() {
                let Some(tp) = turn[d][nd2] else {
                    continue;
                };
                // The very first step from a free start turns for nothing.
                let tp = if i == start && start_dir.is_none() {
                    if nd2 != d {
                        continue;
                    }
                    0.0
                } else {
                    tp
                };
                let (nc, nr) = (c + dc as i64, r + dr as i64);
                if !self.passable(nc, nr) {
                    continue;
                }
                // A knight's move must not jump a blocked cell.
                if dc.abs() == 2 || dr.abs() == 2 {
                    let (mc, mr) = (c + (dc / 2) as i64, r + (dr / 2) as i64);
                    let (oc, or) = (c + dc.signum() as i64, r + dr.signum() as i64);
                    if !self.passable(mc, mr) || !self.passable(oc, or) {
                        continue;
                    }
                }
                let j = nr as usize * w + nc as usize;
                let len = ((dc * dc + dr * dr) as f32).sqrt();
                let mut ng = gi + len * 0.5 * (ci + cell_cost(j)) + tp;
                if j == goal {
                    if let Some(e) = end_dir {
                        match turn[nd2][e] {
                            Some(t) => ng += t,
                            None => continue,
                        }
                    }
                }
                let sj = j * nd + nd2;
                if ng < g[sj] {
                    g[sj] = ng;
                    from[sj] = st as u32;
                    heap.push(Reverse(((ng + heur(j)).to_bits(), sj)));
                }
            }
        }
        let end = found?;
        let mut states = vec![end];
        let mut at = end;
        while from[at] != u32::MAX {
            at = from[at] as usize;
            states.push(at);
        }
        states.reverse();
        let first_dir = states.get(1).map_or(end % nd, |s| s % nd);
        Some(Leg {
            points: states
                .iter()
                .map(|&s| self.world_of((s / nd) % w, (s / nd) / w))
                .collect(),
            first_dir,
            last_dir: end % nd,
        })
    }

    /// Scale the cost within `radius` yards of `pts` by `factor`, for the legs laid after them.
    pub fn repel(&self, extra: &mut HashMap<usize, f32>, pts: &[Vec2], radius: f32, factor: f32) {
        let k = (radius / self.yards()).ceil() as i64;
        let y = self.yards();
        for (n, p) in pts.iter().enumerate() {
            if n % 3 != 0 {
                continue;
            }
            let Some((c0, r0)) = self.cell_of(*p) else {
                continue;
            };
            for dr in -k..=k {
                for dc in -k..=k {
                    if ((dr * dr + dc * dc) as f32).sqrt() * y > radius {
                        continue;
                    }
                    let (c, r) = (c0 as i64 + dc, r0 as i64 + dr);
                    if c < 0 || r < 0 || c as usize >= self.w || r as usize >= self.h {
                        continue;
                    }
                    let e = extra.entry(r as usize * self.w + c as usize).or_insert(1.0);
                    *e = e.max(factor);
                }
            }
        }
    }
}

/// 8-connected components of the finite cells of `cost`.
fn components(cost: &[f32], w: usize, h: usize) -> Vec<u32> {
    let mut comp = vec![u32::MAX; w * h];
    let mut next = 0u32;
    let mut stack = Vec::new();
    for start in 0..w * h {
        if comp[start] != u32::MAX || !cost[start].is_finite() {
            continue;
        }
        comp[start] = next;
        stack.push(start);
        while let Some(i) = stack.pop() {
            let (c, r) = ((i % w) as i64, (i / w) as i64);
            for dr in -1..=1 {
                for dc in -1..=1 {
                    let (nc, nr) = (c + dc, r + dr);
                    if nc < 0 || nr < 0 || nc as usize >= w || nr as usize >= h {
                        continue;
                    }
                    let j = nr as usize * w + nc as usize;
                    if comp[j] == u32::MAX && cost[j].is_finite() {
                        comp[j] = next;
                        stack.push(j);
                    }
                }
            }
        }
        next += 1;
    }
    comp
}

/// Circuit or sprint.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Circuit,
    Sprint,
}

/// A course before it is measured: an evenly spaced polyline in world yards.
#[derive(Clone, Debug)]
pub struct Candidate {
    pub kind: Kind,
    pub name: String,
    pub pts: Vec<Vec2>,
    pub closed: bool,
    pub seed: u64,
}

/// Choices for [`generate`].
#[derive(Clone, Debug)]
pub struct GenParams {
    pub attempts: usize,
    pub sprint_share: f32,
    pub circuit_len: (f32, f32),
    pub sprint_len: (f32, f32),
    /// Bends are eased toward this radius (yards) where the ground allows ...
    pub min_turn_radius: f32,
    /// ... and a line with any bend tighter than this is rejected.
    pub min_bend_radius: f32,
    /// Two parts of a course this far apart along it must stay `min_separation` apart.
    pub min_separation: f32,
    pub threads: usize,
    pub seed: u64,
}

impl Default for GenParams {
    fn default() -> Self {
        Self {
            attempts: 1500,
            sprint_share: 0.2,
            // 1.0–3.5 km a lap, 1.5–4 km a sprint, in yards.
            circuit_len: (1094.0, 3828.0),
            sprint_len: (1640.0, 4374.0),
            min_turn_radius: 24.0,
            min_bend_radius: 6.0,
            min_separation: 35.0,
            threads: 8,
            seed: 1,
        }
    }
}

/// The places to build round: each subzone's middle, the road junctions and ends, snapped to
/// open ground with at least `min_reach` square yards of connected open ground round it.
pub fn hubs(t: &ZoneTerrain, roads: &RoadGraph, planner: &Planner, min_reach: f32) -> Vec<Hub> {
    let g = t.grid;
    let mut sums: HashMap<u32, (f64, f64, usize)> = HashMap::new();
    for (i, &a) in t.area.iter().enumerate() {
        if planner.field.block[i] != Block::Clear || a == 0 {
            continue;
        }
        let (x, y) = g.world_of((i % g.width) as f32 + 0.5, (i / g.width) as f32 + 0.5);
        let e = sums.entry(a).or_default();
        e.0 += f64::from(x);
        e.1 += f64::from(y);
        e.2 += 1;
    }
    let mut out = Vec::new();
    let roomy = |q: Vec2| planner.reach(q) >= min_reach;
    let mut areas: Vec<_> = sums.into_iter().collect();
    areas.sort_by_key(|(id, _)| *id);
    for (id, (sx, sy, n)) in areas {
        // A subzone of under ~0.01 km² is a building or a pond.
        if n < 2500 || id == t.area_id {
            continue;
        }
        let p = Vec2::new((sx / n as f64) as f32, (sy / n as f64) as f32);
        if let Some(q) = planner.snap(p, 150.0, 8.0, None).filter(|&q| roomy(q)) {
            let name = t
                .area_names
                .get(&id)
                .cloned()
                .unwrap_or_else(|| format!("area {id}"));
            out.push(Hub {
                name,
                x: q.x,
                y: q.y,
            });
        }
    }
    // Open ground on a lattice, so courses start everywhere and not only at named places.
    let spacing = 380.0;
    let (gx0, gy0) = g.world_of(0.0, 0.0);
    let (gx1, gy1) = g.world_of(g.width as f32, g.height as f32);
    let mut x = gx1 + spacing / 2.0;
    while x < gx0 {
        let mut y = gy1 + spacing / 2.0;
        while y < gy0 {
            let p = Vec2::new(x, y);
            if let Some(q) = planner.snap(p, 90.0, 10.0, None).filter(|&q| roomy(q)) {
                let near = t
                    .area_at(q.x, q.y)
                    .and_then(|a| t.area_names.get(&a).cloned())
                    .unwrap_or_else(|| t.zone.clone());
                out.push(Hub {
                    name: format!("{near} fields"),
                    x: q.x,
                    y: q.y,
                });
            }
            y += spacing;
        }
        x += spacing;
    }
    for n in &roads.nodes {
        let p = Vec2::new(n.x, n.y);
        if let Some(q) = planner.snap(p, 60.0, 6.0, None).filter(|&q| roomy(q)) {
            let near = t
                .area_at(q.x, q.y)
                .and_then(|a| t.area_names.get(&a).cloned())
                .unwrap_or_else(|| t.zone.clone());
            let what = if n.degree >= 3 {
                "junction"
            } else {
                "road end"
            };
            out.push(Hub {
                name: format!("{near} {what}"),
                x: q.x,
                y: q.y,
            });
        }
    }
    out
}

/// Generate up to `p.attempts` tries' worth of valid candidates, with a tally of why the rest
/// failed.
pub fn generate(
    planner: &Planner,
    hubs: &[Hub],
    p: &GenParams,
) -> (Vec<Candidate>, HashMap<&'static str, usize>) {
    if hubs.is_empty() {
        return (Vec::new(), HashMap::new());
    }
    let per = p.attempts.div_ceil(p.threads.max(1));
    let results: Vec<Result<Candidate, &'static str>> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..p.threads.max(1))
            .map(|t| {
                s.spawn(move || {
                    let mut found = Vec::new();
                    for k in 0..per {
                        let seed = p.seed.wrapping_mul(1_000_003) + (t * per + k) as u64;
                        let mut rng = Rng::new(seed);
                        let sprint = rng.range(0.0, 1.0) < p.sprint_share;
                        let c = if sprint {
                            try_sprint(planner, hubs, p, &mut rng)
                        } else {
                            try_circuit(planner, hubs, p, &mut rng)
                        };
                        found.push(c.map(|mut c| {
                            c.seed = seed;
                            c
                        }));
                    }
                    found
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    });
    let mut out = Vec::new();
    let mut why: HashMap<&'static str, usize> = HashMap::new();
    for r in results {
        match r {
            Ok(c) => out.push(c),
            Err(e) => *why.entry(e).or_default() += 1,
        }
    }
    out.sort_by_key(|c| c.seed);
    (out, why)
}

fn try_circuit(
    planner: &Planner,
    hubs: &[Hub],
    p: &GenParams,
    rng: &mut Rng,
) -> Result<Candidate, &'static str> {
    let hub = &hubs[rng.below(hubs.len())];
    let centre = Vec2::new(hub.x, hub.y);
    let comp = planner.component(centre);
    let n = 3 + rng.below(4);
    let base = rng.range(200.0, 520.0);
    let a0 = rng.range(0.0, std::f32::consts::TAU);
    let sector = std::f32::consts::TAU / n as f32;
    // The hub itself sits off-centre in the loop: shift the ring's centre a little.
    let shift =
        Vec2::from_angle(rng.range(0.0, std::f32::consts::TAU)) * base * rng.range(0.0, 0.6);
    let mut anchors = Vec::new();
    for i in 0..n {
        let a = a0 + sector * i as f32 + rng.range(-0.3, 0.3) * sector;
        let r = base * rng.range(0.55, 1.45);
        let target = centre + shift + Vec2::from_angle(a) * r;
        let paved = if rng.range(0.0, 1.0) < 0.4 {
            planner.snap_road(target, 160.0, 4.0, comp)
        } else {
            None
        };
        if let Some(q) = paved.or_else(|| planner.snap(target, 120.0, 6.0, comp)) {
            anchors.push(q);
        }
    }
    if anchors.len() < 3 {
        return Err("too few anchors");
    }
    let mut extra = HashMap::new();
    let mut raw: Vec<Vec2> = Vec::new();
    let (mut first, mut last): (Option<usize>, Option<usize>) = (None, None);
    for i in 0..anchors.len() {
        let (a, b) = (anchors[i], anchors[(i + 1) % anchors.len()]);
        let closing = if i + 1 == anchors.len() { first } else { None };
        let routed = planner
            .route(a, b, &extra, last, closing)
            .ok_or("no route")?;
        first = first.or(Some(routed.first_dir));
        last = Some(routed.last_dir);
        let leg = routed.points;
        // Keep later legs off this one, but not off its far end, where the next leg starts.
        let keep = leg
            .iter()
            .copied()
            .filter(|q| q.distance(b) > 60.0 && q.distance(a) > 60.0)
            .collect::<Vec<_>>();
        planner.repel(&mut extra, &keep, 40.0, 8.0);
        raw.extend_from_slice(&leg[..leg.len() - 1]);
    }
    let pts = finish(planner.field, &raw, true, p, &OPEN_GROUND)?;
    let len = geom::length(&pts, true);
    if len < p.circuit_len.0 {
        return Err("too short");
    }
    if len > p.circuit_len.1 {
        return Err("too long");
    }
    Ok(Candidate {
        kind: Kind::Circuit,
        name: format!("{} circuit", hub.name),
        pts,
        closed: true,
        seed: 0,
    })
}

fn try_sprint(
    planner: &Planner,
    hubs: &[Hub],
    p: &GenParams,
    rng: &mut Rng,
) -> Result<Candidate, &'static str> {
    let a = &hubs[rng.below(hubs.len())];
    let b = &hubs[rng.below(hubs.len())];
    let (pa, pb) = (Vec2::new(a.x, a.y), Vec2::new(b.x, b.y));
    let d = pa.distance(pb);
    if !(900.0..=3200.0).contains(&d) {
        return Err("sprint ends too near or far");
    }
    let comp = planner.component(pa);
    if comp != planner.component(pb) {
        return Err("sprint ends not connected");
    }
    let mid = (pa + pb) / 2.0 + (pb - pa).perp().normalize() * d * rng.range(-0.35, 0.35);
    let via = planner.snap(mid, 150.0, 6.0, comp).ok_or("no via")?;
    let mut extra = HashMap::new();
    let first = planner
        .route(pa, via, &extra, None, None)
        .ok_or("no route")?;
    let arrive = first.last_dir;
    let first = first.points;
    planner.repel(
        &mut extra,
        &first
            .iter()
            .copied()
            .filter(|q| q.distance(via) > 60.0)
            .collect::<Vec<_>>(),
        40.0,
        8.0,
    );
    let second = planner
        .route(via, pb, &extra, Some(arrive), None)
        .ok_or("no route")?
        .points;
    let mut raw = first;
    raw.extend_from_slice(&second[1..]);
    let pts = finish(planner.field, &raw, false, p, &OPEN_GROUND)?;
    let len = geom::length(&pts, false);
    if len < p.sprint_len.0 || len > p.sprint_len.1 {
        return Err("sprint length");
    }
    Ok(Candidate {
        kind: Kind::Sprint,
        name: format!("{} to {}", a.name, b.name),
        pts,
        closed: false,
        seed: 0,
    })
}

/// How close to obstacles [`finish`] lets a line go.
struct Keep {
    /// A smoothed point keeps this much clearance (yards) unless it had less to begin with.
    clearance: f32,
    /// The share of the line's points that may brush an obstacle's margin.
    margin_share: f32,
}

/// Open ground: the line may brush past a tree now and then.
const OPEN_GROUND: Keep = Keep {
    clearance: 2.5,
    margin_share: 0.01,
};
/// A city street: walls a raster's staircase and carts on both kerbs, so the line keeps further
/// off them where it can, and never crosses a cart.
const STREETS: Keep = Keep {
    clearance: 4.5,
    margin_share: 0.0,
};

/// Smooth a raw grid path into a course line, or reject it.
fn finish(
    field: &CostField,
    raw: &[Vec2],
    closed: bool,
    p: &GenParams,
    keep: &Keep,
) -> Result<Vec<Vec2>, &'static str> {
    if raw.len() < 8 {
        return Err("degenerate");
    }
    let pts = smoothed(field, raw, closed, p, keep);
    let tightest = geom::curvature(&pts, closed, 3)
        .iter()
        .fold(0.0f32, |m, k| m.max(k.abs()));
    if tightest > 1.0 / p.min_bend_radius {
        return Err("bend too tight");
    }
    if pts.len() < 20 {
        return Err("degenerate");
    }
    // Blocked ground under the line: none of water, steep or outside, and only a brush with an
    // obstacle's margin.
    let mut margin = 0usize;
    for q in &pts {
        match field.block_at(q.x, q.y) {
            Block::Clear => {}
            Block::Obstacle => margin += 1,
            Block::Water => return Err("smoothed into water"),
            Block::Steep => return Err("smoothed onto steep ground"),
            Block::Outside => return Err("smoothed out of the zone"),
        }
    }
    if margin as f32 > keep.margin_share * pts.len() as f32 {
        return Err("smoothed into obstacles");
    }
    if geom::self_intersects(&pts, closed) {
        return Err("crosses itself");
    }
    if geom::min_separation(&pts, closed, STEP, 120.0) < p.min_separation {
        return Err("runs alongside itself");
    }
    Ok(pts)
}

/// A raw grid path pulled taut, its turns opened to a kart's circle, and respaced.
fn smoothed(
    field: &CostField,
    raw: &[Vec2],
    closed: bool,
    p: &GenParams,
    keep: &Keep,
) -> Vec<Vec2> {
    let ok = |q: Vec2| field.clearance_at(q.x, q.y) >= keep.clearance;
    let mut pts = geom::resample(raw, closed, STEP);
    elastic(&mut pts, closed, 80, &ok);
    relax_turns(&mut pts, closed, p.min_turn_radius, &ok);
    geom::resample(&pts, closed, STEP)
}

/// Pull the line taut and smooth like an elastic band, `iters` rounds, never moving a point onto
/// ground `ok` refuses; respaced every few rounds so points do not bunch.
pub fn elastic(pts: &mut Vec<Vec2>, closed: bool, iters: usize, ok: &dyn Fn(Vec2) -> bool) {
    for it in 0..iters {
        let n = pts.len();
        if n < 9 {
            return;
        }
        let old = pts.clone();
        for i in 0..n {
            if !closed && (i < 2 || i + 2 >= n) {
                continue;
            }
            let at = |d: isize| old[geom::wrap(i as isize + d, n, closed)];
            let target = (at(-2) + at(-1) + at(1) + at(2)) * 0.25;
            let q = old[i].lerp(target, 0.5);
            if ok(q) {
                pts[i] = q;
            }
        }
        if it % 8 == 7 {
            let even = geom::resample(pts, closed, STEP);
            // Respacing cuts chords; keep the old spacing where it would cut a corner.
            if even.iter().all(|&q| ok(q)) {
                *pts = even;
            }
        }
    }
}

/// Ease every bend tighter than `min_radius` by local averaging until none is left (or a bound),
/// keeping to ground `ok` allows.
pub fn relax_turns(pts: &mut [Vec2], closed: bool, min_radius: f32, ok: &dyn Fn(Vec2) -> bool) {
    let n = pts.len();
    if n < 9 {
        return;
    }
    for _ in 0..120 {
        let k = geom::curvature(pts, closed, 3);
        let tight: Vec<usize> = (0..n).filter(|&i| k[i].abs() * min_radius > 1.0).collect();
        if tight.is_empty() {
            return;
        }
        let mut mark = vec![false; n];
        for &i in &tight {
            for d in -4isize..=4 {
                let j = i as isize + d;
                if !closed && (j <= 0 || j >= n as isize - 1) {
                    continue;
                }
                mark[geom::wrap(j, n, closed)] = true;
            }
        }
        let old = pts.to_vec();
        for i in 0..n {
            if mark[i] {
                let a = old[geom::wrap(i as isize - 1, n, closed)];
                let b = old[geom::wrap(i as isize + 1, n, closed)];
                let q = old[i] * 0.4 + (a + b) * 0.3;
                if ok(q) {
                    pts[i] = q;
                }
            }
        }
    }
}

/// Fraction of `a`'s points within `tol` yards of some point of `b`.
pub fn overlap(a: &[Vec2], b: &[Vec2], tol: f32) -> f32 {
    let cell = tol;
    let mut grid: HashMap<(i32, i32), Vec<Vec2>> = HashMap::new();
    for q in b {
        grid.entry(((q.x / cell).floor() as i32, (q.y / cell).floor() as i32))
            .or_default()
            .push(*q);
    }
    let near = a
        .iter()
        .filter(|q| {
            let (cx, cy) = ((q.x / cell).floor() as i32, (q.y / cell).floor() as i32);
            (-1..=1).any(|dx| {
                (-1..=1).any(|dy| {
                    grid.get(&(cx + dx, cy + dy))
                        .is_some_and(|v| v.iter().any(|r| r.distance(**q) <= tol))
                })
            })
        })
        .count();
    near as f32 / a.len().max(1) as f32
}

/// Whether two courses are the same course near enough.
pub fn similar(a: &Candidate, b: &Candidate) -> bool {
    (overlap(&a.pts, &b.pts, 30.0) + overlap(&b.pts, &a.pts, 30.0)) / 2.0 > 0.55
}

/// Rotate a closed course so it starts on its straightest open stretch, preferring road.
pub fn place_start(c: &mut Candidate, t: &ZoneTerrain, field: &CostField) {
    if !c.closed {
        return;
    }
    let n = c.pts.len();
    let k = geom::smooth(&geom::curvature(&c.pts, true, 4), true, 2);
    let span = (60.0 / STEP) as isize;
    let mut best = (f32::INFINITY, 0usize);
    for i in 0..n {
        let bend = (-span..=span)
            .map(|d| k[geom::wrap(i as isize + d, n, true)].abs())
            .fold(0.0, f32::max);
        let q = c.pts[i];
        let road = t.road_at(q.x, q.y).unwrap_or(0.0);
        let clr = field.clearance_at(q.x, q.y).min(15.0);
        let score = bend * 60.0 - 0.6 * road - 0.03 * clr;
        if score < best.0 {
            best = (score, i);
        }
    }
    c.pts.rotate_left(best.1);
}

/// A street circuit's line is routed between points this far apart along its waypoints' polygon.
const STREET_ANCHOR_EVERY: f32 = 30.0;
/// The most yards of line a doubled-back leg can loop over before it meets itself again.
const STREET_LOOP: f32 = 60.0;
/// How many unreachable anchors in a row a waypoint circuit's leg passes over.
const WAYPOINT_SKIP: usize = 2;

/// A circuit through a city's streets by the waypoints `via` (WoW `x, y`, in driving order), for a
/// map whose ground is its streets ([`ZoneTerrain::wmo_ground`]): too narrow for [`generate`]'s
/// open-ground hubs, and its street mask's centrelines break at every cart, so the streets are
/// named and the line found. The waypoints' polygon gives anchors every [`STREET_ANCHOR_EVERY`]
/// yards, each on the nearest routable cell; the legs between are routed on the field round the
/// carts and statues, little loops cut, and the line smoothed as a generated circuit's is
/// ([`finish`]). It starts at the first waypoint.
pub fn street_circuit(
    planner: &Planner,
    via: &[Vec2],
    p: &GenParams,
) -> Result<Candidate, &'static str> {
    waypoint_circuit(planner, via, p, &STREETS)
}

/// A designed circuit on open ground by the waypoints `via` (WoW `x, y`, in driving order), routed
/// and smoothed as [`street_circuit`]'s is, but keeping a generated line's room round trees.
pub fn designed_circuit(
    planner: &Planner,
    via: &[Vec2],
    p: &GenParams,
) -> Result<Candidate, &'static str> {
    waypoint_circuit(planner, via, p, &OPEN_GROUND)
}

fn waypoint_circuit(
    planner: &Planner,
    via: &[Vec2],
    p: &GenParams,
    keep: &Keep,
) -> Result<Candidate, &'static str> {
    if via.len() < 3 {
        return Err("too few anchors");
    }
    let anchors: Vec<Vec2> = geom::resample(via, true, STREET_ANCHOR_EVERY)
        .iter()
        .filter_map(|&q| planner.nearest_open(q, 8.0))
        .collect();
    if anchors.len() < 4 {
        return Err("too few anchors");
    }
    // Each leg on its own: a heading carried into a narrow alley leaves no room to turn, and the
    // smoothing after joins the legs.
    let extra = HashMap::new();
    let mut raw: Vec<Vec2> = Vec::new();
    // An anchor snapped into a pocket no route reaches (open ground walled in by trees) is passed
    // over: the leg goes on to the next one, up to [`WAYPOINT_SKIP`] of them.
    let n = anchors.len();
    let mut i = 0;
    while i < n {
        let a = anchors[i];
        let mut j = i + 1;
        loop {
            let b = anchors[j % n];
            if a.distance(b) < 1.0 {
                break;
            }
            if let Some(leg) = planner.route(a, b, &extra, None, None) {
                raw.extend_from_slice(&leg.points[..leg.points.len() - 1]);
                break;
            }
            if j == n || j > i + WAYPOINT_SKIP {
                return Err("no route");
            }
            j += 1;
        }
        i = j;
    }
    let raw = cut_loops(&raw, STREET_LOOP);
    let mut pts = match finish(planner.field, &raw, true, p, keep) {
        Ok(pts) => pts,
        Err(e) => {
            // Where it went wrong, for whoever is laying the waypoints.
            let pts = smoothed(planner.field, &raw, true, p, keep);
            let curv = geom::curvature(&pts, true, 3);
            if let Some((i, k)) = curv
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            {
                eprintln!(
                    "waypoint circuit: {e}; tightest bend {:.1} yd radius at {:.0},{:.0}",
                    1.0 / k.abs().max(1e-6),
                    pts[i].x,
                    pts[i].y
                );
            }
            return Err(e);
        }
    };
    // The line starts at the first waypoint.
    if let Some(at) = pts
        .iter()
        .enumerate()
        .min_by(|a, b| a.1.distance(via[0]).total_cmp(&b.1.distance(via[0])))
        .map(|(i, _)| i)
    {
        pts.rotate_left(at);
    }
    Ok(Candidate {
        kind: Kind::Circuit,
        name: "street circuit".into(),
        pts,
        closed: true,
        seed: 0,
    })
}

/// `pts` with every little loop cut out: where the line comes back within a yard and a half of a
/// point it passed at most `span` yards before, the stretch between goes. Two anchors routed past
/// each other make such a loop, which no smoothing straightens.
fn cut_loops(pts: &[Vec2], span: f32) -> Vec<Vec2> {
    let mut out: Vec<Vec2> = Vec::with_capacity(pts.len());
    for &q in pts {
        let mut walked = 0.0;
        let mut cut = None;
        for k in (0..out.len()).rev() {
            if k + 1 < out.len() {
                walked += out[k].distance(out[k + 1]);
            }
            if walked > span {
                break;
            }
            if walked > 4.0 && out[k].distance(q) < 1.5 {
                cut = Some(k);
            }
        }
        if let Some(k) = cut {
            out.truncate(k + 1);
        } else {
            out.push(q);
        }
    }
    out
}
