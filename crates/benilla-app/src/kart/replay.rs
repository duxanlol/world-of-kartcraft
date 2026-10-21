//! Replays: every race we are in, to rewatch. From GO until a little after our finish, our kart
//! and every racer's on the roster are sampled [`RATE`] times a second, as the ghost samples ours
//! (`super::ghost`); when the race is over for us the take goes to `benilla-config/kart-replays/`
//! ([`crate::local_state`]), the newest [`KEEP`] kept. `KartReplays` lists them for the
//! Kartcraft menu's Replays window, newest first; setting `KartReplay` to one plays it while we
//! sit in the kart: every racer's kart drives its run again, the camera behind the one we watch.
//! Steering left or right watches the one ahead or behind in the field's order, the camera
//! button changes the camera (behind, low, circling, a tripod), and the take ends by itself or
//! when `KartReplay` goes back to `0`. Only the karts are kept, not their drivers.

use std::collections::HashMap;

use bevy::prelude::*;
use kartcraft::camera::{CineCamera, CineMode, CineSubject};
use kartcraft::model::{KartRoot, KartSpawner, KartWheels};
use kartcraft::KartView;

use super::ghost::{Pose, Reader};
use crate::net::{Guid, SelfPlayer};

/// Samples a second.
pub(crate) const RATE: u8 = 20;
/// The longest take kept (s).
const MAX_SECS: u32 = 20 * 60;
/// How long past our finish the others are still recorded (s).
const AFTER_FINISH: f32 = 8.0;
/// The shortest take worth keeping (s).
const MIN_SECS: f32 = 5.0;
/// Replays kept on disk.
pub(crate) const KEEP: usize = 10;
/// The most racers in one take.
const MAX_TRACKS: usize = 16;
const MAGIC: &[u8; 4] = b"KRPL";
const VERSION: u8 = 1;

/// One racer's run in a take: from sample `from` on, one pose a sample.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Track {
    pub(crate) name: String,
    /// Whose kart (`crash`, `coco`, ...).
    pub(crate) racer: String,
    /// Ours.
    pub(crate) us: bool,
    pub(crate) from: u32,
    pub(crate) poses: Vec<Pose>,
}

impl Track {
    /// Where this racer was `t` seconds after GO, between the samples around it; `None` before
    /// its first sample or after its last.
    fn pose_at(&self, t: f32, rate: u8) -> Option<Pose> {
        let f = t * f32::from(rate) - self.from as f32;
        let last = self.poses.len().checked_sub(1)?;
        if f < 0.0 || f > last as f32 {
            return None;
        }
        let i = (f.floor() as usize).min(last);
        if i == last {
            return Some(self.poses[last]);
        }
        Some(self.poses[i].lerp(self.poses[i + 1], f - i as f32))
    }

    fn end(&self, rate: u8) -> f32 {
        (self.from as usize + self.poses.len().saturating_sub(1)) as f32 / f32::from(rate)
    }
}

/// A race as recorded.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Replay {
    pub(crate) course: String,
    /// When the race was run, unix seconds.
    pub(crate) at: u64,
    /// Our finish (ms) and place; 0 none.
    pub(crate) ms: u32,
    pub(crate) place: u8,
    pub(crate) field: u8,
    pub(crate) time_trial: bool,
    pub(crate) rate: u8,
    pub(crate) tracks: Vec<Track>,
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    let b = &s.as_bytes()[..s.len().min(255)];
    out.push(b.len() as u8);
    out.extend_from_slice(b);
}

fn take_str(r: &mut Reader<'_>) -> Option<String> {
    let len = usize::from(r.u8()?);
    Some(std::str::from_utf8(r.take(len)?).ok()?.to_string())
}

impl Replay {
    /// The file: `KRPL`, a version byte, the rate, the time (u64), our time (u32), our place,
    /// the field, a time trial (u8), the course (a length byte and UTF-8), the track count (u8),
    /// then per track its name and racer (as the course), whether it is ours (u8), its first
    /// sample (u32), its sample count (u32) and its samples (`Pose::put`). Little endian.
    pub(crate) fn encode(&self) -> Vec<u8> {
        let samples: usize = self.tracks.iter().map(|t| t.poses.len()).sum();
        let mut out = Vec::with_capacity(64 + samples * Pose::BYTES);
        out.extend_from_slice(MAGIC);
        out.push(VERSION);
        out.push(self.rate);
        out.extend_from_slice(&self.at.to_le_bytes());
        out.extend_from_slice(&self.ms.to_le_bytes());
        out.push(self.place);
        out.push(self.field);
        out.push(u8::from(self.time_trial));
        put_str(&mut out, &self.course);
        let tracks = &self.tracks[..self.tracks.len().min(MAX_TRACKS)];
        out.push(tracks.len() as u8);
        for t in tracks {
            put_str(&mut out, &t.name);
            put_str(&mut out, &t.racer);
            out.push(u8::from(t.us));
            out.extend_from_slice(&t.from.to_le_bytes());
            out.extend_from_slice(&(t.poses.len() as u32).to_le_bytes());
            for p in &t.poses {
                p.put(&mut out);
            }
        }
        out
    }

    /// Reads [`Replay::encode`]'s file; `None` for anything else.
    pub(crate) fn decode(bytes: &[u8]) -> Option<Replay> {
        let mut r = Reader(bytes);
        if r.take(4)? != MAGIC || r.u8()? != VERSION {
            return None;
        }
        let rate = r.u8()?;
        let at = u64::from_le_bytes(r.take(8)?.try_into().ok()?);
        let ms = r.u32()?;
        let place = r.u8()?;
        let field = r.u8()?;
        let time_trial = r.u8()? != 0;
        let course = take_str(&mut r)?;
        let count = usize::from(r.u8()?);
        let max = MAX_SECS as usize * usize::from(rate);
        if rate == 0 || count > MAX_TRACKS {
            return None;
        }
        let mut tracks = Vec::with_capacity(count);
        for _ in 0..count {
            let name = take_str(&mut r)?;
            let racer = take_str(&mut r)?;
            let us = r.u8()? != 0;
            let from = r.u32()?;
            let n = r.u32()? as usize;
            if from as usize + n > max || r.0.len() < n * Pose::BYTES {
                return None;
            }
            let poses = (0..n)
                .map(|_| Pose::take(&mut r))
                .collect::<Option<Vec<_>>>()?;
            tracks.push(Track {
                name,
                racer,
                us,
                from,
                poses,
            });
        }
        r.0.is_empty().then_some(Replay {
            course,
            at,
            ms,
            place,
            field,
            time_trial,
            rate,
            tracks,
        })
    }

    /// When the last racer's run ends, seconds after GO.
    pub(crate) fn length(&self) -> f32 {
        self.tracks
            .iter()
            .map(|t| t.end(self.rate))
            .fold(0.0, f32::max)
    }

    /// Its line in the Replays window: the course, how we did, and when.
    pub(crate) fn label(&self) -> String {
        let course = super::race::COURSES
            .iter()
            .find(|c| c.key == self.course)
            .map_or(self.course.as_str(), |c| c.name.as_str());
        let how = match (self.time_trial, self.place, self.ms) {
            (true, _, ms) if ms > 0 => format!("time trial, {}", clock(ms)),
            (true, ..) => "time trial, did not finish".to_string(),
            (false, 0, _) => "did not finish".to_string(),
            (false, place, ms) => format!("{} of {}, {}", ordinal(place), self.field, clock(ms)),
        };
        format!("{course}: {how}")
    }
}

fn clock(ms: u32) -> String {
    format!(
        "{}:{:02}.{:02}",
        ms / 60_000,
        ms / 1000 % 60,
        ms % 1000 / 10
    )
}

fn ordinal(n: u8) -> String {
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

/// One racer's run being recorded.
struct Take {
    name: String,
    racer: String,
    from: u32,
    poses: Vec<Pose>,
    /// The last pose given and when (s after GO), to place the samples between frames.
    last: Option<(f32, Pose)>,
}

impl Take {
    fn new(name: &str, racer: &str) -> Self {
        Self {
            name: name.to_string(),
            racer: racer.to_string(),
            from: 0,
            poses: Vec::new(),
            last: None,
        }
    }

    /// The kart's pose `t` seconds after GO: every sample time since the last pose given is
    /// filled, between that pose and this one; the first sample is the first sample time at or
    /// after `t`.
    fn push(&mut self, t: f32, pose: Pose) {
        let rate = f32::from(RATE);
        if self.last.is_none() {
            self.from = (t.max(0.0) * rate).ceil() as u32;
        }
        let (t0, p0) = self.last.unwrap_or((t, pose));
        loop {
            let i = self.from as usize + self.poses.len();
            if i >= MAX_SECS as usize * usize::from(RATE) {
                break;
            }
            let at = i as f32 / rate;
            if at > t {
                break;
            }
            let k = if t > t0 {
                ((at - t0) / (t - t0)).clamp(0.0, 1.0)
            } else {
                1.0
            };
            self.poses.push(p0.lerp(pose, k));
        }
        self.last = Some((t, pose));
    }
}

/// The race being recorded.
struct Recording {
    race: u32,
    go: f32,
    course: String,
    time_trial: bool,
    /// Ours under 0, the others under their guid.
    takes: HashMap<u64, Take>,
    /// When we finished (local s), our time and place.
    finished: Option<(f32, u32, u8)>,
    field: u8,
    /// Its take is saved: nothing more of this race is recorded.
    kept: bool,
}

impl Recording {
    /// The take so far as a replay, if it is worth keeping; the samples go with it.
    fn take_replay(&mut self, at: u64) -> Option<Replay> {
        let (ms, place) = self.finished.map_or((0, 0), |(_, ms, place)| (ms, place));
        let mut tracks: Vec<Track> = std::mem::take(&mut self.takes)
            .into_iter()
            .filter(|(_, t)| t.poses.len() > 1)
            .map(|(key, t)| Track {
                name: t.name,
                racer: t.racer,
                us: key == 0,
                from: t.from,
                poses: t.poses,
            })
            .collect();
        // Ours first, then by name, so a file reads the same each time.
        tracks.sort_by(|a, b| b.us.cmp(&a.us).then_with(|| a.name.cmp(&b.name)));
        tracks.truncate(MAX_TRACKS);
        let replay = Replay {
            course: self.course.clone(),
            at,
            ms,
            place,
            field: self.field,
            time_trial: self.time_trial,
            rate: RATE,
            tracks,
        };
        (replay.tracks.iter().any(|t| t.us) && replay.length() >= MIN_SECS).then_some(replay)
    }
}

/// How the replay is filmed; the camera button steps through them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Shot {
    #[default]
    Behind,
    Low,
    Orbit,
    Tripod,
}

impl Shot {
    fn next(self) -> Self {
        match self {
            Shot::Behind => Shot::Low,
            Shot::Low => Shot::Orbit,
            Shot::Orbit => Shot::Tripod,
            Shot::Tripod => Shot::Behind,
        }
    }
}

/// A replay playing.
struct Playing {
    token: String,
    replay: Replay,
    /// Seconds into the take; it starts a moment before GO.
    t: f32,
    /// The track watched.
    watch: usize,
    karts: Vec<Option<Entity>>,
    spins: Vec<(f32, Option<Vec3>)>,
    shot: Shot,
    cine: Option<CineCamera>,
    /// The chase camera's place behind the kart, in the kart's frame, as ours was when the
    /// replay began; and its look.
    behind: (Vec3, Vec3),
    eye: Option<Vec3>,
    /// Last frame's steer and camera buttons, for their presses.
    steer_was: f32,
    camera_was: bool,
}

/// The replays: the race being recorded, the one playing, and what is asked for.
#[derive(Resource, Default)]
pub(crate) struct Replays {
    recording: Option<Recording>,
    playing: Option<Playing>,
    /// `KartReplay` as last set: a replay's token, or "" / "0" for none.
    wanted: String,
    /// `KartReplays` needs writing.
    list_stale: bool,
    listed: bool,
    /// This frame's controls, as the kart would have read them.
    input: kartcraft::input::Controls,
    /// A server replay being fetched ([`super::server_replays`]).
    fetch: Option<super::server_replays::Fetch>,
}

impl Replays {
    /// Whether a replay has the camera, our kart held meanwhile.
    pub(crate) fn playing(&self) -> bool {
        self.playing.is_some()
    }

    /// This frame's controls, which steer the replay while it plays.
    pub(crate) fn feed(&mut self, controls: kartcraft::input::Controls) {
        self.input = controls;
    }
}

/// The newest replays on disk, newest first: (token, path).
fn on_disk() -> Vec<(String, std::path::PathBuf)> {
    let Some(dir) = crate::local_state::kart_replays_dir() else {
        return Vec::new();
    };
    let mut files: Vec<(String, std::path::PathBuf)> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let token = path
                .file_name()?
                .to_str()?
                .strip_suffix(".krpl")?
                .to_string();
            Some((token, path))
        })
        .collect();
    // The token begins with the unix time, zero-padded: names sort as times.
    files.sort_by(|a, b| b.0.cmp(&a.0));
    files
}

fn token_of(replay: &Replay) -> String {
    let course: String = replay
        .course
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("{:012}-{course}", replay.at)
}

/// Writes `replay` and drops the oldest past [`KEEP`]; its token, once written.
fn save(replay: &Replay) -> Option<String> {
    let dir = crate::local_state::kart_replays_dir()?;
    let path = dir.join(format!("{}.krpl", token_of(replay)));
    if let Err(e) = crate::local_state::write_atomic_bytes(&path, &replay.encode()) {
        warn!("kart replay: {}: {e}", path.display());
        return None;
    }
    info!(
        "kart replay: {} kept ({} racers, {:.0} s)",
        path.display(),
        replay.tracks.len(),
        replay.length()
    );
    for (_, old) in on_disk().into_iter().skip(KEEP) {
        let _ = std::fs::remove_file(old);
    }
    Some(token_of(replay))
}

/// The unattended hook, `WOW_KART_REPLAY=latest`: plays each replay as soon as it is kept.
fn hook_plays() -> bool {
    static ON: std::sync::LazyLock<bool> =
        std::sync::LazyLock::new(|| std::env::var("WOW_KART_REPLAY").is_ok_and(|v| v == "latest"));
    *ON
}

fn read(token: &str) -> Option<Replay> {
    let (_, path) = on_disk().into_iter().find(|(t, _)| t == token)?;
    let replay = Replay::decode(&std::fs::read(&path).ok()?);
    if replay.is_none() {
        warn!("kart replay: {} is not a replay file", path.display());
    }
    replay
}

/// `KartReplays`: `token|label` per replay, newest first, joined by `;`.
fn listing() -> String {
    on_disk()
        .into_iter()
        .take(KEEP)
        .filter_map(|(token, path)| {
            let replay = Replay::decode(&std::fs::read(path).ok()?)?;
            let label = replay.label().replace([';', '|'], ",");
            Some(format!("{token}|{label}"))
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// `KartReplay` as it moves: the replay to play, `0` none.
pub(crate) fn on_cvar(ev: On<crate::cvars::CvarChanged>, mut replays: ResMut<Replays>) {
    if ev.is("KartReplay") {
        replays.wanted = ev.new.trim().to_string();
    }
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Records the race we are in, and plays a replay asked for.
#[allow(clippy::too_many_arguments)]
pub(crate) fn update(
    time: Res<Time>,
    races: Res<super::race::Races>,
    remotes: Res<super::remote::RemoteKarts>,
    names: Res<crate::names::NameCache>,
    me: Query<&Guid, With<SelfPlayer>>,
    view: Res<KartView>,
    (mut drive, mut cvars): (ResMut<super::KartDrive>, ResMut<crate::cvars::Cvars>),
    mut replays: ResMut<Replays>,
    mut spawner: KartSpawner,
    mut karts: Query<(&mut Transform, &mut Visibility, &mut KartWheels), With<KartRoot>>,
) {
    let now = time.elapsed_secs();
    let replays = &mut *replays;
    let our_name = me
        .single()
        .ok()
        .and_then(|g| names.peek(g.0))
        .unwrap_or("")
        .to_string();

    if !replays.listed {
        replays.listed = true;
        replays.list_stale = true;
    }

    record(
        now, &races, &remotes, &names, &our_name, &view, &karts, replays,
    );

    if std::mem::take(&mut replays.list_stale) {
        cvars.set("KartReplays", &listing());
    }

    // The replay asked for: started, changed or stopped.
    let wanted = replays.wanted.clone();
    let off = wanted.is_empty() || wanted == "0";
    let current = replays.playing.as_ref().map(|p| p.token.clone());
    if off || current.as_deref() != Some(wanted.as_str()) {
        if let Some(p) = replays.playing.take() {
            for kart in p.karts.into_iter().flatten() {
                spawner.despawn(kart);
            }
        }
    }
    // Asked for out of the kart, it waits for the kart.
    if !off && replays.playing.is_none() && drive.active && view.active {
        // A server replay (`s<key>`) is fetched first, waiting meanwhile.
        let loaded = match wanted.strip_prefix(super::server_replays::TOKEN) {
            Some(key) => match super::server_replays::load(key, &our_name, &mut replays.fetch) {
                super::server_replays::Load::Ready(r) => Some(Some(r)),
                super::server_replays::Load::Waiting => None,
                super::server_replays::Load::Missing => Some(None),
            },
            None => Some(read(&wanted)),
        };
        match loaded {
            None => {}
            Some(Some(replay)) => {
                let n = replay.tracks.len();
                let watch = replay.tracks.iter().position(|t| t.us).unwrap_or(0);
                // The chase camera's place and look in our kart's frame, to film theirs alike.
                let behind = view.camera.map_or(
                    (Vec3::new(0.0, 1.4, -3.5), Vec3::new(0.0, 0.6, 2.0)),
                    |c| {
                        let inv = view.transform.rotation.inverse();
                        let eye = inv * (c.translation - view.transform.translation);
                        let look =
                            inv * (c.translation + c.forward() * 6.0 - view.transform.translation);
                        (eye, look)
                    },
                );
                info!(
                    "kart replay: playing {wanted} ({} racers, {:.0} s)",
                    n,
                    replay.length()
                );
                replays.playing = Some(Playing {
                    token: wanted.clone(),
                    replay,
                    t: -1.0,
                    watch,
                    karts: vec![None; n],
                    spins: vec![(0.0, None); n],
                    shot: Shot::Behind,
                    cine: None,
                    behind,
                    eye: None,
                    steer_was: 0.0,
                    camera_was: false,
                });
            }
            Some(None) => {
                warn!("kart replay: no replay {wanted}");
                replays.wanted = String::new();
                cvars.set("KartReplay", "0");
            }
        }
    }

    let Some(p) = replays.playing.as_mut() else {
        return;
    };
    if !drive.active || !view.active {
        replays.wanted = String::new();
        cvars.set("KartReplay", "0");
        return;
    }
    let dt = time.delta_secs();
    p.t += dt;
    if p.t > p.replay.length() + 1.0 {
        info!("kart replay: {} is over", p.token);
        replays.wanted = String::new();
        cvars.set("KartReplay", "0");
        return;
    }

    // Steer right or left: the next racer or the one before; the camera button: the next shot.
    let c = replays.input;
    let n = p.replay.tracks.len();
    if c.steer.abs() > 0.6 && p.steer_was.abs() <= 0.6 && n > 1 {
        p.watch = if c.steer > 0.0 {
            (p.watch + 1) % n
        } else {
            (p.watch + n - 1) % n
        };
        p.cine = None;
        p.eye = None;
    }
    p.steer_was = c.steer;
    if c.camera && !p.camera_was {
        p.shot = p.shot.next();
        p.cine = None;
    }
    p.camera_was = c.camera;

    // Every racer's kart where its run puts it.
    let radius = super::remote::WHEEL_RADIUS * kartcraft::kart_scale() / kartcraft::METERS_PER_YARD;
    let mut watched = None;
    for i in 0..n {
        let track = &p.replay.tracks[i];
        let pose = track.pose_at(p.t.max(0.0), p.replay.rate);
        if p.karts[i].is_none() {
            p.karts[i] = spawner.spawn(&track.racer);
        }
        let Some(kart) = p.karts[i] else {
            continue;
        };
        let Ok((mut tf, mut vis, mut wheels)) = karts.get_mut(kart) else {
            continue;
        };
        let Some(pose) = pose else {
            *vis = Visibility::Hidden;
            p.spins[i].1 = None;
            continue;
        };
        let (spin, last) = &mut p.spins[i];
        let moved = last
            .replace(pose.pos)
            .map_or(0.0, |q| (pose.pos - q).dot(pose.rot * Vec3::Z));
        *spin = (*spin + moved / radius).rem_euclid(std::f32::consts::TAU);
        *tf = Transform {
            translation: pose.pos,
            rotation: pose.rot,
            scale: Vec3::splat(kartcraft::model_scale()),
        };
        *vis = Visibility::Visible;
        *wheels = KartWheels {
            steer: pose.steer,
            spin: *spin,
        };
        if i == p.watch {
            watched = Some(pose);
        }
    }

    // The camera on the one watched.
    let Some(pose) = watched.or_else(|| p.replay.tracks[p.watch].poses.first().copied()) else {
        return;
    };
    let yaw = {
        let f = pose.rot * Vec3::Z;
        Quat::from_rotation_y(f.x.atan2(f.z))
    };
    let want_eye = pose.pos + yaw * p.behind.0;
    let eye = match p.eye {
        Some(e) => e.lerp(want_eye, (dt * 8.0).min(1.0)),
        None => want_eye,
    };
    p.eye = Some(eye);
    // At the kart itself, at the chase camera's look height: however the eye lags a spin, the one
    // watched stays in frame.
    let look = pose.pos + Vec3::Y * p.behind.1.y.clamp(0.0, 2.0);
    let forward = yaw * Vec3::Z;
    let (camera, fov) = if p.shot == Shot::Behind {
        (
            Transform::from_translation(eye).looking_at(look, Vec3::Y),
            view.fov,
        )
    } else {
        let mode = match p.shot {
            Shot::Low => CineMode::Low,
            Shot::Orbit => CineMode::Orbit,
            _ => CineMode::Tripod,
        };
        let cine = p.cine.get_or_insert_with(|| CineCamera::new(mode, eye));
        let subject = CineSubject {
            position: pose.pos,
            forward,
            velocity: forward * 10.0,
        };
        let shot = cine.shot(&subject, Some((eye, look)), view.fov, dt);
        (
            Transform::from_translation(shot.eye).looking_at(shot.look, Vec3::Y),
            shot.fov,
        )
    };
    drive.camera = Some(camera);
    drive.fov = fov;
    drive.engine_camera = true;
}

/// Samples the race we are in, and keeps the take once it is over for us.
#[allow(clippy::too_many_arguments)]
fn record(
    now: f32,
    races: &super::race::Races,
    remotes: &super::remote::RemoteKarts,
    names: &crate::names::NameCache,
    our_name: &str,
    view: &KartView,
    karts: &Query<(&mut Transform, &mut Visibility, &mut KartWheels), With<KartRoot>>,
    replays: &mut Replays,
) {
    let started = races.run().and_then(|r| Some((r, r.go_at?)));
    // A take of another race, or of none, is over: kept if it was not yet.
    let over = match (&replays.recording, started) {
        (Some(rec), Some((r, go))) => rec.race != r.id || rec.go != go,
        (Some(_), None) => true,
        _ => false,
    };
    let due = replays.recording.as_ref().is_some_and(|rec| {
        !rec.kept && rec.finished.is_some_and(|(at, ..)| now - at > AFTER_FINISH)
    });
    if over || due {
        if let Some(rec) = replays.recording.as_mut().filter(|rec| !rec.kept) {
            rec.kept = true;
            if let Some(replay) = rec.take_replay(now_unix()) {
                if let Some(token) = save(&replay) {
                    if hook_plays() {
                        replays.wanted = token;
                    }
                }
                replays.list_stale = true;
            }
        }
    }
    if over {
        replays.recording = None;
    }
    let Some((r, go)) = started else {
        return;
    };
    if now < go || r.course.is_empty() {
        return;
    }
    if replays.recording.is_none() {
        replays.recording = Some(Recording {
            race: r.id,
            go,
            course: r.course.to_string(),
            time_trial: r.time_trial,
            takes: HashMap::new(),
            finished: None,
            field: 1,
            kept: false,
        });
    }
    let rec = replays.recording.as_mut().expect("just made");
    if rec.kept {
        return;
    }
    let roster: Vec<String> = races
        .status()
        .map(|(_, roster, ..)| roster.to_vec())
        .unwrap_or_default();
    rec.field = roster.len().clamp(1, 255) as u8;
    let t = now - go;
    if view.active {
        rec.takes
            .entry(0)
            .or_insert_with(|| Take::new(our_name, kartcraft::model::our_kart()))
            .push(
                t,
                Pose {
                    pos: view.transform.translation,
                    rot: view.transform.rotation,
                    steer: view.steer,
                },
            );
    }
    for remote in remotes.karts.values() {
        let Some(name) = names.peek(remote.guid) else {
            continue;
        };
        if !roster.iter().any(|n| n == name) {
            continue;
        }
        let Ok((tf, vis, wheels)) = karts.get(remote.kart) else {
            continue;
        };
        if *vis == Visibility::Hidden {
            continue;
        }
        if rec.takes.len() >= MAX_TRACKS && !rec.takes.contains_key(&remote.guid) {
            continue;
        }
        rec.takes
            .entry(remote.guid)
            .or_insert_with(|| Take::new(name, remotes.racer(remote.guid)))
            .push(
                t,
                Pose {
                    pos: tf.translation,
                    rot: tf.rotation,
                    steer: wheels.steer,
                },
            );
    }
    if let (Some(ms), None) = (r.finish_ms, rec.finished) {
        let place = races
            .order()
            .iter()
            .position(|n| n == our_name)
            .map_or(0, |i| i as u8 + 1);
        rec.finished = Some((now, ms, if r.time_trial { 1 } else { place }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pose(x: f32) -> Pose {
        Pose {
            pos: Vec3::new(x, 2.0, -x),
            rot: Quat::from_rotation_y(x * 0.1),
            steer: 0.25,
        }
    }

    fn replay() -> Replay {
        Replay {
            course: "elwynn-goldshire-crystal".into(),
            at: 1_792_000_000,
            ms: 83_450,
            place: 2,
            field: 3,
            time_trial: false,
            rate: RATE,
            tracks: vec![
                Track {
                    name: "Racer".into(),
                    racer: "crash".into(),
                    us: true,
                    from: 0,
                    poses: (0..200).map(|i| pose(i as f32)).collect(),
                },
                Track {
                    name: "Asdad".into(),
                    racer: "coco".into(),
                    us: false,
                    from: 10,
                    poses: (0..150).map(|i| pose(i as f32 * 2.0)).collect(),
                },
            ],
        }
    }

    #[test]
    fn a_replay_file_round_trips() {
        let r = replay();
        let bytes = r.encode();
        let back = Replay::decode(&bytes).expect("decodes");
        assert_eq!(
            (
                back.course.as_str(),
                back.at,
                back.ms,
                back.place,
                back.field
            ),
            ("elwynn-goldshire-crystal", 1_792_000_000, 83_450, 2, 3)
        );
        assert_eq!(back.tracks.len(), 2);
        assert_eq!(
            (back.tracks[1].name.as_str(), back.tracks[1].from),
            ("Asdad", 10)
        );
        assert_eq!(back.tracks[1].poses[3].pos, r.tracks[1].poses[3].pos);
        // Anything else is refused: short, long, or another kind of file.
        assert!(Replay::decode(&bytes[..bytes.len() - 1]).is_none());
        let mut long = bytes.clone();
        long.push(0);
        assert!(Replay::decode(&long).is_none());
        assert!(Replay::decode(b"KGST").is_none());
    }

    #[test]
    fn a_track_plays_from_its_first_sample_to_its_last() {
        let r = replay();
        let theirs = &r.tracks[1];
        let step = 1.0 / f32::from(RATE);
        assert!(theirs.pose_at(step * 9.0, RATE).is_none(), "not yet seen");
        assert_eq!(theirs.pose_at(step * 10.0, RATE), Some(theirs.poses[0]));
        let mid = theirs.pose_at(step * 10.5, RATE).unwrap();
        assert!((mid.pos.x - 1.0).abs() < 1e-4);
        assert!(theirs.pose_at(step * 160.0, RATE).is_none(), "over");
        assert!((r.length() - 199.0 * step).abs() < 1e-4);
    }

    #[test]
    fn a_take_starts_on_the_sample_after_it_is_first_seen() {
        let step = 1.0 / f32::from(RATE);
        let mut take = Take::new("Asdad", "coco");
        take.push(step * 4.5, pose(0.0));
        assert_eq!((take.from, take.poses.len()), (5, 0));
        take.push(step * 6.5, pose(20.0));
        // Samples 5 and 6, a quarter and three quarters of the way.
        assert_eq!(take.poses.len(), 2);
        assert!((take.poses[0].pos.x - 5.0).abs() < 1e-3);
        assert!((take.poses[1].pos.x - 15.0).abs() < 1e-3);
    }

    #[test]
    fn a_take_without_us_or_too_short_is_not_kept() {
        let mut takes = HashMap::new();
        let mut ours = Take::new("Racer", "crash");
        for i in 0..(RATE as usize * 6) {
            ours.push(i as f32 / f32::from(RATE), pose(i as f32));
        }
        takes.insert(0, ours);
        let rec = |takes| Recording {
            race: 1,
            go: 0.0,
            course: "sprint".into(),
            time_trial: true,
            takes,
            finished: Some((0.0, 6000, 1)),
            field: 1,
            kept: false,
        };
        let kept = rec(takes).take_replay(5).expect("six seconds of us");
        assert_eq!((kept.ms, kept.place, kept.tracks.len()), (6000, 1, 1));
        assert!(rec(HashMap::new()).take_replay(5).is_none());
    }

    #[test]
    fn a_replay_is_labelled_by_its_course_and_our_result() {
        let mut r = replay();
        r.course = "elwynn-maclure-crystal".into();
        assert_eq!(
            r.label(),
            "The Maclure Vineyards & Crystal Lake: 2nd of 3, 1:23.45"
        );
        r.place = 0;
        assert_eq!(
            r.label(),
            "The Maclure Vineyards & Crystal Lake: did not finish"
        );
        r.time_trial = true;
        assert_eq!(
            r.label(),
            "The Maclure Vineyards & Crystal Lake: time trial, 1:23.45"
        );
        r.ms = 0;
        assert_eq!(
            r.label(),
            "The Maclure Vineyards & Crystal Lake: time trial, did not finish"
        );
        assert_eq!(
            [1, 2, 3, 4, 11, 12, 13, 21, 22].map(ordinal),
            ["1st", "2nd", "3rd", "4th", "11th", "12th", "13th", "21st", "22nd"]
        );
    }

    #[test]
    fn tokens_sort_by_time() {
        let mut a = replay();
        a.at = 999;
        let mut b = replay();
        b.at = 1_000_000;
        assert!(token_of(&a) < token_of(&b));
        assert_eq!(token_of(&a), "000000000999-elwynn_goldshire_crystal");
    }
}
