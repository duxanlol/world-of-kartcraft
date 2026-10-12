//! Showcase runs: short choreographed drives at famous places, authored offline with the driver
//! over the world's collision and kept as input tapes (`showcase/showcase.json`). A tape is the
//! controls of every 30 Hz tick from a start pose; the driving model is deterministic, so the
//! client feeding the same tape from the same pose over the same faces drives the same run, and
//! the recorded pose path lets it pull a drifting kart back onto the line ([`Playback`]).
//!
//! The kart starts as the client's session starts it (`kartcraft::Session::start`): its lu origin
//! is the start, the kart on the first ground face below 200 lu above it, 4 lu up, at the heading.

use ctr_kart::{Class, Collision, Event, Input, Kart, Modifiers, State};
use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};

use crate::driver::{Driver, SlideTuning};
use crate::route::{xz, Path};

/// A path sample every this many ticks.
pub const PATH_EVERY: usize = 3;

/// Steer's resolution: the driving model reads the stick in 127ths (`Kart::tick`'s steer).
const STEER_STEPS: f32 = 127.0;

const GAS: u8 = 1;
const BRAKE: u8 = 2;
const DOWN: u8 = 4;
const HOP: u8 = 8;
const BOOST: u8 = 16;

/// One tick's controls on the tape: the steer in 127ths, and the buttons (gas, brake, reverse,
/// the hop shoulder R1, the boost shoulder L1). No item is ever fired.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TapeTick {
    pub steer: i8,
    pub flags: u8,
}

impl TapeTick {
    /// The controls as the tape keeps them: the steer rounded to the model's own resolution.
    pub fn from_input(i: &Input) -> Self {
        let bit = |on: bool, b: u8| if on { b } else { 0 };
        Self {
            steer: (i.steer.clamp(-1.0, 1.0) * STEER_STEPS).round() as i8,
            flags: bit(i.gas, GAS)
                | bit(i.brake, BRAKE)
                | bit(i.down, DOWN)
                | bit(i.r1, HOP)
                | bit(i.l1, BOOST),
        }
    }

    pub fn input(self) -> Input {
        Input {
            steer: f32::from(self.steer) / STEER_STEPS,
            gas: self.flags & GAS != 0,
            brake: self.flags & BRAKE != 0,
            down: self.flags & DOWN != 0,
            r1: self.flags & HOP != 0,
            l1: self.flags & BOOST != 0,
        }
    }
}

/// The tape as text: four hex digits a tick, the steer's byte then the buttons'.
pub fn encode(ticks: &[TapeTick]) -> String {
    ticks
        .iter()
        .map(|t| format!("{:02x}{:02x}", t.steer as u8, t.flags))
        .collect()
}

/// Inverse of [`encode`].
pub fn decode(text: &str) -> Result<Vec<TapeTick>, String> {
    if !text.is_ascii() || !text.len().is_multiple_of(4) {
        return Err("a tape is four hex digits a tick".into());
    }
    text.as_bytes()
        .chunks(4)
        .map(|c| {
            let c = std::str::from_utf8(c).map_err(|e| e.to_string())?;
            let byte = |s: &str| u8::from_str_radix(s, 16).map_err(|e| format!("{s:?}: {e}"));
            Ok(TapeTick {
                steer: byte(&c[..2])? as i8,
                flags: byte(&c[2..])?,
            })
        })
        .collect()
}

/// Every showcase run.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ShowcaseFile {
    pub runs: Vec<Run>,
}

impl ShowcaseFile {
    pub fn from_json(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|e| e.to_string())
    }

    pub fn run(&self, name: &str) -> Option<&Run> {
        self.runs.iter().find(|r| r.name.eq_ignore_ascii_case(name))
    }
}

/// One showcase run.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Run {
    /// Its key, as `/kartshow <name>` takes it.
    pub name: String,
    pub title: String,
    /// What it does, for the list.
    pub note: String,
    /// The map, and the start: WoW x, y, z (yards; the session's origin) and the kart's heading
    /// (CTR's 4096ths, forward `(sin h, 0, cos h)` in the kart's frame).
    pub map: u32,
    pub start: [f32; 3],
    pub heading: i32,
    /// The engine class it was driven in ("balanced", "accel", "speed", "turn").
    pub class: String,
    /// The cameras it looks best through, best first ("tripod", "low", "orbit", "chase").
    pub cameras: Vec<String>,
    /// Where the tripod stands: WoW x, y, z.
    pub tripod: [f32; 3],
    /// What the authoring drive counted: boosts, powerslides and seconds.
    pub boosts: u32,
    pub slides: u32,
    pub seconds: f32,
    /// The controls, [`encode`]d.
    pub tape: String,
    /// The kart after every [`PATH_EVERY`]th tick (ticks 0, 3, 6, ...): x, y, z in lu from the
    /// start, and the heading.
    pub path: Vec<[i32; 4]>,
}

impl Run {
    pub fn ticks(&self) -> Result<Vec<TapeTick>, String> {
        decode(&self.tape)
    }

    pub fn class(&self) -> Class {
        match self.class.as_str() {
            "accel" => Class::Accel,
            "speed" => Class::Speed,
            "turn" => Class::Turn,
            _ => Class::Balanced,
        }
    }
}

/// The kart at a run's start, as the client's session puts it there; `None` with no ground under
/// the start.
pub fn start_kart(world: &Collision, class: Class, heading: i32) -> Option<Kart> {
    let ground = world.ground(Vec3::Y * 200.0, 800.0)?;
    Some(Kart::new(class, ground.point + Vec3::Y * 4.0, heading))
}

/// The session's own guard (`kartcraft::session`): with nothing at all below an airborne kart the
/// world there has not streamed in, and it waits where it was.
fn hold_at_void(world: &Collision, kart: &mut Kart, before: Vec3) {
    let probe = kart.pose().position + Vec3::Y * 200.0;
    if !kart.grounded() && world.ray(probe, Vec3::NEG_Y, 1.0e6).is_none() {
        kart.halt_at(before);
    }
}

/// A kart's pose as a path sample.
fn sample(kart: &Kart) -> [i32; 4] {
    let p = kart.pose();
    [
        p.position.x.round() as i32,
        p.position.y.round() as i32,
        p.position.z.round() as i32,
        p.heading,
    ]
}

/// How a tape drove.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Replay {
    pub ticks: usize,
    pub wall_contacts: u32,
    /// Where each wall contact happened, lu.
    pub wall_points: Vec<Vec3>,
    pub boosts: u32,
    pub slides: u32,
    pub spins: u32,
    /// Ever off the ground longer than half a second (a drop, not a hop).
    pub longest_air: f32,
    /// Speed at the end, lu/s.
    pub end_speed: f32,
    pub path: Vec<[i32; 4]>,
}

/// Plays `ticks` from the start over `world`, as the client does.
pub fn replay(world: &Collision, class: Class, heading: i32, ticks: &[TapeTick]) -> Option<Replay> {
    let mut kart = start_kart(world, class, heading)?;
    let mut out = Replay::default();
    let mods = Modifiers::default();
    let mut air = 0.0f32;
    for (i, t) in ticks.iter().enumerate() {
        let before = kart.pose().position;
        kart.tick(world, &t.input(), &mods);
        hold_at_void(world, &mut kart, before);
        count(&mut kart, &mut out);
        air = if kart.grounded() {
            0.0
        } else {
            air + ctr_kart::TICK
        };
        out.longest_air = out.longest_air.max(air);
        if i % PATH_EVERY == 0 {
            out.path.push(sample(&kart));
        }
    }
    out.ticks = ticks.len();
    out.end_speed = kart.speed().abs() as f32 * 30.0 / 256.0;
    Some(out)
}

fn count(kart: &mut Kart, out: &mut Replay) {
    let here = kart.pose().position;
    for e in kart.events.drain(..) {
        match e {
            Event::WallRub | Event::Slam { .. } => {
                out.wall_contacts += 1;
                out.wall_points.push(here);
            }
            Event::DriftStart { .. } => out.slides += 1,
            Event::Boost { .. } => out.boosts += 1,
            Event::SpinOut => out.spins += 1,
            _ => {}
        }
    }
}

/// How a run is authored.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AuthorOpts {
    pub class: Class,
    pub slide: SlideTuning,
    /// Give up after this many seconds of driving.
    pub time_limit: f32,
    /// The most seconds the stop at the end may take.
    pub stop_secs: f32,
}

impl Default for AuthorOpts {
    fn default() -> Self {
        Self {
            class: Class::Balanced,
            // A showcase slides into every real bend and chains its boosts quickly.
            slide: SlideTuning {
                min_turn: 0.7,
                look_secs: 1.0,
                min_radius: 450.0,
                min_speed: 0.55,
                end_turn: 0.2,
                rest_frames: 8,
                boost_at: 0.9,
                rehop: true,
                max_boosts: 3,
                drift_plan: true,
            },
            time_limit: 40.0,
            stop_secs: 4.0,
        }
    }
}

/// A run's controls as the driver drove them along `waypoints` (lu from the start; the kart
/// starts facing `heading`), then a clean stop; `None` with no ground under the start or no path.
pub fn author(
    world: &Collision,
    waypoints: &[Vec3],
    heading: i32,
    opts: &AuthorOpts,
) -> Option<Vec<TapeTick>> {
    let path = Path::new(waypoints, false);
    if path.len() < 2 {
        return None;
    }
    let mut kart = start_kart(world, opts.class, heading)?;
    let mut driver = Driver::new(&path, opts.class, opts.slide);
    let mods = Modifiers::default();
    let mut ticks = Vec::new();
    let mut seg = 0usize;
    let limit = (opts.time_limit / ctr_kart::TICK) as usize;
    let mut stopping = None::<usize>;
    let stop_ticks = (opts.stop_secs / ctr_kart::TICK) as usize;
    for i in 0..limit {
        let pos = kart.pose().position;
        let (j, s, _) = path.nearest(pos, seg, 25, 75);
        seg = j;
        if stopping.is_none() && s >= path.length - 60.0 {
            stopping = Some(i);
        }
        let input = match stopping {
            // A clean stop: off the gas and the shoulders, braking straight.
            Some(from) => {
                if kart.speed().abs() < 40 && kart.state == State::Normal && kart.grounded() {
                    break;
                }
                if i - from > stop_ticks {
                    break;
                }
                Input {
                    brake: true,
                    ..Default::default()
                }
            }
            None => driver.input(&kart, pos, seg, s),
        };
        let tick = TapeTick::from_input(&input);
        let before = pos;
        kart.tick(world, &tick.input(), &mods);
        hold_at_void(world, &mut kart, before);
        kart.events.clear();
        ticks.push(tick);
    }
    Some(ticks)
}

/// Turns a heading (CTR's 4096ths) into its forward in the kart's XZ.
pub fn forward(heading: i32) -> Vec2 {
    let h = heading as f32 / 4096.0 * std::f32::consts::TAU;
    Vec2::new(h.sin(), h.cos())
}

/// The heading that faces along `dir` (XZ).
pub fn heading_along(dir: Vec2) -> i32 {
    (dir.x.atan2(dir.y) / std::f32::consts::TAU * 4096.0).round() as i32 & 0xFFF
}

/// How far (lu) off its recorded line a kart may be and still be pulled back onto it: farther,
/// something real happened (a kart in the way) and the run drives on as it is.
const PULL_MAX: f32 = 220.0;
/// The share of the gap closed at each path sample.
const PULL_GAIN: f32 = 0.35;

/// A run playing in the client: the tape's controls tick by tick, and after each sampled tick a
/// gentle pull of the kart back onto the recorded line (horizontally; the ground holds it up).
#[derive(Clone, Debug)]
pub struct Playback {
    ticks: Vec<TapeTick>,
    path: Vec<[i32; 4]>,
    tick: usize,
    /// The last sampled gap to the line, lu, and the largest.
    pub gap: f32,
    pub max_gap: f32,
}

impl Playback {
    pub fn new(run: &Run) -> Result<Self, String> {
        Ok(Self {
            ticks: run.ticks()?,
            path: run.path.clone(),
            tick: 0,
            gap: 0.0,
            max_gap: 0.0,
        })
    }

    /// This tick's controls; `None` once the tape has run out.
    pub fn input(&self) -> Option<Input> {
        self.ticks.get(self.tick).map(|t| t.input())
    }

    /// After the tick: measures the gap to the line and pulls the kart toward it, and moves on.
    pub fn after_tick(&mut self, kart: &mut Kart) {
        if self.tick.is_multiple_of(PATH_EVERY) {
            if let Some(p) = self.path.get(self.tick / PATH_EVERY) {
                let want = Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32);
                let off = xz(want - kart.pose().position);
                self.gap = off.length();
                self.max_gap = self.max_gap.max(self.gap);
                if self.gap > 0.5 && self.gap < PULL_MAX {
                    let by = off * PULL_GAIN;
                    kart.nudge(Vec3::new(by.x, 0.0, by.y));
                }
            }
        }
        self.tick += 1;
    }

    pub fn done(&self) -> bool {
        self.tick >= self.ticks.len()
    }

    /// Ticks played.
    pub fn tick(&self) -> usize {
        self.tick
    }

    pub fn len(&self) -> usize {
        self.ticks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ticks.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Flat ground `half` lu each way, in 1000 lu squares (a grid cell takes no huge face).
    fn flat(half: f32) -> Collision {
        let n = (half / 1000.0) as i32;
        let mut tris = Vec::new();
        for i in -n..n {
            for j in -n..n {
                let (x, z) = (i as f32 * 1000.0, j as f32 * 1000.0);
                let (x1, z1) = (x + 1000.0, z + 1000.0);
                tris.push([[x, 0.0, z], [x, 0.0, z1], [x1, 0.0, z1]]);
                tris.push([[x, 0.0, z], [x1, 0.0, z1], [x1, 0.0, z]]);
            }
        }
        Collision::new(tris, 160.0)
    }

    #[test]
    fn a_tape_round_trips_through_its_text() {
        let ticks = vec![
            TapeTick::from_input(&Input {
                steer: -1.0,
                gas: true,
                r1: true,
                ..Default::default()
            }),
            TapeTick::from_input(&Input {
                steer: 0.31,
                l1: true,
                brake: true,
                down: true,
                ..Default::default()
            }),
            TapeTick::default(),
        ];
        let text = encode(&ticks);
        assert_eq!(text.len(), 12);
        assert_eq!(decode(&text).unwrap(), ticks);
        assert_eq!(ticks[0].input().steer, -1.0);
        assert!((ticks[1].input().steer - 0.31).abs() < 0.5 / 127.0);
        assert!(ticks[1].input().l1 && ticks[1].input().down && !ticks[1].input().gas);
        assert!(decode("abc").is_err());
        assert!(decode("zz00").is_err());
    }

    #[test]
    fn a_run_file_round_trips() {
        let file = ShowcaseFile {
            runs: vec![Run {
                name: "flats".into(),
                tape: encode(&[TapeTick::default(); 4]),
                path: vec![[1, 2, 3, 4]],
                cameras: vec!["tripod".into()],
                ..Default::default()
            }],
        };
        let text = serde_json::to_string(&file).unwrap();
        let back = ShowcaseFile::from_json(&text).unwrap();
        assert_eq!(back, file);
        assert_eq!(back.run("FLATS").unwrap().ticks().unwrap().len(), 4);
    }

    #[test]
    fn an_authored_donut_drifts_with_a_triple_boost_and_replays_exactly() {
        let world = flat(40000.0);
        // Down +Z, a turn and a quarter round a 1500 lu circle to the left, and away: a donut.
        let r = 1500.0;
        let mut pts: Vec<Vec3> = (0..=20)
            .map(|i| Vec3::new(0.0, 0.0, i as f32 * 200.0))
            .collect();
        // Left of +Z is +X (CTR's left is (cos h, −sin h)); the circle's centre is there.
        let centre = Vec3::new(r, 0.0, 4000.0);
        for i in 1..=90 {
            let a = std::f32::consts::PI + i as f32 / 72.0 * std::f32::consts::TAU;
            pts.push(centre + Vec3::new(r * a.cos(), 0.0, -r * a.sin()));
        }
        let last = *pts.last().unwrap();
        pts.extend((1..=20).map(|i| last + Vec3::new(i as f32 * 200.0, 0.0, 0.0)));
        let opts = AuthorOpts::default();
        let ticks = author(&world, &pts, 0, &opts).unwrap();
        let run = replay(&world, opts.class, 0, &ticks).unwrap();
        assert!(run.slides >= 1, "{run:?}");
        assert!(run.boosts >= 3, "{} boosts", run.boosts);
        assert_eq!(run.wall_contacts, 0);
        assert!(run.end_speed < 200.0, "stopped: {}", run.end_speed);
        // The same tape from the same pose drives the same line, tick for tick.
        let again = replay(&world, opts.class, 0, &ticks).unwrap();
        assert_eq!(again.path, run.path);

        // Played back with the pull on, the kart follows the line with no gap.
        let mut play = Playback {
            ticks: ticks.clone(),
            path: run.path.clone(),
            tick: 0,
            gap: 0.0,
            max_gap: 0.0,
        };
        let mut kart = start_kart(&world, opts.class, 0).unwrap();
        while let Some(input) = play.input() {
            kart.tick(&world, &input, &Modifiers::default());
            play.after_tick(&mut kart);
        }
        assert!(play.max_gap < 1.0, "{}", play.max_gap);
    }

    #[test]
    fn the_pull_brings_a_displaced_kart_back_onto_the_line() {
        let world = flat(40000.0);
        let ticks = vec![
            TapeTick::from_input(&Input {
                gas: true,
                ..Default::default()
            });
            90
        ];
        let run = replay(&world, Class::Balanced, 0, &ticks).unwrap();
        let mut play = Playback {
            ticks,
            path: run.path,
            tick: 0,
            gap: 0.0,
            max_gap: 0.0,
        };
        let mut kart = start_kart(&world, Class::Balanced, 0).unwrap();
        // Knocked a yard and a half sideways at the start.
        kart.nudge(Vec3::new(55.0, 0.0, 0.0));
        while let Some(input) = play.input() {
            kart.tick(&world, &input, &Modifiers::default());
            play.after_tick(&mut kart);
        }
        assert!(play.max_gap > 40.0);
        assert!(play.gap < 2.0, "{}", play.gap);
    }

    #[test]
    fn headings_and_forwards_agree() {
        for h in [0, 512, 1024, 2048, 3500] {
            assert_eq!(heading_along(forward(h)), h);
        }
    }
}
