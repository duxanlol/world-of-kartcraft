//! Races between karts, refereed by the Kartcraft server (`Kartcraft.cpp`, "Races"). At a
//! course's start F5 hosts a race; drivers near it see the offer and F5 joins; the host's F5
//! starts it. The server sends the course's checkpoints and a countdown: CTR's 3-2-1-GO, the kart
//! held on the line, with CTR's start boost for an engine revved just before GO. Each checkpoint
//! passed is reported, and the server checks it against where we really are; it relays everyone's
//! progress, from which our place is worked out, and the finishes with their times. Checkpoints
//! stand in the world as beams of light, the next one bright. F6 leaves a race. Where two
//! courses share a start, F7 picks which one F5 hosts.
//!
//! The Race Finder (`finder.rs`, `I`) queues for races from anywhere: the server groups the
//! queued, sends a ready check (F5 accepts, F6 declines) and teleports everyone who accepted to
//! the start. Its lines: `Qs` (queue status), `Qx` (not queued), `Pop` (ready check), `Pop-`
//! (the ready check failed); `Q+`, `Q-`, `W` (heartbeat), `Y` and `N` go up.

use std::collections::HashMap;

use benilla_assets::coords::{bevy_to_wow, wow_to_bevy};
use benilla_protocol::SessionEvent;
use benilla_world::collision::WorldCollision;
use bevy::prelude::*;

/// The courses' starts, so a driver at one may host (the server's table, `Kartcraft.cpp`,
/// holds the whole course and sends it at the start).
pub(crate) const COURSES: [(&str, &str, [f32; 2]); 3] = [
    ("sprint", "Northshire Sprint", [-8930.0, -118.0]),
    ("elwynn", "Elwynn Loop", [-9436.0, 59.0]),
    ("stormwind", "Road to Stormwind", [-9436.0, 59.0]),
];
/// How near a start (yards) a driver may host or join.
const START_RADIUS: f32 = 55.0;
/// A checkpoint is a gate: passed on crossing the line through it across the course, within
/// this many yards of its centre, or on coming this near it (the server allows 45).
const GATE_HALF_WIDTH: f32 = 20.0;
const CHECKPOINT_RADIUS: f32 = 8.0;
/// The arrow over our kart (yards above its origin, and its length).
const GUIDE_HEIGHT: f32 = 3.4;
const GUIDE_LENGTH: f32 = 1.6;
/// The arrow on each beam towards the checkpoint after it (yards above the ground, length).
const BEAM_ARROW_HEIGHT: f32 = 9.0;
const BEAM_ARROW_LENGTH: f32 = 6.0;
/// The start boost: revving this long before GO (s) is big, a little earlier small, earlier
/// still bogs the engine down.
const BOOST_BIG: std::ops::Range<f32> = 0.15..0.6;
const BOOST_SMALL: std::ops::Range<f32> = 0.6..1.2;
/// How often a queued finder says it is still there (s); the server drops it after 10.
const FINDER_HEARTBEAT: f32 = 2.0;
/// A checkpoint's beam: its height (yards) and radius.
const BEAM_HEIGHT: f32 = 40.0;
const BEAM_RADIUS: f32 = 1.2;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Offer {
    id: u32,
    name: String,
    host: String,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Active {
    id: u32,
    laps: u8,
    /// Checkpoints in WoW x, y; the first is the start (and finish of a lap).
    points: Vec<[f32; 2]>,
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
    done: bool,
    /// When the gas went down during the countdown (local s), for the start boost.
    revved_at: Option<f32>,
    beeped: i32,
    /// The standings, leader first, as last worked out.
    order: Vec<String>,
    /// Where we were last frame (WoW x, y), to see a gate crossed.
    last_at: Option<[f32; 2]>,
    /// A Race Finder race is not left for want of a kart until then (local s): the server
    /// summons ours and teleports us, and the aura may land after the roster.
    grace_until: f32,
}

impl Active {
    /// The checkpoint to reach next; `None` before there is a course.
    fn next(&self) -> Option<usize> {
        let n = self.points.len();
        (n > 1).then(|| {
            if self.laps > 0 {
                (self.checkpoint + 1) % n
            } else {
                (self.checkpoint + 1).min(n - 1)
            }
        })
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
        along(from) < 0.0 && along(to) >= 0.0 && across(to).min(across(from)) <= GATE_HALF_WIDTH
    }
}

/// A ready check from the Race Finder.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ReadyCheck {
    pub(crate) id: u32,
    pub(crate) course: String,
    pub(crate) racers: u32,
    /// When the answer is due, local seconds.
    pub(crate) until: f32,
    pub(crate) accepted: bool,
}

/// The Race Finder's side of the races.
#[derive(Clone, Debug, Default)]
pub(crate) struct Finder {
    /// The course keys we queued for (`any` for all), while in the queue.
    pub(crate) queued: Option<String>,
    /// (racers waiting for our courses, seconds we have waited) from the server.
    pub(crate) status: Option<(u32, u32)>,
    pub(crate) check: Option<ReadyCheck>,
    beat_at: f32,
}

#[derive(Resource, Default)]
pub(crate) struct Races {
    pub(crate) offer: Option<Offer>,
    pub(crate) finder: Finder,
    /// Which of the courses sharing a start F5 hosts (F7 steps it).
    pick: usize,
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

    /// The racers by name, leader first.
    pub(crate) fn order(&self) -> &[String] {
        self.race.as_ref().map_or(&[], |r| r.order.as_slice())
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

/// Handles the server's lines, the keys, the countdown, checkpoints and standings.
#[allow(clippy::too_many_arguments)]
pub(crate) fn update(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    ui: Res<crate::ui_script::UiKeyboardCapture>,
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
            Some("Qs") => races.finder.status = parse_status(&f),
            Some("Qx") => {
                races.finder.queued = None;
                races.finder.status = None;
            }
            Some("Pop") if f.len() >= 5 => {
                races.finder.check = Some(ReadyCheck {
                    id,
                    course: f[2].clone(),
                    racers: f[3].parse().unwrap_or(0),
                    until: now + f[4].parse::<f32>().unwrap_or(20.0),
                    accepted: false,
                });
                audio.cues.push(kartcraft::sound::Cue::Countdown(true));
            }
            Some("Pop-") => {
                // The server queues again those who accepted.
                if races.finder.check.take().is_none_or(|c| !c.accepted) {
                    races.finder.queued = None;
                }
                races.finder.status = None;
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
                    let from_finder = races.finder.check.as_ref().is_some_and(|c| c.accepted);
                    races.finder.check = None;
                    races.finder.queued = None;
                    races.finder.status = None;
                    let race = races.race.get_or_insert_with(|| Active {
                        id,
                        grace_until: if from_finder { now + 10.0 } else { 0.0 },
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
                    race.laps = f[3].parse().unwrap_or(0);
                    race.points = f[4]
                        .split(';')
                        .filter_map(|p| {
                            let (x, y) = p.split_once(':')?;
                            Some([x.parse().ok()?, y.parse().ok()?])
                        })
                        .collect();
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
                    let ms: f32 = f[2].parse().unwrap_or(4000.0);
                    race.go_at = Some(now + ms / 1000.0);
                    race.lap = 0;
                    race.checkpoint = 0;
                    race.progress.clear();
                    race.finished.clear();
                    race.revved_at = None;
                    race.beeped = 0;
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
                    info!("kart race: {} finished {place} in {ms} ms", f[2]);
                    race.finished.push((f[2].clone(), place, ms));
                    if f[2] == me {
                        race.done = true;
                    }
                }
            }
            _ => {}
        }
    }

    // F5: answer a ready check, else join, host, or start; F6: decline or leave. `WOW_KART_RACE`
    // presses F5 for unattended runs.
    let f5 = (keys.just_pressed(KeyCode::F5) && !ui.typing) || auto.press(now, races);
    let f6 = keys.just_pressed(KeyCode::F6) && !ui.typing;

    // The Race Finder: the heartbeat that keeps our place, and the ready check's answer.
    let finder = &mut races.finder;
    if finder.check.as_ref().is_some_and(|c| now > c.until + 2.0) {
        finder.check = None;
    }
    if (finder.queued.is_some() || finder.check.is_some()) && now >= finder.beat_at {
        finder.beat_at = now + FINDER_HEARTBEAT;
        profession.send("KARTCRAFT\tR\tW".into());
    }
    let answering = races.race.is_none() && finder.check.as_ref().is_some_and(|c| !c.accepted);
    if answering && (f5 || f6) {
        if let Some(check) = finder.check.as_mut() {
            if f5 {
                check.accepted = true;
                profession.send(format!("KARTCRAFT\tR\tY\t{}", check.id));
            } else {
                profession.send(format!("KARTCRAFT\tR\tN\t{}", check.id));
                finder.check = None;
                finder.queued = None;
            }
        }
    }
    let (f5, f6) = if answering { (false, false) } else { (f5, f6) };
    *text = RaceText::default();
    let queue_banner = finder_banner(&races.finder, now);

    // Out of the kart, out of the race; but not while the kart aura stays on us, as it does
    // through the Race Finder's teleport to the start.
    if !drive.active {
        let grace = races.race.as_ref().is_some_and(|r| now < r.grace_until);
        if !profession.auras().kart && !grace {
            if let Some(race) = races.race.take() {
                profession.send(format!("KARTCRAFT\tR\tQ\t{}", race.id));
            }
        }
        races.offer = None;
        extras.lap = None;
        extras.place = None;
        audio.music_rate = 1.0;
        text.banner = if races.race.is_some() {
            "Getting to the start...".into()
        } else {
            queue_banner
        };
        return;
    }
    let wow = bevy_to_wow(drive.pos);
    let at = [wow[0], wow[1]];
    let near = |p: [f32; 2], r: f32| (p[0] - at[0]).hypot(p[1] - at[1]) <= r;

    // The courses starting here; F7 steps through them where several share a start.
    let here: Vec<_> = COURSES.iter().filter(|c| near(c.2, START_RADIUS)).collect();
    if keys.just_pressed(KeyCode::F7) && !ui.typing && here.len() > 1 {
        races.pick += 1;
    }
    let host_here = (!here.is_empty()).then(|| here[races.pick % here.len()]);
    if f6 {
        if let Some(race) = races.race.take() {
            profession.send(format!("KARTCRAFT\tR\tQ\t{}", race.id));
        }
    } else if f5 {
        match (&races.race, &races.offer, host_here) {
            (Some(race), _, _) if race.host && race.go_at.is_none() => {
                profession.send(format!("KARTCRAFT\tR\tS\t{}", race.id));
            }
            (None, Some(offer), _) => {
                profession.send(format!("KARTCRAFT\tR\tJ\t{}", offer.id));
            }
            (None, None, Some(course)) => {
                profession.send(format!("KARTCRAFT\tR\tH\t{}", course.0));
            }
            _ => {}
        }
    }

    // The countdown, the start boost and the race.
    let Some(race) = races.race.as_mut() else {
        extras.lap = None;
        extras.place = None;
        audio.music_rate = 1.0;
        text.banner = match (&races.offer, host_here) {
            _ if races.finder.check.is_some() => queue_banner,
            (Some(o), _) => format!("{}'s {} — F5 to join", o.host, o.name),
            (None, Some(c)) if here.len() > 1 => {
                let other = here[(races.pick + 1) % here.len()];
                format!(
                    "{} starts here — F5 to host a race, F7 for {}",
                    c.1, other.1
                )
            }
            (None, Some(c)) => format!("{} starts here — F5 to host a race", c.1),
            _ => queue_banner,
        };
        return;
    };
    let Some(go) = race.go_at else {
        text.banner = format!(
            "Waiting to race: {}{}",
            race.roster.join(", "),
            if race.host {
                " — F5 to start, F6 to leave"
            } else {
                " — F6 to leave"
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
            Some((_, place, ms)) => format!("{} — {}", ordinal(*place), clock(*ms)),
            None => String::new(),
        };
    }
    text.lines = race
        .finished
        .iter()
        .map(|(name, place, ms)| format!("{}. {name}  {}", place, clock(*ms)))
        .collect();
}

/// `WOW_KART_RACE=host:<course>` hosts at that course's start and starts once another racer is
/// in; `WOW_KART_RACE=join` joins the first race offered. A test hook: it presses F5.
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

/// A course's name from its key; `any` reads "any course".
fn course_name(key: &str) -> &str {
    COURSES
        .iter()
        .find(|c| c.0 == key)
        .map_or(if key == "any" { "any course" } else { key }, |c| c.1)
}

/// The queue status line: `Qs\t<waiting>\t<secs>`.
fn parse_status(f: &[String]) -> Option<(u32, u32)> {
    Some((f.get(1)?.parse().ok()?, f.get(2)?.parse().ok()?))
}

/// The Race Finder's banner: the ready check, else the queue, else nothing.
pub(crate) fn finder_banner(finder: &Finder, now: f32) -> String {
    if let Some(c) = &finder.check {
        return if c.accepted {
            "Race ready — waiting for the others...".into()
        } else {
            format!(
                "Race ready: {} ({} racers) — F5 to accept, F6 to decline ({})",
                course_name(&c.course),
                c.racers,
                (c.until - now).max(0.0).ceil()
            )
        };
    }
    let Some(keys) = &finder.queued else {
        return String::new();
    };
    let names: Vec<&str> = keys.split(',').map(course_name).collect();
    match finder.status {
        Some((waiting, secs)) => format!(
            "Race Finder: {} — {}:{:02}, {waiting} waiting",
            names.join(", "),
            secs / 60,
            secs % 60
        ),
        None => format!("Race Finder: {}", names.join(", ")),
    }
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

/// The arrow on beam `.0`, pointing on to the checkpoint after it.
#[derive(Component)]
pub(crate) struct BeamArrow(usize);

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
        Self {
            beam: meshes.add(Cylinder::new(BEAM_RADIUS, BEAM_HEIGHT)),
            arrow: meshes.add(arrow_mesh()),
            bright: materials.add(glow(Color::srgb(1.0, 0.85, 0.2))),
            dim: materials.add(glow(Color::srgb(0.18, 0.12, 0.04))),
            guide: materials.add(StandardMaterial {
                base_color: Color::srgb(1.0, 0.6, 0.08),
                unlit: true,
                cull_mode: None,
                double_sided: true,
                ..default()
            }),
        }
    }
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
    let half = 0.06;
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
    mesh
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

/// Stands a beam at each checkpoint of the race we are in, the next one bright, each with an
/// arrow on to the checkpoint after it.
#[allow(clippy::type_complexity)]
pub(crate) fn beams(
    mut commands: Commands,
    races: Res<Races>,
    collide: WorldCollision,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut beams: Query<
        (
            Entity,
            &Beam,
            &mut Transform,
            &mut MeshMaterial3d<StandardMaterial>,
        ),
        Without<BeamArrow>,
    >,
    mut arrows: Query<
        (
            Entity,
            &BeamArrow,
            &mut Transform,
            &mut MeshMaterial3d<StandardMaterial>,
        ),
        Without<Beam>,
    >,
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
    let look = |i: usize| {
        if i == next {
            looks.bright.clone()
        } else {
            looks.dim.clone()
        }
    };
    let n = race.points.len();
    let mut have = vec![false; n];
    for (_, beam, mut t, mut material) in &mut beams {
        let Some(&p) = race.points.get(beam.0) else {
            continue;
        };
        have[beam.0] = true;
        let ground = ground_at(&collide, p, t.translation.y - BEAM_HEIGHT / 2.0);
        t.translation = ground + Vec3::Y * (BEAM_HEIGHT / 2.0);
        material.0 = look(beam.0);
    }
    let mut have_arrow = vec![false; n];
    for (e, arrow, mut t, mut material) in &mut arrows {
        let (Some(&p), Some(on)) = (race.points.get(arrow.0), race.after(arrow.0)) else {
            commands.entity(e).despawn();
            continue;
        };
        have_arrow[arrow.0] = true;
        let at = ground_at(&collide, p, t.translation.y - BEAM_ARROW_HEIGHT);
        let towards = wow_to_bevy([race.points[on][0], race.points[on][1], 0.0]);
        t.translation = at + Vec3::Y * BEAM_ARROW_HEIGHT;
        t.rotation = pointing(at, Vec3::new(towards.x, at.y, towards.z));
        material.0 = look(arrow.0);
    }
    for (i, p) in race.points.iter().enumerate() {
        let at = wow_to_bevy([p[0], p[1], 0.0]);
        if !have[i] {
            commands.spawn((
                Beam(i),
                Mesh3d(looks.beam.clone()),
                MeshMaterial3d(looks.dim.clone()),
                Transform::from_translation(at),
                Name::new("race checkpoint"),
            ));
        }
        if !have_arrow[i] && race.after(i).is_some() {
            commands.spawn((
                BeamArrow(i),
                Mesh3d(looks.arrow.clone()),
                MeshMaterial3d(looks.dim.clone()),
                Transform::from_translation(at).with_scale(Vec3::splat(BEAM_ARROW_LENGTH)),
                Name::new("race checkpoint arrow"),
            ));
        }
    }
}

/// What the minimap calls the next checkpoint.
fn checkpoint_name(race: &Active, next: usize) -> String {
    let n = race.points.len();
    let last_lap = race.laps == 0 || race.lap + 1 >= race.laps;
    if (race.laps == 0 && next == n - 1) || (race.laps > 0 && next == 0 && last_lap) {
        "Finish".into()
    } else if race.laps > 0 && next == 0 {
        format!("Lap {} of {}", race.lap + 2, race.laps)
    } else {
        format!("Checkpoint {next} of {}", n - 1)
    }
}

/// In a race: the arrow over our kart towards the next checkpoint, and that checkpoint on the
/// minimap, in the slot a guard's directions use (`crate::poi_marker`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn guide(
    mut commands: Commands,
    races: Res<Races>,
    drive: Res<super::KartDrive>,
    collide: WorldCollision,
    mut marker: ResMut<crate::poi_marker::PoiMarker>,
    current_map: Option<Res<benilla_world::world_map::CurrentMap>>,
    real: Res<Time<bevy::time::Real>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut arrows: Query<(Entity, &mut Transform), With<GuideArrow>>,
    mut looks: Local<Option<Looks>>,
    mut shown: Local<Option<String>>,
) {
    let race = races.race.as_ref().filter(|r| r.go_at.is_some() && !r.done);
    let target = race
        .and_then(|r| Some((r, r.next()?)))
        .filter(|_| drive.active);
    let Some((race, next)) = target else {
        for (e, _) in &arrows {
            commands.entity(e).despawn();
        }
        if let Some(name) = shown.take() {
            marker.clear_named(&name);
        }
        return;
    };
    let p = race.points[next];
    let name = checkpoint_name(race, next);
    if let Some(old) = shown.as_ref().filter(|old| **old != name) {
        marker.clear_named(old);
    }
    let map = current_map.as_ref().map_or(0, |m| m.0);
    marker.set_landmark(&name, p, map, real.elapsed_secs_f64());
    *shown = Some(name);

    let over = drive.pos + Vec3::Y * GUIDE_HEIGHT;
    let towards = ground_at(&collide, p, over.y);
    let rotation = pointing(over, Vec3::new(towards.x, over.y, towards.z));
    if let Ok((_, mut t)) = arrows.single_mut() {
        t.translation = over;
        t.rotation = rotation;
    } else {
        let looks = looks.get_or_insert_with(|| Looks::new(&mut meshes, &mut materials));
        commands.spawn((
            GuideArrow,
            Mesh3d(looks.arrow.clone()),
            MeshMaterial3d(looks.guide.clone()),
            Transform::from_translation(over)
                .with_rotation(rotation)
                .with_scale(Vec3::splat(GUIDE_LENGTH)),
            Name::new("race guide arrow"),
        ));
    }
}

/// The race's banner and results on screen.
#[derive(Component)]
pub(crate) struct RaceBanner;

#[derive(Component)]
pub(crate) struct RaceResults;

pub(crate) fn spawn_text(mut commands: Commands) {
    commands.spawn((
        RaceBanner,
        Text::new(String::new()),
        TextFont {
            font_size: 40.0,
            ..default()
        },
        TextColor(Color::srgb(1.0, 0.82, 0.0)),
        TextShadow::default(),
        TextLayout::new_with_justify(Justify::Center),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Percent(26.0),
            width: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            ..default()
        },
        Pickable::IGNORE,
        GlobalZIndex(11),
    ));
    commands.spawn((
        RaceResults,
        Text::new(String::new()),
        TextFont {
            font_size: 22.0,
            ..default()
        },
        TextColor(Color::WHITE),
        TextShadow::default(),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Percent(34.0),
            right: Val::Percent(4.0),
            ..default()
        },
        Pickable::IGNORE,
        GlobalZIndex(11),
    ));
}

pub(crate) fn draw_text(
    race: Res<RaceText>,
    mut banner: Query<&mut Text, (With<RaceBanner>, Without<RaceResults>)>,
    mut results: Query<&mut Text, (With<RaceResults>, Without<RaceBanner>)>,
) {
    if let Ok(mut t) = banner.single_mut() {
        if t.0 != race.banner {
            t.0.clone_from(&race.banner);
        }
    }
    if let Ok(mut t) = results.single_mut() {
        let lines = race.lines.join("\n");
        if t.0 != lines {
            t.0 = lines;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clocks_and_ordinals_read_like_ctr() {
        assert_eq!(clock(83_456), "1:23.45");
        assert_eq!(clock(5_000), "0:05.00");
        assert_eq!(ordinal(1), "1ST");
        assert_eq!(ordinal(3), "3RD");
        assert_eq!(ordinal(7), "7TH");
    }

    #[test]
    fn finder_reads_the_queue_and_the_ready_check() {
        let line = |s: &str| s.split('\t').map(str::to_string).collect::<Vec<_>>();
        assert_eq!(parse_status(&line("Qs\t3\t42")), Some((3, 42)));
        assert_eq!(parse_status(&line("Qs\tx")), None);
        assert_eq!(course_name("elwynn"), "Elwynn Loop");
        assert_eq!(course_name("any"), "any course");

        let mut finder = Finder::default();
        assert_eq!(finder_banner(&finder, 0.0), "");
        finder.queued = Some("sprint,stormwind".into());
        assert_eq!(
            finder_banner(&finder, 0.0),
            "Race Finder: Northshire Sprint, Road to Stormwind"
        );
        finder.status = Some((2, 75));
        assert_eq!(
            finder_banner(&finder, 0.0),
            "Race Finder: Northshire Sprint, Road to Stormwind — 1:15, 2 waiting"
        );
        finder.check = Some(ReadyCheck {
            id: 7,
            course: "elwynn".into(),
            racers: 3,
            until: 20.0,
            accepted: false,
        });
        assert_eq!(
            finder_banner(&finder, 4.5),
            "Race ready: Elwynn Loop (3 racers) — F5 to accept, F6 to decline (16)"
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
