//! A driving session: one [`Kart`] ticked at CTR's 30 Hz over the world's collision gathered
//! around it, rebuilt on a worker thread as it drives, with the pose between ticks interpolated
//! and a chase camera behind it. The client and the offline demo both drive through this.

use std::sync::{mpsc, Arc, Mutex};

use benilla_world::collision::WorldCollision;
use bevy::prelude::*;
use ctr_kart::{Class, Collision, Event, Input, Kart, Modifiers, Pose, State};

use crate::{from_lu, heading_of, lu_per_yard, to_lu, yaw_of};

/// Half extents (yards) of the collision box gathered around the kart.
const GATHER_HALF: Vec3 = Vec3::new(120.0, 60.0, 120.0);
/// How far (yards) the kart drives from the box centre before it is rebuilt around it.
const RECENTRE: f32 = 50.0;
/// Seconds between rebuilds while driving, so terrain streamed in since becomes solid.
const REFRESH_SECS: f32 = 3.0;
/// The collision grid's cell, lu.
const CELL_LU: f32 = 160.0;
/// How far above the kart (lu) the look for any ground below starts, so ground a little above
/// it (a kart that has sunk into a bank) counts.
const VOID_PROBE_LU: f32 = 200.0;
/// How far down (lu) the look for anything below the kart goes: deeper than any fall.
const VOID_DEPTH_LU: f32 = 1.0e6;
/// Seconds off the ground before a falling kart is put back where it last stood.
const RESCUE_AFTER: f32 = 3.0;
/// CTR's wheel radius, lu, for the wheels' roll.
const WHEEL_RADIUS_LU: f32 = 10.0;

type Tri = [[f32; 3]; 3];
/// A grid the builder thread made, for the session epoch it was asked for.
type Built = (u64, Arc<Collision>);

/// The kart as drawn this frame: written by whoever runs the session, read by the model.
#[derive(Resource, Default, Clone)]
pub struct KartView {
    pub active: bool,
    /// The kart's origin (its bottom centre) and frame (model +Z forward, +Y up), Bevy world,
    /// yards; the scale converts the model's meters.
    pub transform: Transform,
    /// Facing, Bevy yaw.
    pub yaw: f32,
    /// Horizontal speed, yd/s.
    pub speed: f32,
    /// The chase camera, Bevy world.
    pub camera: Option<Transform>,
    /// The chase camera's vertical field of view, radians.
    pub fov: f32,
    /// Front-wheel steer, −1 (right) to 1 (left).
    pub steer: f32,
    /// The wheels' roll, radians.
    pub wheel_spin: f32,
    pub grounded: bool,
    pub state: State,
    /// Turbo time left, seconds.
    pub reserves: f32,
    /// The powerslide meter: filled 0..1 and in the red zone.
    pub meter: Option<(f32, bool)>,
    /// The drawn kart's turn off its heading (the powerslide look), radians, + left.
    pub drift_yaw: f32,
    /// The hang-time meter's airtime (CTR-ms) while a hop counts for it.
    pub jump: Option<i32>,
    /// Speed in CTR's units (1/256 lu per frame), the class's top speed, for the speedometer
    /// (the revs while revving on the grid).
    pub ctr_speed: i32,
    pub class_speed: i32,
    /// The turbo flame's size, 0 none, else 4 to 8.
    pub fire: u8,
    /// A boost pressed too early is puffing exhaust.
    pub failed_boost: bool,
    /// The engine's distortion and volume, as CTR mixes it.
    pub engine: (u8, u8),
}

#[derive(Default)]
pub struct Session {
    kart: Option<Kart>,
    world: Option<Arc<Collision>>,
    /// Asks the builder thread for a grid over these faces; the newest wins.
    build: Option<mpsc::Sender<(u64, Vec<Tri>)>>,
    built: Option<Mutex<mpsc::Receiver<Built>>>,
    /// Bumped at every start and stop, so a grid built for an old session is dropped.
    epoch: u64,
    /// The session's lu origin, Bevy world.
    origin: Vec3,
    /// Where the collision was last gathered around, and when.
    centre: Vec3,
    gathered_at: f32,
    /// Simulation time owed, seconds, and the last two ticks' poses to draw between.
    owed: f32,
    previous: Option<Pose>,
    current: Option<Pose>,
    /// CTR's chase camera and its last two shots, to draw between.
    camera: crate::camera::ChaseCamera,
    shots: Option<(crate::camera::Shot, crate::camera::Shot)>,
    wheel_spin: f32,
    /// Where the kart last stood on the ground, its heading, and how long it has been off it
    /// since (s): a kart lost in a fall is put back, as Aku Aku lifts a fallen racer.
    last_ground: Option<(Vec3, i32)>,
    falling: f32,
    /// Held where it stands across the ground (a race's countdown): it settles onto the ground
    /// but neither rolls nor slides, however the start slopes.
    pinned: bool,
}

/// Every collision triangle in the box around `centre`, in lu from `origin`; zero-area and
/// non-finite faces are left out.
fn gather(collide: &WorldCollision, centre: Vec3, origin: Vec3) -> Vec<Tri> {
    collide
        .faces_near_body(centre, GATHER_HALF, usize::MAX)
        .into_iter()
        .filter_map(|face| {
            let t = face.verts().map(|v| to_lu(v, origin));
            let solid = t.iter().all(|v| v.is_finite())
                && (t[1] - t[0]).cross(t[2] - t[0]).length_squared() > 1e-4;
            solid.then(|| t.map(|v| v.to_array()))
        })
        .collect()
}

type Builder = (
    mpsc::Sender<(u64, Vec<Tri>)>,
    mpsc::Receiver<(u64, Arc<Collision>)>,
);

/// The builder thread: the newest face set asked for, bucketed into a grid.
fn spawn_builder() -> std::io::Result<Builder> {
    let (send, jobs) = mpsc::channel::<(u64, Vec<Tri>)>();
    let (publish, built) = mpsc::channel();
    std::thread::Builder::new()
        .name("kart-collision".into())
        .spawn(move || {
            while let Ok(mut job) = jobs.recv() {
                while let Ok(newer) = jobs.try_recv() {
                    job = newer;
                }
                let (epoch, tris) = job;
                let grid = Arc::new(Collision::new(tris, CELL_LU));
                if publish.send((epoch, grid)).is_err() {
                    break;
                }
            }
        })?;
    Ok((send, built))
}

/// Nothing at all below the kart: the world there has not streamed in (or not into this grid
/// yet), and a kart is faster than the streaming. It waits where it was, `before` the tick,
/// rather than falling through. Any face below is world, ground or not: past a cliff's edge or
/// over a slope too steep to stand on there is only that steep face, and the kart falls onto it
/// and slides down as CTR's walls have it. Asking for ground alone held it there in mid-air.
fn hold_at_void(world: &Collision, kart: &mut Kart, before: Vec3) -> bool {
    let probe = kart.pose().position + Vec3::Y * VOID_PROBE_LU;
    let void = !kart.grounded() && world.ray(probe, Vec3::NEG_Y, VOID_DEPTH_LU).is_none();
    if void {
        kart.halt_at(before);
    }
    void
}

/// Puts the kart back where it stood `before` the tick across the ground, keeping the height
/// the tick gave it: it settles onto the ground but goes nowhere.
fn pin_in_place(kart: &mut Kart, before: Vec3) {
    let settled = kart.pose().position.y;
    kart.halt_at(Vec3::new(before.x, settled, before.z));
}

/// The pose to draw: the last two ticks blended by how far into the next one we are.
fn blend(a: &Pose, b: &Pose, t: f32) -> (Vec3, Quat) {
    let at = a.position.lerp(b.position, t);
    let qa = Quat::from_mat3(&a.basis).normalize();
    let qb = Quat::from_mat3(&b.basis).normalize();
    (at, qa.slerp(qb, t))
}

impl Session {
    pub fn active(&self) -> bool {
        self.kart.is_some()
    }

    pub fn kart(&self) -> Option<&Kart> {
        self.kart.as_ref()
    }

    pub fn kart_mut(&mut self) -> Option<&mut Kart> {
        self.kart.as_mut()
    }

    /// Puts a kart of `class` at `at` (Bevy world, the feet) facing `yaw`, over the collision
    /// around it, gathered now. Returns how many faces it found; with no ground under `at` yet (the
    /// world still streaming in) it starts nothing and returns 0, to be asked again.
    pub fn start(
        &mut self,
        collide: &WorldCollision,
        at: Vec3,
        yaw: f32,
        class: Class,
        now: f32,
    ) -> std::io::Result<usize> {
        if self.build.is_none() {
            let (send, built) = spawn_builder()?;
            self.build = Some(send);
            self.built = Some(Mutex::new(built));
        }
        self.epoch = self.epoch.wrapping_add(1);
        self.origin = at;
        self.centre = at;
        self.gathered_at = now;
        let world = Collision::new(gather(collide, at, at), CELL_LU);
        let faces = world.len();
        // The ground under us, from a little above the feet: a body the server put a little under
        // the terrain (its positions are not exact) must not start the kart beneath it.
        let Some(ground) = world.ground(Vec3::Y * 200.0, 800.0) else {
            return Ok(0);
        };
        // A little above the ground: the first ticks settle the kart onto it.
        let kart = Kart::new(class, ground.point + Vec3::Y * 4.0, heading_of(yaw));
        self.current = Some(kart.pose());
        self.previous = self.current;
        self.kart = Some(kart);
        self.world = Some(Arc::new(world));
        self.owed = 0.0;
        self.camera.reset();
        self.last_ground = None;
        self.falling = 0.0;
        self.shots = None;
        Ok(faces)
    }

    pub fn stop(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.kart = None;
        self.world = None;
        self.previous = None;
        self.current = None;
        self.camera.reset();
        self.last_ground = None;
        self.falling = 0.0;
        self.shots = None;
    }

    /// The session's lu origin, Bevy world: where [`to_lu`] measures from.
    pub fn origin(&self) -> Vec3 {
        self.origin
    }

    /// Pins the kart where it stands across the ground, or lets it go (a race's countdown: a
    /// start on a slope would otherwise roll it back off the line).
    pub fn pin(&mut self, on: bool) {
        self.pinned = on;
    }

    /// The chase camera's settings for the coming ticks: CTR's far zoom table (L2), and its
    /// look-back (R2 held).
    pub fn set_camera(&mut self, far: bool, look_back: bool) {
        self.camera.far = far;
        self.camera.look_back = look_back;
    }

    /// What shapes the chase camera from the next tick on ([`crate::camera::CameraTuning`]).
    pub fn tune_camera(&mut self, tuning: crate::camera::CameraTuning) {
        self.camera.tuning = tuning;
    }

    /// How far into the next tick the drawn frame is, 0 to 1, for drawing between ticks.
    pub fn tick_fraction(&self) -> f32 {
        (self.owed / ctr_kart::TICK).clamp(0.0, 1.0)
    }

    /// Runs the ticks `dt` owes and returns what happened in them. `at` is where the kart was
    /// last drawn (Bevy world), for the collision's recentring.
    pub fn step(
        &mut self,
        collide: &WorldCollision,
        now: f32,
        dt: f32,
        input: &Input,
        mods: &Modifiers,
        nearby: &[NearbyKart],
    ) -> Vec<Event> {
        self.step_with(collide, now, dt, input, mods, nearby, |_, _, _| {})
    }

    /// [`Session::step`], calling `after_tick` after each kart tick with the collision, the kart
    /// and the lu origin: CTR runs its weapon threads after the driver stages
    /// (`MainFrame.c:294-330`), so the weapons step in lockstep with the kart.
    #[allow(clippy::too_many_arguments)]
    pub fn step_with(
        &mut self,
        collide: &WorldCollision,
        now: f32,
        dt: f32,
        input: &Input,
        mods: &Modifiers,
        nearby: &[NearbyKart],
        mut after_tick: impl FnMut(&Collision, &mut Kart, Vec3),
    ) -> Vec<Event> {
        // A grid built around where we are now.
        if let Some(built) = &self.built {
            let built = built.lock().unwrap();
            while let Ok((epoch, grid)) = built.try_recv() {
                if epoch == self.epoch {
                    self.world = Some(grid);
                }
            }
        }
        let Some(current) = self.current else {
            return Vec::new();
        };
        let at = from_lu(current.position, self.origin);
        let rolled = at - self.centre;
        if Vec3::new(rolled.x, 0.0, rolled.z).length() > RECENTRE
            || rolled.y.abs() > GATHER_HALF.y * 0.5
            || now - self.gathered_at > REFRESH_SECS
        {
            let triangles = gather(collide, at, self.origin);
            self.centre = at;
            self.gathered_at = now;
            if let Some(send) = &self.build {
                let _ = send.send((self.epoch, triangles));
            }
        }
        let origin = self.origin;
        let (Some(world), Some(kart)) = (self.world.clone(), self.kart.as_mut()) else {
            return Vec::new();
        };
        let to_speed = lu_per_yard() * 256.0 / 30.0;
        let others: Vec<ctr_kart::OtherKart> = nearby
            .iter()
            .map(|n| ctr_kart::OtherKart {
                position: to_lu(n.position, origin),
                velocity: n.velocity * to_speed,
                weight: n.weight,
            })
            .collect();
        self.owed = (self.owed + dt.min(0.1)).min(0.25);
        let mut events = std::mem::take(&mut kart.events);
        while self.owed >= ctr_kart::TICK {
            self.owed -= ctr_kart::TICK;
            let before = kart.pose().position;
            kart.tick_among(&world, input, mods, &others);
            after_tick(&world, kart, origin);
            hold_at_void(&world, kart, before);
            if self.pinned {
                pin_in_place(kart, before);
            }
            if kart.grounded() {
                let pose = kart.pose();
                self.last_ground = Some((pose.position, pose.heading));
                self.falling = 0.0;
            } else {
                self.falling += ctr_kart::TICK;
                if self.falling > RESCUE_AFTER {
                    if let Some((at, heading)) = self.last_ground {
                        kart.place(at + Vec3::Y * 40.0, heading);
                    }
                    self.falling = 0.0;
                }
            }
            let shot = self.camera.tick(&kart.camera_input(), &world);
            // A cut (look-back) is drawn at once, never swept to through the kart.
            self.shots = Some(match self.shots {
                Some((_, last)) if !shot.cut => (last, shot),
                _ => (shot, shot),
            });
            events.append(&mut kart.events);
            self.previous = self.current.take();
            self.current = Some(kart.pose());
        }
        events
    }

    /// The kart as drawn this frame, `dt` after the last.
    pub fn view(&mut self, dt: f32) -> KartView {
        let (Some(current), Some(kart)) = (self.current, self.kart.as_ref()) else {
            return KartView::default();
        };
        let previous = self.previous.unwrap_or(current);
        let t = (self.owed / ctr_kart::TICK).clamp(0.0, 1.0);
        let (at_lu, rotation) = blend(&previous, &current, t);
        let origin = self.origin;
        // A hit's look: the tumble's squash and pivot, the squish's flat model lifted off the
        // ground (`Pose::scale`, `Pose::offset`).
        let offset = previous.offset.lerp(current.offset, t);
        let scale = previous.scale.lerp(current.scale, t);
        let at = from_lu(at_lu + offset, origin);
        let velocity = current.velocity / lu_per_yard();

        // CTR's chase camera, between its last two shots.
        let chase = self.shots.map(|(a, b)| {
            let eye = from_lu(a.eye.lerp(b.eye, t), origin);
            let look = from_lu(
                a.look.lerp(b.look, t) - Vec3::Y * self.camera.tuning.aim_drop,
                origin,
            );
            let roll = a.roll + (b.roll - a.roll) * t;
            let mut cam = Transform::from_translation(eye).looking_at(look, Vec3::Y);
            cam.rotate_local_z(-roll);
            cam
        });

        let forward_speed = kart.forward_speed() as f32 * 30.0 / 256.0;
        self.wheel_spin = (self.wheel_spin + forward_speed / WHEEL_RADIUS_LU * dt)
            .rem_euclid(std::f32::consts::TAU);
        KartView {
            active: true,
            transform: Transform {
                translation: at,
                rotation,
                scale: scale.max(Vec3::splat(0.01)) * crate::model_scale(),
            },
            yaw: yaw_of(current.heading),
            speed: Vec3::new(velocity.x, 0.0, velocity.z).length(),
            camera: chase,
            fov: self.camera.tuning.fov_radians(),
            steer: current.steer,
            wheel_spin: self.wheel_spin,
            grounded: current.grounded,
            state: current.state,
            reserves: kart.reserves() as f32 / 960.0,
            meter: kart.turbo_meter(),
            drift_yaw: kart.drawn_yaw_offset(),
            jump: kart.jump_meter(),
            ctr_speed: kart.needle(),
            class_speed: kart.stats().accel_speed,
            fire: kart.fire_size(),
            failed_boost: kart.failed_boost(),
            engine: kart.engine_sound(),
        }
    }
}

/// Another driver's kart near ours, for bumping: its origin (Bevy world, yards), velocity (yd/s)
/// and CollisionWeight (256 if unknown).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NearbyKart {
    pub position: Vec3,
    pub velocity: Vec3,
    pub weight: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat floor at height `y`, `half` lu across each way from the origin, in 100 lu tiles
    /// (the grid leaves out faces too large to drive on, as terrain's never are).
    fn floor(y: f32, half: f32) -> Vec<Tri> {
        let n = (half / 50.0) as i32;
        let mut tris = Vec::new();
        for i in -n..n {
            for j in -n..n {
                let (x0, z0) = (i as f32 * 50.0, j as f32 * 50.0);
                let (x1, z1) = (x0 + 50.0, z0 + 50.0);
                let (a, b, c, d) = ([x0, y, z0], [x1, y, z0], [x1, y, z1], [x0, y, z1]);
                tris.push([a, c, b]);
                tris.push([a, d, c]);
            }
        }
        tris
    }

    /// Drives straight on the gas for `secs`, holding at the void as the session does; returns
    /// how many ticks were held.
    fn drive(world: &Collision, kart: &mut Kart, secs: usize) -> usize {
        let gas = Input {
            gas: true,
            ..Default::default()
        };
        let mut held = 0;
        for _ in 0..30 * secs {
            let before = kart.pose().position;
            kart.tick(world, &gas, &Modifiers::default());
            held += usize::from(hold_at_void(world, kart, before));
        }
        held
    }

    #[test]
    fn a_kart_waits_at_the_edge_of_the_loaded_world_instead_of_falling() {
        let world = Collision::new(floor(0.0, 2000.0), CELL_LU);
        let mut kart = Kart::new(Class::Balanced, Vec3::new(0.0, 0.0, 0.0), 0);
        drive(&world, &mut kart, 1);
        assert!(kart.grounded(), "it drives on the floor first");
        let held = drive(&world, &mut kart, 20);
        let at = kart.pose().position;
        assert!(held > 0, "the kart reached the edge in 20 s");
        assert!(at.y > -50.0, "it did not fall: {at:?}");
        assert!(
            at.x.abs() < 2100.0 && at.z.abs() < 2100.0,
            "it stayed at the edge: {at:?}"
        );
    }

    #[test]
    fn a_kart_still_drives_off_a_ledge_with_ground_below() {
        let mut tris = floor(0.0, 4000.0);
        tris.extend(floor(400.0, 1000.0));
        let world = Collision::new(tris, CELL_LU);
        let mut kart = Kart::new(Class::Balanced, Vec3::new(0.0, 400.0, 0.0), 0);
        assert_eq!(
            drive(&world, &mut kart, 3),
            0,
            "ground below is not the void"
        );
        let at = kart.pose().position;
        assert!(at.y < 100.0, "it dropped to the floor below: {at:?}");
    }

    #[test]
    fn a_pinned_kart_holds_its_place_on_a_slope_and_rolls_when_let_go() {
        // A 20 degree slope rising towards +z, in 50 lu tiles.
        let rise = 20f32.to_radians().tan();
        let mut tris: Vec<Tri> = Vec::new();
        for i in -20..20 {
            for j in -20..20 {
                let (x0, z0) = (i as f32 * 50.0, j as f32 * 50.0);
                let (x1, z1) = (x0 + 50.0, z0 + 50.0);
                let y = |z: f32| z * rise;
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
        let world = Collision::new(tris, CELL_LU);
        // Facing uphill, a little above the ground, as a teleport leaves it.
        let mut kart = Kart::new(Class::Balanced, Vec3::new(0.0, 30.0, 0.0), 0);
        let brake = Input {
            brake: true,
            ..Default::default()
        };
        let start = kart.pose().position;
        for _ in 0..30 * 4 {
            let before = kart.pose().position;
            kart.tick(&world, &brake, &Modifiers::default());
            pin_in_place(&mut kart, before);
        }
        let held = kart.pose().position;
        assert!(
            (held.x - start.x).abs() < 0.01 && (held.z - start.z).abs() < 0.01,
            "pinned: {held:?}"
        );
        assert!(held.y < 15.0, "it settled onto the slope: {held:?}");
        assert!(kart.grounded(), "and stands on it");

        for _ in 0..30 * 3 {
            kart.tick(&world, &Input::default(), &Modifiers::default());
        }
        assert!(
            kart.pose().position.z < held.z - 20.0,
            "let go, it rolls back down: {:?}",
            kart.pose().position
        );
    }

    /// A height field `h(x, z)` in 50 lu tiles over `half` lu each way, as WoW's terrain is: one
    /// surface over each point, so under a steep stretch there is only that stretch.
    fn field(h: impl Fn(f32, f32) -> f32, half: f32) -> Vec<Tri> {
        let n = (half / 50.0) as i32;
        let mut tris = Vec::new();
        for i in -n..n {
            for j in -n..n {
                let (x0, z0) = (i as f32 * 50.0, j as f32 * 50.0);
                let (x1, z1) = (x0 + 50.0, z0 + 50.0);
                let p = |x: f32, z: f32| [x, h(x, z), z];
                let (a, b, c, d) = (p(x0, z0), p(x1, z0), p(x1, z1), p(x0, z1));
                tris.push([a, c, b]);
                tris.push([a, d, c]);
            }
        }
        tris
    }

    #[test]
    fn a_kart_drives_off_a_sheer_drop_instead_of_waiting_at_its_edge() {
        // A plateau 800 lu up for z < 0, a cliff face down to the valley floor at z = 300 (70
        // degrees): past the edge there is only the face below, too steep to be ground.
        let height = |_: f32, z: f32| (800.0 - z * 800.0 / 300.0).clamp(0.0, 800.0);
        let world = Collision::new(field(height, 3000.0), CELL_LU);
        let mut kart = Kart::new(Class::Balanced, Vec3::new(0.0, 800.0, -600.0), 0);
        let gas = Input {
            gas: true,
            ..Default::default()
        };
        // Until it is well into the valley, short of the loaded world's own far edge.
        let mut held_at = None;
        for _ in 0..30 * 6 {
            let before = kart.pose().position;
            kart.tick(&world, &gas, &Modifiers::default());
            if hold_at_void(&world, &mut kart, before) {
                held_at.get_or_insert(before);
            }
            if kart.pose().position.z > 1500.0 {
                break;
            }
        }
        let at = kart.pose().position;
        assert_eq!(held_at, None, "held at the cliff's edge");
        assert!(
            at.y < 100.0 && at.z > 1500.0,
            "down in the valley and on: {at:?}"
        );
    }

    #[test]
    fn a_kart_over_a_slope_too_steep_to_stand_on_comes_down_instead_of_hanging() {
        // A 60 degree slope (steeper than CTR's ground, 50) rising towards +z, and the kart in the
        // air a little above it, as a hop or a fall onto a steep hill leaves it.
        let rise = 60f32.to_radians().tan();
        let world = Collision::new(field(|_, z| z * rise, 2000.0), CELL_LU);
        let start = Vec3::new(0.0, 100.0, 0.0);
        let mut kart = Kart::new(Class::Balanced, start, 0);
        // Until it is well down the slope, short of the loaded world's own edge.
        let mut held_at = None;
        for _ in 0..30 * 3 {
            let before = kart.pose().position;
            kart.tick(&world, &Input::default(), &Modifiers::default());
            if hold_at_void(&world, &mut kart, before) {
                held_at.get_or_insert(before);
            }
            if kart.pose().position.z < -1200.0 {
                break;
            }
        }
        let at = kart.pose().position;
        assert_eq!(held_at, None, "held in the air over the slope");
        assert!(at.y < start.y - 500.0, "it slid down the slope: {at:?}");
    }

    #[test]
    fn a_kart_still_waits_where_nothing_has_streamed_in() {
        // Nothing of any kind under the far half: the world there has not loaded.
        let world = Collision::new(field(|_, _| 0.0, 1000.0), CELL_LU);
        let mut kart = Kart::new(Class::Balanced, Vec3::new(0.0, 0.0, 600.0), 0);
        let held = drive(&world, &mut kart, 10);
        let at = kart.pose().position;
        assert!(held > 0 && at.y > -50.0, "it waits at the edge: {at:?}");
    }
}
