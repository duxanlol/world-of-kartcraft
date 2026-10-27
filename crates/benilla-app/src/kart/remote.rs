//! Other players' karts. A unit wearing Summon Kart gets a kart of its own; its position packets
//! (applied by `net::motion` at its relay's pace) are sampled as they land and the kart is drawn
//! [`DELAY`] behind them, interpolated between samples rather than dead-reckoned at WoW's run
//! speed, which a kart outruns six times over. Its tilt comes from the ground under it, and its
//! steer, powerslide look, turbo fire and engine from the state its driver relays through the
//! server ("KARTCRAFT\tS\t...", [`state_line`]). The driver sits in its seat as ours does.

use std::collections::{HashMap, VecDeque};

use benilla_assets::coords::wow_to_bevy;
use benilla_protocol::SessionEvent;
use benilla_world::collision::WorldCollision;
use bevy::prelude::*;
use kartcraft::model::{KartRoot, KartSpawner, KartStatus, KartWheels};
use kartcraft::{KartView, NearbyKart};

use crate::net::{Guid, ObjectStore, RemoteMotion, SelfPlayer};

/// How far behind its last packet a remote kart is drawn (s): one heartbeat and a margin.
const DELAY: f32 = 0.15;
/// A flying kart's drawn pitch at most (radians, as ours reaches, `ctr_kart`'s dive), and its bank
/// at full steer.
const FLY_PITCH_DRAWN: f32 = 0.55;
const FLY_BANK_DRAWN: f32 = 0.6;
/// How long past its last sample a kart may coast before it holds (s).
const MAX_COAST: f32 = 0.25;
/// A relayed state this old no longer applies (s).
pub(super) const STATE_TTL: f32 = 0.5;
/// The model's wheel radius, meters.
pub(super) const WHEEL_RADIUS: f32 = 0.25;

/// What a driver relays about their kart.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Relayed {
    /// Front-wheel steer, −1 right to 1 left.
    pub(crate) steer: f32,
    /// The drawn kart's turn off its heading, radians, + left.
    pub(crate) drift: f32,
    /// The turbo flame's size, 0 none, else 4 to 8.
    pub(crate) fire: u8,
    pub(crate) engine: (u8, u8),
    pub(crate) grounded: bool,
    pub(crate) drifting: bool,
    pub(crate) boost_window: bool,
    pub(crate) failed_boost: bool,
    /// Hovering on water and flying: its modes (the flags' 16 and 32).
    pub(crate) afloat: bool,
    pub(crate) flying: bool,
    /// Whose kart it is (`crash`, `coco`, ...), for its colours.
    pub(crate) racer: String,
    /// The boost wheelie's frame ([`kartcraft::wheelie_frame`]), 0 none; absent from an older
    /// client's line.
    pub(crate) wheelie: u8,
    /// The paint and boost trail they wear ([`kartcraft::cosmetic`]), 0 none.
    pub(crate) paint: u8,
    pub(crate) trail: u8,
    /// The flag they fly ([`kartcraft::goods::FLAGS`]), 0 none.
    pub(crate) flag: u8,
    /// The race's scale they drive at ([`kartcraft::race_scale`]), 1 on an older client's line.
    pub(crate) scale: f32,
}

/// No state yet, or a stale one: a kart at rest at its own size. The scale is 1, as an older
/// client's line reads; 0 draws no kart and makes the wheels' spin, a speed over the radius, NaN.
impl Default for Relayed {
    fn default() -> Self {
        Self {
            steer: 0.0,
            drift: 0.0,
            fire: 0,
            engine: (0, 0),
            grounded: false,
            drifting: false,
            boost_window: false,
            failed_boost: false,
            afloat: false,
            flying: false,
            racer: String::new(),
            wheelie: 0,
            paint: 0,
            trail: 0,
            flag: 0,
            scale: 1.0,
        }
    }
}

/// The line our kart relays:
/// `S\t<steer>,<drift>,<fire>,<distortion>,<volume>,<flags>,<racer>,<wheelie>,<paint>,<trail>,
/// <flag>,<scale>` (the race's scale in hundredths); an older client reads the first seven and
/// ignores the rest. The flags: grounded 1, drifting 2, the boost window 4, a failed boost 8,
/// hovering on water 16, flying 32 (an older client ignores the last two).
pub(crate) fn state_line(view: &KartView) -> String {
    let flags = u8::from(view.grounded)
        | (u8::from(view.state == kartcraft::State::Drifting) << 1)
        | (u8::from(kartcraft::model::boost_window(view.meter)) << 2)
        | (u8::from(view.failed_boost) << 3)
        | (u8::from(view.afloat) << 4)
        | (u8::from(view.flying) << 5);
    format!(
        "KARTCRAFT\tS\t{},{},{},{},{},{},{},{},{},{},{},{}",
        (view.steer * 100.0).round() as i32,
        (view.drift_yaw * 1000.0).round() as i32,
        view.fire,
        view.engine.0,
        view.engine.1,
        flags,
        kartcraft::model::our_kart(),
        view.wheelie,
        kartcraft::cosmetic::ours().0,
        kartcraft::cosmetic::ours().1,
        kartcraft::goods::our_flag(),
        (kartcraft::race_scale() * 100.0).round() as i32
    )
}

/// Reads a relayed state line (the part after `KARTCRAFT\tS\t`).
pub(crate) fn parse_state(body: &str) -> Option<Relayed> {
    let racer = body
        .split(',')
        .nth(6)
        .map(str::trim)
        .filter(|r| r.len() <= 12 && r.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or_default()
        .to_string();
    let mut f = body
        .split(',')
        .take(6)
        .map(|v| v.trim().parse::<i32>().ok());
    let mut next = || f.next().flatten();
    let steer = next()?;
    let drift = next()?;
    let fire = next()?;
    let distortion = next()?;
    let volume = next()?;
    let flags = next()?;
    Some(Relayed {
        steer: (steer as f32 / 100.0).clamp(-1.0, 1.0),
        drift: (drift as f32 / 1000.0).clamp(-3.2, 3.2),
        fire: fire.clamp(0, 8) as u8,
        engine: (distortion.clamp(0, 255) as u8, volume.clamp(0, 255) as u8),
        grounded: flags & 1 != 0,
        drifting: flags & 2 != 0,
        boost_window: flags & 4 != 0,
        failed_boost: flags & 8 != 0,
        afloat: flags & 16 != 0,
        flying: flags & 32 != 0,
        racer,
        wheelie: field(body, 7).filter(|&w| w <= 21).unwrap_or(0),
        paint: field(body, 8)
            .filter(|&p| kartcraft::cosmetic::paint(p).is_some())
            .unwrap_or(0),
        trail: field(body, 9)
            .filter(|&t| kartcraft::cosmetic::trail(t).is_some())
            .unwrap_or(0),
        flag: field(body, 10)
            .filter(|&f| usize::from(f) <= kartcraft::goods::FLAGS.len())
            .unwrap_or(0),
        scale: body
            .split(',')
            .nth(11)
            .and_then(|v| v.trim().parse::<f32>().ok())
            // `nan` parses, and `clamp` keeps it.
            .filter(|h| h.is_finite())
            .map_or(1.0, |h| {
                (h / 100.0).clamp(kartcraft::MIN_RACE_SCALE, kartcraft::MAX_RACE_SCALE)
            }),
    })
}

/// The state line's `n`th field as a small number, if there and one.
fn field(body: &str, n: usize) -> Option<u8> {
    body.split(',').nth(n).and_then(|w| w.trim().parse().ok())
}

/// One remote kart.
pub(crate) struct Remote {
    pub(crate) kart: Entity,
    pub(crate) guid: u64,
    /// (time received, position, facing yaw), oldest first.
    samples: VecDeque<(f32, Vec3, f32)>,
    last_apply: f64,
    pub(crate) position: Vec3,
    pub(crate) velocity: Vec3,
    pub(crate) yaw: f32,
    spin: f32,
    /// Whose kart is drawn ("" until the driver's first state names it), in which paint.
    racer: String,
    paint: u8,
}

#[derive(Resource, Default)]
pub(crate) struct RemoteKarts {
    /// By the driver's unit entity.
    pub(crate) karts: HashMap<Entity, Remote>,
    /// The latest relayed state by driver guid, and when it came.
    states: HashMap<u64, (Relayed, f32)>,
}

impl RemoteKarts {
    /// The karts near enough to bump, for our session.
    pub(crate) fn nearby(&self, around: Vec3, within: f32) -> Vec<NearbyKart> {
        self.karts
            .values()
            .filter(|r| r.position.distance(around) < within)
            .map(|r| NearbyKart {
                position: r.position,
                velocity: r.velocity,
                weight: 256,
                id: super::weapons::low_guid(r.guid),
                ..Default::default()
            })
            .collect()
    }

    /// Whose kart a driver's is (`crash`, `cortex`, ...), as they last relayed it.
    pub(crate) fn racer(&self, guid: u64) -> &str {
        self.states.get(&guid).map_or("", |(s, _)| s.racer.as_str())
    }
}

/// Kart state relayed by other drivers.
pub(crate) fn on_chat(In(ev): In<SessionEvent>, time: Res<Time>, mut karts: ResMut<RemoteKarts>) {
    let SessionEvent::Chat(m) = ev else {
        return;
    };
    if !m.is_addon() {
        return;
    }
    let Some(body) = m.text.strip_prefix("KARTCRAFT\tS\t") else {
        return;
    };
    if let Some(state) = parse_state(body) {
        karts
            .states
            .insert(m.sender_guid, (state, time.elapsed_secs()));
    }
}

/// The pose `t` seconds into the samples: interpolated between the two around it, coasting a
/// little past the last.
fn sample_at(samples: &VecDeque<(f32, Vec3, f32)>, t: f32) -> Option<(Vec3, f32, Vec3)> {
    let (first, last) = (samples.front()?, samples.back()?);
    if t <= first.0 {
        return Some((first.1, first.2, Vec3::ZERO));
    }
    let velocity_of = |a: &(f32, Vec3, f32), b: &(f32, Vec3, f32)| {
        let dt = (b.0 - a.0).max(1e-3);
        (b.1 - a.1) / dt
    };
    if t >= last.0 {
        let v = samples
            .len()
            .checked_sub(2)
            .map_or(Vec3::ZERO, |i| velocity_of(&samples[i], last));
        let coast = (t - last.0).min(MAX_COAST);
        return Some((last.1 + v * coast, last.2, v));
    }
    let i = samples.iter().position(|s| s.0 > t)?;
    let (a, b) = (&samples[i - 1], &samples[i]);
    let k = ((t - a.0) / (b.0 - a.0).max(1e-3)).clamp(0.0, 1.0);
    let dyaw =
        (b.2 - a.2 + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    Some((a.1.lerp(b.1, k), a.2 + dyaw * k, velocity_of(a, b)))
}

/// The ground's normal under a kart at `at` facing `forward`, from four probes around it.
fn ground_normal(collide: &WorldCollision, at: Vec3, forward: Vec3) -> Vec3 {
    let right = forward.cross(Vec3::Y).normalize_or(Vec3::X);
    let probe = |offset: Vec3| {
        let from = at + offset + Vec3::Y * 2.0;
        collide
            .ray_body(from, Dir3::NEG_Y, 5.0)
            .map(|hit| from - Vec3::Y * hit.distance)
    };
    let (Some(f), Some(b), Some(l), Some(r)) = (
        probe(forward * 0.9),
        probe(-forward * 0.9),
        probe(-right * 0.5),
        probe(right * 0.5),
    ) else {
        return Vec3::Y;
    };
    let n = (r - l).cross(f - b).normalize_or(Vec3::Y);
    if n.y < 0.0 {
        -n
    } else {
        n
    }
}

/// Keeps a kart for each other driver, samples their packets, and draws and seats them.
#[allow(clippy::type_complexity)]
pub(crate) fn update(
    mut commands: Commands,
    frame: Res<bevy::diagnostic::FrameCount>,
    time: Res<Time>,
    collide: WorldCollision,
    mut karts: ResMut<RemoteKarts>,
    mut spawner: KartSpawner,
    mut units: Query<
        (
            Entity,
            &Guid,
            &ObjectStore,
            &RemoteMotion,
            &mut Transform,
            Option<&benilla_assets::ModelAnimations>,
            Option<&benilla_world::rig_anim::RigPose>,
        ),
        (Without<SelfPlayer>, Without<KartRoot>),
    >,
    mut kart_parts: Query<
        (
            &mut Transform,
            &mut Visibility,
            &mut KartWheels,
            &mut KartStatus,
            &mut kartcraft::goods::KartFlag,
        ),
        With<KartRoot>,
    >,
    weapons: Res<kartcraft::weapons::Weapons>,
    mut fits: ResMut<super::seat::SeatFits>,
    anim_data: Option<Res<crate::creature_anim::AnimData>>,
) {
    let catalog = anim_data.as_deref().map(|d| &d.0);
    let now = time.elapsed_secs();
    let dt = time.delta_secs();
    let karts = &mut *karts;
    let mut live = Vec::new();
    for (entity, guid, store, motion, mut body, anims, rig) in &mut units {
        if !super::drives_kart(store) {
            continue;
        }
        live.push(entity);
        let remote = match karts.karts.get_mut(&entity) {
            Some(r) => r,
            None => {
                let (racer, paint) = karts
                    .states
                    .get(&guid.0)
                    .map(|(s, _)| (s.racer.clone(), s.paint))
                    .unwrap_or_default();
                let Some(kart) = spawner.spawn_painted(&racer, paint) else {
                    continue;
                };
                karts.karts.insert(
                    entity,
                    Remote {
                        kart,
                        guid: guid.0,
                        samples: VecDeque::new(),
                        last_apply: f64::NAN,
                        position: wow_to_bevy(motion.wow_pos),
                        velocity: Vec3::ZERO,
                        yaw: motion.orientation,
                        spin: 0.0,
                        racer,
                        paint,
                    },
                );
                karts.karts.get_mut(&entity).unwrap()
            }
        };
        if motion.last_apply_ms != remote.last_apply {
            remote.last_apply = motion.last_apply_ms;
            remote
                .samples
                .push_back((now, wow_to_bevy(motion.last_apply_pos), motion.orientation));
            while remote.samples.front().is_some_and(|s| now - s.0 > 1.0) {
                remote.samples.pop_front();
            }
        }
        let Some((at, yaw, velocity)) = sample_at(&remote.samples, now - DELAY) else {
            continue;
        };
        remote.position = at;
        remote.velocity = velocity;
        remote.yaw = yaw;
        let state = karts
            .states
            .get(&remote.guid)
            .filter(|(_, t)| now - t < STATE_TTL);
        let state = state.map(|(s, _)| s.clone()).unwrap_or_default();
        // The driver named their kart, or painted it: redraw it so.
        if !state.racer.is_empty() && (state.racer != remote.racer || state.paint != remote.paint) {
            if let Some(kart) = spawner.spawn_painted(&state.racer, state.paint) {
                spawner.despawn(remote.kart);
                remote.kart = kart;
                remote.racer = state.racer.clone();
                remote.paint = state.paint;
            }
        }

        // The kart's frame: its heading (plus the powerslide's look) on the ground's normal; in
        // flight on the world's up, its nose along its climb or dive and banked by its steer, as
        // ours is drawn (`ctr_kart::Kart::pose`).
        let heading = Vec3::new(-yaw.sin(), 0.0, -yaw.cos());
        let n = if state.flying {
            Vec3::Y
        } else {
            ground_normal(&collide, at, heading)
        };
        let drawn = yaw + state.drift;
        let fwd = Vec3::new(-drawn.sin(), 0.0, -drawn.cos());
        let z = (fwd - n * fwd.dot(n)).normalize_or(fwd);
        let mut rotation = Quat::from_mat3(&Mat3::from_cols(n.cross(z), n, z));
        if state.flying {
            let flat = Vec3::new(velocity.x, 0.0, velocity.z).length();
            let pitch = velocity
                .y
                .atan2(flat.max(0.1))
                .clamp(-FLY_PITCH_DRAWN, FLY_PITCH_DRAWN);
            rotation = rotation
                * Quat::from_rotation_x(-pitch)
                * Quat::from_rotation_z(-state.steer * FLY_BANK_DRAWN);
        }
        let mut kart = Transform {
            translation: at,
            rotation,
            scale: Vec3::splat(kartcraft::model_scale_at(state.scale)),
        };
        // A blast's tumble or a squish, shown from the hit the server relayed.
        if let Some(r) = weapons.reaction(super::weapons::low_guid(remote.guid)) {
            kart.translation += rotation * r.offset;
            kart.rotation = rotation * r.rotation;
            kart.scale *= r.scale.max(Vec3::splat(0.01));
        } else if let Some((turn, s, lift)) = kartcraft::wheelie_frame(state.wheelie) {
            // Their boost's wheelie, as ours is drawn (`ctr_kart::Kart::pose`).
            kart.translation += rotation * (lift / kartcraft::lu_per_yard_at(state.scale));
            kart.rotation = rotation * Quat::from_mat3(&turn);
            kart.scale *= s;
        }
        let speed = Vec3::new(velocity.x, 0.0, velocity.z).length();
        let forward_speed = velocity.dot(heading);
        let radius = WHEEL_RADIUS * kartcraft::kart_scale() / kartcraft::race_scale() * state.scale
            / kartcraft::METERS_PER_YARD;
        remote.spin = (remote.spin + forward_speed / radius * dt).rem_euclid(std::f32::consts::TAU);
        if let Ok((mut t, mut vis, mut wheels, mut status, mut flag)) =
            kart_parts.get_mut(remote.kart)
        {
            flag.set_if_neq(kartcraft::goods::KartFlag(state.flag));
            *t = kart;
            *vis = Visibility::Visible;
            *wheels = KartWheels {
                steer: state.steer,
                spin: remote.spin,
            };
            *status = KartStatus {
                fire: state.fire,
                drifting: state.drifting,
                boost_window: state.boost_window,
                failed_boost: state.failed_boost,
                grounded: state.grounded,
                engine: state.engine,
                speed,
                // The wire carries the powerslide, not the skid flags: a remote kart marks the
                // ground through its drifts only.
                skids: (state.drifting, state.drifting),
                trail: state.trail,
                // On the water its tyres throw spray, as ours do.
                ground: if state.afloat {
                    kartcraft::Ground::Water
                } else {
                    kartcraft::Ground::Asphalt
                },
                ..Default::default()
            };
        }
        // The driver in the seat of their kart, as ours, hands on its wheel.
        let fit = anims
            .zip(rig)
            .and_then(|(anims, rig)| fits.get(anims, rig, catalog));
        let sit = fit.map_or_else(Default::default, |f| f.legs.clone());
        let scale = body.scale.x;
        (body.translation, body.rotation) = super::seat::place(
            &kart,
            kartcraft::model::seat_of(&remote.racer),
            super::driver_turn(),
            super::driver_lean(state.steer),
            fit,
            scale,
        );
        let wheel = kartcraft::model::steering_wheel_of(&remote.racer);
        let ring = super::wheel::Ring::of(&kart, &body, wheel);
        super::wheel::hold(&mut commands, entity, state.steer, ring, sit, &frame);
    }
    // Karts whose drivers left the kart or our sight.
    let gone: Vec<Entity> = karts
        .karts
        .keys()
        .filter(|e| !live.contains(e))
        .copied()
        .collect();
    for e in gone {
        if let Some(r) = karts.karts.remove(&e) {
            spawner.despawn(r.kart);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_state_line_round_trips() {
        let view = KartView {
            steer: -0.5,
            drift_yaw: 1.178,
            fire: 2,
            engine: (141, 200),
            grounded: true,
            state: kartcraft::State::Drifting,
            wheelie: 12,
            ..Default::default()
        };
        let line = state_line(&view);
        assert!(line.len() < 120, "{line}");
        let body = line.strip_prefix("KARTCRAFT\tS\t").unwrap();
        let s = parse_state(body).unwrap();
        assert_eq!(s.steer, -0.5);
        assert!((s.drift - 1.178).abs() < 1e-3);
        assert_eq!(
            (s.fire, s.engine, s.grounded, s.drifting),
            (2, (141, 200), true, true)
        );
        assert_eq!(s.wheelie, 12);
        assert_eq!((s.paint, s.trail), kartcraft::cosmetic::ours());
        assert!(!s.afloat && !s.flying);
        // The modes ride in the flags: hovering, then flying.
        for (afloat, flying) in [(true, false), (false, true)] {
            let line = state_line(&KartView {
                afloat,
                flying,
                ..view.clone()
            });
            let s = parse_state(line.strip_prefix("KARTCRAFT	S	").unwrap()).unwrap();
            assert_eq!((s.afloat, s.flying), (afloat, flying));
        }
        // An older client's line, without the wheelie and the cosmetics, still reads; a nonsense
        // one is none.
        let old: Vec<&str> = body.split(',').take(7).collect();
        let old = old.join(",");
        let s = parse_state(&old).unwrap();
        assert_eq!((s.wheelie, s.paint, s.trail), (0, 0, 0));
        let s = parse_state(&format!("{old},99,200,7")).unwrap();
        assert_eq!((s.wheelie, s.paint, s.trail), (0, 0, 0));
        let s = parse_state(&format!("{old},0,3,4")).unwrap();
        assert_eq!((s.paint, s.trail, s.flag), (3, 4, 0));
        // The flag after them; one past the catalogue is none.
        let s = parse_state(&format!("{old},0,3,4,1")).unwrap();
        assert_eq!(s.flag, 1);
        let s = parse_state(&format!("{old},0,3,4,99")).unwrap();
        assert_eq!(s.flag, 0);
        // The race's scale after the flag, in hundredths: ours on our own line, 1 on an older
        // client's, bounded.
        assert_eq!(parse_state(body).unwrap().scale, kartcraft::race_scale());
        assert_eq!(parse_state(&old).unwrap().scale, 1.0);
        assert_eq!(
            parse_state(&format!("{old},0,3,4,1,50")).unwrap().scale,
            0.5
        );
        let tiny = parse_state(&format!("{old},0,3,4,1,1")).unwrap().scale;
        assert_eq!(tiny, kartcraft::MIN_RACE_SCALE);
        // Not a number is 1, as none; no state at all is a kart at its own size too.
        let nan = parse_state(&format!("{old},0,3,4,1,nan")).unwrap().scale;
        assert_eq!(nan, 1.0);
        assert_eq!(Relayed::default().scale, 1.0);
        assert!(parse_state("1,2,3").is_none());
        assert!(parse_state("x,2,3,4,5,6").is_none());
    }

    #[test]
    fn samples_interpolate_and_coast() {
        let mut s = VecDeque::new();
        s.push_back((0.0, Vec3::ZERO, 0.0));
        s.push_back((0.1, Vec3::new(4.0, 0.0, 0.0), 0.2));
        let (at, yaw, v) = sample_at(&s, 0.05).unwrap();
        assert!((at.x - 2.0).abs() < 1e-4 && (yaw - 0.1).abs() < 1e-4);
        assert!((v.x - 40.0).abs() < 1e-2);
        // Past the last sample it coasts on, but not for ever.
        let (ahead, ..) = sample_at(&s, 0.2).unwrap();
        assert!((ahead.x - 8.0).abs() < 1e-3);
        let (held, ..) = sample_at(&s, 5.0).unwrap();
        assert!((held.x - (4.0 + 40.0 * MAX_COAST)).abs() < 1e-2);
    }
}
