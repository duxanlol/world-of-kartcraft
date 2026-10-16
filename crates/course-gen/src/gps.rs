//! The courier deliveries' GPS roads (`kart_gps`). Painted road alone does not join the towns:
//! many zones (Desolace, Tanaris, the Barrens' south) drive on ground their dirt shares with the
//! open country. So each continent is one drivability grid of [`CELL`]-yard cells, built from
//! every zone's terrain and its WMOs' floors: painted road is cheap, open ground dearer the
//! steeper it is, water dearer still but crossable (a ford), a cliff, a wall, a building or a
//! doodad impassable; a bridge's deck, a tunnel's floor and a city's streets are laid over the
//! ground and driven on. From each delivery town (`deliveries/deliveries.json`) the cheapest way to
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
/// Painted road at or over this weight is driven whatever stands on it (a signpost, a fence's
/// end, an arch's foot: Dun Algaz's gatehouse) and up to [`ROAD_CLIFF`], as a player walks it
/// (WoW's walkable slope is about 50°): a road through a pass is the way, not a wall. Where a
/// kart cannot in fact get by, the test drives find it ([`repair`]).
const ROAD_HOLDS: f32 = 0.5;
const ROAD_CLIFF: f32 = 50.0;
/// The steepest step between neighbouring cells, rise over run (about 38°, [`CLIFF`]): a cell's
/// height is its drivable pixels' mean, so a ledge between two flat cells is no slope a pixel
/// shows.
const STEP_GRADE: f32 = 0.78;
/// A ledge this high (yards) is hopped up, whatever the step's run: a single hop clears 1.15 yd
/// (kart-driver's `measure_the_tallest_box_a_hop_clears`).
const HOP_LEDGE: f32 = 1.1;
/// The test-drive rounds ([`repair`]): the roads driven by the CTR-physics bot over the world's
/// real collision, the ground round each spot it could not drive made this many times dearer
/// this far round (yards), and the ways found again. Dearer, not shut: a way the bot cannot
/// take that is the only one (a city's gate it fails) is still found, and still reported.
pub const REPAIR_ROUNDS: usize = 10;
/// Rounds in a row that drive no better than the best before the repair stops: past that the
/// dearer ground piles up and the ways wander.
const REPAIR_PATIENCE: usize = 2;
const REPAIR_SPOT: f32 = 10.0;
/// A delivery's par: its way's yards at the speed the bot drove the kept round's roads at, this
/// much slower, up to the next [`PAR_STEP`] seconds and at least [`PAR_LEAST`].
const PAR_SLACK: f32 = 1.25;
const PAR_STEP: f32 = 30.0;
const PAR_LEAST: u32 = 120;
const REPAIR_DEARER: f32 = 25.0;
/// Water: crossable (a ford), at this cost a yard.
const WATER: f32 = 12.0;
/// Road weight at or over which a cell counts as road in the report.
const ON_ROAD: f32 = 0.3;
/// How far (yards) a way's point may be from one already laid and join it.
const MERGE: f32 = 6.0;
/// The widest gap (yards, level) between two pieces of drivable ground that is linked: where two
/// groups of towns come this near, the grid has missed the way between (a lift, a tunnel's mouth,
/// a ramp it reads as a cliff), and the nearest point of each is where it is.
const MAX_LINK: f32 = 80.0;
/// A tunnel the grid cannot hold (it runs under the ground the grid shows): its mouths at most
/// this far apart (yards), about level (rise over run), and the ground over this share of the
/// line between them by at least this many yards.
const MAX_TUNNEL: f32 = 400.0;
const TUNNEL_GRADE: f32 = 0.15;
const TUNNEL_COVER: f32 = 0.7;
const TUNNEL_ROOF: f32 = 3.0;
/// Seams: where two pieces of painted road or laid WMO floor, each of at least [`SEAM_PIECE`]
/// cells, come within [`SEAM_REACH`] yards, the bot drives across on the world's real collision
/// ([`crate::gps_drive::drivable`]), [`SEAM_RUN`] yards back into each, and a link joins them if
/// it gets through. A grid of one height a cell cannot hold what a kart drives there: a road over
/// a tunnel (Dun Algaz's lower tunnel under its upper road), a doorway's step, an arch's foot.
const SEAM_PIECE: usize = 20;
const SEAM_REACH: f32 = 30.0;
const SEAM_RUN: f32 = 15.0;

/// The steepest a link climbs, rise over run: about 31°.
const LINK_GRADE: f32 = 0.6;
/// What a yard over a link costs.
const LINK: f32 = 2.0;
/// How far (yards) above or below a town's own height the ground it joins may be, where any is.
const SNAP_RISE: f32 = 20.0;
/// The fewest cells a piece of ground a town joins must hold: a pocket is no way out.
const MIN_GROUND: usize = 5000;
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
    // A WMO's wall: never crossed, road or not.
    if t.walls[i] {
        return Some(f32::INFINITY);
    }
    let slope = t.slope[i];
    let road = t.road[i].clamp(0.0, 1.0);
    if road >= ROAD_HOLDS {
        if slope > ROAD_CLIFF {
            return Some(f32::INFINITY);
        }
        return Some((1.0 - (1.0 - ROAD) * road) * (1.0 + (slope / SLOPE_DOUBLES).powi(2)));
    }
    // A wall, a building or a doodad stands there, or a cliff: no way through. A laid floor (a
    // bridge's deck, a gate's passage) is never solid.
    if slope > CLIFF || t.solid[i] {
        return Some(f32::INFINITY);
    }
    if t.water[i] {
        return Some(WATER);
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
    /// A WMO's wall shuts the cell: no link crosses it.
    pub wall: Vec<bool>,
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
    /// The drivable pixels' heights summed: the ground a kart is on, not the cliff beside it.
    open_height: f32,
    /// A WMO's wall stands on one of the cell's pixels: the cell is shut.
    wall: bool,
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
            wall: Vec::new(),
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
                                s.open_height += t.height[i];
                            } else {
                                s.cliff = s.cliff.saturating_add(1);
                            }
                            s.height += t.height[i];
                            s.road += t.road[i];
                            s.wall |= t.walls[i];
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
            // A wall shuts its cell; otherwise a cell is passable where at least half its pixels
            // are: a mountainside a flat pixel here and there dots is no way up.
            .map(|s| match (s.n, s.cliff) {
                _ if s.laid > 0 => s.laid_cost / f32::from(s.laid),
                (0, _) => f32::NAN,
                _ if s.wall => f32::INFINITY,
                (n, cliff) if 2 * cliff > n => f32::INFINITY,
                (n, cliff) => s.cost / f32::from(n - cliff),
            })
            .collect();
        grid.height = sums
            .iter()
            .map(|s| {
                if s.laid > 0 {
                    s.laid_height / f32::from(s.laid)
                } else if s.n > s.cliff {
                    s.open_height / f32::from(s.n - s.cliff)
                } else {
                    s.height / f32::from(s.n.max(1))
                }
            })
            .collect();
        grid.laid = sums.iter().map(|s| s.laid > 0).collect();
        grid.wall = sums.iter().map(|s| s.wall && s.laid == 0).collect();
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

    /// The cells of `from`'s piece of passable ground and `to`'s that come nearest each other
    /// across a gap `ok` lets a link take, and how far apart they are (yards): a breadth-first
    /// walk over every cell from all of `from`'s piece's cells `seed` keeps at once, out to
    /// `reach` yards, its crossings tried nearest first. `None` when the pieces are one or no
    /// gap within reach may be crossed.
    fn gap(
        &self,
        from: usize,
        to: usize,
        reach: f32,
        seed: impl Fn(usize) -> bool,
        ok: impl Fn(usize, usize) -> bool,
    ) -> Option<(usize, usize, f32)> {
        let (a, b) = (self.piece(from), self.piece(to));
        if a[to] {
            return None;
        }
        self.gap_between(&a, |i| b[i], reach, seed, ok)
    }

    /// [`Grid::gap`] from the cells `a` holds to the first cells `is_b` keeps.
    fn gap_between(
        &self,
        a: &[bool],
        is_b: impl Fn(usize) -> bool,
        reach: f32,
        seed: impl Fn(usize) -> bool,
        ok: impl Fn(usize, usize) -> bool,
    ) -> Option<(usize, usize, f32)> {
        let layers = (reach / CELL * 1.5).ceil() as u16;
        let mut origin = vec![u32::MAX; self.cost.len()];
        let mut layer = vec![0u16; self.cost.len()];
        let mut queue = std::collections::VecDeque::new();
        for (i, &in_a) in a.iter().enumerate() {
            if in_a && seed(i) {
                origin[i] = i as u32;
                queue.push_back(i);
            }
        }
        let mut found: Vec<(f32, usize, usize)> = Vec::new();
        while let Some(i) = queue.pop_front() {
            if is_b(i) {
                let o = origin[i] as usize;
                let (p, q) = (self.centre(o), self.centre(i));
                found.push(((p[0] - q[0]).hypot(p[1] - q[1]), o, i));
                continue;
            }
            if layer[i] >= layers {
                continue;
            }
            for j in self.neighbours(i) {
                if origin[j] == u32::MAX {
                    origin[j] = origin[i];
                    layer[j] = layer[i] + 1;
                    queue.push_back(j);
                }
            }
        }
        found.sort_by(|x, y| x.0.total_cmp(&y.0));
        found
            .into_iter()
            .find(|&(d, o, i)| d <= reach && ok(o, i))
            .map(|(d, o, i)| (o, i, d))
    }

    /// The mouth of a way under the ground: painted road or a laid WMO floor.
    fn mouth(&self, i: usize) -> bool {
        self.laid[i] || self.road[i] >= ON_ROAD
    }

    /// Whether cells `p` and `q`, both [`Grid::mouth`]s, are the ends of a tunnel the grid cannot
    /// hold: about level with each other ([`TUNNEL_GRADE`]), the ground between standing over
    /// the line joining them ([`TUNNEL_COVER`] of it at least [`TUNNEL_ROOF`] yards over), and
    /// no WMO wall on the way.
    fn tunnel(&self, p: usize, q: usize) -> bool {
        if !self.mouth(p) || !self.mouth(q) {
            return false;
        }
        let (a, b) = (self.centre(p), self.centre(q));
        let level = (a[0] - b[0]).hypot(a[1] - b[1]);
        if level < MAX_LINK * 0.25 || (a[2] - b[2]).abs() > level * TUNNEL_GRADE + 5.0 {
            return false;
        }
        let steps = (level / CELL).ceil() as usize;
        let mut covered = 0;
        for s in 1..steps {
            let t = s as f32 / steps as f32;
            let Some(i) = self.cell(a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t) else {
                return false;
            };
            if self.wall[i] {
                return false;
            }
            if self.height[i] >= a[2] + (b[2] - a[2]) * t + TUNNEL_ROOF {
                covered += 1;
            }
        }
        steps > 1 && covered as f32 >= (steps - 1) as f32 * TUNNEL_COVER
    }

    /// Whether a link may join cells `p` and `q`: no WMO wall on the way between, and no steeper
    /// than [`LINK_GRADE`] (a cliff is no gap the grid missed).
    fn crossable(&self, p: usize, q: usize) -> bool {
        let (a, b) = (self.centre(p), self.centre(q));
        let level = (a[0] - b[0]).hypot(a[1] - b[1]);
        if (a[2] - b[2]).abs() > level * LINK_GRADE + 4.0 {
            return false;
        }
        let steps = (level / (CELL * 0.5)).ceil().max(1.0) as usize;
        (0..=steps).all(|s| {
            let t = s as f32 / steps as f32;
            self.cell(a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t)
                .is_none_or(|i| !self.wall[i])
        })
    }

    /// Each passable cell's piece of ground (steps and links followed), `u32::MAX` where none,
    /// and each piece's size in cells.
    fn pieces(&self) -> (Vec<u32>, Vec<usize>) {
        let mut label = vec![u32::MAX; self.cost.len()];
        let mut sizes = Vec::new();
        let mut stack = Vec::new();
        for s in 0..self.cost.len() {
            if label[s] != u32::MAX || !self.passable(s) {
                continue;
            }
            let id = sizes.len() as u32;
            let mut n = 0;
            label[s] = id;
            stack.push(s);
            while let Some(i) = stack.pop() {
                n += 1;
                let linked = self.links.get(&i).into_iter().flatten().copied();
                let stepped = self.neighbours(i).filter(|&j| self.step(i, j));
                for j in stepped.chain(linked) {
                    if label[j] == u32::MAX && self.passable(j) {
                        label[j] = id;
                        stack.push(j);
                    }
                }
            }
            sizes.push(n);
        }
        (label, sizes)
    }

    /// Joins pieces of road across the seams the bot drives over ([`SEAM_REACH`]); one line a
    /// map for the report.
    fn seam_links(&mut self, jobs: usize) -> String {
        let (label, sizes) = self.pieces();
        let big = |i: usize| label[i] != u32::MAX && sizes[label[i] as usize] >= SEAM_PIECE;
        let k = (SEAM_REACH / CELL).ceil() as i64;
        // The nearest pair of mouths of each two pieces.
        let mut best: std::collections::HashMap<(u32, u32), (f32, usize, usize)> =
            Default::default();
        for i in 0..self.cost.len() {
            if !big(i) || !self.mouth(i) {
                continue;
            }
            // Only a piece's edge: a cell with a neighbour outside its piece.
            if self.neighbours(i).all(|j| label[j] == label[i]) {
                continue;
            }
            let (cx, cy) = ((i % self.w) as i64, (i / self.w) as i64);
            let a = self.centre(i);
            for dy in -k..=k {
                for dx in -k..=k {
                    let (x, y) = (cx + dx, cy + dy);
                    if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
                        continue;
                    }
                    let j = x as usize + y as usize * self.w;
                    if !big(j) || label[j] <= label[i] || !self.mouth(j) {
                        continue;
                    }
                    let b = self.centre(j);
                    let d = (a[0] - b[0]).hypot(a[1] - b[1]);
                    if d > SEAM_REACH || (a[2] - b[2]).abs() > d * LINK_GRADE + 4.0 {
                        continue;
                    }
                    let key = (label[i], label[j]);
                    if best.get(&key).is_none_or(|e| d < e.0) {
                        best.insert(key, (d, i, j));
                    }
                }
            }
        }
        let tries: Vec<(usize, usize)> = best.values().map(|&(_, i, j)| (i, j)).collect();
        // Each seam driven: from SEAM_RUN back in one piece, across, SEAM_RUN on into the other.
        let joined = std::sync::Mutex::new(Vec::new());
        let next = std::sync::atomic::AtomicUsize::new(0);
        std::thread::scope(|s| {
            for _ in 0..jobs.max(1) {
                s.spawn(|| {
                    let Ok(mut chain) = open_install() else {
                        return;
                    };
                    loop {
                        let n = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(&(i, j)) = tries.get(n) else {
                            break;
                        };
                        let (a, b) = (self.centre(i), self.centre(j));
                        let d = ((b[0] - a[0]).hypot(b[1] - a[1])).max(1.0);
                        let (ux, uy) = ((b[0] - a[0]) / d, (b[1] - a[1]) / d);
                        let back = [a[0] - ux * SEAM_RUN, a[1] - uy * SEAM_RUN, a[2]];
                        let on = [b[0] + ux * SEAM_RUN, b[1] + uy * SEAM_RUN, b[2]];
                        let line = [back, a, b, on];
                        if crate::gps_drive::drivable(&mut chain, self.map, &line) {
                            joined.lock().unwrap().push((i, j, d));
                        }
                    }
                });
            }
        });
        let joined = joined.into_inner().unwrap();
        for &(i, j, _) in &joined {
            self.links.entry(i).or_default().push(j);
            self.links.entry(j).or_default().push(i);
        }
        format!(
            "seams: {} pieces of {SEAM_PIECE}+ cells, {} seams within {SEAM_REACH} yd tried, {} driven and joined",
            sizes.iter().filter(|&&n| n >= SEAM_PIECE).count(),
            tries.len(),
            joined.len()
        )
    }

    /// The cells `start`'s piece of passable ground holds, links crossed.
    fn piece(&self, start: usize) -> Vec<bool> {
        let mut seen = vec![false; self.cost.len()];
        let mut stack = vec![start];
        seen[start] = true;
        while let Some(i) = stack.pop() {
            let linked = self.links.get(&i).into_iter().flatten().copied();
            let stepped = self.neighbours(i).filter(|&j| self.step(i, j));
            for j in stepped.chain(linked) {
                if !seen[j] && self.passable(j) {
                    seen[j] = true;
                    stack.push(j);
                }
            }
        }
        seen
    }

    /// Links the towns' pieces of ground across their narrowest gaps, nearest first: a gap a
    /// link may take ([`MAX_LINK`], [`Grid::crossable`]), else a tunnel ([`Grid::tunnel`]). One
    /// line a link, for the report.
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
                .filter_map(|(a, b)| {
                    self.gap(a, b, MAX_LINK, |_| true, |o, i| self.crossable(o, i))
                })
                .min_by(|x, y| x.2.total_cmp(&y.2))
                .map(|g| (g, ""));
            // No gap a link may take: a tunnel under the ground between, mouth to mouth.
            let nearest = nearest.or_else(|| {
                firsts
                    .iter()
                    .enumerate()
                    .flat_map(|(k, &a)| firsts[k + 1..].iter().map(move |&b| (a, b)))
                    .filter_map(|(a, b)| {
                        self.gap(
                            a,
                            b,
                            MAX_TUNNEL,
                            |i| self.mouth(i),
                            |o, i| self.tunnel(o, i),
                        )
                    })
                    .min_by(|x, y| x.2.total_cmp(&y.2))
                    .map(|g| (g, " (a tunnel)"))
            });
            match nearest {
                Some(((p, q, d), kind)) => {
                    self.links.entry(p).or_default().push(q);
                    self.links.entry(q).or_default().push(p);
                    let (a, b) = (self.centre(p), self.centre(q));
                    said.push(format!(
                        "linked {d:.0} yd from ({:.0}, {:.0}, {:.0}) to ({:.0}, {:.0}, {:.0}){kind}",
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

    /// The grid with the ground each of `cells`' pieces holds in a colour of its own: where one
    /// town group's reach ends and another's begins. Elsewhere grey, cliffs red, unknown black.
    fn render_pieces(&self, file: &Path, cells: &[usize]) -> Result<()> {
        const COLOURS: [[u8; 3]; 6] = [
            [60, 120, 230],
            [60, 200, 90],
            [220, 120, 220],
            [240, 150, 40],
            [140, 90, 200],
            [200, 200, 60],
        ];
        let mut label = vec![u8::MAX; self.cost.len()];
        let mut next = 0u8;
        for &c in cells {
            if label[c] != u8::MAX {
                continue;
            }
            for (i, inside) in self.piece(c).into_iter().enumerate() {
                if inside {
                    label[i] = next;
                }
            }
            next = next.saturating_add(1);
        }
        let mut img = image::RgbImage::new(self.h as u32, self.w as u32);
        for (i, &l) in label.iter().enumerate() {
            let (cx, cy) = (i % self.w, i / self.w);
            let (col, row) = ((self.h - 1 - cy) as u32, (self.w - 1 - cx) as u32);
            let c = self.cost[i];
            let px = if let Some(&k) = COLOURS.get(l as usize) {
                k
            } else if c.is_nan() {
                [0, 0, 0]
            } else if c.is_infinite() {
                [170, 30, 30]
            } else {
                [120, 120, 120]
            };
            img.put_pixel(col, row, image::Rgb(px));
        }
        img.save(file)
            .with_context(|| format!("writing {}", file.display()))
    }

    fn passable(&self, i: usize) -> bool {
        self.cost[i].is_finite()
    }

    /// Whether a kart steps from cell `i` to its neighbour `j`: no steeper than [`STEP_GRADE`].
    fn step(&self, i: usize, j: usize) -> bool {
        let (a, b) = (self.centre(i), self.centre(j));
        let run = (a[0] - b[0]).hypot(a[1] - b[1]);
        (a[2] - b[2]).abs() <= run * STEP_GRADE + HOP_LEDGE
    }

    /// Every cell within `r` yards of `(x, y)` made `times` dearer.
    fn dearer(&mut self, x: f32, y: f32, r: f32, times: f32) {
        let k = (r / CELL).ceil() as i32;
        for dx in -k..=k {
            for dy in -k..=k {
                let (px, py) = (x + dx as f32 * CELL, y + dy as f32 * CELL);
                if (px - x).hypot(py - y) > r {
                    continue;
                }
                if let Some(i) = self.cell(px, py) {
                    self.cost[i] *= times;
                }
            }
        }
    }

    /// The passable cell nearest `(x, y)`, within 30 cells, on ground that goes somewhere (a
    /// pocket, painted road under a building or a ledge, is passed over for the open ground
    /// beyond) and within [`SNAP_RISE`] yards of `z` where there is one: a town inside a
    /// mountain (Ironforge) joins its own floor, not the hilltop over it.
    fn nearest_passable(&self, x: f32, y: f32, z: f32) -> Option<usize> {
        self.nearest_passable_within(x, y, Some(z))
            .or_else(|| self.nearest_passable_within(x, y, None))
    }

    fn nearest_passable_within(&self, x: f32, y: f32, z: Option<f32>) -> Option<usize> {
        let at = self.cell(x, y)?;
        let (cx, cy) = ((at % self.w) as i64, (at / self.w) as i64);
        let mut pockets = std::collections::HashSet::new();
        (0..30i64).find_map(|r| {
            let mut ring: Vec<usize> = (-r..=r)
                .flat_map(|d| [(d, -r), (d, r), (-r, d), (r, d)])
                .filter_map(|(dx, dy)| {
                    let (x, y) = (cx + dx, cy + dy);
                    (x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h)
                        .then(|| x as usize + y as usize * self.w)
                })
                .filter(|&i| self.passable(i))
                .filter(|&i| z.is_none_or(|z| (self.height[i] - z).abs() <= SNAP_RISE))
                .collect();
            ring.sort_by_key(|&i| {
                let (x, y) = ((i % self.w) as i64 - cx, (i / self.w) as i64 - cy);
                (x * x + y * y, i)
            });
            ring.dedup();
            ring.into_iter().find(|&i| {
                if pockets.contains(&i) {
                    return false;
                }
                match self.reach(i, MIN_GROUND) {
                    Ok(()) => true,
                    Err(pocket) => {
                        pockets.extend(pocket);
                        false
                    }
                }
            })
        })
    }

    /// Whether `start`'s piece of passable ground holds at least `cells` cells; if not, its
    /// cells (a pocket).
    fn reach(&self, start: usize, cells: usize) -> Result<(), Vec<usize>> {
        let mut seen = std::collections::HashSet::from([start]);
        let mut stack = vec![start];
        while let Some(i) = stack.pop() {
            let linked = self.links.get(&i).into_iter().flatten().copied();
            let stepped = self.neighbours(i).filter(|&j| self.step(i, j));
            for j in stepped.chain(linked) {
                if self.passable(j) && seen.insert(j) {
                    if seen.len() >= cells {
                        return Ok(());
                    }
                    stack.push(j);
                }
            }
        }
        Err(seen.into_iter().collect())
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
                if !self.passable(j) || !self.step(i, j) {
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
    key: String,
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
        .map(|t| grid.nearest_passable(t.x, t.y, t.z))
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
                    // From the ground each town joins to the other's: an innkeeper stands in
                    // the inn, and the way in from the street is no road (the client walks it).
                    let points = simplify(
                        &path.iter().map(|&c| grid.centre(c)).collect::<Vec<_>>(),
                        SIMPLIFY,
                    );
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

/// The GPS roads `ways` make, laid as trails that share what they share.
fn lay(ways: &[Way], map: u32) -> kart_gps::RoadMap {
    let mut b = Builder::default();
    for w in ways {
        b.trail(&w.points, MERGE);
    }
    b.finish(map)
}

/// The ways between `towns` and the roads they make, test-driven: each round the roads are driven
/// by the CTR-physics bot over the world's real collision ([`crate::gps_drive`]), the ground round
/// every spot it could not get past made dearer ([`REPAIR_DEARER`]) and the ways found again,
/// until every stretch drives or [`REPAIR_ROUNDS`] are done; the round with the fewest yards the
/// bot could not drive is kept (steering round one spot can find another). A spot the bot names
/// nowhere (it kept leaving the line) is left as it is.
/// Also the bot's speed over the kept round's roads (yd/s; 0 with no test drive).
fn repair(
    grid: &mut Grid,
    towns: &[&Town],
    map: u32,
    jobs: usize,
    rounds: usize,
    out: &mut String,
) -> (Vec<Way>, Vec<String>, kart_gps::RoadMap, f32) {
    if rounds == 0 {
        let (ways, stranded) = ways(grid, towns, jobs);
        let roads = lay(&ways, map);
        return (ways, stranded, roads, 0.0);
    }
    struct Round {
        yards: f32,
        speed: f32,
        round: usize,
        ways: Vec<Way>,
        stranded: Vec<String>,
        roads: kart_gps::RoadMap,
        bad: Vec<String>,
    }
    let mut best: Option<Round> = None;
    let mut worse = 0;
    for round in 1..=rounds {
        let (ways, stranded) = ways(grid, towns, jobs);
        let roads = lay(&ways, map);
        let driven = crate::gps_drive::drive_map(&roads, jobs);
        let bad: Vec<&crate::gps_drive::Stretch> = driven.iter().filter(|s| !s.ok).collect();
        let total: f32 = driven.iter().map(|s| s.length).sum();
        let yards: f32 = bad.iter().map(|s| s.length).sum();
        // The bot's speed over what it drove: the roads' own pace, its slopes and bends in it.
        let (run, secs) = driven
            .iter()
            .filter(|s| s.ok && s.time > 0.0)
            .fold((0.0, 0.0), |(l, t), s| (l + s.length, t + s.time));
        let speed = if secs > 0.0 { run / secs } else { 0.0 };
        *out += &format!(
            "  test drive {round}: {} of {} stretches, {yards:.0} of {total:.0} yd, not drivable\n",
            bad.len(),
            driven.len()
        );
        let spots: Vec<[f32; 3]> = bad.iter().filter_map(|s| s.blocked_at()).collect();
        let lines: Vec<String> = bad
            .iter()
            .map(|s| {
                format!(
                    "    still not drivable: {} ({:.0} yd from ({:.0}, {:.0}, {:.0})): {}\n",
                    s.zone,
                    s.length,
                    s.from[0],
                    s.from[1],
                    s.from[2],
                    s.reject.as_deref().unwrap_or("")
                )
            })
            .collect();
        let mut done = bad.is_empty() || spots.is_empty();
        if best.as_ref().is_some_and(|b| yards >= b.yards) {
            worse += 1;
            done |= worse >= REPAIR_PATIENCE;
        } else {
            worse = 0;
        }
        if best.as_ref().is_none_or(|b| yards < b.yards) {
            best = Some(Round {
                yards,
                speed,
                round,
                ways,
                stranded,
                roads,
                bad: lines,
            });
        }
        if done {
            break;
        }
        for q in spots {
            grid.dearer(q[0], q[1], REPAIR_SPOT, REPAIR_DEARER);
        }
    }
    let best = best.expect("one round at least");
    *out += &format!(
        "  kept test drive {}; the bot drove its roads at {:.1} yd/s\n",
        best.round, best.speed
    );
    for l in &best.bad {
        *out += l;
    }
    (best.ways, best.stranded, best.roads, best.speed)
}

/// The delivery pars (`deliveries/pars.json`): per town pair, both ways, the seconds a route
/// takes by its way at the bot's speed ([`PAR_SLACK`]), and the speed each map was timed at.
#[derive(Default, serde::Serialize, Deserialize)]
pub struct Pars {
    #[serde(rename = "_comment", default)]
    pub comment: String,
    /// Each map's (by its id) speed, yd/s.
    #[serde(default)]
    pub speed: std::collections::BTreeMap<String, f32>,
    /// `"<from key>><to key>"`: seconds.
    #[serde(default)]
    pub pars: std::collections::BTreeMap<String, u32>,
}

/// A way's par at `speed` yd/s.
fn par(yards: f32, speed: f32) -> u32 {
    let secs = yards / speed * PAR_SLACK;
    ((secs / PAR_STEP).ceil() * PAR_STEP).max(PAR_LEAST as f32) as u32
}

/// Builds every continent's grid, the ways between its delivery towns and the GPS roads they
/// make, and reports how the towns fared.
/// How much of `roads` (the GPS roads a map was given before) crosses what `grid` holds
/// impassable, yards, and the spots it does most, 100-yard squares by yards crossed.
fn audit(grid: &Grid, roads: &kart_gps::RoadMap) -> (f32, Vec<([f32; 2], f32)>) {
    let points: Vec<[f32; 3]> = roads
        .points
        .chunks(3)
        .map(|c| [c[0] as f32, c[1] as f32, c[2] as f32])
        .collect();
    let mut spots: std::collections::HashMap<(i32, i32), f32> = Default::default();
    let mut total = 0.0;
    for l in roads.links.chunks(2) {
        let (a, b) = (points[l[0] as usize], points[l[1] as usize]);
        let len = (a[0] - b[0]).hypot(a[1] - b[1]);
        let steps = (len / 2.0).ceil().max(1.0) as usize;
        for s in 0..steps {
            let t = (s as f32 + 0.5) / steps as f32;
            let (x, y) = (a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t);
            if grid.cell(x, y).is_some_and(|i| grid.cost[i].is_infinite()) {
                let d = len / steps as f32;
                total += d;
                *spots
                    .entry(((x / 100.0).floor() as i32, (y / 100.0).floor() as i32))
                    .or_default() += d;
            }
        }
    }
    let mut spots: Vec<([f32; 2], f32)> = spots
        .into_iter()
        .map(|((x, y), d)| ([x as f32 * 100.0 + 50.0, y as f32 * 100.0 + 50.0], d))
        .collect();
    spots.sort_by(|a, b| b.1.total_cmp(&a.1));
    (total, spots)
}

/// `rounds` of test drives at most ([`repair`]; 0 for none), `only` one map if given.
pub fn run(
    deliveries: &Path,
    jobs: usize,
    render: Option<&Path>,
    previous: Option<&RoadFile>,
    rounds: usize,
    only: Option<u32>,
) -> Result<(RoadFile, String)> {
    let zones = continent_zones(&mut open_install()?)?;
    let towns = read_towns(deliveries)?;
    // The pars written before: a map not rebuilt keeps its own.
    let pars_file = deliveries.with_file_name("pars.json");
    let mut pars: Pars = std::fs::read_to_string(&pars_file)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    pars.comment =
        "Written by course-gen gps: each delivery's par, seconds, from its GPS way at the \
        speed the CTR-physics bot drove the map's roads (pack.py reads it; a pair missing here \
        falls back to the straight line)."
            .into();
    let mut maps = Vec::new();
    let mut out = String::new();
    for map in CONTINENTS
        .into_iter()
        .filter(|m| only.is_none_or(|o| o == *m))
    {
        let started = std::time::Instant::now();
        let grid = Grid::build(map, &zones, jobs)?;
        let known = grid.cost.iter().filter(|c| !c.is_nan()).count();
        out += &format!(
            "map {map}: {}×{} cells of {CELL} yd from x {} y {}, {known} on the ground, built in {:.0}s\n",
            grid.w,
            grid.h,
            grid.x0 as f32 * CELL,
            grid.y0 as f32 * CELL,
            started.elapsed().as_secs_f32()
        );
        if let Some(old) = previous.and_then(|p| p.map(map)) {
            let (yards, spots) = audit(&grid, old);
            out += &format!(
                "  the previous roads cross impassable ground for {yards:.0} yd; most at: {:?}\n",
                &spots[..spots.len().min(8)]
            );
        }
        let on_map: Vec<&Town> = towns.iter().filter(|t| t.map == map).collect();
        if let Some(dir) = render {
            let file = dir.join(format!("gps-map{map}.png"));
            grid.render(&file, &on_map)?;
            out += &format!("  drew {}\n", file.display());
        }
        let mut grid = grid;
        let cells: Vec<usize> = on_map
            .iter()
            .filter_map(|t| grid.nearest_passable(t.x, t.y, t.z))
            .collect();
        out += &format!("  {}\n", grid.seam_links(jobs));
        for line in grid.link_towns(&cells) {
            out += &format!("  {line}\n");
        }
        if let Some(dir) = render {
            let file = dir.join(format!("gps-map{map}-pieces.png"));
            grid.render_pieces(&file, &cells)?;
        }
        let (ways, stranded, roads, speed) =
            repair(&mut grid, &on_map, map, jobs, rounds, &mut out);
        if speed > 0.0 {
            pars.speed.insert(map.to_string(), speed);
            pars.pars.retain(|k, _| {
                let from = k.split('>').next().unwrap_or("");
                !on_map.iter().any(|t| t.key == from)
            });
            for w in &ways {
                let p = par(w.length, speed);
                let (a, b) = (&on_map[w.from].key, &on_map[w.to].key);
                pars.pars.insert(format!("{a}>{b}"), p);
                pars.pars.insert(format!("{b}>{a}"), p);
            }
            out += &format!(
                "  {} pars at {speed:.1} yd/s, {PAR_SLACK}× slack\n",
                ways.len() * 2
            );
        }
        // The new roads against the same grid: only the links' spans may cross it.
        let (yards, spots) = audit(&grid, &roads);
        out += &format!(
            "  these roads cross impassable ground for {yards:.0} yd (the links' spans); most at: {:?}\n",
            &spots[..spots.len().min(6)]
        );
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
        // Each town's leg from its innkeeper to the ground it joins, where that crosses ground
        // the grid holds shut.
        for t in &on_map {
            let Some(c) = grid.nearest_passable(t.x, t.y, t.z) else {
                continue;
            };
            let at = grid.centre(c);
            let leg = (at[0] - t.x).hypot(at[1] - t.y);
            let steps = (leg / 2.0).ceil().max(1.0) as usize;
            let shut = (0..=steps)
                .filter(|&s| {
                    let f = s as f32 / steps as f32;
                    grid.cell(t.x + (at[0] - t.x) * f, t.y + (at[1] - t.y) * f)
                        .is_some_and(|i| grid.cost[i].is_infinite())
                })
                .count();
            if shut > 0 {
                out += &format!(
                    "  {}'s leg: {leg:.0} yd from the innkeeper ({:.0}, {:.0}, {:.0}) to ({:.0}, {:.0}, {:.0}), {} yd shut\n",
                    t.name, t.x, t.y, t.z, at[0], at[1], at[2], shut * 2
                );
            }
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
                    grid.nearest_passable(on_map[a].x, on_map[a].y, on_map[a].z),
                    grid.nearest_passable(on_map[b].x, on_map[b].y, on_map[b].z),
                ) else {
                    continue;
                };
                if let Some((p, q, d)) =
                    grid.gap(ca, cb, 400.0, |_| true, |o, i| grid.crossable(o, i))
                {
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
    if !pars.pars.is_empty() {
        std::fs::write(&pars_file, serde_json::to_string_pretty(&pars)? + "\n")
            .with_context(|| format!("writing {}", pars_file.display()))?;
        out += &format!("wrote {}\n", pars_file.display());
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
            wall: vec![false; cells.len()],
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
