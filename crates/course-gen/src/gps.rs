//! The courier deliveries' GPS roads (`kart_gps`). Painted road alone does not join the towns:
//! many zones (Desolace, Tanaris, the Barrens' south) drive on ground their dirt shares with the
//! open country. So each continent is one drivability grid of [`CELL`]-yard cells, built from
//! every zone's terrain: painted road is cheap, open ground dearer the steeper it is, buildings,
//! doodads and water dearer still but crossable (a bridge stands on water, a gate in a wall), a
//! cliff impassable. From each delivery town (`deliveries/deliveries.json`) the cheapest way to
//! every other town on its map is found over the grid, and those ways, laid as trails that share
//! the road they share, are the GPS's roads (`roads/roads.json`).

use std::collections::BinaryHeap;
use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result};
use benilla_formats::{load_area_table_catalog, load_world_map_area_catalog, Chain};
use kart_gps::{Builder, RoadFile, Router};
use serde::Deserialize;

use crate::{load_zone, open_install, TerrainParams, ZoneTerrain};

/// The continents: Eastern Kingdoms and Kalimdor.
const CONTINENTS: [u32; 2] = [0, 1];
/// The grid's cell, yards.
pub const CELL: f32 = 4.0;
/// Ground steeper than this (degrees) is a cliff.
const CLIFF: f32 = 38.0;
/// Slope (degrees) at which open ground costs twice as much.
const SLOPE_DOUBLES: f32 = 25.0;
/// What a fully painted road costs against open ground's 1.
const ROAD: f32 = 0.3;
/// A solid model's footprint and water: crossable, at this cost a yard.
const SOLID: f32 = 6.0;
const WATER: f32 = 12.0;
/// Road weight at or over which a cell counts as road in the report.
const ON_ROAD: f32 = 0.3;
/// How far (yards) a way's point may be from one already laid and join it.
const MERGE: f32 = 6.0;
/// The widest gap (yards, level) between two pieces of drivable ground that is linked: where two
/// groups of towns come this near, the grid has missed the way between (a lift, a tunnel's mouth,
/// a ramp it reads as a cliff), and the nearest point of each is where it is.
const MAX_LINK: f32 = 80.0;
/// What a yard over a link costs.
const LINK: f32 = 2.0;
/// Douglas–Peucker tolerance (yards) for a way before it is laid.
const SIMPLIFY: f32 = 2.0;

/// A zone with a world map on a continent: its map, area, name and world rect.
pub struct Zone {
    pub map: u32,
    pub area: u32,
    pub name: String,
    /// `(x_lo, x_hi, y_lo, y_hi)`, yards.
    pub rect: (f32, f32, f32, f32),
}

pub fn continent_zones(chain: &mut Chain) -> Result<Vec<Zone>> {
    let areas = load_area_table_catalog(chain)?;
    let wma = load_world_map_area_catalog(chain)?;
    let mut zones: Vec<Zone> = wma
        .iter()
        .filter(|(_, a)| a.area_id != 0 && CONTINENTS.contains(&a.map_id))
        .filter_map(|(_, a)| {
            Some(Zone {
                map: a.map_id,
                area: a.area_id,
                name: areas.name(a.area_id)?.to_string(),
                rect: (
                    a.loc_top.min(a.loc_bottom),
                    a.loc_top.max(a.loc_bottom),
                    a.loc_left.min(a.loc_right),
                    a.loc_left.max(a.loc_right),
                ),
            })
        })
        .collect();
    zones.sort_by_key(|z| (z.map, z.area));
    zones.dedup_by_key(|z| z.area);
    Ok(zones)
}

/// What a yard of `t`'s pixel `i` costs to drive: `None` off the ground (a hole, off the tiles),
/// infinite up a cliff.
fn pixel_cost(t: &ZoneTerrain, i: usize) -> Option<f32> {
    if !t.height[i].is_finite() {
        return None;
    }
    let slope = t.slope[i];
    if slope > CLIFF {
        return Some(f32::INFINITY);
    }
    if t.water[i] {
        return Some(WATER);
    }
    if t.solid[i] {
        return Some(SOLID);
    }
    let paved = 1.0 - (1.0 - ROAD) * t.road[i].clamp(0.0, 1.0);
    Some(paved * (1.0 + (slope / SLOPE_DOUBLES).powi(2)))
}

/// A continent's drivability: per cell, the cost of a yard (infinite impassable, NaN unknown),
/// the ground's height and its painted road weight.
pub struct Grid {
    pub map: u32,
    /// The cell index of the grid's first column and row (x, y).
    pub x0: i32,
    pub y0: i32,
    pub w: usize,
    pub h: usize,
    pub cost: Vec<f32>,
    pub height: Vec<f32>,
    pub road: Vec<f32>,
    /// A laid WMO floor (a tunnel, a bridge, a street) decides the cell.
    pub laid: Vec<bool>,
    /// Cells joined across a gap the grid could not drive ([`MAX_LINK`]), both ways.
    pub links: std::collections::HashMap<usize, Vec<usize>>,
}

/// Sums of a cell's pixels while the zones are added.
#[derive(Clone, Copy, Default)]
struct Sum {
    n: u16,
    cliff: u16,
    cost: f32,
    height: f32,
    road: f32,
    /// The pixels of a laid WMO floor (a tunnel's, a bridge's), which decide the cell: a
    /// neighbouring zone sees only the mountain over the tunnel.
    laid: u16,
    laid_cost: f32,
    laid_height: f32,
}

impl Grid {
    fn cell(&self, x: f32, y: f32) -> Option<usize> {
        let (cx, cy) = (
            (x / CELL).floor() as i32 - self.x0,
            (y / CELL).floor() as i32 - self.y0,
        );
        (cx >= 0 && cy >= 0 && (cx as usize) < self.w && (cy as usize) < self.h)
            .then(|| cx as usize + cy as usize * self.w)
    }

    /// World `(x, y, z)` of cell `i`'s centre.
    fn centre(&self, i: usize) -> [f32; 3] {
        let (cx, cy) = ((i % self.w) as i32 + self.x0, (i / self.w) as i32 + self.y0);
        [
            (cx as f32 + 0.5) * CELL,
            (cy as f32 + 0.5) * CELL,
            self.height[i],
        ]
    }

    /// Every zone of `map` loaded and laid onto one grid, `jobs` zones at a time.
    pub fn build(map: u32, zones: &[Zone], jobs: usize) -> Result<Grid> {
        let zones: Vec<&Zone> = zones.iter().filter(|z| z.map == map).collect();
        let margin = 600.0;
        let lo_x = zones.iter().map(|z| z.rect.0).fold(f32::MAX, f32::min) - margin;
        let hi_x = zones.iter().map(|z| z.rect.1).fold(f32::MIN, f32::max) + margin;
        let lo_y = zones.iter().map(|z| z.rect.2).fold(f32::MAX, f32::min) - margin;
        let hi_y = zones.iter().map(|z| z.rect.3).fold(f32::MIN, f32::max) + margin;
        let (x0, y0) = ((lo_x / CELL).floor() as i32, (lo_y / CELL).floor() as i32);
        let w = ((hi_x - lo_x) / CELL).ceil() as usize + 1;
        let h = ((hi_y - lo_y) / CELL).ceil() as usize + 1;
        let mut grid = Grid {
            map,
            x0,
            y0,
            w,
            h,
            cost: Vec::new(),
            height: Vec::new(),
            road: Vec::new(),
            laid: Vec::new(),
            links: Default::default(),
        };
        let sums = Mutex::new(vec![Sum::default(); w * h]);
        let next = std::sync::atomic::AtomicUsize::new(0);
        std::thread::scope(|s| {
            for _ in 0..jobs.max(1) {
                s.spawn(|| loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(zone) = zones.get(i) else {
                        break;
                    };
                    let started = std::time::Instant::now();
                    let loaded = open_install().and_then(|mut c| {
                        let params = TerrainParams {
                            wmo_floors: true,
                            ..TerrainParams::default()
                        };
                        load_zone(&mut c, &zone.name, &params)
                    });
                    let t = match loaded {
                        Ok(t) => t,
                        Err(e) => {
                            eprintln!("{}: {e:#}", zone.name);
                            continue;
                        }
                    };
                    let g = t.grid;
                    let mut sums = sums.lock().unwrap();
                    for row in 0..g.height {
                        for col in 0..g.width {
                            let i = col + row * g.width;
                            let Some(c) = pixel_cost(&t, i) else {
                                continue;
                            };
                            let (x, y) = g.world_of(col as f32 + 0.5, row as f32 + 0.5);
                            let Some(at) = grid.cell(x, y) else {
                                continue;
                            };
                            let s = &mut sums[at];
                            s.n = s.n.saturating_add(1);
                            if c.is_finite() {
                                s.cost += c;
                            } else {
                                s.cliff = s.cliff.saturating_add(1);
                            }
                            s.height += t.height[i];
                            s.road += t.road[i];
                            if t.laid[i] && c.is_finite() {
                                s.laid = s.laid.saturating_add(1);
                                s.laid_cost += c;
                                s.laid_height += t.height[i];
                            }
                        }
                    }
                    eprintln!(
                        "{}: laid in {:.0}s",
                        zone.name,
                        started.elapsed().as_secs_f32()
                    );
                });
            }
        });
        let sums = sums.into_inner().unwrap();
        grid.cost = sums
            .iter()
            .map(|s| match (s.n, s.cliff) {
                _ if s.laid > 0 => s.laid_cost / f32::from(s.laid),
                (0, _) => f32::NAN,
                (n, cliff) if cliff * 2 > n => f32::INFINITY,
                (n, cliff) => s.cost / f32::from(n - cliff),
            })
            .collect();
        grid.height = sums
            .iter()
            .map(|s| {
                if s.laid > 0 {
                    s.laid_height / f32::from(s.laid)
                } else {
                    s.height / f32::from(s.n.max(1))
                }
            })
            .collect();
        grid.laid = sums.iter().map(|s| s.laid > 0).collect();
        grid.road = sums
            .iter()
            .map(|s| s.road / f32::from(s.n.max(1)))
            .collect();
        Ok(grid)
    }

    /// The grid as a picture, north up, one pixel a cell: unknown black, cliffs red, open ground
    /// grey (darker dearer), painted road yellow, laid floors cyan, the towns white crosses.
    fn render(&self, file: &Path, towns: &[&Town]) -> Result<()> {
        // Rows run south from the north (+x) edge, columns east from the west (+y) edge.
        let mut img = image::RgbImage::new(self.h as u32, self.w as u32);
        for i in 0..self.cost.len() {
            let (cx, cy) = (i % self.w, i / self.w);
            let (col, row) = ((self.h - 1 - cy) as u32, (self.w - 1 - cx) as u32);
            let c = self.cost[i];
            let px = if c.is_nan() {
                [0, 0, 0]
            } else if c.is_infinite() {
                [170, 30, 30]
            } else if self.laid[i] {
                [40, 220, 230]
            } else if self.road[i] >= ON_ROAD {
                [230, 200, 40]
            } else {
                let g = (200.0 - 30.0 * c.min(5.0)) as u8;
                [g, g, g]
            };
            img.put_pixel(col, row, image::Rgb(px));
        }
        for t in towns {
            if let Some(i) = self.cell(t.x, t.y) {
                let (cx, cy) = (i % self.w, i / self.w);
                let (col, row) = ((self.h - 1 - cy) as i64, (self.w - 1 - cx) as i64);
                for d in -12..=12i64 {
                    for (c, r) in [(col + d, row), (col, row + d)] {
                        if c >= 0 && r >= 0 && (c as u32) < img.width() && (r as u32) < img.height()
                        {
                            img.put_pixel(c as u32, r as u32, image::Rgb([255, 255, 255]));
                        }
                    }
                }
            }
        }
        img.save(file)
            .with_context(|| format!("writing {}", file.display()))
    }

    /// The cells of `from`'s piece of passable ground and `to`'s that come nearest each other,
    /// and how far apart they are (yards): a breadth-first walk over every cell from all of
    /// `from`'s piece at once, until it reaches `to`'s.
    fn gap(&self, from: usize, to: usize) -> Option<(usize, usize, f32)> {
        let (a, b) = (self.piece(from), self.piece(to));
        if a[to] {
            return None;
        }
        let mut origin = vec![u32::MAX; self.cost.len()];
        let mut queue = std::collections::VecDeque::new();
        for (i, &in_a) in a.iter().enumerate() {
            if in_a {
                origin[i] = i as u32;
                queue.push_back(i);
            }
        }
        while let Some(i) = queue.pop_front() {
            if b[i] {
                let o = origin[i] as usize;
                let (p, q) = (self.centre(o), self.centre(i));
                return Some((o, i, (p[0] - q[0]).hypot(p[1] - q[1])));
            }
            for j in self.neighbours(i) {
                if origin[j] == u32::MAX {
                    origin[j] = origin[i];
                    queue.push_back(j);
                }
            }
        }
        None
    }

    /// The cells `start`'s piece of passable ground holds, links crossed.
    fn piece(&self, start: usize) -> Vec<bool> {
        let mut seen = vec![false; self.cost.len()];
        let mut stack = vec![start];
        seen[start] = true;
        while let Some(i) = stack.pop() {
            let linked = self.links.get(&i).into_iter().flatten().copied();
            for j in self.neighbours(i).chain(linked) {
                if !seen[j] && self.passable(j) {
                    seen[j] = true;
                    stack.push(j);
                }
            }
        }
        seen
    }

    /// Links the towns' pieces of ground across their narrowest gaps, nearest first, while any
    /// is under [`MAX_LINK`]; one line a link, for the report.
    fn link_towns(&mut self, cells: &[usize]) -> Vec<String> {
        let mut said = Vec::new();
        loop {
            // One town of each piece.
            let mut firsts: Vec<usize> = Vec::new();
            let mut covered = vec![false; cells.len()];
            for (k, &c) in cells.iter().enumerate() {
                if covered[k] {
                    continue;
                }
                let piece = self.piece(c);
                for (m, &d) in cells.iter().enumerate() {
                    covered[m] |= piece[d];
                }
                firsts.push(c);
            }
            let nearest = firsts
                .iter()
                .enumerate()
                .flat_map(|(k, &a)| firsts[k + 1..].iter().map(move |&b| (a, b)))
                .filter_map(|(a, b)| self.gap(a, b))
                .min_by(|x, y| x.2.total_cmp(&y.2));
            match nearest {
                Some((p, q, d)) if d <= MAX_LINK => {
                    self.links.entry(p).or_default().push(q);
                    self.links.entry(q).or_default().push(p);
                    let (a, b) = (self.centre(p), self.centre(q));
                    said.push(format!(
                        "linked {d:.0} yd from ({:.0}, {:.0}, {:.0}) to ({:.0}, {:.0}, {:.0})",
                        a[0], a[1], a[2], b[0], b[1], b[2]
                    ));
                }
                _ => return said,
            }
        }
    }

    /// The 8 cells round `i` on the grid.
    fn neighbours(&self, i: usize) -> impl Iterator<Item = usize> + '_ {
        let (cx, cy) = ((i % self.w) as i64, (i / self.w) as i64);
        [
            (1, 0),
            (-1, 0),
            (0, 1),
            (0, -1),
            (1, 1),
            (1, -1),
            (-1, 1),
            (-1, -1),
        ]
        .into_iter()
        .filter_map(move |(dx, dy)| {
            let (x, y) = (cx + dx, cy + dy);
            (x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h)
                .then(|| x as usize + y as usize * self.w)
        })
    }

    fn passable(&self, i: usize) -> bool {
        self.cost[i].is_finite()
    }

    /// The passable cell nearest `(x, y)`, within 30 cells.
    fn nearest_passable(&self, x: f32, y: f32) -> Option<usize> {
        let at = self.cell(x, y)?;
        if self.passable(at) {
            return Some(at);
        }
        let (cx, cy) = ((at % self.w) as i64, (at / self.w) as i64);
        (1..30i64).find_map(|r| {
            (-r..=r)
                .flat_map(|d| [(d, -r), (d, r), (-r, d), (r, d)])
                .filter_map(|(dx, dy)| {
                    let (x, y) = (cx + dx, cy + dy);
                    (x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h)
                        .then(|| x as usize + y as usize * self.w)
                })
                .filter(|&i| self.passable(i))
                .min_by_key(|&i| {
                    let (x, y) = ((i % self.w) as i64 - cx, (i / self.w) as i64 - cy);
                    x * x + y * y
                })
        })
    }

    /// The cheapest ways from `from` to each of `to` (cells), as each cell's predecessor: a
    /// Dijkstra over the 8-neighbour grid that stops once every goal is settled.
    fn ways_from(&self, from: usize, to: &[usize]) -> Vec<u32> {
        let mut dist = vec![f32::INFINITY; self.cost.len()];
        let mut prev = vec![u32::MAX; self.cost.len()];
        let mut open = BinaryHeap::new();
        dist[from] = 0.0;
        open.push(Open(0.0, from as u32));
        let mut left = to.iter().filter(|&&t| t != from).count();
        let diagonal = CELL * std::f32::consts::SQRT_2;
        while let Some(Open(d, i)) = open.pop() {
            let i = i as usize;
            if d > dist[i] {
                continue;
            }
            if to.contains(&i) && i != from {
                left -= 1;
                if left == 0 {
                    break;
                }
            }
            let (cx, cy) = ((i % self.w) as i64, (i / self.w) as i64);
            for (dx, dy) in [
                (1, 0),
                (-1, 0),
                (0, 1),
                (0, -1),
                (1, 1),
                (1, -1),
                (-1, 1),
                (-1, -1),
            ] {
                let (x, y) = (cx + dx, cy + dy);
                if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
                    continue;
                }
                let j = x as usize + y as usize * self.w;
                if !self.passable(j) {
                    continue;
                }
                let step = if dx != 0 && dy != 0 { diagonal } else { CELL };
                let c = d + step * 0.5 * (self.cost[i] + self.cost[j]);
                if c < dist[j] {
                    dist[j] = c;
                    prev[j] = i as u32;
                    open.push(Open(c, j as u32));
                }
            }
            for &j in self.links.get(&i).into_iter().flatten() {
                let (a, b) = (self.centre(i), self.centre(j));
                let span = (a[0] - b[0]).hypot(a[1] - b[1]).hypot(a[2] - b[2]);
                let c = d + span * LINK;
                if c < dist[j] {
                    dist[j] = c;
                    prev[j] = i as u32;
                    open.push(Open(c, j as u32));
                }
            }
        }
        prev
    }
}

#[derive(PartialEq)]
struct Open(f32, u32);

impl Eq for Open {}

impl Ord for Open {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other.0.total_cmp(&self.0).then(other.1.cmp(&self.1))
    }
}

impl PartialOrd for Open {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// `points` with every point within `eps` yards (flat) of the line its neighbours keep dropped.
fn simplify(points: &[[f32; 3]], eps: f32) -> Vec<[f32; 3]> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let (a, b) = (points[0], points[points.len() - 1]);
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len = dx.hypot(dy).max(1e-6);
    let (far, d) = points[1..points.len() - 1]
        .iter()
        .enumerate()
        .map(|(i, p)| (i + 1, ((p[0] - a[0]) * dy - (p[1] - a[1]) * dx).abs() / len))
        .max_by(|x, y| x.1.total_cmp(&y.1))
        .unwrap();
    if d <= eps {
        return vec![a, b];
    }
    let mut left = simplify(&points[..=far], eps);
    left.pop();
    left.extend(simplify(&points[far..], eps));
    left
}

#[derive(Deserialize)]
struct Deliveries {
    towns: Vec<Town>,
}

#[derive(Deserialize)]
struct Town {
    name: String,
    map: u32,
    x: f32,
    y: f32,
    z: f32,
}

fn read_towns(deliveries: &Path) -> Result<Vec<Town>> {
    let file: Deliveries = serde_json::from_str(
        &std::fs::read_to_string(deliveries)
            .with_context(|| format!("reading {}", deliveries.display()))?,
    )?;
    Ok(file.towns)
}

/// A way between two towns: its points (WoW yards) and how much of it is on painted road.
struct Way {
    from: usize,
    to: usize,
    points: Vec<[f32; 3]>,
    length: f32,
    on_road: f32,
}

/// The cheapest way between every pair of `map`'s towns over its grid, and the towns that could
/// not join it.
fn ways(grid: &Grid, towns: &[&Town], jobs: usize) -> (Vec<Way>, Vec<String>) {
    let cells: Vec<Option<usize>> = towns
        .iter()
        .map(|t| grid.nearest_passable(t.x, t.y))
        .collect();
    let stranded = towns
        .iter()
        .zip(&cells)
        .filter(|(_, c)| c.is_none())
        .map(|(t, _)| t.name.clone())
        .collect();
    let out = Mutex::new(Vec::new());
    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..jobs.max(1) {
            s.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if i >= towns.len() {
                    break;
                }
                let Some(from) = cells[i] else { continue };
                let goals: Vec<usize> = cells[i + 1..].iter().flatten().copied().collect();
                let prev = grid.ways_from(from, &goals);
                for (j, cell) in cells.iter().enumerate().skip(i + 1) {
                    let Some(goal) = *cell else { continue };
                    let mut path = vec![goal];
                    while *path.last().unwrap() != from {
                        let p = prev[*path.last().unwrap()];
                        if p == u32::MAX {
                            path.clear();
                            break;
                        }
                        path.push(p as usize);
                    }
                    if path.is_empty() {
                        continue;
                    }
                    path.reverse();
                    let (mut length, mut on_road) = (0.0, 0.0);
                    for w in path.windows(2) {
                        let (a, b) = (grid.centre(w[0]), grid.centre(w[1]));
                        let d = (a[0] - b[0]).hypot(a[1] - b[1]);
                        length += d;
                        if grid.road[w[1]] >= ON_ROAD {
                            on_road += d;
                        }
                    }
                    let (a, b) = (towns[i], towns[j]);
                    let mut points = vec![[a.x, a.y, a.z]];
                    points.extend(simplify(
                        &path.iter().map(|&c| grid.centre(c)).collect::<Vec<_>>(),
                        SIMPLIFY,
                    ));
                    points.push([b.x, b.y, b.z]);
                    out.lock().unwrap().push(Way {
                        from: i,
                        to: j,
                        points,
                        length,
                        on_road,
                    });
                }
                eprintln!("  ways from {} found", towns[i].name);
            });
        }
    });
    let mut ways = out.into_inner().unwrap();
    ways.sort_by_key(|w| (w.from, w.to));
    (ways, stranded)
}

/// Builds every continent's grid, the ways between its delivery towns and the GPS roads they
/// make, and reports how the towns fared.
pub fn run(deliveries: &Path, jobs: usize, render: Option<&Path>) -> Result<(RoadFile, String)> {
    let zones = continent_zones(&mut open_install()?)?;
    let towns = read_towns(deliveries)?;
    let mut maps = Vec::new();
    let mut out = String::new();
    for map in CONTINENTS {
        let started = std::time::Instant::now();
        let grid = Grid::build(map, &zones, jobs)?;
        let known = grid.cost.iter().filter(|c| !c.is_nan()).count();
        out += &format!(
            "map {map}: {}×{} cells of {CELL} yd, {known} on the ground, built in {:.0}s\n",
            grid.w,
            grid.h,
            started.elapsed().as_secs_f32()
        );
        let on_map: Vec<&Town> = towns.iter().filter(|t| t.map == map).collect();
        if let Some(dir) = render {
            let file = dir.join(format!("gps-map{map}.png"));
            grid.render(&file, &on_map)?;
            out += &format!("  drew {}\n", file.display());
        }
        let mut grid = grid;
        let cells: Vec<usize> = on_map
            .iter()
            .filter_map(|t| grid.nearest_passable(t.x, t.y))
            .collect();
        for line in grid.link_towns(&cells) {
            out += &format!("  {line}\n");
        }
        let (ways, stranded) = ways(&grid, &on_map, jobs);
        let mut b = Builder::default();
        for w in &ways {
            b.trail(&w.points, MERGE);
        }
        let roads = b.finish(map);
        let router = Router::new(&roads);
        let pairs = on_map.len() * on_map.len().saturating_sub(1) / 2;
        out += &format!(
            "  {} of {pairs} town pairs have a way; {} points, {} links\n",
            ways.len(),
            roads.points.len() / 3,
            roads.links.len() / 2
        );
        if !stranded.is_empty() {
            out += &format!("  towns off the grid: {stranded:?}\n");
        }
        // The towns in the groups the ways join.
        let mut group: Vec<usize> = (0..on_map.len()).collect();
        fn root(g: &mut [usize], i: usize) -> usize {
            let mut r = i;
            while g[r] != r {
                r = g[r];
            }
            g[i] = r;
            r
        }
        for w in &ways {
            let (a, b) = (root(&mut group, w.from), root(&mut group, w.to));
            group[a] = b;
        }
        let mut groups: std::collections::BTreeMap<usize, Vec<&str>> = Default::default();
        for (i, town) in on_map.iter().enumerate() {
            let r = root(&mut group, i);
            groups.entry(r).or_default().push(town.name.as_str());
        }
        for g in groups.values() {
            out += &format!("  joined: {}\n", g.join(", "));
        }
        // Where each group's ground comes nearest another's: the gaps to bridge.
        let firsts: Vec<usize> = groups
            .keys()
            .filter_map(|&r| (0..on_map.len()).find(|&i| root(&mut group, i) == r))
            .collect();
        for (k, &a) in firsts.iter().enumerate() {
            for &b in &firsts[k + 1..] {
                let (Some(ca), Some(cb)) = (
                    grid.nearest_passable(on_map[a].x, on_map[a].y),
                    grid.nearest_passable(on_map[b].x, on_map[b].y),
                ) else {
                    continue;
                };
                if let Some((p, q, d)) = grid.gap(ca, cb) {
                    let (p, q) = (grid.centre(p), grid.centre(q));
                    out += &format!(
                        "  gap {} – {}: {d:.0} yd between ({:.0}, {:.0}, {:.0}) and ({:.0}, {:.0}, {:.0})\n",
                        on_map[a].name, on_map[b].name, p[0], p[1], p[2], q[0], q[1], q[2]
                    );
                }
            }
        }
        let mut detour: Vec<(f32, &Way)> = ways
            .iter()
            .map(|w| {
                let (a, b) = (on_map[w.from], on_map[w.to]);
                (w.length / (a.x - b.x).hypot(a.y - b.y), w)
            })
            .collect();
        detour.sort_by(|x, y| x.0.total_cmp(&y.0));
        let road: f32 = ways.iter().map(|w| w.on_road).sum::<f32>()
            / ways.iter().map(|w| w.length).sum::<f32>().max(1.0);
        if let Some(mid) = detour.get(detour.len() / 2) {
            out += &format!(
                "  way ÷ crow-flies: median {:.2}; {:.0}% of all ways' yards on painted road\n",
                mid.0,
                road * 100.0
            );
        }
        for (r, w) in detour.iter().rev().take(6) {
            out += &format!(
                "    {r:.2} {} – {} ({:.0} yd, {:.0}% road)\n",
                on_map[w.from].name,
                on_map[w.to].name,
                w.length,
                100.0 * w.on_road / w.length.max(1.0)
            );
        }
        // The GPS's own routes over the laid roads, against the ways they were laid from.
        let mut worse = Vec::new();
        for w in &ways {
            let (a, b) = (on_map[w.from], on_map[w.to]);
            match router.route([a.x, a.y, a.z], [b.x, b.y, b.z]) {
                Some(r) if r.length() <= w.length * 1.15 + 50.0 => {}
                Some(r) => worse.push(format!(
                    "{} – {} {:.0}/{:.0}",
                    a.name,
                    b.name,
                    r.length(),
                    w.length
                )),
                None => worse.push(format!("{} – {} none", a.name, b.name)),
            }
        }
        out += &format!(
            "  GPS routes over the laid roads off their way by more than 15%: {} {:?}\n",
            worse.len(),
            &worse[..worse.len().min(8)]
        );
        maps.push(roads);
    }
    Ok((RoadFile { maps }, out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(cost: &[&str]) -> Grid {
        let (w, h) = (cost[0].len(), cost.len());
        let cells: Vec<f32> = cost
            .iter()
            .flat_map(|row| {
                row.chars().map(|c| match c {
                    '#' => f32::INFINITY,
                    '=' => ROAD,
                    _ => 1.0,
                })
            })
            .collect();
        Grid {
            map: 0,
            x0: 0,
            y0: 0,
            w,
            h,
            road: cells
                .iter()
                .map(|&c| if c == ROAD { 1.0 } else { 0.0 })
                .collect(),
            laid: vec![false; cells.len()],
            links: Default::default(),
            height: vec![0.0; cells.len()],
            cost: cells,
        }
    }

    fn path(g: &Grid, from: usize, to: usize) -> Vec<usize> {
        let prev = g.ways_from(from, &[to]);
        let mut p = vec![to];
        while *p.last().unwrap() != from {
            p.push(prev[*p.last().unwrap()] as usize);
        }
        p
    }

    #[test]
    fn the_way_takes_the_road_round_rather_than_the_rough_across_and_never_the_cliff() {
        // Rows are y, columns x. A wall with one gap; a road round the long way.
        let g = grid(&[
            "..........",
            "..........",
            "####.#####",
            "..........",
            "==========",
        ]);
        let (from, to) = (0, 9);
        let p = path(&g, from, to);
        assert!(p.iter().all(|&i| g.cost[i].is_finite()));
        // From the top row's ends the open row is shortest: the road below is behind the wall.
        assert!(p.iter().all(|&i| i / g.w < 2), "{p:?}");
        let (from, to) = (3 * g.w, 3 * g.w + 9);
        let p = path(&g, from, to);
        assert!(
            p.iter().filter(|&&i| i / g.w == 4).count() >= 6,
            "a step down to the road pays: {p:?}"
        );
    }

    #[test]
    fn simplify_keeps_the_corner_and_drops_the_straight() {
        let line: Vec<[f32; 3]> = (0..10)
            .map(|i| [i as f32, 0.0, 0.0])
            .chain((1..10).map(|i| [9.0, i as f32, 0.0]))
            .collect();
        assert_eq!(
            simplify(&line, 0.5),
            [[0.0, 0.0, 0.0], [9.0, 0.0, 0.0], [9.0, 9.0, 0.0]]
        );
    }
}
