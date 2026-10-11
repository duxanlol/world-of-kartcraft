//! Races between karts, refereed by the Kartcraft server (`Kartcraft.cpp`, "Races"). A course's
//! Race Marshal hosts, joins and starts a race over gossip; a driver at a start is told so here.
//! The server sends the course's checkpoints and a countdown: CTR's 3-2-1-GO, the kart held on
//! the line, with CTR's start boost for an engine revved just before GO. Each checkpoint passed is
//! reported, and the server checks it against where we really are; it relays everyone's progress,
//! from which our place is worked out, and the finishes with their times. Checkpoints stand in the
//! world as beams of light, the next one bright. The server keeps each character's best per course
//! and the course record: it sends both at the start (`Rec`), marks a finish that beats one
//! (`F`'s sixth field), and the results board ends with them.
//!
//! The Race Finder is a WoW window in the client's UI layer (`assets/ui/KartcraftFrames.xml`,
//! opened from its minimap button or `/racefinder`) that queues, keeps its place, answers the
//! ready check and leaves races (`/leaverace`) over the addon channel itself. Here only a pending
//! ready check is remembered (`Pop`, `Pop-`), so the race it starts keeps us through the teleport
//! to the start. The banner and results below go to the layer's race frame (`super::ui`).

use std::collections::HashMap;

use benilla_assets::coords::{bevy_to_wow, wow_to_bevy};
use benilla_protocol::SessionEvent;
use benilla_world::collision::WorldCollision;
use bevy::prelude::*;

/// A course of `courses/courses.json`, the file the server's table is made from
/// (`docs/courses.md`). The server sends the checkpoints at the start ("R\tC"); here are the
/// start a driver hosts at and the gates' widths.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct Course {
    pub(crate) key: String,
    pub(crate) name: String,
    /// The zone it is in, as the Race Finder offers courses; may be empty.
    #[serde(default)]
    pub(crate) zone: String,
    /// Its map, and where its Race Marshal stands.
    #[serde(default)]
    pub(crate) map: u32,
    #[serde(default)]
    pub(crate) marshal: Option<MarshalSpot>,
    /// Laps; 0 is a sprint, start to finish.
    #[serde(default)]
    pub(crate) laps: u8,
    /// The course generator's note ("... 2.62 km, test lap 76.1s."), for the reference lap.
    #[serde(default)]
    note: String,
    checkpoints: Vec<CourseCheckpoint>,
}

/// Where a course's Race Marshal stands: WoW x, y, z and facing.
#[derive(Clone, Copy, Debug, serde::Deserialize)]
pub(crate) struct MarshalSpot {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) z: f32,
    pub(crate) o: f32,
}

#[derive(Debug, serde::Deserialize)]
struct CourseCheckpoint {
    x: f32,
    y: f32,
    /// The gate's half-width (yards); absent or 0 for `GATE_HALF_WIDTH`.
    #[serde(default)]
    gate: f32,
}

impl Course {
    /// The first checkpoint, WoW x, y.
    pub(crate) fn start(&self) -> [f32; 2] {
        self.checkpoints.first().map_or([0.0; 2], |c| [c.x, c.y])
    }

    /// The checkpoints in order, WoW x, y: the route a circuit runs round, back to the first.
    pub(crate) fn route(&self) -> Vec<[f32; 2]> {
        self.checkpoints.iter().map(|c| [c.x, c.y]).collect()
    }

    /// The track browser's description: what kind of course, where, how long, how quick.
    pub(crate) fn description(&self) -> String {
        let route = self.route();
        let mut yards: f32 = route
            .windows(2)
            .map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]))
            .sum();
        if self.laps > 0 {
            if let (Some(first), Some(last)) = (route.first(), route.last()) {
                yards += (first[0] - last[0]).hypot(first[1] - last[1]);
            }
        }
        let km = yards * 0.9144 / 1000.0;
        let place = if self.zone.is_empty() {
            String::new()
        } else {
            format!(" in {}", self.zone)
        };
        let mut text = match self.laps {
            0 => format!("A sprint{place}, {km:.2} km from start to finish"),
            1 => format!("A one-lap circuit{place}, {km:.2} km"),
            n => format!("A {n}-lap circuit{place}, {km:.2} km a lap"),
        };
        text += &format!(" through {} checkpoints.", route.len());
        // The generator's test lap, "test lap 76.1s": a clean lap's time.
        let lap = self
            .note
            .split("test lap ")
            .nth(1)
            .and_then(|rest| rest.split('s').next())
            .and_then(|secs| secs.trim().parse::<f32>().ok());
        if let Some(secs) = lap {
            let secs = secs.round() as u32;
            let what = if self.laps > 0 {
                "a clean lap"
            } else {
                "a clean run"
            };
            text += &format!(
                " {} takes about {}:{:02}.",
                capitalise(what),
                secs / 60,
                secs % 60
            );
        }
        text
    }

    /// Each checkpoint's gate half-width.
    fn gates(&self) -> Vec<f32> {
        self.checkpoints
            .iter()
            .map(|c| {
                if c.gate > 0.0 {
                    c.gate
                } else {
                    GATE_HALF_WIDTH
                }
            })
            .collect()
    }
}

fn capitalise(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map_or(String::new(), |c| c.to_uppercase().chain(chars).collect())
}

/// The courses, in the file's order (the Race Finder's).
pub(crate) static COURSES: std::sync::LazyLock<Vec<Course>> = std::sync::LazyLock::new(|| {
    #[derive(serde::Deserialize)]
    struct File {
        courses: Vec<Course>,
    }
    serde_json::from_str::<File>(include_str!("../../../../courses/courses.json"))
        .expect("courses/courses.json")
        .courses
});

/// The Race Finder's zones, in the order the file first names them; the server keeps the same
/// list, and a queue line asks for a zone by its place in it (`z3`).
pub(crate) static ZONES: std::sync::LazyLock<Vec<String>> = std::sync::LazyLock::new(|| {
    let mut zones: Vec<String> = Vec::new();
    for c in COURSES.iter() {
        if !c.zone.is_empty() && !zones.contains(&c.zone) {
            zones.push(c.zone.clone());
        }
    }
    zones
});
/// How near a start (yards) a driver may host or join.
const START_RADIUS: f32 = 55.0;
/// A checkpoint is a gate: passed on crossing the line through it across the course, within
/// this many yards of its centre, or on coming this near it (the server allows 45).
const GATE_HALF_WIDTH: f32 = 20.0;
const CHECKPOINT_RADIUS: f32 = 8.0;
/// The flat arrow floating in the next checkpoint, pointing on to the one after it: its centre's
/// height above the ground and its length (yards), the one after next's share of that size, and
/// its bob (yards), pulse (share of its size) and their rate (Hz).
const CP_ARROW_HEIGHT: f32 = 5.0;
const CP_ARROW_SIZE: f32 = 7.0;
const CP_ARROW_SECOND: f32 = 0.7;
const CP_ARROW_BOB: f32 = 0.35;
const CP_ARROW_PULSE: f32 = 0.06;
const CP_ARROW_HZ: f32 = 0.8;
/// How long the results stay up once everyone is in (s), and how long we wait for racers still
/// out on the course after we finish before closing anyway.
const RESULTS_SECS: f32 = 10.0;
const FINISH_WAIT_SECS: f32 = 60.0;
/// How long a race just entered waits for the kart the server is summoning (s).
const KART_GRACE_SECS: f32 = 10.0;
/// The start boost: revving this long before GO (s) is big, a little earlier small, earlier
/// still bogs the engine down.
const BOOST_BIG: std::ops::Range<f32> = 0.15..0.6;
const BOOST_SMALL: std::ops::Range<f32> = 0.6..1.2;
/// A checkpoint's beam: its height (yards) and radius.
const BEAM_HEIGHT: f32 = 40.0;
const BEAM_RADIUS: f32 = 1.2;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Offer {
    id: u32,
    name: String,
    host: String,
}

/// What the race map draws: the track, and where each racer last passed a checkpoint (for one
/// too far off for its kart to be relayed).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RaceMap {
    /// The course's name.
    pub(crate) name: String,
    pub(crate) laps: u8,
    /// Checkpoints, WoW x, y; the first is the start.
    pub(crate) points: Vec<[f32; 2]>,
    /// The standings, leader first.
    pub(crate) order: Vec<String>,
    /// Each racer and the checkpoint the server last has them past, WoW x, y.
    pub(crate) checkpoints: Vec<(String, [f32; 2])>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Active {
    id: u32,
    /// The course's key, as the server's course line names it; empty until it comes.
    course: String,
    laps: u8,
    /// Checkpoints in WoW x, y; the first is the start (and finish of a lap).
    points: Vec<[f32; 2]>,
    /// Each checkpoint's gate half-width, from our copy of the course; empty for the default.
    gates: Vec<f32>,
    /// When GO falls, local seconds; `None` until the countdown arrives.
    go_at: Option<f32>,
    roster: Vec<String>,
    host: bool,
    lap: u8,
    checkpoint: usize,
    /// Each racer's (laps, checkpoint, when) as the server relays it.
    progress: HashMap<String, (u8, u8, f32)>,
    /// Finishes: (name, place, ms).
    finished: Vec<(String, u8, u32)>,
    /// Finishers who set a new best with it: true a course record, false a personal best.
    bests: HashMap<String, bool>,
    /// The course record (ms, holder) and our own best (ms), from the server's `Rec`; 0 none.
    record: (u32, String),
    best: u32,
    done: bool,
    /// When the gas went down during the countdown (local s), for the start boost.
    revved_at: Option<f32>,
    beeped: i32,
    /// The standings, leader first, as last worked out.
    order: Vec<String>,
    /// Where we were last frame (WoW x, y), to see a gate crossed.
    last_at: Option<[f32; 2]>,
    /// A race is not left for want of a kart until then (local s): the server summons ours as it
    /// puts us in (and a Race Finder race teleports us), and the aura lands after the roster.
    grace_until: f32,
    /// When we crossed the finish (local s), and when the results stop showing and the race is
    /// left: RESULTS_SECS after everyone is in, or after FINISH_WAIT_SECS for the stragglers.
    finished_at: Option<f32>,
    closes_at: Option<f32>,
}

impl Active {
    /// The countdown's line (`G`): GO `ms` from `now`, the race from scratch. The server sends it
    /// as it teleports everyone to the grid, taking a driver out of the kart for the teleport and
    /// putting them back after it: the grace a kart being summoned gets starts again, or a race
    /// hosted long ago would be left in that moment without one.
    fn count_down(&mut self, now: f32, ms: f32) {
        self.go_at = Some(now + ms / 1000.0);
        self.lap = 0;
        self.checkpoint = 0;
        self.progress.clear();
        self.finished.clear();
        self.revved_at = None;
        self.beeped = 0;
        self.grace_until = self.grace_until.max(now + KART_GRACE_SECS);
    }

    /// A frame with our kart aura read: `None` is no object of ours, a worldport's loading screen.
    /// The server puts a driver back in the kart only once the new map has them (and the summon
    /// has its cast time), so the grace starts again from the moment we land: a Race Finder race
    /// on another continent loads for longer than the grace the countdown gave.
    fn landing(&mut self, kart_aura: Option<bool>, now: f32) {
        if kart_aura.is_none() {
            self.grace_until = self.grace_until.max(now + KART_GRACE_SECS);
        }
    }

    /// The checkpoint to reach next; `None` before there is a course, and once a sprint's finish
    /// is passed (its result is on its way: the finish is reported once).
    fn next(&self) -> Option<usize> {
        let n = self.points.len();
        if n < 2 {
            None
        } else if self.laps > 0 {
            Some((self.checkpoint + 1) % n)
        } else {
            (self.checkpoint + 1 < n).then_some(self.checkpoint + 1)
        }
    }

    /// The checkpoint after `i` on the course; `None` after a sprint's finish.
    fn after(&self, i: usize) -> Option<usize> {
        let n = self.points.len();
        if self.laps > 0 {
            Some((i + 1) % n)
        } else {
            (i + 1 < n).then_some(i + 1)
        }
    }

    /// The direction the course runs through checkpoint `i` (WoW x, y, unit): halfway between
    /// the way in and the way out, so its gate stands square across a bend.
    fn heading(&self, i: usize) -> [f32; 2] {
        let n = self.points.len();
        let p = self.points[i];
        let unit = |a: [f32; 2], b: [f32; 2]| {
            let (x, y) = (b[0] - a[0], b[1] - a[1]);
            let l = x.hypot(y).max(1e-3);
            [x / l, y / l]
        };
        let before = if self.laps > 0 || i > 0 {
            Some(self.points[(i + n - 1) % n])
        } else {
            None
        };
        let into = before.map(|b| unit(b, p));
        let out = self.after(i).map(|j| unit(p, self.points[j]));
        match (into, out) {
            (Some(a), Some(b)) => unit([0.0, 0.0], [a[0] + b[0], a[1] + b[1]]),
            (Some(a), None) | (None, Some(a)) => a,
            (None, None) => [1.0, 0.0],
        }
    }

    /// Whether going from `from` to `to` (WoW x, y) passes checkpoint `i`: through its gate, or
    /// near its centre.
    fn passes(&self, i: usize, from: Option<[f32; 2]>, to: [f32; 2]) -> bool {
        let p = self.points[i];
        if (to[0] - p[0]).hypot(to[1] - p[1]) <= CHECKPOINT_RADIUS {
            return true;
        }
        let Some(from) = from else {
            return false;
        };
        let h = self.heading(i);
        let along = |q: [f32; 2]| (q[0] - p[0]) * h[0] + (q[1] - p[1]) * h[1];
        let across = |q: [f32; 2]| ((q[0] - p[0]) * h[1] - (q[1] - p[1]) * h[0]).abs();
        along(from) < 0.0
            && along(to) >= 0.0
            && across(to).min(across(from)) <= self.gates.get(i).copied().unwrap_or(GATE_HALF_WIDTH)
    }
}

/// What the ghost needs of the race we are in ([`Races::run`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Run<'a> {
    pub(crate) id: u32,
    /// The course's key; empty until the server's course line.
    pub(crate) course: &'a str,
    /// When GO falls, local seconds.
    pub(crate) go_at: Option<f32>,
    /// We have finished.
    pub(crate) done: bool,
    /// Our time (ms) as the server timed it, once we have finished.
    pub(crate) finish_ms: Option<u32>,
}

#[derive(Resource, Default)]
pub(crate) struct Races {
    pub(crate) offer: Option<Offer>,
    /// A Race Finder ready check is out (`Pop` until `Pop-` or the race's roster).
    check_pending: bool,
    pub(crate) race: Option<Active>,
    /// Our character's name, to find ourselves in rosters.
    pub(crate) me: String,
    /// Lines from the server not yet handled.
    inbox: Vec<Vec<String>>,
}

impl Races {
    /// The next checkpoint of a race under way, Bevy world (ground height unknown: y 0).
    pub(crate) fn next_target(&self, now: f32) -> Option<Vec3> {
        let race = self.race.as_ref()?;
        if race.done || race.go_at.is_none_or(|go| now < go) || race.points.len() < 2 {
            return None;
        }
        let p = race.points[race.next()?];
        Some(wow_to_bevy([p[0], p[1], 0.0]))
    }

    /// The race whose weapons are live: one we are in and have not finished (the server lays
    /// its crates at the start; they go when we leave or finish).
    pub(crate) fn weapons_race(&self) -> Option<u32> {
        self.race.as_ref().filter(|r| !r.done).map(|r| r.id)
    }

    /// The race's track for the race map: once its course line has come.
    pub(crate) fn map(&self) -> Option<RaceMap> {
        let race = self.race.as_ref().filter(|r| r.points.len() >= 2)?;
        let name = COURSES
            .iter()
            .find(|c| c.key == race.course)
            .map_or_else(|| race.course.clone(), |c| c.name.clone());
        // Each racer at the last checkpoint the server has them past (the start before any).
        let n = race.points.len();
        let at = |name: &String| {
            let cp = race.progress.get(name).map_or(0, |p| usize::from(p.1));
            race.points[cp.min(n - 1)]
        };
        Some(RaceMap {
            name,
            laps: race.laps,
            points: race.points.clone(),
            order: race.order.clone(),
            checkpoints: race.roster.iter().map(|r| (r.clone(), at(r))).collect(),
        })
    }

    /// The racers by name, leader first.
    pub(crate) fn order(&self) -> &[String] {
        self.race.as_ref().map_or(&[], |r| r.order.as_slice())
    }

    /// The race we are in, for the test hook ([`super::autotest`]): its id, roster, whether its
    /// countdown has come, and whether we have finished.
    pub(crate) fn status(&self) -> Option<(u32, &[String], bool, bool)> {
        let r = self.race.as_ref()?;
        Some((r.id, r.roster.as_slice(), r.go_at.is_some(), r.done))
    }

    /// The race we are in, for its ghost ([`super::ghost`]): its id, course, when GO falls, and
    /// our finish time (ms) once the server has it.
    pub(crate) fn run(&self) -> Option<Run<'_>> {
        let r = self.race.as_ref()?;
        Some(Run {
            id: r.id,
            course: &r.course,
            go_at: r.go_at,
            finish_ms: r
                .finished
                .iter()
                .find(|f| f.0 == self.me)
                .map(|f| f.2)
                .filter(|_| r.done),
            done: r.done,
        })
    }

    /// Whether the kart must hold on the line: a countdown is running.
    pub(crate) fn holding(&self, now: f32) -> bool {
        self.race
            .as_ref()
            .is_some_and(|r| r.go_at.is_some_and(|go| now < go))
    }
}

/// Race lines from the server.
pub(crate) fn on_chat(In(ev): In<SessionEvent>, mut races: ResMut<Races>) {
    let SessionEvent::Chat(m) = ev else {
        return;
    };
    if !m.is_addon() {
        return;
    }
    if let Some(body) = m.text.strip_prefix("KARTCRAFT\tR\t") {
        races
            .inbox
            .push(body.split('\t').map(str::to_string).collect());
    }
}

/// Whether a race is left for want of a kart: our own object known and no Summon Kart on it, past
/// the grace a kart being summoned gets. A Race Finder race on another continent sends us through a
/// loading screen that drops our object until the new map has us, for longer than the grace.
fn left_without_kart(kart_aura: Option<bool>, now: f32, grace_until: f32) -> bool {
    kart_aura == Some(false) && now >= grace_until
}

/// Where a racer stands in the race, for ordering: laps, then checkpoints, then who got there
/// first.
fn standing(progress: &(u8, u8, f32), points: usize) -> (i64, f32) {
    (
        i64::from(progress.0) * points as i64 + i64::from(progress.1),
        -progress.2,
    )
}

/// What the race shows on screen this frame: a prompt or countdown at the top, results below.
#[derive(Resource, Default)]
pub(crate) struct RaceText {
    pub(crate) banner: String,
    pub(crate) lines: Vec<String>,
}

/// Handles the server's lines, the countdown, checkpoints and standings.
#[allow(clippy::too_many_arguments)]
pub(crate) fn update(
    time: Res<Time>,
    drive: Res<super::KartDrive>,
    mut races: ResMut<Races>,
    mut text: ResMut<RaceText>,
    mut extras: ResMut<kartcraft::hud::HudExtras>,
    mut audio: ResMut<kartcraft::sound::KartAudio>,
    profession: super::profession::Profession,
    mut auto: Local<AutoRace>,
) {
    let now = time.elapsed_secs();
    let races = &mut *races;
    let me = races.me.clone();

    // The server's lines.
    for f in std::mem::take(&mut races.inbox) {
        let id = f.get(1).and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
        match f.first().map(String::as_str) {
            Some("Pop") => {
                races.check_pending = true;
                audio.cues.push(kartcraft::sound::Cue::Countdown(true));
            }
            Some("Pop-") => races.check_pending = false,
            // Out of everything (`/leaverace`): the server has let us go, so whatever we
            // thought we were in goes too.
            Some("X") => {
                info!("kart race: left every race and the queue");
                races.race = None;
                races.offer = None;
                races.check_pending = false;
                *text = RaceText::default();
                extras.lap = None;
                extras.place = None;
                audio.music_rate = 1.0;
            }
            Some("O") if f.len() >= 5 => {
                if races.race.is_none() {
                    races.offer = Some(Offer {
                        id,
                        name: f[3].clone(),
                        host: f[4].clone(),
                    });
                }
            }
            Some("L") if f.len() >= 3 => {
                let roster: Vec<String> = f[2].split(',').map(str::to_string).collect();
                if roster.contains(&me) {
                    races.check_pending = false;
                    // A Race Finder pop or a Race Marshal on foot summons the kart as it puts us
                    // in: its aura lands after the roster, when the cast time has run.
                    let race = races.race.get_or_insert_with(|| Active {
                        id,
                        grace_until: now + KART_GRACE_SECS,
                        ..Default::default()
                    });
                    race.host = roster.first() == Some(&me);
                    race.roster = roster;
                    races.offer = None;
                } else if races.race.as_ref().is_some_and(|r| r.id == id) {
                    races.race = None;
                }
            }
            Some("C") if f.len() >= 5 => {
                if let Some(race) = races.race.as_mut().filter(|r| r.id == id) {
                    race.course = f[2].clone();
                    race.laps = f[3].parse().unwrap_or(0);
                    race.points = f[4]
                        .split(';')
                        .filter_map(|p| {
                            let (x, y) = p.split_once(':')?;
                            Some([x.parse().ok()?, y.parse().ok()?])
                        })
                        .collect();
                    race.gates = COURSES
                        .iter()
                        .find(|c| c.key == f[2] && c.checkpoints.len() == race.points.len())
                        .map_or_else(Vec::new, Course::gates);
                    info!(
                        "kart race: course {} — {} checkpoints, {} laps",
                        f[2],
                        race.points.len(),
                        race.laps
                    );
                }
            }
            Some("G") if f.len() >= 3 => {
                if let Some(race) = races.race.as_mut().filter(|r| r.id == id) {
                    race.count_down(now, f[2].parse().unwrap_or(4000.0));
                    info!("kart race: countdown, {} racers", race.roster.len());
                }
            }
            Some("P") if f.len() >= 5 => {
                if let Some(race) = races.race.as_mut().filter(|r| r.id == id) {
                    let lap = f[3].parse().unwrap_or(0);
                    let cp = f[4].parse().unwrap_or(0);
                    race.progress.insert(f[2].clone(), (lap, cp, now));
                }
            }
            Some("F") if f.len() >= 5 => {
                if let Some(race) = races.race.as_mut().filter(|r| r.id == id) {
                    let place = f[3].parse().unwrap_or(0);
                    let ms = f[4].parse().unwrap_or(0);
                    // A sixth field: "r" a new course record, "p" a new personal best.
                    let best = f.get(5).map(String::as_str).unwrap_or("");
                    info!("kart race: {} finished {place} in {ms} ms {best}", f[2]);
                    if best == "r" || best == "p" {
                        race.bests.insert(f[2].clone(), best == "r");
                    }
                    race.finished.push((f[2].clone(), place, ms));
                    if f[2] == me {
                        race.done = true;
                    }
                }
            }
            // The course record and holder, and our best: at the start, and after a new best.
            Some("Rec") if f.len() >= 6 => {
                if let Some(race) = races.race.as_mut().filter(|r| r.id == id) {
                    race.record = (f[3].parse().unwrap_or(0), f[4].clone());
                    race.best = f[5].parse().unwrap_or(0);
                    info!(
                        "kart race: course record {} by {}, our best {}",
                        race.record.0, race.record.1, race.best
                    );
                }
            }
            _ => {}
        }
    }

    // `WOW_KART_RACE` joins, hosts or starts for unattended runs, as the Race Marshal does.
    let press = auto.press(now, races);
    *text = RaceText::default();

    // Out of the kart, out of the race; but not while the kart aura stays on us, as it does
    // through the Race Finder's teleport to the start.
    if !drive.active {
        let kart_aura = profession.kart_aura();
        if let Some(race) = races.race.as_mut() {
            race.landing(kart_aura, now);
        }
        let grace_until = races.race.as_ref().map_or(0.0, |r| r.grace_until);
        if left_without_kart(kart_aura, now, grace_until) {
            if let Some(race) = races.race.take() {
                profession.send(format!("KARTCRAFT\tR\tQ\t{}", race.id));
            }
        }
        races.offer = None;
        extras.lap = None;
        extras.place = None;
        audio.music_rate = 1.0;
        if races.race.is_some() {
            text.banner = "Getting to the start...".into();
        }
        return;
    }
    let wow = bevy_to_wow(drive.pos);
    let at = [wow[0], wow[1]];
    let near = |p: [f32; 2], r: f32| (p[0] - at[0]).hypot(p[1] - at[1]) <= r;

    // The courses starting here.
    let here: Vec<&Course> = COURSES
        .iter()
        .filter(|c| near(c.start(), START_RADIUS))
        .collect();
    if press {
        match (&races.race, &races.offer, here.first()) {
            (Some(race), _, _) if race.host && race.go_at.is_none() => {
                profession.send(format!("KARTCRAFT\tR\tS\t{}", race.id));
            }
            (None, Some(offer), _) => {
                profession.send(format!("KARTCRAFT\tR\tJ\t{}", offer.id));
            }
            (None, None, Some(course)) => {
                profession.send(format!("KARTCRAFT\tR\tH\t{}", course.key));
            }
            _ => {}
        }
    }

    // The countdown, the start boost and the race.
    let Some(race) = races.race.as_mut() else {
        extras.lap = None;
        extras.place = None;
        audio.music_rate = 1.0;
        text.banner = match &races.offer {
            Some(o) => format!("{}'s {} — talk to the Race Marshal to join", o.host, o.name),
            None => start_banner(&here),
        };
        return;
    };
    let Some(go) = race.go_at else {
        text.banner = format!(
            "Waiting to race: {}{}",
            race.roster.join(", "),
            if race.host {
                " — talk to the Race Marshal to start, /leaverace to leave"
            } else {
                " — /leaverace to leave"
            }
        );
        return;
    };
    if now < go {
        let left = go - now;
        let beat = left.ceil() as i32;
        if beat <= 3 && beat != race.beeped {
            race.beeped = beat;
            audio.cues.push(kartcraft::sound::Cue::Countdown(false));
        }
        text.banner = if beat <= 3 {
            beat.to_string()
        } else {
            "Ready".into()
        };
        // The gas held from when it went down.
        if drive.revving {
            race.revved_at.get_or_insert(now);
        } else {
            race.revved_at = None;
        }
        return;
    }
    if race.beeped != 0 {
        race.beeped = 0;
        audio.cues.push(kartcraft::sound::Cue::Countdown(true));
        let early = race.revved_at.map(|t| go - t);
        let boost = early.and_then(|e| {
            if BOOST_BIG.contains(&e) {
                Some(true)
            } else if BOOST_SMALL.contains(&e) {
                Some(false)
            } else {
                None
            }
        });
        races_start_boost(boost);
    }
    if now - go < 1.0 {
        text.banner = "GO!".into();
    }

    // Checkpoints: the next one passed is reported (the server checks it).
    let last_at = race.last_at.replace(at);
    if let Some(next) = race.next().filter(|_| !race.done) {
        if std::env::var_os("WOW_KART_DEBUG").is_some()
            && (now * 2.0) as i32 != ((now - time.delta_secs()) * 2.0) as i32
        {
            let p = race.points[next];
            info!(
                "kart race debug: next {next} at {p:?}, {:.0} yd away (we are at {at:?})",
                (p[0] - at[0]).hypot(p[1] - at[1])
            );
        }
        if race.passes(next, last_at, at) {
            info!("kart race: checkpoint {next}, lap {}", race.lap);
            profession.send(format!("KARTCRAFT\tR\tK\t{}\t{next}", race.id));
            race.checkpoint = next;
            if race.laps > 0 && next == 0 {
                race.lap += 1;
            }
            race.progress
                .insert(me.clone(), (race.lap, next as u8, now));
        }
    }

    // Standings: the finished in order, then the rest by progress.
    let n = race.points.len();
    let mut order: Vec<String> = race.finished.iter().map(|f| f.0.clone()).collect();
    let mut running: Vec<(&String, (i64, f32))> = race
        .roster
        .iter()
        .filter(|r| !order.contains(r))
        .map(|r| {
            let p = race.progress.get(r).copied().unwrap_or((0, 0, go));
            (r, standing(&p, n))
        })
        .collect();
    running.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    order.extend(running.into_iter().map(|(r, _)| r.clone()));
    extras.place = order.iter().position(|r| *r == me).map(|i| i as u8 + 1);
    race.order = order;
    extras.lap = (race.laps > 0).then(|| ((race.lap + 1).min(race.laps), race.laps));
    audio.music_rate = if race.laps > 1 && race.lap + 1 == race.laps && !race.done {
        // CTR's final lap: 20 BPM on its tempo (about 180).
        200.0 / 180.0
    } else {
        1.0
    };
    if race.done {
        text.banner = match race.finished.iter().find(|f| f.0 == me) {
            Some((_, place, ms)) => {
                let news = match race.bests.get(&me) {
                    Some(true) => " — New course record!",
                    Some(false) => " — New personal best!",
                    None => "",
                };
                format!("{} — {}{news}", ordinal(*place), clock(*ms))
            }
            None => String::new(),
        };
    }
    text.lines = board(race);

    // Over: once everyone is in (or the stragglers have had their minute), the results stay up a
    // while and then we leave the race.
    if race.done {
        let finished_at = *race.finished_at.get_or_insert(now);
        let everyone = race
            .roster
            .iter()
            .all(|r| race.finished.iter().any(|f| f.0 == *r));
        if race.closes_at.is_none() && (everyone || now - finished_at >= FINISH_WAIT_SECS) {
            race.closes_at = Some(now + RESULTS_SECS);
        }
        if race.closes_at.is_some_and(|t| now >= t) {
            info!("kart race: {} over, leaving", race.id);
            profession.send(format!("KARTCRAFT\tR\tQ\t{}", race.id));
            races.race = None;
            *text = RaceText::default();
            extras.lap = None;
            extras.place = None;
            audio.music_rate = 1.0;
        }
    }
}

/// `WOW_KART_RACE=host:<course>` hosts at that course's start and starts once another racer is
/// in; `WOW_KART_RACE=join` joins the first race offered. A test hook: it sends what the Race
/// Marshal's gossip would.
#[derive(Default)]
pub(crate) struct AutoRace {
    since: Option<f32>,
    hosted: bool,
    started: bool,
    joined: bool,
}

impl AutoRace {
    fn press(&mut self, now: f32, races: &Races) -> bool {
        let Ok(spec) = std::env::var("WOW_KART_RACE") else {
            return false;
        };
        let since = *self.since.get_or_insert(now);
        if now - since < 6.0 {
            return false;
        }
        if spec == "join" {
            if !self.joined && races.offer.is_some() && races.race.is_none() {
                self.joined = true;
                return true;
            }
            return false;
        }
        match &races.race {
            None if !self.hosted => {
                self.hosted = true;
                true
            }
            Some(r) if r.host && !self.started && r.roster.len() >= 2 && r.go_at.is_none() => {
                self.started = true;
                true
            }
            _ => false,
        }
    }
}

/// The start boost is fired by the kart update, which holds the session.
pub(crate) static START_BOOST: std::sync::Mutex<Option<bool>> = std::sync::Mutex::new(None);

fn races_start_boost(boost: Option<bool>) {
    if let Some(big) = boost {
        *START_BOOST.lock().unwrap() = Some(big);
    }
}

/// The banner at a start: the courses that start here, and who races them.
fn start_banner(here: &[&Course]) -> String {
    let names: Vec<&str> = here.iter().map(|c| c.name.as_str()).collect();
    let (verb, names) = match names.as_slice() {
        [] => return String::new(),
        [one] => ("starts", (*one).to_string()),
        [first @ .., last] => ("start", format!("{} and {last}", first.join(", "))),
    };
    format!("{names} {verb} here — talk to the Race Marshal to race")
}

/// The results board: the finished in order with their times, a new best marked, then the course
/// record and our own best. Empty until someone finishes.
fn board(race: &Active) -> Vec<String> {
    if race.finished.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<String> = race
        .finished
        .iter()
        .map(|(name, place, ms)| {
            let mark = match race.bests.get(name) {
                Some(true) => "  (record)",
                Some(false) => "  (best)",
                None => "",
            };
            format!("{place}. {name}  {}{mark}", clock(*ms))
        })
        .collect();
    if race.record.0 != 0 {
        lines.push(format!(
            "Course record: {} by {}",
            clock(race.record.0),
            race.record.1
        ));
    }
    if race.best != 0 {
        lines.push(format!("Your best: {}", clock(race.best)));
    }
    lines
}

fn ordinal(place: u8) -> String {
    let suffix = match place {
        1 => "ST",
        2 => "ND",
        3 => "RD",
        _ => "TH",
    };
    format!("{place}{suffix}")
}

fn clock(ms: u32) -> String {
    format!(
        "{}:{:02}.{:02}",
        ms / 60000,
        (ms / 1000) % 60,
        (ms % 1000) / 10
    )
}

/// The checkpoints' beams.
#[derive(Component)]
pub(crate) struct Beam(usize);

/// An arrow floating in an upcoming checkpoint, pointing on to the one after it: `slot` 0 in the
/// next checkpoint, 1 (smaller and dimmer) in the one after; `ground` is the last ground found
/// under it, the fallback where there is none. Its dark outline is its child.
#[derive(Component)]
pub(crate) struct CheckpointArrow {
    slot: usize,
    ground: f32,
}

/// The arrow over our kart, pointing at the next checkpoint.
#[derive(Component)]
pub(crate) struct GuideArrow;

/// The beams' and arrows' meshes and looks, made once.
#[derive(Clone)]
pub(crate) struct Looks {
    beam: Handle<Mesh>,
    arrow: Handle<Mesh>,
    bright: Handle<StandardMaterial>,
    dim: Handle<StandardMaterial>,
    guide: Handle<StandardMaterial>,
    /// The guide arrow's outline: its shape grown a little, laid flat just under it.
    guide_outline: Handle<Mesh>,
    /// The checkpoint arrow: its face and outline, the next one's and the one after's colour,
    /// and the outline's.
    cp_face: Handle<Mesh>,
    cp_outline: Handle<Mesh>,
    cp_next: Handle<StandardMaterial>,
    cp_after: Handle<StandardMaterial>,
    cp_edge: Handle<StandardMaterial>,
}

impl Looks {
    fn new(meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>) -> Self {
        let glow = |c: Color| StandardMaterial {
            base_color: c,
            unlit: true,
            alpha_mode: AlphaMode::Add,
            cull_mode: None,
            double_sided: true,
            ..default()
        };
        let solid = |c: Color| StandardMaterial {
            base_color: c,
            unlit: true,
            cull_mode: None,
            double_sided: true,
            ..default()
        };
        let (face, outline) = checkpoint_arrow_meshes();
        Self {
            beam: meshes.add(Cylinder::new(BEAM_RADIUS, BEAM_HEIGHT)),
            arrow: meshes.add(arrow_mesh()),
            bright: materials.add(glow(Color::srgb(1.0, 0.85, 0.2))),
            dim: materials.add(glow(Color::srgb(0.18, 0.12, 0.04))),
            guide: materials.add(solid(Color::srgb(1.0, 0.6, 0.08))),
            guide_outline: meshes.add(guide_outline_mesh()),
            cp_face: meshes.add(face),
            cp_outline: meshes.add(outline),
            cp_next: materials.add(solid(Color::srgb(1.0, 0.85, 0.1))),
            cp_after: materials.add(solid(Color::srgb(0.6, 0.5, 0.16))),
            cp_edge: materials.add(solid(Color::srgb(0.03, 0.02, 0.01))),
        }
    }
}

/// The checkpoint arrow's shape, one unit long up its +Y with its centre at the origin, flat in
/// the XY plane facing +Z: a shaft and a head, each convex.
const CP_ARROW_SHAFT: [[f32; 2]; 4] = [[-0.16, -0.5], [0.16, -0.5], [0.16, 0.04], [-0.16, 0.04]];
const CP_ARROW_HEAD: [[f32; 2]; 3] = [[-0.44, 0.0], [0.44, 0.0], [0.0, 0.5]];
/// How far the outline stands out round it (units), and how far behind the face it lies.
const CP_ARROW_EDGE: f32 = 0.07;
const CP_ARROW_EDGE_DEPTH: f32 = 0.03;

/// A convex polygon (anticlockwise) grown by `by` on every side, its corners mitred.
fn grown(poly: &[[f32; 2]], by: f32) -> Vec<[f32; 2]> {
    let n = poly.len();
    let normal = |a: [f32; 2], b: [f32; 2]| Vec2::new(b[1] - a[1], a[0] - b[0]).normalize();
    (0..n)
        .map(|i| {
            let (prev, p, next) = (poly[(i + n - 1) % n], poly[i], poly[(i + 1) % n]);
            let (n1, n2) = (normal(prev, p), normal(p, next));
            let out = Vec2::from(p) + (n1 + n2) / (1.0 + n1.dot(n2)) * by;
            out.into()
        })
        .collect()
}

/// Flat convex polygons as one mesh in the plane z, facing +Z.
fn flat_mesh(parts: &[Vec<[f32; 2]>], z: f32) -> Mesh {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology};
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    for part in parts {
        let first = positions.len() as u32;
        positions.extend(part.iter().map(|p| [p[0], p[1], z]));
        for k in 1..part.len() as u32 - 1 {
            indices.extend([first, first + k, first + k + 1]);
        }
    }
    let normals = vec![[0.0, 0.0, 1.0]; positions.len()];
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_indices(Indices::U32(indices))
}

/// The checkpoint arrow's face and its outline, the face's shape grown by the edge just behind it.
fn checkpoint_arrow_meshes() -> (Mesh, Mesh) {
    let face = flat_mesh(&[CP_ARROW_SHAFT.to_vec(), CP_ARROW_HEAD.to_vec()], 0.0);
    let outline = flat_mesh(
        &[
            grown(&CP_ARROW_SHAFT, CP_ARROW_EDGE),
            grown(&CP_ARROW_HEAD, CP_ARROW_EDGE),
        ],
        -CP_ARROW_EDGE_DEPTH,
    );
    (face, outline)
}

/// The checkpoint arrow's rotation: its +Y tip along the line from `from` to `to` in the world,
/// so it can only point at the checkpoint after, and turned about that line so its face (+Z)
/// looks as nearly at the eye as the line allows. Seen straight along the line it foreshortens,
/// as any arrow pointing at or away from the viewer does; it never points anywhere else.
fn checkpoint_arrow_rotation(from: Vec3, to: Vec3, eye: Vec3) -> Quat {
    let tip = (to - from).try_normalize().unwrap_or(Vec3::Y);
    let to_eye = eye - from;
    let face = (to_eye - tip * to_eye.dot(tip))
        .try_normalize()
        .unwrap_or_else(|| tip.any_orthonormal_vector());
    Quat::from_mat3(&Mat3::from_cols(tip.cross(face), tip, face))
}

/// The checkpoints that show an arrow and the checkpoint each points on to: the next one's, then
/// the one after's. None in a finish (a sprint's last, a circuit's start on its last lap) or past
/// it, and none in a checkpoint already passed.
fn arrow_legs(race: &Active) -> Vec<(usize, usize)> {
    let mut legs = Vec::new();
    let mut at = race.next();
    while let Some(i) = at.filter(|_| legs.len() < 2) {
        let Some(on) = race.after(i).filter(|_| !is_finish(race, i)) else {
            break;
        };
        legs.push((i, on));
        at = Some(on);
    }
    legs
}

/// A flat arrow one unit long, lying in the ground plane, its centre at the origin and its tip
/// along −Z (Bevy's forward); a little thick, so it reads edge-on too.
fn arrow_mesh() -> Mesh {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology};
    // The outline, (x, z): the shaft's tail, the head's shoulders, the tip.
    let outline: [[f32; 2]; 7] = [
        [-0.12, 0.5],
        [0.12, 0.5],
        [0.12, -0.1],
        [0.4, -0.1],
        [0.0, -0.5],
        [-0.4, -0.1],
        [-0.12, -0.1],
    ];
    let half = GUIDE_HALF_THICKNESS;
    let mut positions = Vec::new();
    for y in [half, -half] {
        positions.extend(outline.iter().map(|p| [p[0], y, p[1]]));
    }
    // Top and bottom: the shaft's two triangles and the head's one.
    let faces = [[0u32, 1, 2], [0, 2, 6], [5, 3, 4]];
    let mut indices: Vec<u32> = Vec::new();
    for f in faces {
        indices.extend(f);
        indices.extend(f.map(|i| i + 7));
    }
    // The rim.
    for i in 0..7u32 {
        let j = (i + 1) % 7;
        indices.extend([i, j, j + 7, i, j + 7, i + 7]);
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_indices(Indices::U32(indices));
    mesh.duplicate_vertices();
    mesh.compute_flat_normals();
    // Two tones: its top and bottom bright, its rim darker, so its shape reads at any angle.
    let colors: Vec<[f32; 4]> = match mesh.attribute(Mesh::ATTRIBUTE_NORMAL) {
        Some(bevy::mesh::VertexAttributeValues::Float32x3(normals)) => normals
            .iter()
            .map(|n| {
                if n[1].abs() > 0.9 {
                    [1.0, 1.0, 1.0, 1.0]
                } else {
                    [GUIDE_RIM, GUIDE_RIM, GUIDE_RIM, 1.0]
                }
            })
            .collect(),
        _ => Vec::new(),
    };
    if !colors.is_empty() {
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    }
    mesh
}

/// Half the guide arrow's thickness, and how dark its rim is against its top.
const GUIDE_HALF_THICKNESS: f32 = 0.06;
const GUIDE_RIM: f32 = 0.45;

/// The guide arrow's outline: how far it stands out round the arrow (units of its length), and
/// how far under the arrow's bottom face it lies.
const GUIDE_EDGE: f32 = 0.055;
const GUIDE_EDGE_DEPTH: f32 = 0.02;

/// The guide arrow's shape as [`arrow_mesh`] draws it, as two convex parts in the flat meshes'
/// plane (x across, y along, the tip at +y): the shaft and the head.
const GUIDE_SHAFT: [[f32; 2]; 4] = [[-0.12, -0.5], [0.12, -0.5], [0.12, 0.1], [-0.12, 0.1]];
const GUIDE_HEAD: [[f32; 2]; 3] = [[-0.4, 0.1], [0.4, 0.1], [0.0, 0.5]];

/// The guide arrow's outline: its two parts grown by [`GUIDE_EDGE`], one flat sheet.
fn guide_outline_mesh() -> Mesh {
    flat_mesh(
        &[
            grown(&GUIDE_SHAFT, GUIDE_EDGE),
            grown(&GUIDE_HEAD, GUIDE_EDGE),
        ],
        0.0,
    )
}

/// The outline's place in the arrow: the flat sheet's +y tip turned to the arrow's −Z, laid in the
/// arrow's plane just under its bottom face, so the border shows round the arrow's top.
fn guide_outline_place() -> Transform {
    Transform::from_xyz(0.0, -(GUIDE_HALF_THICKNESS + GUIDE_EDGE_DEPTH), 0.0)
        .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2))
}

/// The guide arrow's rotation: its −Z tip along the ground toward `to`, then its face (+Y) tipped
/// `tilt` degrees about the arrow's own axis toward `eye`, so it is not seen edge-on.
fn guide_rotation(at: Vec3, to: Vec3, eye: Vec3, tilt: f32) -> Quat {
    let level = pointing(at, Vec3::new(to.x, at.y, to.z));
    let tip = level * Vec3::NEG_Z;
    let to_eye = eye - at;
    let side = to_eye - tip * to_eye.dot(tip) - Vec3::Y * to_eye.y;
    let Some(side) = side.try_normalize() else {
        return level;
    };
    // Tipping +Y toward `side` is a turn about the tip's axis, signed by which side the eye is.
    let angle = tilt.clamp(0.0, GuideHud::MAX_TILT).to_radians();
    let sign = if Vec3::Y.cross(side).dot(tip) >= 0.0 {
        1.0
    } else {
        -1.0
    };
    Quat::from_axis_angle(tip, sign * angle) * level
}

/// The rotation turning the arrow's −Z to point from `from` to `to` (Bevy world, level).
fn pointing(from: Vec3, to: Vec3) -> Quat {
    let d = to - from;
    Quat::from_rotation_y((-d.x).atan2(-d.z))
}

/// The ground under a WoW x, y, in Bevy world; at height `fallback` where there is none.
fn ground_at(collide: &WorldCollision, p: [f32; 2], fallback: f32) -> Vec3 {
    let column = wow_to_bevy([p[0], p[1], 0.0]);
    let y = collide
        .ray_body(Vec3::new(column.x, 2000.0, column.z), Dir3::NEG_Y, 4000.0)
        .map_or(fallback, |hit| 2000.0 - hit.distance);
    Vec3::new(column.x, y, column.z)
}

/// Stands a beam at each checkpoint of the race we are in, the next one bright, and floats an
/// arrow in the next checkpoint and the one after, each pointing on to the checkpoint after it.
///
/// The arrow is a flat 2D sign whose tip lies along the line to the checkpoint after, turned about
/// that line to face the camera: it always points the true way, and reads from every side.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn beams(
    mut commands: Commands,
    time: Res<Time>,
    races: Res<Races>,
    collide: WorldCollision,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    camera: Query<&GlobalTransform, With<benilla_world::view::WorldCamera>>,
    mut beams: Query<
        (
            Entity,
            &Beam,
            &mut Transform,
            &mut MeshMaterial3d<StandardMaterial>,
        ),
        Without<CheckpointArrow>,
    >,
    mut arrows: Query<(Entity, &mut CheckpointArrow, &mut Transform), Without<Beam>>,
    mut looks: Local<Option<Looks>>,
) {
    let race = races.race.as_ref().filter(|r| r.go_at.is_some() && !r.done);
    let Some((race, next)) = race.and_then(|r| Some((r, r.next()?))) else {
        for (e, ..) in &beams {
            commands.entity(e).despawn();
        }
        for (e, ..) in &arrows {
            commands.entity(e).despawn();
        }
        return;
    };
    let looks = looks
        .get_or_insert_with(|| Looks::new(&mut meshes, &mut materials))
        .clone();
    let n = race.points.len();
    let mut have = vec![false; n];
    for (_, beam, mut t, mut material) in &mut beams {
        let Some(&p) = race.points.get(beam.0) else {
            continue;
        };
        have[beam.0] = true;
        let ground = ground_at(&collide, p, t.translation.y - BEAM_HEIGHT / 2.0);
        t.translation = ground + Vec3::Y * (BEAM_HEIGHT / 2.0);
        material.0 = if beam.0 == next {
            looks.bright.clone()
        } else {
            looks.dim.clone()
        };
    }
    for (i, p) in race.points.iter().enumerate() {
        if !have[i] {
            commands.spawn((
                Beam(i),
                Mesh3d(looks.beam.clone()),
                MeshMaterial3d(looks.dim.clone()),
                Transform::from_translation(wow_to_bevy([p[0], p[1], 0.0])),
                Name::new("race checkpoint"),
            ));
        }
    }

    // The arrows: turned to face the camera, so none without one. An arrow not wanted is gone
    // rather than hidden, and one wanted again is spawned where it goes.
    let legs = arrow_legs(race);
    let eye = camera.single().ok().map(GlobalTransform::translation);
    let phase = std::f32::consts::TAU * CP_ARROW_HZ * time.elapsed_secs();
    let place = |slot: usize, fallback: f32| {
        let (i, on) = *legs.get(slot)?;
        let eye = eye?;
        let ground = ground_at(&collide, race.points[i], fallback);
        let at = ground + Vec3::Y * CP_ARROW_HEIGHT;
        let to = ground_at(&collide, race.points[on], ground.y) + Vec3::Y * CP_ARROW_HEIGHT;
        let size = if slot == 0 {
            CP_ARROW_SIZE
        } else {
            CP_ARROW_SIZE * CP_ARROW_SECOND
        };
        let placed = Transform::from_translation(at + Vec3::Y * (CP_ARROW_BOB * phase.sin()))
            .with_rotation(checkpoint_arrow_rotation(at, to, eye))
            .with_scale(Vec3::splat(
                size * (1.0 + CP_ARROW_PULSE * (2.0 * phase).sin()),
            ));
        Some((ground.y, placed))
    };
    let mut have_arrow = [false; 2];
    for (e, mut arrow, mut t) in &mut arrows {
        let placed = have_arrow
            .get(arrow.slot)
            .filter(|had| !**had)
            .and_then(|_| place(arrow.slot, arrow.ground));
        let Some((ground, placed)) = placed else {
            commands.entity(e).despawn();
            continue;
        };
        have_arrow[arrow.slot] = true;
        arrow.ground = ground;
        *t = placed;
    }
    for (slot, had) in have_arrow.into_iter().enumerate() {
        let Some((ground, placed)) = (!had).then(|| place(slot, 0.0)).flatten() else {
            continue;
        };
        let face = if slot == 0 {
            looks.cp_next.clone()
        } else {
            looks.cp_after.clone()
        };
        commands
            .spawn((
                CheckpointArrow { slot, ground },
                Mesh3d(looks.cp_face.clone()),
                MeshMaterial3d(face),
                placed,
                Name::new("race checkpoint arrow"),
            ))
            .with_child((
                Mesh3d(looks.cp_outline.clone()),
                MeshMaterial3d(looks.cp_edge.clone()),
                Transform::default(),
                Name::new("race checkpoint arrow outline"),
            ));
    }
}

/// Whether checkpoint `i` is the finish: a sprint's last, or a circuit's start on its last lap.
fn is_finish(race: &Active, i: usize) -> bool {
    if race.laps == 0 {
        i + 1 == race.points.len()
    } else {
        i == 0 && race.lap + 1 >= race.laps
    }
}

/// What the minimap calls the next checkpoint.
fn checkpoint_name(race: &Active, next: usize) -> String {
    let n = race.points.len();
    if is_finish(race, next) {
        "Finish".into()
    } else if race.laps > 0 && next == 0 {
        format!("Lap {} of {}", race.lap + 2, race.laps)
    } else {
        format!("Checkpoint {next} of {}", n - 1)
    }
}

/// Where the guide arrow sits on screen and how big it is: `x`, `y` across and down the screen
/// (0 to 1), `scale` times its standard size, and whether it shows. The settings window places
/// it (`/kartcraft`, "Move arrow"): its CVars, read by [`super::settings::on_cvar`].
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub(crate) struct GuideHud {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) scale: f32,
    /// Shown in a race (`KartArrowShow`).
    pub(crate) shown: bool,
    /// Being placed (`KartArrowMove`): shown, pointing north, race or not.
    pub(crate) editing: bool,
    /// How far its face tips up toward the viewer, degrees (`KartArrowTilt`): 0 lies level.
    pub(crate) tilt: f32,
    /// The field of view it is seen through, degrees (`KartArrowFov`): narrower is flatter.
    pub(crate) fov: f32,
}

impl Default for GuideHud {
    fn default() -> Self {
        Self {
            x: 0.5,
            y: 0.22,
            scale: 1.0,
            shown: true,
            editing: false,
            tilt: 30.0,
            fov: 45.0,
        }
    }
}

impl GuideHud {
    pub(crate) const MIN_SCALE: f32 = 0.4;
    pub(crate) const MAX_SCALE: f32 = 3.0;
    pub(crate) const MAX_TILT: f32 = 80.0;
    pub(crate) const MIN_FOV: f32 = 20.0;
    pub(crate) const MAX_FOV: f32 = 110.0;

    /// How far out the arrow floats, yards, to be seen through its own field of view from a
    /// camera with `world_fov` (radians): the same size on screen, its perspective that of a
    /// camera with `fov`. Further out and larger is flatter; capped so it stays close in.
    fn distance(&self, world_fov: f32) -> f32 {
        let own = self.fov.clamp(Self::MIN_FOV, Self::MAX_FOV).to_radians();
        (GUIDE_DISTANCE * (world_fov / 2.0).tan() / (own / 2.0).tan()).clamp(1.0, 24.0)
    }

    /// The arrow's place in front of a camera looking down −Z with this vertical field of view
    /// and aspect, `distance` ahead: the point under (x, y) on screen.
    fn local_position(&self, fov_y: f32, aspect: f32, distance: f32) -> Vec3 {
        let half = (fov_y / 2.0).tan() * distance;
        Vec3::new(
            (self.x * 2.0 - 1.0) * half * aspect,
            (1.0 - self.y * 2.0) * half,
            -distance,
        )
    }
}

/// How far in front of the camera the guide arrow floats (yards), and its length there at
/// scale 1.
const GUIDE_DISTANCE: f32 = 4.0;
const GUIDE_SIZE: f32 = 0.7;

/// In a race (or while it is being placed, race or not): the arrow towards the next checkpoint, a child of
/// the camera so it holds its place on screen; and that checkpoint on the minimap, in the slot a
/// guard's directions use (`crate::poi_marker`).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn guide(
    mut commands: Commands,
    races: Res<Races>,
    drive: Res<super::KartDrive>,
    hud: Res<GuideHud>,
    collide: WorldCollision,
    mut marker: ResMut<crate::poi_marker::PoiMarker>,
    current_map: Option<Res<benilla_world::world_map::CurrentMap>>,
    real: Res<Time<bevy::time::Real>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    camera: Query<(Entity, &GlobalTransform, &Projection), With<benilla_world::view::WorldCamera>>,
    mut arrows: Query<(Entity, &mut Transform, &ChildOf), With<GuideArrow>>,
    mut looks: Local<Option<Looks>>,
    mut shown: Local<Option<String>>,
) {
    let race = races.race.as_ref().filter(|r| r.go_at.is_some() && !r.done);
    let target = race
        .and_then(|r| Some((r, r.next()?)))
        .filter(|_| drive.active);

    // The minimap.
    match target {
        Some((race, next)) => {
            let name = checkpoint_name(race, next);
            if let Some(old) = shown.as_ref().filter(|old| **old != name) {
                marker.clear_named(old);
            }
            let map = current_map.as_ref().map_or(0, |m| m.0);
            marker.set_landmark(&name, race.points[next], map, real.elapsed_secs_f64());
            *shown = Some(name);
        }
        None => {
            if let Some(name) = shown.take() {
                marker.clear_named(&name);
            }
        }
    }

    // The arrow: north while it is being placed, else towards the next checkpoint if it shows.
    let cam = camera.single().ok();
    let towards = match (target, cam) {
        (_, Some((_, cam_at, _))) if hud.editing => {
            Some(cam_at.translation() + wow_to_bevy([100.0, 0.0, 0.0]))
        }
        (Some((race, next)), _) if hud.shown => {
            Some(ground_at(&collide, race.points[next], drive.pos.y))
        }
        _ => None,
    };
    let (Some(towards), Some((cam, cam_at, projection))) = (towards, cam) else {
        for (e, ..) in &arrows {
            commands.entity(e).despawn();
        }
        return;
    };
    let (fov, aspect) = match projection {
        Projection::Perspective(p) => (p.fov, p.aspect_ratio),
        _ => (1.0, 16.0 / 9.0),
    };
    let distance = hud.distance(fov);
    let local = hud.local_position(fov, aspect, distance);
    // Pointing along the ground at the checkpoint, its face tipped up toward the camera.
    let at = cam_at.transform_point(local);
    let world = guide_rotation(at, towards, cam_at.translation(), hud.tilt);
    let rotation = cam_at.compute_transform().rotation.inverse() * world;
    let scale = Vec3::splat(GUIDE_SIZE * hud.scale * distance / GUIDE_DISTANCE);
    match arrows.single_mut() {
        Ok((_, mut t, parent)) if parent.parent() == cam => {
            t.translation = local;
            t.rotation = rotation;
            t.scale = scale;
        }
        _ => {
            for (e, ..) in &arrows {
                commands.entity(e).despawn();
            }
            let looks = looks.get_or_insert_with(|| Looks::new(&mut meshes, &mut materials));
            commands
                .spawn((
                    GuideArrow,
                    Mesh3d(looks.arrow.clone()),
                    MeshMaterial3d(looks.guide.clone()),
                    Transform::from_translation(local)
                        .with_rotation(rotation)
                        .with_scale(scale),
                    ChildOf(cam),
                    Name::new("race guide arrow"),
                ))
                .with_child((
                    Mesh3d(looks.guide_outline.clone()),
                    MeshMaterial3d(looks.cp_edge.clone()),
                    guide_outline_place(),
                    Name::new("race guide arrow outline"),
                ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_courses_file_holds_the_courses_and_their_starts() {
        let course = |key: &str| COURSES.iter().find(|c| c.key == key).unwrap();
        assert_eq!(course("sprint").name, "Northshire Sprint");
        assert_eq!(course("sprint").start(), [-8930.0, -118.0]);
        assert_eq!(course("stormwind").start(), [-9436.0, 59.0]);
        assert!(course("stormwind")
            .gates()
            .iter()
            .all(|&g| g == GATE_HALF_WIDTH));
        let mut keys: Vec<&str> = COURSES.iter().map(|c| c.key.as_str()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), COURSES.len(), "keys are unique");
        for c in COURSES.iter() {
            assert!(c.checkpoints.len() >= 2, "{} has a course", c.key);
            assert!(
                ZONES.contains(&c.zone),
                "{} is in a Race Finder zone",
                c.key
            );
        }
    }

    #[test]
    fn clocks_and_ordinals_read_like_ctr() {
        assert_eq!(clock(83_456), "1:23.45");
        assert_eq!(clock(5_000), "0:05.00");
        assert_eq!(ordinal(1), "1ST");
        assert_eq!(ordinal(3), "3RD");
        assert_eq!(ordinal(7), "7TH");
    }

    #[test]
    fn the_board_marks_new_bests_and_ends_with_the_record() {
        let mut race = Active::default();
        assert!(board(&race).is_empty(), "no board before a finish");
        race.finished = vec![("Racer".into(), 1, 62_400), ("Goblin".into(), 2, 65_000)];
        race.bests.insert("Racer".into(), true);
        race.bests.insert("Goblin".into(), false);
        race.record = (62_400, "Racer".into());
        race.best = 62_400;
        assert_eq!(
            board(&race),
            [
                "1. Racer  1:02.40  (record)",
                "2. Goblin  1:05.00  (best)",
                "Course record: 1:02.40 by Racer",
                "Your best: 1:02.40",
            ]
        );
        // No record yet and no best of ours: just the finishes.
        race.record = (0, String::new());
        race.best = 0;
        assert_eq!(board(&race).len(), 2);
    }

    #[test]
    fn a_race_is_kept_through_a_worldport_and_left_when_the_kart_is_gone() {
        // In the kart, or with no object of ours (a loading screen), the race stays.
        assert!(!left_without_kart(Some(true), 30.0, 10.0));
        assert!(
            !left_without_kart(None, 30.0, 10.0),
            "a worldport is no answer"
        );
        // No kart, but a kart being summoned still has its grace.
        assert!(!left_without_kart(Some(false), 5.0, 10.0));
        // Our object back without Summon Kart, the grace run out: out of the race.
        assert!(left_without_kart(Some(false), 30.0, 10.0));
    }

    #[test]
    fn a_track_describes_its_kind_length_and_lap() {
        let circuit = COURSES
            .iter()
            .find(|c| c.key == "elwynn-fargodeep-maclure")
            .expect("the course");
        let text = circuit.description();
        assert!(
            text.starts_with("A 3-lap circuit in Elwynn Forest, 2.")
                && text.contains(" km a lap through ")
                && text.ends_with("A clean lap takes about 1:16."),
            "{text}"
        );
        let sprint = COURSES
            .iter()
            .find(|c| c.key == "sprint")
            .expect("the sprint");
        let text = sprint.description();
        assert!(
            text.starts_with("A sprint") && text.contains("from start to finish"),
            "{text}"
        );
        assert!(
            !text.contains("takes about"),
            "no test lap in its note: {text}"
        );
    }

    #[test]
    fn standings_order_by_laps_then_checkpoints_then_first_there() {
        let a = standing(&(1, 2, 10.0), 6);
        let b = standing(&(1, 3, 11.0), 6);
        let c = standing(&(1, 3, 12.0), 6);
        assert!(b > a);
        assert!(b > c, "the earlier of two at the same checkpoint leads");
    }

    /// A sprint east along x, then a bend north: checkpoints at x 0, 100, 200 (then y 100).
    fn course() -> Active {
        Active {
            laps: 0,
            points: vec![[0.0, 0.0], [100.0, 0.0], [200.0, 0.0], [200.0, 100.0]],
            ..Default::default()
        }
    }

    #[test]
    fn a_gate_counts_when_crossed_not_when_merely_near() {
        let race = course();
        // 30 yd short of checkpoint 1 is no longer "passed" (the old 38 yd circle was).
        assert!(!race.passes(1, Some([69.0, 0.0]), [70.0, 0.0]));
        // Crossing its line, a little off centre, is.
        assert!(race.passes(1, Some([99.5, 6.0]), [100.5, 6.0]));
        // Crossing the line 30 yd to the side is not.
        assert!(!race.passes(1, Some([99.5, 30.0]), [100.5, 30.0]));
        // Driving back across it (off the centre) is not.
        assert!(!race.passes(1, Some([100.5, 12.0]), [99.5, 12.0]));
        // Arriving on the centre is.
        assert!(race.passes(1, None, [100.0, 5.0]));
    }

    #[test]
    fn a_gate_at_a_bend_stands_square_to_both_legs() {
        let race = course();
        let h = race.heading(2);
        assert!(
            (h[0] - h[1]).abs() < 1e-4 && h[0] > 0.0,
            "halfway between east and north: {h:?}"
        );
        // Cutting the corner diagonally through the gate counts.
        assert!(race.passes(2, Some([190.0, -2.0]), [194.0, 8.0]));
    }

    #[test]
    fn the_guide_arrow_lands_where_it_is_put() {
        // The screen's centre is straight ahead; its top-left corner is up and to the left.
        let centre = GuideHud {
            x: 0.5,
            y: 0.5,
            ..default()
        }
        .local_position(1.0, 2.0, 4.0);
        assert!(centre.abs_diff_eq(Vec3::new(0.0, 0.0, -4.0), 1e-5));
        let corner = GuideHud {
            x: 0.0,
            y: 0.0,
            ..default()
        }
        .local_position(1.0, 2.0, 4.0);
        let half = 0.5f32.tan() * 4.0;
        assert!(corner.abs_diff_eq(Vec3::new(-half * 2.0, half, -4.0), 1e-4));
    }

    #[test]
    fn checkpoint_arrows_point_on_to_the_checkpoint_after_and_never_in_a_finish() {
        // The sprint: the next checkpoint's arrow and the one after's, each to the one after it.
        let mut race = course();
        assert_eq!(arrow_legs(&race), [(1, 2), (2, 3)]);
        // The last before the finish shows alone: the finish has none.
        race.checkpoint = 1;
        assert_eq!(arrow_legs(&race), [(2, 3)]);
        race.checkpoint = 2;
        assert!(arrow_legs(&race).is_empty(), "nothing in a sprint's finish");
        // A circuit's last checkpoint points to the start, and the start on to the first.
        let mut lap = Active {
            laps: 3,
            checkpoint: 2,
            ..course()
        };
        assert_eq!(arrow_legs(&lap), [(3, 0), (0, 1)]);
        // On the last lap the start is the finish: no arrow in it.
        lap.lap = 2;
        assert_eq!(arrow_legs(&lap), [(3, 0)]);
        lap.checkpoint = 3;
        assert!(arrow_legs(&lap).is_empty());
    }

    #[test]
    fn checkpoints_passed_show_no_arrow() {
        for passed in 0..3 {
            let race = Active {
                checkpoint: passed,
                ..course()
            };
            for (i, on) in arrow_legs(&race) {
                assert!(i > passed, "checkpoint {i} is passed at {passed}");
                assert_eq!(on, i + 1);
            }
        }
        let lap = Active {
            laps: 2,
            checkpoint: 1,
            ..course()
        };
        assert!(arrow_legs(&lap).iter().all(|&(i, _)| i == 2 || i == 3));
    }

    #[test]
    fn the_checkpoint_arrow_points_along_the_line_to_the_next_and_faces_the_eye() {
        let pairs = [
            (Vec3::new(0.0, 5.0, 0.0), Vec3::new(100.0, 5.0, 0.0)),
            (Vec3::new(10.0, 5.0, -20.0), Vec3::new(-60.0, 12.0, 80.0)),
            (Vec3::new(-5.0, 2.0, 7.0), Vec3::new(-5.0, 30.0, 7.5)),
        ];
        let eyes = [
            Vec3::new(0.0, 10.0, 50.0),
            Vec3::new(-80.0, 3.0, -10.0),
            Vec3::new(200.0, 40.0, 0.0),
            Vec3::new(30.0, -5.0, 90.0),
        ];
        for (from, to) in pairs {
            let want = (to - from).normalize();
            for eye in eyes {
                let r = checkpoint_arrow_rotation(from, to, eye);
                // The tip is the true direction, wherever the eye is.
                assert!((r * Vec3::Y).abs_diff_eq(want, 1e-4), "{from} {to} {eye}");
                // The face is square to the line and turned towards the eye.
                let face = r * Vec3::Z;
                assert!(face.dot(want).abs() < 1e-4);
                assert!(face.dot(eye - from) >= 0.0, "{from} {to} {eye}");
                assert!(r.is_normalized());
            }
        }
        // An eye straight down the line still gives a proper rotation along it.
        let r = checkpoint_arrow_rotation(Vec3::ZERO, Vec3::X, Vec3::X * 50.0);
        assert!((r * Vec3::Y).abs_diff_eq(Vec3::X, 1e-4) && r.is_normalized());
    }

    #[test]
    fn the_guide_arrow_points_at_the_checkpoint_and_tips_its_face_toward_the_eye() {
        let at = Vec3::new(0.0, 2.0, 0.0);
        let to = Vec3::new(50.0, 0.0, -30.0);
        let level_tip = Vec3::new(to.x, 0.0, to.z).normalize();
        for eye in [
            Vec3::new(-6.0, 4.0, 3.0),
            Vec3::new(3.0, 3.0, 8.0),
            Vec3::new(9.0, 1.0, -9.0),
        ] {
            let level = guide_rotation(at, to, eye, 0.0);
            let tilted = guide_rotation(at, to, eye, 40.0);
            // Either way the tip lies along the ground toward the checkpoint.
            assert!((level * Vec3::NEG_Z).abs_diff_eq(level_tip, 1e-4));
            assert!((tilted * Vec3::NEG_Z).abs_diff_eq(level_tip, 1e-4));
            // Level, the face is up; tilted 40 degrees, it leans that far toward the eye.
            assert!((level * Vec3::Y).abs_diff_eq(Vec3::Y, 1e-4));
            let face = tilted * Vec3::Y;
            assert!((face.y - 40f32.to_radians().cos()).abs() < 1e-4);
            // It leans to the eye's side of the arrow, across the ground square to its tip.
            let to_eye = eye - at;
            let side = to_eye - level_tip * to_eye.dot(level_tip) - Vec3::Y * to_eye.y;
            assert!(
                Vec3::new(face.x, 0.0, face.z).dot(side) > 0.0,
                "{eye}: {face}"
            );
        }
    }

    #[test]
    fn the_guide_arrow_s_own_field_of_view_moves_it_out_at_the_same_size() {
        let world = 45f32.to_radians();
        let mut hud = GuideHud::default();
        // At the world's own field of view it floats where it always has.
        assert!((hud.distance(world) - GUIDE_DISTANCE).abs() < 1e-4);
        // Narrower is further out and grown alike, so its size on screen holds.
        hud.fov = 25.0;
        let d = hud.distance(world);
        assert!(d > GUIDE_DISTANCE * 1.5);
        let on_screen = |d: f32| (GUIDE_SIZE * d / GUIDE_DISTANCE) / d;
        assert!((on_screen(d) - on_screen(GUIDE_DISTANCE)).abs() < 1e-6);
        // Wider comes in, never closer than a yard.
        hud.fov = 110.0;
        assert!(hud.distance(world) < GUIDE_DISTANCE && hud.distance(world) >= 1.0);
    }

    #[test]
    fn the_guide_arrow_outline_lies_under_it_and_shows_round_its_rim() {
        use bevy::mesh::VertexAttributeValues;
        let points = |mesh: &Mesh| -> Vec<Vec3> {
            match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
                Some(VertexAttributeValues::Float32x3(p)) => {
                    p.iter().map(|p| Vec3::from_array(*p)).collect()
                }
                _ => panic!("no positions"),
            }
        };
        let arrow = points(&arrow_mesh());
        let place = guide_outline_place();
        let outline: Vec<Vec3> = points(&guide_outline_mesh())
            .into_iter()
            .map(|p| place.transform_point(p))
            .collect();
        // Flat, just under the arrow's bottom face.
        let bottom = arrow.iter().map(|p| p.y).fold(f32::MAX, f32::min);
        for p in &outline {
            assert!(p.y < bottom && p.y > bottom - 0.05, "{p}");
        }
        // Beyond the arrow on every side: across, at the tail and past the tip (−Z).
        let span = |pts: &[Vec3], f: fn(&Vec3) -> f32| {
            let v: Vec<f32> = pts.iter().map(f).collect();
            (
                v.iter().copied().fold(f32::MAX, f32::min),
                v.iter().copied().fold(f32::MIN, f32::max),
            )
        };
        let (ax, bx) = (span(&arrow, |p| p.x), span(&outline, |p| p.x));
        let (az, bz) = (span(&arrow, |p| p.z), span(&outline, |p| p.z));
        assert!(bx.0 < ax.0 - 0.03 && bx.1 > ax.1 + 0.03, "{ax:?} {bx:?}");
        assert!(bz.0 < az.0 - 0.03 && bz.1 > az.1 + 0.03, "{az:?} {bz:?}");
        // The arrow is two-toned: some vertices darker (the rim) than the rest.
        let Some(VertexAttributeValues::Float32x4(colors)) =
            arrow_mesh().attribute(Mesh::ATTRIBUTE_COLOR).cloned()
        else {
            panic!("no colours");
        };
        assert!(colors.iter().any(|c| c[0] < 0.5) && colors.iter().any(|c| c[0] == 1.0));
    }

    #[test]
    fn the_checkpoint_arrow_outline_surrounds_its_face() {
        // Every corner of the face lies inside its grown outline, by the edge width.
        for (face, outline) in [
            (
                CP_ARROW_SHAFT.to_vec(),
                grown(&CP_ARROW_SHAFT, CP_ARROW_EDGE),
            ),
            (CP_ARROW_HEAD.to_vec(), grown(&CP_ARROW_HEAD, CP_ARROW_EDGE)),
        ] {
            let n = outline.len();
            for p in &face {
                for k in 0..n {
                    let (a, b) = (Vec2::from(outline[k]), Vec2::from(outline[(k + 1) % n]));
                    let inward = (b - a).perp().normalize();
                    let depth = (Vec2::from(*p) - a).dot(inward);
                    assert!(depth >= CP_ARROW_EDGE - 1e-4, "{p:?} {depth}");
                }
            }
        }
        // The shaft grows square: its corners move out by the edge on both axes.
        assert!(
            Vec2::from(grown(&CP_ARROW_SHAFT, 0.1)[0]).abs_diff_eq(Vec2::new(-0.26, -0.6), 1e-5)
        );
    }

    #[test]
    fn a_start_names_its_courses_and_sends_the_driver_to_the_race_marshal() {
        let course = |key: &str| COURSES.iter().find(|c| c.key == key).unwrap();
        assert_eq!(start_banner(&[]), "");
        assert_eq!(
            start_banner(&[course("sprint")]),
            "Northshire Sprint starts here — talk to the Race Marshal to race"
        );
        assert_eq!(
            start_banner(&[
                course("sprint"),
                course("stormwind"),
                course("elwynn-goldshire-crystal")
            ]),
            "Northshire Sprint, Road to Stormwind and Goldshire & Crystal Lake start here — talk \
             to the Race Marshal to race"
        );
    }

    #[test]
    fn the_countdown_gives_a_kart_taken_off_for_the_grid_time_to_come_back() {
        // A race hosted long ago: its first grace is long gone.
        let mut race = course();
        race.grace_until = 10.0;
        race.checkpoint = 2;
        race.count_down(100.0, 8000.0);
        assert_eq!(race.go_at, Some(108.0));
        assert_eq!(race.checkpoint, 0, "from scratch");
        // Out of the kart for the teleport: kept while the kart comes back, left after.
        assert!(!left_without_kart(Some(false), 101.5, race.grace_until));
        assert!(left_without_kart(
            Some(false),
            100.0 + KART_GRACE_SECS,
            race.grace_until
        ));
    }

    #[test]
    fn a_far_grid_teleport_keeps_the_race_while_the_kart_comes_back_after_the_load() {
        // The countdown as the teleport starts, then a loading screen far longer than the grace.
        let mut race = course();
        race.count_down(100.0, 28000.0);
        let mut t = 100.0;
        while t < 125.0 {
            race.landing(None, t);
            assert!(!left_without_kart(None, t, race.grace_until));
            t += 0.5;
        }
        // Landed at 125: no kart yet (the server summons it after we arrive), still racing.
        race.landing(Some(false), 125.0);
        assert!(!left_without_kart(Some(false), 128.0, race.grace_until));
        // In the kart again: racing; never back in it: out, a grace after landing.
        race.landing(Some(true), 128.5);
        assert!(!left_without_kart(Some(true), 140.0, race.grace_until));
        assert!(left_without_kart(
            Some(false),
            125.0 + KART_GRACE_SECS,
            race.grace_until
        ));
    }

    #[test]
    fn a_sprint_finish_is_reported_once() {
        let mut race = course();
        race.checkpoint = 2;
        assert_eq!(race.next(), Some(3));
        race.checkpoint = 3;
        assert_eq!(
            race.next(),
            None,
            "past the finish there is nothing to reach"
        );
        race.laps = 2;
        assert_eq!(race.next(), Some(0));
    }

    #[test]
    fn the_minimap_names_checkpoints_laps_and_the_finish() {
        let mut race = course();
        assert_eq!(checkpoint_name(&race, 1), "Checkpoint 1 of 3");
        assert_eq!(checkpoint_name(&race, 3), "Finish");
        assert_eq!(race.after(3), None);
        race.laps = 3;
        assert_eq!(checkpoint_name(&race, 0), "Lap 2 of 3");
        race.lap = 2;
        assert_eq!(checkpoint_name(&race, 0), "Finish");
        assert_eq!(race.after(3), Some(0));
    }
}
