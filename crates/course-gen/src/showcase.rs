//! Showcase runs authored over the real world (`kart_driver::showcase`): at each famous place a
//! short choreographed drive (figure-8s, donuts, slaloms, long boosted sweeps) is laid out as a
//! turtle path, fitted into the open ground around the place, driven over the world's collision
//! ([`crate::soup`]) by the sliding driver, and kept when it drifts and boosts and never touches a
//! wall. What is kept is the input tape, the start pose and the line it drove, and a tripod spot
//! that sees the run.

use std::f32::consts::TAU;

use anyhow::{anyhow, Result};
use benilla_formats::Chain;
use glam::{Vec2, Vec3};
use kart_driver::showcase::{self, AuthorOpts, Replay, Run, TapeTick};
use kart_driver::{Class, Collision, CELL, LU_PER_YARD};

use crate::soup::{self, bevy_to_wow, wow_to_bevy, Region};

/// A path piece, in yards and degrees: a straight, or an arc to the left or right.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seg {
    S(f32),
    L(f32, f32),
    R(f32, f32),
}

/// A choreography.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A figure-8 round two drift lobes, then away.
    Eight,
    /// A slalom of short alternating slides, into a donut.
    SlalomDonut,
    /// A long boosted sweep across open ground, into a figure-8.
    SweepEight,
    /// Donuts round a point, re-hopping for every three boosts.
    Donut,
    /// A slalom, then a long sweep.
    SlalomSweep,
    /// A long weave of shallow alternating slides, for a narrow way.
    Weave,
}

impl Kind {
    /// What the run does, its slides taking `boosts` boosts each.
    pub fn describe(self, boosts: u8) -> String {
        let chain = match boosts {
            3 => "triple boosts",
            2 => "double boosts",
            _ => "a boost out of every slide",
        };
        match self {
            Kind::Eight => format!("a figure-8 of tight powerslides, {chain} on both lobes"),
            Kind::SlalomDonut => format!("a powerslide slalom into a donut, {chain}"),
            Kind::SweepEight => {
                format!("a long boosted slide across the open ground into a figure-8, {chain}")
            }
            Kind::Donut => format!("tight donuts, hopping into fresh slides for {chain}"),
            Kind::SlalomSweep => format!("a powerslide slalom into a long sweep, {chain}"),
            Kind::Weave => format!("a long weave of alternating powerslides, {chain}"),
        }
    }
}

/// The pieces of `kind` at drift radius `r` (yards), turning first to the left if `left`, after a
/// `lead` yards' run-up from a standstill.
pub fn program(kind: Kind, r: f32, left: bool, lead: f32) -> Vec<Seg> {
    let a = |r: f32, deg: f32, first: bool| {
        if first == left {
            Seg::L(r, deg)
        } else {
            Seg::R(r, deg)
        }
    };
    // A figure-8's crossing straights meet at 2φ; each lobe is a circle tangent to both.
    let eight = |r: f32| {
        let phi = 50.0f32;
        let c = r / phi.to_radians().tan();
        vec![
            Seg::S(c),
            a(r, 180.0 + 2.0 * phi, true),
            Seg::S(2.0 * c),
            a(r, 180.0 + 2.0 * phi, false),
            Seg::S(c),
        ]
    };
    let slalom = |n: usize, turn: f32| {
        let mut v = vec![a(r, turn, true)];
        for i in 1..n {
            v.push(a(r, 2.0 * turn, i % 2 == 0));
        }
        v.push(a(r, turn, n.is_multiple_of(2)));
        v
    };
    let mut segs = vec![Seg::S(lead)];
    match kind {
        Kind::Eight => segs.extend(eight(r)),
        Kind::SlalomDonut => {
            segs.extend(slalom(4, 65.0));
            segs.push(Seg::S(10.0));
            segs.push(a(r, 400.0, true));
        }
        Kind::SweepEight => {
            segs.push(a(r * 1.9, 200.0, false));
            segs.push(Seg::S(14.0));
            segs.extend(eight(r));
        }
        Kind::Donut => segs.push(a(r, 760.0, true)),
        Kind::SlalomSweep => {
            segs.extend(slalom(5, 65.0));
            segs.push(Seg::S(12.0));
            segs.push(a(r * 1.9, 210.0, true));
        }
        Kind::Weave => segs.extend(slalom(8, 42.0)),
    }
    // The straight out before the stop.
    segs.push(Seg::S((lead * 0.7).max(10.0)));
    segs
}

/// The pieces as points a yard apart, from the origin facing +X (2D, yards; +Y is the left).
pub fn trace(segs: &[Seg]) -> Vec<Vec2> {
    let mut p = Vec2::ZERO;
    let mut h = 0.0f32;
    let mut out = vec![p];
    for s in segs {
        match *s {
            Seg::S(len) => {
                let n = len.ceil().max(1.0) as usize;
                let step = len / n as f32;
                for _ in 0..n {
                    p += Vec2::from_angle(h) * step;
                    out.push(p);
                }
            }
            Seg::L(r, deg) | Seg::R(r, deg) => {
                let sign = if matches!(s, Seg::L(..)) { 1.0 } else { -1.0 };
                let sweep = deg.to_radians();
                let n = (r * sweep).ceil().max(1.0) as usize;
                let centre = p + Vec2::from_angle(h + sign * TAU / 4.0) * r;
                let a0 = h - sign * TAU / 4.0;
                for i in 1..=n {
                    let a = a0 + sign * sweep * i as f32 / n as f32;
                    out.push(centre + Vec2::from_angle(a) * r);
                }
                h += sign * sweep;
            }
        }
    }
    out
}

/// A place a showcase runs at.
#[derive(Clone, Copy, Debug)]
pub struct Site {
    pub name: &'static str,
    pub title: &'static str,
    pub map: u32,
    /// The landmark: WoW x, y, z.
    pub spot: [f32; 3],
    /// How far (yards) from the spot the run may start.
    pub reach: f32,
    /// How far above the spot the ground is looked for (under a roof, a little).
    pub probe: f32,
    /// The ground looked at, yards each way from the spot.
    pub span: f32,
    /// How far (yards) the ground may climb or fall from the spot and still be driven.
    pub band: f32,
    /// The clear ground a planned line keeps to either side, yards.
    pub margin: f32,
    /// The lead-in from a standstill, yards.
    pub lead: f32,
    pub kinds: &'static [Kind],
    /// The engine class driven: Turning slides the tightest.
    pub class: Class,
    /// Boosts taken a slide before hopping into the next: three is CTR's whole chain, fewer
    /// keeps the kart slower and its slides tighter where the ground is small.
    pub boosts: u8,
    /// Drift radii tried, yards.
    pub radii: &'static [f32],
}

/// The showcase's places (`/kartshow`): the user's list. A triple-boosted slide runs some 30
/// yards wide at boost speed, so where the spot itself has no such room the run is moved onto the
/// landmark's nearest open ground: the field before Stormwind's gate (a tight one-boost donut in
/// the Turning class, between its lamp posts), the Golden Plains under Thunder Bluff, the foot of
/// Blackrock Mountain on the Burning Steppes side and the shore south of Booty Bay. The Gurubashi
/// Arena's floor takes the same tight one-boost donut.
pub const SITES: &[Site] = &[
    Site {
        name: "mirage",
        title: "Mirage Raceway",
        map: 1,
        spot: [-6221.4, -3927.6, -58.7],
        reach: 90.0,
        probe: 20.0,
        span: 200.0,
        band: 8.0,
        margin: 6.0,
        lead: 38.0,
        kinds: &[Kind::SweepEight, Kind::Eight, Kind::SlalomDonut],
        class: Class::Balanced,
        boosts: 3,
        radii: &[30.0, 34.0, 38.0],
    },
    Site {
        name: "flats",
        title: "Shimmering Flats",
        map: 1,
        spot: [-5588.8, -3752.2, -58.7],
        reach: 90.0,
        probe: 20.0,
        span: 200.0,
        band: 8.0,
        margin: 6.0,
        lead: 38.0,
        kinds: &[Kind::SlalomSweep, Kind::SlalomDonut, Kind::Donut],
        class: Class::Balanced,
        boosts: 3,
        radii: &[28.0, 32.0, 36.0],
    },
    Site {
        name: "aq",
        title: "Gates of Ahn'Qiraj",
        map: 1,
        spot: [-8216.1, 1536.4, 1.3],
        reach: 110.0,
        probe: 20.0,
        span: 200.0,
        band: 12.0,
        margin: 5.0,
        lead: 30.0,
        kinds: &[
            Kind::SweepEight,
            Kind::Eight,
            Kind::SlalomDonut,
            Kind::Donut,
        ],
        class: Class::Balanced,
        boosts: 3,
        radii: &[28.0, 32.0, 36.0],
    },
    Site {
        name: "gurubashi",
        title: "Gurubashi Arena",
        map: 0,
        spot: [-13202.0, 274.0, 22.0],
        reach: 20.0,
        probe: 6.0,
        span: 120.0,
        band: 3.0,
        margin: 3.0,
        lead: 14.0,
        kinds: &[Kind::Donut, Kind::SlalomDonut],
        class: Class::Turn,
        boosts: 1,
        radii: &[16.0, 18.0, 20.0, 22.0, 24.0],
    },
    Site {
        name: "stormwind",
        title: "Stormwind Gate",
        map: 0,
        spot: [-9124.0, 344.0, 94.0],
        reach: 40.0,
        probe: 30.0,
        span: 200.0,
        band: 10.0,
        margin: 2.0,
        lead: 20.0,
        kinds: &[Kind::Donut, Kind::SlalomDonut],
        class: Class::Turn,
        boosts: 1,
        radii: &[16.0, 18.0, 20.0, 22.0],
    },
    Site {
        name: "thunderbluff",
        title: "Thunder Bluff",
        map: 1,
        spot: [-1474.0, 141.0, -3.8],
        reach: 110.0,
        probe: 20.0,
        span: 200.0,
        band: 8.0,
        margin: 5.0,
        lead: 30.0,
        kinds: &[
            Kind::Eight,
            Kind::SlalomDonut,
            Kind::SlalomSweep,
            Kind::Donut,
        ],
        class: Class::Balanced,
        boosts: 3,
        radii: &[28.0, 32.0, 36.0],
    },
    Site {
        name: "blackrock",
        title: "Blackrock Mountain",
        map: 0,
        spot: [-8003.0, -1271.0, 134.0],
        reach: 110.0,
        probe: 30.0,
        span: 200.0,
        band: 10.0,
        margin: 4.0,
        lead: 30.0,
        kinds: &[
            Kind::SlalomSweep,
            Kind::SlalomDonut,
            Kind::Eight,
            Kind::Donut,
        ],
        class: Class::Balanced,
        boosts: 3,
        radii: &[28.0, 32.0, 36.0],
    },
    Site {
        name: "bootybay",
        title: "Booty Bay coast",
        map: 0,
        spot: [-14598.0, 230.0, 1.1],
        reach: 60.0,
        probe: 20.0,
        span: 160.0,
        band: 5.0,
        margin: 3.5,
        lead: 24.0,
        kinds: &[Kind::SlalomDonut, Kind::Donut, Kind::Eight],
        class: Class::Balanced,
        boosts: 3,
        radii: &[26.0, 28.0, 30.0],
    },
];

/// The analysis grid's cell, yards.
const GRID: f32 = 2.0;
/// The faces gathered past the ground looked at, yards, and up and down from the spot.
const GATHER_PAST: f32 = 60.0;
const GATHER_UP: f32 = 70.0;
/// The clear ground the driven line keeps to either side, yards.
const DRIVEN_MARGIN: f32 = 2.0;

/// A place loaded: its faces, the ground round the spot and how clear it is.
pub struct Loaded {
    pub site: Site,
    /// The spot, Bevy world.
    pub spot: Vec3,
    pub faces: soup::Faces,
    /// The faces in lu from the spot.
    pub world: Collision,
    /// Cells along an edge.
    pub n: usize,
    /// The ground's height at the spot (Bevy y from the spot's, yards): the band's middle.
    pub level: f32,
    /// Per cell: the ground height (Bevy y, yards), if any.
    pub height: Vec<Option<f32>>,
    /// Per cell: drivable (ground near the spot's level, not steep, dry, nothing standing on it).
    pub clear: Vec<bool>,
    /// Per cell: yards to the nearest cell that is not clear.
    pub clearance: Vec<f32>,
    /// Per cell: under liquid.
    pub wet: Vec<bool>,
    /// Per cell: something stands on the ground.
    pub blocked: Vec<bool>,
}

impl Loaded {
    pub fn load(chain: &mut Chain, site: Site) -> Result<Self> {
        let spot = wow_to_bevy(site.spot);
        let half = site.span + GATHER_PAST;
        let faces = soup::gather_world(
            chain,
            site.map,
            Region::around(site.spot, Vec3::new(half, GATHER_UP, half)),
        )?;
        let world = Collision::new(soup::to_lu(&faces.solid, spot), CELL);
        let n = (2.0 * site.span / GRID) as usize;
        let mut height = vec![None; n * n];
        let mut steep = vec![false; n * n];
        let mut blocked = vec![false; n * n];
        let mut wet = vec![false; n * n];
        let water = Collision::new(soup::to_lu(&faces.liquid, spot), CELL);
        for r in 0..n {
            for c in 0..n {
                let i = r * n + c;
                let xz = cell_centre(site.span, c, r);
                let from = Vec3::new(xz.x, site.probe, xz.y) * LU_PER_YARD;
                let Some(hit) = world.ground(from, (site.probe + 40.0) * LU_PER_YARD) else {
                    continue;
                };
                let h = hit.point.y / LU_PER_YARD;
                height[i] = Some(h);
                steep[i] = hit.normal.y < 0.86;
                // Anything solid where the kart and its driver would be.
                let body = hit.point + Vec3::Y * 1.5 * LU_PER_YARD;
                blocked[i] = world.deepest(body, 1.2 * LU_PER_YARD).is_some();
                if let Some(w) = water.ray(from, Vec3::NEG_Y, (site.probe + 40.0) * LU_PER_YARD) {
                    wet[i] = w.point.y > hit.point.y + 0.2 * LU_PER_YARD;
                }
            }
        }
        let level = height[(n / 2) * n + n / 2].unwrap_or(0.0);
        let mut clear = vec![false; n * n];
        for r in 0..n {
            for c in 0..n {
                let i = r * n + c;
                let Some(h) = height[i] else { continue };
                if steep[i] || blocked[i] || wet[i] || (h - level).abs() > site.band {
                    continue;
                }
                // A step to a neighbour a kart cannot roll over is an edge.
                let smooth = [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)]
                    .iter()
                    .all(|(dc, dr)| {
                        let (cc, rr) = (c as i32 + dc, r as i32 + dr);
                        if cc < 0 || rr < 0 || cc >= n as i32 || rr >= n as i32 {
                            return false;
                        }
                        height[rr as usize * n + cc as usize].is_some_and(|o| (o - h).abs() < 0.9)
                    });
                clear[i] = smooth;
            }
        }
        let clearance = distance_field(&clear, n);
        Ok(Self {
            site,
            spot,
            faces,
            world,
            n,
            level,
            height,
            clear,
            clearance,
            wet,
            blocked,
        })
    }

    /// The cell under a spot-relative point (yards, Bevy x and z).
    fn cell(&self, p: Vec2) -> Option<usize> {
        let n = self.n as i32;
        let c = ((p.x + self.site.span) / GRID).floor() as i32;
        let r = ((p.y + self.site.span) / GRID).floor() as i32;
        (c >= 0 && r >= 0 && c < n && r < n).then(|| (r * n + c) as usize)
    }

    /// The clearance under a spot-relative point, yards; 0 off the grid.
    pub fn clearance_at(&self, p: Vec2) -> f32 {
        self.cell(p).map_or(0.0, |i| self.clearance[i])
    }

    pub fn height_at(&self, p: Vec2) -> Option<f32> {
        self.cell(p).and_then(|i| self.height[i])
    }
}

/// A cell's centre, yards from the spot (Bevy x, z).
fn cell_centre(span: f32, c: usize, r: usize) -> Vec2 {
    Vec2::new(
        -span + (c as f32 + 0.5) * GRID,
        -span + (r as f32 + 0.5) * GRID,
    )
}

/// Yards from each cell to the nearest cell that is not `clear` (a two-pass chamfer).
fn distance_field(clear: &[bool], n: usize) -> Vec<f32> {
    let big = 1.0e6;
    let mut d: Vec<f32> = clear.iter().map(|&c| if c { big } else { 0.0 }).collect();
    let diag = GRID * std::f32::consts::SQRT_2;
    let at = |d: &Vec<f32>, c: i32, r: i32| -> f32 {
        if c < 0 || r < 0 || c >= n as i32 || r >= n as i32 {
            0.0
        } else {
            d[r as usize * n + c as usize]
        }
    };
    for r in 0..n as i32 {
        for c in 0..n as i32 {
            let i = r as usize * n + c as usize;
            let v = d[i]
                .min(at(&d, c - 1, r) + GRID)
                .min(at(&d, c, r - 1) + GRID)
                .min(at(&d, c - 1, r - 1) + diag)
                .min(at(&d, c + 1, r - 1) + diag);
            d[i] = v;
        }
    }
    for r in (0..n as i32).rev() {
        for c in (0..n as i32).rev() {
            let i = r as usize * n + c as usize;
            let v = d[i]
                .min(at(&d, c + 1, r) + GRID)
                .min(at(&d, c, r + 1) + GRID)
                .min(at(&d, c + 1, r + 1) + diag)
                .min(at(&d, c - 1, r + 1) + diag);
            d[i] = v;
        }
    }
    d
}

/// A laid-out run: the choreography, where it starts and which way it faces.
#[derive(Clone, Debug)]
pub struct Layout {
    pub kind: Kind,
    pub radius: f32,
    pub left: bool,
    /// The start, yards from the spot (Bevy x, z), and the facing (radians, Bevy x-z angle).
    pub at: Vec2,
    pub angle: f32,
    /// The planned line, yards from the spot (Bevy x, z).
    pub line: Vec<Vec2>,
    /// The least clearance along it.
    pub min_clear: f32,
}

/// Every layout whose planned line keeps the site's margin of clear ground.
pub fn layouts(site: &Loaded) -> Vec<Layout> {
    let mut out = Vec::new();
    let step = 6.0;
    let reach = site.site.reach;
    let k = (reach / step) as i32;
    for &kind in site.site.kinds {
        for &radius in site.site.radii {
            for left in [true, false] {
                let local = trace(&program(kind, radius, left, site.site.lead));
                for a in 0..24 {
                    let angle = a as f32 / 24.0 * TAU;
                    let rot = Vec2::from_angle(angle);
                    let mut starts: Vec<Vec2> = (-k..=k)
                        .flat_map(|i| (-k..=k).map(move |j| Vec2::new(i as f32, j as f32) * step))
                        .collect();
                    // Donuts also round the spot itself, as near as two yards.
                    if kind == Kind::Donut {
                        let side = if left { radius } else { -radius };
                        let lead = rot.rotate(Vec2::new(site.site.lead, side));
                        for i in -3..=3 {
                            for j in -3..=3 {
                                starts.push(Vec2::new(i as f32, j as f32) * 2.0 - lead);
                            }
                        }
                    }
                    for at in starts {
                        {
                            if (at.length() > reach && kind != Kind::Donut)
                                || at.length() > reach + 2.0 * radius
                                || site.clearance_at(at) < site.site.margin
                            {
                                continue;
                            }
                            let mut min_clear = f32::INFINITY;
                            let ok = local.iter().step_by(2).all(|p| {
                                let q = at + rot.rotate(*p);
                                let c = site.clearance_at(q);
                                min_clear = min_clear.min(c);
                                c >= site.site.margin
                            });
                            if ok {
                                out.push(Layout {
                                    kind,
                                    radius,
                                    left,
                                    at,
                                    angle,
                                    line: local.iter().map(|p| at + rot.rotate(*p)).collect(),
                                    min_clear,
                                });
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

/// A layout driven.
#[derive(Clone, Debug)]
pub struct Drive {
    pub layout: Layout,
    pub ticks: Vec<TapeTick>,
    pub replay: Replay,
    /// The least clearance along the driven line, yards.
    pub driven_clear: f32,
    /// How far the kart's height strayed from the spot's, yards.
    pub score: f32,
}

/// The kart's frame for a start: lu from it, as the client's session gathers the world.
pub fn world_from(site: &Loaded, start: Vec3) -> Collision {
    Collision::new(soup::to_lu(&site.faces.solid, start), CELL)
}

/// The start of a layout, Bevy world: on the ground at its first point, at the hundredths of a
/// yard the file keeps.
pub fn start_of(site: &Loaded, layout: &Layout) -> Option<Vec3> {
    let h = site.height_at(layout.at)?;
    let at = site.spot + Vec3::new(layout.at.x, h + 0.3, layout.at.y);
    Some(wow_to_bevy(bevy_to_wow(at).map(round2)))
}

/// The heading (CTR's 4096ths) facing along a Bevy x-z angle: CTR's forward `(sin h, cos h)`
/// is Bevy `(x, z)`.
pub fn heading_of(angle: f32) -> i32 {
    showcase::heading_along(Vec2::from_angle(angle))
}

/// Drives `layout` in the frame of its own start, as the client will.
pub fn drive_layout(site: &Loaded, layout: &Layout, opts: &AuthorOpts) -> Option<Drive> {
    let start = start_of(site, layout)?;
    let world = world_from(site, start);
    let lu = |p: Vec2| -> Vec3 {
        let q = site.spot + Vec3::new(p.x, 0.0, p.y);
        let h = site.height_at(p).unwrap_or(start.y - site.spot.y);
        (Vec3::new(q.x, site.spot.y + h, q.z) - start) * LU_PER_YARD
    };
    let waypoints: Vec<Vec3> = layout.line.iter().step_by(3).map(|p| lu(*p)).collect();
    let heading = heading_of(layout.angle);
    let ticks = showcase::author(&world, &waypoints, heading, opts)?;
    let replay = showcase::replay(&world, opts.class, heading, &ticks)?;
    let driven_clear = replay
        .path
        .iter()
        .map(|p| {
            let b = start + Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32) / LU_PER_YARD;
            site.clearance_at(Vec2::new(b.x - site.spot.x, b.z - site.spot.z))
        })
        .fold(f32::INFINITY, f32::min);
    let secs = ticks.len() as f32 * ctr_kart_tick();
    let centre = layout.line.iter().copied().sum::<Vec2>() / layout.line.len() as f32;
    let mut score = replay.boosts as f32 * 10.0 + replay.slides as f32 * 2.0
        - centre.length() * 0.15
        + driven_clear.min(10.0);
    if !(10.0..=25.0).contains(&secs) {
        score -= 40.0;
    }
    // The site's choreographies are listed best first.
    let kinds = site.site.kinds;
    if let Some(i) = kinds.iter().position(|k| *k == layout.kind) {
        score += 15.0 * (kinds.len() - 1 - i) as f32;
    }
    Some(Drive {
        layout: layout.clone(),
        ticks,
        replay,
        driven_clear,
        score,
    })
}

fn ctr_kart_tick() -> f32 {
    1.0 / 30.0
}

/// The clear ground the driven line keeps at a site: [`DRIVEN_MARGIN`], or the site's own margin
/// where that is tighter (the arena's floor).
fn driven_margin(site: &Site) -> f32 {
    DRIVEN_MARGIN.min(site.margin)
}

/// Whether a drive is fit to keep: no wall touched, no spin, no drop, clear ground all along and
/// at least [`MIN_BOOSTS`] boosts.
pub fn clean(site: &Site, d: &Drive) -> bool {
    d.replay.wall_contacts == 0
        && d.replay.spins == 0
        && d.replay.longest_air < 0.6
        && d.driven_clear >= driven_margin(site)
        && d.replay.boosts >= MIN_BOOSTS
}

/// The fewest boosts a kept run has.
pub const MIN_BOOSTS: u32 = 6;

/// The best clean drive among `layouts` (tried best-cleared first, at most `budget`).
pub fn best_drive(
    site: &Loaded,
    layouts: &mut [Layout],
    budget: usize,
    tally: &mut String,
    nearest: &mut Option<Drive>,
) -> Option<Drive> {
    let mut opts = AuthorOpts {
        class: site.site.class,
        ..AuthorOpts::default()
    };
    opts.slide.max_boosts = site.site.boosts;
    // Spread the tries over the choreographies and radii, most clearance first in each.
    layouts.sort_by(|a, b| b.min_clear.total_cmp(&a.min_clear));
    let mut best: Option<Drive> = None;
    let mut tried = 0;
    let mut per_kind: std::collections::HashMap<(u8, u32), usize> = Default::default();
    let mut why = [0usize; 6];
    for l in layouts.iter() {
        let key = (l.kind as u8, l.radius as u32);
        let n = per_kind.entry(key).or_default();
        if *n > budget / 6 {
            continue;
        }
        *n += 1;
        tried += 1;
        if tried > budget {
            break;
        }
        let Some(d) = drive_layout(site, l, &opts) else {
            why[5] += 1;
            continue;
        };
        why[0] += usize::from(d.replay.wall_contacts > 0);
        why[1] += usize::from(d.replay.spins > 0);
        why[2] += usize::from(d.replay.longest_air >= 0.6);
        why[3] += usize::from(d.driven_clear < driven_margin(&site.site));
        why[4] += usize::from(d.replay.boosts < MIN_BOOSTS);
        let near_key = |d: &Drive| (d.replay.wall_contacts, -(d.score as i32));
        if nearest.as_ref().is_none_or(|n| near_key(&d) < near_key(n)) {
            *nearest = Some(d.clone());
        }
        if clean(&site.site, &d) && best.as_ref().is_none_or(|b| d.score > b.score) {
            best = Some(d);
        }
    }
    *tally = format!(
        "{tried} driven: walls {}, spins {}, drops {}, off the clear {}, few boosts {}, no start {}",
        why[0], why[1], why[2], why[3], why[4], why[5]
    );
    best
}

/// Where the tripod stands: a spot off the run that sees the kart all along, Bevy world.
pub fn tripod(site: &Loaded, start: Vec3, path: &[[i32; 4]]) -> (Vec3, f32) {
    let world = world_from(site, start);
    let pts: Vec<Vec3> = path
        .iter()
        .step_by(4)
        .map(|p| Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32))
        .collect();
    let centre = pts.iter().copied().sum::<Vec3>() / pts.len().max(1) as f32;
    let extent = pts
        .iter()
        .map(|p| (*p - centre).with_y(0.0).length())
        .fold(0.0, f32::max)
        / LU_PER_YARD;
    let mut best = (start + Vec3::Y * 2.0, -1.0f32);
    // Inside the run (where it leaves room) or round it.
    for ring in [-0.55 * extent, 12.0, 22.0, 34.0] {
        for a in 0..32 {
            let dir = Vec2::from_angle(a as f32 / 32.0 * TAU);
            let flat = centre / LU_PER_YARD + Vec3::new(dir.x, 0.0, dir.y) * (extent + ring);
            // Never where the kart drives.
            let off_line = pts
                .iter()
                .map(|p| (*p / LU_PER_YARD - flat).with_y(0.0).length())
                .fold(f32::INFINITY, f32::min);
            if off_line < 6.0 {
                continue;
            }
            let b = start + flat.with_y(0.0);
            let rel = Vec2::new(b.x - site.spot.x, b.z - site.spot.z);
            let Some(h) = site.height_at(rel) else {
                continue;
            };
            let eye_b = Vec3::new(b.x, site.spot.y + h + 2.2, b.z);
            let eye = (eye_b - start) * LU_PER_YARD;
            // Not inside anything.
            if world.deepest(eye, 0.6 * LU_PER_YARD).is_some() {
                continue;
            }
            let seen = pts
                .iter()
                .filter(|p| {
                    let target = **p + Vec3::Y * 1.0 * LU_PER_YARD;
                    let to = target - eye;
                    let len = to.length();
                    world
                        .ray(eye, to, len)
                        .is_none_or(|hit| hit.distance > len - 0.8 * LU_PER_YARD)
                })
                .count() as f32
                / pts.len().max(1) as f32;
            // Better seen, then nearer the landmark's side.
            let toward = -(rel.length()) * 0.001;
            if seen + toward > best.1 {
                best = (eye_b, seen + toward);
            }
        }
    }
    best
}

/// The run kept for a site: the drive in its start's frame, its tripod and cameras.
pub fn make_run(site: &Loaded, d: &Drive) -> Result<Run> {
    let start = start_of(site, &d.layout).ok_or_else(|| anyhow!("no ground at the start"))?;
    let (tripod, seen) = tripod(site, start, &d.replay.path);
    // The tripod leads where it sees most of the run; the orbit where nothing does.
    let mut cameras: Vec<String> = if seen >= 0.85 {
        vec!["tripod", "low", "orbit", "chase"]
    } else {
        vec!["orbit", "low", "chase", "tripod"]
    }
    .into_iter()
    .map(String::from)
    .collect();
    if d.layout.kind == Kind::Donut && seen >= 0.85 {
        cameras.swap(1, 2);
    }
    let start_wow = bevy_to_wow(start);
    Ok(Run {
        name: site.site.name.into(),
        title: site.site.title.into(),
        note: d.layout.kind.describe(site.site.boosts),
        map: site.site.map,
        start: start_wow.map(round2),
        heading: heading_of(d.layout.angle),
        class: class_key(site.site.class).into(),
        cameras,
        tripod: bevy_to_wow(tripod).map(round2),
        boosts: d.replay.boosts,
        slides: d.replay.slides,
        seconds: (d.ticks.len() as f32 / 30.0 * 10.0).round() / 10.0,
        tape: showcase::encode(&d.ticks),
        path: d.replay.path.clone(),
    })
}

fn round2(v: f32) -> f32 {
    (v * 100.0).round() / 100.0
}

/// A run read back: its faces in its own start's frame (the start rounded as the file keeps it),
/// replayed from its tape.
pub fn verify(chain: &mut Chain, run: &Run) -> Result<Replay> {
    let start = wow_to_bevy(run.start);
    let region = Region::around(run.start, Vec3::new(220.0, GATHER_UP, 220.0));
    let faces = soup::gather_world(chain, run.map, region)?;
    let world = Collision::new(soup::to_lu(&faces.solid, start), CELL);
    let ticks = run.ticks().map_err(|e| anyhow!(e))?;
    showcase::replay(&world, run.class(), run.heading, &ticks)
        .ok_or_else(|| anyhow!("{}: no ground at the start", run.name))
}

/// A class's key, as a run keeps it ([`Run::class`]).
pub fn class_key(class: Class) -> &'static str {
    match class {
        Class::Balanced => "balanced",
        Class::Accel => "accel",
        Class::Speed => "speed",
        Class::Turn => "turn",
    }
}

/// The showcase file as written: one run a block, each field on its own line and the long ones
/// (the tape, the path) on one.
pub fn format(file: &showcase::ShowcaseFile) -> Result<String> {
    let mut out = String::from("{\n  \"runs\": [\n");
    for (i, run) in file.runs.iter().enumerate() {
        // Field by field, each in its own type: a float as the f32 it is ("-58.45"), not
        // widened to an f64's digits.
        let fields = [
            ("name", j(&run.name)?),
            ("title", j(&run.title)?),
            ("note", j(&run.note)?),
            ("map", j(&run.map)?),
            ("start", j(&run.start)?),
            ("heading", j(&run.heading)?),
            ("class", j(&run.class)?),
            ("cameras", j(&run.cameras)?),
            ("tripod", j(&run.tripod)?),
            ("boosts", j(&run.boosts)?),
            ("slides", j(&run.slides)?),
            ("seconds", j(&run.seconds)?),
            ("tape", j(&run.tape)?),
            ("path", j(&run.path)?),
        ];
        out.push_str("    {\n");
        for (n, (k, v)) in fields.iter().enumerate() {
            let comma = if n + 1 < fields.len() { "," } else { "" };
            out.push_str(&format!("      \"{k}\": {v}{comma}\n"));
        }
        out.push_str(if i + 1 < file.runs.len() {
            "    },\n"
        } else {
            "    }\n"
        });
    }
    out.push_str("  ]\n}\n");
    Ok(out)
}

/// One value as compact JSON.
fn j<T: serde::Serialize>(v: &T) -> serde_json::Result<String> {
    serde_json::to_string(v)
}

/// A top-down picture of a site, a pixel a yard: clear ground green by height, steep grey,
/// blocked red, water blue, nothing black; the planned line white, the driven one yellow, the
/// start magenta, the tripod cyan and the spot orange.
pub fn render(
    site: &Loaded,
    half: f32,
    px_per_yard: f32,
    planned: &[Vec2],
    driven: &[Vec2],
    marks: &[(Vec2, [u8; 3])],
) -> image::RgbImage {
    let size = (2.0 * half * px_per_yard) as u32;
    let mut img = image::RgbImage::new(size, size);
    let to_rel = |x: u32, y: u32| {
        Vec2::new(
            (x as f32 + 0.5) / px_per_yard - half,
            (y as f32 + 0.5) / px_per_yard - half,
        )
    };
    for y in 0..size {
        for x in 0..size {
            let p = to_rel(x, y);
            let mut px = match site.cell(p) {
                None => [0, 0, 0],
                Some(i) => match site.height[i] {
                    None => [0, 0, 0],
                    Some(_) if site.wet[i] => [30, 60, 170],
                    Some(_) if site.blocked[i] => [170, 30, 30],
                    Some(h) if site.clear[i] => {
                        let t = (((h - site.level) / site.site.band) * 0.5 + 0.5).clamp(0.0, 1.0);
                        [40, (120.0 + 100.0 * t) as u8, 40]
                    }
                    // Out of the site's height band dark, steep or an edge light.
                    Some(h) if (h - site.level).abs() > site.site.band => [55, 55, 55],
                    Some(_) => [140, 140, 140],
                },
            };
            // A faint line every ten yards, for scale.
            if (p.x.rem_euclid(10.0) < 1.0 / px_per_yard)
                || (p.y.rem_euclid(10.0) < 1.0 / px_per_yard)
            {
                px = px.map(|c| c.saturating_add(25));
            }
            img.put_pixel(x, y, image::Rgb(px));
        }
    }
    let mut dot = |p: Vec2, col: [u8; 3], radius: i32| {
        let x = ((p.x + half) * px_per_yard) as i32;
        let y = ((p.y + half) * px_per_yard) as i32;
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                let (xx, yy) = (x + dx, y + dy);
                if xx >= 0 && yy >= 0 && (xx as u32) < size && (yy as u32) < size {
                    img.put_pixel(xx as u32, yy as u32, image::Rgb(col));
                }
            }
        }
    };
    for p in planned {
        dot(*p, [255, 255, 255], 0);
    }
    for w in driven.windows(2) {
        let n = ((w[1] - w[0]).length() * px_per_yard).ceil().max(1.0) as usize;
        for k in 0..=n {
            dot(w[0].lerp(w[1], k as f32 / n as f32), [255, 230, 0], 0);
        }
    }
    for (p, col) in marks {
        dot(*p, *col, 2);
    }
    dot(Vec2::ZERO, [255, 140, 0], 2);
    img
}

/// `course-gen showcase`: authors the named sites (every site with none), draws each into `out`
/// (`<name>.png`), and with `write` puts the runs into that showcase file, each replacing the run
/// of its name. Returns the report.
/// What `course-gen showcase` may change in a site for a try: its spot and how its ground is read.
#[derive(Clone, Copy, Debug, Default)]
pub struct Overrides {
    pub spot: Option<[f32; 3]>,
    pub probe: Option<f32>,
    pub band: Option<f32>,
    pub span: Option<f32>,
    pub reach: Option<f32>,
    pub margin: Option<f32>,
    pub class: Option<Class>,
    pub boosts: Option<u8>,
    pub radii: Option<&'static [f32]>,
}

impl Overrides {
    fn apply(&self, mut s: Site) -> Site {
        s.spot = self.spot.unwrap_or(s.spot);
        s.probe = self.probe.unwrap_or(s.probe);
        s.band = self.band.unwrap_or(s.band);
        s.span = self.span.unwrap_or(s.span);
        s.reach = self.reach.unwrap_or(s.reach);
        s.margin = self.margin.unwrap_or(s.margin);
        s.class = self.class.unwrap_or(s.class);
        s.boosts = self.boosts.unwrap_or(s.boosts);
        s.radii = self.radii.unwrap_or(s.radii);
        s
    }
}

pub fn author_sites(
    chain: &mut Chain,
    names: &[String],
    out: &std::path::Path,
    write: Option<&std::path::Path>,
    budget: usize,
    overrides: &Overrides,
) -> Result<String> {
    std::fs::create_dir_all(out)?;
    let mut log = String::new();
    let mut runs = Vec::new();
    for site in SITES
        .iter()
        .filter(|s| names.is_empty() || names.iter().any(|n| n == s.name))
    {
        let t0 = std::time::Instant::now();
        let site = &overrides.apply(*site);
        let loaded = Loaded::load(chain, *site)?;
        let mut all = layouts(&loaded);
        let fitted = all.len();
        let mut tally = String::new();
        let mut nearest = None;
        let best = best_drive(&loaded, &mut all, budget, &mut tally, &mut nearest);
        let mut marks = Vec::new();
        let (planned, driven) = match &best {
            Some(d) => {
                let start = start_of(&loaded, &d.layout).unwrap_or(loaded.spot);
                let rel = |b: Vec3| Vec2::new(b.x - loaded.spot.x, b.z - loaded.spot.z);
                let driven: Vec<Vec2> = d
                    .replay
                    .path
                    .iter()
                    .map(|p| {
                        rel(start + Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32) / LU_PER_YARD)
                    })
                    .collect();
                let run = make_run(&loaded, d)?;
                marks.push((rel(start), [255, 0, 255]));
                marks.push((rel(wow_to_bevy(run.tripod)), [0, 255, 255]));
                log.push_str(&format!(
                    "{}: {} (r {} yd, {}), {:.1} s, {} slides, {} boosts, clear {:.1} yd, \
                     start {:?} heading {}, tripod {:?}, cameras {:?}; {} layouts fitted, {:.1} s\n",
                    site.name,
                    d.layout.kind.describe(site.boosts),
                    d.layout.radius,
                    if d.layout.left { "left first" } else { "right first" },
                    run.seconds,
                    run.slides,
                    run.boosts,
                    d.driven_clear,
                    run.start,
                    run.heading,
                    run.tripod,
                    run.cameras,
                    fitted,
                    t0.elapsed().as_secs_f32()
                ));
                runs.push(run);
                (d.layout.line.clone(), driven)
            }
            None => {
                log.push_str(&format!(
                    "{}: no clean run ({} layouts fitted; {})\n",
                    site.name, fitted, tally
                ));
                // Where the open ground is, for another try: the most open cells, apart.
                let mut open: Vec<(f32, usize)> = loaded
                    .clearance
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| **c >= 12.0)
                    .map(|(i, c)| (*c, i))
                    .collect();
                open.sort_by(|a, b| b.0.total_cmp(&a.0));
                let mut picked: Vec<Vec2> = Vec::new();
                for (c, i) in open {
                    let p = cell_centre(site.span, i % loaded.n, i / loaded.n);
                    if picked.iter().any(|q| q.distance(p) < 40.0) {
                        continue;
                    }
                    picked.push(p);
                    let h = loaded.height[i].unwrap_or(0.0);
                    let w = bevy_to_wow(loaded.spot + Vec3::new(p.x, h, p.y));
                    log.push_str(&format!(
                        "  open {c:.0} yd at {:.0},{:.0},{:.1}\n",
                        w[0], w[1], w[2]
                    ));
                    if picked.len() >= 8 {
                        break;
                    }
                }
                // The nearest miss, to see where it went wrong.
                match &nearest {
                    Some(d) => {
                        let start = start_of(&loaded, &d.layout).unwrap_or(loaded.spot);
                        let rel = |b: Vec3| Vec2::new(b.x - loaded.spot.x, b.z - loaded.spot.z);
                        for w in &d.replay.wall_points {
                            marks.push((rel(start + *w / LU_PER_YARD), [255, 0, 0]));
                        }
                        let far = d
                            .replay
                            .path
                            .iter()
                            .map(|p| {
                                rel(start
                                    + Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32)
                                        / LU_PER_YARD)
                                .length()
                            })
                            .fold(0.0, f32::max);
                        log.push_str(&format!(
                            "  nearest miss: {:?} r {} at {:?}: {} walls, first at {:?} yd \
                             from the spot; farthest {far:.0} yd; {} boosts\n",
                            d.layout.kind,
                            d.layout.radius,
                            d.layout.at,
                            d.replay.wall_contacts,
                            d.replay
                                .wall_points
                                .first()
                                .map(|w| rel(start + *w / LU_PER_YARD)),
                            d.replay.boosts
                        ));
                        let driven = d
                            .replay
                            .path
                            .iter()
                            .map(|p| {
                                rel(start
                                    + Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32)
                                        / LU_PER_YARD)
                            })
                            .collect();
                        (d.layout.line.clone(), driven)
                    }
                    None => (Vec::new(), Vec::new()),
                }
            }
        };
        render(&loaded, site.span, 1.0, &planned, &driven, &marks)
            .save(out.join(format!("{}.png", site.name)))?;
        render(&loaded, 70.0, 5.0, &planned, &driven, &marks)
            .save(out.join(format!("{}-zoom.png", site.name)))?;
    }
    if let Some(path) = write {
        let mut file = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| showcase::ShowcaseFile::from_json(&t).ok())
            .unwrap_or_default();
        for run in runs {
            match file.runs.iter_mut().find(|r| r.name == run.name) {
                Some(old) => *old = run,
                None => file.runs.push(run),
            }
        }
        // In the sites' order.
        file.runs.sort_by_key(|r| {
            SITES
                .iter()
                .position(|s| s.name == r.name)
                .unwrap_or(usize::MAX)
        });
        std::fs::write(path, format(&file)?)?;
        log.push_str(&format!("wrote {}\n", path.display()));
    }
    Ok(log)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_figure_eight_comes_back_through_its_crossing() {
        let segs = program(Kind::Eight, 30.0, true, 38.0);
        let pts = trace(&segs);
        // After the lead-in, the figure-8 starts and ends at its crossing, 38 yards on.
        let crossing = Vec2::new(38.0, 0.0);
        let near = pts
            .iter()
            .rev()
            .find(|p| (**p - crossing).length() < 1.5)
            .copied();
        assert!(near.is_some(), "the 8 closes at {crossing:?}");
        // Its two lobes lie either side of the line of travel.
        assert!(pts.iter().any(|p| p.y > 20.0) && pts.iter().any(|p| p.y < -20.0));
    }

    #[test]
    fn the_trace_turns_left_for_l() {
        let pts = trace(&[Seg::L(10.0, 90.0)]);
        let end = *pts.last().unwrap();
        assert!((end - Vec2::new(10.0, 10.0)).length() < 0.05, "{end:?}");
    }

    #[test]
    fn the_distance_field_measures_to_the_nearest_obstacle() {
        let n = 21;
        let mut clear = vec![true; n * n];
        clear[10 * n + 10] = false;
        let d = distance_field(&clear, n);
        assert_eq!(d[10 * n + 10], 0.0);
        assert!((d[10 * n + 13] - 3.0 * GRID).abs() < 1e-3);
    }

    #[test]
    fn every_showcase_run_drives_its_tape_clean_over_the_real_world() {
        let data = benilla_formats::wow_data_or_skip!();
        let file =
            showcase::ShowcaseFile::from_json(include_str!("../../../showcase/showcase.json"))
                .unwrap();
        assert_eq!(file.runs.len(), SITES.len());
        let mut chain = Chain::open(&data).unwrap();
        for run in &file.runs {
            let r = verify(&mut chain, run).unwrap();
            let ticks = run.ticks().unwrap();
            assert_eq!(r.ticks, ticks.len(), "{}: to its end", run.name);
            assert_eq!(
                r.wall_contacts, 0,
                "{}: walls at {:?}",
                run.name, r.wall_points
            );
            assert_eq!(r.spins, 0, "{}", run.name);
            assert!(r.boosts >= MIN_BOOSTS, "{}: {} boosts", run.name, r.boosts);
            assert_eq!(r.boosts, run.boosts, "{}", run.name);
            assert!(r.end_speed < 100.0, "{}: stops", run.name);
            assert!(
                (10.0..=25.0).contains(&(ticks.len() as f32 / 30.0)),
                "{}: {} s",
                run.name,
                ticks.len() as f32 / 30.0
            );
            // Tick for tick the line the file keeps.
            assert_eq!(r.path, run.path, "{}", run.name);
        }
    }
}
