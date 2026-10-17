//! The driver's hands on the wheel. The seat's chair sit leaves the arms on the lap, and no
//! animation every playable model shares holds a wheel (`Mount`'s reins sit low at the saddle);
//! so each arm reaches for its grip by a two-bone solve over the animated pose, in the pose
//! post-pass: the upper arm (KeyBoneID 1 right, 0 left) and the forearm turn so the wrist lands on
//! the wheel's rim, at ten to two, the wheel sized and placed from the model's own shoulders and
//! arms, facing the driver, and turning with the steer. The fingers close as on a weapon's grip.

use benilla_world::rig_anim::RigPose;
use bevy::diagnostic::FrameCount;
use bevy::math::Affine3A;
use bevy::prelude::*;

/// The wheel's centre, from the shoulders' midpoint: this share of the arm's reach out, this
/// far (radians) below the horizontal, so the elbows stay bent.
const REACH_SHARE: f32 = 0.65;
const DROP: f32 = 0.5;
/// The wheel's radius, a share of the shoulders' width, and at most this share of the arm's
/// reach: a gnome's arms are short for its shoulders, and a turned grip must stay in reach.
const RADIUS_SHARE: f32 = 0.45;
const RADIUS_REACH: f32 = 0.3;
/// The grips, radians round the rim from the top: ten to two.
const GRIP: f32 = std::f32::consts::FRAC_PI_3;
/// The wheel's turn at full lock, radians.
const LOCK: f32 = std::f32::consts::FRAC_PI_2;
/// The elbow's lean: out from the body, down, and back toward the driver, per unit of each.
const POLE_OUT: f32 = 0.6;
const POLE_DOWN: f32 = 1.0;
const POLE_BACK: f32 = 0.3;

/// On a driver's body while it sits in a kart: the steer (−1 right to 1 left) and the frame it
/// was set, so a body left out of the kart lets go.
#[derive(Component)]
pub(crate) struct Wheel {
    pub(crate) steer: f32,
    pub(crate) frame: u32,
    arms: Option<[Arm; 2]>,
}

impl Wheel {
    pub(crate) fn new(steer: f32, frame: u32) -> Self {
        Self {
            steer,
            frame,
            arms: None,
        }
    }
}

/// One arm's bones, and the rotations the solve composed on (the animated ones) and last wrote:
/// a bone still holding what was written was not re-keyed, so its base stands.
#[derive(Clone, Copy)]
struct Arm {
    upper: usize,
    fore: usize,
    hand: usize,
    base: [Quat; 2],
    last: [Quat; 2],
}

/// The seat code's per-frame mark: this body drives, steering `steer`.
pub(crate) fn hold(commands: &mut Commands, body: Entity, steer: f32, frame: &FrameCount) {
    commands.entity(body).insert(Wheel::new(steer, frame.0));
}

/// The arm whose upper bone has KeyBoneID `arm_key`: the upper arm, its child toward the fingers
/// (KeyBoneIDs `fingers`) and that one's child.
fn find_arm(rig: &RigPose, arm_key: i16, fingers: std::ops::RangeInclusive<i16>) -> Option<Arm> {
    let upper = rig.key_bones.iter().position(|&k| k == arm_key)?;
    let finger = fingers
        .clone()
        .find_map(|k| rig.key_bones.iter().position(|&b| b == k))?;
    // The finger's ancestors, up to the upper arm.
    let mut path = vec![finger];
    let mut i = finger;
    while i != upper {
        i = usize::try_from(*rig.parents.get(i)?).ok()?;
        path.push(i);
        if path.len() > 16 {
            return None;
        }
    }
    // [finger .. hand, fore, upper]
    let n = path.len();
    (n >= 3).then(|| Arm {
        upper,
        fore: path[n - 2],
        hand: path[n - 3],
        base: [Quat::IDENTITY; 2],
        last: [Quat::NAN; 2],
    })
}

/// Both arms, right then left.
fn find_arms(rig: &RigPose) -> Option<[Arm; 2]> {
    find_arm(rig, 1, 8..=12)
        .zip(find_arm(rig, 0, 13..=17))
        .map(|(r, l)| [r, l])
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

/// The wheel's grips (right, left) and its facing, in the model frame (+Y up, −Z forward, +X the
/// driver's right), from the shoulders and the arms' reach, turned by `steer`.
pub(crate) fn grips(right: Vec3, left: Vec3, reach: f32, steer: f32) -> ([Vec3; 2], Vec3, Vec3) {
    let mid = (right + left) / 2.0;
    let out = REACH_SHARE * reach;
    let centre = mid + Vec3::NEG_Z * out * DROP.cos() + Vec3::NEG_Y * out * DROP.sin();
    let radius = (RADIUS_SHARE * right.distance(left)).min(RADIUS_REACH * reach);
    // The wheel faces the driver: its axis from the centre to the shoulders.
    let axis = (mid - centre).normalize_or(Vec3::Z);
    let up = (Vec3::Y - axis * Vec3::Y.dot(axis)).normalize_or(Vec3::Y);
    let side = up.cross(axis);
    // A left steer (+) turns the wheel's top to the left.
    let turn = -steer.clamp(-1.0, 1.0) * LOCK;
    let at = |angle: f32| centre + radius * (angle.sin() * side + angle.cos() * up);
    ([at(GRIP + turn), at(-GRIP + turn)], side, axis)
}

/// The pose post-pass: every driver's arms on the wheel; a body no longer marked this frame lets
/// go, its arms back on their animated pose.
pub(crate) fn grip_wheel(
    mut commands: Commands,
    frame: Res<FrameCount>,
    mut drivers: Query<
        (Entity, &mut Wheel, &mut RigPose),
        Without<benilla_world::rig_anim::AnimParked>,
    >,
) {
    for (entity, mut wheel, mut rig) in &mut drivers {
        let rig = &mut *rig;
        let (stamp, steer) = (wheel.frame, wheel.steer);
        if wheel.arms.is_none() {
            wheel.arms = find_arms(rig);
        }
        let Some(arms) = wheel.arms.as_mut() else {
            continue;
        };
        // Each bone's animated rotation this frame.
        for arm in arms.iter_mut() {
            for (k, bone) in [arm.upper, arm.fore].into_iter().enumerate() {
                let cur = rig.locals[bone].rotation;
                if cur != arm.last[k] {
                    arm.base[k] = cur;
                }
                rig.locals[bone].rotation = arm.base[k];
            }
        }
        if frame.0.wrapping_sub(stamp) > 1 {
            // Out of the kart: the animated arms stand, and the mark goes.
            rig.pose_dirty = true;
            commands.entity(entity).remove::<Wheel>();
            continue;
        }
        reach_wheel(rig, arms, steer);
        rig.pose_dirty = true;
    }
}

/// Each arm of `rig` (its animated pose in the locals) onto the wheel's grips, turned by `steer`.
fn reach_wheel(rig: &mut RigPose, arms: &mut [Arm; 2], steer: f32) {
    let at = |rig: &RigPose, i| Vec3::from(model_of(&rig.locals, &rig.parents, i).translation);
    let reach = arms
        .iter()
        .map(|a| {
            at(rig, a.upper).distance(at(rig, a.fore)) + at(rig, a.fore).distance(at(rig, a.hand))
        })
        .sum::<f32>()
        / 2.0;
    let (targets, side, axis) = grips(at(rig, arms[0].upper), at(rig, arms[1].upper), reach, steer);
    for (k, arm) in arms.iter_mut().enumerate() {
        let out = if k == 0 { side } else { -side };
        let pole = out * POLE_OUT + Vec3::NEG_Y * POLE_DOWN + axis * POLE_BACK;
        let (a, b, c) = (at(rig, arm.upper), at(rig, arm.fore), at(rig, arm.hand));
        let (b2, c2) = two_bone(a, b, c, targets[k], pole);
        // The upper arm turns by `r1` in the model frame, the forearm by `r2` after it.
        let r1 = Quat::from_rotation_arc((b - a).normalize_or_zero(), (b2 - a).normalize_or_zero());
        let c1 = a + r1 * (c - a);
        let r2 =
            Quat::from_rotation_arc((c1 - b2).normalize_or_zero(), (c2 - b2).normalize_or_zero());
        let parent = usize::try_from(rig.parents[arm.upper]).ok();
        let q_parent = parent.map_or(Quat::IDENTITY, |p| {
            model_of(&rig.locals, &rig.parents, p)
                .to_scale_rotation_translation()
                .1
        });
        let q_upper = q_parent * rig.locals[arm.upper].rotation;
        let q_fore = q_upper * rig.locals[arm.fore].rotation;
        let upper = (q_parent.inverse() * r1 * q_upper).normalize();
        let fore = ((r1 * q_upper).inverse() * r2 * r1 * q_fore).normalize();
        rig.locals[arm.upper].rotation = upper;
        rig.locals[arm.fore].rotation = fore;
        arm.last = [upper, fore];
    }
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

    /// Every playable model in its seat pose (`SitChairLow` at its start), arms solved onto the
    /// wheel at three steers: each wrist on its grip (or at full stretch toward it), the elbows
    /// below the shoulders.
    #[test]
    fn every_playable_drivers_wrists_reach_the_wheel() {
        let data = benilla_formats::wow_data_or_skip!();
        let mut chain = benilla_formats::open_chain(&data).expect("chain");
        let cat = benilla_formats::load_creature_catalog(&mut chain).expect("creatures");
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
            let sit = benilla_formats::parse_m2_animations(&bytes)
                .into_iter()
                .find(|a| a.anim_id == super::super::seat::SIT_CHAIR_LOW)
                .expect("SitChairLow");
            let clip = benilla_assets::pose_clip(&sit, &ms);
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
            let mut arms = find_arms(&rig).unwrap_or_else(|| panic!("{display}: two arms"));
            let at =
                |rig: &RigPose, i| Vec3::from(model_of(&rig.locals, &rig.parents, i).translation);
            let posed = rig.locals.clone();
            for steer in [0.0, 0.7, -0.7] {
                rig.locals.clone_from(&posed);
                let reach = arms
                    .iter()
                    .map(|a| {
                        at(&rig, a.upper).distance(at(&rig, a.fore))
                            + at(&rig, a.fore).distance(at(&rig, a.hand))
                    })
                    .sum::<f32>()
                    / 2.0;
                let (want, _, _) = grips(
                    at(&rig, arms[0].upper),
                    at(&rig, arms[1].upper),
                    reach,
                    steer,
                );
                reach_wheel(&mut rig, &mut arms, steer);
                for (k, arm) in arms.iter().enumerate() {
                    let wrist = at(&rig, arm.hand);
                    // On its grip, or as near as the arm's full stretch gets (a gnome's at full
                    // lock).
                    let shoulder = at(&rig, arm.upper);
                    let short = (shoulder.distance(want[k]) - reach).max(0.0);
                    assert!(
                        wrist.distance(want[k]) < short + 0.005,
                        "{display} steer {steer} arm {k}: wrist {wrist} off its grip {} by {:.3}",
                        want[k],
                        wrist.distance(want[k])
                    );
                    assert!(
                        at(&rig, arm.fore).y < at(&rig, arm.upper).y,
                        "{display}: the elbow below the shoulder"
                    );
                }
            }
        }
    }

    #[test]
    fn the_grips_sit_at_ten_to_two_and_turn_with_the_steer() {
        let (r, l) = (Vec3::new(0.2, 1.5, 0.0), Vec3::new(-0.2, 1.5, 0.0));
        let ([gr, gl], _, _) = grips(r, l, 0.7, 0.0);
        assert!(
            gr.x > 0.0 && gl.x < 0.0 && (gr.y - gl.y).abs() < 1e-5,
            "{gr} {gl}"
        );
        assert!(
            gr.z < -0.3 && gr.y < 1.5,
            "out in front, below the shoulders: {gr}"
        );
        // A left steer turns the top of the wheel left: the right hand rises, the left drops.
        let ([sr, sl], _, _) = grips(r, l, 0.7, 0.5);
        assert!(sr.y > gr.y && sl.y < gl.y, "{sr} {sl}");
    }
}
