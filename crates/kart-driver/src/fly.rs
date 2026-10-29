//! A bot that flies a kart (`ctr-kart`'s flight, DKR's glider) through a course's rings, or as far
//! as it goes off a launch, to find what a player can fly. It searches its controls rather than
//! keeping to a rule: every [`CHUNK`] ticks each kept flight tries each of [`MOVES`] (the glide on
//! the gas, the dive off it, the pull-up on ↓), and the [`BEAM`] best go on, by rings passed, then
//! the energy kept (height and speed) against the next ring, then the way made to it. It steers by
//! pure pursuit to the next ring, opens the wings with the double tap as soon as it may, and fires
//! no turbo and slides no powerslide: what it flies, a player on a pad can.

use std::f32::consts::TAU;

use ctr_kart::stats::GRAVITY;
pub use ctr_kart::Kart;
use ctr_kart::{Class, Collision, Event, Input, Modifiers};
use glam::{Vec2, Vec3};

/// Ticks a move holds before the search tries the next.
pub const CHUNK: usize = 8;
/// Flights kept at each step.
pub const BEAM: usize = 96;
/// A flight's last tick: three minutes at 30 Hz.
const MAX_TICKS: usize = 5400;
/// How far past a ring's edge a pass still counts, lu (the client's `RING_REACH`, a yard).
const RING_REACH: f32 = 36.6;
/// How high over the ground (lu) the bot opens its wings: past an edge, not in a hop's air.
const TAP_OVER: f32 = 200.0;
/// The steepest a dive comes down, height over the ground covered: the dive's pitch (`ctr-kart`'s
/// `FLY_DIVE_PITCH`, 22.5°) with some of the sink, kept short of it.
const DIVE_SLOPE: f32 = 0.35;
/// The share of gravity along the nose (`ctr-kart`'s `FLY_NOSE_GRAVITY`): what a dive turns into
/// speed and a climb back into height.
const NOSE_GRAVITY: f32 = 0.6;

/// A ring: its centre and the way through it (unit), lu, and its radius.
#[derive(Clone, Copy, Debug)]
pub struct Ring {
    pub centre: Vec3,
    pub through: Vec3,
    pub radius: f32,
}

/// The controls a move holds, steering aside.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    /// On the gas: the nose at the glide's pitch.
    Glide,
    /// Off the gas: the nose down.
    Dive,
    /// ↓ on the gas: the nose up.
    Climb,
}

pub const MOVES: [Move; 3] = [Move::Glide, Move::Dive, Move::Climb];

/// A ring as flown: the tick, how far from its centre (lu) and how high over the ground (lu).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pass {
    pub tick: usize,
    pub miss: f32,
    pub over_ground: f32,
    /// Where it crossed, lu.
    pub at: Vec3,
}

/// How the best flight went.
#[derive(Clone, Debug, Default)]
pub struct FlyReport {
    /// Each ring passed, in order; a ring missed ends the list.
    pub passes: Vec<Pass>,
    /// Ticks flown (to the touchdown, the last ring or the end).
    pub ticks: usize,
    /// The flight's path, a point every [`CHUNK`] ticks, lu.
    pub path: Vec<Vec3>,
    /// Where it first touched down after the wings opened, lu.
    pub touchdown: Option<Vec3>,
    /// The moves it flew, one a chunk.
    pub moves: Vec<Move>,
    /// The next ring as the nearest flight missed it, when none passed it: where it crossed the
    /// ring's plane and how far from its centre.
    pub missed: Option<Pass>,
}

#[derive(Clone)]
struct Flight {
    kart: Kart,
    tick: usize,
    passed: usize,
    passes: Vec<Pass>,
    flew: bool,
    down: Option<Vec3>,
    lost: bool,
    missed: Option<Pass>,
    taps: u8,
    path: Vec<Vec3>,
    moves: Vec<Move>,
}

/// What the course lays over the world for the kart: the water's faces (lu), to float on and
/// land on, as the session's host gives it the surface under it ([`Kart::water`]), and its turbo
/// pads, fired as the session fires them ([`Kart::turbo_pad`]); none, a dry world bare.
#[derive(Clone, Copy, Default)]
pub struct Sea<'a> {
    pub water: Option<&'a Collision>,
    pub pads: &'a [Pad],
}

/// A turbo pad (`kartcraft::pads::Pad`), lu: its centre on the ground and its forward.
#[derive(Clone, Copy, Debug)]
pub struct Pad {
    pub centre: Vec3,
    pub forward: Vec3,
}

/// A pad's side, lu: CTR's 384 (`kartcraft::pads`), and how far over or under it (lu) the kart
/// counts as on it, its 1.5 yd.
const PAD_LU: f32 = 384.0;
const PAD_REACH_LU: f32 = 55.0;

impl Pad {
    /// Whether a kart whose origin is at `p` is on the pad.
    pub fn contains(&self, p: Vec3) -> bool {
        let d = p - self.centre;
        let right = Vec3::new(self.forward.z, 0.0, -self.forward.x);
        d.dot(self.forward).abs() <= PAD_LU * 0.5
            && d.dot(right).abs() <= PAD_LU * 0.5
            && d.y.abs() <= PAD_REACH_LU
    }
}

impl Sea<'_> {
    /// The water's surface under or over `p` (lu height), if any.
    pub fn level(&self, p: Vec3) -> Option<f32> {
        let top = Vec3::new(p.x, p.y + 20000.0, p.z);
        self.water?
            .ray(top, Vec3::NEG_Y, 1.0e6)
            .map(|h| top.y - h.distance)
    }
}

/// Flies `kart` (on its start, facing the first ring) through `rings` in order, the best it can.
pub fn fly_rings(world: &Collision, sea: Sea, kart: Kart, rings: &[Ring]) -> FlyReport {
    search(world, sea, kart, rings, None, false)
}

/// As [`fly_rings`], but the last ring is only its plane, crossed as high as the bot can: how high
/// a ring there can hang for a flight through the rings before it.
pub fn fly_reach(world: &Collision, sea: Sea, kart: Kart, rings: &[Ring]) -> FlyReport {
    search(world, sea, kart, rings, None, true)
}

/// Flies `kart` through `rings` as the client's race autopilot does (`WOW_KART_AUTOPILOT`): full
/// gas, steering for the next ring by `(-off * 2.5)` of the angle off it, and in the air the
/// hop held two ticks in four, a double tap (`WOW_KART_AUTOPILOT=fly`): the instrument a live
/// run of the autopilot is laid against, ring for ring.
pub fn fly_simple(world: &Collision, sea: Sea, mut kart: Kart, rings: &[Ring]) -> FlyReport {
    let mods = Modifiers::default();
    let mut report = FlyReport::default();
    let mut passed = 0;
    for tick in 1..=MAX_TICKS {
        let before = kart.pose().position;
        let Some(r) = rings.get(passed) else {
            break;
        };
        let want = yaw_to(before, r.centre);
        let off = wrap_signed(want - kart.pose().heading) as f32 * TAU / 4096.0;
        let input = Input {
            steer: (-off * 2.5).clamp(-1.0, 1.0),
            gas: true,
            r1: !kart.grounded() && tick % 4 < 2,
            ..Input::default()
        };
        kart.water = sea.level(before);
        if sea.pads.iter().any(|p| p.contains(before)) {
            kart.turbo_pad();
        }
        kart.tick(world, &input, &mods);
        kart.events.clear();
        let here = kart.pose().position;
        if tick % CHUNK == 0 {
            report.path.push(here);
        }
        report.ticks = tick;
        let (da, db) = (
            (before - r.centre).dot(r.through),
            (here - r.centre).dot(r.through),
        );
        if da < 0.0 && db >= 0.0 {
            let hit = before + (here - before) * (da / (da - db));
            let pass = Pass {
                tick,
                miss: hit.distance(r.centre),
                over_ground: over_ground(world, hit),
                at: hit,
            };
            if pass.miss > r.radius + RING_REACH {
                report.missed = Some(pass);
                break;
            }
            report.passes.push(pass);
            passed += 1;
        }
        if world
            .ray(here + Vec3::Y * 200.0, Vec3::NEG_Y, 1.0e6)
            .is_none()
        {
            break;
        }
    }
    report
}

/// Flies `kart` off its launch along `heading` (4096ths of a turn) as far as it can: the best
/// flight's touchdown is the farthest across the ground from `kart`'s start.
pub fn fly_far(world: &Collision, sea: Sea, kart: Kart, heading: i32) -> FlyReport {
    search(world, sea, kart, &[], Some(heading), false)
}

fn search(
    world: &Collision,
    sea: Sea,
    kart: Kart,
    rings: &[Ring],
    far: Option<i32>,
    reach: bool,
) -> FlyReport {
    let from = kart.pose().position;
    let start = Flight {
        kart,
        tick: 0,
        passed: 0,
        passes: Vec::new(),
        flew: false,
        down: None,
        lost: false,
        missed: None,
        taps: 0,
        path: vec![from],
        moves: Vec::new(),
    };
    let mods = Modifiers::default();
    let mut beam = vec![start];
    let mut done: Vec<Flight> = Vec::new();
    // The flight that got farthest before a miss, the nearest of those to the ring it missed.
    let mut nearest: Option<Flight> = None;
    while !beam.is_empty() {
        let mut next = Vec::with_capacity(beam.len() * MOVES.len());
        for f in &beam {
            // Out of flight there is nothing to choose: the gas, toward the next ring.
            let moves: &[Move] = if f.kart.flying() { &MOVES } else { &MOVES[..1] };
            for &m in moves {
                let mut g = f.clone();
                step(world, sea, &mut g, m, rings, far, reach, &mods);
                if g.lost {
                    if let Some(m) = g.missed {
                        let better = nearest.as_ref().is_none_or(|n| {
                            let b = n.missed.map_or(f32::INFINITY, |p| p.miss);
                            g.passed > n.passed || (g.passed == n.passed && m.miss < b)
                        });
                        if better {
                            nearest = Some(g.clone());
                        }
                    }
                    continue;
                }
                // Off a launch a flight ends at its touchdown; through rings the kart lands, drives
                // on and flies off the next edge, ending at the last ring.
                let over = if rings.is_empty() {
                    g.down.is_some()
                } else {
                    g.passed == rings.len()
                };
                if over || g.tick >= MAX_TICKS {
                    done.push(g);
                } else {
                    next.push(g);
                }
            }
        }
        next.sort_by(|a, b| {
            score(b, rings, far, from, reach).total_cmp(&score(a, rings, far, from, reach))
        });
        // Of flights all but the same, one: the beam keeps different ones.
        let mut kept: Vec<Flight> = Vec::with_capacity(BEAM);
        for f in next {
            let p = f.kart.pose().position;
            if kept
                .iter()
                .any(|k| k.passed == f.passed && k.kart.pose().position.distance(p) < 24.0)
            {
                continue;
            }
            kept.push(f);
            if kept.len() == BEAM {
                break;
            }
        }
        beam = kept;
    }
    let mut best = done
        .into_iter()
        .max_by(|a, b| finish(a, far, from, reach).total_cmp(&finish(b, far, from, reach)));
    // No flight through every ring: the one that got farthest, and how it missed the next.
    if best.as_ref().is_none_or(|b| b.passed < rings.len())
        && nearest
            .as_ref()
            .is_some_and(|n| best.as_ref().is_none_or(|b| n.passed >= b.passed))
    {
        best = nearest;
    }
    best.map_or_else(FlyReport::default, |f| FlyReport {
        passes: f.passes,
        ticks: f.tick,
        path: f.path,
        touchdown: f.down,
        moves: f.moves,
        missed: f.missed,
    })
}

/// A move held for a chunk: pursuit to the next ring (or straight on), the double tap once off
/// the ground, each ring checked as the kart crosses it.
#[allow(clippy::too_many_arguments)]
fn step(
    world: &Collision,
    sea: Sea,
    f: &mut Flight,
    m: Move,
    rings: &[Ring],
    far: Option<i32>,
    reach: bool,
    mods: &Modifiers,
) {
    f.moves.push(m);
    for _ in 0..CHUNK {
        let before = f.kart.pose().position;
        let heading = f.kart.pose().heading;
        let want = match (rings.get(f.passed), far) {
            (Some(r), _) => yaw_to(before, r.centre),
            (None, Some(h)) => h,
            (None, None) => heading,
        };
        let err = wrap_signed(want - heading) as f32;
        // −1 full left to 1 full right; the heading grows to the left.
        let steer = (-err / 160.0).clamp(-1.0, 1.0);
        let mut input = Input {
            steer,
            gas: m != Move::Dive,
            down: m == Move::Climb && f.kart.flying(),
            ..Input::default()
        };
        // The double tap: two presses in the air, a tick apart, to open the wings at once once
        // over a drop (a hop's own air is not one), off every edge.
        if !f.kart.grounded() && !f.kart.flying() && over_ground(world, before) > TAP_OVER {
            input.r1 = f.taps.is_multiple_of(2);
            f.taps = f.taps.wrapping_add(1);
        }
        f.kart.water = sea.level(before);
        if sea.pads.iter().any(|p| p.contains(before)) {
            f.kart.turbo_pad();
        }
        f.kart.tick(world, &input, mods);
        f.tick += 1;
        let here = f.kart.pose().position;
        // Nothing under it: off the gathered world, where the session would hold it.
        if world
            .ray(here + Vec3::Y * 200.0, Vec3::NEG_Y, 1.0e6)
            .is_none()
        {
            f.lost = true;
            return;
        }
        for e in f.kart.events.drain(..) {
            match e {
                Event::TakeOff => f.flew = true,
                Event::Touchdown if f.flew && f.down.is_none() => f.down = Some(here),
                _ => {}
            }
        }
        if let Some(r) = rings.get(f.passed) {
            let (da, db) = (
                (before - r.centre).dot(r.through),
                (here - r.centre).dot(r.through),
            );
            if da < 0.0 && db >= 0.0 {
                let hit = before + (here - before) * (da / (da - db));
                let pass = Pass {
                    tick: f.tick,
                    miss: hit.distance(r.centre),
                    over_ground: over_ground(world, hit),
                    at: hit,
                };
                let open = reach && f.passed + 1 == rings.len();
                if !open && pass.miss > r.radius + RING_REACH {
                    f.missed = Some(pass);
                    f.lost = true;
                    return;
                }
                f.passes.push(pass);
                f.passed += 1;
            }
        }
        let landed = rings.is_empty() && f.down.is_some();
        if landed || (!rings.is_empty() && f.passed == rings.len()) {
            break;
        }
    }
    f.path.push(f.kart.pose().position);
}

/// How good a flight under way is: rings passed, then the height it could still reach (its height
/// and what its speed would climb) over the next ring, then the way made toward it; off a launch,
/// the ground covered and that height.
fn score(f: &Flight, rings: &[Ring], far: Option<i32>, from: Vec3, reach: bool) -> f32 {
    let p = f.kart.pose();
    // The speed in the kart's own units (256ths of a lu a tick), and the height it would climb.
    let speed = p.velocity.length() * 256.0 / 30.0;
    let energy = p.position.y + speed * speed / (2.0 * GRAVITY as f32 * NOSE_GRAVITY * 256.0);
    if let Some(r) = rings.get(f.passed) {
        let flat = Vec2::new(r.centre.x - p.position.x, r.centre.z - p.position.z).length();
        // The open plane: all the height it can take there.
        if reach && f.passed + 1 == rings.len() {
            return f.passed as f32 * 1.0e7 + energy * 4.0 - flat;
        }
        // Above the ring is height to spend; below it, a climb it must still make; but above the
        // steepest dive down to it, height it cannot spend in time.
        let slack = (energy - r.centre.y).min(4000.0);
        let steep = (p.position.y - r.centre.y - flat * DIVE_SLOPE).max(0.0);
        return f.passed as f32 * 1.0e7 + slack * 2.0 - flat - steep * 6.0;
    }
    if far.is_some() {
        let flat = Vec2::new(p.position.x - from.x, p.position.z - from.z).length();
        return flat + energy * 4.0;
    }
    f.passed as f32 * 1.0e7
}

/// How good a finished flight is: rings passed, then the sooner; off a launch, the farther down;
/// through an open plane, the higher across it.
fn finish(f: &Flight, far: Option<i32>, from: Vec3, reach: bool) -> f32 {
    if far.is_some() {
        return f
            .down
            .map_or(0.0, |d| Vec2::new(d.x - from.x, d.z - from.z).length());
    }
    if reach {
        return f.passed as f32 * 1.0e7 + f.passes.last().map_or(0.0, |p| p.at.y);
    }
    f.passed as f32 * 1.0e7 - f.tick as f32
}

/// The heading from `a` to `b`, 4096ths of a turn.
pub fn yaw_to(a: Vec3, b: Vec3) -> i32 {
    let d = b - a;
    (d.x.atan2(d.z) / TAU * 4096.0).round() as i32
}

fn wrap_signed(a: i32) -> i32 {
    (a + 2048).rem_euclid(4096) - 2048
}

/// How high over the ground `p` is, lu.
pub fn over_ground(world: &Collision, p: Vec3) -> f32 {
    world
        .ray(p + Vec3::Y * 16.0, Vec3::NEG_Y, 1.0e6)
        .map_or(f32::INFINITY, |h| h.distance - 16.0)
}

/// A kart of `class` on the ground under `at`, facing `heading`.
pub fn kart_at(world: &Collision, class: Class, at: Vec3, heading: i32) -> Option<Kart> {
    let ground = world.ground(at + Vec3::Y * 200.0, 2000.0)?;
    Some(Kart::new(class, ground.point + Vec3::Y * 4.0, heading))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CELL;

    /// A plateau `height` lu up for z < 0, a cliff at z = 0, the flat far out below it.
    fn cliff(height: f32) -> Collision {
        let mut tris = Vec::new();
        let tile = 2000.0;
        let mut quad = |x0: f32, z0: f32, y: f32| {
            let (x1, z1) = (x0 + tile, z0 + tile);
            tris.push([[x0, y, z0], [x0, y, z1], [x1, y, z1]]);
            tris.push([[x0, y, z0], [x1, y, z1], [x1, y, z0]]);
        };
        for i in -3..3 {
            for j in -3..0 {
                quad(i as f32 * tile, j as f32 * tile, height);
            }
        }
        for i in -25..25 {
            for j in 0..60 {
                quad(i as f32 * tile, j as f32 * tile, 0.0);
            }
        }
        Collision::new(tris, CELL)
    }

    fn launch(w: &Collision, height: f32) -> Kart {
        kart_at(w, Class::Balanced, Vec3::new(0.0, height, -4000.0), 0).unwrap()
    }

    /// Off a cliff it flies through a ring set off to one side and below: it steers to it and
    /// spends the height to make it.
    #[test]
    fn the_bot_flies_through_a_ring_off_to_the_side() {
        let w = cliff(3000.0);
        for side in [-1.0f32, 1.0] {
            let ring = Ring {
                centre: Vec3::new(side * 3000.0, 2200.0, 9000.0),
                through: Vec3::new(side * 3000.0, 0.0, 9000.0).normalize(),
                radius: 16.0 * 36.6,
            };
            let r = fly_rings(&w, Sea::default(), launch(&w, 3000.0), &[ring]);
            assert_eq!(r.passes.len(), 1, "side {side}: missed {:?}", r.missed);
            assert!(r.passes[0].miss <= ring.radius + RING_REACH);
        }
    }

    /// The search flies farther off a cliff than holding the gas the whole way.
    #[test]
    fn the_bot_flies_farther_than_the_gas_held() {
        let w = cliff(6000.0);
        let mods = Modifiers::default();
        let mut held = launch(&w, 6000.0);
        let mut flew = false;
        let gas = Input {
            gas: true,
            ..Input::default()
        };
        let mut down = None;
        for _ in 0..MAX_TICKS {
            held.tick(&w, &gas, &mods);
            let here = held.pose().position;
            for e in held.events.drain(..) {
                flew |= e == Event::TakeOff;
                if e == Event::Touchdown && flew && down.is_none() {
                    down = Some(here);
                }
            }
            if down.is_some() {
                break;
            }
        }
        let held_far = down.expect("the held gas lands").z;
        let best = fly_far(&w, Sea::default(), launch(&w, 6000.0), 0)
            .touchdown
            .expect("the bot lands");
        assert!(best.z > held_far, "{} against {held_far}", best.z);
    }

    /// The instrument: the farthest flight off cliffs of a few heights, yards, and the height it
    /// held (`cargo test -p kart-driver glide_table -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn glide_table() {
        let yd = crate::LU_PER_YARD;
        for h in [40.0f32, 80.0, 110.0, 140.0, 170.0, 210.0] {
            let w = cliff(h * yd);
            let r = fly_far(&w, Sea::default(), launch(&w, h * yd), 0);
            let d = r.touchdown.map_or(0.0, |d| (d.z + 4000.0) / yd);
            let profile: Vec<String> = r
                .path
                .iter()
                .step_by(8)
                .map(|p| format!("{:.0}@{:.0}", p.y / yd, (p.z + 4000.0) / yd))
                .collect();
            eprintln!(
                "cliff {h} yd: {d:.0} yd, {} s: {}",
                r.ticks / 30,
                profile.join(" ")
            );
        }
    }

    /// Two roofs `up` lu over the ground, the second `gap` lu past the first's edge, a ring on
    /// it; and a pad at the first's edge, or none.
    fn roofs(up: f32, gap: f32, rise: f32) -> Collision {
        let mut tris = Vec::new();
        let tile = 1000.0;
        let mut quad = |x0: f32, z0: f32, y: f32, w: f32| {
            let (x1, z1) = (x0 + w, z0 + w);
            tris.push([[x0, y, z0], [x0, y, z1], [x1, y, z1]]);
            tris.push([[x0, y, z0], [x1, y, z1], [x1, y, z0]]);
        };
        for i in -2..2 {
            for j in -4..0 {
                quad(i as f32 * tile, j as f32 * tile, up, tile);
            }
            for j in 0..4 {
                quad(i as f32 * tile, gap + j as f32 * tile, up + rise, tile);
            }
        }
        for i in -20..20 {
            for j in -10..40 {
                quad(i as f32 * tile, j as f32 * tile, 0.0, tile);
            }
        }
        Collision::new(tris, CELL)
    }

    /// The instrument: the widest gap between equal roofs the bot clears, with a pad at the
    /// edge and without (`cargo test -p kart-driver pad_table -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn pad_table() {
        let yd = crate::LU_PER_YARD;
        for (up, rise) in [
            (20.0f32, 0.0f32),
            (40.0, 0.0),
            (40.0, 4.0),
            (40.0, 8.0),
            (40.0, 12.0),
        ] {
            for padded in [false, true] {
                let mut widest = 0.0;
                for gap in [
                    5.0f32, 10.0, 15.0, 20.0, 30.0, 40.0, 50.0, 60.0, 80.0, 100.0, 130.0,
                ] {
                    let w = roofs(up * yd, gap * yd, rise * yd);
                    let ring = Ring {
                        centre: Vec3::new(0.0, (up + rise) * yd + 9.0 * yd, gap * yd + 600.0),
                        through: Vec3::Z,
                        radius: 10.0 * yd,
                    };
                    let pads = [Pad {
                        centre: Vec3::new(0.0, up * yd, -250.0),
                        forward: Vec3::Z,
                    }];
                    let sea = Sea {
                        water: None,
                        pads: if padded { &pads } else { &[] },
                    };
                    let kart =
                        kart_at(&w, Class::Balanced, Vec3::new(0.0, up * yd, -3000.0), 0).unwrap();
                    if fly_rings(&w, sea, kart, &[ring]).passes.len() == 1 {
                        widest = gap;
                    }
                }
                eprintln!("roofs {up} yd up, the next {rise} yd higher, pad {padded}: widest gap {widest} yd");
            }
        }
    }
}
