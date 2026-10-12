//! CTR's in-race chase camera, `CAM_FollowDriver_Normal` (`CAM.c:1047-1546`), ticked with the
//! kart at 30 Hz in level units (`docs/ctr-camera-spec.md`). Its yaw is the kart's physics
//! heading plus the powerslide's swing, with no lag: a boom pitched 8.8° up trails the kart,
//! longer with speed (179 to 224 lu near, 224 to 288 far) and pushed out up to 96 lu by every
//! turbo; it looks at a point just over and ahead of the kart along the drawn nose, banks by half
//! the ground's lean, and keeps at least 64 lu (far: 96) over the ground under it. Look-back (R2)
//! turns both yaws half a turn. Vertical FOV 73.74°, CTR's 4:3 frustum kept vertically for a wide
//! screen.
//!
//! Every one of those numbers is a [`CameraTuning`] field the player sets while driving:
//! [`CameraTuning::CTR`] is CTR's camera exactly, [`CameraTuning::KARTCRAFT`] (the default) the
//! one fitted to WoW's world, its deviations named on it.

use bevy::math::{Quat, Vec3};
use ctr_kart::{CameraInput, Collision};

/// One of CTR's 1P 4:3 zoom tables (`NearCam4x3`, `FarCam4x3`, `zGlobal_DATA.c:983-1008`): the
/// boom from the slowest speed to the fastest, the look-at's height and the eye's height base,
/// which is also its least clearance over the ground (lu). The far one is also how the far
/// camera stands to any tuning's near one ([`CameraTuning`]).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Zoom {
    dist_min: f32,
    dist_max: f32,
    look_h: f32,
    vert: f32,
}

const NEAR: Zoom = Zoom {
    dist_min: 179.0,
    dist_max: 224.0,
    look_h: 112.0,
    vert: 64.0,
};
const FAR: Zoom = Zoom {
    dist_min: 224.0,
    dist_max: 288.0,
    look_h: 96.0,
    vert: 96.0,
};
/// The boom's pitch (4096ths, upward).
const PITCH: i32 = 100;
/// Look-back's half turn (4096ths, `CAM.c:1073-1098`).
const REVERSE: i32 = 0x800;
/// CTR's vertical field of view, radians (73.74°).
pub const FOV_Y: f32 = 1.2870;
/// Where the ground's grade is read along the heading, from behind the kart to ahead of it (lu).
const GRADE_SAMPLES: [f32; 17] = [
    -192.0, -160.0, -128.0, -96.0, -64.0, -32.0, 0.0, 32.0, 64.0, 96.0, 128.0, 160.0, 192.0, 224.0,
    256.0, 288.0, 320.0,
];
/// The steepest grade heeded (rise per run, about 37°).
const GRADE_MAX: f32 = 0.75;
/// The tilt's pivot over the kart's origin (lu): the kart's middle, which keeps its place on the
/// screen however the rig tilts.
const PIVOT_H: f32 = 32.0;
/// How far the look-at moves toward or away from the kart a tick (lu, `CAM.c:1251-1271`).
const AHEAD_STEP: f32 = 8.0;
/// The turbo push-back grows 8 lu a tick and comes back 2 (lu·256, `CAM.c:1159-1196`).
const TURBO_GROW: i32 = 2048;
const TURBO_DECAY: i32 = 512;

/// Everything that shapes the chase camera, read every tick, so a change applies while driving.
/// Lengths in lu, angles in degrees, smoothing as a time constant in seconds (0: none).
///
/// The far camera stands to the near one as CTR's `FarCam4x3` to its `NearCam4x3`: its boom runs
/// from the near one's racing distance at a crawl to [`Self::far_extra`] beyond it at speed (CTR
/// 224 to 288), its lift and clearance are half as much again (64 to 96), and it looks 6/7 as
/// high (112 to 96).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraTuning {
    /// The boom at racing speed (CTR `distMax` 224).
    pub distance: f32,
    /// The boom at a crawl (CTR `distMin` 179); it stretches to [`Self::distance`] by 600 lu/s.
    pub slow_distance: f32,
    /// The eye's lift over the boom's end (CTR `vertDistance` 64).
    pub height: f32,
    /// The look-at's height over the kart's origin (CTR `angle[2]` 112).
    pub look_height: f32,
    /// The boom's pitch, degrees up (CTR 100/4096 of a turn, 8.79°).
    pub pitch: f32,
    /// How far ahead along the drawn nose the look-at sits (CTR `damagePitchOffset` 32).
    pub look_ahead: f32,
    /// The eye's least height over the ground under it (CTR `vertDistance` 64).
    pub clearance: f32,
    /// The far camera's boom beyond the near one's at speed (CTR 288 − 224).
    pub far_extra: f32,
    /// The vertical field of view, degrees (CTR 73.74).
    pub fov: f32,
    /// How far below the shot's look-at the drawn view aims (CTR none).
    pub aim_drop: f32,
    /// The share of the powerslide's swing (`rotCurr.w`, up to 33.75° out of the slide) the eye
    /// turns by (CTR 1).
    pub swing: f32,
    /// The eye aims along the kart's travel over the ground rather than its body (CTR: its body,
    /// heading + swing).
    pub travel: bool,
    /// Following the travel, the share of the body's turn and swing off it the eye takes on top
    /// (1 is the body's again).
    pub follow: f32,
    /// The yaw's easing after its aim, a time constant in seconds (CTR 0: rigid).
    pub turn_smoothing: f32,
    /// The boom turned round the kart, degrees counter-clockwise seen from above (CTR 0: behind).
    pub yaw_offset: f32,
    /// The whole rig moved to the right of the way the kart faces (CTR 0).
    pub side_offset: f32,
    /// The share of the ground's grade the rig tilts with (CTR 0: world-up offsets, spec 2.5).
    pub tilt: f32,
    /// Each of the tilt's two filter stages' time constant, seconds.
    pub tilt_smoothing: f32,
    /// The screen's roll as a share of the ground's lean under the kart (CTR `angle[0]` 0.5),
    /// the rig's slide toward the lean going with it.
    pub bank: f32,
    /// The turbo push-back's reach (CTR 96).
    pub turbo_push: f32,
    /// The eye comes in short of a wall between it and the kart (CTR has no wall test).
    pub wall_pull: bool,
}

impl CameraTuning {
    /// CTR's own chase camera (`docs/ctr-camera-spec.md`): the eye rigid on the heading plus the
    /// powerslide's swing, world-up offsets on a hill, no wall test, its 73.74° frustum.
    pub const CTR: CameraTuning = CameraTuning {
        distance: NEAR.dist_max,
        slow_distance: NEAR.dist_min,
        height: NEAR.vert,
        look_height: NEAR.look_h,
        pitch: PITCH as f32 * 360.0 / 4096.0,
        look_ahead: 32.0,
        clearance: NEAR.vert,
        far_extra: FAR.dist_max - NEAR.dist_max,
        fov: 73.74,
        aim_drop: 0.0,
        swing: 1.0,
        travel: false,
        follow: 0.35,
        turn_smoothing: 0.0,
        yaw_offset: 0.0,
        side_offset: 0.0,
        tilt: 0.0,
        tilt_smoothing: 0.316,
        bank: 0.5,
        turbo_push: (0x6000 >> 8) as f32,
        wall_pull: false,
    };

    /// Kartcraft's: CTR's camera, turning as CTR's does, over WoW's world. Deviations: the rig
    /// tilts with the ground's grade (its two stages 0.316 s each, 63% of a new grade in about
    /// 0.7 s), as CTR's world-up offsets fit its gentle tracks but on a WoW hill the road ahead
    /// fills the screen or drops out of it; the eye comes in short of a wall, as WoW's pillars
    /// and trees stand behind the kart where CTR's tracks keep walls away; and the view aims 70 lu
    /// lower at the client's own 45° field of view, so the kart clears WoW's action bars along
    /// the bottom of the screen, where CTR puts it.
    pub const KARTCRAFT: CameraTuning = CameraTuning {
        fov: 45.0,
        aim_drop: 70.0,
        tilt: 1.0,
        wall_pull: true,
        ..CameraTuning::CTR
    };

    /// Kartcraft's with the eye on the kart's travel: 35% of the body's turn and swing on top,
    /// easing after it (0.116 s, a quarter of the gap a tick), so through a slide it looks mostly
    /// where the kart is going rather than where its body points.
    pub const SMOOTH: CameraTuning = CameraTuning {
        travel: true,
        follow: 0.35,
        turn_smoothing: 0.116,
        ..CameraTuning::KARTCRAFT
    };

    fn rig(&self, far: bool) -> Rig {
        if far {
            Rig {
                dist_min: self.distance,
                dist_max: self.distance + self.far_extra,
                look_h: self.look_height * FAR.look_h / NEAR.look_h,
                vert: self.height * FAR.vert / NEAR.vert,
                clearance: self.clearance * FAR.vert / NEAR.vert,
            }
        } else {
            Rig {
                dist_min: self.slow_distance,
                dist_max: self.distance,
                look_h: self.look_height,
                vert: self.height,
                clearance: self.clearance,
            }
        }
    }

    /// The vertical field of view, radians.
    pub fn fov_radians(&self) -> f32 {
        self.fov.to_radians()
    }
}

impl Default for CameraTuning {
    fn default() -> Self {
        Self::KARTCRAFT
    }
}

/// One tick's zoom from a [`CameraTuning`] (lu): the boom's reach from a crawl to speed, the
/// look-at's height, the eye's lift and its least clearance.
struct Rig {
    dist_min: f32,
    dist_max: f32,
    look_h: f32,
    vert: f32,
    clearance: f32,
}

/// The share of the gap a filter with time constant `tau` (s) closes in one 30 Hz tick; all of
/// it with none.
fn share(tau: f32) -> f32 {
    if tau <= 0.0 {
        1.0
    } else {
        1.0 - (-ctr_kart::TICK / tau).exp()
    }
}

/// An angle difference in 4096ths into −2048..2048.
fn wrapf(a: f32) -> f32 {
    (a + 2048.0).rem_euclid(4096.0) - 2048.0
}

fn rad(angle: i32) -> f32 {
    angle as f32 * std::f32::consts::TAU / 4096.0
}

/// CTR's forward for a heading: `(sin a, 0, cos a)`.
fn fwd(angle: i32) -> Vec3 {
    let a = rad(angle);
    Vec3::new(a.sin(), 0.0, a.cos())
}

/// The kart's right for a heading (X is its left).
fn right(angle: i32) -> Vec3 {
    let a = rad(angle);
    Vec3::new(-a.cos(), 0.0, a.sin())
}

/// One camera pose, lu: the eye, what it looks at, and its roll (radians). `cut`: the shot does
/// not follow on from the last (look-back turned on or off), so nothing is drawn between them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shot {
    pub eye: Vec3,
    pub look: Vec3,
    pub roll: f32,
    pub cut: bool,
}

#[derive(Default)]
pub struct ChaseCamera {
    /// What shapes the camera, read every tick.
    pub tuning: CameraTuning,
    /// CTR's L2 toggle (`nearOrFar`): the far zoom table.
    pub far: bool,
    /// CTR's R2 held: the camera looks back.
    pub look_back: bool,
    looked_back: bool,
    /// `cameraMoveSpeed`: |speedApprox| filtered.
    speed: i32,
    /// The kart's roll filtered.
    roll: i32,
    /// `damagePitchOffset`: how far ahead the look-at sits (−32 normally).
    ahead: f32,
    /// The turbo push-back (lu·256) and its timer (CTR-ms).
    turbo: i32,
    turbo_timer: i32,
    /// The ground clamp's height re-blend.
    height: f32,
    start_height: f32,
    blend: i32,
    started: bool,
    /// The ground's grade along the heading as an angle (radians), through two filter stages.
    tilt: [f32; 2],
    /// The eye's yaw (4096ths), easing after its aim.
    yaw: f32,
    /// Last tick's eye and look-at (`cameraPos`, `lookAtPos`), before the wall pull.
    last: Option<(Vec3, Vec3)>,
    /// Through a blast (`BlastedLerp.unkOffset`): the look-at's height over the eye and the
    /// eye's over the ground, held from the blast's first frame until the kart lands.
    blast: Option<(f32, f32)>,
    /// After it (`BlastedLerp.desiredPos`, `desiredRot`, `framesRemaining`): how far the eye and
    /// the look-at were from where they now belong, given back over 8 frames.
    blast_ease: (Vec3, Vec3, i32),
}

/// The ground's grade along `heading` around the kart at `at` (rise per run): a least-squares
/// line through the ground at [`GRADE_SAMPLES`], so no one bump or hollow swings it. `None` with
/// fewer than three samples on ground.
fn grade(world: &Collision, at: Vec3, heading: i32) -> Option<f32> {
    let f = fwd(heading);
    let mut points = [(0.0, 0.0); GRADE_SAMPLES.len()];
    let mut n = 0;
    for s in GRADE_SAMPLES {
        // From over the steepest heeded grade at that distance, down twice as far.
        let over = 128.0 + GRADE_MAX * s.abs();
        if let Some(hit) = world.ground(at + f * s + Vec3::Y * over, 2.0 * over) {
            points[n] = (s, hit.point.y - at.y);
            n += 1;
        }
    }
    if n < 3 {
        return None;
    }
    let points = &points[..n];
    let ms = points.iter().map(|p| p.0).sum::<f32>() / n as f32;
    let my = points.iter().map(|p| p.1).sum::<f32>() / n as f32;
    let (mut sy, mut ss) = (0.0, 0.0);
    for (s, y) in points {
        sy += (s - ms) * (y - my);
        ss += (s - ms) * (s - ms);
    }
    (ss > 0.0).then(|| (sy / ss).clamp(-GRADE_MAX, GRADE_MAX))
}

impl ChaseCamera {
    /// A fresh camera (a new drive) with this one's tuning.
    pub fn reset(&mut self) {
        *self = ChaseCamera {
            tuning: self.tuning,
            ..Default::default()
        };
    }

    /// A blast's camera (`CAM.c:1312-1392`, spec 3.2), for every tuning: from the blast's first
    /// frame the eye may not rise above the height over the ground it had then, so it does not
    /// follow the tumble's flight, until the kart is out of the tumble and on the ground; then
    /// how far the eye and look-at were from where they now belong is given back over 8 frames
    /// (8/8 first, so the first frame holds still).
    fn blasted(&mut self, k: &CameraInput, eye: &mut Vec3, look: &mut Vec3) {
        let (last_eye, last_look) = self.last.unwrap_or((*eye, *look));
        if !k.blasted && k.grounded && self.blast.take().is_some() {
            self.blast_ease = (last_eye - *eye, last_look - *look, 8);
        }
        if k.blasted || self.blast.is_some() {
            let (_, over) = *self
                .blast
                .get_or_insert((last_look.y - last_eye.y, last_eye.y - k.ground));
            let ceiling = k.ground + over;
            if last_eye.y < eye.y && ceiling < eye.y {
                eye.y = ceiling;
            }
            return;
        }
        let (d_eye, d_look, n) = &mut self.blast_ease;
        if *n > 0 {
            let share = *n as f32 / 8.0;
            *eye += *d_eye * share;
            *look += *d_look * share;
            *n -= 1;
        }
    }

    /// One 30 Hz frame for `k`, over `world`'s ground.
    pub fn tick(&mut self, k: &CameraInput, world: &Collision) -> Shot {
        let tuning = self.tuning;
        let zoom = tuning.rig(self.far);
        // 3.4: look-back turning on or off is DIRECTION_CHANGED, an instant cut that restarts
        // the speed filter; so is the kart put somewhere else (Aku Aku's respawn,
        // `VehStuckProc.c:436`, `:1320`).
        let cut = self.started && (self.look_back != self.looked_back || k.cut);
        self.looked_back = self.look_back;

        // 2.2: the boom lengthens with the filtered speed.
        let x = k.speed_approx.abs();
        if !self.started || cut {
            self.speed = x;
        } else if x < self.speed {
            self.speed = (240 * self.speed + x) >> 8;
        } else {
            self.speed = (240 * self.speed + 16 * x) >> 8;
        }
        if !self.started {
            self.ahead = -tuning.look_ahead;
        }
        let t = ((self.speed - 1024) as f32 / 4096.0).clamp(0.0, 1.0);
        let mut d = zoom.dist_min + t * (zoom.dist_max - zoom.dist_min);

        // 3.3: every turbo pushes the boom out.
        if k.turbo {
            self.turbo_timer = self.turbo_timer.max(k.fire_speed_cap >> 4);
        }
        let cap = (tuning.turbo_push.max(0.0) * 256.0) as i32;
        if self.turbo_timer == 0 {
            self.turbo = (self.turbo - TURBO_DECAY).max(0);
        } else {
            self.turbo = (self.turbo + TURBO_GROW).min(cap);
            self.turbo_timer = (self.turbo_timer - 32).max(0);
        }
        self.turbo = self.turbo.min(cap);
        d += (self.turbo >> 8) as f32;

        // 2.3: the eye trails the heading plus the powerslide's swing (and look-back's half turn).
        // CTR's eye is rigid on that. A tuning may take only `swing` of the swing, aim along the
        // kart's travel over the ground with `follow` of the body's turn and swing on top, ease
        // after the aim, and turn the boom round the kart by `yaw_offset`. Look-back's half turn
        // stays an instant cut.
        let body = k.heading as f32 + k.spin as f32 * tuning.swing;
        let aim = match k.travel.filter(|_| tuning.travel) {
            Some(travel) => travel as f32 + wrapf(body - travel as f32) * tuning.follow,
            None => body,
        };
        self.yaw = if self.started {
            self.yaw + wrapf(aim - self.yaw) * share(tuning.turn_smoothing)
        } else {
            aim
        };
        let rev = if self.look_back { REVERSE } else { 0 };
        let offset = tuning.yaw_offset * 4096.0 / 360.0;
        let a_eye = (self.yaw + offset).round() as i32 + rev;
        let (f, r) = (fwd(a_eye), right(a_eye));
        let pitch = tuning.pitch.to_radians();
        let (sp, cp) = (pitch.sin(), pitch.cos());
        let roll2 = rad(2 * self.roll);
        // The rig slides toward the lean as far as it banks: CTR's 64 lu at its half share.
        let slide = 64.0 * tuning.bank / CameraTuning::CTR.bank;
        // And the whole rig stands `side_offset` to the right of the way the kart faces.
        let side = r * (slide * roll2.sin())
            + f * (64.0 * sp * roll2.cos())
            + right(self.yaw.round() as i32) * tuning.side_offset;
        let base = Vec3::new(k.position.x + side.x, k.position.y, k.position.z + side.z);
        let mut eye = base - f * (cp * d) + Vec3::Y * (sp * d + zoom.vert);
        // Lowered by Aku Aku, the eye keeps the height of a kart 0x40 lu over the ground
        // (`CAMERA_FLAG_MASK_GRAB`, `CAM.c:1221-1224`); the look-at follows the kart.
        if let Some(h) = k.mask_grab {
            eye.y += h - k.position.y;
        }

        // 2.4: it looks just ahead of the drawn nose, over the kart.
        self.ahead = if k.spinning {
            (self.ahead + AHEAD_STEP).min(0.0)
        } else {
            (self.ahead - AHEAD_STEP).max(-tuning.look_ahead)
        };
        let g = fwd(a_eye + k.turn_angle);
        let ahead = -self.ahead;
        let mut look = base + g * (cp * ahead) + Vec3::Y * (zoom.look_h - sp * ahead);

        // CTR's camera keeps world-up offsets (spec 2.5), fine on its gentle tracks; on a WoW
        // hill the road ahead fills the screen or drops out of it. A tuning may tilt the whole
        // rig about the kart's middle with `tilt` of the ground's grade along the heading, read
        // over several samples, clamped and filtered, so the camera sits over a slope as it sits
        // over flat road and the kart keeps its place on the screen.
        let target = if tuning.tilt > 0.0 {
            grade(world, k.position, k.heading).map_or(self.tilt[1], |g| g.atan() * tuning.tilt)
        } else {
            0.0
        };
        if !self.started {
            self.tilt = [target; 2];
        }
        let follow = share(tuning.tilt_smoothing);
        self.tilt[0] += (target - self.tilt[0]) * follow;
        self.tilt[1] += (self.tilt[0] - self.tilt[1]) * follow;
        // About the heading's right, a positive angle lifts its forward.
        let tilt = Quat::from_axis_angle(right(k.heading), self.tilt[1]);
        let pivot = k.position + Vec3::Y * PIVOT_H;
        eye = pivot + tilt * (eye - pivot);
        look = pivot + tilt * (look - pivot);

        // 2.5: the ground's lean, filtered (after use, as the game does).
        self.roll = (224 * self.roll + 32 * k.roll) >> 8;

        self.blasted(k, &mut eye, &mut look);

        // 5: never closer than vertDistance over the ground under the eye.
        match world.ground(eye + Vec3::Y * 256.0, 2304.0) {
            Some(hit) => {
                eye.y = eye.y.max(hit.point.y + zoom.clearance);
                if self.blend > 0 {
                    let n = self.blend as f32;
                    eye.y = ((8.0 - n) * eye.y + n * (k.position.y + self.start_height)) / 8.0;
                    self.blend -= 1;
                }
                self.height = eye.y - k.position.y;
            }
            None => {
                self.blend = 8;
                if self.started && !cut && eye.y < k.position.y + self.height {
                    eye.y = k.position.y + self.height;
                    self.start_height = self.height;
                } else {
                    self.start_height = eye.y - k.position.y;
                    self.height = self.start_height;
                }
            }
        }
        // Through a blast the look-at keeps its height over the eye (`CAM.c:1447-1450`).
        if let Some((look_over, _)) = self.blast {
            look.y = eye.y + look_over;
        }
        self.last = Some((eye, look));
        // CTR's camera has no wall test (its tracks keep walls out from behind the kart); WoW's
        // pillars and trees do not, so a tuning may bring the eye in short of what blocks it.
        let to_eye = eye - look;
        let reach = to_eye.length();
        if tuning.wall_pull {
            if let Some(hit) = world.ray(look, to_eye, reach) {
                eye = look + to_eye * ((hit.distance - 12.0).max(24.0) / reach);
            }
        }
        self.started = true;
        // The screen's roll, `(angle[0] * rollS) >> 8` with `angle[0]` the bank in 256ths.
        let bank = (tuning.bank * 256.0).round() as i32;
        Shot {
            eye,
            look,
            roll: rad((bank * self.roll) >> 8),
            cut,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat() -> Collision {
        let s = 50000.0;
        Collision::new(
            [
                [[-s, 0.0, -s], [-s, 0.0, s], [s, 0.0, s]],
                [[-s, 0.0, -s], [s, 0.0, s], [s, 0.0, -s]],
            ],
            160.0,
        )
    }

    fn input(speed: i32) -> CameraInput {
        CameraInput {
            position: Vec3::ZERO,
            heading: 0,
            turn_angle: 0,
            spin: 0,
            roll: 0,
            speed_approx: speed,
            spinning: false,
            turbo: false,
            fire_speed_cap: 0,
            travel: None,
            blasted: false,
            grounded: true,
            ground: 0.0,
            cut: false,
            mask_grab: None,
        }
    }

    /// The eye's view direction as a heading, degrees.
    fn view_deg(shot: &Shot) -> f32 {
        let d = shot.look - shot.eye;
        d.x.atan2(d.z).to_degrees()
    }

    #[test]
    fn through_a_powerslide_the_eye_swings_with_the_kart_as_ctr_s_does() {
        // Travelling along +Z, the body turned 200 into the slide and CTR's swing at its full
        // 384: CTR's rigid eye looks 584/4096 of a turn (51.3 degrees) off the travel.
        let (mut cam, world) = (ChaseCamera::default(), flat());
        let mut slide = input(13000);
        slide.travel = Some(0);
        slide.heading = 200;
        slide.spin = 384;
        let shot = cam.tick(&slide, &world);
        let off = view_deg(&shot);
        assert!((off - 51.3).abs() < 0.5, "{off} degrees off the travel");
    }

    #[test]
    fn a_sharp_turn_is_taken_at_once_as_ctr_s_rigid_eye_does() {
        let (mut cam, world) = (ChaseCamera::default(), flat());
        let mut k = input(13000);
        k.travel = Some(0);
        for _ in 0..30 {
            cam.tick(&k, &world);
        }
        k.heading = 1024;
        k.travel = Some(1024);
        let first = view_deg(&cam.tick(&k, &world));
        assert!((first - 90.0).abs() < 0.5, "first tick: {first}");
    }

    #[test]
    fn at_racing_speed_the_eye_sits_behind_and_above_and_looks_over_the_kart() {
        let (mut cam, world) = (ChaseCamera::default(), flat());
        let mut shot = cam.tick(&input(13000), &world);
        for _ in 0..90 {
            shot = cam.tick(&input(13000), &world);
        }
        // About 212 lu behind (heading 0 faces +Z; the rig sits 9.8 lu forward of the origin at
        // no roll) and 98 up; the look-at 107 up and 31.6 ahead of that.
        assert!((shot.eye.z + 211.6).abs() < 2.0, "{shot:?}");
        assert!((shot.eye.y - 98.0).abs() < 3.0, "{shot:?}");
        assert!(
            (shot.look.y - 107.1).abs() < 1.0 && (shot.look.z - 41.4).abs() < 1.0,
            "{shot:?}"
        );
    }

    #[test]
    fn a_turbo_pushes_the_boom_out_and_it_comes_back() {
        let (mut cam, world) = (ChaseCamera::default(), flat());
        for _ in 0..90 {
            cam.tick(&input(13000), &world);
        }
        let before = cam.tick(&input(13000), &world).eye.z;
        let mut kick = input(13000);
        kick.turbo = true;
        kick.fire_speed_cap = 4096;
        let mut out: f32 = 0.0;
        cam.tick(&kick, &world);
        for _ in 0..12 {
            out = out.min(cam.tick(&input(13000), &world).eye.z);
        }
        assert!(out < before - 60.0, "{out} vs {before}");
        for _ in 0..120 {
            cam.tick(&input(13000), &world);
        }
        assert!((cam.tick(&input(13000), &world).eye.z - before).abs() < 1.0);
    }

    /// Ground rising `grade` (rise per run) towards +Z, in 64 lu tiles.
    fn slope(grade: f32) -> Collision {
        let mut tris = Vec::new();
        for i in -30..30 {
            for j in -30..30 {
                let (x0, z0) = (i as f32 * 64.0, j as f32 * 64.0);
                let (x1, z1) = (x0 + 64.0, z0 + 64.0);
                let y = |z: f32| z * grade;
                let (a, b, c, d) = (
                    [x0, y(z0), z0],
                    [x1, y(z0), z0],
                    [x1, y(z1), z1],
                    [x0, y(z1), z1],
                );
                tris.push([a, c, b]);
                tris.push([a, d, c]);
            }
        }
        Collision::new(tris, 160.0)
    }

    /// The view's pitch, degrees up.
    fn pitch(shot: &Shot) -> f32 {
        (shot.look - shot.eye).normalize().y.asin().to_degrees()
    }

    /// How far below the view's centre the kart's middle (32 lu over its origin `at`) is seen,
    /// degrees.
    fn kart_angle(shot: &Shot, at: Vec3) -> f32 {
        (at + Vec3::Y * 32.0 - shot.eye)
            .angle_between(shot.look - shot.eye)
            .to_degrees()
    }

    /// Three seconds at racing speed from a standing start on `world`.
    fn settle(cam: &mut ChaseCamera, world: &Collision) -> Shot {
        let mut shot = cam.tick(&input(13000), world);
        for _ in 0..90 {
            shot = cam.tick(&input(13000), world);
        }
        shot
    }

    #[test]
    fn on_a_slope_the_rig_tilts_with_it_and_the_kart_keeps_its_place() {
        let level = settle(&mut ChaseCamera::default(), &slope(0.0));
        let flat = kart_angle(&level, Vec3::ZERO);
        let half_fov = FOV_Y.to_degrees() / 2.0;
        for grade in [0.35, -0.35, 0.6, -0.6] {
            let shot = settle(&mut ChaseCamera::default(), &slope(grade));
            let seen = kart_angle(&shot, Vec3::ZERO);
            assert!((seen - flat).abs() < 2.0, "{grade}: {seen} vs {flat}");
            assert!(seen < half_fov - 10.0, "{grade}: {seen} of {half_fov}");
            // The view pitches with the hill: up it uphill, down it downhill.
            let tilt = pitch(&shot) - pitch(&level);
            let hill = grade.atan().to_degrees();
            assert!((tilt - hill).abs() < 1.0, "{grade}: {tilt} vs {hill}");
            // Still over the ground behind as over flat road.
            let ground = shot.eye.z * grade;
            assert!(shot.eye.y >= ground + NEAR.vert - 0.01, "{grade}: {shot:?}");
        }
    }

    #[test]
    fn onto_a_hill_the_tilt_comes_in_over_most_of_a_second() {
        // The kart stands at the foot of a 0.35 grade: the tilt takes it up smoothly.
        let world = slope(0.35);
        let mut cam = ChaseCamera::default();
        cam.tick(&input(13000), &slope(0.0));
        let start = pitch(&cam.tick(&input(13000), &slope(0.0)));
        let goal = 0.35f32.atan().to_degrees();
        let mut pitches = vec![start];
        for _ in 0..60 {
            pitches.push(pitch(&cam.tick(&input(13000), &world)));
        }
        let at = |n: usize| (pitches[n] - start) / goal;
        assert!(at(6) < 0.25 && at(21) > 0.5 && at(60) > 0.9, "{pitches:?}");
        let step = pitches
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max);
        assert!(step < 0.8, "{step}° in a tick");
    }

    /// Bumps across the road: crests and dips 10–22 lu off level by turns, one every 64–128 lu at
    /// random, eased between (steepest grade 1.08, still ground), so 20–44 lu from one to the next.
    fn bump_height(z: f32) -> f32 {
        let mut seed = 0x2545_f491_u32;
        let mut next = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1 << 24) as f32
        };
        let (mut z0, mut h0, mut crest) = (-2000.0, 0.0, 1.0);
        loop {
            let (z1, h1) = (z0 + 64.0 + 64.0 * next(), crest * (10.0 + 12.0 * next()));
            crest = -crest;
            if z < z1 {
                let t = (z - z0) / (z1 - z0);
                return h0 + (h1 - h0) * (1.0 - (t * std::f32::consts::PI).cos()) / 2.0;
            }
            (z0, h0) = (z1, h1);
        }
    }

    /// [`bump_height`]'s road, 400 lu wide, in 16 lu strips.
    fn bumpy() -> Collision {
        let mut tris = Vec::new();
        let (x0, x1) = (-200.0, 200.0);
        for j in -100..1100 {
            let (z0, z1) = (j as f32 * 16.0, (j + 1) as f32 * 16.0);
            let (h0, h1) = (bump_height(z0), bump_height(z1));
            let (a, b, c, d) = ([x0, h0, z0], [x1, h0, z0], [x1, h1, z1], [x0, h1, z1]);
            tris.push([a, c, b]);
            tris.push([a, d, c]);
        }
        Collision::new(tris, 160.0)
    }

    #[test]
    fn over_bumps_the_view_holds_steady() {
        // Ten seconds over the bumps at racing speed (50.8 lu a tick, about 1520 lu/s), the kart
        // riding each one. The slope-follow this replaced (one probe 480 lu ahead) swung the view
        // 0.41° in a tick here, spread 0.17° (sd), and moved the kart on the screen 0.73° a tick.
        // 0.1° is about 1.5 lines of a 1080-line screen at this FOV.
        let world = bumpy();
        let mut cam = ChaseCamera::default();
        let (mut pitches, mut karts) = (Vec::new(), Vec::new());
        for n in 0..300 {
            let z = n as f32 * 50.8;
            let mut k = input(13000);
            k.position = Vec3::new(0.0, bump_height(z), z);
            let shot = cam.tick(&k, &world);
            pitches.push(pitch(&shot));
            karts.push(kart_angle(&shot, k.position));
        }
        let (pitches, karts) = (&pitches[60..], &karts[60..]);
        let step = |p: &[f32]| {
            p.windows(2)
                .map(|w| (w[1] - w[0]).abs())
                .fold(0.0, f32::max)
        };
        let mean = pitches.iter().sum::<f32>() / pitches.len() as f32;
        let sd =
            (pitches.iter().map(|p| (p - mean).powi(2)).sum::<f32>() / pitches.len() as f32).sqrt();
        assert!(step(pitches) < 0.1, "{}° in a tick", step(pitches));
        assert!(sd < 0.1, "spread {sd}°");
        assert!(step(karts) < 0.01, "the kart moved {}°", step(karts));
    }

    #[test]
    fn the_far_camera_sits_further_back_and_higher() {
        let near = settle(&mut ChaseCamera::default(), &flat());
        let far = settle(
            &mut ChaseCamera {
                far: true,
                ..Default::default()
            },
            &flat(),
        );
        // FarCam4x3 at full speed: 288 lu of boom, so 284.6 behind less the rig's 9.8, and
        // 0.1528·288 + 96 = 140 up; the look-at 96 − 4.9 up.
        assert!((far.eye.z + 274.8).abs() < 2.0, "{far:?}");
        assert!((far.eye.y - 140.0).abs() < 3.0, "{far:?}");
        assert!((far.look.y - 91.1).abs() < 1.0, "{far:?}");
        assert!(far.eye.z < near.eye.z - 50.0 && far.eye.y > near.eye.y + 30.0);
    }

    #[test]
    fn looking_back_cuts_to_the_front_at_once_and_back() {
        let world = flat();
        let mut cam = ChaseCamera::default();
        // Slow, so the boom is short and the speed filter low.
        for _ in 0..90 {
            cam.tick(&input(1024), &world);
        }
        let ahead = cam.tick(&input(1024), &world);
        assert!(ahead.eye.z < -150.0 && !ahead.cut, "{ahead:?}");
        // R2 at racing speed: in front of the kart, looking back at it, on this very tick, and
        // the speed filter restarts at the speed, so the boom is already full.
        cam.look_back = true;
        let back = cam.tick(&input(13000), &world);
        assert!(back.cut, "{back:?}");
        assert!((back.eye.z - 211.6).abs() < 2.0, "{back:?}");
        assert!(back.look.z < back.eye.z, "{back:?}");
        let held = cam.tick(&input(13000), &world);
        assert!(
            !held.cut && (held.eye - back.eye).length() < 1.0,
            "{held:?}"
        );
        cam.look_back = false;
        let again = cam.tick(&input(13000), &world);
        assert!(again.cut && (again.eye.z + 211.6).abs() < 2.0, "{again:?}");
    }

    #[test]
    fn aku_aku_s_respawn_cuts_and_his_lowering_holds_the_eye_s_height() {
        let world = flat();
        let mut cam = ChaseCamera::default();
        for _ in 0..30 {
            cam.tick(&input(0), &world);
        }
        let level = cam.tick(&input(0), &world);
        // Put back 126 lu over the ground: the shot cuts, and its eye stands where a kart 64 lu
        // over the ground would have it, whatever the kart's height while it comes down.
        let mut held = input(0);
        held.position.y = 126.0;
        held.cut = true;
        held.mask_grab = Some(64.0);
        let first = cam.tick(&held, &world);
        assert!(first.cut, "{first:?}");
        assert!(
            (first.eye.y - (level.eye.y + 64.0)).abs() < 0.5,
            "{first:?}"
        );
        assert!(first.look.y > level.look.y + 120.0, "it looks at the kart");
        held.cut = false;
        held.position.y = 70.0;
        let lower = cam.tick(&held, &world);
        assert!(!lower.cut);
        assert!((lower.eye.y - first.eye.y).abs() < 0.5, "{lower:?}");
    }

    fn ctr() -> ChaseCamera {
        ChaseCamera {
            tuning: CameraTuning::CTR,
            ..Default::default()
        }
    }

    #[test]
    fn the_default_tuning_turns_as_ctr_and_smooth_follow_eases_a_quarter_a_tick() {
        let t = CameraTuning::default();
        assert_eq!(t, CameraTuning::KARTCRAFT);
        assert_eq!(ChaseCamera::default().tuning, t);
        // CTR's turning: on the body, all of the swing, rigid, straight behind.
        assert!(!t.travel && t.swing == 1.0 && share(t.turn_smoothing) == 1.0);
        assert_eq!((t.yaw_offset, t.side_offset), (0.0, 0.0));
        // A tenth of each tilt stage's gap a tick; smooth follow's yaw a quarter.
        assert!((share(t.tilt_smoothing) - 0.1).abs() < 0.001);
        assert!((share(CameraTuning::SMOOTH.turn_smoothing) - 0.25).abs() < 0.001);
        // CTR's zoom tables and pitch, as numbers.
        assert_eq!(
            (t.distance, t.slow_distance, t.height, t.look_height),
            (224.0, 179.0, 64.0, 112.0)
        );
        assert_eq!(
            (t.far_extra, t.turbo_push, t.look_ahead),
            (64.0, 96.0, 32.0)
        );
        assert!((t.pitch - 8.789).abs() < 0.001);
        assert!((CameraTuning::CTR.fov_radians() - FOV_Y).abs() < 1e-4);
    }

    #[test]
    fn ctr_classic_keeps_the_eye_rigid_on_heading_and_swing() {
        // The slide that the Kartcraft camera follows only 35% of: CTR's eye looks along heading
        // + swing, 584/4096 of a turn, on the very first ticks and every one after.
        let (mut cam, world) = (ctr(), flat());
        let mut slide = input(13000);
        slide.travel = Some(0);
        slide.heading = 200;
        slide.spin = 384;
        let rigid = 584.0 * 360.0 / 4096.0;
        for _ in 0..3 {
            let off = view_deg(&cam.tick(&slide, &world));
            assert!((off - rigid).abs() < 0.5, "{off} vs {rigid}");
        }
        // A quarter turn at once is taken at once.
        slide.heading = 1024;
        slide.spin = 0;
        slide.travel = Some(1024);
        let off = view_deg(&cam.tick(&slide, &world));
        assert!((off - 90.0).abs() < 0.5, "{off}");
    }

    #[test]
    fn ctr_classic_keeps_world_up_offsets_on_a_hill_and_the_full_far_table() {
        // Downhill behind the kart, so the ground clamp stays out of it: the view pitches as on
        // flat road.
        let level = settle(&mut ctr(), &slope(0.0));
        let hill = settle(&mut ctr(), &slope(0.35));
        assert!((pitch(&hill) - pitch(&level)).abs() < 0.01, "{hill:?}");
        assert!((hill.eye.y - level.eye.y).abs() < 0.01, "{hill:?}");
        // FarCam4x3 as the Kartcraft one has it.
        let far = settle(&mut ChaseCamera { far: true, ..ctr() }, &flat());
        assert!((far.eye.z + 274.8).abs() < 2.0 && (far.eye.y - 140.0).abs() < 3.0);
        assert!((far.look.y - 91.1).abs() < 1.0, "{far:?}");
    }

    #[test]
    fn a_change_of_tuning_applies_on_the_next_tick() {
        // Ground out to 1920 lu, under the longer boom.
        let world = slope(0.0);
        let mut cam = ChaseCamera::default();
        let before = settle(&mut cam, &world);
        cam.tuning.distance = 300.0;
        cam.tuning.height = 100.0;
        cam.tuning.look_height = 50.0;
        let after = cam.tick(&input(13000), &world);
        // 76 lu more boom at the pitch, 36 more lift; the look-at 62 lower.
        assert!(
            (after.eye.z - (before.eye.z - 76.0 * 0.98826)).abs() < 1.0,
            "{after:?}"
        );
        assert!((after.eye.y - (before.eye.y + 36.0 + 76.0 * 0.1528)).abs() < 1.0);
        assert!((after.look.y - (before.look.y - 62.0)).abs() < 0.5);

        // A level boom straight behind, no look-ahead.
        cam.tuning.pitch = 0.0;
        cam.tuning.look_ahead = 0.0;
        let mut shot = cam.tick(&input(13000), &world);
        for _ in 0..8 {
            shot = cam.tick(&input(13000), &world);
        }
        assert!(
            (shot.eye.y - 100.0).abs() < 0.5 && shot.look.z.abs() < 0.5,
            "{shot:?}"
        );
    }

    #[test]
    fn turbo_push_back_and_wall_pull_in_turn_off() {
        let world = flat();
        let mut cam = ChaseCamera::default();
        cam.tuning.turbo_push = 0.0;
        let before = settle(&mut cam, &world).eye.z;
        let mut kick = input(13000);
        kick.turbo = true;
        kick.fire_speed_cap = 4096;
        cam.tick(&kick, &world);
        for _ in 0..12 {
            let z = cam.tick(&input(13000), &world).eye.z;
            assert!((z - before).abs() < 0.01, "{z} vs {before}");
        }

        // A wall 100 lu behind the kart, across the road: pulled in short of it, or not.
        let s = 5000.0;
        let wall = Collision::new(
            [
                [[-s, 0.0, -s], [-s, 0.0, s], [s, 0.0, s]],
                [[-s, 0.0, -s], [s, 0.0, s], [s, 0.0, -s]],
                [[-s, -10.0, -100.0], [s, -10.0, -100.0], [s, 1000.0, -100.0]],
                [
                    [-s, -10.0, -100.0],
                    [s, 1000.0, -100.0],
                    [-s, 1000.0, -100.0],
                ],
            ],
            160.0,
        );
        let pulled = settle(&mut ChaseCamera::default(), &wall);
        assert!(pulled.eye.z > -100.0, "{pulled:?}");
        let mut through = ChaseCamera::default();
        through.tuning.wall_pull = false;
        let through = settle(&mut through, &wall);
        assert!(through.eye.z < -200.0, "{through:?}");
    }

    #[test]
    fn bank_scales_the_screen_s_roll() {
        let world = flat();
        let mut lean = input(13000);
        lean.roll = 200;
        let roll = |bank: f32| {
            let mut cam = ChaseCamera::default();
            cam.tuning.bank = bank;
            let mut shot = cam.tick(&lean, &world);
            for _ in 0..90 {
                shot = cam.tick(&lean, &world);
            }
            shot.roll
        };
        // CTR's half: the filtered lean (settling 7 short of 200) over two.
        assert!((roll(0.5) - rad(96)).abs() < rad(2), "{}", roll(0.5));
        assert!((roll(1.0) - 2.0 * roll(0.5)).abs() < rad(2));
        assert_eq!(roll(0.0), 0.0);
    }

    fn smooth() -> ChaseCamera {
        ChaseCamera {
            tuning: CameraTuning::SMOOTH,
            ..Default::default()
        }
    }

    #[test]
    fn smooth_follow_looks_mostly_where_the_kart_is_going_and_eases_into_a_turn() {
        // The full slide: 35% of its 51.3° off the travel.
        let (mut cam, world) = (smooth(), flat());
        let mut slide = input(13000);
        slide.travel = Some(0);
        slide.heading = 200;
        slide.spin = 384;
        let mut shot = cam.tick(&slide, &world);
        for _ in 0..60 {
            shot = cam.tick(&slide, &world);
        }
        let off = view_deg(&shot);
        assert!(
            (off - 0.35 * 51.33).abs() < 1.0,
            "{off} degrees off the travel"
        );

        // A quarter turn at once: a quarter of it the first tick, all of it within a second.
        let mut k = input(13000);
        k.travel = Some(0);
        let mut cam = smooth();
        for _ in 0..30 {
            cam.tick(&k, &world);
        }
        k.heading = 1024;
        k.travel = Some(1024);
        let first = view_deg(&cam.tick(&k, &world));
        assert!((first - 22.5).abs() < 1.0, "first tick: {first}");
        let mut last = first;
        for _ in 0..30 {
            last = view_deg(&cam.tick(&k, &world));
        }
        assert!((last - 90.0).abs() < 1.0, "after a second: {last}");
    }

    #[test]
    fn the_swing_share_and_the_yaw_and_side_offsets_turn_and_move_the_boom() {
        let world = flat();
        let mut slide = input(13000);
        slide.heading = 200;
        slide.spin = 384;
        let view = |t: CameraTuning| {
            let mut cam = ChaseCamera {
                tuning: t,
                ..Default::default()
            };
            cam.tick(&slide, &world)
        };
        let deg = |a: f32| a * 360.0 / 4096.0;
        // None of the swing: the eye on the heading alone.
        let no_swing = view(CameraTuning {
            swing: 0.0,
            ..CameraTuning::KARTCRAFT
        });
        assert!((view_deg(&no_swing) - deg(200.0)).abs() < 0.5);
        // Half of it.
        let half = view(CameraTuning {
            swing: 0.5,
            ..CameraTuning::KARTCRAFT
        });
        assert!((view_deg(&half) - deg(392.0)).abs() < 0.5);

        // Turned 30° round the kart, still looking at it.
        let straight = input(13000);
        let mut cam = ChaseCamera::default();
        cam.tuning.yaw_offset = 30.0;
        cam.tuning.look_ahead = 0.0;
        let shot = cam.tick(&straight, &world);
        assert!((view_deg(&shot) - 30.0).abs() < 0.5, "{shot:?}");
        assert!(
            shot.eye.x < -100.0,
            "behind and to the kart's right: {shot:?}"
        );

        // Moved 40 lu to the kart's right (heading 0 faces +Z, its right is −X), eye and look.
        let base = ChaseCamera::default().tick(&straight, &world);
        let mut cam = ChaseCamera::default();
        cam.tuning.side_offset = 40.0;
        let moved = cam.tick(&straight, &world);
        assert!(
            (moved.eye.x - (base.eye.x - 40.0)).abs() < 0.01,
            "{moved:?}"
        );
        assert!(
            (moved.look.x - (base.look.x - 40.0)).abs() < 0.01,
            "{moved:?}"
        );
        assert!((view_deg(&moved) - view_deg(&base)).abs() < 0.01);
    }

    /// A floor at y = 0 in 100 lu tiles 1000 lu each way, small enough for the grid to keep, so
    /// the eye finds the ground under it.
    fn tiled() -> Collision {
        let mut tris = Vec::new();
        for i in -10..10 {
            for j in -10..10 {
                let (x0, z0) = (i as f32 * 100.0, j as f32 * 100.0);
                let (x1, z1) = (x0 + 100.0, z0 + 100.0);
                tris.push([[x0, 0.0, z0], [x0, 0.0, z1], [x1, 0.0, z1]]);
                tris.push([[x0, 0.0, z0], [x1, 0.0, z1], [x1, 0.0, z0]]);
            }
        }
        Collision::new(tris, 160.0)
    }

    /// A kart blasted at `t` = 0, flung up 300 lu and down again over 20 ticks, still tumbling
    /// on the ground to tick 24: the shots.
    fn blast_flight(tuning: CameraTuning) -> Vec<Shot> {
        let world = tiled();
        let mut cam = ChaseCamera {
            tuning,
            ..Default::default()
        };
        let mut k = input(6000);
        for _ in 0..60 {
            cam.tick(&k, &world);
        }
        let mut shots = Vec::new();
        for t in 0..40 {
            let flying = t < 20;
            k.blasted = t < 24;
            k.grounded = !flying;
            k.position.y = if flying {
                300.0 * (std::f32::consts::PI * t as f32 / 20.0).sin()
            } else {
                0.0
            };
            shots.push(cam.tick(&k, &world));
        }
        shots
    }

    #[test]
    fn a_blasted_kart_s_flight_does_not_lift_the_eye_and_it_eases_back_after_the_landing() {
        for tuning in [
            CameraTuning::CTR,
            CameraTuning::KARTCRAFT,
            CameraTuning::SMOOTH,
        ] {
            let world = tiled();
            let mut steady = ChaseCamera {
                tuning,
                ..Default::default()
            };
            let mut rest = steady.tick(&input(6000), &world);
            for _ in 0..60 {
                rest = steady.tick(&input(6000), &world);
            }
            let shots = blast_flight(tuning);
            // CAM.c:1335-1345: the eye never rises over its height at the blast, and the look-at
            // keeps its height over the eye.
            for (t, s) in shots[..24].iter().enumerate() {
                assert!(s.eye.y <= rest.eye.y + 0.01, "{tuning:?} tick {t}: {s:?}");
                let over = s.look.y - s.eye.y;
                assert!((over - (rest.look.y - rest.eye.y)).abs() < 0.01, "tick {t}");
            }
            // Landed (tick 20) but still tumbling to tick 24: held; then 8 ticks back, the first
            // of them where the last left off.
            assert!(
                (shots[24].eye - shots[23].eye).length() < 0.01,
                "{tuning:?}"
            );
            assert!(
                (shots[24].look - shots[23].look).length() < 0.01,
                "{tuning:?}"
            );
            let settled = &shots[32];
            assert!(
                (settled.eye - rest.eye).length() < 0.01,
                "{tuning:?} {settled:?}"
            );
            assert!(
                (settled.look - rest.look).length() < 0.01,
                "{tuning:?} {settled:?}"
            );
        }
    }

    #[test]
    fn without_a_blast_the_eye_rises_with_the_kart() {
        let world = tiled();
        let mut cam = ChaseCamera::default();
        let mut k = input(6000);
        let rest = cam.tick(&k, &world);
        k.position.y = 300.0;
        k.grounded = false;
        let up = cam.tick(&k, &world);
        assert!(up.eye.y > rest.eye.y + 250.0, "{rest:?} {up:?}");
    }
}
