//! The road network: a thresholded, cleaned road mask, thinned to a one-pixel skeleton
//! (Zhang–Suen) and traced into a graph of junction and end nodes joined by polylines, then
//! carried into world yards with a width (twice the distance to the mask's edge) and a height.

use std::collections::VecDeque;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::terrain::ZoneTerrain;

/// A row-major boolean raster.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mask {
    pub width: usize,
    pub height: usize,
    pub data: Vec<bool>,
}

/// The 8-neighbourhood, clockwise from north: `(dc, dr)`.
const N8: [(i64, i64); 8] = [
    (0, -1),
    (1, -1),
    (1, 0),
    (1, 1),
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
];

impl Mask {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            data: vec![false; width * height],
        }
    }

    /// A mask from rows of text, `#` set and anything else clear (for tests and sketches).
    pub fn from_ascii(rows: &[&str]) -> Self {
        let height = rows.len();
        let width = rows.iter().map(|r| r.len()).max().unwrap_or(0);
        let mut m = Self::new(width, height);
        for (r, line) in rows.iter().enumerate() {
            for (c, ch) in line.bytes().enumerate() {
                m.data[r * width + c] = ch == b'#';
            }
        }
        m
    }

    pub fn get(&self, c: i64, r: i64) -> bool {
        c >= 0
            && r >= 0
            && (c as usize) < self.width
            && (r as usize) < self.height
            && self.data[r as usize * self.width + c as usize]
    }

    pub fn count(&self) -> usize {
        self.data.iter().filter(|&&b| b).count()
    }

    fn disk(radius: i64) -> Vec<(i64, i64)> {
        let mut out = Vec::new();
        for dr in -radius..=radius {
            for dc in -radius..=radius {
                if dc * dc + dr * dr <= radius * radius + radius / 2 {
                    out.push((dc, dr));
                }
            }
        }
        out
    }

    /// Set where any pixel within a disk of `radius` is set.
    pub fn dilate(&self, radius: i64) -> Self {
        let disk = Self::disk(radius);
        let mut out = Self::new(self.width, self.height);
        for r in 0..self.height as i64 {
            for c in 0..self.width as i64 {
                out.data[r as usize * self.width + c as usize] =
                    disk.iter().any(|&(dc, dr)| self.get(c + dc, r + dr));
            }
        }
        out
    }

    /// Set where every pixel within a disk of `radius` is set (off the raster counts as set).
    pub fn erode(&self, radius: i64) -> Self {
        let disk = Self::disk(radius);
        let (w, h) = (self.width as i64, self.height as i64);
        let mut out = Self::new(self.width, self.height);
        for r in 0..h {
            for c in 0..w {
                out.data[r as usize * self.width + c as usize] = disk.iter().all(|&(dc, dr)| {
                    let (cc, rr) = (c + dc, r + dr);
                    !(0..w).contains(&cc) || !(0..h).contains(&rr) || self.get(cc, rr)
                });
            }
        }
        out
    }

    /// Dilate then erode: bridges gaps narrower than `2·radius`.
    pub fn close(&self, radius: i64) -> Self {
        self.dilate(radius).erode(radius)
    }

    /// Erode then dilate: drops specks and threads narrower than `2·radius`.
    pub fn open(&self, radius: i64) -> Self {
        self.erode(radius).dilate(radius)
    }

    /// 8-connected components of pixels equal to `value`, as lists of flat indices.
    fn components(&self, value: bool) -> Vec<Vec<usize>> {
        let mut seen = vec![false; self.data.len()];
        let mut out = Vec::new();
        for start in 0..self.data.len() {
            if seen[start] || self.data[start] != value {
                continue;
            }
            let mut comp = Vec::new();
            let mut queue = VecDeque::from([start]);
            seen[start] = true;
            while let Some(i) = queue.pop_front() {
                comp.push(i);
                let (c, r) = ((i % self.width) as i64, (i / self.width) as i64);
                for (dc, dr) in N8 {
                    let (cc, rr) = (c + dc, r + dr);
                    if cc < 0 || rr < 0 || cc >= self.width as i64 || rr >= self.height as i64 {
                        continue;
                    }
                    let j = rr as usize * self.width + cc as usize;
                    if !seen[j] && self.data[j] == value {
                        seen[j] = true;
                        queue.push_back(j);
                    }
                }
            }
            out.push(comp);
        }
        out
    }

    /// Clear set components smaller than `min_px` pixels.
    pub fn remove_small(&self, min_px: usize) -> Self {
        let mut out = self.clone();
        for comp in self.components(true) {
            if comp.len() < min_px {
                for i in comp {
                    out.data[i] = false;
                }
            }
        }
        out
    }

    /// Set clear holes smaller than `max_px` pixels that do not touch the border.
    pub fn fill_holes(&self, max_px: usize) -> Self {
        let mut out = self.clone();
        for comp in self.components(false) {
            let touches = comp.iter().any(|&i| {
                let (c, r) = (i % self.width, i / self.width);
                c == 0 || r == 0 || c + 1 == self.width || r + 1 == self.height
            });
            if !touches && comp.len() < max_px {
                for i in comp {
                    out.data[i] = true;
                }
            }
        }
        out
    }

    /// Zhang–Suen thinning to an 8-connected one-pixel skeleton.
    pub fn zhang_suen(&self) -> Self {
        let mut m = self.clone();
        let w = self.width as i64;
        loop {
            let mut changed = false;
            for pass in 0..2 {
                let mut clear = Vec::new();
                for i in 0..m.data.len() {
                    if !m.data[i] {
                        continue;
                    }
                    let (c, r) = ((i as i64) % w, (i as i64) / w);
                    let p: [bool; 8] = std::array::from_fn(|k| m.get(c + N8[k].0, r + N8[k].1));
                    let b = p.iter().filter(|&&v| v).count();
                    if !(2..=6).contains(&b) {
                        continue;
                    }
                    let a = (0..8).filter(|&k| !p[k] && p[(k + 1) % 8]).count();
                    if a != 1 {
                        continue;
                    }
                    // p[0] N, p[2] E, p[4] S, p[6] W.
                    let ok = if pass == 0 {
                        !(p[0] && p[2] && p[4]) && !(p[2] && p[4] && p[6])
                    } else {
                        !(p[0] && p[2] && p[6]) && !(p[0] && p[4] && p[6])
                    };
                    if ok {
                        clear.push(i);
                    }
                }
                changed |= !clear.is_empty();
                for i in clear {
                    m.data[i] = false;
                }
            }
            if !changed {
                return m;
            }
        }
    }

    /// Euclidean distance (pixels) from each set pixel to the nearest clear one; 0 on clear pixels.
    /// Felzenszwalb–Huttenlocher's separable squared transform; off the raster counts as clear.
    pub fn distance_transform(&self) -> Vec<f32> {
        let (w, h) = (self.width, self.height);
        let inf = 1.0e12f64;
        let mut f = vec![0.0f64; w * h];
        for (i, v) in f.iter_mut().enumerate() {
            *v = if self.data[i] { inf } else { 0.0 };
        }
        // Pad by one clear pixel on each side so the border counts as background.
        let mut col = vec![0.0; h + 2];
        for c in 0..w {
            col[0] = 0.0;
            col[h + 1] = 0.0;
            for r in 0..h {
                col[r + 1] = f[r * w + c];
            }
            let d = edt_1d(&col);
            for r in 0..h {
                f[r * w + c] = d[r + 1];
            }
        }
        let mut row = vec![0.0; w + 2];
        for r in 0..h {
            row[0] = 0.0;
            row[w + 1] = 0.0;
            row[1..w + 1].copy_from_slice(&f[r * w..r * w + w]);
            let d = edt_1d(&row);
            for c in 0..w {
                f[r * w + c] = d[c + 1];
            }
        }
        f.iter().map(|&v| v.sqrt() as f32).collect()
    }
}

/// The 1-D squared distance transform of sampled function `f` (lower envelope of parabolas).
fn edt_1d(f: &[f64]) -> Vec<f64> {
    let n = f.len();
    let mut d = vec![0.0; n];
    let mut v = vec![0usize; n];
    let mut z = vec![0.0f64; n + 1];
    let mut k = 0usize;
    v[0] = 0;
    z[0] = f64::NEG_INFINITY;
    z[1] = f64::INFINITY;
    for q in 1..n {
        // z[0] is −∞, so the walk back stops at k = 0.
        let s = loop {
            let p = v[k];
            let s =
                ((f[q] + (q * q) as f64) - (f[p] + (p * p) as f64)) / (2.0 * (q as f64 - p as f64));
            if s <= z[k] {
                k -= 1;
            } else {
                break s;
            }
        };
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = f64::INFINITY;
    }
    k = 0;
    for (q, out) in d.iter_mut().enumerate() {
        while z[k + 1] < q as f64 {
            k += 1;
        }
        let p = v[k];
        *out = (q as f64 - p as f64).powi(2) + f[p];
    }
    d
}

/// A graph in pixel coordinates: `(col, row)` with pixel centres at integer + 0.5.
#[derive(Clone, Debug, Default)]
pub struct PixelGraph {
    pub nodes: Vec<[f32; 2]>,
    pub edges: Vec<PixelEdge>,
}

/// An edge's polyline runs from node `a`'s position to node `b`'s, both ends included.
#[derive(Clone, Debug)]
pub struct PixelEdge {
    pub a: usize,
    pub b: usize,
    pub points: Vec<[f32; 2]>,
}

impl PixelEdge {
    pub fn length(&self) -> f32 {
        polyline_length(&self.points)
    }
}

fn polyline_length(p: &[[f32; 2]]) -> f32 {
    p.windows(2)
        .map(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt())
        .sum()
}

fn centre(i: usize, w: usize) -> [f32; 2] {
    [(i % w) as f32 + 0.5, (i / w) as f32 + 0.5]
}

/// Walk a run of two-neighbour pixels from node pixel `origin` through `first` to the next node
/// pixel, adding the edge; a run that closes on itself without reaching a node is dropped.
fn trace(
    g: &mut PixelGraph,
    nbrs: &dyn Fn(usize) -> Vec<usize>,
    width: usize,
    label: &[usize],
    visited: &mut [bool],
    origin: usize,
    first: usize,
) {
    let from = label[origin];
    let mut pts = vec![g.nodes[from], centre(first, width)];
    visited[first] = true;
    let (mut prev, mut cur) = (origin, first);
    loop {
        let Some(next) = nbrs(cur).into_iter().find(|&j| j != prev) else {
            return;
        };
        if label[next] != usize::MAX {
            pts.push(g.nodes[label[next]]);
            g.edges.push(PixelEdge {
                a: from,
                b: label[next],
                points: pts,
            });
            return;
        }
        if visited[next] {
            return;
        }
        visited[next] = true;
        pts.push(centre(next, width));
        prev = cur;
        cur = next;
    }
}

impl PixelGraph {
    /// Trace a one-pixel skeleton: every pixel without exactly two set neighbours is part of a
    /// node (8-connected clusters of them merge into one node at their centroid), and each run of
    /// two-neighbour pixels between nodes is an edge. A ring with no node gets one.
    pub fn from_skeleton(skel: &Mask) -> Self {
        let (w, h) = (skel.width, skel.height);
        let nbrs = |i: usize| -> Vec<usize> {
            let (c, r) = ((i % w) as i64, (i / w) as i64);
            N8.iter()
                .filter_map(|&(dc, dr)| {
                    let (cc, rr) = (c + dc, r + dr);
                    skel.get(cc, rr).then(|| rr as usize * w + cc as usize)
                })
                .collect()
        };
        let degree: Vec<u8> = (0..w * h)
            .map(|i| if skel.data[i] { nbrs(i).len() as u8 } else { 0 })
            .collect();
        let is_node_px = |i: usize| skel.data[i] && degree[i] != 2;

        // Cluster node pixels.
        let mut label = vec![usize::MAX; w * h];
        let mut g = PixelGraph::default();
        for start in 0..w * h {
            if !is_node_px(start) || label[start] != usize::MAX {
                continue;
            }
            let id = g.nodes.len();
            let mut queue = VecDeque::from([start]);
            label[start] = id;
            let (mut sx, mut sy, mut n) = (0.0, 0.0, 0.0);
            while let Some(i) = queue.pop_front() {
                let p = centre(i, w);
                sx += p[0];
                sy += p[1];
                n += 1.0;
                for j in nbrs(i) {
                    if is_node_px(j) && label[j] == usize::MAX {
                        label[j] = id;
                        queue.push_back(j);
                    }
                }
            }
            g.nodes.push([sx / n, sy / n]);
        }

        let mut visited = vec![false; w * h];
        for i in 0..w * h {
            if label[i] == usize::MAX {
                continue;
            }
            for j in nbrs(i) {
                if label[j] == usize::MAX && !visited[j] {
                    trace(&mut g, &nbrs, w, &label, &mut visited, i, j);
                }
            }
        }
        // Rings: a set two-neighbour pixel never walked becomes a node, walked back to itself.
        for i in 0..w * h {
            if skel.data[i] && degree[i] == 2 && !visited[i] {
                g.nodes.push(centre(i, w));
                label[i] = g.nodes.len() - 1;
                visited[i] = true;
                if let Some(&j) = nbrs(i).first() {
                    if !visited[j] {
                        trace(&mut g, &nbrs, w, &label, &mut visited, i, j);
                    }
                }
            }
        }
        // A walk that turned straight back into its own node is a staircase artefact.
        g.edges.retain(|e| e.a != e.b || e.points.len() > 4);
        g
    }

    fn degrees(&self) -> Vec<usize> {
        let mut d = vec![0; self.nodes.len()];
        for e in &self.edges {
            d[e.a] += 1;
            d[e.b] += 1;
        }
        d
    }

    /// Total polyline length.
    pub fn total_length(&self) -> f32 {
        self.edges.iter().map(PixelEdge::length).sum()
    }

    /// Remove dead-end edges shorter than `min_len` that hang off a junction.
    pub fn prune_spurs(&mut self, min_len: f32) -> bool {
        let d = self.degrees();
        let before = self.edges.len();
        // Shortest first, one per junction per round, so two spurs never strip a real branch.
        let mut order: Vec<usize> = (0..self.edges.len()).collect();
        order.sort_by(|&x, &y| self.edges[x].length().total_cmp(&self.edges[y].length()));
        let mut dropped = vec![false; self.edges.len()];
        let mut touched = vec![false; self.nodes.len()];
        for k in order {
            let e = &self.edges[k];
            if e.a == e.b || e.length() >= min_len {
                continue;
            }
            let junction = match (d[e.a], d[e.b]) {
                (1, j) if j >= 3 => e.b,
                (j, 1) if j >= 3 => e.a,
                _ => continue,
            };
            if touched[junction] {
                continue;
            }
            touched[junction] = true;
            dropped[k] = true;
        }
        let mut k = 0;
        self.edges.retain(|_| {
            k += 1;
            !dropped[k - 1]
        });
        self.edges.len() != before
    }

    /// Drop self-loops shorter than `min_len`.
    pub fn prune_small_loops(&mut self, min_len: f32) -> bool {
        let before = self.edges.len();
        self.edges.retain(|e| e.a != e.b || e.length() >= min_len);
        self.edges.len() != before
    }

    /// Contract edges shorter than `max_len` between two junctions into one node at their middle.
    pub fn merge_close_nodes(&mut self, max_len: f32) -> bool {
        let d = self.degrees();
        let Some(k) = (0..self.edges.len()).find(|&k| {
            let e = &self.edges[k];
            e.a != e.b && d[e.a] >= 3 && d[e.b] >= 3 && e.length() < max_len
        }) else {
            return false;
        };
        let e = self.edges.remove(k);
        let (keep, gone) = (e.a, e.b);
        let mid = e.points[e.points.len() / 2];
        self.nodes[keep] = mid;
        for o in &mut self.edges {
            if o.a == gone {
                o.a = keep;
            }
            if o.b == gone {
                o.b = keep;
            }
            if o.a == keep {
                o.points[0] = mid;
            }
            if o.b == keep {
                *o.points.last_mut().expect("edge has points") = mid;
            }
        }
        true
    }

    /// Join the two edges at every node of degree two into one.
    pub fn dissolve_degree_two(&mut self) -> bool {
        let mut changed = false;
        loop {
            let d = self.degrees();
            let Some(n) = (0..self.nodes.len())
                .find(|&n| d[n] == 2 && !self.edges.iter().any(|e| e.a == n && e.b == n))
            else {
                return changed;
            };
            let ks: Vec<usize> = (0..self.edges.len())
                .filter(|&k| self.edges[k].a == n || self.edges[k].b == n)
                .collect();
            let (k1, k2) = (ks[0], ks[1]);
            let mut e1 = self.edges[k1].clone();
            let mut e2 = self.edges[k2].clone();
            // e1 ends at n, e2 starts at n.
            if e1.b != n {
                e1.points.reverse();
                std::mem::swap(&mut e1.a, &mut e1.b);
            }
            if e2.a != n {
                e2.points.reverse();
                std::mem::swap(&mut e2.a, &mut e2.b);
            }
            e1.points.extend_from_slice(&e2.points[1..]);
            e1.b = e2.b;
            let (hi, lo) = (k1.max(k2), k1.min(k2));
            self.edges.remove(hi);
            self.edges.remove(lo);
            self.edges.push(e1);
            changed = true;
        }
    }

    /// Join pairs of dead ends closer than `max_gap` whose roads point at each other (each end's
    /// outward heading within 60° of the other end), closest pair first: a break in the paint.
    pub fn bridge_gaps(&mut self, max_gap: f32) -> bool {
        let d = self.degrees();
        // Each dead end's outward heading, from a point a few pixels back along its edge.
        let mut heading: Vec<Option<[f32; 2]>> = vec![None; self.nodes.len()];
        for e in &self.edges {
            for (n, pts) in [
                (e.a, e.points.iter().rev().copied().collect::<Vec<_>>()),
                (e.b, e.points.clone()),
            ] {
                if d[n] != 1 || e.a == e.b {
                    continue;
                }
                let tip = pts[pts.len() - 1];
                let back = pts[pts.len().saturating_sub(6)];
                let v = [tip[0] - back[0], tip[1] - back[1]];
                let l = (v[0] * v[0] + v[1] * v[1]).sqrt();
                if l > 1e-3 {
                    heading[n] = Some([v[0] / l, v[1] / l]);
                }
            }
        }
        let ends: Vec<usize> = (0..self.nodes.len())
            .filter(|&n| heading[n].is_some())
            .collect();
        let mut pairs = Vec::new();
        for (i, &a) in ends.iter().enumerate() {
            for &b in &ends[i + 1..] {
                let (pa, pb) = (self.nodes[a], self.nodes[b]);
                let v = [pb[0] - pa[0], pb[1] - pa[1]];
                let gap = (v[0] * v[0] + v[1] * v[1]).sqrt();
                if gap >= max_gap || gap < 1e-3 {
                    continue;
                }
                let u = [v[0] / gap, v[1] / gap];
                let (ha, hb) = (
                    heading[a].unwrap_or_default(),
                    heading[b].unwrap_or_default(),
                );
                let cos = 0.5;
                if ha[0] * u[0] + ha[1] * u[1] >= cos && -(hb[0] * u[0] + hb[1] * u[1]) >= cos {
                    pairs.push((gap, a, b));
                }
            }
        }
        pairs.sort_by(|x, y| x.0.total_cmp(&y.0));
        let mut used = vec![false; self.nodes.len()];
        let mut changed = false;
        for (_, a, b) in pairs {
            if used[a] || used[b] {
                continue;
            }
            used[a] = true;
            used[b] = true;
            self.edges.push(PixelEdge {
                a,
                b,
                points: vec![self.nodes[a], self.nodes[b]],
            });
            changed = true;
        }
        changed
    }

    /// Remove connected components whose total length is under `min_len`.
    pub fn remove_small_components(&mut self, min_len: f32) -> bool {
        let n = self.nodes.len();
        let mut parent: Vec<usize> = (0..n).collect();
        fn find(p: &mut [usize], mut i: usize) -> usize {
            while p[i] != i {
                p[i] = p[p[i]];
                i = p[i];
            }
            i
        }
        for e in &self.edges {
            let (ra, rb) = (find(&mut parent, e.a), find(&mut parent, e.b));
            parent[ra] = rb;
        }
        let mut len = vec![0.0f32; n];
        for e in &self.edges {
            let r = find(&mut parent, e.a);
            len[r] += e.length();
        }
        let before = self.edges.len();
        self.edges.retain(|e| {
            let r = find(&mut parent, e.a);
            len[r] >= min_len
        });
        self.edges.len() != before
    }

    /// Drop nodes no edge uses and renumber.
    pub fn compact(&mut self) {
        let d = self.degrees();
        let mut map = vec![usize::MAX; self.nodes.len()];
        let mut nodes = Vec::new();
        for (i, &deg) in d.iter().enumerate() {
            if deg > 0 {
                map[i] = nodes.len();
                nodes.push(self.nodes[i]);
            }
        }
        self.nodes = nodes;
        for e in &mut self.edges {
            e.a = map[e.a];
            e.b = map[e.b];
        }
    }

    /// Bridge paint gaps, then prune spurs and small loops, merge close junctions, join
    /// through-nodes and drop small pieces until nothing changes.
    pub fn clean(&mut self, spur_len: f32, merge_len: f32, min_component: f32, max_gap: f32) {
        self.dissolve_degree_two();
        if max_gap > 0.0 && self.bridge_gaps(max_gap) {
            self.dissolve_degree_two();
        }
        loop {
            let mut changed = self.prune_spurs(spur_len);
            changed |= self.prune_small_loops(spur_len * 2.0);
            changed |= self.dissolve_degree_two();
            while self.merge_close_nodes(merge_len) {
                changed = true;
            }
            changed |= self.dissolve_degree_two();
            changed |= self.remove_small_components(min_component);
            if !changed {
                break;
            }
        }
        self.compact();
    }
}

/// The 3×3 mean of `f`, over the in-raster neighbours at the border.
fn box3(f: &[f32], w: usize, h: usize) -> Vec<f32> {
    let mut out = vec![0.0; f.len()];
    for r in 0..h {
        for c in 0..w {
            let (mut s, mut n) = (0.0, 0.0);
            for rr in r.saturating_sub(1)..(r + 2).min(h) {
                for cc in c.saturating_sub(1)..(c + 2).min(w) {
                    s += f[rr * w + cc];
                    n += 1.0;
                }
            }
            out[r * w + c] = s / n;
        }
    }
    out
}

/// Moving-average smoothing with fixed ends, `passes` times over a 3-tap kernel.
fn smooth(points: &[[f32; 2]], passes: usize) -> Vec<[f32; 2]> {
    let mut p = points.to_vec();
    for _ in 0..passes {
        if p.len() < 3 {
            break;
        }
        let q = p.clone();
        for i in 1..p.len() - 1 {
            p[i] = [
                (q[i - 1][0] + 2.0 * q[i][0] + q[i + 1][0]) / 4.0,
                (q[i - 1][1] + 2.0 * q[i][1] + q[i + 1][1]) / 4.0,
            ];
        }
    }
    p
}

/// Douglas–Peucker simplification to `eps`.
fn simplify(points: &[[f32; 2]], eps: f32) -> Vec<[f32; 2]> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[points.len() - 1] = true;
    let mut stack = vec![(0usize, points.len() - 1)];
    while let Some((s, e)) = stack.pop() {
        let (a, b) = (points[s], points[e]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = (dx * dx + dy * dy).sqrt();
        let mut best = (0.0f32, 0usize);
        for (i, p) in points.iter().enumerate().take(e).skip(s + 1) {
            let d = if len < 1e-6 {
                ((p[0] - a[0]).powi(2) + (p[1] - a[1]).powi(2)).sqrt()
            } else {
                ((p[0] - a[0]) * dy - (p[1] - a[1]) * dx).abs() / len
            };
            if d > best.0 {
                best = (d, i);
            }
        }
        if best.0 > eps {
            keep[best.1] = true;
            stack.push((s, best.1));
            stack.push((best.1, e));
        }
    }
    points
        .iter()
        .zip(keep)
        .filter_map(|(p, k)| k.then_some(*p))
        .collect()
}

/// Tunables for [`extract_roads`], in yards unless named otherwise.
#[derive(Clone, Debug)]
pub struct RoadParams {
    /// Road weight at or above which a pixel is road.
    pub threshold: f32,
    /// Closing radius: bridges breaks in the painted road.
    pub close_radius: f32,
    /// Opening radius: drops threads and specks.
    pub open_radius: f32,
    /// Road patches smaller than this many square yards are dropped.
    pub min_area: f32,
    /// Holes in the road smaller than this many square yards are filled (plazas, islands).
    pub max_hole: f32,
    /// Dead ends shorter than this are pruned.
    pub spur_len: f32,
    /// Junctions closer than this are merged.
    pub merge_len: f32,
    /// Dead ends closer than this that point at each other are joined.
    pub max_gap: f32,
    /// Networks shorter than this in total are dropped.
    pub min_network: f32,
    /// Douglas–Peucker tolerance for the output polylines.
    pub simplify: f32,
    /// Only the zone's own chunks, grown by this many yards, are traced.
    pub zone_margin: f32,
}

impl Default for RoadParams {
    fn default() -> Self {
        Self {
            threshold: 0.15,
            close_radius: 4.0,
            open_radius: 0.0,
            min_area: 600.0,
            max_hole: 400.0,
            spur_len: 30.0,
            merge_len: 24.0,
            max_gap: 60.0,
            min_network: 120.0,
            simplify: 1.5,
            zone_margin: 40.0,
        }
    }
}

/// One road-network vertex in WoW world yards.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoadNode {
    pub id: usize,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// Edges meeting here: 1 a dead end, 3+ a junction.
    pub degree: usize,
}

/// One polyline sample: WoW world position, the terrain height and the painted road's width.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct RoadPoint {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub width: f32,
}

/// A road between two nodes; `points` runs from node `a` to node `b`, ends included.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoadEdge {
    pub id: usize,
    pub a: usize,
    pub b: usize,
    /// Horizontal length in yards.
    pub length: f32,
    pub points: Vec<RoadPoint>,
}

/// A zone's road network in WoW world coordinates (+X north, +Y west, +Z up, yards).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoadGraph {
    pub zone: String,
    pub area_id: u32,
    pub map_id: u32,
    pub map: String,
    pub nodes: Vec<RoadNode>,
    pub edges: Vec<RoadEdge>,
}

impl RoadGraph {
    /// Total road length in yards.
    pub fn total_length(&self) -> f32 {
        self.edges.iter().map(|e| e.length).sum()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json).with_context(|| format!("writing {}", path.display()))
    }

    pub fn load(path: &Path) -> Result<Self> {
        let s =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        serde_json::from_str(&s).with_context(|| format!("parsing {}", path.display()))
    }
}

/// The intermediate rasters of [`extract_roads`], for the debug render.
pub struct RoadRasters {
    /// The thresholded, cleaned road mask that was thinned.
    pub mask: Mask,
    pub skeleton: Mask,
    /// The cleaned graph in pixel coordinates.
    pub pixel_graph: PixelGraph,
}

/// Extract `terrain`'s road network.
pub fn extract_roads(terrain: &ZoneTerrain, p: &RoadParams) -> (RoadGraph, RoadRasters) {
    let g = terrain.grid;
    let res = g.yards_per_px;
    let px = |yd: f32| (yd / res).round().max(0.0) as i64;
    let area = |yd2: f32| (yd2 / (res * res)).round() as usize;

    let zone = Mask {
        width: g.width,
        height: g.height,
        data: terrain.in_zone.clone(),
    }
    .dilate(px(p.zone_margin));
    // A 3×3 box mean first: painted alpha is speckled at the texel scale.
    let blurred = box3(&terrain.road, g.width, g.height);
    let mut mask = Mask::new(g.width, g.height);
    for (i, (m, &b)) in mask.data.iter_mut().zip(&blurred).enumerate() {
        *m = zone.data[i] && b >= p.threshold;
    }
    let mut mask = mask.close(px(p.close_radius));
    if p.open_radius > 0.0 {
        mask = mask.open(px(p.open_radius));
    }
    let mask = mask
        .fill_holes(area(p.max_hole))
        .remove_small(area(p.min_area));
    let skeleton = mask.zhang_suen();
    let dist = mask.distance_transform();
    let mut pg = PixelGraph::from_skeleton(&skeleton);
    pg.clean(
        p.spur_len / res,
        p.merge_len / res,
        p.min_network / res,
        p.max_gap / res,
    );

    let degrees = pg.degrees();
    let to_world = |q: [f32; 2]| {
        let (x, y) = g.world_of(q[0], q[1]);
        (x, y, terrain.height_at(x, y).unwrap_or(0.0))
    };
    let nodes = pg
        .nodes
        .iter()
        .enumerate()
        .map(|(id, &q)| {
            let (x, y, z) = to_world(q);
            RoadNode {
                id,
                x,
                y,
                z,
                degree: degrees[id],
            }
        })
        .collect();
    let width_at = |q: [f32; 2]| {
        // The medial axis can sit a pixel off the ridge: take the 3×3 maximum.
        let (c, r) = (q[0].floor() as i64, q[1].floor() as i64);
        let mut best = 0.0f32;
        for dr in -1..=1 {
            for dc in -1..=1 {
                if let Some(i) = g.index(c + dc, r + dr) {
                    best = best.max(dist[i]);
                }
            }
        }
        2.0 * best * res
    };
    let edges = pg
        .edges
        .iter()
        .enumerate()
        .map(|(id, e)| {
            let pts = simplify(&smooth(&e.points, 3), p.simplify / res);
            let points: Vec<RoadPoint> = pts
                .iter()
                .map(|&q| {
                    let (x, y, z) = to_world(q);
                    RoadPoint {
                        x,
                        y,
                        z,
                        width: width_at(q),
                    }
                })
                .collect();
            RoadEdge {
                id,
                a: e.a,
                b: e.b,
                length: polyline_length(&pts) * res,
                points,
            }
        })
        .collect();
    (
        RoadGraph {
            zone: terrain.zone.clone(),
            area_id: terrain.area_id,
            map_id: terrain.map_id,
            map: terrain.map.clone(),
            nodes,
            edges,
        },
        RoadRasters {
            mask,
            skeleton,
            pixel_graph: pg,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A thick mask's skeleton and graph.
    fn graph_of(rows: &[&str]) -> PixelGraph {
        PixelGraph::from_skeleton(&Mask::from_ascii(rows).zhang_suen())
    }

    #[test]
    fn straight_bar_is_one_edge() {
        let mut g = graph_of(&[
            "..............................",
            ".############################.",
            ".############################.",
            ".############################.",
            "..............................",
        ]);
        g.clean(2.0, 2.0, 1.0, 0.0);
        assert_eq!(g.edges.len(), 1, "{g:?}");
        assert_eq!(g.nodes.len(), 2);
        assert!(g.total_length() > 20.0);
    }

    #[test]
    fn plus_is_one_junction_with_four_arms() {
        let mut rows = Vec::new();
        for r in 0..41 {
            let line: String = (0..41)
                .map(|c| {
                    if (19..=21).contains(&r) && (1..40).contains(&c)
                        || (19..=21).contains(&c) && (1..40).contains(&r)
                    {
                        '#'
                    } else {
                        '.'
                    }
                })
                .collect();
            rows.push(line);
        }
        let rows: Vec<&str> = rows.iter().map(String::as_str).collect();
        let mut g = graph_of(&rows);
        g.clean(3.0, 4.0, 1.0, 0.0);
        let d = g.degrees();
        assert_eq!(g.edges.len(), 4, "{g:?}");
        assert_eq!(d.iter().filter(|&&x| x == 4).count(), 1);
        assert_eq!(d.iter().filter(|&&x| x == 1).count(), 4);
        // The junction sits on the crossing.
        let j = g.nodes[d.iter().position(|&x| x == 4).unwrap()];
        assert!(
            (j[0] - 20.5).abs() < 2.0 && (j[1] - 20.5).abs() < 2.0,
            "{j:?}"
        );
    }

    #[test]
    fn spur_is_pruned_and_the_road_rejoins() {
        // A long road with a 3-pixel nub on top: pruning leaves one edge.
        let mut g = PixelGraph {
            nodes: vec![[0.0, 0.0], [10.0, 0.0], [20.0, 0.0], [10.0, -3.0]],
            edges: vec![
                PixelEdge {
                    a: 0,
                    b: 1,
                    points: vec![[0.0, 0.0], [10.0, 0.0]],
                },
                PixelEdge {
                    a: 1,
                    b: 2,
                    points: vec![[10.0, 0.0], [20.0, 0.0]],
                },
                PixelEdge {
                    a: 1,
                    b: 3,
                    points: vec![[10.0, 0.0], [10.0, -3.0]],
                },
            ],
        };
        g.clean(5.0, 1.0, 1.0, 0.0);
        assert_eq!(g.edges.len(), 1);
        assert_eq!(g.nodes.len(), 2);
        assert!((g.total_length() - 20.0).abs() < 1e-3);
    }

    #[test]
    fn long_branch_survives_pruning() {
        let mut g = PixelGraph {
            nodes: vec![[0.0, 0.0], [10.0, 0.0], [20.0, 0.0], [10.0, -30.0]],
            edges: vec![
                PixelEdge {
                    a: 0,
                    b: 1,
                    points: vec![[0.0, 0.0], [10.0, 0.0]],
                },
                PixelEdge {
                    a: 1,
                    b: 2,
                    points: vec![[10.0, 0.0], [20.0, 0.0]],
                },
                PixelEdge {
                    a: 1,
                    b: 3,
                    points: vec![[10.0, 0.0], [10.0, -30.0]],
                },
            ],
        };
        g.clean(5.0, 1.0, 1.0, 0.0);
        assert_eq!(g.edges.len(), 3);
    }

    #[test]
    fn aligned_gap_is_bridged_and_a_side_by_side_end_is_not() {
        let mut g = PixelGraph {
            nodes: vec![
                [0.0, 0.0],
                [20.0, 0.0],
                [26.0, 0.0],
                [50.0, 0.0],
                [20.0, 5.0],
                [5.0, 5.0],
            ],
            edges: vec![
                PixelEdge {
                    a: 0,
                    b: 1,
                    points: (0..=20).map(|i| [i as f32, 0.0]).collect(),
                },
                PixelEdge {
                    a: 2,
                    b: 3,
                    points: (26..=50).map(|i| [i as f32, 0.0]).collect(),
                },
                // Parallel to the first road and pointing the same way: not a continuation.
                PixelEdge {
                    a: 5,
                    b: 4,
                    points: (5..=20).map(|i| [i as f32, 5.0]).collect(),
                },
            ],
        };
        g.clean(1.0, 1.0, 1.0, 10.0);
        assert_eq!(g.edges.len(), 2, "{g:?}");
        assert!(g.edges.iter().any(|e| (e.length() - 50.0).abs() < 1e-3));
    }

    #[test]
    fn close_junctions_merge() {
        // Two junctions 2 px apart, each with two long arms: one node of degree 4 after merging.
        let arm = |a: usize, b: usize, p: [f32; 2], q: [f32; 2]| PixelEdge {
            a,
            b,
            points: vec![p, q],
        };
        let mut g = PixelGraph {
            nodes: vec![
                [0.0, 0.0],
                [2.0, 0.0],
                [-20.0, -20.0],
                [-20.0, 20.0],
                [22.0, -20.0],
                [22.0, 20.0],
            ],
            edges: vec![
                arm(0, 1, [0.0, 0.0], [2.0, 0.0]),
                arm(0, 2, [0.0, 0.0], [-20.0, -20.0]),
                arm(0, 3, [0.0, 0.0], [-20.0, 20.0]),
                arm(1, 4, [2.0, 0.0], [22.0, -20.0]),
                arm(1, 5, [2.0, 0.0], [22.0, 20.0]),
            ],
        };
        g.clean(1.0, 5.0, 1.0, 0.0);
        assert_eq!(g.edges.len(), 4);
        assert_eq!(g.degrees().iter().filter(|&&d| d == 4).count(), 1);
    }

    #[test]
    fn ring_traces_as_a_loop() {
        let mut g = graph_of(&[
            "..........................",
            ".########################.",
            ".########################.",
            ".##....................##.",
            ".##....................##.",
            ".##....................##.",
            ".##....................##.",
            ".########################.",
            ".########################.",
            "..........................",
        ]);
        g.clean(2.0, 2.0, 1.0, 0.0);
        assert!(!g.edges.is_empty());
        assert!(g.edges.iter().all(|e| e.a == e.b), "{g:?}");
        assert!(g.total_length() > 40.0);
    }

    #[test]
    fn distance_transform_measures_half_width() {
        let m = Mask::from_ascii(&[
            ".........",
            ".#######.",
            ".#######.",
            ".#######.",
            ".........",
        ]);
        let d = m.distance_transform();
        // The middle row is 2 px from the clear rows above and below.
        assert!((d[2 * 9 + 4] - 2.0).abs() < 1e-5);
        assert!((d[9 + 4] - 1.0).abs() < 1e-5);
        assert_eq!(d[0], 0.0);
    }

    #[test]
    fn small_components_and_holes() {
        let m = Mask::from_ascii(&["#.....", "......", ".####.", ".#..#.", ".####."]);
        assert_eq!(m.remove_small(2).count(), 10);
        assert_eq!(m.fill_holes(3).count(), 13);
    }

    #[test]
    fn simplify_keeps_corners() {
        let pts: Vec<[f32; 2]> = (0..=10)
            .map(|i| [i as f32, 0.0])
            .chain((1..=10).map(|i| [10.0, i as f32]))
            .collect();
        let s = simplify(&pts, 0.5);
        assert_eq!(s, vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]]);
    }
}
