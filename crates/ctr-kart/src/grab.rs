//! Aku Aku's rescue: CTR's mask grab (`KS_MASK_GRABBED`, `VehStuckProc.c:404-720`) and the rev
//! state it hands the kart to (`VehStuckProc_RevEngine_*` with `boolMaskGrab`,
//! `VehStuckProc.c:944-1033`, `:1296-1321`).
//!
//! CTR does not fly the kart along the track. For `NoInputTimer` 1440 ms the kart stays where it
//! fell; from 960 ms left the mask comes down from 0x140 lu above it at 1 lu a millisecond,
//! growing in over 240 ms; once it reaches the kart it lifts it at half a lu a millisecond. At 0
//! the kart is put 0x80 lu over the ground of its respawn point (a cut: the camera's
//! `DIRECTION_CHANGED`) and the rev state lowers it, the mask still holding it, 2 lu a frame until
//! it is under 0x40 lu over that ground, when it lets go and the kart drives (and falls the rest).
//! The camera meanwhile keeps the height of a kart 0x40 lu over that ground
//! (`CAMERA_FLAG_MASK_GRAB`, `CAM.c:1221-1224`). No input drives the kart throughout; in the rev
//! state the gas revs the engine and a well-revved release earns the start boost.

use glam::Vec3;

/// `VEH_STUCK_MASK_GRAB_INITIAL_TIMER`: the grab's `NoInputTimer`, CTR-ms.
pub const GRAB_MS: i32 = 1440;
/// `VEH_STUCK_MASK_GRAB_SCALE_START_TIME`, `_SCALE_FULL_TIME`, `_SCALE_RAMP_TIME`: the mask is
/// drawn from 960 ms left, growing in over 240 ms.
const SCALE_START_MS: i32 = 960;
const SCALE_FULL_MS: i32 = 721;
const SCALE_RAMP_MS: i32 = 0xF0;
/// `VEH_STUCK_MASK_HEAD_Y_OFFSET`: where the mask starts over the kart, lu.
const MASK_START_LU: f32 = 320.0;
/// `VEH_STUCK_MASK_GRAB_LIFT_SHIFT`: the lift, `elapsedTimeMS << 7` in 1/256 lu.
const LIFT_SHIFT: i32 = 7;
/// `VEH_STUCK_RESPAWN_Y_OFFSET`: the respawn over its ground, lu (`VehBirth_TeleportSelf`).
pub const RESPAWN_LU: f32 = 128.0;
/// `VEH_STUCK_REV_MASK_RELEASE_HEIGHT` (0x4000 in 1/256 lu): the mask lets go under this, lu.
pub const RELEASE_LU: f32 = 64.0;
/// `VEH_STUCK_REV_MASK_DESCENT_STEP` (0x200 in 1/256 lu): the lowering, lu a frame.
pub const DESCENT_LU: f32 = 2.0;
/// `VEH_STUCK_REV_CAMERA_HEIGHT_OFFSET`: the camera's kart height over the ground, lu.
pub const CAMERA_LU: f32 = 64.0;

/// Where the grab is.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    /// `VehStuckProc_MaskGrab_*`: the mask coming down to the kart, then lifting it.
    Lift {
        /// `NoInputTimer`, CTR-ms.
        timer: i32,
        /// The mask's height, lu (`maskObj->pos.y`).
        mask_y: f32,
        /// `boolLiftingPlayer`.
        lifting: bool,
    },
    /// `VehStuckProc_RevEngine_*` with `boolMaskGrab`: lowered onto the respawn's ground.
    Lower,
}

/// What a frame of the grab did.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    /// The kart is held.
    Hold,
    /// The kart was put over its respawn point (`VehBirth_TeleportSelf`), facing this heading.
    Respawned(i32),
    /// The mask let go: this frame drives (`VehPhysProc_Driving_Init`).
    Released,
}

/// How the mask holding the kart is drawn: around `at` (lu, world space: the mask's own
/// position, `MASK_HEAD_ROT_WORLD_SPACE`) or, with `None`, worn on the kart as the mask item is;
/// at `scale` of its size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GrabMask {
    pub at: Option<Vec3>,
    pub scale: f32,
}

/// One mask grab under way.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaskGrab {
    phase: Phase,
    /// The respawn point's ground and the heading the kart is put back facing.
    ground: Vec3,
    heading: i32,
}

impl MaskGrab {
    /// `VehStuckProc_MaskGrab_Init` with the kart at `at`, to be put back on `ground` facing
    /// `heading`: the mask starts 0x140 lu over the kart.
    pub fn new(at: Vec3, ground: Vec3, heading: i32) -> Self {
        Self {
            phase: Phase::Lift {
                timer: GRAB_MS,
                mask_y: at.y + MASK_START_LU,
                lifting: false,
            },
            ground,
            heading,
        }
    }

    /// The respawn point's ground (`quadBlockHeight` after the respawn), lu.
    pub fn ground(&self) -> Vec3 {
        self.ground
    }

    /// Lifted and put back, being lowered: the rev state.
    pub fn lowering(&self) -> bool {
        self.phase == Phase::Lower
    }

    /// One frame of `ms` CTR-ms with the kart at `pos`, which it moves: `_Update`, then
    /// `_PhysLinear`, then `_Animate`, as the game's stages run.
    pub fn tick(&mut self, pos: &mut Vec3, ms: i32) -> Step {
        match &mut self.phase {
            Phase::Lift {
                timer,
                mask_y,
                lifting,
            } => {
                // `VehStuckProc_MaskGrab_Update` (`VehStuckProc.c:404-437`).
                *timer = (*timer - ms).max(0);
                if *timer == 0 {
                    *pos = self.ground + Vec3::Y * RESPAWN_LU;
                    self.phase = Phase::Lower;
                    // `VehStuckProc_RevEngine_PhysLinear` runs this same frame.
                    pos.y -= DESCENT_LU;
                    return Step::Respawned(self.heading);
                }
                // `VehStuckProc_MaskGrab_Animate` (`VehStuckProc.c:575-608`): nothing until the
                // mask shows.
                if *timer > SCALE_START_MS {
                    return Step::Hold;
                }
                if *lifting {
                    pos.y += (ms << LIFT_SHIFT) as f32 / 256.0;
                } else {
                    *mask_y -= ms as f32;
                }
                if *mask_y < pos.y {
                    *mask_y = pos.y;
                    *lifting = true;
                }
                Step::Hold
            }
            Phase::Lower => {
                // `VehStuckProc_RevEngine_Update` (`VehStuckProc.c:944-952`): held until under
                // the release height, then driving.
                if pos.y < self.ground.y + RELEASE_LU {
                    return Step::Released;
                }
                // `VehStuckProc_RevEngine_PhysLinear` (`VehStuckProc.c:1020-1026`).
                pos.y -= DESCENT_LU;
                Step::Hold
            }
        }
    }

    /// The mask as drawn with the kart at `pos`; `None` before it shows (scale 0).
    pub fn mask(&self, pos: Vec3) -> Option<GrabMask> {
        match self.phase {
            Phase::Lift { timer, mask_y, .. } => {
                if timer > SCALE_START_MS {
                    return None;
                }
                // `VehStuckProc.c:611-617`.
                let scale = if timer >= SCALE_FULL_MS {
                    ((SCALE_START_MS - timer) << 12) / SCALE_RAMP_MS
                } else {
                    4096
                };
                (scale > 0).then(|| GrabMask {
                    at: Some(Vec3::new(pos.x, mask_y, pos.z)),
                    scale: scale as f32 / 4096.0,
                })
            }
            // `VehStuckProc_RevEngine_Init` takes the mask back onto the kart
            // (`VehPickupItem_MaskUseWeapon(d, false)`).
            Phase::Lower => Some(GrabMask {
                at: None,
                scale: 1.0,
            }),
        }
    }

    /// The camera's kart height while lowered (`CAMERA_FLAG_MASK_GRAB`,
    /// `maskGrabHeightOffset`), lu.
    pub fn camera_height(&self) -> Option<f32> {
        self.lowering().then_some(self.ground.y + CAMERA_LU)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kart::MS;

    /// Ticks until the grab respawns the kart, recording the kart's height and the mask's each
    /// frame.
    fn lift(grab: &mut MaskGrab, pos: &mut Vec3) -> (Vec<f32>, Vec<Option<GrabMask>>, usize) {
        let (mut ys, mut masks) = (Vec::new(), Vec::new());
        for frame in 1.. {
            let step = grab.tick(pos, MS);
            if let Step::Respawned(_) = step {
                return (ys, masks, frame);
            }
            assert_eq!(step, Step::Hold);
            ys.push(pos.y);
            masks.push(grab.mask(*pos));
        }
        unreachable!()
    }

    #[test]
    fn the_mask_comes_down_lifts_the_kart_and_puts_it_back_in_ctr_s_time() {
        let fell = Vec3::new(10.0, -500.0, 20.0);
        let ground = Vec3::new(400.0, 30.0, -60.0);
        let mut grab = MaskGrab::new(fell, ground, 1024);
        let mut pos = fell;
        let (ys, masks, frame) = lift(&mut grab, &mut pos);
        // 1440 ms at 32 a frame: the 45th frame respawns.
        assert_eq!(frame, 45);
        // Nothing shows for the first 14 frames (more than 960 ms left), and at 960 the mask's
        // scale is still 0.
        assert!(masks[..15].iter().all(Option::is_none), "{masks:?}");
        // It grows in: 32/240 of its size a frame, full from 704 ms left (frame 23).
        assert_eq!(masks[15].unwrap().scale, 546.0 / 4096.0);
        assert!((masks[21].unwrap().scale - 224.0 / 240.0).abs() < 1e-3);
        assert_eq!(masks[22].unwrap().scale, 1.0);
        // It comes down from 320 lu over the kart at 32 lu a frame from frame 15 ...
        assert_eq!(masks[14], None);
        let first = masks[15].unwrap().at.unwrap();
        assert_eq!(first, Vec3::new(10.0, -500.0 + 320.0 - 64.0, 20.0));
        // ... reaches it on frame 25, and lifts it 16 lu a frame from frame 26 to 44.
        assert!(ys[..25].iter().all(|y| *y == -500.0));
        assert_eq!(masks[24].unwrap().at.unwrap().y, -500.0);
        assert_eq!(ys[25], -484.0);
        assert_eq!(ys[43], -500.0 + 19.0 * 16.0);
        for (y, m) in ys.iter().zip(&masks).skip(25) {
            assert_eq!(m.unwrap().at.unwrap().y, *y, "the mask holds the kart");
        }
        // Respawned 128 lu over its ground, lowered that same frame.
        assert_eq!(pos, ground + Vec3::Y * (128.0 - 2.0));
        assert!(grab.lowering());
        assert_eq!(grab.camera_height(), Some(30.0 + 64.0));
        assert_eq!(
            grab.mask(pos),
            Some(GrabMask {
                at: None,
                scale: 1.0
            })
        );
    }

    #[test]
    fn lowered_two_lu_a_frame_it_is_let_go_under_sixty_four_over_the_ground() {
        let ground = Vec3::new(0.0, 100.0, 0.0);
        let mut grab = MaskGrab::new(Vec3::ZERO, ground, 0);
        let mut pos = Vec3::ZERO;
        lift(&mut grab, &mut pos);
        let mut held = 0;
        loop {
            match grab.tick(&mut pos, MS) {
                Step::Hold => held += 1,
                Step::Released => break,
                s => panic!("{s:?}"),
            }
        }
        // From 126 over the ground down to 62: 32 more frames held, released on the 33rd.
        assert_eq!(held, 32);
        assert_eq!(pos.y, 100.0 + 62.0);
    }
}
