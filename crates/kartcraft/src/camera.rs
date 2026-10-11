//! CTR's in-race chase camera, `CAM_FollowDriver_Normal` (`CAM.c:1047-1546`), ticked with the
//! kart at 30 Hz in level units (`docs/ctr-camera-spec.md`). Its yaw is the kart's physics
//! heading plus the powerslide's swing, with no lag: a boom pitched 8.8° up trails the kart,
//! longer with speed (179 to 224 lu near, 224 to 288 far) and pushed out up to 96 lu by every
//! turbo; it looks at a point just over and ahead of the kart along the drawn nose, banks by half
//! the ground's lean, and keeps at least 64 lu (far: 96) over the ground under it. Look-back (R2)
//! turns both yaws half a turn. Vertical FOV 73.74°, CTR's 4:3 frustum kept vertically for a wide
//! screen.

use bevy::math::{Quat, Vec3};
use ctr_kart::{CameraInput, Collision};

/// One of CTR's 1P 4:3 zoom tables (`NearCam4x3`, `FarCam4x3`, `zGlobal_DATA.c:983-1008`): the
/// boom from the slowest speed to the fastest, the look-at's height and the eye's height base,
/// which is also its least clearance over the ground (lu).
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
/// The vertical field of view, radians (73.74°).
pub const FOV_Y: f32 = 1.2870;
/// Where the ground's grade is read along the heading, from behind the kart to ahead of it (lu).
const GRADE_SAMPLES: [f32; 17] = [
    -192.0, -160.0, -128.0, -96.0, -64.0, -32.0, 0.0, 32.0, 64.0, 96.0, 128.0, 160.0, 192.0, 224.0,
    256.0, 288.0, 320.0,
];
/// The steepest grade heeded (rise per run, about 37°).
const GRADE_MAX: f32 = 0.75;
/// Each of the tilt's two filter stages' share of the gap per tick: 0.1 at 30 Hz is a 0.32 s
/// time constant each, so the tilt reaches 63% of a new grade in about 0.7 s and starts moving
/// smoothly rather than with a jump.
const TILT_FOLLOW: f32 = 0.1;
/// The tilt's pivot over the kart's origin (lu): the kart's middle, which keeps its place on the
/// screen however the rig tilts.
const PIVOT_H: f32 = 32.0;

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
    ahead: i32,
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
    /// One 30 Hz frame for `k`, over `world`'s ground.
    pub fn tick(&mut self, k: &CameraInput, world: &Collision) -> Shot {
        let zoom = if self.far { FAR } else { NEAR };
        // 3.4: look-back turning on or off is DIRECTION_CHANGED, an instant cut that restarts
        // the speed filter.
        let cut = self.started && self.look_back != self.looked_back;
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
            self.ahead = -32;
        }
        let t = ((self.speed - 1024) as f32 / 4096.0).clamp(0.0, 1.0);
        let mut d = zoom.dist_min + t * (zoom.dist_max - zoom.dist_min);

        // 3.3: every turbo pushes the boom out.
        if k.turbo {
            self.turbo_timer = self.turbo_timer.max(k.fire_speed_cap >> 4);
        }
        if self.turbo_timer == 0 {
            self.turbo = (self.turbo - 512).max(0);
        } else {
            self.turbo = (self.turbo + 2048).min(0x6000);
            self.turbo_timer = (self.turbo_timer - 32).max(0);
        }
        d += (self.turbo >> 8) as f32;

        // 2.3: the eye trails the heading plus the powerslide's swing (and look-back's half turn).
        let rev = if self.look_back { REVERSE } else { 0 };
        let a_eye = k.heading + k.spin + rev;
        let (f, r) = (fwd(a_eye), right(a_eye));
        let (sp, cp) = (rad(PITCH).sin(), rad(PITCH).cos());
        let roll2 = rad(2 * self.roll);
        let side = r * (64.0 * roll2.sin()) + f * (64.0 * sp * roll2.cos());
        let base = Vec3::new(k.position.x + side.x, k.position.y, k.position.z + side.z);
        let mut eye = base - f * (cp * d) + Vec3::Y * (sp * d + zoom.vert);

        // 2.4: it looks just ahead of the drawn nose, over the kart.
        self.ahead = if k.spinning {
            (self.ahead + 8).min(0)
        } else {
            (self.ahead - 8).max(-32)
        };
        let g = fwd(a_eye + k.turn_angle);
        let ahead = -(self.ahead as f32);
        let mut look = base + g * (cp * ahead) + Vec3::Y * (zoom.look_h - sp * ahead);

        // Deviation: CTR's camera keeps world-up offsets (spec 2.5), fine on its gentle tracks;
        // on a WoW hill the road ahead fills the screen or drops out of it. The whole rig tilts
        // about the kart's middle with the ground's grade along the heading, read over several
        // samples, clamped and filtered slowly, so the camera sits over a slope as it sits over
        // flat road and the kart keeps its place on the screen.
        let target = grade(world, k.position, k.heading).map_or(self.tilt[1], f32::atan);
        if !self.started {
            self.tilt = [target; 2];
        }
        self.tilt[0] += (target - self.tilt[0]) * TILT_FOLLOW;
        self.tilt[1] += (self.tilt[0] - self.tilt[1]) * TILT_FOLLOW;
        // About the heading's right, a positive angle lifts its forward.
        let tilt = Quat::from_axis_angle(right(k.heading), self.tilt[1]);
        let pivot = k.position + Vec3::Y * PIVOT_H;
        eye = pivot + tilt * (eye - pivot);
        look = pivot + tilt * (look - pivot);

        // 2.5: the ground's lean, filtered (after use, as the game does).
        self.roll = (224 * self.roll + 32 * k.roll) >> 8;

        // 5: never closer than vertDistance over the ground under the eye.
        match world.ground(eye + Vec3::Y * 256.0, 2304.0) {
            Some(hit) => {
                eye.y = eye.y.max(hit.point.y + zoom.vert);
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
        // Deviation: CTR's camera has no wall test (its tracks keep walls out from behind the
        // kart); WoW's pillars and trees do not, so the eye comes in short of what blocks it.
        let to_eye = eye - look;
        let reach = to_eye.length();
        if let Some(hit) = world.ray(look, to_eye, reach) {
            eye = look + to_eye * ((hit.distance - 12.0).max(24.0) / reach);
        }
        self.started = true;
        Shot {
            eye,
            look,
            roll: rad(self.roll / 2),
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
        }
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
}
