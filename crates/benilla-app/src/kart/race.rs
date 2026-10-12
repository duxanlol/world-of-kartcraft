//! Races between karts, refereed by the Kartcraft server (`Kartcraft.cpp`, "Races"). A course's
//! Race Marshal hosts, joins and starts a race over gossip; a driver at a start is told so here.
//! The server sends the course's checkpoints and a countdown: CTR's start lights, the kart held on
//! the line revving, with CTR's start boost for the revs at GO (`ctr_kart::rev`). Each checkpoint
//! passed is reported, and the server checks it against where we really are; it relays everyone's
//! progress, and the finishes with their times. Our place is worked out every frame, as CTR ranks
//! by each racer's distance along its path: everyone projected onto the leg past their last
//! checkpoint, where their kart is relayed. Checkpoints stand in the world as beams of light, the
//! next one bright.
//!
//! CTR's race HUD comes with it (`kartcraft::hud`): the race clock with our lap times, which we
//! keep ourselves as we cross the start; "FINAL LAP" flying by as the last lap begins, the music
//! 20 BPM up on its own tempo soon after; and "WRONG WAY" flashing once we have driven back along
//! the course far enough, facing back down it. The server keeps each character's best per course
//! and the course record: it sends both at the start (`Rec`), marks a finish that beats one
//! (`F`'s sixth field), and the results board ends with them.
//!
//! A time trial is the Race Marshal's race of one (`TT` after its roster): no crates or items
//! (no weapons are bound to it), our ghost to race (`super::ghost`, only there), no place on the
//! HUD, and a board that sets our time against our best before the run and the time trial record,
//! which the server keeps apart from the races'.
//!
//! CTR's ground types are the race's (`GT`, the host's choice; off in a Race Finder race and a
//! time trial): from the countdown to our finish it rules over our own `KartTerrain`
//! ([`Races::ground_types`], `super::ground`), so every racer drives the same course.
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
use kartcraft::hud::RaceClock;

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
    /// Whether it is a party course: in a dungeon, not the open world (1.12's two continents, 0
    /// and 1). Each party races it inside its own copy, hosted at the marshal there; the Race
    /// Finder never offers it (`Kartcraft.cpp` `PartyCourse`).
    pub(crate) fn party(&self) -> bool {
        self.map > 1
    }

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
        if self.party() {
            text += " A party race: your party's leader hosts it at the Race Marshal inside.";
        }
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

/// The Race Finder's zones, in the order the file first names them, but a party course's; the
/// server keeps the same list, and a queue line asks for a zone by its place in it (`z3`).
pub(crate) static ZONES: std::sync::LazyLock<Vec<String>> = std::sync::LazyLock::new(|| {
    let mut zones: Vec<String> = Vec::new();
    for c in COURSES.iter() {
        if !c.zone.is_empty() && !c.party() && !zones.contains(&c.zone) {
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
/// out on the course after we finish before closing anyway: the server closes the race a minute
/// after its first finish, those still out "DNF" (`Kartcraft.cpp` `DNF_WAIT`), so ours is only a
/// fallback for a close that never comes, a little longer.
const RESULTS_SECS: f32 = 10.0;
const FINISH_WAIT_SECS: f32 = 75.0;
/// How long our progress may run ahead of the server's for us (s) before the checkpoints it lacks
/// are reported again, and again as often while it still lacks them.
const RESYNC_SECS: f32 = 1.5;
/// How long a race just entered waits for the kart the server is summoning (s).
const KART_GRACE_SECS: f32 = 10.0;
/// A checkpoint as drawn: its gate, a sheet of light as wide as the gate it is passed through
/// and `KartGateHeight` tall, between two beams `BEAM_HEIGHT` tall and `BEAM_RADIUS` round at
/// its ends (yards).
const BEAM_HEIGHT: f32 = 40.0;
const BEAM_RADIUS: f32 = 0.6;
/// CTR runs its race at 30 frames a second (`CTR_FRAMES_PER_SECOND`), and "FINAL LAP" flies by
/// for 90 of them (`PlayLevel.c` `PLAYLEVEL_FINAL_LAP_TEXT_FRAMES`, three seconds).
const CTR_FPS: f32 = 30.0;
const FINAL_LAP_FRAMES: u32 = 90;
/// The final lap's music: 20 BPM on the song's own tempo (`HOWL_AudioState.c:338`), set once the
/// final-lap XA sting has played 0xE1 of its 256-sample blocks at 44.1 kHz
/// (`HOWL_AudioState.c:334`; the block size from CTR-native's `NativeAudio_GetXACurrOffset`).
const FINAL_LAP_BPM: f64 = 20.0;
const FINAL_LAP_STING_SECS: f32 = kartcraft::sound::FINAL_LAP_TEMPO_SECS;
/// "WRONG WAY": CTR adds what a racer's distance to the finish grew by each frame and clamps it
/// to 0..1000 lu (`PlayLevel.c:80-95`), and shows the text from 0x1F5 lu while the racer faces
/// within 45 degrees of straight back down the path (`VehLap.c:114-131`, the dot product against
/// 0x5A801 of 0x80000; `UI_RenderFrame.c:130-160`).
const WRONG_WAY_MAX_LU: f32 = 1000.0;
const WRONG_WAY_SHOWN_LU: f32 = 501.0;
const WRONG_WAY_FACING: f32 = 0x5a801 as f32 / 0x80000 as f32;

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
    /// Our checkpoints passed as the server last relayed them, counted as [`Active::passed`]
    /// counts ours, and when the ones it lacks are next reported again.
    server_passes: u32,
    resync_at: Option<f32>,
    /// Finishes: (name, place, ms); place 0 is out of time (the server's `DNF`).
    finished: Vec<(String, u8, u32)>,
    /// Our finish time (ms) as the server timed it.
    our_ms: Option<u32>,
    /// Finishers who set a new best with it: true a course record, false a personal best.
    bests: HashMap<String, bool>,
    /// The course record (ms, holder) and our own best (ms), from the server's `Rec`; 0 none.
    record: (u32, String),
    best: u32,
    /// The record and our best as the race began (the first `Rec`), for a time trial's finish to
    /// be measured against once a new best has replaced them.
    before: Option<(u32, u32)>,
    /// A time trial (`TT`): us alone, no crates or items, and our ghost to race.
    time_trial: bool,
    /// The race's ground types (`GT`), the host's choice: every racer drives off-road as CTR's
    /// grounds hold it, or on asphalt everywhere, whatever their own `KartTerrain`.
    ground_types: bool,
    done: bool,
    /// Last frame's start-light timer, for the lights' sounds.
    lights: Option<i32>,
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
    /// Our laps' times (s), and when the lap under way began (local s): GO, then each crossing
    /// of the start (CTR's `lapTime`, `PlayLevel.c:145`).
    lap_times: Vec<f32>,
    lap_began: f32,
    /// When we crossed the finish line (local s): the race clock stops there.
    crossed_finish: Option<f32>,
    /// When our final lap began (local s): its fly-by and its music.
    final_lap_at: Option<f32>,
    /// How far along the course we were last frame (yards), and how far we have driven back
    /// along it since (lu, CTR's `distanceDrivenBackwards`).
    progress_at: Option<f32>,
    backwards: f32,
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
        self.server_passes = 0;
        self.resync_at = None;
        self.finished.clear();
        self.our_ms = None;
        self.lights = None;
        self.grace_until = self.grace_until.max(now + KART_GRACE_SECS);
        self.lap_times.clear();
        self.lap_began = now + ms / 1000.0;
        self.crossed_finish = None;
        self.final_lap_at = None;
        self.progress_at = None;
        self.backwards = 0.0;
    }

    /// Checkpoint `next` passed at `now`: on to it; crossing the start finishes a lap, whose time
    /// is kept, and may begin the final one (`PlayLevel.c:145-167`) or end the race.
    fn pass(&mut self, next: usize, now: f32) {
        self.checkpoint = next;
        if self.laps > 0 && next == 0 {
            self.lap += 1;
            self.lap_times.push(now - self.lap_began);
            self.lap_began = now;
            // CTR's `lapIndex == numLaps - 1`: never in a one-lap race, whose first lap is its last.
            if self.laps > 1 && self.lap + 1 == self.laps {
                self.final_lap_at = Some(now);
            }
            if self.lap >= self.laps {
                self.crossed_finish = Some(now);
            }
        } else if self.laps == 0 && next + 1 == self.points.len() {
            self.crossed_finish = Some(now);
        }
    }

    /// The race clock at `now` (`UI_DrawRaceClock`): the time since GO, stopped at the finish,
    /// with a row for each lap so far, the one under way running. `finish_ms` is our time as the
    /// server has it: once it comes it is the time shown, the server's clock being the race's.
    fn clock(&self, now: f32, finish_ms: Option<u32>) -> Option<RaceClock> {
        let go = self.go_at?;
        let end = self.crossed_finish.unwrap_or(now).max(go);
        let total = finish_ms.map_or(end - go, |ms| ms as f32 / 1000.0);
        let mut laps = Vec::new();
        if self.laps > 0 {
            laps = self.shown_laps(finish_ms);
            if laps.len() < usize::from(self.laps) && finish_ms.is_none() {
                laps.push((end - self.lap_began).max(0.0));
            }
        }
        Some(RaceClock {
            total,
            time_trial: self.time_trial,
            laps,
        })
    }

    /// Our laps' times as we timed them at the start line, but with the server's time for the
    /// whole race (`finish_ms`) the last made up to it, so the laps add up to the time shown.
    fn shown_laps(&self, finish_ms: Option<u32>) -> Vec<f32> {
        let mut laps: Vec<f32> = self
            .lap_times
            .iter()
            .take(usize::from(self.laps))
            .copied()
            .collect();
        if let (Some(ms), true) = (finish_ms, laps.len() == usize::from(self.laps)) {
            let before: f32 = laps.iter().rev().skip(1).sum();
            let last = ms as f32 / 1000.0 - before;
            if let (Some(l), true) = (laps.last_mut(), last > 0.0) {
                *l = last;
            }
        }
        laps
    }

    /// Our checkpoints passed in order, as the server counts a racer's: laps times checkpoints,
    /// plus the last one passed.
    fn passed(&self) -> u32 {
        u32::from(self.lap) * self.points.len() as u32 + self.checkpoint as u32
    }

    /// The checkpoints we have passed that the server has not taken, in order: reports it lost,
    /// or refused before it could see us there. Due `RESYNC_SECS` after the gap opens and as
    /// often again while it stays open; empty when nothing is due at `now`.
    fn resend(&mut self, now: f32) -> Vec<usize> {
        let n = self.points.len();
        if n < 2 || self.done || self.passed() <= self.server_passes {
            self.resync_at = None;
            return Vec::new();
        }
        let due = *self.resync_at.get_or_insert(now + RESYNC_SECS);
        if now < due {
            return Vec::new();
        }
        self.resync_at = Some(now + RESYNC_SECS);
        (self.server_passes + 1..=self.passed())
            .take(n)
            .map(|k| k as usize % n)
            .collect()
    }

    /// Frames into "FINAL LAP"'s fly-by at `now`, while it runs.
    fn final_lap_frame(&self, now: f32) -> Option<u32> {
        let since = now - self.final_lap_at?;
        let frame = (since.max(0.0) * CTR_FPS) as u32;
        (frame < FINAL_LAP_FRAMES).then_some(frame)
    }

    /// Beats a minute on the song at `now`: CTR's 20 once the final lap's sting has played its
    /// first 1.3 s, until the race is over for us.
    fn music_bpm_delta(&self, now: f32) -> f64 {
        match self.final_lap_at {
            Some(t) if !self.done && now - t >= FINAL_LAP_STING_SECS => FINAL_LAP_BPM,
            _ => 0.0,
        }
    }

    /// Seconds into our final lap at `now`, until the race is over for us: its sting plays as it
    /// begins, the song lowered under it (`AUDIO_FINAL_LAP`, `HOWL_AudioState.c:79-87`).
    fn final_lap_secs(&self, now: f32) -> Option<f32> {
        self.final_lap_at
            .filter(|_| !self.done)
            .map(|t| (now - t).max(0.0))
    }

    /// How far along the course (yards from the start, a circuit's laps counted in) a racer is
    /// with `lap` laps done, checkpoint `cp` the last passed, standing at `at` (WoW x, y): CTR's
    /// distance along its path (`VehLap.c:14-131`), projected onto the leg from `cp` to the next
    /// checkpoint. Behind `cp` it counts back down the leg before it, so turning round reads as
    /// going backwards; beyond the next checkpoint it holds there until that gate is passed.
    /// Without a position, the racer is at `cp`.
    fn progress(&self, lap: u8, cp: usize, at: Option<[f32; 2]>) -> f32 {
        let n = self.points.len();
        if n < 2 {
            return 0.0;
        }
        let cp = cp.min(n - 1);
        let leg = |i: usize, j: usize| {
            let (a, b) = (self.points[i], self.points[j]);
            (b[0] - a[0]).hypot(b[1] - a[1])
        };
        let mut to_cp = 0.0;
        let mut length = 0.0;
        for i in 1..n {
            length += leg(i - 1, i);
            if i == cp {
                to_cp = length;
            }
        }
        if self.laps > 0 {
            length += leg(n - 1, 0);
        }
        let base = f32::from(lap) * length + to_cp;
        let (Some(at), Some(next)) = (at, self.after(cp)) else {
            return base;
        };
        let (a, b) = (self.points[cp], self.points[next]);
        let len = leg(cp, next).max(1e-3);
        let along = ((at[0] - a[0]) * (b[0] - a[0]) + (at[1] - a[1]) * (b[1] - a[1])) / len;
        let back = if self.laps > 0 || cp > 0 {
            leg((cp + n - 1) % n, cp)
        } else {
            0.0
        };
        base + along.clamp(-back, len)
    }

    /// Whether `facing` (WoW x, y, unit) looks back down the leg past checkpoint `cp`: within 45
    /// degrees of straight back (`VehLap.c:127`).
    fn facing_back(&self, cp: usize, facing: [f32; 2]) -> bool {
        let Some(next) = self.after(cp).filter(|_| cp < self.points.len()) else {
            return false;
        };
        let (a, b) = (self.points[cp], self.points[next]);
        let len = (b[0] - a[0]).hypot(b[1] - a[1]).max(1e-3);
        let back = [(a[0] - b[0]) / len, (a[1] - b[1]) / len];
        facing[0] * back[0] + facing[1] * back[1] >= WRONG_WAY_FACING
    }

    /// One frame of our progress (yards): what it fell by since the last, in lu at
    /// `lu_per_yard`, added to how far we have driven backwards, clamped as CTR clamps it.
    fn drove(&mut self, progress: f32, lu_per_yard: f32) {
        if let Some(before) = self.progress_at.replace(progress) {
            self.backwards =
                (self.backwards + (before - progress) * lu_per_yard).clamp(0.0, WRONG_WAY_MAX_LU);
        }
    }

    /// Whether "WRONG WAY" is up: driven back far enough, facing back down the course.
    fn wrong_way(&self, facing_back: bool) -> bool {
        facing_back && self.backwards >= WRONG_WAY_SHOWN_LU
    }

    /// The standings, leader first: the finished in their order, then the rest by how far along
    /// the course each is, the first to their last checkpoint ahead on a tie. Our kart is at
    /// `me_at`; another racer at their relayed kart in `near`, else at their last checkpoint.
    fn standings(
        &self,
        me: &str,
        me_at: Option<[f32; 2]>,
        near: &[(String, [f32; 2])],
    ) -> Vec<String> {
        let go = self.go_at.unwrap_or(0.0);
        let mut order: Vec<String> = self.finished.iter().map(|f| f.0.clone()).collect();
        let mut running: Vec<(&String, f32, f32)> = self
            .roster
            .iter()
            .filter(|r| !order.contains(r))
            .map(|r| {
                let (lap, cp, when) = self.progress.get(r).copied().unwrap_or((0, 0, go));
                let (lap, cp, at) = if r == me {
                    (self.lap, self.checkpoint, me_at)
                } else {
                    let at = near.iter().find(|(n, _)| n == r).map(|(_, at)| *at);
                    (lap, usize::from(cp), at)
                };
                (r, self.progress(lap, cp, at), when)
            })
            .collect();
        running.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.2.total_cmp(&b.2)));
        order.extend(running.into_iter().map(|(r, ..)| r.clone()));
        order
    }

    /// CTR's start lights `left` seconds before GO: their timer, and the sound of a light come
    /// on since the last frame, each once (DotLights.c:91-136). A countdown that arrives late
    /// still sounds the light it lands on.
    fn start_lights(&mut self, left: f32, cues: &mut Vec<kartcraft::sound::Cue>) -> i32 {
        use kartcraft::hud::{start_light_sound, StartSound, LIGHTS_TIMER_START};
        let timer = kartcraft::hud::lights_timer(left);
        let prev = self.lights.replace(timer).unwrap_or(LIGHTS_TIMER_START);
        match start_light_sound(prev, timer) {
            Some(StartSound::Red) => cues.push(kartcraft::sound::Cue::Countdown(false)),
            Some(StartSound::Green) => cues.push(kartcraft::sound::Cue::Countdown(true)),
            None => {}
        }
        timer
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

    /// Checkpoint `i`'s gate half-width (yards).
    fn gate_half_width(&self, i: usize) -> f32 {
        self.gates.get(i).copied().unwrap_or(GATE_HALF_WIDTH)
    }

    /// The two ends of checkpoint `i`'s gate (WoW x, y): its half-width either side of its
    /// centre, square to the course.
    fn gate_ends(&self, i: usize) -> [[f32; 2]; 2] {
        let (p, h, w) = (self.points[i], self.heading(i), self.gate_half_width(i));
        let side = [h[1] * w, -h[0] * w];
        [
            [p[0] + side[0], p[1] + side[1]],
            [p[0] - side[0], p[1] - side[1]],
        ]
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
            && across(to).min(across(from)) <= self.gate_half_width(i)
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
    /// A time trial: the only race with a ghost.
    pub(crate) time_trial: bool,
}

/// What a server line did beyond the race state, for [`update`] to show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Heard {
    Nothing,
    /// A Race Finder ready check: its cue.
    ReadyCheck,
    /// Out of every race and the queue: whatever the race showed goes.
    LeftAll,
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
        self.race
            .as_ref()
            .filter(|r| !r.done && !r.time_trial)
            .map(|r| r.id)
    }

    /// Whether the race we are in is a time trial: no weapons in it at all.
    pub(crate) fn time_trial(&self) -> bool {
        self.race.as_ref().is_some_and(|r| r.time_trial)
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
            finish_ms: r.our_ms.filter(|_| r.done),
            done: r.done,
            time_trial: r.time_trial,
        })
    }

    /// Whether the kart drives on CTR's grounds by the race's rule rather than our own
    /// `KartTerrain`: `Some` from the countdown until we finish or leave, the race's `GT` (off
    /// until it comes, and always off in a time trial, whose board is one board); `None` outside
    /// a race, where the setting applies.
    pub(crate) fn ground_types(&self) -> Option<bool> {
        let r = self.race.as_ref()?;
        (r.go_at.is_some() && !r.done).then_some(r.ground_types && !r.time_trial)
    }

    /// Whether the kart must hold on the line: a countdown is running.
    pub(crate) fn holding(&self, now: f32) -> bool {
        self.race
            .as_ref()
            .is_some_and(|r| r.go_at.is_some_and(|go| now < go))
    }
}

impl Races {
    /// One server line (`R\t` taken off, split at the tabs): the race state it changes.
    fn hear(&mut self, f: &[String], now: f32) -> Heard {
        let me = self.me.clone();
        let id = f.get(1).and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
        match f.first().map(String::as_str) {
            Some("Pop") => {
                self.check_pending = true;
                return Heard::ReadyCheck;
            }
            Some("Pop-") => self.check_pending = false,
            // Out of everything (`/leaverace`): the server has let us go, so whatever we
            // thought we were in goes too.
            Some("X") => {
                info!("kart race: left every race and the queue");
                self.race = None;
                self.offer = None;
                self.check_pending = false;
                return Heard::LeftAll;
            }
            Some("O") if f.len() >= 5 => {
                if self.race.is_none() {
                    self.offer = Some(Offer {
                        id,
                        name: f[3].clone(),
                        host: f[4].clone(),
                    });
                }
            }
            Some("L") if f.len() >= 3 => {
                let roster: Vec<String> = f[2].split(',').map(str::to_string).collect();
                if roster.contains(&me) {
                    self.check_pending = false;
                    // Another race's roster with us in it: the server has moved us (a time trial
                    // takes us out of every other race), so the old one goes.
                    if self.race.as_ref().is_some_and(|r| r.id != id) {
                        self.race = None;
                    }
                    // A Race Finder pop or a Race Marshal on foot summons the kart as it puts us
                    // in: its aura lands after the roster, when the cast time has run.
                    let race = self.race.get_or_insert_with(|| Active {
                        id,
                        grace_until: now + KART_GRACE_SECS,
                        ..Default::default()
                    });
                    race.host = roster.first() == Some(&me);
                    race.roster = roster;
                    self.offer = None;
                } else if self.race.as_ref().is_some_and(|r| r.id == id) {
                    self.race = None;
                }
            }
            // The race is a time trial: no weapons, and our ghost.
            Some("TT") => {
                if let Some(race) = self.race.as_mut().filter(|r| r.id == id) {
                    race.time_trial = true;
                    info!("kart race: {id} is a time trial");
                }
            }
            // The race's ground types, "1" on, sent with its course at the start.
            Some("GT") if f.len() >= 3 => {
                if let Some(race) = self.race.as_mut().filter(|r| r.id == id) {
                    race.ground_types = f[2] == "1";
                    info!("kart race: {id} ground types {}", f[2]);
                }
            }
            Some("C") if f.len() >= 5 => {
                if let Some(race) = self.race.as_mut().filter(|r| r.id == id) {
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
                if let Some(race) = self.race.as_mut().filter(|r| r.id == id) {
                    race.count_down(now, f[2].parse().unwrap_or(4000.0));
                    info!("kart race: countdown, {} racers", race.roster.len());
                }
            }
            Some("P") if f.len() >= 5 => {
                if let Some(race) = self.race.as_mut().filter(|r| r.id == id) {
                    let lap: u8 = f[3].parse().unwrap_or(0);
                    let cp: u8 = f[4].parse().unwrap_or(0);
                    race.progress.insert(f[2].clone(), (lap, cp, now));
                    if f[2] == me {
                        race.server_passes =
                            u32::from(lap) * race.points.len() as u32 + u32::from(cp);
                    }
                }
            }
            // Out of time: the race closed DNF_WAIT after its first finish with them still out.
            Some("DNF") if f.len() >= 3 => {
                if let Some(race) = self.race.as_mut().filter(|r| r.id == id) {
                    info!("kart race: {} did not finish", f[2]);
                    if !race.finished.iter().any(|x| x.0 == f[2]) {
                        race.finished.push((f[2].clone(), 0, 0));
                    }
                    if f[2] == me {
                        race.done = true;
                        // The clock stops where the race closed on us.
                        race.crossed_finish.get_or_insert(now);
                    }
                }
            }
            Some("F") if f.len() >= 5 => {
                if let Some(race) = self.race.as_mut().filter(|r| r.id == id) {
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
                        race.our_ms = Some(ms);
                    }
                }
            }
            // The course record and holder, and our best: at the start, and after a new best.
            Some("Rec") if f.len() >= 6 => {
                if let Some(race) = self.race.as_mut().filter(|r| r.id == id) {
                    race.record = (f[3].parse().unwrap_or(0), f[4].clone());
                    race.best = f[5].parse().unwrap_or(0);
                    race.before.get_or_insert((race.record.0, race.best));
                    info!(
                        "kart race: course record {} by {}, our best {}",
                        race.record.0, race.record.1, race.best
                    );
                }
            }
            _ => {}
        }
        Heard::Nothing
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

/// The banner while the race counts down, `left` seconds to GO: the seconds to the start, then
/// CTR's 3, 2, 1 in the kart; on the way to the grid (out of the kart, a teleport's load), the
/// seconds still. A race with ground types on says so before the 3, 2, 1.
fn countdown_banner(left: f32, in_kart: bool, ground_types: bool) -> String {
    let secs = left.ceil().max(1.0) as u32;
    let note = if ground_types {
        " — ground types on"
    } else {
        ""
    };
    match (in_kart, secs) {
        (true, 1..=3) => secs.to_string(),
        (true, _) => format!("Race starts in {secs}{note}"),
        (false, _) => format!("Getting to the start... race starts in {secs}{note}"),
    }
}

/// The other racers whose karts are relayed to us, by name, where they are (WoW x, y).
pub(crate) fn relayed(
    remotes: &super::remote::RemoteKarts,
    names: Option<&crate::names::NameCache>,
) -> Vec<(String, [f32; 2])> {
    remotes
        .karts
        .values()
        .filter_map(|k| {
            let name = names?.peek(k.guid)?;
            let w = bevy_to_wow(k.position);
            Some((name.to_string(), [w[0], w[1]]))
        })
        .collect()
}

/// The race's things on CTR's HUD, and the music's final-lap tempo, taken down.
fn clear_hud(extras: &mut kartcraft::hud::HudExtras, audio: &mut kartcraft::sound::KartAudio) {
    extras.lap = None;
    extras.place = None;
    extras.clock = None;
    extras.final_lap_frame = None;
    extras.wrong_way = false;
    audio.music_bpm_delta = 0.0;
    audio.final_lap = None;
}

/// Whether the flashing "WRONG WAY" is lit `secs` into the race: while bit 0x80 of the race's
/// time in CTR's 1/960 s is set (`UI_RenderFrame.c:155`), on and off every 0.13 s.
fn wrong_way_lit(secs: f32) -> bool {
    (secs.max(0.0) * 960.0) as u32 & 0x80 != 0
}

/// The way our kart faces, WoW x, y: Bevy's yaw turns its forward from −Z.
fn facing_of(yaw: f32) -> [f32; 2] {
    let w = bevy_to_wow(Vec3::new(-yaw.sin(), 0.0, -yaw.cos()));
    [w[0], w[1]]
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
    remotes: Res<super::remote::RemoteKarts>,
    names: Option<Res<crate::names::NameCache>>,
    current_map: Option<Res<benilla_world::world_map::CurrentMap>>,
    mut auto: Local<AutoRace>,
) {
    let now = time.elapsed_secs();
    let races = &mut *races;
    let me = races.me.clone();

    // The server's lines.
    for f in std::mem::take(&mut races.inbox) {
        match races.hear(&f, now) {
            Heard::Nothing => {}
            Heard::ReadyCheck => audio.cues.push(kartcraft::sound::Cue::Ready),
            Heard::LeftAll => {
                *text = RaceText::default();
                clear_hud(&mut extras, &mut audio);
            }
        }
    }

    // `WOW_KART_RACE` joins, hosts or starts for unattended runs, as the Race Marshal does.
    let press = auto.press(now, races);
    *text = RaceText::default();
    extras.start_timer = None;

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
        clear_hud(&mut extras, &mut audio);
        if let Some(race) = &races.race {
            text.banner = match race.go_at {
                Some(go) if now < go => {
                    countdown_banner(go - now, false, race.ground_types && !race.time_trial)
                }
                _ => "Getting to the start...".into(),
            };
        }
        return;
    }
    let wow = bevy_to_wow(drive.pos);
    let at = [wow[0], wow[1]];
    let near = |p: [f32; 2], r: f32| (p[0] - at[0]).hypot(p[1] - at[1]) <= r;

    // The courses starting here, on this map: Stratholme's streets lie over Eastern Plaguelands'
    // coordinates.
    let map = current_map.as_ref().map_or(0, |m| m.0);
    let here: Vec<&Course> = COURSES
        .iter()
        .filter(|c| c.map == map && near(c.start(), START_RADIUS))
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
        clear_hud(&mut extras, &mut audio);
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
    // CTR's start lights: their last 4 s to GO on the HUD, green with GO, and their sounds.
    let timer = race.start_lights(go - now, &mut audio.cues);
    let lights = kartcraft::hud::has_start_lights();
    if timer >= kartcraft::hud::LIGHTS_TIMER_END {
        extras.start_timer = Some(timer);
    }
    if now < go {
        // CTR's clock stands at 0:00:00 through the countdown.
        extras.clock = race.clock(now, None);
        // The seconds to the start until the lights take over (the text's 3, 2, 1 without them).
        text.banner = if lights && timer <= kartcraft::hud::LIGHTS_TIMER_START {
            String::new()
        } else {
            countdown_banner(go - now, true, race.ground_types && !race.time_trial)
        };
        return;
    }
    if now - go < 1.0 && !lights {
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
            race.pass(next, now);
            race.progress
                .insert(me.clone(), (race.lap, next as u8, now));
        }
    }
    // Checkpoints the server has not taken from us: reported again, in order, until it has.
    let missing = race.resend(now);
    if !missing.is_empty() {
        warn!(
            "kart race: the server has us {} checkpoints behind — reporting {missing:?} again",
            missing.len()
        );
        for i in missing {
            profession.send(format!("KARTCRAFT\tR\tK\t{}\t{i}", race.id));
        }
    }

    // Standings: the finished in order, then the rest by how far along the course they are.
    let near = relayed(&remotes, names.as_deref());
    let order = race.standings(&me, Some(at), &near);
    // A time trial has no place to show: it is us alone against the clock; nor has a racer the
    // race closed on (the server's DNF, place 0).
    let out_of_time = race.finished.iter().any(|f| f.0 == me && f.1 == 0);
    extras.place = order
        .iter()
        .position(|r| *r == me)
        .map(|i| i as u8 + 1)
        .filter(|_| !race.time_trial && !out_of_time);
    race.order = order;
    extras.lap = (race.laps > 0).then(|| ((race.lap + 1).min(race.laps), race.laps));
    extras.clock = race.clock(now, race.our_ms.filter(|_| race.done));
    extras.final_lap_frame = race.final_lap_frame(now);
    audio.music_bpm_delta = race.music_bpm_delta(now);
    audio.final_lap = race.final_lap_secs(now);
    // Wrong way: our progress along the course this frame, against how we face.
    let progress = race.progress(race.lap, race.checkpoint, Some(at));
    race.drove(progress, kartcraft::lu_per_yard());
    let facing_back = race.facing_back(race.checkpoint, facing_of(drive.yaw));
    extras.wrong_way = !race.done
        && race.crossed_finish.is_none()
        && race.wrong_way(facing_back)
        && wrong_way_lit(now - go);
    if race.done {
        text.banner = match race.finished.iter().find(|f| f.0 == me) {
            Some((_, 0, _)) => "DNF — out of time".into(),
            Some((_, place, ms)) => {
                let news = match race.bests.get(&me) {
                    Some(true) => " — New course record!",
                    Some(false) => " — New personal best!",
                    None => "",
                };
                if race.time_trial {
                    format!("Time trial — {}{news}", clock(*ms))
                } else {
                    format!("{} — {}{news}", ordinal(*place), clock(*ms))
                }
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
            clear_hud(&mut extras, &mut audio);
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
/// record and our own best, and the race's ground types when on. Empty until someone finishes.
fn board(race: &Active) -> Vec<String> {
    if race.finished.is_empty() {
        return Vec::new();
    }
    if race.time_trial {
        return trial_board(race);
    }
    let mut lines: Vec<String> = race
        .finished
        .iter()
        .map(|(name, place, ms)| {
            if *place == 0 {
                return format!("DNF  {name}");
            }
            let mark = match race.bests.get(name) {
                Some(true) => "  (record)",
                Some(false) => "  (best)",
                None => "",
            };
            format!("{place}. {name}  {}{mark}", clock(*ms))
        })
        .collect();
    lines.extend(lap_lines(race));
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
    if race.ground_types {
        lines.push("Ground types: on".into());
    }
    lines
}

/// A time trial's board: our time, against our best before this run, then the time trial record.
fn trial_board(race: &Active) -> Vec<String> {
    let Some((name, _, ms)) = race.finished.first() else {
        return Vec::new();
    };
    let mark = match race.bests.get(name) {
        Some(true) => "  (record)",
        Some(false) => "  (best)",
        None => "",
    };
    let mut lines = vec![format!("Your time: {}{mark}", clock(*ms))];
    match race.before.map_or(0, |b| b.1) {
        0 => lines.push("Your first time trial here".into()),
        best if *ms < best => lines.push(format!(
            "{} faster than your best of {}",
            clock(best - ms),
            clock(best)
        )),
        best => lines.push(format!(
            "{} behind your best of {}",
            clock(ms - best),
            clock(best)
        )),
    }
    lines.extend(lap_lines(race));
    if race.record.0 != 0 {
        lines.push(format!(
            "Time trial record: {} by {}",
            clock(race.record.0),
            race.record.1
        ));
    }
    lines
}

/// Our laps' times, timed by us as we crossed the start (the server times only the finish), the
/// fastest of several marked.
fn lap_lines(race: &Active) -> Vec<String> {
    if race.laps == 0 {
        return Vec::new();
    }
    let fastest = race
        .lap_times
        .iter()
        .enumerate()
        .min_by(|a, b| a.1.total_cmp(b.1))
        .map(|(i, _)| i)
        .filter(|_| race.lap_times.len() > 1);
    let laps = race.shown_laps(race.our_ms);
    laps.iter()
        .enumerate()
        .map(|(i, secs)| {
            let mark = if fastest == Some(i) {
                "  (fastest)"
            } else {
                ""
            };
            format!(
                "Lap {}: {}{mark}",
                i + 1,
                clock((secs * 1000.0).round() as u32)
            )
        })
        .collect()
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

/// A piece of a checkpoint as drawn: checkpoint `.0`'s gate sheet, or the beam at one of its ends.
#[derive(Component)]
pub(crate) struct Beam(usize, GatePart);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum GatePart {
    Sheet,
    End(usize),
}

const GATE_PARTS: [GatePart; 3] = [GatePart::Sheet, GatePart::End(0), GatePart::End(1)];

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
    /// A unit square, facing +Z, scaled to each gate's width and `KartGateHeight`.
    sheet: Handle<Mesh>,
    arrow: Handle<Mesh>,
    bright: Handle<StandardMaterial>,
    dim: Handle<StandardMaterial>,
    sheet_bright: Handle<StandardMaterial>,
    sheet_dim: Handle<StandardMaterial>,
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
        let see_through = |c: Color| StandardMaterial {
            alpha_mode: AlphaMode::Blend,
            ..solid(c)
        };
        let (face, outline) = checkpoint_arrow_meshes();
        Self {
            beam: meshes.add(Cylinder::new(BEAM_RADIUS, BEAM_HEIGHT)),
            sheet: meshes.add(Rectangle::new(1.0, 1.0)),
            arrow: meshes.add(arrow_mesh()),
            // Added light, so the colour is how much shows through: the next gate faint, the
            // others fainter still, and the sheets fainter than their beams.
            bright: materials.add(glow(Color::srgb(0.55, 0.45, 0.1))),
            dim: materials.add(glow(Color::srgb(0.09, 0.06, 0.02))),
            sheet_bright: materials.add(glow(Color::srgb(0.16, 0.13, 0.03))),
            sheet_dim: materials.add(glow(Color::srgb(0.03, 0.02, 0.007))),
            guide: materials.add(solid(Color::srgb(1.0, 0.6, 0.08))),
            guide_outline: meshes.add(guide_outline_mesh()),
            cp_face: meshes.add(face),
            cp_outline: meshes.add(outline),
            // See-through, as the gates are: the next arrow half there, the one after less.
            cp_next: materials.add(see_through(Color::srgba(1.0, 0.85, 0.1, 0.5))),
            cp_after: materials.add(see_through(Color::srgba(0.6, 0.5, 0.16, 0.3))),
            cp_edge: materials.add(see_through(Color::srgba(0.03, 0.02, 0.01, 0.45))),
        }
    }
}

/// The checkpoint arrow's shape, one unit long up its +Y with its centre at the origin, flat in
/// the XY plane facing +Z: a shaft and a head, each convex, meeting edge to edge so the
/// see-through face is drawn once everywhere; and the two as one outline, anticlockwise.
const CP_ARROW_SHAFT: [[f32; 2]; 4] = [[-0.16, -0.5], [0.16, -0.5], [0.16, 0.0], [-0.16, 0.0]];
const CP_ARROW_HEAD: [[f32; 2]; 3] = [[-0.44, 0.0], [0.44, 0.0], [0.0, 0.5]];
const CP_ARROW_SHAPE: [[f32; 2]; 7] = [
    [-0.16, -0.5],
    [0.16, -0.5],
    [0.16, 0.0],
    [0.44, 0.0],
    [0.0, 0.5],
    [-0.44, 0.0],
    [-0.16, 0.0],
];
/// How far the outline stands out round it (units), and how far behind the face it lies.
const CP_ARROW_EDGE: f32 = 0.07;
const CP_ARROW_EDGE_DEPTH: f32 = 0.03;

/// A simple polygon (anticlockwise) grown by `by` on every side, its corners mitred.
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

/// The checkpoint arrow's face and its outline: a band the edge wide round the face's shape,
/// just behind it, and not under it, so the see-through face shows the world through it.
fn checkpoint_arrow_meshes() -> (Mesh, Mesh) {
    let face = flat_mesh(&[CP_ARROW_SHAFT.to_vec(), CP_ARROW_HEAD.to_vec()], 0.0);
    let outline = band_mesh(
        &CP_ARROW_SHAPE,
        &grown(&CP_ARROW_SHAPE, CP_ARROW_EDGE),
        -CP_ARROW_EDGE_DEPTH,
    );
    (face, outline)
}

/// The band between a polygon and the same polygon grown, a quad per side, in the plane z
/// facing +Z.
fn band_mesh(inner: &[[f32; 2]], outer: &[[f32; 2]], z: f32) -> Mesh {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology};
    let n = inner.len() as u32;
    let positions: Vec<[f32; 3]> = inner.iter().chain(outer).map(|p| [p[0], p[1], z]).collect();
    let indices: Vec<u32> = (0..n)
        .flat_map(|i| {
            let j = (i + 1) % n;
            [i, j, n + j, i, n + j, n + i]
        })
        .collect();
    let normals = vec![[0.0, 0.0, 1.0]; positions.len()];
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_indices(Indices::U32(indices))
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

/// Where a piece of checkpoint `i` stands: the sheet on the ground at the gate's centre, as wide
/// as the gate, `sheet_height` tall and square to the course; a beam on the ground at its end.
/// `was` is the piece's last height, kept where there is no ground under it.
fn gate_part_transform(
    race: &Active,
    i: usize,
    part: GatePart,
    collide: &WorldCollision,
    was: f32,
    sheet_height: f32,
) -> Transform {
    match part {
        GatePart::Sheet => {
            let p = race.points[i];
            let h = race.heading(i);
            let ground = ground_at(collide, p, was - sheet_height / 2.0);
            let along =
                wow_to_bevy([p[0] + h[0], p[1] + h[1], 0.0]) - wow_to_bevy([p[0], p[1], 0.0]);
            Transform::from_translation(ground + Vec3::Y * (sheet_height / 2.0))
                .with_rotation(Quat::from_rotation_y(along.x.atan2(along.z)))
                .with_scale(Vec3::new(2.0 * race.gate_half_width(i), sheet_height, 1.0))
        }
        GatePart::End(k) => {
            let end = race.gate_ends(i)[k];
            let ground = ground_at(collide, end, was - BEAM_HEIGHT / 2.0);
            Transform::from_translation(ground + Vec3::Y * (BEAM_HEIGHT / 2.0))
        }
    }
}

/// Draws each checkpoint of the race we are in as its gate, the next one bright, and floats an
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
    settings: Res<super::settings::KartSettings>,
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
    let mut have = vec![[false; GATE_PARTS.len()]; n];
    let part_index = |part: GatePart| match part {
        GatePart::Sheet => 0,
        GatePart::End(k) => 1 + k,
    };
    for (_, beam, mut t, mut material) in &mut beams {
        let Beam(i, part) = *beam;
        if i >= n {
            continue;
        }
        have[i][part_index(part)] = true;
        *t = gate_part_transform(
            race,
            i,
            part,
            &collide,
            t.translation.y,
            settings.gate_height,
        );
        material.0 = match (part, i == next) {
            (GatePart::Sheet, true) => looks.sheet_bright.clone(),
            (GatePart::Sheet, false) => looks.sheet_dim.clone(),
            (GatePart::End(_), true) => looks.bright.clone(),
            (GatePart::End(_), false) => looks.dim.clone(),
        };
    }
    for (i, had) in have.iter().enumerate() {
        for part in GATE_PARTS {
            if had[part_index(part)] {
                continue;
            }
            let (mesh, material) = match part {
                GatePart::Sheet => (looks.sheet.clone(), looks.sheet_dim.clone()),
                GatePart::End(_) => (looks.beam.clone(), looks.dim.clone()),
            };
            commands.spawn((
                Beam(i, part),
                Mesh3d(mesh),
                MeshMaterial3d(material),
                gate_part_transform(race, i, part, &collide, 0.0, settings.gate_height),
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
            assert_eq!(
                ZONES.contains(&c.zone),
                !c.party(),
                "{} is in a Race Finder zone unless it is a party course",
                c.key
            );
        }
    }

    #[test]
    fn stratholme_is_a_party_course_the_finder_never_lists() {
        let strat = COURSES
            .iter()
            .find(|c| c.key == "stratholme-streets")
            .unwrap();
        assert_eq!(strat.map, 329);
        assert!(strat.party());
        assert_eq!(strat.laps, 3);
        assert!(strat.marshal.is_some(), "a marshal inside the instance");
        assert!(!ZONES.contains(&strat.zone));
        assert!(strat.description().contains("A party race"));
        assert!(COURSES.iter().filter(|c| c.map <= 1).all(|c| !c.party()));
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

    /// A server line as [`on_chat`] queues it, `R\t` taken off.
    fn hear(races: &mut Races, line: &str, now: f32) -> Heard {
        let f: Vec<String> = line.split('\t').map(str::to_string).collect();
        races.hear(&f, now)
    }

    fn racer() -> Races {
        Races {
            me: "Racer".into(),
            ..Default::default()
        }
    }

    /// A circuit of four checkpoints, three laps, under way since 10 s.
    fn racing() -> Races {
        let mut races = racer();
        hear(&mut races, "L\t5\tRacer,Goblin", 1.0);
        hear(&mut races, "C\t5\tsprint\t3\t0:0;100:0;200:0;200:100", 1.0);
        hear(&mut races, "G\t5\t9000\tRacer,Goblin", 1.0);
        races
    }

    #[test]
    fn a_report_the_server_did_not_take_is_sent_again_in_order() {
        // The lap smoke run: checkpoint 4 reported as the warp's ack was still on its way, refused,
        // and every report after it refused for being out of order.
        let mut races = racing();
        let race = races.race.as_mut().unwrap();
        race.pass(1, 11.0);
        // Reported and taken at once: nothing to send again.
        hear(&mut races, "P\t5\tRacer\t0\t1", 11.05);
        let race = races.race.as_mut().unwrap();
        assert!(race.resend(11.1).is_empty());
        assert!(race.resend(20.0).is_empty(), "in step");
        // Checkpoint 2's report goes unanswered; 3, then 0 (a lap) pass meanwhile.
        race.pass(2, 21.0);
        assert!(race.resend(21.0).is_empty(), "its answer may be on its way");
        race.pass(3, 22.0);
        assert!(race.resend(22.4).is_empty());
        assert_eq!(
            race.resend(22.5),
            [2, 3],
            "1.5 s on: every one missing, in order"
        );
        assert!(race.resend(23.0).is_empty(), "not again at once");
        race.pass(0, 23.5);
        assert_eq!(
            race.resend(24.0),
            [2, 3, 0],
            "and again while still missing"
        );
        // The server catches up on 2 and 3; 0 is still owed.
        hear(&mut races, "P\t5\tRacer\t0\t3", 24.1);
        let race = races.race.as_mut().unwrap();
        assert_eq!(race.resend(25.5), [0]);
        hear(&mut races, "P\t5\tRacer\t1\t0", 25.6);
        let race = races.race.as_mut().unwrap();
        assert!(race.resend(30.0).is_empty(), "caught up");
        // Another racer's progress is not ours.
        race.pass(1, 31.0);
        hear(&mut races, "P\t5\tGoblin\t1\t1", 31.1);
        let race = races.race.as_mut().unwrap();
        assert!(race.resend(31.1).is_empty());
        assert_eq!(race.resend(33.0), [1]);
        // Finished: nothing more to report.
        hear(&mut races, "F\t5\tRacer\t1\t60000\t", 33.1);
        assert!(races.race.as_mut().unwrap().resend(40.0).is_empty());
    }

    #[test]
    fn the_clock_and_board_show_the_server_s_finish_time() {
        // The lap smoke run: the HUD read 0:24.76 at the line, the server timed 0:24.72.
        let mut races = racing();
        let race = races.race.as_mut().unwrap();
        for (cp, t) in [(1, 12.0), (2, 14.0), (3, 16.0), (0, 18.0)] {
            race.pass(cp, t);
        }
        for (cp, t) in [(1, 20.0), (2, 22.0), (3, 24.0), (0, 26.0)] {
            race.pass(cp, t);
        }
        for (cp, t) in [(1, 28.0), (2, 30.0), (3, 32.0), (0, 34.76)] {
            race.pass(cp, t);
        }
        assert!((race.clock(40.0, None).unwrap().total - 24.76).abs() < 1e-3);
        hear(&mut races, "F\t5\tRacer\t1\t24720\t", 34.8);
        let race = races.race.as_ref().unwrap();
        let shown = race.clock(40.0, races.run().unwrap().finish_ms).unwrap();
        assert_eq!(shown.total, 24.72);
        // The laps add up to it: the last made up to the server's time.
        assert_eq!(shown.laps.len(), 3);
        assert!((shown.laps.iter().sum::<f32>() - 24.72).abs() < 1e-3);
        assert!((shown.laps[0] - 8.0).abs() < 1e-3 && (shown.laps[1] - 8.0).abs() < 1e-3);
        let lines = board(race);
        assert_eq!(lines[0], "1. Racer  0:24.72");
        assert!(lines.contains(&"Lap 3: 0:08.72".to_string()), "{lines:?}");
    }

    #[test]
    fn out_of_time_racers_are_dnf_on_the_board_and_our_race_ends() {
        let mut races = racing();
        hear(&mut races, "F\t5\tGoblin\t1\t61000\t", 71.0);
        assert!(!races.race.as_ref().unwrap().done);
        // A minute on, the server closes the race on us.
        hear(&mut races, "DNF\t5\tRacer", 131.0);
        let race = races.race.as_ref().unwrap();
        assert!(race.done);
        assert_eq!(board(race)[..2], ["1. Goblin  1:01.00", "DNF  Racer"]);
        assert_eq!(
            races.run().unwrap().finish_ms,
            None,
            "no time to keep a ghost of"
        );
        // The clock stops there: what had run, not nought.
        let clock = race.clock(140.0, None).unwrap();
        assert_eq!(clock.total, 121.0);
        // A repeated line is one entry; another race's is not ours.
        hear(&mut races, "DNF\t5\tRacer", 131.5);
        hear(&mut races, "DNF\t6\tGoblin", 131.5);
        assert_eq!(races.race.as_ref().unwrap().finished.len(), 2);
    }

    #[test]
    fn a_time_trial_is_marked_and_has_no_weapons() {
        let mut races = racer();
        hear(&mut races, "L\t7\tRacer", 1.0);
        hear(&mut races, "TT\t7", 1.0);
        hear(&mut races, "C\t7\tsprint\t0\t1:2;3:4;5:6", 1.0);
        assert_eq!(hear(&mut races, "G\t7\t8000\tRacer", 1.0), Heard::Nothing);
        assert!(races.time_trial());
        assert_eq!(races.weapons_race(), None, "no crates, no items");
        let run = races.run().expect("in the trial");
        assert!(run.time_trial && run.course == "sprint" && run.go_at == Some(9.0));

        // Another race's mark is not ours.
        let mut races = racer();
        hear(&mut races, "L\t8\tRacer,Goblin", 1.0);
        hear(&mut races, "TT\t9", 1.0);
        hear(&mut races, "C\t8\tsprint\t0\t1:2;3:4;5:6", 1.0);
        assert!(!races.time_trial());
        assert_eq!(races.weapons_race(), Some(8), "a race has its weapons");
        assert!(!races.run().unwrap().time_trial);
    }

    #[test]
    fn the_race_s_ground_types_rule_from_the_countdown_to_the_finish() {
        let mut races = racer();
        assert_eq!(
            races.ground_types(),
            None,
            "outside a race: our own setting"
        );
        hear(&mut races, "L\t5\tRacer,Goblin", 1.0);
        assert_eq!(races.ground_types(), None, "waiting to race: still ours");
        // Another race's line is not ours; ours comes with the course at the start.
        hear(&mut races, "GT\t6\t1", 2.0);
        hear(&mut races, "GT\t5\t1", 2.0);
        hear(&mut races, "C\t5\tsprint\t0\t1:2;3:4;5:6", 2.0);
        assert_eq!(races.ground_types(), None, "no countdown yet");
        hear(&mut races, "G\t5\t8000\tRacer,Goblin", 2.0);
        assert_eq!(races.ground_types(), Some(true), "the race's rule");
        // A finish board says so.
        hear(&mut races, "F\t5\tRacer\t1\t60000\t", 20.0);
        let race = races.race.as_ref().unwrap();
        assert_eq!(board(race).last().unwrap(), "Ground types: on");
        assert_eq!(races.ground_types(), None, "finished: ours again");

        // Off is off whatever our setting, and a server that sends no `GT` races off.
        for line in ["GT\t5\t0", ""] {
            let mut races = racer();
            hear(&mut races, "L\t5\tRacer,Goblin", 1.0);
            hear(&mut races, line, 1.0);
            hear(&mut races, "G\t5\t8000\tRacer,Goblin", 1.0);
            assert_eq!(races.ground_types(), Some(false), "{line:?}");
            hear(&mut races, "X", 2.0);
            assert_eq!(races.ground_types(), None, "left: ours again");
        }

        // A time trial is always off, even sent on: its leaderboard is one board.
        let mut races = racer();
        hear(&mut races, "L\t7\tRacer", 1.0);
        hear(&mut races, "TT\t7", 1.0);
        hear(&mut races, "GT\t7\t1", 1.0);
        hear(&mut races, "G\t7\t8000\tRacer", 1.0);
        assert_eq!(races.ground_types(), Some(false));
    }

    #[test]
    fn a_new_roster_with_us_in_it_replaces_the_race_we_were_in() {
        let mut races = racer();
        hear(&mut races, "L\t3\tRacer,Goblin", 1.0);
        hear(&mut races, "F\t3\tRacer\t1\t60000\t", 2.0);
        assert!(races.race.as_ref().unwrap().done);
        // The results still showing, a time trial at the marshal: the server has moved us.
        hear(&mut races, "L\t4\tRacer", 3.0);
        hear(&mut races, "TT\t4", 3.0);
        let race = races.race.as_ref().unwrap();
        assert!(race.id == 4 && race.time_trial && !race.done && race.finished.is_empty());
        // The ready check's cue and a leave-everything still come out.
        assert_eq!(
            hear(&mut races, "Pop\t1\tsprint\t2\t20", 4.0),
            Heard::ReadyCheck
        );
        assert_eq!(hear(&mut races, "X", 4.0), Heard::LeftAll);
        assert!(races.race.is_none());
    }

    #[test]
    fn a_time_trial_s_board_sets_the_time_against_the_best_before_it() {
        let mut races = racer();
        hear(&mut races, "L\t5\tRacer", 1.0);
        hear(&mut races, "TT\t5", 1.0);
        hear(&mut races, "Rec\t5\tsprint\t58000\tGoblin\t61000", 1.0);
        hear(&mut races, "F\t5\tRacer\t1\t60000\tp", 50.0);
        // The new best comes back as the server's records: the board still measures the old one.
        hear(&mut races, "Rec\t5\tsprint\t58000\tGoblin\t60000", 50.0);
        let race = races.race.as_ref().unwrap();
        assert_eq!(
            board(race),
            [
                "Your time: 1:00.00  (best)",
                "0:01.00 faster than your best of 1:01.00",
                "Time trial record: 0:58.00 by Goblin",
            ]
        );
        // Slower than the best, and a first run with no record yet.
        let mut slow = race.clone();
        slow.bests.clear();
        slow.finished = vec![("Racer".into(), 1, 62_500)];
        assert_eq!(board(&slow)[1], "0:01.50 behind your best of 1:01.00");
        let mut first = slow.clone();
        first.before = Some((0, 0));
        first.record = (0, String::new());
        assert_eq!(
            board(&first),
            ["Your time: 1:02.50", "Your first time trial here"]
        );
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

    /// The test course as a three-lap circuit with a race of four: us, two others and one in.
    fn circuit_race() -> Active {
        Active {
            laps: 3,
            go_at: Some(10.0),
            roster: vec![
                "Racer".into(),
                "Goblin".into(),
                "Gnome".into(),
                "Dwarf".into(),
            ],
            ..course()
        }
    }

    #[test]
    fn progress_runs_along_the_course_between_checkpoints() {
        let sprint = course();
        // Projected onto the leg past the last checkpoint: yards from the start.
        assert!((sprint.progress(0, 0, Some([50.0, 7.0])) - 50.0).abs() < 1e-4);
        assert!((sprint.progress(0, 2, Some([203.0, 40.0])) - 240.0).abs() < 1e-4);
        // Beyond the next checkpoint it holds there until its gate is passed.
        assert!((sprint.progress(0, 1, Some([260.0, 0.0])) - 200.0).abs() < 1e-4);
        // Behind the last checkpoint it counts back down the leg before; behind a sprint's
        // start there is nothing before it.
        assert!((sprint.progress(0, 1, Some([90.0, 0.0])) - 90.0).abs() < 1e-4);
        assert_eq!(sprint.progress(0, 0, Some([-10.0, 0.0])), 0.0);
        // Without a kart, at the checkpoint; the finish has no leg after it.
        assert_eq!(sprint.progress(0, 2, None), 200.0);
        assert_eq!(sprint.progress(0, 3, Some([500.0, 0.0])), 300.0);
        // A circuit's lap is the loop back to the start; laps done count whole.
        let circuit = circuit_race();
        let lap = 300.0 + 200f32.hypot(100.0);
        assert!((circuit.progress(2, 1, Some([150.0, 0.0])) - (2.0 * lap + 150.0)).abs() < 1e-3);
        // Just behind the start line on the grid: a little short of it.
        assert!(circuit.progress(0, 0, Some([-5.0, 0.0])) < 0.0);
    }

    #[test]
    fn standings_rank_by_distance_along_the_course_every_frame() {
        let mut race = circuit_race();
        race.checkpoint = 1;
        race.progress.insert("Goblin".into(), (0, 1, 20.0));
        race.progress.insert("Gnome".into(), (0, 1, 21.0));
        race.progress.insert("Dwarf".into(), (0, 1, 19.0));
        // Goblin's kart is relayed 80 yd past checkpoint 1; Gnome and Dwarf are too far off to
        // be, so they stand at the checkpoint, the one there first ahead.
        let near = [("Goblin".to_string(), [180.0, 3.0])];
        assert_eq!(
            race.standings("Racer", Some([150.0, 0.0]), &near),
            ["Goblin", "Racer", "Dwarf", "Gnome"]
        );
        // We pass Goblin between checkpoints: our place changes with no checkpoint passed.
        assert_eq!(
            race.standings("Racer", Some([190.0, 0.0]), &near)[..2],
            ["Racer", "Goblin"]
        );
        // A lap ahead beats any point on the lap before; the finished lead in their order.
        race.progress.insert("Gnome".into(), (1, 0, 40.0));
        race.finished = vec![("Dwarf".into(), 1, 90_000)];
        assert_eq!(
            race.standings("Racer", Some([190.0, 0.0]), &near),
            ["Dwarf", "Gnome", "Racer", "Goblin"]
        );
    }

    #[test]
    fn wrong_way_shows_after_driving_back_501_lu_facing_back() {
        let lu = 36.0;
        let mut race = circuit_race();
        race.checkpoint = 1;
        let back = [-1.0, 0.0];
        assert!(race.facing_back(1, back));
        // 45 degrees either side of straight back (`0x5A801 / 0x80000`), and no further.
        assert!(race.facing_back(1, [-0.8, 0.6]));
        assert!(!race.facing_back(1, [-0.6, 0.8]));
        assert!(!race.facing_back(1, [1.0, 0.0]));
        // Driving back 13 yd (468 lu) is not enough; 14 (504 lu) is, facing back.
        race.drove(race.progress(0, 1, Some([180.0, 0.0])), lu);
        race.drove(race.progress(0, 1, Some([167.0, 0.0])), lu);
        assert!(!race.wrong_way(true));
        race.drove(race.progress(0, 1, Some([166.0, 0.0])), lu);
        assert!(race.wrong_way(true));
        assert!(
            !race.wrong_way(false),
            "reversing, facing on down the course"
        );
        // It stops counting at 1000 lu: 14 yd back the right way clears it after any reverse.
        race.drove(race.progress(0, 1, Some([100.0, 0.0])), lu);
        assert_eq!(race.backwards, WRONG_WAY_MAX_LU);
        race.drove(race.progress(0, 1, Some([114.0, 0.0])), lu);
        assert!(!race.wrong_way(true));
        // It flashes on bit 0x80 of the race's time in 1/960 s: off, on, off again.
        assert!(!wrong_way_lit(0.0));
        assert!(wrong_way_lit(0.14));
        assert!(!wrong_way_lit(0.27));
    }

    #[test]
    fn our_kart_faces_where_its_yaw_points() {
        for d in [[1.0f32, 0.0], [0.0, 1.0], [-0.6, 0.8], [0.0, -1.0]] {
            // The autopilot's yaw for a heading, as `kart::update` steers for a checkpoint.
            let to = wow_to_bevy([d[0], d[1], 0.0]);
            let f = facing_of(f32::atan2(-to.x, -to.z));
            assert!(
                (f[0] - d[0]).abs() < 1e-5 && (f[1] - d[1]).abs() < 1e-5,
                "{d:?} {f:?}"
            );
        }
    }

    #[test]
    fn laps_are_timed_at_the_start_line_and_the_last_begins_the_final_lap() {
        let mut race = circuit_race();
        race.count_down(100.0, 4000.0);
        // Before GO the clock stands at nought, its first lap with it.
        let clock = race.clock(102.0, None).unwrap();
        assert_eq!((clock.total, clock.laps.as_slice()), (0.0, &[0.0][..]));
        for (cp, t) in [(1, 110.0), (2, 115.0), (3, 120.0), (0, 130.0)] {
            race.pass(cp, t);
        }
        assert_eq!(race.lap_times, [26.0]);
        assert_eq!(race.final_lap_at, None, "lap 2 of 3 is not the last");
        for (cp, t) in [(1, 135.0), (2, 140.0), (3, 148.0), (0, 155.0)] {
            race.pass(cp, t);
        }
        assert_eq!(race.lap_times, [26.0, 25.0]);
        assert_eq!(race.final_lap_at, Some(155.0));
        // The clock runs, its third lap under way.
        let clock = race.clock(160.0, None).unwrap();
        assert_eq!(clock.total, 56.0);
        assert_eq!(clock.laps, [26.0, 25.0, 5.0]);
        // "FINAL LAP" flies by for 90 frames at 30 fps.
        assert_eq!(race.final_lap_frame(155.0), Some(0));
        assert_eq!(race.final_lap_frame(156.0), Some(30));
        assert_eq!(race.final_lap_frame(157.99), Some(89));
        assert_eq!(race.final_lap_frame(158.0), None);
        // The final lap's sting starts with it, the song lowered under it ...
        assert_eq!(race.final_lap_secs(155.0), Some(0.0));
        let ducked = kartcraft::sound::final_lap_volume(race.final_lap_secs(156.2));
        assert_eq!(ducked, 90.0 / 190.0);
        // ... and 1.31 s in the song is back up and 20 BPM faster.
        assert_eq!(race.music_bpm_delta(156.2), 0.0);
        assert_eq!(race.music_bpm_delta(156.4), FINAL_LAP_BPM);
        assert_eq!(
            kartcraft::sound::final_lap_volume(race.final_lap_secs(156.4)),
            1.0
        );
        // Over the line: the clock stops with three laps.
        for (cp, t) in [(1, 160.0), (2, 165.0), (3, 172.0), (0, 179.0)] {
            race.pass(cp, t);
        }
        let clock = race.clock(200.0, None).unwrap();
        assert_eq!(clock.total, 75.0);
        assert_eq!(clock.laps, [26.0, 25.0, 24.0]);
        assert!(!clock.time_trial);
        race.done = true;
        assert_eq!(race.music_bpm_delta(200.0), 0.0);
        assert_eq!(race.final_lap_secs(200.0), None);
        // A one-lap race has no final lap to announce.
        let mut one = Active {
            laps: 1,
            ..circuit_race()
        };
        one.count_down(0.0, 0.0);
        for cp in [1, 2, 3, 0] {
            one.pass(cp, 10.0);
        }
        assert_eq!((one.final_lap_at, one.crossed_finish), (None, Some(10.0)));
    }

    #[test]
    fn a_sprint_s_clock_has_no_laps_and_stops_at_its_finish() {
        let mut race = course();
        race.time_trial = true;
        race.count_down(0.0, 2000.0);
        for (cp, t) in [(1, 10.0), (2, 20.0), (3, 32.5)] {
            race.pass(cp, t);
        }
        let clock = race.clock(50.0, None).unwrap();
        assert_eq!(clock.total, 30.5);
        assert!(clock.laps.is_empty() && clock.time_trial);
        // A finish we did not see ourselves takes the server's time.
        let mut unseen = course();
        unseen.count_down(0.0, 0.0);
        assert_eq!(unseen.clock(90.0, Some(61_250)).unwrap().total, 61.25);
    }

    #[test]
    fn the_board_ends_with_our_lap_times_the_fastest_marked() {
        let mut race = circuit_race();
        race.finished = vec![("Racer".into(), 2, 75_750)];
        race.lap_times = vec![26.0, 25.5, 24.25];
        assert_eq!(
            board(&race),
            [
                "2. Racer  1:15.75",
                "Lap 1: 0:26.00",
                "Lap 2: 0:25.50",
                "Lap 3: 0:24.25  (fastest)",
            ]
        );
        // A sprint has no laps to show.
        let mut sprint = course();
        sprint.finished = vec![("Racer".into(), 1, 30_000)];
        sprint.lap_times = vec![30.0];
        assert_eq!(board(&sprint), ["1. Racer  0:30.00"]);
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
    fn the_gate_is_drawn_as_wide_as_it_counts() {
        let mut race = course();
        race.gates = vec![20.0, 15.0, 25.0, 20.0];
        for i in [1, 2] {
            let ends = race.gate_ends(i);
            let p = race.points[i];
            let h = race.heading(i);
            for end in ends {
                // Each end is the half-width out from the centre, square to the course.
                let off = [end[0] - p[0], end[1] - p[1]];
                assert!((off[0].hypot(off[1]) - race.gate_half_width(i)).abs() < 1e-3);
                assert!((off[0] * h[0] + off[1] * h[1]).abs() < 1e-3);
                // Crossing just inside the drawn end counts; just outside it does not.
                let at =
                    |s: f32, t: f32| [p[0] + off[0] * s + h[0] * t, p[1] + off[1] * s + h[1] * t];
                assert!(race.passes(i, Some(at(0.98, -0.5)), at(0.98, 0.5)));
                assert!(!race.passes(i, Some(at(1.02, -0.5)), at(1.02, 0.5)));
            }
        }
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
        // The shaft and head meet edge to edge: the see-through face is one layer everywhere.
        let shaft_top = CP_ARROW_SHAFT.iter().map(|p| p[1]).fold(f32::MIN, f32::max);
        let head_base = CP_ARROW_HEAD.iter().map(|p| p[1]).fold(f32::MAX, f32::min);
        assert_eq!(shaft_top, head_base);
        // The outline band runs the edge out from every side of the whole shape.
        let band = grown(&CP_ARROW_SHAPE, CP_ARROW_EDGE);
        for (k, (p, q)) in CP_ARROW_SHAPE.iter().zip(&band).enumerate() {
            let next = CP_ARROW_SHAPE[(k + 1) % CP_ARROW_SHAPE.len()];
            let side = (Vec2::from(next) - Vec2::from(*p)).normalize();
            let out = (Vec2::from(*q) - Vec2::from(*p)).dot(Vec2::new(side.y, -side.x));
            assert!((out - CP_ARROW_EDGE).abs() < 1e-4, "corner {k}: {out}");
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
    fn the_countdown_banner_shows_the_seconds_to_the_start() {
        assert_eq!(countdown_banner(24.2, true, false), "Race starts in 25");
        assert_eq!(countdown_banner(3.5, true, false), "Race starts in 4");
        assert_eq!(countdown_banner(2.1, true, false), "3");
        assert_eq!(countdown_banner(0.2, true, false), "1");
        assert_eq!(
            countdown_banner(17.0, false, false),
            "Getting to the start... race starts in 17"
        );
        assert_eq!(
            countdown_banner(0.4, false, false),
            "Getting to the start... race starts in 1"
        );
        // A race with ground types on says so, but not over CTR's 3, 2, 1.
        assert_eq!(
            countdown_banner(9.5, true, true),
            "Race starts in 10 — ground types on"
        );
        assert_eq!(
            countdown_banner(17.0, false, true),
            "Getting to the start... race starts in 17 — ground types on"
        );
        assert_eq!(countdown_banner(2.1, true, true), "3");
    }

    #[test]
    fn the_start_lights_beep_three_reds_and_a_green_at_go_once_each() {
        use kartcraft::sound::Cue;
        // A Race Finder countdown of 28 s, every frame at 60 Hz until 2 s after GO.
        let mut race = Active::default();
        race.count_down(100.0, 28000.0);
        let go = race.go_at.unwrap();
        let mut heard = Vec::new();
        let mut shown = Vec::new();
        let mut now = 100.0f32;
        while now < go + 2.0 {
            let mut cues = Vec::new();
            let timer = race.start_lights(go - now, &mut cues);
            heard.extend(cues.into_iter().map(|c| (c, go - now)));
            if (kartcraft::hud::LIGHTS_TIMER_END..=kartcraft::hud::LIGHTS_TIMER_START)
                .contains(&timer)
            {
                shown.push(go - now);
            }
            now += 1.0 / 60.0;
        }
        let cues: Vec<Cue> = heard.iter().map(|h| h.0).collect();
        assert_eq!(
            cues,
            [
                Cue::Countdown(false),
                Cue::Countdown(false),
                Cue::Countdown(false),
                Cue::Countdown(true)
            ]
        );
        // Each in the first frame at or past its second: 3, 2, 1 before GO, then GO.
        for (&(_, left), at) in heard.iter().zip([3.0f32, 2.0, 1.0, 0.0]) {
            assert!(
                left <= at + 1e-3 && left > at - 1.0 / 60.0 - 1e-3,
                "{left} {at}"
            );
        }
        // The lights are up for CTR's last 4 s before GO and the second after it.
        let (first, last) = (shown[0], *shown.last().unwrap());
        assert!(
            (first - 4.0).abs() < 0.02 && (last + 1.0).abs() < 0.02,
            "{first} {last}"
        );

        // A countdown arriving 1.5 s before GO sounds the light it lands on, then GO.
        let mut late = Active::default();
        late.count_down(0.0, 1500.0);
        let mut cues = Vec::new();
        for t in [0.0f32, 0.1, 0.6, 1.0, 1.5, 1.6] {
            late.start_lights(1.5 - t, &mut cues);
        }
        assert_eq!(
            cues,
            [
                Cue::Countdown(false),
                Cue::Countdown(false),
                Cue::Countdown(true)
            ]
        );
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
