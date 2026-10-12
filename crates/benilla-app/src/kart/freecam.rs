//! The free camera, for filming the kart (`/kartfree`, the `KartFreeCam` CVar, held for the
//! session only): `fly` holds the kart and flies the camera on the kart's own controls (gas and
//! brake forward and back, steer sideways, hop up, look-back down, boost four times as fast) and
//! the mouse held right to look; `tripod` stands the camera where it is and pans and zooms after
//! the kart as the showcase's tripod does, the controls the kart's again; `low` and `orbit` are
//! the showcase's other cameras ([`kartcraft::camera::CineCamera`]); `off` is the chase camera.

use bevy::prelude::*;
use kartcraft::camera::{CineCamera, CineMode, CineSubject};
use kartcraft::{input::Controls, KartView};

/// How fast the flying camera goes (yards a second), and boosted.
const FLY_SPEED: f32 = 10.0;
const FLY_BOOST: f32 = 4.0;
/// How far a pixel of mouse turns the flying camera (radians), and how far it may pitch.
const LOOK_PER_PIXEL: f32 = 0.0035;
const PITCH_LIMIT: f32 = 1.5;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum FreeMode {
    #[default]
    Off,
    Fly,
    Locked(CineMode),
}

impl FreeMode {
    /// `KartFreeCam`'s value: `off`, `fly`, or a cinematic camera's name.
    pub(crate) fn named(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "fly" => FreeMode::Fly,
            name => CineMode::named(name)
                .filter(|m| *m != CineMode::Chase)
                .map_or(FreeMode::Off, FreeMode::Locked),
        }
    }
}

/// The free camera: its mode, the flying eye and its look, and the cinematic camera it hands
/// over to.
#[derive(Resource, Default)]
pub(crate) struct FreeCam {
    mode: FreeMode,
    /// The flying eye and its yaw and pitch; `None` until it takes over from the chase camera.
    eye: Option<(Vec3, f32, f32)>,
    cine: Option<CineCamera>,
}

impl FreeCam {
    pub(crate) fn set(&mut self, mode: FreeMode) {
        if mode == self.mode {
            return;
        }
        self.mode = mode;
        self.cine = None;
        if mode == FreeMode::Off {
            self.eye = None;
        }
    }

    #[cfg(test)]
    pub(crate) fn mode(&self) -> FreeMode {
        self.mode
    }

    /// Whether the camera flies on the kart's controls, the kart held meanwhile.
    pub(crate) fn flying(&self) -> bool {
        self.mode == FreeMode::Fly
    }

    /// This frame's camera, or `None` for the kart's own. `controls` are the kart's, read this
    /// frame; `look` is the mouse's motion while it is held to look (pixels).
    pub(crate) fn frame(
        &mut self,
        view: &KartView,
        controls: &Controls,
        look: Vec2,
        dt: f32,
    ) -> Option<(Transform, f32)> {
        let chase = view.camera?;
        match self.mode {
            FreeMode::Off => None,
            FreeMode::Fly => {
                let (mut eye, mut yaw, mut pitch) = self.eye.unwrap_or_else(|| {
                    let (yaw, pitch, _) = chase.rotation.to_euler(EulerRot::YXZ);
                    (chase.translation, yaw, pitch)
                });
                yaw -= look.x * LOOK_PER_PIXEL;
                pitch = (pitch - look.y * LOOK_PER_PIXEL).clamp(-PITCH_LIMIT, PITCH_LIMIT);
                let rotation = Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0);
                let forward = rotation * Vec3::NEG_Z;
                let right = rotation * Vec3::X;
                let along =
                    f32::from(u8::from(controls.accelerate)) - f32::from(u8::from(controls.brake));
                let up =
                    f32::from(u8::from(controls.hop)) - f32::from(u8::from(controls.look_back));
                let speed = if controls.hop_alt {
                    FLY_SPEED * FLY_BOOST
                } else {
                    FLY_SPEED
                };
                eye += (forward * along + right * controls.steer + Vec3::Y * up) * speed * dt;
                self.eye = Some((eye, yaw, pitch));
                Some((
                    Transform::from_translation(eye).with_rotation(rotation),
                    view.fov,
                ))
            }
            FreeMode::Locked(mode) => {
                let stand = self.eye.map_or(chase.translation, |(eye, ..)| eye);
                let cine = self
                    .cine
                    .get_or_insert_with(|| CineCamera::new(mode, stand));
                let forward = Vec3::new(-view.yaw.sin(), 0.0, -view.yaw.cos());
                let subject = CineSubject {
                    position: view.transform.translation,
                    forward,
                    velocity: forward * view.speed,
                };
                let chase_aim = (
                    chase.translation,
                    chase.translation + chase.forward() * 10.0,
                );
                let shot = cine.shot(&subject, Some(chase_aim), view.fov, dt);
                let t = Transform::from_translation(shot.eye).looking_at(shot.look, Vec3::Y);
                Some((t, shot.fov))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kart() -> KartView {
        KartView {
            camera: Some(Transform::from_xyz(0.0, 3.0, 8.0).looking_at(Vec3::ZERO, Vec3::Y)),
            fov: 1.0,
            ..default()
        }
    }

    #[test]
    fn the_modes_are_named_as_the_command_takes_them() {
        assert_eq!(FreeMode::named("fly"), FreeMode::Fly);
        assert_eq!(
            FreeMode::named(" Tripod "),
            FreeMode::Locked(CineMode::Tripod)
        );
        assert_eq!(FreeMode::named("orbit"), FreeMode::Locked(CineMode::Orbit));
        assert_eq!(FreeMode::named("low"), FreeMode::Locked(CineMode::Low));
        // The chase camera is `off`; anything else is too.
        assert_eq!(FreeMode::named("chase"), FreeMode::Off);
        assert_eq!(FreeMode::named("off"), FreeMode::Off);
        assert_eq!(FreeMode::named("nonsense"), FreeMode::Off);
    }

    #[test]
    fn flying_starts_at_the_chase_camera_and_moves_on_the_controls() {
        let mut cam = FreeCam::default();
        let view = kart();
        assert!(cam
            .frame(&view, &Controls::default(), Vec2::ZERO, 0.1)
            .is_none());
        cam.set(FreeMode::Fly);
        let (start, _) = cam
            .frame(&view, &Controls::default(), Vec2::ZERO, 0.1)
            .unwrap();
        assert!(start
            .translation
            .abs_diff_eq(Vec3::new(0.0, 3.0, 8.0), 1e-4));
        // A second of gas flies it FLY_SPEED yards along where it looks.
        let gas = Controls {
            accelerate: true,
            ..default()
        };
        let (moved, _) = cam.frame(&view, &gas, Vec2::ZERO, 1.0).unwrap();
        let went = moved.translation - start.translation;
        assert!((went.length() - FLY_SPEED).abs() < 1e-3);
        assert!(went.normalize().dot(*start.forward()) > 0.999);
        // Hop rises straight up, boosted four times as fast.
        let up = Controls {
            hop: true,
            hop_alt: true,
            ..default()
        };
        let (rose, _) = cam.frame(&view, &up, Vec2::ZERO, 1.0).unwrap();
        assert!((rose.translation - moved.translation)
            .abs_diff_eq(Vec3::Y * FLY_SPEED * FLY_BOOST, 1e-3));
    }

    #[test]
    fn the_tripod_stands_where_the_flight_ended_and_watches_the_kart() {
        let mut cam = FreeCam::default();
        let view = kart();
        cam.set(FreeMode::Fly);
        let gas = Controls {
            accelerate: true,
            ..default()
        };
        cam.frame(&view, &gas, Vec2::ZERO, 0.5);
        let (flown, _) = cam
            .frame(&view, &Controls::default(), Vec2::ZERO, 0.1)
            .unwrap();
        cam.set(FreeMode::Locked(CineMode::Tripod));
        let (stood, _) = cam
            .frame(&view, &Controls::default(), Vec2::ZERO, 0.1)
            .unwrap();
        assert!(stood.translation.abs_diff_eq(flown.translation, 1e-4));
        let to_kart = (view.transform.translation - stood.translation).normalize();
        assert!(stood.forward().dot(to_kart) > 0.95, "it looks at the kart");
        // Off hands the picture back to the kart's camera.
        cam.set(FreeMode::Off);
        assert!(cam
            .frame(&view, &Controls::default(), Vec2::ZERO, 0.1)
            .is_none());
    }
}
