//! The courier deliveries' GPS. A continent's roads are a graph of points along them (WoW world
//! yards: +X north, +Y west, +Z up) joined by links, built once from the player's install by
//! `course-gen gps` out of every zone's painted roads (`course-gen`'s `roads`) and written to
//! `roads/roads.json`; the client routes along it from wherever the kart is to the recipient, the
//! shortest way by road.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

use serde::{Deserialize, Serialize};

/// The longest step (yards) between two neighbouring points of a road.
pub const SPACING: f32 = 8.0;
/// Two points join only this near in height too (yards): a road over a tunnel (Dun Algaz's upper
/// road over its lower tunnel) is two roads, not one.
pub const MERGE_RISE: f32 = 4.0;
/// How far (yards) a place may be from the road and still join it; farther, there is no route.
pub const MAX_SNAP: f32 = 400.0;
/// The spatial index's cell, yards.
const CELL: f32 = 32.0;

/// The road file: one network a map.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RoadFile {
    pub maps: Vec<RoadMap>,
}

impl RoadFile {
    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    pub fn map(&self, id: u32) -> Option<&RoadMap> {
        self.maps.iter().find(|m| m.map == id)
    }
}

/// One map's roads as written: the points' `x, y, z` in whole yards and the links' point
/// indices, both flattened.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RoadMap {
    pub map: u32,
    pub points: Vec<i32>,
    pub links: Vec<u32>,
}

fn flat(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// A map's roads being made: the ways driven between places laid one after another, a way's
/// points joining any already laid within reach, so ways that share a road share its points.
#[derive(Default)]
pub struct Builder {
    pos: Vec<[f32; 3]>,
    links: Vec<(u32, u32)>,
    linked: HashSet<(u32, u32)>,
    index: Index,
}

impl Builder {
    /// The point at `p`: one already laid within `merge` yards, else a new one.
    fn point(&mut self, p: [f32; 3], merge: f32) -> u32 {
        let pos = &self.pos;
        let level = |j: u32| (pos[j as usize][2] - p[2]).abs() <= MERGE_RISE;
        if let Some((j, _)) = self.index.nearest(pos, p, merge, level) {
            return j;
        }
        self.pos.push(p);
        let i = (self.pos.len() - 1) as u32;
        self.index.insert(i, p);
        i
    }

    /// The way along `line`, cut into steps of at most [`SPACING`], each point joining one
    /// already laid within `merge` yards.
    pub fn trail(&mut self, line: &[[f32; 3]], merge: f32) {
        let Some(&first) = line.first() else {
            return;
        };
        let mut last = self.point(first, merge);
        for w in line.windows(2) {
            let steps = (flat(w[0], w[1]) / SPACING).ceil().max(1.0) as usize;
            for s in 1..=steps {
                let t = s as f32 / steps as f32;
                let at = self.point(
                    std::array::from_fn(|k| w[0][k] + (w[1][k] - w[0][k]) * t),
                    merge,
                );
                if at != last && self.linked.insert((at.min(last), at.max(last))) {
                    self.links.push((last, at));
                }
                last = at;
            }
        }
    }

    pub fn finish(self, map: u32) -> RoadMap {
        RoadMap {
            map,
            points: self
                .pos
                .iter()
                .flat_map(|p| p.map(|v| v.round() as i32))
                .collect(),
            links: self.links.iter().flat_map(|&(a, b)| [a, b]).collect(),
        }
    }
}

/// Points by the [`CELL`] they fall in.
#[derive(Default)]
struct Index {
    cells: HashMap<(i32, i32), Vec<u32>>,
}

fn cell(p: [f32; 3]) -> (i32, i32) {
    ((p[0] / CELL).floor() as i32, (p[1] / CELL).floor() as i32)
}

impl Index {
    fn new(pos: &[[f32; 3]]) -> Self {
        let mut cells: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (i, &p) in pos.iter().enumerate() {
            cells.entry(cell(p)).or_default().push(i as u32);
        }
        Self { cells }
    }

    fn insert(&mut self, i: u32, p: [f32; 3]) {
        self.cells.entry(cell(p)).or_default().push(i);
    }

    /// The nearest point to `p` within `reach` yards that `keep` keeps, with its distance.
    fn nearest(
        &self,
        pos: &[[f32; 3]],
        p: [f32; 3],
        reach: f32,
        mut keep: impl FnMut(u32) -> bool,
    ) -> Option<(u32, f32)> {
        let (cx, cy) = cell(p);
        let r = (reach / CELL).ceil() as i32;
        let mut best: Option<(u32, f32)> = None;
        for dx in -r..=r {
            for dy in -r..=r {
                for &j in self.cells.get(&(cx + dx, cy + dy)).into_iter().flatten() {
                    let d = flat(p, pos[j as usize]);
                    if d <= reach && best.is_none_or(|(_, b)| d < b) && keep(j) {
                        best = Some((j, d));
                    }
                }
            }
        }
        best
    }
}

/// A road route: the way to drive, from the start to the goal, with how much of it is road.
#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    /// WoW world yards: the start, the road's points, the goal.
    pub points: Vec<[f32; 3]>,
    /// Yards along the road.
    pub road: f32,
    /// Yards off it: from the start onto the road and from the road to the goal.
    pub off_road: f32,
}

impl Route {
    pub fn length(&self) -> f32 {
        self.road + self.off_road
    }
}

/// A map's roads ready to route on.
pub struct Router {
    pos: Vec<[f32; 3]>,
    /// Point `i`'s neighbours are `adj[start[i]..start[i + 1]]`.
    start: Vec<u32>,
    adj: Vec<u32>,
    /// Each point's connected network.
    network: Vec<u32>,
    index: Index,
}

#[derive(PartialEq)]
struct Open(f32, u32);

impl Eq for Open {}

impl Ord for Open {
    fn cmp(&self, other: &Self) -> Ordering {
        other.0.total_cmp(&self.0).then(other.1.cmp(&self.1))
    }
}

impl PartialOrd for Open {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Router {
    pub fn new(map: &RoadMap) -> Self {
        let pos: Vec<[f32; 3]> = map
            .points
            .as_chunks::<3>()
            .0
            .iter()
            .map(|c| c.map(|v| v as f32))
            .collect();
        let n = pos.len();
        let mut count = vec![0u32; n + 1];
        for &i in &map.links {
            count[i as usize + 1] += 1;
        }
        for i in 0..n {
            count[i + 1] += count[i];
        }
        let start = count.clone();
        let mut adj = vec![0u32; map.links.len()];
        for l in map.links.as_chunks::<2>().0 {
            for (a, b) in [(l[0], l[1]), (l[1], l[0])] {
                adj[count[a as usize] as usize] = b;
                count[a as usize] += 1;
            }
        }
        let mut network = vec![u32::MAX; n];
        let mut stack = Vec::new();
        for s in 0..n {
            if network[s] != u32::MAX {
                continue;
            }
            network[s] = s as u32;
            stack.push(s as u32);
            while let Some(i) = stack.pop() {
                for &j in &adj[start[i as usize] as usize..start[i as usize + 1] as usize] {
                    if network[j as usize] == u32::MAX {
                        network[j as usize] = s as u32;
                        stack.push(j);
                    }
                }
            }
        }
        let index = Index::new(&pos);
        Self {
            pos,
            start,
            adj,
            network,
            index,
        }
    }

    fn neighbours(&self, i: u32) -> &[u32] {
        &self.adj[self.start[i as usize] as usize..self.start[i as usize + 1] as usize]
    }

    /// The nearest road point of each network within [`MAX_SNAP`] of `p`, nearest counting the
    /// height (a kart in a tunnel joins the tunnel's road, not the road over it), and how far it
    /// is on the level.
    fn joins(&self, p: [f32; 3]) -> HashMap<u32, (u32, f32)> {
        let mut by_network: HashMap<u32, (u32, f32, f32)> = HashMap::new();
        self.index.nearest(&self.pos, p, MAX_SNAP, |j| {
            let q = self.pos[j as usize];
            let d = flat(p, q);
            let near = d.hypot(p[2] - q[2]);
            let e = by_network
                .entry(self.network[j as usize])
                .or_insert((j, d, near));
            if near < e.2 {
                *e = (j, d, near);
            }
            false
        });
        by_network
            .into_iter()
            .map(|(n, (j, d, _))| (n, (j, d)))
            .collect()
    }

    /// The shortest road route from `from` to `to`: onto the network both reach with the least
    /// driving off it, along it, and off to the goal. `None` when no network is near both.
    pub fn route(&self, from: [f32; 3], to: [f32; 3]) -> Option<Route> {
        let starts = self.joins(from);
        let goals = self.joins(to);
        let (&(a, da), &(b, db)) = starts
            .iter()
            .filter_map(|(net, s)| goals.get(net).map(|g| (s, g)))
            .min_by(|x, y| (x.0 .1 + x.1 .1).total_cmp(&(y.0 .1 + y.1 .1)))?;
        let goal = self.pos[b as usize];
        // Dense tables over the points: a search touches a good share of a continent's.
        let mut cost = vec![f32::INFINITY; self.pos.len()];
        cost[a as usize] = 0.0;
        let mut came = vec![u32::MAX; self.pos.len()];
        let mut open = BinaryHeap::from([Open(flat(self.pos[a as usize], goal), a)]);
        while let Some(Open(_, i)) = open.pop() {
            if i == b {
                break;
            }
            let here = cost[i as usize];
            for &j in self.neighbours(i) {
                let c = here + flat(self.pos[i as usize], self.pos[j as usize]);
                if c < cost[j as usize] {
                    cost[j as usize] = c;
                    came[j as usize] = i;
                    open.push(Open(c + flat(self.pos[j as usize], goal), j));
                }
            }
        }
        let mut path = vec![b];
        while let Some(&p) = came.get(*path.last()? as usize).filter(|&&p| p != u32::MAX) {
            path.push(p);
        }
        path.reverse();
        let mut points = vec![from];
        points.extend(path.iter().map(|&i| self.pos[i as usize]));
        points.push(to);
        Some(Route {
            points,
            road: Some(cost[b as usize]).filter(|c| c.is_finite())?,
            off_road: da + db,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two ways laid along the same road for a stretch (a yard apart), an L and a straight that
    /// carries on from its corner, and a loose way far off.
    fn map() -> RoadMap {
        let mut b = Builder::default();
        b.trail(
            &[[0.0, 0.0, 0.0], [100.0, 0.0, 0.0], [100.0, 100.0, 0.0]],
            4.0,
        );
        b.trail(
            &[[50.0, 1.0, 0.0], [100.0, 1.0, 0.0], [100.0, 300.0, 5.0]],
            4.0,
        );
        b.trail(&[[-300.0, 0.0, 0.0], [-300.0, 50.0, 0.0]], 4.0);
        b.finish(0)
    }

    #[test]
    fn ways_share_the_road_they_share_in_short_steps_and_survive_their_file() {
        let m = map();
        let back = RoadFile::parse(
            &serde_json::to_string(&RoadFile {
                maps: vec![m.clone()],
            })
            .unwrap(),
        )
        .unwrap();
        assert_eq!(back.map(0), Some(&m));
        let pos: Vec<&[i32]> = m.points.chunks(3).collect();
        for l in m.links.chunks(2) {
            let (a, b) = (pos[l[0] as usize], pos[l[1] as usize]);
            let d = ((a[0] - b[0]) as f32).hypot((a[1] - b[1]) as f32);
            assert!(d <= SPACING + 2.0 * 4.0, "a step of {d}");
        }
        // The second way's first 150 yards lie on the first's points: none of its own until it
        // leaves the corner's road, 100 yards up.
        let own = pos.iter().filter(|p| p[1] == 1).count();
        assert_eq!(own, 0, "the shared stretch made points of its own");
        let new_up_the_road = pos.iter().filter(|p| p[0] == 100 && p[1] > 104).count();
        assert_eq!(
            new_up_the_road, 25,
            "past the shared road, its own points: 200 yd / 8"
        );
    }

    #[test]
    fn a_road_over_a_tunnel_stays_two_roads() {
        let mut b = Builder::default();
        b.trail(&[[0.0, -50.0, 100.0], [0.0, 50.0, 100.0]], 6.0);
        b.trail(&[[-50.0, 0.0, 30.0], [50.0, 0.0, 30.0]], 6.0);
        let m = b.finish(0);
        let z: Vec<i32> = m.points.chunks(3).map(|p| p[2]).collect();
        for l in m.links.chunks(2) {
            assert_eq!(
                z[l[0] as usize], z[l[1] as usize],
                "no link between the levels"
            );
        }
        // Crossing level, they share the crossing.
        let mut b = Builder::default();
        b.trail(&[[0.0, -50.0, 30.0], [0.0, 50.0, 30.0]], 6.0);
        b.trail(&[[-50.0, 0.0, 30.0], [50.0, 0.0, 30.0]], 6.0);
        let level = b.finish(0);
        assert!(level.points.len() < m.points.len());
    }

    #[test]
    fn the_route_follows_the_shared_road_round_the_corner() {
        let r = Router::new(&map());
        let route = r
            .route([-5.0, 3.0, 0.0], [100.0, 305.0, 5.0])
            .expect("one network");
        assert!(
            (395.0..410.0).contains(&route.road),
            "the L's two legs and the straight on: {}",
            route.road
        );
        assert!((route.off_road - (3.0f32.hypot(5.0) + 5.0)).abs() < 0.01);
        assert_eq!(route.points.first(), Some(&[-5.0, 3.0, 0.0]));
        assert_eq!(route.points.last(), Some(&[100.0, 305.0, 5.0]));
        assert!(
            route.points.contains(&[100.0, 0.0, 0.0]),
            "round the corner"
        );
    }

    #[test]
    fn a_route_keeps_to_one_network_and_none_is_too_far_from_both() {
        let r = Router::new(&map());
        // The loose road is the nearer to the start, but the goal is only on the big network.
        let route = r.route([-260.0, 0.0, 0.0], [100.0, 200.0, 0.0]).unwrap();
        assert_eq!(route.points[1], [0.0, 0.0, 0.0], "joins the big network");
        assert!(r.route([-300.0, 0.0, 0.0], [5000.0, 0.0, 0.0]).is_none());
    }
}
