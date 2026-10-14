//! The controller's rumble, as CTR shakes the pad (`GAMEPAD_ShockForce1`, the big motor, and
//! `GAMEPAD_ShockFreq`, the small one, each a strength 0 to 0xFF for a number of frames): a slam
//! into a wall and a crash into another kart, a hard landing, and the scrape along a wall.
//! [`Rumble`] collects the pulses a frame asks for; the host hands them to its gamepads.

use bevy::input::gamepad::{GamepadRumbleIntensity, GamepadRumbleRequest};
use bevy::prelude::*;

use crate::session::Feedback;
use crate::Event;

/// One shake: the big and small motors' strengths (0 to 1) and how long, seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pulse {
    pub strong: f32,
    pub weak: f32,
    pub seconds: f32,
}

/// `GAMEPAD_ShockForce1(d, frames, force)` with `GAMEPAD_ShockFreq(d, frames, 0)`.
fn shock(frames: u32, force: u8) -> Pulse {
    Pulse {
        strong: f32::from(force) / 255.0,
        weak: 0.0,
        seconds: frames as f32 / 30.0,
    }
}

/// `VehEmitter.c:1284-1296`: a landing from a jump faster than 22.0 (FP8) shakes 3 frames at
/// full force.
pub const LANDING_FALL: i32 = 22 * 256;
/// `VehPhysCrash.c:530`: a kart crash harder than this shakes.
const BUMP_MIN: i32 = 0x200;
/// `VEH_EMITTER_WALL_RUMBLE_FRAME_LIMIT` (`VehEmitter.c:113`): a scrape shakes this many frames
/// at most.
const WALL_FRAMES: u32 = 450;

/// The pulses this frame asks for, and the scrape's frame count.
#[derive(Resource, Default, Debug)]
pub struct Rumble {
    pub pulses: Vec<Pulse>,
    /// The host's setting: the pad stays still (CTR's vibration is on by default).
    pub off: bool,
    /// `frameAgainstWall`.
    wall_frames: u32,
}

impl Rumble {
    /// Our kart's events: a wall slam (`COLL.c:2635-2636`) and a kart crash
    /// (`VehPhysCrash.c:547-560`) shake 8 frames at 0x7F.
    pub fn observe(&mut self, event: &Event) {
        match *event {
            Event::Slam { .. } => self.pulses.push(shock(8, 0x7F)),
            Event::Bump { strength } if strength > BUMP_MIN => self.pulses.push(shock(8, 0x7F)),
            _ => {}
        }
    }

    /// The step's landing and scrapes: a hard landing shakes 3 frames at 0xFF; a scrape at
    /// speed 8 frames at 0x7F, refreshed every frame for its first 450 (`VehEmitter.c:534-550`).
    pub fn feedback(&mut self, fb: &Feedback, speed: i32) {
        if let Some((fall, _)) = fb.landing {
            if fall > LANDING_FALL {
                self.pulses.push(shock(3, 0xFF));
            }
        }
        let scraping = fb.wall_frames > 0 && speed.abs() > 2 * 256;
        if !scraping {
            self.wall_frames = 0;
        } else if self.wall_frames < WALL_FRAMES {
            self.wall_frames += u32::from(fb.wall_frames);
            self.pulses.push(shock(8, 0x7F));
        }
    }
}

/// Hands this frame's pulses to every gamepad the player drives with (none ignored,
/// [`crate::input::PadIgnored`]), unless the host turned the rumble off.
pub fn send(
    mut rumble: ResMut<Rumble>,
    pads: Query<Entity, (With<Gamepad>, Without<crate::input::PadIgnored>)>,
    requests: Option<ResMut<Messages<GamepadRumbleRequest>>>,
) {
    let pulses = std::mem::take(&mut rumble.pulses);
    let Some(mut requests) = requests.filter(|_| !rumble.off) else {
        return;
    };
    for pulse in pulses {
        for gamepad in &pads {
            requests.write(GamepadRumbleRequest::Add {
                gamepad,
                duration: std::time::Duration::from_secs_f32(pulse.seconds),
                intensity: GamepadRumbleIntensity {
                    strong_motor: pulse.strong,
                    weak_motor: pulse.weak,
                },
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crashes_landings_and_scrapes_shake_as_ctr_s_do() {
        let mut r = Rumble::default();
        r.observe(&Event::Slam { speed: 9000 });
        r.observe(&Event::Bump { strength: 0x100 });
        r.observe(&Event::Bump { strength: 0x900 });
        assert_eq!(r.pulses.len(), 2);
        assert!((r.pulses[0].strong - 127.0 / 255.0).abs() < 1e-6);
        assert!((r.pulses[0].seconds - 8.0 / 30.0).abs() < 1e-6);
        r.pulses.clear();
        let land = |fall| Feedback {
            frames: 1,
            wall_frames: 0,
            landing: Some((fall, 5000)),
        };
        r.feedback(&land(1000), 5000);
        assert!(r.pulses.is_empty());
        r.feedback(&land(6000), 5000);
        assert_eq!(r.pulses, [shock(3, 0xFF)]);
        r.pulses.clear();
        let scrape = Feedback {
            frames: 2,
            wall_frames: 2,
            landing: None,
        };
        // A scrape shakes at speed, not crawling.
        r.feedback(&scrape, 100);
        assert!(r.pulses.is_empty());
        for _ in 0..300 {
            r.feedback(&scrape, 4000);
        }
        // 450 frames of it, then still.
        assert_eq!(r.pulses.len(), 225);
    }
}
