//! CTR's in-race chase camera, `CAM_FollowDriver_Normal` (`CAM.c:1047-1546`), ticked with the
//! kart at 30 Hz in level units (`docs/ctr-camera-spec.md`). Its yaw is the kart's physics
//! heading plus the powerslide's swing, with no lag: a boom pitched 8.8° up trails the kart,
//! longer with speed (179 to 224 lu) and pushed out up to 96 lu by every turbo; it looks at a
//! point just over and ahead of the kart along the drawn nose, banks by half the ground's lean,
//! and keeps at least 64 lu over the ground under it. Vertical FOV 73.74°, CTR's 4:3 frustum
//! kept vertically for a wide screen.

use bevy::math::Vec3;
use ctr_kart::{CameraInput, Collision};

/// The near camera's boom from the slowest speed to the fastest, its height base and the
/// look-at's height (lu); its pitch (4096ths, upward).
const D_MIN: f32 = 179.0;
const D_MAX: f32 = 224.0;
const V_DIST: f32 = 64.0;
const LOOK_H: f32 = 112.0;
const PITCH: i32 = 100;
/// The vertical field of view, radians (73.74°).
pub const FOV_Y: f32 = 1.2870;
/// How far ahead of the kart the ground's rise is read (lu), the most of it heeded (lu), how
/// fast the camera takes it up each tick, and how much of it raises the gaze and the eye.
const SLOPE_PROBE: f32 = 480.0;
const SLOPE_MAX: f32 = 320.0;
const SLOPE_FOLLOW: f32 = 0.12;
const SLOPE_LOOK: f32 = 0.6;
const SLOPE_EYE: f32 = 0.25;

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

/// One camera pose, lu: the eye, what it looks at, and its roll (radians).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shot {
    pub eye: Vec3,
    pub look: Vec3,
    pub roll: f32,
}

#[derive(Default)]
pub struct ChaseCamera {
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
    /// The ground's rise ahead of the kart (lu), filtered.
    slope: f32,
}

impl ChaseCamera {
    /// One 30 Hz frame for `k`, over `world`'s ground.
    pub fn tick(&mut self, k: &CameraInput, world: &Collision) -> Shot {
        // 2.2: the boom lengthens with the filtered speed.
        let x = k.speed_approx.abs();
        if !self.started {
            self.speed = x;
            self.ahead = -32;
        } else if x < self.speed {
            self.speed = (240 * self.speed + x) >> 8;
        } else {
            self.speed = (240 * self.speed + 16 * x) >> 8;
        }
        let t = ((self.speed - 1024) as f32 / 4096.0).clamp(0.0, 1.0);
        let mut d = D_MIN + t * (D_MAX - D_MIN);

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

        // 2.3: the eye trails the heading plus the powerslide's swing.
        let a_eye = k.heading + k.spin;
        let (f, r) = (fwd(a_eye), right(a_eye));
        let (sp, cp) = (rad(PITCH).sin(), rad(PITCH).cos());
        let roll2 = rad(2 * self.roll);
        let side = r * (64.0 * roll2.sin()) + f * (64.0 * sp * roll2.cos());
        let base = Vec3::new(k.position.x + side.x, k.position.y, k.position.z + side.z);
        let mut eye = base - f * (cp * d) + Vec3::Y * (sp * d + V_DIST);

        // 2.4: it looks just ahead of the drawn nose, over the kart.
        self.ahead = if k.spinning {
            (self.ahead + 8).min(0)
        } else {
            (self.ahead - 8).max(-32)
        };
        let g = fwd(a_eye + k.turn_angle);
        let ahead = -(self.ahead as f32);
        let mut look = base + g * (cp * ahead) + Vec3::Y * (LOOK_H - sp * ahead);

        // Deviation: CTR's tracks climb gently, so its level gaze sees the road; a WoW hill fills
        // the screen. The gaze, and a little of the eye, follow the ground's rise ahead, filtered.
        let probe = k.position + fwd(k.heading) * SLOPE_PROBE;
        let rise = world
            .ground(probe + Vec3::Y * 1024.0, 4096.0)
            .map_or(0.0, |hit| {
                (hit.point.y - k.position.y).clamp(-SLOPE_MAX, SLOPE_MAX)
            });
        self.slope += (rise - self.slope) * SLOPE_FOLLOW;
        look.y += self.slope * SLOPE_LOOK;
        eye.y += self.slope * SLOPE_EYE;

        // 2.5: the ground's lean, filtered (after use, as the game does).
        self.roll = (224 * self.roll + 32 * k.roll) >> 8;

        // 5: never closer than 64 lu over the ground under the eye.
        match world.ground(eye + Vec3::Y * 256.0, 2304.0) {
            Some(hit) => {
                eye.y = eye.y.max(hit.point.y + V_DIST);
                if self.blend > 0 {
                    let n = self.blend as f32;
                    eye.y = ((8.0 - n) * eye.y + n * (k.position.y + self.start_height)) / 8.0;
                    self.blend -= 1;
                }
                self.height = eye.y - k.position.y;
            }
            None => {
                self.blend = 8;
                if self.started && eye.y < k.position.y + self.height {
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

    #[test]
    fn uphill_the_gaze_lifts_to_the_road_ahead() {
        let settle = |world: &Collision| {
            let mut cam = ChaseCamera::default();
            let mut shot = cam.tick(&input(13000), world);
            for _ in 0..90 {
                shot = cam.tick(&input(13000), world);
            }
            shot
        };
        let level = settle(&slope(0.0));
        let uphill = settle(&slope(0.35));
        let downhill = settle(&slope(-0.35));
        assert!(
            uphill.look.y > level.look.y + 80.0,
            "{uphill:?} vs {level:?}"
        );
        assert!(
            downhill.look.y < level.look.y - 80.0,
            "{downhill:?} vs {level:?}"
        );
    }
}
