//! The ghost of our best run on each course, as CTR's time trial races one. From a race's GO to
//! our finish our kart's pose is sampled [`RATE`] times a second; a finish the server times faster
//! than the run on file replaces it, in `benilla-config/kart-ghosts/` ([`crate::local_state`]).
//! The next race on that course replays it from GO: a translucent kart in the run's colours
//! (`kartcraft::model::GhostKart`), interpolated between samples, solid to nothing and silent, and
//! gone at our finish or when the race ends. `KartGhost` shows it; the runs are kept either way.

use bevy::prelude::*;
use kartcraft::model::{GhostKart, KartSpawner, KartWheels};
use kartcraft::KartView;

/// Samples a second.
pub(crate) const RATE: u8 = 20;
/// The longest run kept (s): a run past it is not saved.
const MAX_SECS: u32 = 30 * 60;
const MAGIC: &[u8; 4] = b"KGST";
const VERSION: u8 = 1;

/// One sample: the kart's origin and frame (Bevy world, its tilt and powerslide look with it) and
/// its front wheels' steer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Pose {
    pub(crate) pos: Vec3,
    pub(crate) rot: Quat,
    pub(crate) steer: f32,
}

impl Pose {
    fn lerp(self, to: Pose, k: f32) -> Pose {
        Pose {
            pos: self.pos.lerp(to.pos, k),
            rot: self.rot.slerp(to.rot, k),
            steer: self.steer + (to.steer - self.steer) * k,
        }
    }
}

/// A recorded run: its time, whose kart it drove, and its poses [`Ghost::rate`] a second from GO.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Ghost {
    /// The finish time, ms, as the server timed it.
    pub(crate) ms: u32,
    pub(crate) racer: String,
    pub(crate) rate: u8,
    pub(crate) poses: Vec<Pose>,
}

impl Ghost {
    /// The file: `KGST`, a version byte, the rate, the time (u32), the racer (a length byte and
    /// ASCII), the sample count (u32), then per sample its position (3 × f32), its rotation
    /// (4 × i16, a unit quaternion's x, y, z, w over 32767) and its steer (i8 over 127). Little
    /// endian.
    pub(crate) fn encode(&self) -> Vec<u8> {
        let racer = &self.racer.as_bytes()[..self.racer.len().min(255)];
        let mut out = Vec::with_capacity(16 + racer.len() + self.poses.len() * 21);
        out.extend_from_slice(MAGIC);
        out.push(VERSION);
        out.push(self.rate);
        out.extend_from_slice(&self.ms.to_le_bytes());
        out.push(racer.len() as u8);
        out.extend_from_slice(racer);
        out.extend_from_slice(&(self.poses.len() as u32).to_le_bytes());
        for p in &self.poses {
            for v in p.pos.to_array() {
                out.extend_from_slice(&v.to_le_bytes());
            }
            for v in p.rot.normalize().to_array() {
                let q = (v.clamp(-1.0, 1.0) * 32767.0).round() as i16;
                out.extend_from_slice(&q.to_le_bytes());
            }
            out.push(((p.steer.clamp(-1.0, 1.0) * 127.0).round() as i8) as u8);
        }
        out
    }

    /// Reads [`Ghost::encode`]'s file; `None` for anything else.
    pub(crate) fn decode(bytes: &[u8]) -> Option<Ghost> {
        let mut r = Reader(bytes);
        if r.take(4)? != MAGIC || r.u8()? != VERSION {
            return None;
        }
        let rate = r.u8()?;
        let ms = r.u32()?;
        let len = usize::from(r.u8()?);
        let racer = std::str::from_utf8(r.take(len)?).ok()?.to_string();
        let count = r.u32()? as usize;
        if rate == 0 || count > MAX_SECS as usize * usize::from(rate) || r.0.len() != count * 21 {
            return None;
        }
        let mut poses = Vec::with_capacity(count);
        for _ in 0..count {
            let pos = Vec3::new(r.f32()?, r.f32()?, r.f32()?);
            let mut q = [0.0; 4];
            for v in &mut q {
                *v = f32::from(r.i16()?) / 32767.0;
            }
            let rot = Quat::from_array(q).normalize();
            let steer = f32::from(r.u8()? as i8) / 127.0;
            if !pos.is_finite() || !rot.is_finite() {
                return None;
            }
            poses.push(Pose { pos, rot, steer });
        }
        Some(Ghost {
            ms,
            racer,
            rate,
            poses,
        })
    }

    /// Where the run was `t` seconds after GO, between the samples around it; the first pose
    /// before GO, `None` once the run is over.
    pub(crate) fn pose_at(&self, t: f32) -> Option<Pose> {
        let last = self.poses.len().checked_sub(1)?;
        let f = t.max(0.0) * f32::from(self.rate);
        if f > last as f32 {
            return None;
        }
        let i = (f.floor() as usize).min(last);
        if i == last {
            return Some(self.poses[last]);
        }
        Some(self.poses[i].lerp(self.poses[i + 1], f - i as f32))
    }
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.0.len() < n {
            return None;
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Some(head)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn i16(&mut self) -> Option<i16> {
        Some(i16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn f32(&mut self) -> Option<f32> {
        Some(f32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
}

/// Whether a finish in `ms` replaces the run on file (`stored`, its time): only a faster one does.
pub(crate) fn replaces(stored: Option<u32>, ms: u32) -> bool {
    ms > 0 && stored.is_none_or(|s| ms < s)
}

/// A run being recorded: the race and GO it is of, and its samples so far.
struct Recording {
    race: u32,
    go: f32,
    racer: String,
    poses: Vec<Pose>,
    /// The last pose given and when (s after GO), to place the samples between frames.
    last: Option<(f32, Pose)>,
    /// The kart left the course mid-run, or the run outlasted [`MAX_SECS`]: it is not kept.
    broken: bool,
    /// Its finish has been dealt with.
    finished: bool,
}

impl Recording {
    fn new(race: u32, go: f32, racer: &str) -> Self {
        Self {
            race,
            go,
            racer: racer.to_string(),
            poses: Vec::new(),
            last: None,
            broken: false,
            finished: false,
        }
    }

    /// The kart's pose `t` seconds after GO: every sample time since the last frame is filled,
    /// between that frame's pose and this one.
    fn push(&mut self, t: f32, pose: Pose) {
        let (t0, p0) = self.last.unwrap_or((t, pose));
        loop {
            if self.poses.len() >= MAX_SECS as usize * usize::from(RATE) {
                self.broken = true;
                break;
            }
            let at = self.poses.len() as f32 / f32::from(RATE);
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

/// The ghost of the race we are in, and the run we are recording.
#[derive(Resource, Default)]
pub(crate) struct Ghosts {
    /// The run on file for the race in hand: its race id and course, and what the file held.
    loaded: Option<(u32, String, Option<Ghost>)>,
    recording: Option<Recording>,
    /// The ghost kart drawn, and whose kart it is.
    kart: Option<(Entity, String)>,
    /// Its wheels' roll, and where it was last frame.
    spin: f32,
    last_pos: Option<Vec3>,
}

fn read(course: &str) -> Option<Ghost> {
    let path = crate::local_state::kart_ghost_path(course)?;
    let bytes = std::fs::read(&path).ok()?;
    let ghost = Ghost::decode(&bytes);
    if ghost.is_none() {
        warn!("kart ghost: {} is not a ghost file", path.display());
    }
    ghost
}

/// Saves `ghost` as `course`'s run when it beats the one on file; whether it did.
fn save(course: &str, ghost: &Ghost) -> bool {
    let Some(path) = crate::local_state::kart_ghost_path(course) else {
        return false;
    };
    let stored = std::fs::read(&path)
        .ok()
        .and_then(|b| Ghost::decode(&b))
        .map(|g| g.ms);
    if !replaces(stored, ghost.ms) {
        return false;
    }
    match crate::local_state::write_atomic_bytes(&path, &ghost.encode()) {
        Ok(()) => true,
        Err(e) => {
            warn!("kart ghost: {}: {e}", path.display());
            false
        }
    }
}

/// Records the race's run, saves a faster one, and replays the run on file as a ghost kart.
#[allow(clippy::too_many_arguments)]
pub(crate) fn update(
    time: Res<Time>,
    races: Res<super::race::Races>,
    drive: Res<super::KartDrive>,
    view: Res<KartView>,
    settings: Res<super::settings::KartSettings>,
    mut ghosts: ResMut<Ghosts>,
    mut spawner: KartSpawner,
    mut karts: Query<(&mut Transform, &mut Visibility, &mut KartWheels), With<GhostKart>>,
) {
    let now = time.elapsed_secs();
    let ghosts = &mut *ghosts;
    let run = races.run();

    // The run on file for this race's course, read once the course is known.
    match run.filter(|r| !r.course.is_empty()) {
        Some(r) => {
            if !ghosts
                .loaded
                .as_ref()
                .is_some_and(|(id, c, _)| *id == r.id && c == r.course)
            {
                let ghost = read(r.course);
                if let Some(g) = &ghost {
                    info!(
                        "kart ghost: {} has a run of {} ms in {}'s kart",
                        r.course, g.ms, g.racer
                    );
                }
                ghosts.loaded = Some((r.id, r.course.to_string(), ghost));
            }
        }
        None => ghosts.loaded = None,
    }

    // The recording: from GO, while we drive, to our finish.
    match run.and_then(|r| Some((r, r.go_at?))) {
        Some((r, go)) => {
            if !ghosts
                .recording
                .as_ref()
                .is_some_and(|rec| rec.race == r.id && rec.go == go)
            {
                ghosts.recording = Some(Recording::new(r.id, go, kartcraft::model::our_kart()));
            }
            let rec = ghosts.recording.as_mut().expect("just made");
            if now >= go && !r.done {
                if drive.active && view.active {
                    let pose = Pose {
                        pos: view.transform.translation,
                        rot: view.transform.rotation,
                        steer: view.steer,
                    };
                    rec.push(now - go, pose);
                } else {
                    rec.broken = true;
                }
            }
            if let (Some(ms), false) = (r.finish_ms, rec.finished) {
                rec.finished = true;
                if !rec.broken && !rec.poses.is_empty() && !r.course.is_empty() {
                    let ghost = Ghost {
                        ms,
                        racer: rec.racer.clone(),
                        rate: RATE,
                        poses: std::mem::take(&mut rec.poses),
                    };
                    if save(r.course, &ghost) {
                        info!(
                            "kart ghost: a new best on {} ({ms} ms, {} samples) kept",
                            r.course,
                            ghost.poses.len()
                        );
                    }
                }
            }
        }
        None => ghosts.recording = None,
    }

    // The ghost: from the countdown to our finish, while we drive and it is wanted.
    let pose = run
        .filter(|r| settings.ghost && drive.active && !r.done)
        .and_then(|r| {
            let go = r.go_at?;
            let ghost = ghosts.loaded.as_ref()?.2.as_ref()?;
            Some((ghost.racer.clone(), ghost.pose_at(now - go)?))
        });
    let Some((racer, pose)) = pose else {
        if let Some((kart, _)) = ghosts.kart.take() {
            spawner.despawn(kart);
        }
        ghosts.last_pos = None;
        return;
    };
    if ghosts.kart.as_ref().is_none_or(|(_, r)| *r != racer) {
        if let Some((kart, _)) = ghosts.kart.take() {
            spawner.despawn(kart);
        }
        let Some(kart) = spawner.spawn_ghost(&racer) else {
            return;
        };
        ghosts.kart = Some((kart, racer));
        ghosts.last_pos = None;
    }
    let Some((kart, _)) = ghosts.kart else {
        return;
    };
    // The wheels roll with the ground it covers along its heading (the model's +Z).
    let moved = ghosts
        .last_pos
        .replace(pose.pos)
        .map_or(0.0, |p| (pose.pos - p).dot(pose.rot * Vec3::Z));
    let radius = super::remote::WHEEL_RADIUS * kartcraft::kart_scale() / kartcraft::METERS_PER_YARD;
    ghosts.spin = (ghosts.spin + moved / radius).rem_euclid(std::f32::consts::TAU);
    if let Ok((mut t, mut vis, mut wheels)) = karts.get_mut(kart) {
        *t = Transform {
            translation: pose.pos,
            rotation: pose.rot,
            scale: Vec3::splat(kartcraft::model_scale()),
        };
        *vis = Visibility::Visible;
        *wheels = KartWheels {
            steer: pose.steer,
            spin: ghosts.spin,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pose(x: f32, yaw: f32, steer: f32) -> Pose {
        Pose {
            pos: Vec3::new(x, 1.0, -x),
            rot: Quat::from_rotation_y(yaw),
            steer,
        }
    }

    fn ghost(ms: u32, poses: Vec<Pose>) -> Ghost {
        Ghost {
            ms,
            racer: "coco".into(),
            rate: RATE,
            poses,
        }
    }

    #[test]
    fn a_ghost_file_round_trips() {
        let g = ghost(
            62_400,
            vec![
                pose(-8930.5, 0.0, 0.0),
                pose(-8929.25, 1.2, -1.0),
                Pose {
                    pos: Vec3::new(3.0, 40.5, 7.0),
                    rot: Quat::from_euler(EulerRot::YXZ, 2.5, 0.3, -0.2),
                    steer: 0.5,
                },
            ],
        );
        let bytes = g.encode();
        assert_eq!(bytes.len(), 4 + 1 + 1 + 4 + 1 + 4 + 4 + 3 * 21);
        let back = Ghost::decode(&bytes).expect("decodes");
        assert_eq!(
            (back.ms, back.racer.as_str(), back.rate),
            (62_400, "coco", RATE)
        );
        assert_eq!(back.poses.len(), 3);
        for (a, b) in g.poses.iter().zip(&back.poses) {
            assert_eq!(a.pos, b.pos, "positions are kept exactly");
            assert!(a.rot.angle_between(b.rot) < 1e-3, "{:?} {:?}", a.rot, b.rot);
            assert!((a.steer - b.steer).abs() < 0.01);
        }
        // Anything else is refused: a short file, a wrong magic or version, trailing bytes.
        assert!(Ghost::decode(&bytes[..bytes.len() - 1]).is_none());
        assert!(Ghost::decode(b"nope").is_none());
        let mut v2 = bytes.clone();
        v2[4] = 2;
        assert!(Ghost::decode(&v2).is_none());
        let mut long = bytes.clone();
        long.push(0);
        assert!(Ghost::decode(&long).is_none());
        assert!(Ghost::decode(&ghost(1, Vec::new()).encode()).is_some());
    }

    #[test]
    fn only_a_faster_finish_replaces_the_run_on_file() {
        assert!(
            replaces(None, 70_000),
            "nothing on file: any finish is kept"
        );
        assert!(replaces(Some(70_000), 69_999));
        assert!(!replaces(Some(70_000), 70_000), "a tie keeps the old run");
        assert!(!replaces(Some(70_000), 80_000));
        assert!(!replaces(None, 0), "an untimed finish is not a run");
    }

    #[test]
    fn a_faster_run_replaces_the_file_and_a_slower_one_does_not() {
        use crate::local_state::test_env::{EnvGuard, ENV_LOCK};
        let _l = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = std::env::temp_dir().join(format!("benilla-ghost-{}", std::process::id()));
        std::fs::remove_dir_all(&home).ok();
        let _c = EnvGuard::unset("WOW_CAPTURE");
        let _h = EnvGuard::set("BENILLA_HOME", home.to_str().unwrap());
        let course = "elwynn-goldshire-crystal";
        assert!(read(course).is_none());
        assert!(save(course, &ghost(70_000, vec![pose(0.0, 0.0, 0.0)])));
        assert!(!save(course, &ghost(75_000, vec![pose(1.0, 0.0, 0.0)])));
        assert_eq!(
            read(course).unwrap().ms,
            70_000,
            "the slower run was not kept"
        );
        assert!(save(course, &ghost(65_000, vec![pose(2.0, 0.0, 0.0)])));
        let kept = read(course).unwrap();
        assert_eq!((kept.ms, kept.poses[0].pos.x), (65_000, 2.0));
        let path = crate::local_state::kart_ghost_path(course).unwrap();
        assert!(
            path.starts_with(&home) && path.ends_with("kart-ghosts/elwynn_goldshire_crystal.bin")
        );
        std::fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn a_ghost_interpolates_between_its_samples() {
        let g = ghost(
            1000,
            vec![
                pose(0.0, 0.0, 0.0),
                pose(4.0, 1.0, 1.0),
                pose(6.0, 1.0, 1.0),
            ],
        );
        let step = 1.0 / f32::from(RATE);
        // Before GO it waits on the line.
        assert_eq!(g.pose_at(-2.0), Some(g.poses[0]));
        // A quarter of the way to the second sample.
        let p = g.pose_at(step * 0.25).unwrap();
        assert!((p.pos.x - 1.0).abs() < 1e-4);
        assert!((p.steer - 0.25).abs() < 1e-4);
        assert!((p.rot.to_euler(EulerRot::YXZ).0 - 0.25).abs() < 1e-4);
        // Halfway along the second leg, and on the last sample exactly.
        assert!((g.pose_at(step * 1.5).unwrap().pos.x - 5.0).abs() < 1e-4);
        assert_eq!(g.pose_at(step * 2.0), Some(g.poses[2]));
        // Past the end the run is over.
        assert_eq!(g.pose_at(step * 2.0 + 0.01), None);
        assert_eq!(ghost(1, Vec::new()).pose_at(0.0), None);
    }

    #[test]
    fn a_recording_samples_at_its_rate_between_frames() {
        let mut rec = Recording::new(1, 10.0, "crash");
        let step = 1.0 / f32::from(RATE);
        // A first frame a little after GO, then one that spans two and a half sample times.
        rec.push(0.01, pose(0.0, 0.0, 0.0));
        assert_eq!(rec.poses.len(), 1);
        rec.push(0.01 + step * 2.5, pose(25.0, 0.0, 0.0));
        assert_eq!(rec.poses.len(), 3, "samples at 0, 1 and 2 steps");
        // The sample at one step lies 0.4 of the way along that frame's 2.5 steps.
        let x = rec.poses[1].pos.x;
        assert!(
            (x - (step - 0.01) / (step * 2.5) * 25.0).abs() < 1e-3,
            "{x}"
        );
        // A frame inside the same sample time adds nothing.
        rec.push(0.01 + step * 2.6, pose(26.0, 0.0, 0.0));
        assert_eq!(rec.poses.len(), 3);
        assert!(!rec.broken);
    }
}
