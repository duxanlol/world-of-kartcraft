//! The driver in the kart's seat: the chair sit a WoW character holds there, and where its body
//! goes so the pose's pelvis rests on CTR's seat anchor and its feet stay above the kart's floor.
//! The fit is measured off the posed skeleton once per model; a model whose legs reach below
//! the floor reclines in the seat until they clear it, as a kart's seat lies back.

use std::collections::HashMap;
use std::sync::Arc;

use benilla_assets::{ModelAnimations, PoseClip, PoseSource};
use benilla_formats::AnimDataCatalog;
use bevy::math::Affine3A;
use bevy::prelude::*;

/// `AnimationData.dbc` 102 `SitChairLow`: thighs forward from the hips, shins down, the one sit
/// every playable model authors alike (`SitGround`, 97, kneels for some and crosses the legs
/// for others).
pub(crate) const SIT_CHAIR_LOW: u16 = 102;
/// `AnimationData.dbc` 91 `Mount`, the saddle: for a model without the chair sit.
const MOUNT: u16 = 91;
/// `AnimationData.dbc` 0 `Stand`.
const STAND: u16 = 0;
/// The driver's pose in a kart, in order of preference, each through the model's own fallback.
pub(crate) const SEAT_ANIMS: [u16; 3] = [SIT_CHAIR_LOW, MOUNT, STAND];

/// `Waist` (KeyBoneID 5), the pelvis that sits on the seat; `Root` (26) on a model without one.
const KEY_WAIST: i16 = 5;
const KEY_ROOT: i16 = 26;

/// The foot bones are the pelvis's lowest descendants in `Stand`, within this fraction of the
/// pelvis's height of the lowest: the toes (or hooves), never the cloth flaps nor a tail.
const FOOT_BAND: f32 = 0.05;

/// How far the lowest sole keeps above the kart's floor, meters in the kart's frame.
pub(crate) const FLOOR_CLEARANCE: f32 = 0.01;

/// The furthest a driver reclines to clear the floor, radians; past it the body lifts instead.
const MAX_RECLINE: f32 = 0.6;
/// The recline search's step, radians.
const RECLINE_STEP: f32 = 0.01;

/// Samples of the clip a fit bounds: a loop's breathing moves the feet.
const SAMPLES: usize = 8;

/// One model's seat fit, in its body frame (yards, before the unit's scale).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SeatFit {
    /// The animation the model plays in the seat, after its fallback.
    pub anim: u16,
    /// The pelvis bone's posed origin at the clip's start.
    pub pelvis: Vec3,
    /// Each foot's sole (its bone's posed origin less its sole depth) less the pelvis, at every
    /// sample of the clip.
    pub feet: Vec<Vec3>,
}

/// Where the body goes in the kart: its origin (the kart's frame, meters before the kart's
/// scale) and its rotation in the kart, at no steer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SeatPose {
    pub origin: Vec3,
    /// The recline about the body's own left-right axis, radians, legs up.
    pub recline: f32,
    /// How far the pelvis rides above the seat, meters, where the recline was not enough.
    pub lift: f32,
}

/// Every bone's posed origin in the model frame at `t` seconds into `clip`; an unkeyed channel
/// keeps its bind value, as the rig's evaluator does.
pub(crate) fn posed_origins(binds: &[Vec3], parents: &[i16], clip: &PoseClip, t: f32) -> Vec<Vec3> {
    let mut locals: Vec<Transform> = binds
        .iter()
        .map(|&b| Transform::from_translation(b))
        .collect();
    for b in &clip.bones {
        let Some(l) = locals.get_mut(b.bone as usize) else {
            continue;
        };
        if let Some(v) = b.translation.sample(t) {
            l.translation = v;
        }
        if let Some(q) = b.rotation.sample(t) {
            l.rotation = q;
        }
        if let Some(s) = b.scale.sample(t) {
            l.scale = s;
        }
    }
    let mut model = vec![Affine3A::IDENTITY; locals.len()];
    for i in 0..locals.len() {
        let parent = parents
            .get(i)
            .and_then(|&p| usize::try_from(p).ok())
            .filter(|&p| p < i)
            .map_or(Affine3A::IDENTITY, |p| model[p]);
        model[i] = parent * locals[i].compute_affine();
    }
    model.iter().map(|m| Vec3::from(m.translation)).collect()
}

/// The pelvis bone and the foot bones under it with each one's sole depth, found in the `stand`
/// pose, where the model stands on its origin: a foot bone's height there is how far its sole
/// lies below it (`HumanMale`'s toes 0.017 yd, `TaurenMale`'s hoof bones 0.31). A bind pose need
/// not stand, so it cannot say.
pub(crate) fn pelvis_and_feet(
    binds: &[Vec3],
    parents: &[i16],
    key_bones: &[i16],
    stand: &PoseClip,
) -> Option<(usize, Vec<(usize, f32)>)> {
    let pelvis = key_bones
        .iter()
        .position(|&k| k == KEY_WAIST)
        .or_else(|| key_bones.iter().position(|&k| k == KEY_ROOT))?;
    let standing = posed_origins(binds, parents, stand, 0.0);
    let under = |mut i: usize| {
        for _ in 0..=parents.len() {
            if i == pelvis {
                return true;
            }
            match parents.get(i).and_then(|&p| usize::try_from(p).ok()) {
                Some(p) => i = p,
                None => return false,
            }
        }
        false
    };
    let legs: Vec<usize> = (0..standing.len())
        .filter(|&i| i != pelvis && under(i))
        .collect();
    let lowest = legs
        .iter()
        .map(|&i| standing[i].y)
        .fold(f32::INFINITY, f32::min);
    let band = lowest + standing.get(pelvis)?.y * FOOT_BAND;
    let feet: Vec<(usize, f32)> = legs
        .into_iter()
        .filter(|&i| standing[i].y <= band)
        .map(|i| (i, standing[i].y.max(0.0)))
        .collect();
    (!feet.is_empty()).then_some((pelvis, feet))
}

/// Measure `clip` (playing `anim`, `duration` seconds) on a skeleton, its feet found in `stand`.
pub(crate) fn fit(
    binds: &[Vec3],
    parents: &[i16],
    key_bones: &[i16],
    stand: &PoseClip,
    anim: u16,
    clip: &PoseClip,
    duration: f32,
) -> Option<SeatFit> {
    let (pelvis, feet) = pelvis_and_feet(binds, parents, key_bones, stand)?;
    let at = posed_origins(binds, parents, clip, 0.0);
    let pelvis_at = at[pelvis];
    let mut rel = Vec::with_capacity(feet.len() * SAMPLES);
    for s in 0..SAMPLES {
        let p = if s == 0 {
            at.clone()
        } else {
            posed_origins(binds, parents, clip, duration * s as f32 / SAMPLES as f32)
        };
        // Each sample's soles against that sample's pelvis: the body is placed by its pelvis.
        rel.extend(
            feet.iter()
                .map(|&(f, sole)| p[f] - Vec3::Y * sole - p[pelvis]),
        );
    }
    Some(SeatFit {
        anim,
        pelvis: pelvis_at,
        feet: rel,
    })
}

/// The recline about the body's X axis (legs up for a body facing −Z).
fn recline_rotation(angle: f32) -> Quat {
    Quat::from_rotation_x(angle)
}

/// The lowest foot's height over the pelvis in the kart, meters, reclined by `angle`; `turn` is
/// the body's rotation in the kart without the recline.
fn lowest_foot(fit: &SeatFit, turn: Quat, angle: f32, ratio: f32) -> f32 {
    let r = turn * recline_rotation(angle);
    fit.feet
        .iter()
        .map(|&f| (r * f).y * ratio)
        .fold(f32::INFINITY, f32::min)
}

/// Seat a fitted body: its pelvis on `seat`, reclined only as far as its feet need to clear the
/// floor (the kart's origin height), then lifted for what the recline could not reach. `turn` is
/// the body's rotation in the kart at no steer; `ratio` its scale over the kart's (body yards to
/// kart meters).
pub(crate) fn seat_pose(fit: &SeatFit, seat: Vec3, turn: Quat, ratio: f32) -> SeatPose {
    let need = FLOOR_CLEARANCE - seat.y;
    let mut recline = 0.0;
    while recline < MAX_RECLINE && lowest_foot(fit, turn, recline, ratio) < need {
        recline += RECLINE_STEP;
    }
    let recline = recline.min(MAX_RECLINE);
    let lift = (need - lowest_foot(fit, turn, recline, ratio)).max(0.0);
    let pelvis = (turn * recline_rotation(recline)) * (fit.pelvis * ratio);
    SeatPose {
        origin: seat - pelvis + Vec3::Y * lift,
        recline,
        lift,
    }
}

/// Place a driver's body in its kart: `kart` the kart's transform, `turn` the body's rotation
/// in the kart at no steer and `lean` the steer's lean over it. The lean turns the body about
/// its pelvis. Without a fit, the body's origin sits on the seat.
pub(crate) fn place(
    kart: &Transform,
    seat: Vec3,
    turn: Quat,
    lean: Quat,
    fit: Option<&SeatFit>,
    body_scale: f32,
) -> (Vec3, Quat) {
    let Some(fit) = fit else {
        return (kart.transform_point(seat), kart.rotation * lean * turn);
    };
    let ratio = body_scale / kart.scale.x.max(1e-6);
    let pose = seat_pose(fit, seat, turn, ratio);
    let body = turn * recline_rotation(pose.recline);
    // The lean pivots about the pelvis, which stays put on the seat.
    let pelvis = seat + Vec3::Y * pose.lift;
    let origin = pelvis + lean * (pose.origin - pelvis);
    (kart.transform_point(origin), kart.rotation * lean * body)
}

/// The seat fit of each model, keyed by its shared pose bake (held, so its address is never
/// reused), measured on first sight.
#[derive(Resource, Default)]
pub(crate) struct SeatFits(HashMap<usize, (Arc<PoseSource>, Option<SeatFit>)>);

impl SeatFits {
    /// The fit for a body's model, from its rig and clips, through the model's fallbacks as the
    /// animation driver resolves them.
    pub(crate) fn get(
        &mut self,
        anims: &ModelAnimations,
        rig: &benilla_world::rig_anim::RigPose,
        catalog: Option<&AnimDataCatalog>,
    ) -> Option<&SeatFit> {
        let key = Arc::as_ptr(&anims.pose) as usize;
        self.0
            .entry(key)
            .or_insert_with(|| (anims.pose.clone(), measure(anims, rig, catalog)))
            .1
            .as_ref()
    }
}

/// [`fit`] for a loaded model: the first of [`SEAT_ANIMS`] it plays, its feet found in `Stand`.
fn measure(
    anims: &ModelAnimations,
    rig: &benilla_world::rig_anim::RigPose,
    catalog: Option<&AnimDataCatalog>,
) -> Option<SeatFit> {
    let pose_of = |id: u16| {
        let c = crate::creature_anim::find_resolved(anims, id, catalog)?;
        let node = anims.pose.node(c.node)?;
        Some((c, anims.pose.clips.get(node.clip as usize)?))
    };
    let (_, stand) = pose_of(STAND)?;
    let (clip, pose) = SEAT_ANIMS.iter().find_map(|&id| pose_of(id))?;
    let f = fit(
        rig.bind_translations(),
        &rig.parents,
        &rig.key_bones,
        stand,
        clip.anim_id,
        pose,
        clip.duration,
    );
    info!(
        "kart: seat fit: anim {} pelvis {:?}",
        clip.anim_id,
        f.as_ref().map(|f| f.pelvis)
    );
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The seat pose is the DBC's `SitChairLow`, falling back to the saddle.
    #[test]
    fn the_seat_pose_is_sit_chair_low() {
        assert_eq!(SEAT_ANIMS, [102, 91, 0]);
        let data = benilla_formats::wow_data_or_skip!();
        let mut chain = benilla_formats::open_chain(&data).expect("open chain");
        let cat = benilla_formats::load_anim_data_catalog(&mut chain).expect("AnimationData");
        assert_eq!(cat.name(SIT_CHAIR_LOW), Some("SitChairLow"));
        assert_eq!(cat.name(MOUNT), Some("Mount"));
        assert_eq!(cat.name(STAND), Some("Stand"));
    }

    fn a_fit(pelvis: Vec3, feet: &[Vec3]) -> SeatFit {
        SeatFit {
            anim: SIT_CHAIR_LOW,
            pelvis,
            feet: feet.to_vec(),
        }
    }

    #[test]
    fn a_short_body_sits_upright_on_the_seat() {
        let turn = Quat::from_rotation_y(std::f32::consts::PI);
        let f = a_fit(Vec3::new(0.0, 0.2, 0.1), &[Vec3::new(0.0, -0.15, -0.3)]);
        let seat = Vec3::new(0.0, 0.25, 0.0);
        let p = seat_pose(&f, seat, turn, 1.0);
        assert_eq!((p.recline, p.lift), (0.0, 0.0));
        assert!((p.origin + turn * f.pelvis - seat).length() < 1e-6, "{p:?}");
    }

    #[test]
    fn long_legs_recline_then_lift() {
        let turn = Quat::from_rotation_y(std::f32::consts::PI);
        let seat = Vec3::new(0.0, 0.25, 0.0);
        // Feet 0.4 down and 0.4 forward: a recline clears them.
        let f = a_fit(Vec3::ZERO, &[Vec3::new(0.0, -0.4, -0.4)]);
        let p = seat_pose(&f, seat, turn, 1.0);
        assert!(p.recline > 0.0 && p.lift == 0.0, "{p:?}");
        assert!(lowest_foot(&f, turn, p.recline, 1.0) + seat.y >= FLOOR_CLEARANCE - 1e-4);
        // Straight down a metre: the most recline, then a lift.
        let f = a_fit(Vec3::ZERO, &[Vec3::new(0.0, -1.0, 0.0)]);
        let p = seat_pose(&f, seat, turn, 1.0);
        assert!(
            (p.recline - MAX_RECLINE).abs() < 1e-6 && p.lift > 0.0,
            "{p:?}"
        );
        let foot = seat.y + p.lift + lowest_foot(&f, turn, p.recline, 1.0);
        assert!((foot - FLOOR_CLEARANCE).abs() < 1e-5, "{foot}");
    }

    /// The display ids of the playable models (`ChrRaces` Male/FemaleDisplayId).
    const PLAYABLE: [(u32, &str); 16] = [
        (49, "HumanMale"),
        (50, "HumanFemale"),
        (51, "OrcMale"),
        (52, "OrcFemale"),
        (53, "DwarfMale"),
        (54, "DwarfFemale"),
        (55, "NightElfMale"),
        (56, "NightElfFemale"),
        (57, "ScourgeMale"),
        (58, "ScourgeFemale"),
        (59, "TaurenMale"),
        (60, "TaurenFemale"),
        (1563, "GnomeMale"),
        (1564, "GnomeFemale"),
        (1478, "TrollMale"),
        (1479, "TrollFemale"),
    ];

    /// How one display sits in the kart as the game draws it (CTR's median seat at the host's
    /// model size), in the kart's frame, meters: the pose played, its seat pose, how far the
    /// pelvis lands from the seat, and the lowest foot bone and sole over the clip.
    struct Sitting {
        anim: u16,
        pose: SeatPose,
        pelvis_off: f32,
        foot: f32,
        sole: f32,
    }

    fn sit(
        chain: &mut benilla_formats::Chain,
        cat: &benilla_formats::CreatureCatalog,
        display: u32,
    ) -> Sitting {
        let seat = kartcraft::model::MEDIAN_SEAT;
        let kart = Transform::from_scale(Vec3::splat(
            super::super::MODEL_SIZE / kartcraft::METERS_PER_YARD,
        ));
        let turn = super::super::driver_turn();
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
        let (stand, _) = clip_of(STAND).expect("Stand");
        let (anim, (clip, duration)) = SEAT_ANIMS
            .iter()
            .find_map(|&id| Some((id, clip_of(id)?)))
            .expect("a seat pose");
        let f = fit(
            &binds,
            &parents,
            &ms.key_bones,
            &stand,
            anim,
            &clip,
            duration,
        )
        .unwrap_or_else(|| panic!("display {display}: a pelvis and feet"));
        let (at, rot) = place(&kart, seat, turn, Quat::IDENTITY, Some(&f), 1.0);
        let body = Transform::from_translation(at).with_rotation(rot);
        // Into the kart's frame, meters.
        let to_kart = |p: Vec3| {
            kart.compute_affine()
                .inverse()
                .transform_point3(body.transform_point(p))
        };
        let (pelvis_bone, feet) = pelvis_and_feet(&binds, &parents, &ms.key_bones, &stand).unwrap();
        let pelvis = to_kart(posed_origins(&binds, &parents, &clip, 0.0)[pelvis_bone]);
        // The lowest foot bone and the lowest sole beneath one, over the clip.
        let (foot, sole) = (0..SAMPLES)
            .flat_map(|s| {
                let p = posed_origins(
                    &binds,
                    &parents,
                    &clip,
                    duration * s as f32 / SAMPLES as f32,
                );
                feet.iter()
                    .map(move |&(i, depth)| (p[i], p[i] - Vec3::Y * depth))
                    .collect::<Vec<_>>()
            })
            .map(|(bone, sole)| (to_kart(bone).y, to_kart(sole).y))
            .fold((f32::INFINITY, f32::INFINITY), |(a, b), (f, s)| {
                (a.min(f), b.min(s))
            });
        Sitting {
            anim,
            pose: seat_pose(&f, seat, turn, 1.0 / kart.scale.x),
            pelvis_off: (pelvis - seat).length(),
            foot,
            sole,
        }
    }

    /// Seated in the chair sit, its pelvis on CTR's seat, its lowest foot bone and sole over the
    /// floor and nothing lifted: the body is in the kart, not standing in it.
    fn assert_seated(label: &str, s: &Sitting) {
        println!(
            "{label:<15} {:>4} {:>7.1}° {:>6.3}m {:>8.3}m {:>8.3}m {:>8.3}m",
            s.anim,
            s.pose.recline.to_degrees(),
            s.pose.lift,
            s.pelvis_off,
            s.foot,
            s.sole
        );
        assert_eq!(s.anim, SIT_CHAIR_LOW, "{label} authors SitChairLow");
        assert!(
            s.foot >= FLOOR_CLEARANCE - 1e-3 && s.sole >= FLOOR_CLEARANCE - 1e-3,
            "{label}: foot {} / sole {} under the floor",
            s.foot,
            s.sole
        );
        assert!(
            s.pelvis_off < 0.05,
            "{label}: pelvis {} off the seat",
            s.pelvis_off
        );
        assert!(
            s.pose.lift < 1e-3,
            "{label}: lifted {} off the seat",
            s.pose.lift
        );
    }

    const HEADER: &str = "model           anim  recline    lift  pelvis_d    foot_y    sole_y";

    /// Every playable model authors the chair sit, and seated in a kart with it, its pelvis is on
    /// CTR's seat and its lowest foot bone over the floor. The placement is checked by posing the
    /// skeleton through the transform [`place`] gives, at every sample of the clip.
    #[test]
    fn every_playable_model_sits_in_the_kart() {
        let data = benilla_formats::wow_data_or_skip!();
        let mut chain = benilla_formats::open_chain(&data).expect("open chain");
        let cat = benilla_formats::load_creature_catalog(&mut chain).expect("creature catalog");
        println!("{HEADER}");
        for (display, label) in PLAYABLE {
            assert_seated(label, &sit(&mut chain, &cat, display));
        }
    }

    /// Every skin the skin shop sells (`Kartcraft.cpp`'s `SKINS`) sits as a playable model does: a
    /// creature model with no chair sit (a gnoll) would stand in the kart.
    #[test]
    fn every_skin_sits_in_the_kart() {
        let data = benilla_formats::wow_data_or_skip!();
        let mut chain = benilla_formats::open_chain(&data).expect("open chain");
        let cat = benilla_formats::load_creature_catalog(&mut chain).expect("creature catalog");
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let cpp = std::fs::read_to_string(root.join("setup/vmangos/src/Kartcraft.cpp")).unwrap();
        let skins: Vec<(String, u32)> = cpp
            .lines()
            .filter_map(|l| {
                let l = l.trim().strip_prefix("{ \"")?.strip_suffix(" },")?;
                let (key, rest) = l.split_once("\", ")?;
                Some((key.to_string(), rest.split_once(", ")?.0.parse().ok()?))
            })
            .collect();
        assert!(!skins.is_empty(), "the server's SKINS");
        println!("{HEADER}");
        for (key, display) in skins {
            assert_seated(&key, &sit(&mut chain, &cat, display));
        }
    }
}
