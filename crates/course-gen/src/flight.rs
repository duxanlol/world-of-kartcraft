//! The flight bot ([`kart_driver::fly`]) over the real world: a flight course's rings flown as the
//! client passes them (`kart/race.rs`, `passes_3d`: through the hoop the way the course runs), a
//! flight distance flown as far as the bot gets off its launch, and a course's rings laid at the
//! heights the bot reaches, short of them by a margin a player can miss the best line by.

use anyhow::{anyhow, Context, Result};
use benilla_formats::Chain;
use glam::Vec3;
use kart_driver::fly::{self, FlyReport, Ring, Sea};
use kart_driver::{Class, Collision, CELL, LU_PER_YARD};
use serde_json::Value;

use crate::soup::{self, wow_to_bevy, Region};

/// How far round the route the world is gathered, yards, and how far above and below its start.
const ROUND: f32 = 200.0;
const DEPTH: f32 = 320.0;

/// A flight in the real world: the terrain round it and the origin its lu count from (WoW).
pub struct Sky {
    pub world: Collision,
    /// The water's faces, lu, if there is any.
    pub water: Option<Collision>,
    /// The open world's trees, lu, if there are any.
    pub props: Option<Collision>,
    pub origin: [f32; 3],
}

impl Sky {
    /// The world `half` yards round WoW `centre` (x, y) on `map`, from the ground at about
    /// `z` to [`DEPTH`] over and under it, its lu from there.
    pub fn gather_area(
        chain: &mut Chain,
        map: u32,
        centre: [f32; 2],
        half: f32,
        z: f32,
    ) -> Result<Self> {
        let origin = [centre[0], centre[1], z];
        let mid = wow_to_bevy(origin);
        let region = Region {
            min: mid - Vec3::new(half, DEPTH, half),
            max: mid + Vec3::new(half, DEPTH, half),
        };
        let faces = soup::gather_world(chain, map, region)?;
        Ok(Self::of(faces, mid, origin))
    }

    /// The world round `points` (WoW x, y, z) on `map`, its lu from `points[0]`.
    pub fn gather(chain: &mut Chain, map: u32, points: &[[f32; 3]]) -> Result<Self> {
        let origin = *points.first().ok_or_else(|| anyhow!("no points"))?;
        let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        for p in points {
            let b = wow_to_bevy(*p);
            lo = lo.min(b);
            hi = hi.max(b);
        }
        let mid = (lo + hi) * 0.5;
        let half = (hi - lo) * 0.5 + Vec3::new(ROUND, 0.0, ROUND);
        let region = Region {
            min: Vec3::new(mid.x - half.x, mid.y - DEPTH, mid.z - half.z),
            max: Vec3::new(mid.x + half.x, mid.y + DEPTH, mid.z + half.z),
        };
        let faces = soup::gather_world(chain, map, region)?;
        Ok(Self::of(faces, wow_to_bevy(origin), origin))
    }

    fn of(faces: soup::Faces, mid: Vec3, origin: [f32; 3]) -> Self {
        let world = Collision::new(soup::to_lu(&faces.solid, mid), CELL);
        let wet = soup::to_lu(&faces.liquid, mid);
        let water = (!wet.is_empty()).then(|| Collision::new(wet, CELL));
        let placed = soup::to_lu(&faces.props, mid);
        let props = (!placed.is_empty()).then(|| Collision::new(placed, CELL));
        Self {
            world,
            water,
            props,
            origin,
        }
    }

    /// The water for the bot.
    pub fn sea(&self) -> Sea<'_> {
        Sea {
            water: self.water.as_ref(),
            pads: &[],
        }
    }

    /// A WoW point in the flight's lu.
    pub fn lu(&self, p: [f32; 3]) -> Vec3 {
        (wow_to_bevy(p) - wow_to_bevy(self.origin)) * LU_PER_YARD
    }

    /// A flight's lu as WoW yards.
    pub fn wow(&self, lu: Vec3) -> [f32; 3] {
        soup::bevy_to_wow(lu / LU_PER_YARD + wow_to_bevy(self.origin))
    }

    /// The ground's height (WoW z) under a WoW point, from high above it.
    pub fn ground(&self, x: f32, y: f32) -> Option<f32> {
        let top = self.lu([x, y, self.origin[2] + DEPTH]);
        let d = fly::over_ground(&self.world, top);
        d.is_finite()
            .then(|| self.origin[2] + DEPTH - d / LU_PER_YARD)
    }

    /// Whether what a kart comes down on at a WoW point is a tree's crown, not the land or a
    /// building.
    pub fn on_prop(&self, x: f32, y: f32) -> bool {
        let Some(props) = &self.props else {
            return false;
        };
        let top = self.lu([x, y, self.origin[2] + DEPTH]);
        let (all, prop) = (
            fly::over_ground(&self.world, top),
            fly::over_ground(props, top),
        );
        prop.is_finite() && prop <= all + 0.3 * LU_PER_YARD
    }

    /// What a kart comes down on under a WoW point (WoW z): the ground, or the water over it.
    pub fn floor(&self, x: f32, y: f32) -> Option<f32> {
        let top = self.lu([x, y, self.origin[2] + DEPTH]);
        let sea = self
            .sea()
            .level(top)
            .map(|l| l / LU_PER_YARD + self.origin[2]);
        match (self.ground(x, y), sea) {
            (Some(g), Some(s)) => Some(g.max(s)),
            (g, s) => g.or(s),
        }
    }

    /// A kart on the ground under WoW `at`, facing WoW `toward`.
    pub fn kart(&self, at: [f32; 3], toward: [f32; 3]) -> Result<fly::Kart> {
        let (a, b) = (self.lu(at), self.lu(toward));
        fly::kart_at(&self.world, Class::Balanced, a, fly::yaw_to(a, b))
            .ok_or_else(|| anyhow!("no ground under the start"))
    }
}

/// A course of the file, by key.
fn course<'a>(file: &'a Value, key: &str) -> Result<&'a Value> {
    file["courses"]
        .as_array()
        .and_then(|cs| cs.iter().find(|c| c["key"] == key))
        .ok_or_else(|| anyhow!("no course {key}"))
}

/// A checkpoint's WoW x, y, z (z required: a ring hangs at it).
fn point(c: &Value) -> Result<[f32; 3]> {
    let n = |k: &str| {
        c[k].as_f64()
            .map(|v| v as f32)
            .ok_or_else(|| anyhow!("checkpoint {k}"))
    };
    Ok([n("x")?, n("y")?, n("z")?])
}

/// The way a checkpoint runs, WoW x, y: between the legs in and out (`kart/race.rs`, `heading`).
fn heading(points: &[[f32; 3]], i: usize) -> [f32; 2] {
    let unit = |a: [f32; 3], b: [f32; 3]| {
        let (x, y) = (b[0] - a[0], b[1] - a[1]);
        let l = x.hypot(y).max(1e-3);
        [x / l, y / l]
    };
    let into = (i > 0).then(|| unit(points[i - 1], points[i]));
    let out = (i + 1 < points.len()).then(|| unit(points[i], points[i + 1]));
    match (into, out) {
        (Some(a), Some(b)) => {
            let (x, y) = (a[0] + b[0], a[1] + b[1]);
            let l = x.hypot(y).max(1e-3);
            [x / l, y / l]
        }
        (Some(a), None) | (None, Some(a)) => a,
        (None, None) => [1.0, 0.0],
    }
}

/// A flight course read for the bot: its checkpoints (WoW), which are rings, and their radii.
pub struct FlightCourse {
    pub map: u32,
    pub points: Vec<[f32; 3]>,
    pub rings: Vec<usize>,
    pub radii: Vec<f32>,
    /// Its turbo pads, WoW x, y, z and facing (`docs/courses.md`, `pads`).
    pub pads: Vec<[f32; 4]>,
}

impl FlightCourse {
    pub fn read(file: &Value, key: &str) -> Result<Self> {
        let c = course(file, key)?;
        let map = c["map"].as_u64().context("map")? as u32;
        let cps = c["checkpoints"].as_array().context("checkpoints")?;
        let mut points = Vec::new();
        let (mut rings, mut radii) = (Vec::new(), Vec::new());
        for (i, cp) in cps.iter().enumerate() {
            points.push(point(cp)?);
            radii.push(cp["gate"].as_f64().unwrap_or(20.0) as f32);
            if cp.get("ring").is_some() {
                rings.push(i);
            }
        }
        let mut pads = Vec::new();
        for p in c["pads"].as_array().into_iter().flatten() {
            let n = |k: &str| p[k].as_f64().map(|v| v as f32);
            let (Some(x), Some(y), Some(o)) = (n("x"), n("y"), n("o")) else {
                continue;
            };
            pads.push([x, y, n("z").unwrap_or(f32::NAN), o]);
        }
        Ok(Self {
            map,
            points,
            rings,
            radii,
            pads,
        })
    }

    /// The rings in the flight's lu, as the client hangs them (pitch and roll 0: upright, across
    /// the way the course runs).
    pub fn rings(&self, sky: &Sky) -> Vec<Ring> {
        self.rings
            .iter()
            .map(|&i| {
                let h = heading(&self.points, i);
                let through = wow_to_bevy([h[0], h[1], 0.0]).normalize();
                Ring {
                    centre: sky.lu(self.points[i]),
                    through,
                    radius: self.radii[i] * LU_PER_YARD,
                }
            })
            .collect()
    }
}

/// How a ring went, in WoW yards: crossed at `z`, `off` from its centre, the ground `under`.
pub struct RingLine {
    pub ring: usize,
    pub passed: bool,
    pub z: f32,
    pub ring_z: f32,
    pub off: f32,
    pub under: Option<f32>,
    /// Whether what is under it is a placed model (a tree's crown, a rock).
    pub on_prop: bool,
    /// When it crossed, seconds from the start.
    pub secs: f32,
}

/// Flies `key`'s rings: each ring as the best flight crossed it, the first missed last.
pub fn fly_course(
    chain: &mut Chain,
    file: &Value,
    key: &str,
) -> Result<(Vec<RingLine>, FlyReport)> {
    fly_course_as(chain, file, key, false)
}

/// As [`fly_course`], or with the race autopilot's pilot when `simple`.
pub fn fly_course_as(
    chain: &mut Chain,
    file: &Value,
    key: &str,
    simple: bool,
) -> Result<(Vec<RingLine>, FlyReport)> {
    let c = FlightCourse::read(file, key)?;
    let sky = Sky::gather(chain, c.map, &c.points)?;
    fly_in_as(&sky, &c, simple)
}

/// Flies `c` in `sky`, gathered round it: each ring as the best flight crossed it, the first
/// missed last.
pub fn fly_in(sky: &Sky, c: &FlightCourse) -> Result<(Vec<RingLine>, FlyReport)> {
    fly_in_as(sky, c, false)
}

/// As [`fly_in`], or with the race autopilot's pilot ([`fly::fly_simple`]) when `simple`.
pub fn fly_in_as(sky: &Sky, c: &FlightCourse, simple: bool) -> Result<(Vec<RingLine>, FlyReport)> {
    // Its pads on the floor under them where they name no height, as the client lays them.
    let pads: Vec<fly::Pad> = c
        .pads
        .iter()
        .map(|p| {
            let z = if p[2].is_finite() {
                p[2]
            } else {
                sky.floor(p[0], p[1]).unwrap_or(0.0)
            };
            let centre = sky.lu([p[0], p[1], z]);
            let ahead = sky.lu([p[0] + p[3].cos(), p[1] + p[3].sin(), z]);
            fly::Pad {
                centre,
                forward: (ahead - centre).normalize(),
            }
        })
        .collect();
    let pads = pads.as_slice();
    let rings = c.rings(sky);
    let first = c.rings.first().map_or(c.points[1], |&i| c.points[i]);
    let kart = sky.kart(c.points[0], first)?;
    let sea = Sea { pads, ..sky.sea() };
    let r = if simple {
        fly::fly_simple(&sky.world, sea, kart, &rings)
    } else {
        fly::fly_rings(&sky.world, sea, kart, &rings)
    };
    let mut lines = Vec::new();
    let crossed = r
        .passes
        .iter()
        .map(|p| (true, p))
        .chain(r.missed.iter().map(|p| (false, p)));
    for (n, (passed, p)) in crossed.enumerate() {
        let at = sky.wow(p.at);
        let i = c.rings[n];
        lines.push(RingLine {
            ring: n + 1,
            passed,
            z: at[2],
            ring_z: c.points[i][2],
            off: p.miss / LU_PER_YARD,
            under: sky.ground(c.points[i][0], c.points[i][1]),
            on_prop: sky.on_prop(c.points[i][0], c.points[i][1]),
            secs: p.tick as f32 / 30.0,
        });
    }
    Ok((lines, r))
}

/// Lays `key`'s rings at the heights the bot reaches, each `margin` yards under the best line
/// through the rings laid before it, and never lower than `clear` yards over the ground under its
/// bottom; the heights (WoW z) of the rings it could lay, the first it could not ending them.
pub fn lay(
    chain: &mut Chain,
    file: &Value,
    key: &str,
    margin: f32,
    clear: f32,
) -> Result<Vec<f32>> {
    let mut c = FlightCourse::read(file, key)?;
    let sky = Sky::gather(chain, c.map, &c.points)?;
    let first = c.rings.first().map_or(c.points[1], |&i| c.points[i]);
    let mut laid = Vec::new();
    for n in 0..c.rings.len() {
        let rings = c.rings(&sky)[..=n].to_vec();
        let kart = sky.kart(c.points[0], first)?;
        let r = fly::fly_reach(&sky.world, sky.sea(), kart, &rings);
        let Some(best) = r.passes.get(n).map(|p| sky.wow(p.at)[2]) else {
            break;
        };
        let i = c.rings[n];
        let ground = sky
            .ground(c.points[i][0], c.points[i][1])
            .unwrap_or(f32::NEG_INFINITY);
        let lowest = ground + c.radii[i] + clear;
        let z = best - margin;
        if z < lowest {
            break;
        }
        c.points[i][2] = z;
        laid.push(z);
    }
    Ok(laid)
}

/// Flies the flight distance `key` as far as the bot gets: the yards across the ground from its
/// launch to the touchdown, and the flight.
pub fn fly_leap(chain: &mut Chain, file: &Value, key: &str) -> Result<(f32, FlyReport)> {
    let l = file["leaps"]
        .as_array()
        .and_then(|ls| ls.iter().find(|l| l["key"] == key))
        .ok_or_else(|| anyhow!("no leap {key}"))?;
    let map = l["map"].as_u64().context("map")? as u32;
    let launch = point(&l["launch"])?;
    let o = l["launch"]["o"].as_f64().context("launch o")? as f32;
    // Far out the way it faces, for the world to gather and the heading.
    let out = [
        launch[0] + o.cos() * 1400.0,
        launch[1] + o.sin() * 1400.0,
        launch[2],
    ];
    let sky = Sky::gather(chain, map, &[launch, out])?;
    let kart = sky.kart(launch, out)?;
    let heading = fly::yaw_to(sky.lu(launch), sky.lu(out));
    let r = fly::fly_far(&sky.world, sky.sea(), kart, heading);
    let yards = r.touchdown.map_or(0.0, |d| {
        let w = sky.wow(d);
        (w[0] - launch[0]).hypot(w[1] - launch[1])
    });
    Ok((yards, r))
}

/// Hangs `key`'s rings at `heights` (WoW z, the rings in order), rounded to a tenth of a yard;
/// its rings past them are dropped, so it finishes at the last ring laid.
pub fn write_heights(file: &mut Value, key: &str, heights: &[f32]) -> Result<()> {
    let c = file["courses"]
        .as_array_mut()
        .and_then(|cs| cs.iter_mut().find(|c| c["key"] == key))
        .ok_or_else(|| anyhow!("no course {key}"))?;
    let cps = c["checkpoints"].as_array_mut().context("checkpoints")?;
    let mut n = 0;
    cps.retain_mut(|cp| {
        if cp.get("ring").is_none() {
            return true;
        }
        let Some(z) = heights.get(n) else {
            return false;
        };
        n += 1;
        cp["z"] = Value::from((f64::from(*z) * 10.0).round() / 10.0);
        true
    });
    anyhow::ensure!(cps.len() >= 2, "{key}: fewer than two checkpoints left");
    Ok(())
}

/// The top-level zone (`AreaTable`'s, as the world map names it) a WoW point on `map` lies in:
/// its map chunk's area, walked up to its zone.
pub fn zone_at(chain: &mut Chain, map: u32, x: f32, y: f32) -> Option<String> {
    let areas = benilla_formats::load_area_table_catalog(chain).ok()?;
    let tiles =
        benilla_formats::load_tiles_around(chain, soup::map_dir(map).ok()?, x, y, 0).ok()?;
    let area = tiles
        .iter()
        .flat_map(|(_, t)| &t.chunks)
        .find_map(|c| c.area_at([x, y, 0.0]))?;
    let zone = areas.top_zone(area)?;
    areas.name(zone).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every flight course of the file is flyable: the bot passes each of its rings, over the
    /// real world.
    #[test]
    fn the_flight_bot_flies_every_flight_course_through_every_ring() {
        let data = benilla_formats::wow_data_or_skip!();
        let mut chain = Chain::open(&data).unwrap();
        let file: Value =
            serde_json::from_str(include_str!("../../../courses/courses.json")).unwrap();
        let flights: Vec<&str> = file["courses"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["flight"] == true)
            .filter_map(|c| c["key"].as_str())
            .collect();
        assert!(!flights.is_empty());
        for key in flights {
            let (lines, _) = fly_course(&mut chain, &file, key).unwrap();
            let rings = FlightCourse::read(&file, key).unwrap().rings.len();
            assert!(
                lines.len() == rings && lines.iter().all(|l| l.passed),
                "{key}: {:?}",
                lines
                    .iter()
                    .map(|l| (l.ring, l.passed, l.z, l.ring_z))
                    .collect::<Vec<_>>()
            );
        }
    }
}
