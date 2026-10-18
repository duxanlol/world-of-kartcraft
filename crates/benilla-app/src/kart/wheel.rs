//! The driver's hands on the wheel. The seat's chair sit leaves the arms on the lap, and no
//! animation every playable model shares holds a wheel (`Mount`'s reins sit low at the saddle);
//! so each arm reaches for the kart's own steering-wheel ring by a two-bone solve over the
//! animated pose, in the pose post-pass: the upper arm (KeyBoneID 1 right, 0 left) and the
//! forearm turn so the palm lands on the rim at ten to two. A driver whose arms fall short of the
//! ring leans in from the lower spine (KeyBoneID 4), the head kept level, then slides the grips
//! down the rim's near side, then slides forward in the seat, and past all three reaches as far
//! as the arm goes. The fingers close as
//! on a weapon's grip.

use std::sync::Arc;

use benilla_world::rig_anim::RigPose;
use bevy::diagnostic::FrameCount;
use bevy::math::Affine3A;
use bevy::prelude::*;

/// The ring's radius, meters in the kart's frame: CTR's karts' rims lie 0.15 to 0.16 m from
/// their `steering_wheel` anchor (Coco's, Roo's and Tiny's a little smaller).
const RING_RADIUS: f32 = 0.155;
/// The ring's tilt from upright, its top away from the driver, radians: Crash's rim runs from
/// (0, 0.458, 0.321) at the bottom to (0, 0.770, 0.430) at the top, 19°.
const RING_TILT: f32 = 0.34;
/// The grips, radians round the rim from the top: ten to two; a driver whose arms fall short
/// slides them down toward the rim's near, lower side, as far as twenty to four.
const GRIP: f32 = std::f32::consts::FRAC_PI_3;
const LOW_GRIP: f32 = 2.0 * std::f32::consts::FRAC_PI_3;
const GRIP_STEP: f32 = 0.1;
/// How far the grips slide round the rim at full lock, radians: the drawn ring does not turn
/// (CTR's `turn` frames are exported straight ahead), so the hands only hint at the steer.
const LOCK: f32 = std::f32::consts::FRAC_PI_8;
/// The furthest the driver leans in from the lower spine to reach the ring, radians, and the
/// search's step.
const MAX_LEAN: f32 = 0.5;
const LEAN_STEP: f32 = 0.05;
/// The furthest a driver still short of the ring slides forward in the seat, a share of the
/// ring's radius: CTR sits its small drivers nearer the wheel, and ours is drawn where it is.
const MAX_SLIDE: f32 = 1.0;
/// The palm's centre, this share of the way from the wrist to the knuckles.
const PALM: f32 = 0.5;
/// The hand on the rim points forward and this much in toward the wheel's centre, its knuckles
/// along the rim; the wrist bends at most this far (radians), the forearm's roll taking the rest.
const HAND_IN: f32 = 0.3;
const WRIST_BEND: f32 = 1.4;
/// The elbow's lean: out from the body, down, and back toward the driver, per unit of each.
const POLE_OUT: f32 = 0.6;
const POLE_DOWN: f32 = 1.0;
const POLE_BACK: f32 = 0.3;

/// `KeyBoneID`s of the right hand's fingers (the left's are 5 on): most models key only the ring
/// finger and the thumb, a few all five, a tauren's three (index, middle and thumb).
const KEY_INDEX: i16 = 8;
const KEY_MIDDLE: i16 = 9;
const KEY_PINKY: i16 = 10;
const KEY_RING: i16 = 11;
const KEY_THUMB: i16 = 12;

/// `KeyBoneID` 4 `SpineLow` and 6 `Head`.
const KEY_SPINE_LOW: i16 = 4;
const KEY_HEAD: i16 = 6;

/// The steering wheel in a seated body's model frame (+Y up, −Z forward): the ring's centre, the
/// unit vectors to its top and to the driver's right, and its radius.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Ring {
    pub(crate) centre: Vec3,
    pub(crate) up: Vec3,
    pub(crate) right: Vec3,
    pub(crate) radius: f32,
}

impl Ring {
    /// The ring centred at `centre` (meters, the frame of the kart drawn at `kart`), seen from the
    /// model frame of the body drawn at `body`.
    pub(crate) fn of(kart: &Transform, body: &Transform, centre: Vec3) -> Self {
        let to_body = body.compute_affine().inverse() * kart.compute_affine();
        // The kart's frame: +Y up, +Z forward, +X left.
        let up = Vec3::new(0.0, RING_TILT.cos(), RING_TILT.sin());
        Self {
            centre: to_body.transform_point3(centre),
            up: to_body.transform_vector3(up).normalize_or(Vec3::Y),
            right: to_body.transform_vector3(Vec3::NEG_X).normalize_or(Vec3::X),
            radius: to_body.transform_vector3(Vec3::X * RING_RADIUS).length(),
        }
    }

    /// The face toward the driver.
    fn facing(&self) -> Vec3 {
        self.right.cross(self.up)
    }

    /// How a hand holds the rim at `grip`: pointing forward and in toward the centre, its
    /// knuckles along the rim toward the top.
    pub(crate) fn hold(&self, grip: Vec3) -> [Vec3; 2] {
        let out = (grip - self.centre).normalize_or(self.up);
        let mut along = self.facing().cross(out);
        if along.dot(self.up) < 0.0 {
            along = -along;
        }
        let point = (-self.facing() - out * HAND_IN).normalize_or(-self.facing());
        [point, along]
    }

    /// The grips (right, left) on the rim, `grip` radians either side of the top, turned by
    /// `steer` (−1 right to 1 left).
    pub(crate) fn grips(&self, grip: f32, steer: f32) -> [Vec3; 2] {
        // A left steer turns the wheel's top to the left.
        let turn = -steer.clamp(-1.0, 1.0) * LOCK;
        let at = |a: f32| self.centre + self.radius * (a.sin() * self.right + a.cos() * self.up);
        [at(grip + turn), at(-grip + turn)]
    }
}

/// On a driver's body while it sits in a kart: the steer, its kart's ring and the frame they were
/// set, so a body left out of the kart lets go.
#[derive(Component)]
pub(crate) struct Wheel {
    pub(crate) steer: f32,
    pub(crate) ring: Ring,
    pub(crate) frame: u32,
    bones: Option<Bones>,
    /// A made chair sit's bones and their locals (`seat::chair_legs`), empty for a model that
    /// authors its own, and each one's animated and last-written local.
    sit: Arc<[(usize, Transform)]>,
    sat: Vec<(Transform, Option<Transform>)>,
}

impl Wheel {
    /// The made sit's bones back on this frame's animated pose.
    fn restore_sit(&mut self, rig: &mut RigPose) {
        if self.sat.len() != self.sit.len() {
            self.sat = vec![(Transform::IDENTITY, None); self.sit.len()];
        }
        for (&(bone, _), (base, last)) in self.sit.iter().zip(&mut self.sat) {
            let Some(cur) = rig.locals.get_mut(bone) else {
                continue;
            };
            if Some(*cur) != *last {
                *base = *cur;
            }
            *cur = *base;
        }
    }

    /// The made sit's bones held over the animated pose.
    fn hold_sit(&mut self, rig: &mut RigPose) {
        for (&(bone, held), (_, last)) in self.sit.iter().zip(&mut self.sat) {
            if let Some(cur) = rig.locals.get_mut(bone) {
                *cur = held;
                *last = Some(held);
            }
        }
    }
}

/// A bone the solve turns, and the rotations it composed on (the animated one) and last wrote:
/// a bone still holding what was written was not re-keyed, so its base stands.
#[derive(Clone, Copy)]
struct Held {
    bone: usize,
    base: Quat,
    last: Quat,
}

impl Held {
    fn new(bone: usize) -> Self {
        Self {
            bone,
            base: Quat::IDENTITY,
            last: Quat::NAN,
        }
    }

    /// Back to this frame's animated rotation.
    fn restore(&mut self, rig: &mut RigPose) {
        let cur = rig.locals[self.bone].rotation;
        if cur != self.last {
            self.base = cur;
        }
        rig.locals[self.bone].rotation = self.base;
    }

    /// Turns the bone by `r` in the model frame.
    fn turn(&mut self, rig: &mut RigPose, r: Quat) {
        let q_parent = parent_rotation(rig, self.bone);
        let local = rig.locals[self.bone].rotation;
        let q = (q_parent.inverse() * r * q_parent * local).normalize();
        rig.locals[self.bone].rotation = q;
        self.last = q;
    }
}

/// One arm: the upper arm, the forearm, the hand (its bone at the wrist), the finger roots that
/// set which way the hand faces (the one it points to, the middle finger or else the ring
/// finger; the thumb side's, index or thumb; the far side's, pinky, ring or middle) and its palm's reach
/// past the wrist.
#[derive(Clone, Copy)]
struct Arm {
    upper: Held,
    fore: Held,
    hand: Held,
    fingers: [usize; 3],
    palm: f32,
}

/// The bones the solve turns: the arms, right then left, and the lower spine and head it leans.
#[derive(Clone)]
struct Bones {
    arms: [Arm; 2],
    lean: Option<[Held; 2]>,
    /// The skeleton's roots, which a slide moves, and their animated and last-written places.
    roots: Vec<(usize, Vec3, Vec3)>,
    /// How many bones the rig they were found in has.
    count: usize,
}

impl Bones {
    /// Every turned bone back on this frame's animated rotation.
    fn restore(&mut self, rig: &mut RigPose) {
        for h in self.lean.iter_mut().flatten() {
            h.restore(rig);
        }
        for (bone, base, last) in &mut self.roots {
            let cur = rig.locals[*bone].translation;
            if cur != *last {
                *base = cur;
            }
            rig.locals[*bone].translation = *base;
        }
        for arm in &mut self.arms {
            arm.upper.restore(rig);
            arm.fore.restore(rig);
            arm.hand.restore(rig);
        }
    }
}

/// The seat code's per-frame mark: this body drives, steering `steer`, its kart's wheel `ring`,
/// seated in `sit` where its model authors no chair sit of its own (`seat::SeatFit::legs`).
pub(crate) fn hold(
    commands: &mut Commands,
    body: Entity,
    steer: f32,
    ring: Ring,
    sit: Arc<[(usize, Transform)]>,
    frame: &FrameCount,
) {
    // Kept, not replaced: its bones hold the animated pose the solve composes on, and a fresh
    // one would take last frame's slid and turned pose for it, and slide again on top.
    let frame = frame.0;
    commands
        .entity(body)
        .entry::<Wheel>()
        .and_modify({
            let sit = sit.clone();
            move |mut w| {
                w.steer = steer;
                w.ring = ring;
                w.frame = frame;
                // A new model's sit holds bones of its own.
                if !Arc::ptr_eq(&w.sit, &sit) {
                    w.sit = sit;
                    w.sat.clear();
                }
            }
        })
        .or_insert(Wheel {
            steer,
            ring,
            frame,
            bones: None,
            sit,
            sat: Vec::new(),
        });
}

fn key(rig: &RigPose, k: i16) -> Option<usize> {
    rig.key_bones.iter().position(|&b| b == k)
}

/// The arm whose upper bone has KeyBoneID `arm_key`, its fingers' KeyBoneIDs `shift` past the
/// right hand's: the upper arm, its child toward the index finger and that one's child.
fn find_arm(rig: &RigPose, arm_key: i16, shift: i16) -> Option<Arm> {
    let upper = key(rig, arm_key)?;
    let finger = |keys: &[i16]| keys.iter().find_map(|&k| key(rig, k + shift));
    let point = finger(&[KEY_MIDDLE, KEY_RING, KEY_PINKY, KEY_INDEX])?;
    let inner = finger(&[KEY_INDEX, KEY_THUMB])?;
    let outer = finger(&[KEY_PINKY, KEY_RING, KEY_MIDDLE])?;
    // The finger's ancestors, up to the upper arm.
    let mut path = vec![point];
    let mut i = point;
    while i != upper {
        i = usize::try_from(*rig.parents.get(i)?).ok()?;
        path.push(i);
        if path.len() > 16 {
            return None;
        }
    }
    // [finger .. hand, fore, upper]
    let n = path.len();
    let hand = *path.get(n.checked_sub(3)?)?;
    Some(Arm {
        upper: Held::new(upper),
        fore: Held::new(path[n - 2]),
        hand: Held::new(hand),
        fingers: [point, inner, outer],
        palm: PALM * at(rig, hand).distance(at(rig, point)),
    })
}

fn find_bones(rig: &RigPose) -> Option<Bones> {
    let arms = find_arm(rig, 1, 0)
        .zip(find_arm(rig, 0, 5))
        .map(|(r, l)| [r, l])?;
    let lean = key(rig, KEY_SPINE_LOW)
        .zip(key(rig, KEY_HEAD))
        .map(|(s, h)| [Held::new(s), Held::new(h)]);
    let roots = (0..rig.parents.len())
        .filter(|&i| rig.parents[i] < 0)
        .map(|i| (i, Vec3::ZERO, Vec3::NAN))
        .collect();
    Some(Bones {
        arms,
        lean,
        roots,
        count: rig.locals.len(),
    })
}

/// Bone `i`'s model-space affine from the locals.
fn model_of(locals: &[Transform], parents: &[i16], i: usize) -> Affine3A {
    let mut m = locals[i].compute_affine();
    let mut b = parents.get(i).copied().unwrap_or(-1);
    while let Ok(p) = usize::try_from(b) {
        m = locals[p].compute_affine() * m;
        b = parents.get(p).copied().unwrap_or(-1);
    }
    m
}

/// Bone `i`'s origin in the model frame.
fn at(rig: &RigPose, i: usize) -> Vec3 {
    Vec3::from(model_of(&rig.locals, &rig.parents, i).translation)
}

/// The model-frame rotation of bone `i`'s parent.
fn parent_rotation(rig: &RigPose, i: usize) -> Quat {
    usize::try_from(rig.parents[i])
        .ok()
        .map_or(Quat::IDENTITY, |p| {
            model_of(&rig.locals, &rig.parents, p)
                .to_scale_rotation_translation()
                .1
        })
}

/// The two-bone solve: shoulder `a`, elbow `b`, wrist `c` (model space) reaching for `target`, the
/// elbow toward `pole`: where the elbow and the wrist go.
pub(crate) fn two_bone(a: Vec3, b: Vec3, c: Vec3, target: Vec3, pole: Vec3) -> (Vec3, Vec3) {
    let (l1, l2) = (a.distance(b), b.distance(c));
    let to = target - a;
    let d = to.length().clamp((l1 - l2).abs() + 1e-4, (l1 + l2) * 0.999);
    let n = to.normalize_or(Vec3::NEG_Z);
    let x = (l1 * l1 - l2 * l2 + d * d) / (2.0 * d);
    let h = (l1 * l1 - x * x).max(0.0).sqrt();
    let side = (pole - n * pole.dot(n)).normalize_or(Vec3::NEG_Y);
    (a + n * x + side * h, a + n * d)
}

/// The pose post-pass: every driver's arms on its kart's wheel; a body no longer marked lets go,
/// back on its animated pose.
pub(crate) fn grip_wheel(
    mut commands: Commands,
    frame: Res<FrameCount>,
    mut drivers: Query<
        (Entity, &mut Wheel, &mut RigPose),
        Without<benilla_world::rig_anim::AnimParked>,
    >,
) {
    for (entity, mut wheel, mut rig) in &mut drivers {
        let (rig, wheel) = (&mut *rig, &mut *wheel);
        // A new model under the body (a skin put on in the seat) has bones of its own.
        if wheel
            .bones
            .as_ref()
            .is_none_or(|b| b.count != rig.locals.len())
        {
            wheel.bones = find_bones(rig);
        }
        wheel.restore_sit(rig);
        if let Some(bones) = wheel.bones.as_mut() {
            bones.restore(rig);
        }
        if frame.0.wrapping_sub(wheel.frame) > 1 {
            // Out of the kart: the animated pose stands, and the mark goes.
            rig.pose_dirty = true;
            commands.entity(entity).remove::<Wheel>();
            continue;
        }
        // A made sit first: the arms reach from the body it seats.
        wheel.hold_sit(rig);
        let (steer, ring) = (wheel.steer, wheel.ring);
        if let Some(bones) = wheel.bones.as_mut() {
            reach_wheel(rig, bones, &ring, steer);
        }
        rig.pose_dirty = true;
    }
}

/// Where an arm's wrist goes to hold `ring` at `grip`: a palm back from the rim, along the hand.
fn wrist_at(ring: &Ring, arm: &Arm, grip: Vec3) -> Vec3 {
    grip - ring.hold(grip)[0] * arm.palm
}

/// How far an arm of `rig` falls short of its wrist's place at `grip` (model frame; ≤ 0 in
/// reach).
fn short_of(rig: &RigPose, ring: &Ring, arm: &Arm, grip: Vec3) -> f32 {
    let (a, b, c) = (
        at(rig, arm.upper.bone),
        at(rig, arm.fore.bone),
        at(rig, arm.hand.bone),
    );
    a.distance(wrist_at(ring, arm, grip)) - a.distance(b) - b.distance(c)
}

/// The worse of the two arms' [`short_of`].
fn shortfall(rig: &RigPose, ring: &Ring, arms: &[Arm; 2], grips: &[Vec3; 2]) -> f32 {
    arms.iter()
        .zip(grips)
        .map(|(arm, &g)| short_of(rig, ring, arm, g))
        .fold(f32::NEG_INFINITY, f32::max)
}

/// Which way an arm's hand faces: from the wrist to the finger it points by, and across from
/// its far side to its thumb side.
fn hand_faces(rig: &RigPose, arm: &Arm) -> [Vec3; 2] {
    let wrist = at(rig, arm.hand.bone);
    let [point, inner, outer] = arm.fingers.map(|i| at(rig, i));
    [
        (point - wrist).normalize_or(Vec3::NEG_Z),
        (inner - outer).normalize_or(Vec3::Y),
    ]
}

/// The rotation whose frame has `forward` its first axis and `across`, made square to it, its
/// second.
fn frame([forward, across]: [Vec3; 2]) -> Quat {
    let x = forward.normalize_or(Vec3::X);
    let y = (across - x * across.dot(x)).normalize_or(x.any_orthonormal_vector());
    Quat::from_mat3(&Mat3::from_cols(x, y, x.cross(y)))
}

/// `r` split into its twist about `axis` and the swing after it: `r = swing * twist`.
fn swing_twist(r: Quat, axis: Vec3) -> (Quat, Quat) {
    let along = axis * Vec3::new(r.x, r.y, r.z).dot(axis);
    let twist = Quat::from_xyzw(along.x, along.y, along.z, r.w);
    let twist = if twist.length_squared() < 1e-12 {
        Quat::IDENTITY
    } else {
        twist.normalize()
    };
    (twist, (r * twist.inverse()).normalize())
}

/// Each arm of `rig` (its animated pose in the locals) onto `ring`'s grips, turned by `steer`,
/// the body leaning in and the grips sliding down as far as they must and may; returns the lean
/// (radians) and the grips.
fn reach_wheel(rig: &mut RigPose, bones: &mut Bones, ring: &Ring, steer: f32) -> (f32, [Vec3; 2]) {
    let mut grips = ring.grips(GRIP, steer);
    let mut lean = 0.0;
    if let Some([spine, head]) = bones.lean.as_mut() {
        let base = rig.locals[spine.bone].rotation;
        while lean < MAX_LEAN && shortfall(rig, ring, &bones.arms, &grips) > 0.0 {
            lean = (lean + LEAN_STEP).min(MAX_LEAN);
            rig.locals[spine.bone].rotation = base;
            spine.turn(rig, Quat::from_axis_angle(ring.right, -lean));
        }
        if lean > 0.0 {
            // The head stays level, its eyes on the road.
            head.turn(rig, Quat::from_axis_angle(ring.right, lean));
        }
    }
    let mut grip = GRIP;
    while grip < LOW_GRIP && shortfall(rig, ring, &bones.arms, &grips) > 0.0 {
        grip = (grip + GRIP_STEP).min(LOW_GRIP);
        grips = ring.grips(grip, steer);
    }
    // Still short: the whole body slides forward along the seat toward the wheel.
    let short = shortfall(rig, ring, &bones.arms, &grips);
    if short > 0.0 {
        let ahead = -ring.facing();
        let ahead = Vec3::new(ahead.x, 0.0, ahead.z).normalize_or(Vec3::NEG_Z);
        let slide = ahead * short.min(MAX_SLIDE * ring.radius);
        for (bone, _, last) in &mut bones.roots {
            rig.locals[*bone].translation += slide;
            *last = rig.locals[*bone].translation;
        }
    }
    for (k, arm) in bones.arms.iter_mut().enumerate() {
        let out = if k == 0 { ring.right } else { -ring.right };
        let pole = out * POLE_OUT + Vec3::NEG_Y * POLE_DOWN + ring.facing() * POLE_BACK;
        let (a, b, c) = (
            at(rig, arm.upper.bone),
            at(rig, arm.fore.bone),
            at(rig, arm.hand.bone),
        );
        // The palm on the rim: the wrist a palm back from the grip, along the hand.
        let [point, along] = ring.hold(grips[k]);
        let (b2, c2) = two_bone(a, b, c, wrist_at(ring, arm, grips[k]), pole);
        // The upper arm turns by `r1` in the model frame, the forearm by `r2` after it.
        let r1 = Quat::from_rotation_arc((b - a).normalize_or_zero(), (b2 - a).normalize_or_zero());
        let c1 = a + r1 * (c - a);
        let r2 =
            Quat::from_rotation_arc((c1 - b2).normalize_or_zero(), (c2 - b2).normalize_or_zero());
        arm.upper.turn(rig, r1);
        arm.fore.turn(rig, r2);
        // Then the hand onto the rim: the turn from where it faces to the grip's, its roll about
        // the forearm taken by the forearm (the wrist stays put), the rest by the wrist.
        let r = frame([point, along]) * frame(hand_faces(rig, arm)).inverse();
        let axis = (c2 - b2).normalize_or_zero();
        let (twist, swing) = swing_twist(r, axis);
        arm.fore.turn(rig, twist);
        let bend = swing.to_axis_angle();
        let swing = Quat::from_axis_angle(bend.0, bend.1.min(WRIST_BEND));
        arm.hand.turn(rig, swing);
    }
    (lean, grips)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wrist_lands_on_the_target_and_the_elbow_bends_toward_the_pole() {
        let (a, b, c) = (
            Vec3::ZERO,
            Vec3::new(0.0, -0.4, 0.0),
            Vec3::new(0.0, -0.7, 0.0),
        );
        let target = Vec3::new(0.0, -0.1, -0.5);
        let pole = Vec3::new(0.5, -1.0, 0.0);
        let (b2, c2) = two_bone(a, b, c, target, pole);
        assert!(c2.distance(target) < 1e-4, "{c2}");
        assert!((b2.distance(a) - 0.4).abs() < 1e-4 && (c2.distance(b2) - 0.3).abs() < 1e-4);
        assert!(b2.x > 0.0 && b2.y < -0.05, "the elbow out and down: {b2}");
        // Out of reach: the arm straightens toward it.
        let (_, c3) = two_bone(a, b, c, Vec3::new(0.0, 0.0, -5.0), pole);
        assert!((c3.length() - 0.7).abs() < 0.01 && c3.z < -0.69);
    }

    /// The kart's ring, seen from a body seated facing the kart's front: in front of it, its top
    /// tipped away, its right the body's right.
    #[test]
    fn the_ring_faces_the_seated_driver_at_ten_to_two() {
        let kart = Transform::from_scale(Vec3::splat(1.4));
        let body = Transform::from_translation(Vec3::new(0.0, 0.3, 0.0))
            .with_rotation(super::super::driver_turn());
        let ring = Ring::of(&kart, &body, Vec3::new(0.0, 0.6, 0.4));
        assert!(ring.centre.z < -0.5 && ring.centre.y > 0.0, "{ring:?}");
        assert!(
            ring.up.z < -0.2 && ring.up.y > 0.9,
            "the top away: {}",
            ring.up
        );
        assert!(ring.right.x > 0.99, "{}", ring.right);
        assert!((ring.radius - 1.4 * RING_RADIUS).abs() < 1e-5);
        assert!(
            ring.facing().z > 0.9,
            "toward the driver: {}",
            ring.facing()
        );
        let [r, l] = ring.grips(GRIP, 0.0);
        assert!(
            r.x > 0.0 && l.x < 0.0 && (r.y - l.y).abs() < 1e-5,
            "{r} {l}"
        );
        assert!(r.y > ring.centre.y, "above the centre: {r}");
        // A left steer turns the top of the wheel left: the right hand rises, the left drops.
        let [sr, sl] = ring.grips(GRIP, 0.5);
        // The low grips sit below the centre, nearer the driver.
        let [lr, _] = ring.grips(LOW_GRIP, 0.0);
        assert!(lr.y < ring.centre.y && lr.z > r.z, "{lr}");
        assert!(sr.y > r.y && sl.y < l.y, "{sr} {sl}");
    }

    /// Every playable model seated in the kart as the game seats it (`SitChairLow` at its start,
    /// placed by [`super::super::seat::place`]), arms solved onto CTR's ring at three steers. The
    /// wrist lands a palm short of its grip, or the driver leans in fully and the arm reaches as
    /// far as it goes; the elbows stay below the shoulders. Run with `--nocapture` for the table, meters in
    /// the kart's frame.
    #[test]
    fn every_playable_driver_holds_the_karts_wheel() {
        use super::super::seat;
        let data = benilla_formats::wow_data_or_skip!();
        let mut chain = benilla_formats::open_chain(&data).expect("chain");
        let cat = benilla_formats::load_creature_catalog(&mut chain).expect("creatures");
        let kart = Transform::from_scale(Vec3::splat(
            super::super::MODEL_SIZE / kartcraft::METERS_PER_YARD,
        ));
        let meters = 1.0 / kart.scale.x;
        let mut held = 0;
        println!(
            "display  lean°  grip°  wrist→target r / l  wrist→centre  lap→centre               facing (point, along)"
        );
        for display in [
            49u32, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 1563, 1564, 1478, 1479,
        ] {
            let raw = cat.model(display).expect("display").model_path;
            let lower = raw.to_ascii_lowercase();
            let path = lower
                .strip_suffix(".mdx")
                .map_or(lower.clone(), |s| format!("{s}.m2"));
            let bytes = chain.read_file(&path).expect("model");
            let skel = benilla_formats::parse_m2_skeleton(&bytes).expect("skeleton");
            let ms = benilla_assets::model_skeleton(&skel);
            let binds: Vec<Vec3> = ms.joints.iter().map(|j| j.local_translation).collect();
            let parents: Vec<i16> = ms.joints.iter().map(|j| j.parent).collect();
            let anims = benilla_formats::parse_m2_animations(&bytes);
            let clip_of = |id: u16| {
                anims
                    .iter()
                    .find(|a| a.anim_id == id)
                    .map(|a| (benilla_assets::pose_clip(a, &ms), a.duration))
            };
            let (stand, _) = clip_of(0).expect("Stand");
            let (clip, duration) = clip_of(seat::SIT_CHAIR_LOW).expect("SitChairLow");
            let fit = seat::fit(
                &binds,
                &parents,
                &ms.key_bones,
                &stand,
                seat::SIT_CHAIR_LOW,
                &clip,
                duration,
            )
            .expect("a seat fit");
            let mut rig = RigPose::new(Entity::PLACEHOLDER, &ms);
            for b in &clip.bones {
                let l = &mut rig.locals[b.bone as usize];
                if let Some(v) = b.translation.sample(0.0) {
                    l.translation = v;
                }
                if let Some(q) = b.rotation.sample(0.0) {
                    l.rotation = q;
                }
            }
            let mut bones = find_bones(&rig).unwrap_or_else(|| panic!("{display}: two arms"));
            assert!(bones.lean.is_some(), "{display}: a lower spine and a head");
            let posed = rig.locals.clone();
            for steer in [0.0, 0.7, -0.7] {
                let (at_, rot) = seat::place(
                    &kart,
                    kartcraft::model::MEDIAN_SEAT,
                    super::super::driver_turn(),
                    super::super::driver_lean(steer),
                    Some(&fit),
                    1.0,
                );
                let body = Transform::from_translation(at_).with_rotation(rot);
                let ring = Ring::of(&kart, &body, kartcraft::model::MEDIAN_STEERING_WHEEL);
                rig.locals.clone_from(&posed);
                let lap = at(&rig, bones.arms[0].hand.bone).distance(ring.centre);
                let (lean, grips) = reach_wheel(&mut rig, &mut bones, &ring, steer);
                let mut off = [0.0; 2];
                let mut face = [[0.0; 2]; 2];
                for (k, arm) in bones.arms.iter().enumerate() {
                    let (a, b, c) = (
                        at(&rig, arm.upper.bone),
                        at(&rig, arm.fore.bone),
                        at(&rig, arm.hand.bone),
                    );
                    // The wrist a palm back from the rim, along the hand.
                    let [point, along] = ring.hold(grips[k]);
                    off[k] = c.distance(wrist_at(&ring, arm, grips[k]));
                    // The hand faces the rim: pointing forward and in, its knuckles along it,
                    // the index finger toward the top (the wrist's bend can leave it a little
                    // short).
                    let [f, x] = hand_faces(&rig, arm);
                    let x = (x - f * x.dot(f)).normalize_or_zero();
                    face[k] = [f.dot(point), x.dot(along)];
                    assert!(
                        f.dot(point) > 0.7 && x.dot(along) > 0.6,
                        "{display} steer {steer} arm {k}: the hand faces {f} across {x}, not {point} along {along}"
                    );
                    let short = short_of(&rig, &ring, arm, grips[k]).max(0.0);
                    // On the rim; or, short of it, leaning in all the way, on the low grips and
                    // slid forward, at full stretch.
                    let low = grips[k].distance(ring.grips(LOW_GRIP, steer)[k]) < 1e-5;
                    assert!(
                        off[k] < 0.01 || (lean >= MAX_LEAN - 1e-4 && low && off[k] < short + 0.02),
                        "{display} steer {steer} arm {k}: palm {off:?} off its grip, short \
                         {short:.3}, lean {lean:.2}"
                    );
                    // The elbow hangs: below the higher of the shoulder and the wrist (a
                    // gnome's wheel is above its shoulders); an arm at full stretch is straight.
                    assert!(
                        b.y < a.y.max(c.y) || off[k] >= 0.01,
                        "{display}: the elbow above both ends of the arm"
                    );
                }
                if steer == 0.0 {
                    held += usize::from(off.iter().all(|&d| d < 0.01));
                    let wrist = at(&rig, bones.arms[0].hand.bone).distance(ring.centre);
                    println!(
                        "{display:>7} {:>5.0}  {:>5.0}  {:>7.3} / {:<7.3}  {:>8.3}  {:>10.3}                           {:.2?}",
                        lean.to_degrees(),
                        ((grips[0] - ring.centre).dot(ring.up) / ring.radius)
                            .clamp(-1.0, 1.0)
                            .acos()
                            .to_degrees(),
                        off[0] * meters,
                        off[1] * meters,
                        wrist * meters,
                        lap * meters,
                        face,
                    );
                }
            }
        }
        // All but the gnomes hold it; theirs, a third of the reach to the ring, end a few
        // centimetres short after the lean and the slide.
        assert!(held >= 14, "{held} of 16 hold the wheel");
    }

    /// Held frame after frame, a body short of its wheel slides forward once, not again each
    /// frame: its root, which no clip re-keys here, stays a slide from where it stood. (Replacing
    /// the hold each frame took the slid pose for the animated one, and the body flew off.)
    #[test]
    fn a_driver_short_of_the_wheel_slides_once_and_stays() {
        use bevy::diagnostic::FrameCount;
        let data = benilla_formats::wow_data_or_skip!();
        let mut chain = benilla_formats::open_chain(&data).expect("chain");
        let cat = benilla_formats::load_creature_catalog(&mut chain).expect("creatures");
        let raw = cat.model(49).expect("display").model_path;
        let lower = raw.to_ascii_lowercase();
        let path = lower
            .strip_suffix(".mdx")
            .map_or(lower.clone(), |s| format!("{s}.m2"));
        let bytes = chain.read_file(&path).expect("model");
        let skel = benilla_formats::parse_m2_skeleton(&bytes).expect("skeleton");
        let ms = benilla_assets::model_skeleton(&skel);
        let rig = RigPose::new(Entity::PLACEHOLDER, &ms);
        let roots: Vec<(usize, Vec3)> = (0..rig.parents.len())
            .filter(|&i| rig.parents[i] < 0)
            .map(|i| (i, rig.locals[i].translation))
            .collect();

        // The ring well ahead of the body, past its reach.
        let kart = Transform::IDENTITY;
        let body = Transform::from_rotation(super::super::driver_turn());
        let ring = Ring::of(&kart, &body, Vec3::new(0.0, 1.0, 1.5));
        let mut app = App::new();
        app.insert_resource(FrameCount(0));
        let driver = app.world_mut().spawn(rig).id();
        app.add_systems(
            Update,
            (
                |mut f: ResMut<FrameCount>| f.0 += 1,
                move |mut commands: Commands, f: Res<FrameCount>| {
                    hold(&mut commands, driver, 0.0, ring, Default::default(), &f);
                },
                grip_wheel,
            )
                .chain(),
        );
        for _ in 0..60 {
            app.update();
        }
        let rig = app.world().get::<RigPose>(driver).expect("rig");
        for (i, start) in roots {
            let moved = rig.locals[i].translation.distance(start);
            assert!(
                moved <= MAX_SLIDE * ring.radius + 1e-4,
                "root {i} moved {moved:.3}, more than one slide ({:.3})",
                MAX_SLIDE * ring.radius
            );
        }
    }
}
