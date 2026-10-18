//! One kart, ticked at CTR's 30 Hz in CTR's own units: positions in level units (lu), speeds in
//! 1/256 lu per frame, angles in 4096ths of a turn, timers in CTR-milliseconds (32 a frame). The
//! stages run in the game's order (`docs/ctr-physics-spec.md` section 2): UPDATE (drift entry and
//! boosts), PHYS_LINEAR (throttle and steer command), PHYS_ANGULAR (yaw), APPLY_FORCES (gravity,
//! friction), the collision sweep, the ground probe, JUMP_FRICTION (engine and hop), then the
//! visual frame and the hang-time meter. Section numbers below are the spec's.

use glam::{Mat3, Vec3};

use crate::collision::{Collision, GROUND_NORMAL_Y};
use crate::grab::{GrabMask, MaskGrab, Step};
use crate::rev::{RevEngine, RevState, StartBoost};
use crate::stats::*;

/// One frame's CTR-milliseconds at 30 fps.
pub const MS: i32 = 32;
/// The tick, seconds.
pub const TICK: f32 = 1.0 / 30.0;

/// One frame of controls.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Input {
    /// −1 full left to 1 full right.
    pub steer: f32,
    /// ✕: gas.
    pub gas: bool,
    /// □: brake (with gas, a power-brake; with `down`, reverse).
    pub brake: bool,
    /// Down on the stick: reverse.
    pub down: bool,
    /// The shoulders: hop, powerslide, boost.
    pub r1: bool,
    pub l1: bool,
}

/// What the abilities change this tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Modifiers {
    /// Wumpa fruit held, 0 to 10; 10 is juiced.
    pub wumpa: u8,
    /// Aku Aku's mask: its speed, and walls do not stop us.
    pub mask: bool,
    /// Super Engine: reserves kept topped up while the gas is held.
    pub super_engine: bool,
}

/// Something that happened this tick, for sounds, the score and the profession.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    Hop,
    /// Back on the ground after `air_ms` CTR-ms, landing at `fall` speed units downward.
    Land {
        air_ms: i32,
        fall: i32,
    },
    DriftStart {
        left: bool,
    },
    DriftEnd,
    /// A powerslide boost: which of the drift's (1–3) and the reserves it gave.
    Boost {
        stage: u8,
        reserves: i32,
    },
    /// A boost pressed before the red zone: the exhaust puff.
    BoostEarly,
    /// The meter ran full with no press: the chain is over.
    MeterFull,
    /// A hang-time boost, level 1–3.
    HangTime {
        level: u8,
    },
    /// A turbo item (the Turbo Boost ability).
    Turbo,
    WallRub,
    /// A head-on wall impact at `speed`.
    Slam {
        speed: i32,
    },
    SpinOut,
    /// A powerslide spun out (`VehPhysProc.c:2542-2552`): a SpinFirst with no hit behind it,
    /// which costs a Wumpa in a race (`VehPhysProc.c:2868-2871`); a hit's spin is paid with the
    /// hit. Comes with [`Event::SpinOut`].
    DriftSpinOut,
    /// Aku Aku put the fallen kart back (`VehStuckProc_MaskGrab_Init`, `VehStuckProc.c:622-720`),
    /// which costs 2 Wumpa in a race (`:648-651`).
    MaskGrab,
    /// The squish wore off and the model springs back up (`VehPhysForce.c:1740-1750`, SFX 0x5B).
    SquishRestored,
    /// Bumped by another kart, this hard (speed units into us).
    Bump {
        strength: i32,
    },
    /// A hit took: the reaction starts next frame (a TNT landing on the head starts its throw).
    Hit {
        kind: HitKind,
    },
    /// A hit the mask or the post-shield invincibility shrugged off: the victim laughs (voice 2).
    HitBlocked,
    /// The shield took a hit and popped (SFX 0x58).
    ShieldPopped,
    /// The green shield ran out (SFX 0x58).
    ShieldExpired,
    /// The mask has faded away.
    MaskEnded,
    /// The thrown TNT sat down on the head: the fuse runs (SFX 0x51).
    TntLanded,
    /// Hopped off: the TNT flies up from the head and explodes where it lands.
    TntThrownOff,
    /// The TNT on the head went off: its fuse ran out, or the kart spun, slammed or was blasted
    /// under it. Unless already blasted, the caller blasts this kart (a self-hit) and the 320 lu
    /// blast catches whoever else is near.
    TntExploded,
    /// Revving on the grid, the engine held at its target too long: locked out of the start
    /// boost (OtherFX 0xF, `VehStuckProc.c:1093-1099`).
    OverRev,
    /// Another kart (the caller's `OtherKart::id`) rammed us with its mask, shield or turbo, and
    /// the hit went through (`Kart::hit`); a mask, shield or invincibility of ours kept it off.
    Rammed {
        by: u32,
        ram: Ram,
    },
    /// We rammed another kart: its own side (the blast or squish) is its owner's to apply; ours,
    /// a spent shield or the high jump, is done.
    Rammer {
        other: u32,
        ram: Ram,
    },
}

/// What hit the kart: `VehPickState_NewState`'s damage types 1 to 3, and a TNT crate touched
/// (`RB_GenericMine.c:211-301`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HitKind {
    /// Type 1: a beaker, the clock, a hazard.
    SpinOut,
    /// Type 2: bomb, missile, Nitro, TNT, warp orb, shield bomb, a mask or shield ram.
    Blasted,
    /// Type 3: a turbo ram, a minecart.
    Squished,
    /// A TNT crate driven into: thrown onto the head, where it counts down.
    TntOnHead,
}

/// What a kart rams another with (`VehPhysCrash_Attack`, `VehPhysCrash.c:199-252`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ram {
    /// Under the mask (`ACTION_MASK_WEAPON`): the other kart is blasted.
    Mask,
    /// The shield (`instBubbleHold`) meets a kart without one: it is blasted, the shield spent.
    Shield,
    /// Under the turbo item (`ACTION_TURBO_ITEM`, reserves left) at an impact over 0xA00 into a
    /// kart with no reserves: it is squished, the rammer high-jumps.
    Turbo,
}

impl Ram {
    /// Damage type 2 for the mask and shield, 3 for the turbo.
    pub fn kind(self) -> HitKind {
        match self {
            Ram::Mask | Ram::Shield => HitKind::Blasted,
            Ram::Turbo => HitKind::Squished,
        }
    }
}

/// `VEH_PHYS_CRASH_TURBO_DAMAGE_IMPACT`: a turbo ram squishes above this impact (speed units).
pub const RAM_TURBO_IMPACT: f32 = 0xA00 as f32;

/// The red beaker's rain cloud over the kart (`RainCloudEffect`, `namespace_Vehicle.h:629-632`):
/// the four effects a cloud can carry, `(MixRNG % 400) / 100` on a second beaker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CloudEffect {
    /// Top speed less `DamagedSpeed` (`VehPhysGeneral.c:2093`).
    Slow,
    /// The held item spins in the roulette until the cloud goes (`RB_MaskShieldCloud.c:543-561`).
    ItemRoll,
    /// The ground under the kart is ice (`COLL.c:1476`).
    Ice,
    /// The shoulders do not hop (`VehPhysProc.c:921-929`).
    NoHop,
}

impl CloudEffect {
    /// CTR's numbering, 0 to 3.
    pub fn id(self) -> u8 {
        match self {
            CloudEffect::Slow => 0,
            CloudEffect::ItemRoll => 1,
            CloudEffect::Ice => 2,
            CloudEffect::NoHop => 3,
        }
    }

    pub fn from_id(id: u8) -> Option<Self> {
        Some(match id {
            0 => CloudEffect::Slow,
            1 => CloudEffect::ItemRoll,
            2 => CloudEffect::Ice,
            3 => CloudEffect::NoHop,
            _ => return None,
        })
    }
}

/// The power shield around the kart.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shield {
    /// Juiced: blue, with no time limit.
    pub blue: bool,
    /// Frames left on a green one.
    pub frames_left: Option<i32>,
    /// Its alpha, 1 solid; a green one fades over its last 60 frames.
    pub alpha: f32,
}

/// SpinFirst whirls, SpinLast recentres, SpinStop plays the steering back
/// (`VehPhysProc.c:2766-3150`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SpinPhase {
    First,
    Last,
    /// Animation frames left once the kart faces forward again.
    Stop(i32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Tnt {
    /// Above the kart's origin, lu, and rising at `vy` lu a frame while thrown on.
    height: i32,
    vy: i32,
    /// `numFramesOnHead` once it sits, else `None` while still in the air.
    fuse: Option<i32>,
    jumps_left: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Mask {
    ms: i32,
    fading: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ShieldState {
    blue: bool,
    ms: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    #[default]
    Normal,
    Drifting,
    /// The wall-slam lockout, CTR-ms left.
    Crashing(i32),
    /// The spin-out, CTR-ms left.
    Spinning(i32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Button {
    R1,
    L1,
}

/// How `VehFire_Increment` adds reserves (7.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FireKind {
    /// Pads and the turbo item: topped up to the amount, not stacked.
    Freeze,
    SuperEngine,
    /// Powerslide, hang time, start boost: stacked.
    Stack,
}

/// The terrain multipliers under the kart (8.2), all 256 = 1, and its `TERRAIN_FLAG_*`s
/// ([`crate::terrain::flags`]); CTR's whole table is [`crate::Ground`]'s.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Terrain {
    pub speed: i32,
    pub accel: i32,
    pub counter_steer: i32,
    pub lean: i32,
    pub ground_friction: i32,
    pub turn_angle: i32,
    pub turn_response: i32,
    pub flags: u32,
}

impl Terrain {
    pub const ASPHALT: Terrain = crate::Ground::Asphalt.terrain();
    pub const ICE: Terrain = crate::Ground::Ice.terrain();
    /// In the air.
    pub const NONE: Terrain = crate::Ground::None.terrain();
    /// Deviates (the owner's call, World of Kartcraft): afloat on deep water ([`Kart::water`]).
    /// CTR has no floating (its shallow water is a ground row, deep water a kill plane): a
    /// hovercraft's feel, CTR's Water row's speed lifted a little, grip let go to a slide, the
    /// steering softer, Water's flags (its rolling sound).
    pub const AFLOAT: Terrain = Terrain {
        speed: 200,
        accel: 200,
        counter_steer: 256,
        lean: 256,
        ground_friction: 96,
        turn_angle: 256,
        turn_response: 200,
        flags: crate::Ground::Water.terrain().flags,
    };
    /// Afloat with a boost burning: CTR's FastWater, the kart planing at full grip.
    pub const PLANING: Terrain = crate::Ground::FastWater.terrain();
}

/// How deep a floating kart sits in the water, lu: its floor is the surface less this.
pub const DRAFT: f32 = 12.0;

// `actionsFlagSet` bits that matter here.
const TOUCH_GROUND: u32 = 0x1;
const STARTED_TOUCH_GROUND: u32 = 0x2;
const JUMP_BUTTON_HELD: u32 = 0x4;
const ACCEL_PREVENTION: u32 = 0x8;
const STEER_LEFT: u32 = 0x10;
const BRAKE_WITH_ACCEL: u32 = 0x20;
const BACK_SKID: u32 = 0x800;
const FRONT_SKID: u32 = 0x1000;
const DRIVING_AGAINST_WALL: u32 = 0x2000;

/// Mashing ✕ (`namespace_Vehicle.h:102-103`): each press and let-go of the gas within this many
/// CTR-ms of the last counts; at this many the kart steers hard and pivots, below this speed
/// (`VehPhysProc.c:55-56`), at this steer strength.
const ACCEL_TAP_WINDOW_MS: i32 = 0x100;
const ACCEL_TAP_STEER_COUNT: i32 = 7;
const ACCEL_TAP_SPEED_MAX: i32 = 0x2600;
const ACCEL_TAP_STRENGTH: i32 = 0x5a;
const REVERSING: u32 = 0x20000;
const AIRBORNE: u32 = 0x80000;
/// `ACTION_TURBO_INPUT_LATCH`: no boost wheelie until the next boost (`VehFire.c:158` clears it).
const TURBO_LATCH: u32 = 0x400;
/// What survives from one frame to the next; the rest is rebuilt each frame.
const CARRY: u32 = TOUCH_GROUND | STEER_LEFT | REVERSING | AIRBORNE | TURBO_LATCH;

/// `VehCalc_MapToRange`: clamps test the old bounds literally, so a reversed range saturates.
pub(crate) fn map(v: i32, omin: i32, omax: i32, nmin: i32, nmax: i32) -> i32 {
    if v <= omin {
        nmin
    } else if v >= omax {
        nmax
    } else {
        nmin + ((i64::from(v - omin) * i64::from(nmax - nmin)) / i64::from(omax - omin)) as i32
    }
}

/// `VehCalc_InterpBySpeed`: `v` toward `target` by `step`, no overshoot.
fn approach(v: f32, step: f32, target: f32) -> f32 {
    if v < target {
        (v + step).min(target)
    } else {
        (v - step).max(target)
    }
}

fn approach_i(v: i32, step: i32, target: i32) -> i32 {
    if v < target {
        (v + step).min(target)
    } else {
        (v - step).max(target)
    }
}

fn rad(angle: i32) -> f32 {
    angle as f32 * std::f32::consts::TAU / 4096.0
}

fn wrap(angle: i32) -> i32 {
    angle & 0xFFF
}

/// The signed shortest difference `a − b`, in [−2048, 2048).
fn diff(a: i32, b: i32) -> i32 {
    ((a - b + 2048) & 0xFFF) - 2048
}

/// An angle wrapped into [−2048, 2048), as the spin and tumble keep `turnAngleCurr`.
fn wrap_signed(angle: i32) -> i32 {
    ((angle + 2048) & 0xFFF) - 2048
}

/// The clock's top-speed cut for a kart in `rank` (0 = first): `DamagedSpeed * (20 − rank) >> 4`
/// (`VehPhysGeneral.c:2098-2105`), 5120 for the leader down to 3328 for eighth.
pub fn clock_slowdown(rank: u8) -> i32 {
    (DAMAGED_SPEED * (20 - i32::from(rank.min(19)))) >> 4
}

/// Which of the boost wheelie's baked tables plays ([`Kart::wheelie`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wheelie {
    Start,
    Hold,
    Recover,
}

/// A wheelie frame by its [`Kart::wheelie_code`]: the turn in the kart's own frame (Z ahead,
/// Y up), the scale, and the lift and forward shift (lu, in the kart's frame). `None` for 0.
pub fn wheelie_frame(code: u8) -> Option<(Mat3, Vec3, Vec3)> {
    let i = usize::from(code);
    let [lift, forward, pitch, x, y, z] = match i {
        1..=11 => WHEELIE_START[i - 1],
        12 => WHEELIE_HOLD[0],
        13..=21 => WHEELIE_RECOVER[i - 13],
        _ => return None,
    };
    Some((
        Mat3::from_rotation_x(rad(pitch)),
        Vec3::new(x as f32, y as f32, z as f32) / 4096.0,
        Vec3::new(0.0, lift as f32, forward as f32) / 256.0,
    ))
}

/// `VehPhysForce_TranslateMatrix_RemapIndex` (`VehPhysForce.c:1205-1232`): a cut-short rise's
/// frame as the matching frame of the way down, or back: the share of the way through the one
/// table, reversed, into the other's frames.
fn wheelie_remap(index: usize, from: usize, to: usize) -> usize {
    let q = ((index as i32) << 8) / (from as i32 - 1).max(1);
    let blend = (256 - q).clamp(0, 256);
    ((blend * (to as i32 - 1)) >> 8) as usize
}

/// Tumble frame `i` of `bakedGteMath[BLASTED]` as `Vector_BakeMatrixTable` builds it
/// (`Vector.c:155-176`): two rolls about the nose over the 27 frames (`rot.z = i·0x2000/27`), a
/// pitch wobble of `−sin(i·0x3000/27)/7`, and a squash and stretch of the width and length. The
/// rotation is `ConvertRotToMatrix`'s X then Z; the scale is returned apart.
fn tumble_frame(i: i32) -> (Mat3, Vec3) {
    let n = BLAST_TUMBLE_FRAMES;
    let roll = (i << 13) / n;
    let wobble = (i * 0x3000) / n;
    let sin = |a: i32| (rad(a).sin() * 4096.0) as i32;
    let pitch = -sin(wobble) / 7;
    let s = sin(roll);
    let scale = Vec3::new(
        (4096 + s / 4) as f32 / 4096.0,
        1.0,
        (4096 + s * 6 / 40) as f32 / 4096.0,
    );
    (
        Mat3::from_rotation_x(rad(pitch)) * Mat3::from_rotation_z(rad(roll)),
        scale,
    )
}

/// The speed over the ground (lu/s) below which the kart has no travel direction for the
/// camera to follow: a crawl, a stop, a hop in place.
const TRAVEL_MIN: f32 = 120.0;

fn atan2_units(y: f32, x: f32) -> i32 {
    wrap((y.atan2(x) * 4096.0 / std::f32::consts::TAU).round() as i32)
}

/// `VehPhysForce_RotAxisAngle`: the frame with Y on `n` and Z the heading `angle` carried onto
/// the plane by the minimal rotation from up to `n`; X = Y × Z (the kart's left).
pub fn rot_axis_angle(n: Vec3, angle: i32) -> Mat3 {
    let a = rad(angle);
    let f = Vec3::new(a.sin(), 0.0, a.cos());
    let k = Vec3::Y.cross(n);
    let s = k.length();
    let z = if s < 1e-6 {
        f
    } else {
        let k = k / s;
        let c = n.y;
        f * c + k.cross(f) * s + k * k.dot(f) * (1.0 - c)
    };
    let x = n.cross(z);
    Mat3::from_cols(x, n, z)
}

/// `VehCalc_SteerAccel`: 0 → max over the first frames, held, then back to 0.
fn steer_accel(frames: i32) -> i32 {
    let s2 = STEER_ACCEL_STAGE2_FIRST;
    let s3 = s2 + STEER_ACCEL_STAGE2_LEN;
    let s4 = STEER_ACCEL_STAGE4_FIRST;
    if frames < s2 {
        map(frames, 0, s2, 0, STEER_ACCEL_MAX)
    } else if frames < s3 {
        STEER_ACCEL_MAX
    } else if frames < s4 {
        map(frames, s3, s4, STEER_ACCEL_MAX, 0)
    } else {
        0
    }
}

/// `VehPhysJoystick_GetStrength` over a stick's distance from centre (0 to 127), retail's
/// defaults: dead zone 48, full at 127, a two-segment curve between.
fn stick_strength(distance: i32, max: i32) -> i32 {
    const DEAD: i32 = 0x30;
    const RANGE: i32 = 0x7f;
    const DIST: i32 = 0x5e;
    if distance < DEAD {
        return 0;
    }
    if distance >= RANGE {
        return max;
    }
    let v = distance - DEAD;
    let half = DIST / 2;
    let fifth = max / 5;
    if v < half {
        v * fifth * 2 / DIST
    } else {
        (v - half) * (max - fifth) * 2 / DIST + fifth
    }
}

/// Another kart this one can bump into.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OtherKart {
    /// Its origin, lu, in this kart's space.
    pub position: Vec3,
    /// Its velocity, speed units (1/256 lu per frame).
    pub velocity: Vec3,
    /// Its class's CollisionWeight.
    pub weight: i32,
    /// The caller's name for it, carried in the ram events.
    pub id: u32,
    /// What it wears for a ram: its mask, its shield, the turbo item burning.
    pub masked: bool,
    pub shielded: bool,
    pub turbo: bool,
}

/// What the chase camera reads from a kart (`docs/ctr-camera-spec.md` section 6).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraInput {
    /// The kart's origin, lu.
    pub position: Vec3,
    /// The physics heading and the drawn turn off it, 4096ths.
    pub heading: i32,
    pub turn_angle: i32,
    /// `rotCurr.w`, the powerslide's camera swing, 4096ths.
    pub spin: i32,
    /// The ground's lean under the kart, + to its right, 4096ths.
    pub roll: i32,
    pub speed_approx: i32,
    pub spinning: bool,
    /// A turbo was granted since the camera last looked.
    pub turbo: bool,
    pub fire_speed_cap: i32,
    /// The direction the kart is travelling over the ground, 4096ths; `None` when it is barely
    /// moving, so has none worth following.
    pub travel: Option<i32>,
    /// Tumbling from a blast (`KS_BLASTED`), on the ground, and `quadBlockHeight`: the ground
    /// under the kart, or 256 lu below it with none in reach (`COLL.c:1362-1373`), lu.
    pub blasted: bool,
    pub grounded: bool,
    pub ground: f32,
    /// The kart was put somewhere else since the camera last looked (`DIRECTION_CHANGED`): the
    /// shot cuts.
    pub cut: bool,
    /// Lowered by Aku Aku: the height the eye keeps over, lu, in place of the kart's
    /// (`CAMERA_FLAG_MASK_GRAB`, `CAM.c:1221-1224`).
    pub mask_grab: Option<f32>,
}

/// The world view of a kart, for drawing and the camera.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    /// The kart's origin (the bottom centre), lu.
    pub position: Vec3,
    /// The drawn frame: columns X (left), Y (up), Z (forward).
    pub basis: Mat3,
    /// World velocity, lu per second.
    pub velocity: Vec3,
    /// The physics heading, 4096ths of a turn.
    pub heading: i32,
    /// The drawn front-wheel steer, −1 (right) to 1 (left).
    pub steer: f32,
    pub grounded: bool,
    pub state: State,
    /// The model's scale along its own X, Y, Z (1 = as built): stretched falling and squashed
    /// landing, flat (Y 0) squished and bouncing back after, and squashing and stretching as a
    /// blasted one tumbles. Applied before `basis`.
    pub scale: Vec3,
    /// Where the model is drawn off `position`, lu: up its normal when squished, up the kart as
    /// it bounces back, round the tumble's pivot when blasted.
    pub offset: Vec3,
}

pub struct Kart {
    stats: Stats,
    pub state: State,
    pub terrain: Terrain,
    /// The ground under the kart, the host's to set before a tick (CTR reads the touched
    /// quadblock's `terrain_type`, `COLL.c:1475-1491`); asphalt until it says otherwise.
    pub ground_type: Terrain,
    /// Deviates (the owner's call): the surface of deep water under the kart (lu, the world's
    /// height), the host's to set before a tick, or none. The kart's floor is then the higher of
    /// the ground and the surface less [`DRAFT`]: the shallows drive their bed, deep water floats
    /// the kart, and the shore joins them with no step. None is CTR's behaviour, unchanged.
    pub water: Option<f32>,
    /// On the water's floor this tick ([`Kart::water`]).
    afloat: bool,
    /// `terrainMeta2`, the terrain the speed multiplier comes from: it stays through a hop and
    /// is let go 128 lu above the ground (`COLL.c:1431-1435`).
    speed_terrain: Terrain,
    /// `terrainScaledBaseSpeed`: the target speed after the terrain's multiplier.
    terrain_base: i32,
    /// On a turbo pad this tick ([`Kart::turbo_pad`]), and the tick before.
    on_pad: bool,
    pad_prev: bool,

    pos: Vec3,
    vel: Vec3,
    speed: i32,
    speed_approx: i32,
    vel_yaw: i32,
    vel_pitch: i32,

    angle: i32,
    turn_angle: i32,
    turn_angle_prev: i32,
    turn_angle_vel: i32,
    simp_turn: i32,
    spin_rate: i32,
    amp_turn: i32,
    frames_steering: i32,

    base_speed: i32,
    fire_speed: i32,
    fwd_accel_impulse: i32,
    forward_dir: i32,

    reserves: i32,
    fire_speed_cap: i32,
    turbo_outside: i32,
    turbo_meter: i32,

    mult_drift: i32,
    prev_mult_drift: i32,
    drift_assist_ms: i32,
    drift_frames: i32,
    drift_boost_ms: i32,
    drift_total_ms: i32,
    boosts_attempted: i32,
    boosts_success: i32,
    drift_button: Button,
    drift_entered_this_frame: bool,

    jump_buffer: i32,
    jump_cooldown: i32,
    coyote: i32,
    jump_forced: i32,
    airtime: i32,
    jump_meter: i32,
    fall: i32,

    wall_rub: i32,
    grip_timer: i32,
    /// `accelTapWindowTimer` and `accelTapCount`: the gas mashed ([`ACCEL_TAP_WINDOW_MS`]).
    accel_tap_window: i32,
    accel_tap_count: i32,

    /// The ground normal touched this frame, the smoothed one, and last frame's.
    n1: Vec3,
    n2: Vec3,
    n4: Vec3,
    moving: Mat3,
    facing: Mat3,
    nudge: Vec3,

    flags: u32,
    flags_prev: u32,
    buttons_prev: (bool, bool),
    /// `jump_InitialVelY`: the vertical speed PROCESS_JUMP gives.
    jump_vel: i32,
    /// A hop started this frame (`ACTION_JUMP_STARTED` from the player's own hop).
    hopped: bool,

    /// The hit reaction to install at the start of the next frame (`funcPtrs[INIT]`).
    pending: Option<HitKind>,
    /// `NoInputTimer`, CTR-ms: the spin's and the tumble's lockout.
    no_input: i32,
    spin_phase: SpinPhase,
    /// `KartStates.Spinning.driftSpinRate` and `spinDir`.
    spin_turn: i32,
    /// Blasted (`KS_BLASTED`), and whether the tumble plays backwards.
    blasted: bool,
    blast_backwards: bool,
    squish: i32,
    invincible: i32,
    clock: i32,
    clock_slowdown: i32,
    mask: Option<Mask>,
    shield: Option<ShieldState>,
    tnt: Option<Tnt>,
    /// The rain cloud's effect while one hangs over us (`driver->rainCloudEffect`).
    cloud: Option<CloudEffect>,
    /// `ACTION_TURBO_ITEM`: the reserves burning came from the turbo item.
    turbo_item: bool,
    /// `forcedJumpType = FORCED_JUMP_HIGH`, from a turbo ram, for this frame's jump.
    high_jump: bool,
    /// `MixRNG_Scramble`'s 16-bit state, for the TNT's hop count and the tumble's direction.
    rng: u16,
    /// `EngineSound_Player`'s smoothed pitch and volume states and the drift's rising whine.
    engine_pitch: i32,
    engine_volume: i32,
    engine_whine: i32,
    engine_sound: (u8, u8),
    failed_boost: i32,
    /// `rotCurr.w`: the camera's swing out of a powerslide's turn, and its rate (`rotPrev.w`).
    camera_spin: i32,
    camera_spin_rate: i32,
    /// A turbo was granted since the camera last looked (`VehFire.c`'s camera flag 0x80).
    camera_kick: bool,
    /// Held on the grid before GO, or lowered by Aku Aku, revving (`KS_ENGINE_REVVING`).
    rev: Option<RevEngine>,
    /// Aku Aku carrying the kart back (`KS_MASK_GRABBED`, then the rev state), and the cut to
    /// its respawn for the camera (`CAMERA_FLAG_DIRECTION_CHANGED`).
    grab: Option<MaskGrab>,
    camera_cut: bool,
    /// `quadBlockHeight`, lu: the camera's floor through a blast.
    ground: f32,
    /// `jumpHeightCurr`: the velocity along the kart's up, speed units; and whether a jump
    /// started this frame (`ACTION_JUMP_STARTED`, set by every PROCESS_JUMP).
    jump_height: i32,
    jump_started: bool,
    /// `jumpSquishStretch` and its lagging twin `jumpSquishStretch2`, and the instance's X, Y, Z
    /// scale ([`SCALE_BASE`] as built) that follows them.
    squash: i32,
    squash_lag: i32,
    model_scale: [i32; 3],
    /// `matrixIndex` into [`SQUISH_RECOVER`] while the squish's recovery bounce plays.
    squish_recover: Option<usize>,
    /// The boost wheelie (`matrixArray` WHEELIE_START, HOLD or RECOVER, and `matrixIndex`).
    wheelie: Option<(Wheelie, usize)>,
    /// `gGT->timer`: frames ticked, for the invincibility flicker.
    frame: u32,

    pub events: Vec<Event>,
}

impl Kart {
    /// A kart of `class` standing at `position` (lu) facing `heading` (4096ths, CTR's convention:
    /// forward is `(sin h, 0, cos h)`).
    pub fn new(class: Class, position: Vec3, heading: i32) -> Self {
        let heading = wrap(heading);
        Self {
            stats: class.stats(),
            state: State::Normal,
            terrain: Terrain::ASPHALT,
            ground_type: Terrain::ASPHALT,
            water: None,
            afloat: false,
            speed_terrain: Terrain::ASPHALT,
            terrain_base: 0,
            on_pad: false,
            pad_prev: false,
            pos: position,
            vel: Vec3::ZERO,
            speed: 0,
            speed_approx: 0,
            vel_yaw: heading,
            vel_pitch: 0,
            angle: heading,
            turn_angle: 0,
            turn_angle_prev: 0,
            turn_angle_vel: 0,
            simp_turn: 0,
            spin_rate: 0,
            amp_turn: 0,
            frames_steering: 10000,
            base_speed: 0,
            fire_speed: 0,
            fwd_accel_impulse: 0,
            forward_dir: 1,
            reserves: 0,
            fire_speed_cap: 0,
            turbo_outside: 0,
            turbo_meter: 0,
            mult_drift: 0,
            prev_mult_drift: 0,
            drift_assist_ms: 0,
            drift_frames: 0,
            drift_boost_ms: 0,
            drift_total_ms: 0,
            boosts_attempted: 0,
            boosts_success: 0,
            drift_button: Button::R1,
            drift_entered_this_frame: false,
            jump_buffer: 0,
            jump_cooldown: 0,
            coyote: 0,
            jump_forced: 0,
            airtime: 0,
            jump_meter: 0,
            fall: 0,
            wall_rub: 0,
            grip_timer: 0,
            accel_tap_window: 0,
            accel_tap_count: 0,
            n1: Vec3::Y,
            n2: Vec3::Y,
            n4: Vec3::Y,
            moving: rot_axis_angle(Vec3::Y, heading),
            facing: rot_axis_angle(Vec3::Y, heading),
            nudge: Vec3::ZERO,
            flags: 0,
            flags_prev: 0,
            buttons_prev: (false, false),
            jump_vel: JUMP_FORCE,
            hopped: false,
            pending: None,
            no_input: 0,
            spin_phase: SpinPhase::First,
            spin_turn: 0,
            blasted: false,
            blast_backwards: false,
            squish: 0,
            invincible: 0,
            clock: 0,
            clock_slowdown: 0,
            mask: None,
            shield: None,
            tnt: None,
            cloud: None,
            turbo_item: false,
            high_jump: false,
            rng: 0,
            engine_pitch: 0,
            engine_volume: 0,
            engine_whine: 0,
            engine_sound: (0x3c, 0x82),
            failed_boost: 0,
            camera_spin: 0,
            camera_spin_rate: 0,
            camera_kick: false,
            rev: None,
            grab: None,
            camera_cut: false,
            ground: position.y,
            jump_height: 0,
            jump_started: false,
            squash: 0,
            squash_lag: 0,
            model_scale: [SCALE_BASE; 3],
            squish_recover: None,
            wheelie: None,
            frame: 0,
            events: Vec::new(),
        }
    }

    pub fn stats(&self) -> Stats {
        self.stats
    }

    pub fn set_class(&mut self, class: Class) {
        self.stats = class.stats();
    }

    /// Speed, 1/256 lu per frame.
    pub fn speed(&self) -> i32 {
        self.speed
    }

    /// Signed speed along the kart, 1/256 lu per frame.
    pub fn forward_speed(&self) -> i32 {
        self.speed_approx
    }

    /// Turbo time left, CTR-ms.
    pub fn reserves(&self) -> i32 {
        self.reserves
    }

    /// The powerslide meter: 0 empty to 1 full, and whether it is in the red zone; `None` when
    /// not drifting or between attempts.
    pub fn turbo_meter(&self) -> Option<(f32, bool)> {
        // Revving on the grid, it shows the revs, red from the acceleration speed up
        // (`VehStuckProc.c:1236-1257`).
        if let Some(rev) = &self.rev {
            let room = rev.meter_room(TURBO_MAX_ROOM_MS, TURBO_LOW_ROOM_MS);
            let filled = 1.0 - room as f32 / TURBO_MAX_ROOM_MS as f32;
            return Some((filled, rev.state() == RevState::Ready));
        }
        (self.state == State::Drifting && self.turbo_meter > 0).then(|| {
            let filled = 1.0 - self.turbo_meter as f32 / TURBO_MAX_ROOM_MS as f32;
            (filled, self.turbo_meter < TURBO_LOW_ROOM_MS)
        })
    }

    /// The hang-time meter (`jumpMeter`, CTR-ms of airtime, up to 2400) while a hop's airtime
    /// counts for it, else `None`.
    pub fn jump_meter(&self) -> Option<i32> {
        (self.flags & AIRBORNE != 0 && !self.grounded()).then_some(self.jump_meter)
    }

    /// How far the drawn kart is turned off its heading (the powerslide look, a spin-out's whirl),
    /// radians, + to the left.
    pub fn drawn_yaw_offset(&self) -> f32 {
        rad(self.turn_angle)
    }

    /// The turbo flame's size while reserves burn, else 0: `clamp(5 + (fireLevel >> 6), 4, 8)`
    /// (`VehFire.c`), the fire level read back from the speed cap it granted (`2048 + 8·level`),
    /// so a slide's boosts burn at 5, 6 and 7, and sacred fire, pads and the turbo item at 8.
    pub fn fire_size(&self) -> u8 {
        if self.reserves == 0 {
            return 0;
        }
        let level = ((self.fire_speed_cap - SINGLE_TURBO_SPEED) / 8).max(0);
        (5 + (level >> 6)).clamp(4, 8) as u8
    }

    /// The exhaust puffs on through a lit flame: CTR's 8 frames after a boost pressed too early
    /// (`failedBoostExhaustTimer`).
    pub fn failed_boost(&self) -> bool {
        self.failed_boost > 0
    }

    /// What the chase camera reads this frame (`CAM_FollowDriver_Normal`); takes the turbo kick.
    pub fn camera_input(&mut self) -> CameraInput {
        // The roll the camera banks by: the drawn normal's lean toward the kart's right
        // (`COLL.c:1580-1585`); X is the kart's left.
        let right = -self.facing.x_axis;
        let roll = atan2_units(self.n2.dot(right), self.n2.y);
        CameraInput {
            position: self.pos,
            heading: self.angle,
            turn_angle: self.turn_angle,
            spin: self.camera_spin,
            roll: if roll >= 2048 { roll - 4096 } else { roll },
            speed_approx: self.speed_approx,
            spinning: matches!(self.state, State::Spinning(_)),
            turbo: std::mem::take(&mut self.camera_kick),
            fire_speed_cap: self.fire_speed_cap,
            travel: (self.vel.x.hypot(self.vel.z) * (30.0 / 256.0) > TRAVEL_MIN)
                .then(|| atan2_units(self.vel.x, self.vel.z)),
            blasted: self.blasted,
            grounded: self.grounded(),
            ground: self.ground,
            cut: std::mem::take(&mut self.camera_cut),
            mask_grab: self.grab.as_ref().and_then(MaskGrab::camera_height),
        }
    }

    /// `rotCurr.w` (`VehPhysProc.c:1953-2007`, `VehPhysGeneral.c:86-167`): through a powerslide
    /// the camera swings ±384 out of the turn, easing in at up to 16 a frame; out of one it
    /// eases back, and in a spin or a slam it snaps back at 32 a frame.
    fn camera_spin_tick(&mut self) {
        match self.state {
            _ if self.blasted => {
                self.camera_spin = approach_i(self.camera_spin, 32, 0);
                self.camera_spin_rate = 0;
            }
            State::Spinning(_) | State::Crashing(_) => {
                self.camera_spin = approach_i(self.camera_spin, 32, 0);
                self.camera_spin_rate = 0;
            }
            _ => {
                let target = match self.state {
                    State::Drifting if self.mult_drift >= 0 => 384,
                    State::Drifting => -384,
                    _ => 0,
                };
                let cap = ((self.camera_spin - target).abs() >> 3).clamp(1, 16);
                self.camera_spin_rate = approach_i(self.camera_spin_rate, 8, cap);
                self.camera_spin = approach_i(self.camera_spin, self.camera_spin_rate, target);
            }
        }
    }

    /// The engine's sound this frame, as CTR mixes it: the distortion (0 to 255; the sample
    /// plays at `2^((d − 128) / 64)` times its own rate) and the volume (0 to 255).
    pub fn engine_sound(&self) -> (u8, u8) {
        self.engine_sound
    }

    /// `VehPhysCrash_AnyTwoCars` for a player against each kart in reach: overlapping hit
    /// spheres push apart by the overlap, and both velocities bounce off the weighted mean
    /// (`VehPhysCrash_BounceSelf`, which takes 4/3 of the closing speed: a third bounces back).
    /// Only this kart's velocity is ours to change; the other's owner bumps it on their side.
    /// Then `VehPhysCrash_Attack` both ways (`VehPhysCrash.c:575-576`): see [`Kart::ram`].
    fn collide_karts(&mut self, others: &[OtherKart]) {
        // `VehTurbo_ThDestroy` clears `ACTION_TURBO_ITEM` with the flame, when reserves run out.
        if self.reserves == 0 {
            self.turbo_item = false;
        }
        let weight = self.stats.collision_weight as f32;
        for other in others {
            let d = self.pos - other.position;
            let distance = d.length();
            let strength = 2.0 * KART_HIT_RADIUS - distance;
            if strength <= 0.0 {
                continue;
            }
            let dir = if distance > 1e-3 {
                d / distance
            } else {
                Vec3::Z
            };
            let w = other.weight as f32;
            let mean = (self.vel * weight + other.velocity * w) / (weight + w);
            let closing = (self.vel - mean).dot(dir);
            // `vehicleCollisionImpactStrength`: the larger closing speed `VehPhysCrash_BounceSelf`
            // sees, the other's toward us or ours toward it.
            let impact = (other.velocity - mean).dot(dir).max(-closing).max(0.0);
            if closing < 0.0 {
                let rel = self.vel - mean - dir * (closing * 4.0 / 3.0);
                self.vel = rel + mean;
                self.vel.y = self.vel.y.min(BOUNCE_Y_CLAMP);
            }
            self.vel += dir * (strength * 4096.0 / 256.0);
            self.events.push(Event::Bump {
                strength: (-closing).max(0.0) as i32,
            });
            self.ram(other, impact);
        }
    }

    /// `VehPhysCrash_Attack(us, other)` then `(other, us)` (`VehPhysCrash.c:199-252`). Against
    /// us: unless masked, its mask blasts us, its shield (with none of ours) blasts us, and its
    /// turbo item over [`RAM_TURBO_IMPACT`] into a kart with no reserves squishes us; the last
    /// one set wins. From us: our shield is spent on a kart without one, and our turbo ram makes
    /// us high-jump. The other kart's own side is its owner's: ours only reports it. Its reserves
    /// are not relayed: a kart not under the turbo item counts as having none.
    fn ram(&mut self, other: &OtherKart, impact: f32) {
        let ours_masked = self.masked();
        let ours_shielded = self.shield.is_some();
        let ours_turbo = self.turbo_item && self.reserves != 0;
        if !ours_masked {
            let mut against = None;
            if other.masked {
                against = Some(Ram::Mask);
            }
            if other.shielded && !ours_shielded {
                against = Some(Ram::Shield);
            }
            if impact > RAM_TURBO_IMPACT && other.turbo && self.reserves == 0 {
                against = Some(Ram::Turbo);
            }
            if let Some(ram) = against {
                if self.hit(ram.kind()) {
                    self.events.push(Event::Rammed { by: other.id, ram });
                }
            }
        }
        if !other.masked {
            let mut from = None;
            if ours_masked {
                from = Some(Ram::Mask);
            }
            if ours_shielded && !other.shielded {
                // `SHIELD_FLAG_CRASH_ATTACK`: the shield pops (`RB_MaskShieldCloud.c:393-408`),
                // unless the other's ram already took it.
                if self.shield.take().is_some() {
                    self.events.push(Event::ShieldPopped);
                }
                from = Some(Ram::Shield);
            }
            if impact > RAM_TURBO_IMPACT && ours_turbo && !other.turbo {
                self.high_jump = true;
                from = Some(Ram::Turbo);
            }
            if let Some(ram) = from {
                self.events.push(Event::Rammer {
                    other: other.id,
                    ram,
                });
            }
        }
    }

    /// Another kart's shield spent ramming (`SHIELD_FLAG_CRASH_ATTACK`): it pops, with no
    /// invincibility after. False with no shield up.
    pub fn spend_shield(&mut self) -> bool {
        if self.shield.take().is_none() {
            return false;
        }
        self.events.push(Event::ShieldPopped);
        true
    }

    /// The rain cloud over us, or none: the caller times it (`RB_RainCloud_ThTick`).
    pub fn set_cloud(&mut self, cloud: Option<CloudEffect>) {
        self.cloud = cloud;
    }

    pub fn cloud(&self) -> Option<CloudEffect> {
        self.cloud
    }

    /// `EngineSound_Player` (`HOWL_Engine.c`), the player's steady-state branch: the pitch chases
    /// the mean of the throttle's target and the speed (the target plus 0xF00 in the air and in a
    /// powerslide, so the engine revs), the volume swells while the revs change and settles when
    /// they hold, and through a powerslide the pitch climbs with the turbo meter.
    fn engine_sound_tick(&mut self) {
        let mut target = self.fire_speed.abs();
        let drifting = self.state == State::Drifting;
        if let Some(rev) = &self.rev {
            // Revving on the grid (`HOWL_Engine.c:170-181`): the pitch chases the revs, higher
            // with the gas down.
            let gas = if rev.gas() { 0x3000 } else { 0 };
            target = (self.engine_pitch * 0x40 + gas * 0x30 + rev.level() * 0x90) >> 8;
            if rev.gas() {
                target += 0x1000;
            }
        } else if self.flags_prev & TOUCH_GROUND == 0 || drifting {
            target += 0xF00;
        } else {
            target = (target + self.speed_approx.abs()) >> 1;
        }
        if (target - self.engine_pitch).abs() < 0x601 {
            self.engine_volume -= 500;
            self.engine_volume = self.engine_volume.max(if drifting { 2000 } else { 0 });
        } else {
            self.engine_volume = (self.engine_volume + 2000).min(14000);
        }
        self.engine_pitch = (target * 0x89 + self.engine_pitch * 0x177) >> 9;
        let accel_speed = self.stats.accel_speed;
        let volume = map(self.engine_volume, 0, accel_speed, 0x82, 0xE6);
        let pitch = map(
            self.engine_pitch,
            0,
            accel_speed + SACRED_FIRE_SPEED + 0xF00,
            0x3C,
            200,
        );
        if drifting {
            if self.turbo_meter == 0 {
                self.engine_whine = (self.engine_whine - 1).max(0);
            } else {
                self.engine_whine = ((TURBO_MAX_ROOM_MS >> 5) >> 1) - (self.turbo_meter >> 6);
            }
        }
        let distortion = (pitch + self.engine_whine).clamp(0, 0xFF);
        self.engine_sound = (distortion as u8, volume.clamp(0, 0xFF) as u8);
    }

    /// Floating on deep water this tick ([`Kart::water`]).
    pub fn afloat(&self) -> bool {
        self.afloat
    }

    pub fn grounded(&self) -> bool {
        self.flags & TOUCH_GROUND != 0
    }

    /// Scraping a wall this tick: `wallRubTimer` still full (`VEH_EMITTER_WALL_RUB_TIMER_FULL`,
    /// `VehEmitter.c:1355`), the test CTR's wall sparks and scrape loop run on.
    pub fn against_wall(&self) -> bool {
        self.wall_rub == WALL_RUB_MS
    }

    /// Moves the kart without changing its motion (a respawn or a teleport).
    /// Stops the kart at `position`, keeping its heading and whatever state it is in: the world
    /// past it has not loaded yet (Kartcraft's own; a CTR track is always whole).
    pub fn halt_at(&mut self, position: Vec3) {
        self.pos = position;
        self.vel = Vec3::ZERO;
        self.speed = 0;
        self.speed_approx = 0;
    }

    /// Shifts the kart by `by` (lu) and leaves its motion as it is: a showcase run's playback
    /// easing the kart back onto its recorded line (Kartcraft's own; CTR has no such thing).
    pub fn nudge(&mut self, by: Vec3) {
        self.pos += by;
    }

    pub fn place(&mut self, position: Vec3, heading: i32) {
        let heading = wrap(heading);
        self.pos = position;
        self.angle = heading;
        self.vel_yaw = heading;
        self.vel = Vec3::ZERO;
        self.speed = 0;
        self.speed_approx = 0;
        self.reserves = 0;
        // `turbo_outsideTimer` goes with them (`VehStuckProc.c:639-640`, `:884-885`).
        self.turbo_outside = 0;
        self.state = State::Normal;
        self.turbo_meter = 0;
        self.turn_angle = 0;
        self.pending = None;
        self.no_input = 0;
        self.blasted = false;
        self.squish = 0;
        self.squash = 0;
        self.squash_lag = 0;
        self.model_scale = [SCALE_BASE; 3];
        self.squish_recover = None;
        self.moving = rot_axis_angle(Vec3::Y, heading);
        self.facing = self.moving;
    }

    /// Aku Aku carries the fallen kart back onto the ground at `ground`, facing `heading` (CTR's
    /// mask grab, `VehStuckProc_MaskGrab_Init`, `VehStuckProc.c:622-720`; [`crate::grab`]): it
    /// stops where it is, no input drives it until he lets go: [`Event::MaskGrab`], 2 Wumpa in
    /// a race.
    pub fn mask_grab(&mut self, ground: Vec3, heading: i32) {
        let at = self.pos;
        self.place(at, self.angle);
        self.rev = None;
        // `actionsFlagSet &= ~(ACTION_AIRBORNE | ACTION_HIGH_JUMP)`; held, it touches nothing.
        self.flags &= !(AIRBORNE | TOUCH_GROUND);
        self.grab = Some(MaskGrab::new(at, ground, heading));
        self.events.push(Event::MaskGrab);
    }

    /// Aku Aku is carrying the kart back ([`Kart::mask_grab`]).
    pub fn grabbed(&self) -> bool {
        self.grab.is_some()
    }

    /// The mask holding the kart, as drawn; `None` when there is none or it has not shown yet.
    pub fn grab_mask(&self) -> Option<GrabMask> {
        self.grab.as_ref().and_then(|g| g.mask(self.pos))
    }

    /// A frame of the mask grab, in place of driving (`DRIVER_FUNC_*` of `KS_MASK_GRABBED` and
    /// the rev state): true while Aku Aku holds the kart, false once he lets go this frame, which
    /// then drives as any other (`VehStuckProc_RevEngine_Update` → `VehPhysProc_Driving_Init`).
    fn grab_tick(&mut self, gas: bool) -> bool {
        let Some(grab) = self.grab.as_mut() else {
            return false;
        };
        let ground = grab.ground();
        match grab.tick(&mut self.pos, MS) {
            Step::Hold => {}
            Step::Respawned(heading) => {
                // `VehBirth_TeleportSelf`, then `VehStuckProc_RevEngine_Init`.
                let at = self.pos;
                self.place(at, heading);
                self.ground = ground.y;
                self.rev = Some(RevEngine::new(self.stats.accel_speed));
                self.camera_cut = true;
            }
            Step::Released => {
                self.grab = None;
                self.release_revs();
                return false;
            }
        }
        // Nobody hits a grabbed kart (`KS_MASK_GRABBED`).
        self.pending = None;
        if !self.grab.is_some_and(|g| g.lowering()) {
            // Grabbed, a shield pops (`RB_MaskShieldCloud.c:393-408`) and a TNT on the head
            // goes off (`RB_TNT.c:124-131`).
            if self.shield.take().is_some() {
                self.events.push(Event::ShieldPopped);
            }
            if self.tnt.is_some_and(|t| t.fuse.is_some()) {
                self.tnt = None;
                self.events.push(Event::TntExploded);
            }
        }
        if let Some(rev) = self.rev.as_mut() {
            if rev.tick(gas, MS) {
                self.events.push(Event::OverRev);
            }
        }
        self.engine_sound_tick();
        self.mask_tick();
        self.shield_tick();
        self.tnt_tick();
        true
    }

    /// `RB_Player_ToggleFlicker` (`231/RB_Player.c:231-257`): with more than 0x2A0 ms of
    /// invincibility left, the kart is not drawn on odd frames. Only battle's tumble end gives
    /// that much (2880, `VehPhysProc.c:1705-1710`); a shield pop's 0x2A0 never flickers, so in a
    /// race the kart never does.
    pub fn flicker_hidden(&self) -> bool {
        self.invincible > FLICKER_INVINCIBLE_MS && self.frame & 1 != 0
    }

    pub fn pose(&self) -> Pose {
        let steer = (self.simp_turn as f32 / self.stats.turn_rate.max(1) as f32).clamp(-1.0, 1.0);
        let mut basis = self.facing;
        // The instance's squash and stretch (`VehPhysForce.c:1643-1772`), then the baked matrix
        // on top of it: the tumble or the squish's recovery bounce.
        let [x, y, z] = self.model_scale.map(|s| s as f32 / SCALE_BASE as f32);
        let mut scale = Vec3::new(x, y, z);
        let mut offset = Vec3::ZERO;
        if self.blasted {
            // `VehStuckProc_Tumble_Animate`: the frame is the lockout's 32 ms count, wrapped.
            let n = BLAST_TUMBLE_FRAMES;
            let mut i = (self.no_input >> 5) % n;
            if self.blast_backwards {
                i = n - (i + 1);
            }
            let (tumble, s) = tumble_frame(i);
            basis = self.facing * tumble;
            scale *= s;
            offset = self.facing * ((Vec3::Y - tumble * Vec3::Y) * BLAST_PIVOT);
        } else if let Some(i) = self.squish_recover {
            // `VehPhysForce.c:1366-1440`: the baked entry's lift is turned with the kart.
            let [lift, x, y, z] = SQUISH_RECOVER[i];
            scale *= Vec3::new(x as f32, y as f32, z as f32) / 4096.0;
            offset = self.facing * (Vec3::Y * (lift as f32 / 256.0));
        } else if let Some((turn, s, lift)) = wheelie_frame(self.wheelie_code()) {
            // The same baked entry's turn, lift and scale, from the wheelie's tables.
            basis = self.facing * turn;
            scale *= s;
            offset = self.facing * lift;
        }
        if self.squish != 0 {
            // `VehPhysForce.c:1737-1740`, `1453-1458`: scale.y = 0, lifted up the drawn normal.
            scale.y = 0.0;
            offset += self.n2 * SQUISH_LIFT;
        } else if let Some(rev) = &self.rev {
            // Revving on the grid (`VehStuckProc.c:1259-1293`) writes the instance's scale itself.
            let (x, y, z) = rev.scale();
            scale = Vec3::new(x, y, z);
        }
        Pose {
            position: self.pos,
            basis,
            velocity: self.vel * (30.0 / 256.0),
            heading: self.angle,
            steer,
            grounded: self.grounded(),
            state: self.state,
            scale,
            offset,
        }
    }

    /// Seeds the kart's `MixRNG_Scramble` stream (the TNT's hop count, the tumble's direction).
    pub fn seed(&mut self, seed: u16) {
        self.rng = seed;
    }

    /// `MixRNG_Scramble` (`MixRNG.c:3-10`).
    fn scramble(&mut self) -> i32 {
        self.rng = self.rng.wrapping_mul(0x6255).wrapping_add(0x3619);
        i32::from(self.rng)
    }

    /// The reaction showing, and its frames of lockout left (0 while a spin recentres): a blast
    /// over a squish over a spin over a TNT's fuse. A hit taken this frame shows already.
    pub fn reaction(&self) -> Option<(HitKind, i32)> {
        let frames = |ms: i32| (ms + MS - 1) / MS;
        if let Some(kind) = self.pending {
            return Some((kind, frames(self.no_input)));
        }
        if self.blasted {
            return Some((HitKind::Blasted, frames(self.no_input)));
        }
        if self.squish != 0 {
            return Some((HitKind::Squished, frames(self.squish)));
        }
        if let State::Spinning(_) = self.state {
            let left = if self.spin_phase == SpinPhase::First {
                self.no_input
            } else {
                0
            };
            return Some((HitKind::SpinOut, frames(left)));
        }
        self.tnt_fuse().map(|f| (HitKind::TntOnHead, f))
    }

    /// Under a mask (`ACTION_MASK_WEAPON`): while it lasts and while it fades.
    pub fn masked(&self) -> bool {
        self.mask.is_some()
    }

    pub fn shield(&self) -> Option<Shield> {
        self.shield.map(|s| Shield {
            blue: s.blue,
            frames_left: (!s.blue).then_some(s.ms / MS),
            alpha: if s.blue || s.ms >= SHIELD_FADE_MS {
                1.0
            } else {
                // `RB_MaskShieldCloud.c:385-389`: alphaScale 0x400 (solid) to 0x1000 (gone).
                let a = ((60 - (s.ms >> 5)) * 3072) / 60 + 0x400;
                1.0 - (a - 0x400) as f32 / 3072.0
            },
        })
    }

    /// Frames until the TNT on the head goes off; the whole fuse while it is still being thrown
    /// on.
    pub fn tnt_fuse(&self) -> Option<i32> {
        self.tnt.map(|t| TNT_FUSE_FRAMES - t.fuse.unwrap_or(0))
    }

    /// The TNT's height over the kart's origin, lu, for drawing it.
    pub fn tnt_height(&self) -> Option<f32> {
        self.tnt.map(|t| t.height as f32)
    }

    /// Under the clock: frames left.
    pub fn clocked(&self) -> Option<i32> {
        (self.clock != 0).then_some(self.clock / MS)
    }

    /// The model's height scale, 0 while squished, else 1.
    pub fn squish_scale(&self) -> f32 {
        if self.squish != 0 {
            0.0
        } else {
            1.0
        }
    }

    /// Whether ○ may fire a held item now (`VehPhysProc.c:804-861`): driving or drifting, no TNT
    /// on the head, not under the clock. The shield fires past the clock: [`Kart::fire_shield`].
    pub fn can_fire(&self) -> bool {
        self.clock == 0 && self.can_fire_shield()
    }

    fn can_fire_shield(&self) -> bool {
        matches!(self.state, State::Normal | State::Drifting) && !self.blasted && self.tnt.is_none()
    }

    /// A hit (`RB_Hazard_HurtDriver` → `VehPickState_NewState`, `VehPickState.c:48-381`). A mask
    /// or the invincibility after a shield pop shrugs it off; a shield pops instead and leaves 21
    /// invincible frames; a blast on a blasted kart does nothing; a spin on a spinning kart only
    /// restarts its 30 frames. A hit that takes cancels the boost, and its reaction installs at
    /// the start of the next frame. True when it took (for a TNT: it is on its way to the head).
    pub fn hit(&mut self, kind: HitKind) -> bool {
        // `KS_MASK_GRABBED` is set "so nobody hits you" (`VehStuckProc.c:628`, `:878-879`).
        if self.grab.is_some() {
            return false;
        }
        if kind != HitKind::TntOnHead {
            return self.damage(Some(kind));
        }
        // `RB_GenericMine.c:211-247`: a second TNT, or one on a squished kart, blasts it.
        if self.tnt.take().is_some() || self.squish != 0 {
            return self.damage(Some(HitKind::Blasted));
        }
        // `:261-301`: damage type 0 through the same gate, then the crate jumps on (vy 48).
        if !self.damage(None) {
            return false;
        }
        self.tnt = Some(Tnt {
            height: 0,
            vy: TNT_THROW_VY,
            fuse: None,
            jumps_left: 0,
        });
        self.events.push(Event::Hit {
            kind: HitKind::TntOnHead,
        });
        // `ThTick_SetAndExec`: the throw's first frame runs at once.
        self.tnt_tick();
        true
    }

    fn damage(&mut self, kind: Option<HitKind>) -> bool {
        if self.masked() || self.invincible != 0 {
            self.events.push(Event::HitBlocked);
            return false;
        }
        if self.shield.take().is_some() {
            self.invincible = SHIELD_POP_INVINCIBLE_MS;
            self.events.push(Event::ShieldPopped);
            return false;
        }
        let Some(kind) = kind else {
            return true;
        };
        match kind {
            HitKind::Blasted => {
                if self.blasted || self.pending == Some(HitKind::Blasted) {
                    return false;
                }
                self.no_input = BLAST_MS;
                self.squish = 0;
                self.pending = Some(kind);
            }
            HitKind::Squished => {
                self.squish = SQUISH_MS;
                self.no_input = SQUISH_NO_INPUT_MS;
                self.pending = Some(kind);
            }
            _ => {
                self.no_input = SPIN_HIT_MS;
                // Retail also drops `kartState` to normal here, so a spin re-hit mid-spin
                // loses the spinning state's friction and fire lock; we keep them.
                if !matches!(self.state, State::Spinning(_)) {
                    self.pending = Some(kind);
                }
            }
        }
        self.reserves = 0;
        self.turbo_outside = 0;
        self.events.push(Event::Hit { kind });
        true
    }

    /// The clock from another kart (`VehPickupItem.c:855-908`): a spin-out, and if it takes,
    /// `clockReceive` (8 s, juiced 12 s) taking `slowdown` off the top speed, which retail sets by
    /// race position: [`clock_slowdown`]. False when shrugged off.
    pub fn clock(&mut self, juiced: bool, slowdown: i32) -> bool {
        if !self.hit(HitKind::SpinOut) {
            return false;
        }
        self.clock = if juiced { CLOCK_JUICED_MS } else { CLOCK_MS };
        self.clock_slowdown = slowdown;
        true
    }

    /// Aku Aku / Uka Uka (`VehPickupItem_MaskUseWeapon`): 8 s, juiced 12 s, plus its fade; a
    /// second mask restarts the time. Invincible, +3900 top speed, walls are rails.
    pub fn mask(&mut self, juiced: bool) {
        self.mask = Some(Mask {
            ms: if juiced { MASK_JUICED_MS } else { MASK_MS },
            fading: false,
        });
    }

    /// The power shield (`VehPickupItem.c:910-988`): green for 12 s, blue (juiced) until used.
    pub fn shield_up(&mut self, juiced: bool) {
        self.shield = Some(ShieldState {
            blue: juiced,
            ms: SHIELD_MS,
        });
    }

    /// ○ with the shield up (`VehPhysProc.c:822-829`): it leaves the kart as a projectile, which
    /// the caller launches. False when there is no shield or the kart may not fire.
    pub fn fire_shield(&mut self) -> bool {
        if self.shield.is_none() || !self.can_fire_shield() {
            return false;
        }
        self.shield = None;
        true
    }

    /// CTR's start-line boost (`VehStuckProc.c:968-983`): a second of turbo as the race starts,
    /// a big flame for an engine pumped to its top, a small one for one merely revved.
    pub fn start_boost(&mut self, big: bool) {
        let fire = if big {
            crate::rev::BIG_BOOST_FIRE
        } else {
            crate::rev::SMALL_BOOST_FIRE
        };
        if self.fire_increment(crate::rev::START_BOOST_RESERVES, FireKind::Stack, fire) {
            self.events.push(Event::Turbo);
        }
    }

    /// Holds the kart on the grid, revving, until GO (`VehStuckProc_RevEngine_Init`, waiting
    /// on the traffic lights: `VehStuckProc.c:954-958`): the gas revs the engine instead of
    /// driving. Let go (GO), the revs earn their start boost, which is returned.
    pub fn hold_on_grid(&mut self, on: bool) -> Option<StartBoost> {
        // Aku Aku's revs are his to end ([`Kart::mask_grab`]).
        if self.grab.is_some() {
            return None;
        }
        match (on, self.rev) {
            (true, None) => {
                self.rev = Some(RevEngine::new(self.stats.accel_speed));
                None
            }
            (false, Some(_)) => self.release_revs(),
            _ => None,
        }
    }

    /// The rev state ends (`VehStuckProc_RevEngine_Update`, `VehStuckProc.c:962-990`): the revs
    /// earn their boost and the meter is full, as driving starts.
    fn release_revs(&mut self) -> Option<StartBoost> {
        let rev = self.rev.take()?;
        self.turbo_meter = 0;
        let boost = rev.release();
        if let Some(b) = boost {
            self.start_boost(b == StartBoost::Big);
        }
        boost
    }

    /// Revving on the grid: the revs, `None` while not held there.
    pub fn revs(&self) -> Option<&RevEngine> {
        self.rev.as_ref()
    }

    /// The speedometer's needle (`speedometerNeedleValue`): the speed, or the revs while revving
    /// on the grid (`VehStuckProc.c:1234`).
    pub fn needle(&self) -> i32 {
        self.rev.map_or(self.speed, |r| r.level())
    }

    /// The Turbo Boost ability: CTR's turbo item, stronger juiced.
    pub fn fire_turbo(&mut self, juiced: bool) {
        let fire = if juiced { 256 } else { 128 };
        if self.fire_increment(TURBO_ITEM_RESERVES, FireKind::Freeze, fire) {
            // `VehFire.c:335-339`: `ACTION_TURBO_ITEM`, for a turbo ram.
            self.turbo_item = true;
            self.events.push(Event::Turbo);
        }
    }

    /// The kart is on a turbo pad for the coming tick (`COLL_STEP_TRIGGER_TURBO_PAD`,
    /// `VehPhysForce.c:1003-1028`): 960 reserves topped up at fire level 256 (the Freeze path,
    /// so standing on it holds them), refused while braking (`VehFire.c:120-129`), and the
    /// engine's 8000 while the gas is down (`VehPhysGeneral.c:1189-1239`). A [`Event::Turbo`]
    /// marks only the tick it is driven onto.
    pub fn turbo_pad(&mut self) {
        if self.flags & ACCEL_PREVENTION != 0 {
            return;
        }
        self.on_pad = true;
        if self.fire_increment(TURBO_PAD_RESERVES, FireKind::Freeze, 256) && !self.pad_prev {
            self.events.push(Event::Turbo);
        }
    }

    /// The skid flags this tick (`ACTION_BACK_SKID`, `ACTION_FRONT_SKID`): the back tyres under
    /// the brake, spinning off the line or pushed against their direction (`VehPhysForce.c:
    /// 448-534`), all four through a powerslide (`VehPhysProc.c:2580`), off ice or on a ground
    /// that forces them (`VehPhysForce.c:630-633`, `VehEmitter.c:1413-1419`); on the ground only,
    /// as `VehEmitter` draws and plays them (`:1449-1456`).
    pub fn skids(&self) -> (bool, bool) {
        if !self.grounded() {
            return (false, false);
        }
        let all = self.state == State::Drifting
            || self.terrain.flags & crate::terrain::flags::FORCE_SKIDMARKS != 0;
        (
            all || self.flags & BACK_SKID != 0,
            all || self.flags & FRONT_SKID != 0,
        )
    }

    /// `VehFire_Increment` (7.1); false when the state refuses it: spinning, blasted or carried
    /// by Aku Aku, and no other (`VehFire.c:142-155`).
    fn fire_increment(&mut self, reserves: i32, kind: FireKind, fire: i32) -> bool {
        if self.blasted || self.grab.is_some() || matches!(self.state, State::Spinning(_)) {
            return false;
        }
        self.flags &= !TURBO_LATCH;
        let cap = SINGLE_TURBO_SPEED + ((fire * (SACRED_FIRE_SPEED - SINGLE_TURBO_SPEED)) >> 8);
        if self.reserves == 0 || cap > self.fire_speed_cap || self.fire_speed_cap > 4096 {
            self.fire_speed_cap = cap;
        }
        match kind {
            FireKind::Freeze => {
                if self.turbo_outside < reserves {
                    self.reserves += reserves - self.turbo_outside;
                    self.turbo_outside = reserves;
                }
            }
            FireKind::SuperEngine => self.reserves = self.reserves.max(reserves),
            FireKind::Stack => self.reserves += reserves,
        }
        self.reserves = self.reserves.min(i32::from(i16::MAX));
        self.camera_kick = true;
        true
    }

    /// One 30 Hz frame.
    pub fn tick(&mut self, world: &Collision, input: &Input, mods: &Modifiers) {
        self.tick_among(world, input, mods, &[]);
    }

    /// One 30 Hz frame among other karts, which bump this one (`VehPhysCrash_AnyTwoCars`).
    pub fn tick_among(
        &mut self,
        world: &Collision,
        input: &Input,
        mods: &Modifiers,
        others: &[OtherKart],
    ) {
        self.hopped = false;
        self.jump_started = false;
        self.frame = self.frame.wrapping_add(1);
        // Carried by Aku Aku, nothing the player does drives the kart (the gas revs it once
        // lowered); the frame he lets go drives.
        if self.grab_tick(input.gas) {
            self.buttons_prev = (input.r1, input.l1);
            return;
        }
        // Held on the grid, the gas revs the engine and the kart stands, braked
        // (`VehStuckProc_RevEngine_*`: no forces apply).
        let held = Input {
            brake: true,
            ..Default::default()
        };
        let rev_gas = self.rev.map(|_| input.gas);
        let input = if rev_gas.is_some() { &held } else { input };
        let tapped_r1 = input.r1 && !self.buttons_prev.0;
        let tapped_l1 = input.l1 && !self.buttons_prev.1;
        self.buttons_prev = (input.r1, input.l1);
        // `ACTION_MASK_WEAPON`: our own mask counts as the caller's.
        let mods = &Modifiers {
            mask: mods.mask || self.masked(),
            ..*mods
        };

        // INIT: a hit taken since the last frame installs its reaction.
        match self.pending.take() {
            Some(HitKind::Blasted) => self.tumble_init(),
            Some(_) => self.spin_init(),
            None => {}
        }
        self.update_stage(input, tapped_r1 || tapped_l1);
        self.phys_linear(input, mods, tapped_r1, tapped_l1);
        match self.state {
            _ if self.blasted => self.tumble_angular(),
            State::Drifting => self.powerslide_angular(),
            State::Spinning(_) => self.spin_angular(),
            _ => self.phys_angular(),
        }
        self.apply_forces();
        self.n4 = self.n1;
        self.n1 = Vec3::Y;
        self.sweep(world, mods);
        self.vel -= self.nudge;
        self.collide_karts(others);
        self.ground_probe(world);
        self.jump_and_friction(mods);
        // TRANSLATE_MATRIX: the visual frame.
        let w = if self.grounded() { 6.0 } else { 7.0 };
        let target = if self.grounded() { self.n1 } else { Vec3::Y };
        self.n2 = ((self.n2 * w + target * (8.0 - w)) / 8.0)
            .try_normalize()
            .unwrap_or(Vec3::Y);
        let yaw = wrap(self.angle + self.turn_angle);
        self.facing = rot_axis_angle(self.n2, yaw);
        self.squash_stretch();
        self.hang_time();
        if let (Some(rev), Some(gas)) = (self.rev.as_mut(), rev_gas) {
            if rev.tick(gas, MS) {
                self.events.push(Event::OverRev);
            }
        }
        self.engine_sound_tick();
        self.camera_spin_tick();
        // ANIMATE: SpinStop hands back to driving once its animation is home.
        if let (State::Spinning(_), SpinPhase::Stop(left)) = (self.state, &mut self.spin_phase) {
            if self.turn_angle == 0 {
                *left -= 1;
            }
        }
        // The weapon threads, after the driver stages (`MainFrame.c:294-330`).
        self.mask_tick();
        self.shield_tick();
        self.tnt_tick();
        self.wheelie_tick();
        self.pad_prev = std::mem::take(&mut self.on_pad);
    }

    /// The boost wheelie's frame (`VehPhysForce_TranslateMatrix_UpdateMatrixAnimation`,
    /// `VehPhysForce.c:1234-1340`): while reserves burn at the class's top speed or over and no
    /// latch holds it, the nose rises over WHEELIE_START and stays at WHEELIE_HOLD; then it
    /// comes down over WHEELIE_RECOVER. A rise cut short comes down from as high as it got.
    fn wheelie_tick(&mut self) {
        let on = self.reserves != 0
            && self.fire_speed >= self.stats.speed
            && self.flags & TURBO_LATCH == 0
            && self.squish_recover.is_none()
            && !self.blasted;
        self.wheelie = match (on, self.wheelie) {
            (true, None) => Some((Wheelie::Start, 0)),
            (true, Some((Wheelie::Start, i))) if i + 1 >= WHEELIE_START.len() => {
                Some((Wheelie::Hold, 0))
            }
            (true, Some((Wheelie::Start, i))) => Some((Wheelie::Start, i + 1)),
            (true, Some((Wheelie::Hold, i))) => Some((Wheelie::Hold, i)),
            (true, Some((Wheelie::Recover, i))) => Some((
                Wheelie::Start,
                wheelie_remap(i, WHEELIE_RECOVER.len(), WHEELIE_START.len()),
            )),
            (false, None) => None,
            (false, Some((Wheelie::Hold, _))) => Some((Wheelie::Recover, 0)),
            (false, Some((Wheelie::Start, i))) => Some((
                Wheelie::Recover,
                wheelie_remap(i, WHEELIE_START.len(), WHEELIE_RECOVER.len()),
            )),
            (false, Some((Wheelie::Recover, i))) if i + 1 >= WHEELIE_RECOVER.len() => None,
            (false, Some((Wheelie::Recover, i))) => Some((Wheelie::Recover, i + 1)),
        };
    }

    /// The boost wheelie's frame, if one plays.
    pub fn wheelie(&self) -> Option<(Wheelie, usize)> {
        self.wheelie
    }

    /// The wheelie's frame as one number for the wire ([`wheelie_frame`]): 0 none, 1-11 the
    /// rise, 12 the hold, 13-21 the way down.
    pub fn wheelie_code(&self) -> u8 {
        match self.wheelie {
            None => 0,
            Some((Wheelie::Start, i)) => 1 + i.min(WHEELIE_START.len() - 1) as u8,
            Some((Wheelie::Hold, _)) => 12,
            Some((Wheelie::Recover, i)) => 13 + i.min(WHEELIE_RECOVER.len() - 1) as u8,
        }
    }

    /// `RB_MaskWeapon_ThTick` and `_FadeAway`: the time runs out, then 16 frames of fade.
    fn mask_tick(&mut self) {
        let Some(m) = &mut self.mask else {
            return;
        };
        if !m.fading {
            if m.ms != 0 {
                m.ms = (m.ms - MS).max(0);
                return;
            }
            m.fading = true;
        }
        if m.ms < MASK_FADE_MS {
            m.ms = (m.ms + MS).min(MASK_FADE_MS);
        } else {
            self.mask = None;
            self.events.push(Event::MaskEnded);
        }
    }

    /// `RB_ShieldDark_ThTick_Grow`'s timer: a green shield goes when its time is up.
    fn shield_tick(&mut self) {
        let Some(s) = &mut self.shield else {
            return;
        };
        if s.blue {
            return;
        }
        if s.ms == 0 {
            self.shield = None;
            self.events.push(Event::ShieldExpired);
        } else {
            s.ms -= MS;
        }
    }

    /// `RB_TNT_ThTick_ThrowOnHead` and `_SitOnHead` (`RB_TNT.c:102-340`).
    fn tnt_tick(&mut self) {
        let Some(mut t) = self.tnt else {
            return;
        };
        if t.fuse.is_none() {
            t.height += t.vy;
            if t.vy < 0 && t.height < TNT_HEAD_HEIGHT {
                t.height = TNT_HEAD_HEIGHT;
                t.fuse = Some(0);
                t.jumps_left = TNT_JUMPS - self.scramble() % 2;
                self.events.push(Event::TntLanded);
            } else {
                t.vy = (t.vy - TNT_GRAVITY).max(TNT_TERMINAL_VY);
                self.tnt = Some(t);
                return;
            }
        }
        // Spun, slammed or blasted under it: it goes off at once.
        if self.blasted || matches!(self.state, State::Spinning(_) | State::Crashing(_)) {
            self.tnt = None;
            self.events.push(Event::TntExploded);
            return;
        }
        if self.hopped {
            if t.jumps_left == 0 {
                self.tnt = None;
                self.events.push(Event::TntThrownOff);
                return;
            }
            t.jumps_left -= 1;
        }
        let fuse = t.fuse.unwrap_or(0);
        if fuse < TNT_FUSE_FRAMES {
            t.fuse = Some(fuse + 1);
            self.tnt = Some(t);
        } else {
            self.tnt = None;
            self.events.push(Event::TntExploded);
        }
    }

    /// `VehPhysProc_SpinFirst_Init` (`VehPhysProc.c:2859-2909`): whirl the way the kart was
    /// turning, 300 a frame.
    fn spin_init(&mut self) {
        if self.state == State::Drifting {
            self.events.push(Event::DriftEnd);
        }
        self.blasted = false;
        self.state = State::Spinning(self.no_input);
        self.spin_phase = SpinPhase::First;
        self.turn_angle_vel = 0;
        self.turbo_meter = 0;
        self.spin_turn = if self.amp_turn < 0 {
            -SPIN_RATE
        } else {
            SPIN_RATE
        };
        self.events.push(Event::SpinOut);
    }

    /// `VehStuckProc_Tumble_Init` (`VehStuckProc.c:1454-1504`): the tumble plays backwards on
    /// `MixRNG & 4`.
    fn tumble_init(&mut self) {
        if self.state == State::Drifting {
            self.events.push(Event::DriftEnd);
        }
        self.state = State::Normal;
        self.blasted = true;
        self.turbo_meter = 0;
        // The tumble's baked matrix takes over from a recovery bounce.
        self.squish_recover = None;
        self.blast_backwards = self.scramble() & 4 != 0;
    }

    /// `VehStuckProc_Tumble_PhysAngular`: the spin and the model's turn die away by an eighth a
    /// frame.
    fn tumble_angular(&mut self) {
        self.frames_steering = 10000;
        self.spin_rate -= self.spin_rate >> 3;
        self.turn_angle_vel -= self.turn_angle_vel >> 3;
        self.amp_turn = self.spin_rate;
        self.turn_angle = wrap_signed(self.turn_angle + self.turn_angle_vel);
        self.angle = wrap(self.angle + ((self.amp_turn * MS) >> 13));
        self.moving = rot_axis_angle(self.n1, self.angle);
        self.nudge = Vec3::ZERO;
    }

    /// UPDATE (2.3): drift entry, the powerslide's meter and boosts, the lockouts' ends.
    fn update_stage(&mut self, input: &Input, tapped: bool) {
        self.drift_entered_this_frame = false;
        if self.blasted {
            // `VehStuckProc_Tumble_Update`: straight back to driving when the lockout is out.
            if self.no_input == 0 {
                self.blasted = false;
                self.turbo_meter = 0;
            }
            return;
        }
        match self.state {
            State::Normal => {
                let held = match self.drift_button {
                    Button::R1 => input.r1,
                    Button::L1 => input.l1,
                };
                if self.flags & STARTED_TOUCH_GROUND != 0
                    && self.simp_turn.abs() > self.stats.turn_rate / 2
                    && held
                    && self.flags & ACCEL_PREVENTION == 0
                    && self.speed_approx >= self.stats.speed / 2
                {
                    self.powerslide_init();
                }
            }
            State::Drifting => self.powerslide_update(input, tapped),
            State::Crashing(left) if left <= 0 => {
                self.state = State::Normal;
                self.turbo_meter = 0;
            }
            State::Spinning(_) => match self.spin_phase {
                // `VehPhysProc_SpinFirst_Update`: whirl while locked out and still moving.
                SpinPhase::First => {
                    if self.no_input == 0 || self.speed_approx.abs() <= SPIN_SLOW_SPEED {
                        self.spin_phase = SpinPhase::Last;
                    }
                }
                SpinPhase::Last => {
                    if self.turn_angle.abs() < SPIN_STOP_ANGLE {
                        self.spin_phase = SpinPhase::Stop(SPIN_STOP_ANIM_FRAMES);
                    }
                }
                SpinPhase::Stop(left) => {
                    if left <= 0 {
                        self.state = State::Normal;
                        self.turbo_meter = 0;
                    }
                }
            },
            _ => {}
        }
    }

    /// `VehPhysProc_PowerSlide_Init` (5.1).
    fn powerslide_init(&mut self) {
        self.state = State::Drifting;
        self.drift_entered_this_frame = true;
        let base = self.stats.drift_turn_base << 8;
        self.mult_drift = if self.simp_turn < 0 { -base } else { base };
        self.drift_frames = 0;
        self.drift_total_ms = 0;
        self.drift_boost_ms = 0;
        self.boosts_attempted = 0;
        self.boosts_success = 0;
        self.spin_rate = 0;
        self.turn_angle_vel = 0;
        self.drift_assist_ms = 0;
        self.turbo_meter = TURBO_MAX_ROOM_MS;
        self.events.push(Event::DriftStart {
            left: self.mult_drift > 0,
        });
    }

    /// `VehPhysProc_PowerSlide_Update` (5.3, 5.4).
    fn powerslide_update(&mut self, input: &Input, tapped: bool) {
        if tapped {
            self.drift_frames = 0;
            let m = self.turbo_meter;
            if m != 0 {
                if m < TURBO_LOW_ROOM_MS {
                    let gain = map(m, 0, TURBO_LOW_ROOM_MS, TURBO_FULL_BAR_GAIN_MS, 0);
                    let fire = self.boosts_success << 6;
                    self.fire_increment(gain, FireKind::Stack, fire);
                    self.boosts_success += 1;
                    // The first two give no wheelie (`VehPhysProc.c:2496-2499`).
                    if self.boosts_success < 3 {
                        self.flags |= TURBO_LATCH;
                    }
                    self.drift_boost_ms = DRIFT_BOOST_MS;
                    self.events.push(Event::Boost {
                        stage: self.boosts_success as u8,
                        reserves: gain,
                    });
                } else {
                    self.failed_boost = 8;
                    self.events.push(Event::BoostEarly);
                }
                self.turbo_meter = 0;
                self.boosts_attempted += 1;
            }
        } else if self.turbo_meter != 0 {
            self.turbo_meter = (self.turbo_meter - MS).max(0);
            if self.turbo_meter == 0 {
                self.boosts_attempted += 3;
                self.events.push(Event::MeterFull);
            }
        } else if self.boosts_attempted < 3 {
            self.turbo_meter = TURBO_MAX_ROOM_MS;
        }

        let held = match self.drift_button {
            Button::R1 => input.r1,
            Button::L1 => input.l1,
        };
        let lim = DRIFT_FRAMES_TILL_SPINOUT;
        if self.drift_frames > lim {
            self.spin_out(SPIN_MS);
        } else if self.speed_approx < 0 && !self.on_pad {
            // Backwards off a turbo pad (`VehPhysProc.c:2547-2551`).
            self.spin_out(REVERSE_SPIN_MS);
        } else if self.drift_frames < -lim
            || self.speed < self.stats.speed / 2
            || self.flags & (DRIVING_AGAINST_WALL | ACCEL_PREVENTION | BRAKE_WITH_ACCEL) != 0
            || !held
        {
            self.prev_mult_drift = self.mult_drift;
            self.drift_assist_ms = DRIFT_RELEASE_ASSIST_MS;
            self.end_drift();
        }
    }

    fn end_drift(&mut self) {
        self.state = State::Normal;
        self.turbo_meter = 0;
        self.events.push(Event::DriftEnd);
    }

    /// A powerslide spun out (`VehPhysProc.c:2542-2552`): SpinFirst at once, `ms` locked out. The
    /// reserves stay and keep draining: neither the spin-out nor `VehPhysProc_SpinFirst_Init`
    /// clears them (`:2859-2909`), only a hit does.
    fn spin_out(&mut self, ms: i32) {
        self.no_input = ms;
        self.spin_init();
        self.events.push(Event::DriftSpinOut);
    }

    /// PHYS_LINEAR (3.1, 3.2, 4.1): timers, the jump button, throttle, the steer command.
    fn phys_linear(&mut self, input: &Input, mods: &Modifiers, tapped_r1: bool, tapped_l1: bool) {
        self.flags_prev = self.flags;
        self.turn_angle_prev = self.turn_angle;
        let tick = |t: &mut i32| *t = (*t - MS).max(0);
        tick(&mut self.reserves);
        tick(&mut self.turbo_outside);
        tick(&mut self.wall_rub);
        tick(&mut self.accel_tap_window);
        tick(&mut self.jump_forced);
        tick(&mut self.jump_cooldown);
        self.jump_buffer = (self.jump_buffer - 1).max(0);
        self.failed_boost = (self.failed_boost - 1).max(0);
        tick(&mut self.squish);
        tick(&mut self.clock);
        tick(&mut self.invincible);
        if let State::Crashing(t) = &mut self.state {
            *t -= MS;
        }
        // SpinFirst and the tumble count their lockout down; SpinLast and SpinStop do not.
        let spinning = matches!(self.state, State::Spinning(_));
        if self.blasted || (spinning && self.spin_phase == SpinPhase::First) {
            tick(&mut self.no_input);
        }
        if let State::Spinning(t) = &mut self.state {
            *t = self.no_input;
        }
        self.flags &= CARRY;

        // The jump button (5.1): a tap buffers a hop and names the drift's button. A cloud's
        // jump lockout takes the hop and the held button (`VehPhysProc.c:921-929`).
        let hop = self.cloud != Some(CloudEffect::NoHop);
        if (tapped_r1 || tapped_l1) && self.state != State::Drifting {
            self.drift_button = if tapped_r1 { Button::R1 } else { Button::L1 };
            if hop {
                self.jump_buffer = JUMP_BUFFER_FRAMES;
            }
        }
        if (input.r1 || input.l1) && hop {
            self.flags |= JUMP_BUTTON_HELD;
        }

        // Square clears the reserves only off a turbo pad, and on one with reserves it counts as
        // let go until the boost is over (`VehPhysProc.c:953-963`, `:1015-1022`).
        if input.brake && !self.on_pad {
            self.reserves = 0;
        }
        let brake = input.brake && !(self.on_pad && self.reserves != 0);
        let cross = input.gas || self.reserves != 0;
        let b = self.target_speed(mods);
        let (gas, flags) = match (brake, cross, input.down) {
            (false, true, _) => (b, 0),
            (false, false, false) => (0, 0),
            (false, false, true) => (-BACKWARD_SPEED, REVERSING),
            (true, true, false) => (b >> 1, BRAKE_WITH_ACCEL),
            (true, _, true) => (-(3 * BACKWARD_SPEED) >> 2, REVERSING | BRAKE_WITH_ACCEL),
            (true, false, false) => (0, ACCEL_PREVENTION),
        };
        self.flags = (self.flags & !REVERSING) | flags;
        // The gas mashed (`VehPhysProc.c:1328-1357`): its count goes once the window lapses or
        // the kart is not in its normal state, and each change between going and not, within
        // the window, counts.
        if self.accel_tap_window == 0 || self.state != State::Normal {
            self.accel_tap_count = 0;
        }
        if (self.fire_speed > 0) != (gas > 0) {
            if self.accel_tap_window != 0 {
                self.accel_tap_count += 1;
            }
            self.accel_tap_window = ACCEL_TAP_WINDOW_MS;
        }
        self.fire_speed = gas;
        // Super Engine's top-up, once the gas is known (`VehPhysProc.c:1281-1308`): moving
        // forward and not reversing, not braking with the gas, and refused under accel prevention
        // as a turbo pad's is (`VehFire.c:118-127`).
        if mods.super_engine
            && input.gas
            && gas > 0
            && self.flags & (REVERSING | BRAKE_WITH_ACCEL | ACCEL_PREVENTION) == 0
        {
            let fire = if mods.wumpa >= 10 { 256 } else { 128 };
            self.fire_increment(SUPER_ENGINE_RESERVES, FireKind::SuperEngine, fire);
        }
        // A power brake at speed leaves the back tyres' marks (`VehPhysGeneral.c:435-450`).
        if self.flags & BRAKE_WITH_ACCEL != 0 && self.speed_approx > 0x300 {
            self.flags |= BACK_SKID;
        }
        // `terrainMeta2`'s multiplier (`VehPhysProc.c:1362-1388`).
        let m = self.speed_terrain.speed;
        let scaled = self.flags & BRAKE_WITH_ACCEL == 0 && !mods.mask && m != 256;
        (self.base_speed, self.terrain_base) = if scaled {
            ((gas * m) >> 8, (b * m) >> 8)
        } else {
            (gas, b)
        };
        let mut lockout = false;
        if self.blasted || matches!(self.state, State::Crashing(_) | State::Spinning(_)) {
            self.base_speed = 0;
            self.fire_speed = 0;
            // SpinStop lets go of the brake (`VehPhysProc_SpinStop_PhysLinear`).
            if !(spinning && matches!(self.spin_phase, SpinPhase::Stop(_))) {
                self.flags |= ACCEL_PREVENTION;
            }
            lockout = true;
        }
        if self.blasted {
            // `VehStuckProc_Tumble_PhysLinear`: every grounded frame bounces it back up.
            self.jump_forced = BLAST_FORCED_JUMP_MS;
            self.jump_vel = BLAST_JUMP_BASE + 2 * self.no_input;
        }

        // The steer command (4.1); + is left.
        let mut strength = self.stats.turn_rate;
        // Mashing ✕ below a pace steers hard, over a wall's rub and a brake
        // (`VehPhysProc.c:1424-1436`).
        if self.accel_tap_count >= ACCEL_TAP_STEER_COUNT && self.speed_approx < ACCEL_TAP_SPEED_MAX
        {
            strength = ACCEL_TAP_STRENGTH;
        } else if self.wall_rub != 0 {
            strength = 48;
        } else if self.flags & (ACCEL_PREVENTION | BRAKE_WITH_ACCEL) != 0 && !lockout {
            strength = if input.gas {
                map(self.speed.abs(), 0x300, self.stats.speed / 2, 64, strength)
            } else {
                64
            };
        }
        let distance = (input.steer.abs().min(1.0) * 127.0).round() as i32;
        let s = stick_strength(distance, strength);
        let simp = if input.steer < 0.0 { s } else { -s };
        if simp == 0 {
            self.frames_steering = 10000;
        } else if self.simp_turn == 0 || simp.signum() != self.simp_turn.signum() {
            self.frames_steering = 0;
            if simp > 0 {
                self.flags |= STEER_LEFT;
            } else {
                self.flags &= !STEER_LEFT;
            }
        }
        self.simp_turn = simp;
    }

    /// `VehPhysGeneral_GetBaseSpeed` (3.2): the top speed this frame.
    fn target_speed(&self, mods: &Modifiers) -> i32 {
        let wumpa = i64::from(mods.wumpa.min(9));
        let mut bonus = ((wumpa * WUMPA_NET_STAT / 10) >> 12) as i32;
        if mods.mask {
            bonus += MASK_SPEED;
        }
        let mut b = self.stats.speed;
        if self.reserves != 0 {
            b += self.fire_speed_cap;
            let cap = SINGLE_TURBO_SPEED + 2 * (SACRED_FIRE_SPEED - SINGLE_TURBO_SPEED)
                - self.fire_speed_cap;
            bonus = bonus.min(cap.max(0));
        }
        // `VehPhysGeneral.c:2082-2105`: what the hits and the clock take off.
        let mut subtract = 0;
        if self.tnt.is_some() {
            subtract = DAMAGED_SPEED >> 1;
        }
        if self.squish != 0 || self.cloud == Some(CloudEffect::Slow) {
            subtract = DAMAGED_SPEED;
        }
        if self.clock != 0 {
            subtract = subtract.max(self.clock_slowdown);
        }
        (b + bonus - subtract).min(MAX_BASE_SPEED)
    }

    /// PHYS_ANGULAR in normal driving (4.2 to 4.6).
    fn phys_angular(&mut self) {
        let grounded = self.grounded();
        // 4.2: the turn rate's response.
        let mut target = self.simp_turn << 8;
        if self.forward_dir < 0 {
            target = -target;
            self.flags ^= STEER_LEFT;
        }
        if grounded {
            target = map(self.speed_approx.abs(), 16, 0x300, 0, target);
        }
        let step = (self.stats.turn_input_delay * self.terrain.turn_response) >> 8;
        self.spin_rate = approach_i(self.spin_rate, step, target);
        let mut rot = self.spin_rate;
        // 4.3: the drift's residue.
        if self.drift_assist_ms != 0 {
            rot += map(self.drift_assist_ms, 0, 320, 0, self.prev_mult_drift);
            self.drift_assist_ms = (self.drift_assist_ms - MS).max(0);
        }
        self.visual_turn(rot);
        // 4.4: a fresh steer turns at about half rate at first.
        if self.speed_approx.abs() >= 0x300 && grounded {
            let mut sa = steer_accel(self.frames_steering);
            self.frames_steering = (self.frames_steering + 1).min(10000);
            sa = sa.min((STEER_ACCEL_TURN_VEL_SCALE * rot.abs()) >> 8);
            if self.flags & STEER_LEFT != 0 {
                sa = -sa;
            }
            rot = if rot > 0 {
                (rot + sa).max(0)
            } else {
                (rot + sa).min(0)
            };
        }
        // 4.5: yaw, and the velocity's heading turned with it (the grip).
        self.amp_turn = rot;
        self.angle = wrap(self.angle + ((rot * MS) >> 13));
        // Braking, or mashing the gas, the heading barely follows the yaw: the kart pivots
        // (`VehPhysGeneral.c:885-899`).
        let lean = if self.flags & ACCEL_PREVENTION != 0
            || self.accel_tap_count >= ACCEL_TAP_STEER_COUNT
        {
            (rot * 10) >> 8
        } else {
            (rot * self.terrain.lean) >> 8
        };
        self.vel_yaw = wrap(self.vel_yaw + ((lean * MS) >> 13));
        self.moving = rot_axis_angle(self.n1, self.angle);
        self.counter_steer();
    }

    /// 4.6: the model's yaw offset, which leans into turns and feeds the counter-steer nudge.
    fn visual_turn(&mut self, rot: i32) {
        let class_speed = self.stats.speed;
        let mut resist_min = (TURN_RESIST_MIN * class_speed) >> 8;
        let mut resist_max = (TURN_RESIST_MAX * class_speed) >> 8;
        let mut model_rot_vel = MODEL_ROT_VEL_MAX;
        if self.flags & BRAKE_WITH_ACCEL != 0 {
            resist_min >>= 1;
            resist_max >>= 1;
            model_rot_vel = map(
                self.speed.abs(),
                0x300,
                class_speed / 2,
                MODEL_ROT_VEL_MIN,
                MODEL_ROT_VEL_MAX,
            );
        }
        let full = self.stats.turn_rate << 8;
        let resistance = map(self.speed.abs(), resist_min, resist_max, full, 0);
        let mut lerp_target = 0;
        if self.speed_approx.abs() >= resist_min && rot.abs() > resistance {
            let lt = map(
                self.fire_speed.abs(),
                resist_min,
                resist_max,
                0,
                model_rot_vel,
            );
            lerp_target = rot.signum() * map(rot.abs(), resistance, full, 0, lt);
        }
        self.turn_angle_vel = self.lerp_to_forwards(lerp_target);
        self.turn_angle_vel = (self.turn_angle_vel * self.terrain.turn_angle) >> 8;
        self.turn_angle += self.turn_angle_vel;
        // The low-speed transfer: the heading takes what the model recentres by, so a stopped kart's
        // drawn yaw holds. Taking it off the model too would undo the recentring and spin it in place.
        let low = map(self.speed_approx.abs(), 0, 0x600, self.turn_angle_vel, 0);
        if low.abs() > 1 {
            self.angle = wrap(self.angle - low);
        }
    }

    fn lerp_to_forwards(&self, target: i32) -> i32 {
        let s = if target < 0 { -1 } else { 1 };
        let (cur, vel, target) = (self.turn_angle * s, self.turn_angle_vel * s, target * s);
        let desired = if self.wall_rub == WALL_RUB_MS {
            0
        } else if target < cur {
            let cap = if cur > 350 { 20 * 15 } else { 20 };
            -cap.min(((cur - target) >> 2).max(1))
        } else if cur < target {
            let cap = if cur < 0 { 60 } else { 30 };
            cap.min(((target - cur) >> 2).max(1))
        } else {
            0
        };
        approach_i(vel, 10, desired) * s
    }

    /// `VehPhysForce_CounterSteer` (4.6): a one-frame sideways nudge from the model's yaw change.
    fn counter_steer(&mut self) {
        self.nudge = Vec3::ZERO;
        let ratio = self.terrain.counter_steer;
        if self.speed_approx.abs() > 768
            && !matches!(self.state, State::Crashing(_))
            && self.wall_rub == 0
            && self.grounded()
            && ratio != 0
        {
            let v = (-8000 * ratio) >> 8;
            let da = (self.turn_angle - self.turn_angle_prev)
                .clamp(-MODEL_TURN_COUNTER_STEER, MODEL_TURN_COUNTER_STEER);
            self.nudge = self.moving * Vec3::new(v as f32 * rad(da).sin(), 0.0, 0.0);
        }
    }

    /// `VehPhysProc_PowerSlide_PhysAngular` (5.2).
    fn powerslide_angular(&mut self) {
        // (a) the velocity's heading and the kart's converge.
        let delta = diff(self.vel_yaw, self.angle);
        if delta != 0 {
            let mut step = delta >> 3;
            if step == 0 {
                step = 1;
            }
            step = step.clamp(-MS * 2, MS * 2);
            self.angle = wrap(self.angle + step);
            self.vel_yaw = wrap(self.vel_yaw - step);
        }
        // (c) the steering's share of the spin.
        let range = self.stats.turn_rate << 8;
        let s = self.simp_turn << 8;
        let m = if self.mult_drift >= 0 { 1 } else { -1 };
        let into = s * m;
        let desired = if into >= 0 {
            m * map(into, 0, range, 0, DRIFT_STANDARD << 8)
        } else {
            m * map(-into, 0, range, 0, DRIFT_SWITCH_WAY << 8)
        };
        let (d, mut c) = (desired * m, self.spin_rate * m);
        c = if d < c {
            (c - DRIFT_SPIN_RATE_DECEL).max(d)
        } else {
            (c + DRIFT_SPIN_RATE_ACCEL).min(d)
        };
        let mut current = c * m;
        // (d) the spin-out counter.
        if desired == 0 {
            self.drift_frames = approach_i(self.drift_frames, 1, 0);
        } else if desired * m > 0 {
            self.drift_frames = (self.drift_frames + 1).max(0);
        } else {
            self.drift_frames = (self.drift_frames - 1).min(0);
        }
        // (e) the drift's own turn ramps in; (f) the spin may not reverse it.
        let dti = map(
            self.drift_total_ms,
            0,
            DRIFT_TURN_RAMP_MS,
            (DRIFT_TURN_STARTUP_SCALE * self.mult_drift) >> 8,
            self.mult_drift,
        );
        current = if dti >= 0 {
            current.max(-dti)
        } else {
            current.min(-dti)
        };
        self.spin_rate = current;
        // (g) the drawn drift angle.
        let side = if self.mult_drift >= 0 { 1 } else { -1 };
        let extra = if current * side >= 0 {
            side * map(
                current.abs(),
                0,
                DRIFT_STANDARD << 8,
                0,
                DRIFT_SAME_DIR_ANGLE,
            )
        } else {
            side * DRIFT_OPPOSITE_DIR_ANGLE
        };
        let target = side * DRIFT_TURN_ANGLE_SCALE + extra;
        let mut step = (target - self.turn_angle) >> 3;
        if step == 0 && target != self.turn_angle {
            step = (target - self.turn_angle).signum();
        }
        self.turn_angle += step;
        // (h) yaw.
        self.amp_turn = current + dti;
        self.angle = wrap(self.angle + ((self.amp_turn * MS) >> 13));
        // (i) a successful boost kicks the velocity into the drift.
        if self.drift_boost_ms > 0 {
            self.drift_boost_ms -= MS;
            self.vel_yaw = wrap(self.vel_yaw + self.turn_angle.signum() * DRIFT_BOOST_AXIS_KICK);
        }
        self.drift_total_ms = (self.drift_total_ms + MS).min(DRIFT_TURN_RAMP_MS);
        self.moving = rot_axis_angle(self.n1, self.angle);
        self.counter_steer();
    }

    /// The spin-out's PHYS_ANGULAR (`VehPhysProc.c:2805-2849`, `2945-3012`, `3031-3048`): the
    /// model (`turnAngleCurr`) whirls 300 a frame over the slowing yaw, then winds in to face
    /// forward, then eases home 128 a frame.
    fn spin_angular(&mut self) {
        self.frames_steering = 10000;
        if !matches!(self.spin_phase, SpinPhase::Stop(_)) {
            self.spin_rate -= self.spin_rate >> 3;
            self.amp_turn = self.spin_rate;
        }
        match self.spin_phase {
            SpinPhase::First => {
                self.turn_angle = wrap_signed(self.turn_angle + self.spin_turn);
            }
            SpinPhase::Last => {
                let a = self.turn_angle;
                let toward = |rate: i32| {
                    let r = (-(a << 2)) >> 3;
                    if rate > 0 {
                        r.max(SPIN_MIN_RECENTER_RATE)
                    } else {
                        r.min(-SPIN_MIN_RECENTER_RATE)
                    }
                };
                if a > 0 {
                    if self.spin_turn < 0 && a < SPIN_RECENTER_ANGLE {
                        self.spin_turn = toward(-1);
                    }
                    self.turn_angle = wrap_signed(a + self.spin_turn);
                    if self.spin_turn < 0 && self.turn_angle < 0 {
                        self.turn_angle = 0;
                    }
                } else if a < 0 {
                    if self.spin_turn > 0 && a > -SPIN_RECENTER_ANGLE {
                        self.spin_turn = toward(1);
                    }
                    self.turn_angle = wrap_signed(a + self.spin_turn);
                    if self.spin_turn > 0 && self.turn_angle > 0 {
                        self.turn_angle = 0;
                    }
                }
            }
            SpinPhase::Stop(_) => {
                self.turn_angle = approach_i(self.turn_angle, (MS << 7) >> 5, 0);
            }
        }
        self.angle = wrap(self.angle + ((self.amp_turn * MS) >> 13));
        self.moving = rot_axis_angle(self.n1, self.angle);
        self.nudge = Vec3::ZERO;
    }

    /// APPLY_FORCES (6.4, 3.6): speed to velocity, gravity and friction in the kart's frame.
    fn apply_forces(&mut self) {
        self.speed = self.speed.min(MAX_SPEED);
        let (yaw, pitch) = (rad(self.vel_yaw), rad(self.vel_pitch));
        let speed = self.speed as f32;
        self.vel = Vec3::new(
            speed * pitch.cos() * yaw.sin(),
            speed * pitch.sin(),
            speed * pitch.cos() * yaw.cos(),
        );
        let to_local = self.moving.transpose();
        let mut l = to_local * self.vel;
        let l0 = l;
        let mut g = to_local * Vec3::new(0.0, -(GRAVITY as f32), 0.0);
        if (g.z < 0.0 && self.fwd_accel_impulse > 0) || (g.z > 0.0 && self.fwd_accel_impulse < 0) {
            g.z = 0.0;
        }
        let opposite = (self.base_speed > 0 && self.speed_approx < 0)
            || (self.base_speed < 0 && self.speed_approx > 0);
        if self.flags & ACCEL_PREVENTION != 0 || opposite {
            g.x = 0.0;
            g.z = 0.0;
        }
        l += g;
        // Gravity may push only so far past these; it never takes back what was already there.
        let fire = self.fire_speed as f32;
        if g.z > 0.0 {
            l.z = l.z.min(l0.z.max(fire + SLOPE_FORWARD_SPEED_BONUS as f32));
        } else if g.z < 0.0 {
            l.z = l.z.max(l0.z.min(fire - 2560.0));
        }
        let side = SIDE_SPEED_CLAMP as f32;
        l.x = l.x.clamp(l0.x.min(-side), l0.x.max(side));
        l.y = l.y.max(l0.y.min(-(TERMINAL_VELOCITY as f32)));
        // A blasted kart's friction acts in the air too, and a slow ground's drag through a hop
        // off it (`VehPhysForce.c:429-430`).
        let slow_drag = self.speed_approx > self.terrain_base && self.speed_terrain.speed < 256;
        if self.flags_prev & TOUCH_GROUND != 0 || self.blasted || slow_drag {
            self.friction(&mut l);
        }
        if l.z.abs() > 64.0 {
            self.forward_dir = if l.z >= 0.0 { 1 } else { -1 };
        }
        self.vel = self.moving * l + self.nudge;
    }

    /// Ground friction (3.6), in the kart's frame.
    fn friction(&mut self, l: &mut Vec3) {
        let fast = self.speed_approx.abs() > 0x300;
        let opposing = fast
            && ((self.base_speed > 0 && self.speed_approx < 0)
                || (self.base_speed < 0 && self.speed_approx > 0));
        // The back tyres skid under the brake, against their direction, or spinning at under
        // half the target speed (`VehPhysForce.c:448-534`).
        let wheelspin = self.base_speed != 0
            && !opposing
            && self.state != State::Drifting
            && fast
            && self.speed_approx.abs() < self.base_speed.abs() >> 1;
        if (fast && (self.flags & ACCEL_PREVENTION != 0 || opposing)) || wheelspin {
            self.flags |= BACK_SKID;
        }
        let (mut perp, mut fwd) = if self.flags & ACCEL_PREVENTION != 0 {
            let f = if self.blasted {
                (BRAKE_FRICTION * 3) >> 2
            } else if matches!(self.state, State::Spinning(_)) {
                BRAKE_FRICTION / 2
            } else {
                BRAKE_FRICTION
            };
            (f, f)
        } else if self.base_speed != 0 {
            if opposing {
                (PEDAL_FRICTION_PERP, BRAKE_FRICTION)
            } else if self.state == State::Drifting {
                (DRIFT_CURVE, DRIFT_FRICTION)
            } else {
                (PEDAL_FRICTION_PERP, PEDAL_FRICTION_FWD)
            }
        } else {
            (NO_PEDAL_FRICTION_PERP, NO_PEDAL_FRICTION_FWD)
        };
        let gfs = self.terrain.ground_friction;
        if gfs != 256 {
            perp = (perp * gfs) >> 8;
            fwd = (fwd * gfs) >> 8;
        }
        if self.grip_timer > 0 {
            perp += perp;
        } else if self.grip_timer < 0 {
            // Just off ice (`VehPhysForce.c:580-649`): the first frame takes half the sideways
            // speed, the rest double grip, and the tyres skid while it slides.
            perp = if self.grip_timer == ICE_EXIT_MS {
                (l.x.abs() / 2.0) as i32
            } else {
                perp + perp
            };
            if l.x.abs() >= 1.0 {
                self.flags |= BACK_SKID | FRONT_SKID;
            }
            self.grip_timer = (self.grip_timer + MS).min(0);
        }
        let flags = self.terrain.flags;
        if flags & crate::terrain::flags::MUD_PHYSICS != 0 && !self.masked() {
            // Mud's least frictions (`VehPhysForce.c:680-787`): an eighth of the sideways
            // speed, and half of any speed past the target (half the speed when against it).
            perp = perp.max((l.x / 8.0).abs() as i32);
            let (z, base) = (l.z, self.base_speed as f32);
            let least = if z != 0.0 && base != 0.0 && (z < 0.0) != (base < 0.0) {
                (z / 2.0).abs()
            } else if (z < base && z <= 0.0) || (base < z && z >= 0.0) {
                (z - base).abs() / 2.0
            } else {
                0.0
            };
            fwd = fwd.max(least as i32);
        }
        if flags & crate::terrain::flags::SIDESLIP_FRICTION != 0 {
            perp = ((perp * 3) >> 2).max(fwd);
        }
        l.x = approach(l.x, perp as f32, 0.0);
        l.z = approach(l.z, fwd as f32, 0.0);
    }

    /// COLL_MOVED (9.1, 9.2): the kart's sphere moved by its velocity in sub-steps, pushed out of
    /// what it overlaps, with CTR's scrub on each impact: the ground takes the into-surface speed
    /// and becomes the kart's normal; a wall rubs, ends turbos and drifts, and head-on, slams.
    fn sweep(&mut self, world: &Collision, mods: &Modifiers) {
        // Ground contact is rebuilt each frame: by this sweep, then by the probe.
        self.flags &= !TOUCH_GROUND;
        let r = SPHERE_RADIUS;
        let lift = self.facing.y_axis * r;
        let mut centre = self.pos + lift;
        let delta = self.vel / 256.0 * (MS as f32 / 32.0);
        let steps = (delta.length() / (r * 0.4)).ceil().clamp(1.0, 32.0) as usize;
        let step = delta / steps as f32;
        let mut slammed = false;
        let floor = self.water.map(|s| s - DRAFT);
        'outer: for _ in 0..steps {
            centre += step;
            for _ in 0..4 {
                // The water's floor is a plane under the sphere, the deeper contact winning.
                let plane = floor.and_then(|f| {
                    let depth = f - (centre.y - r);
                    (depth > 0.0).then_some(crate::collision::Contact {
                        depth,
                        normal: Vec3::Y,
                        point: Vec3::new(centre.x, f, centre.z),
                    })
                });
                let contact = match (world.deepest(centre, r), plane) {
                    (Some(w), Some(p)) => {
                        if p.depth > w.depth {
                            p
                        } else {
                            w
                        }
                    }
                    (Some(w), None) => w,
                    (None, Some(p)) => p,
                    (None, None) => break,
                };
                centre += contact.normal * (contact.depth + 0.01);
                let ground = contact.normal.y >= GROUND_NORMAL_Y;
                let dot = self.vel.dot(contact.normal);
                if dot < -2560.0 {
                    self.flags |= TURBO_LATCH;
                }
                if ground {
                    self.n1 = contact.normal;
                    self.flags |= TOUCH_GROUND;
                    if dot < 0.0 {
                        self.vel -= contact.normal * dot;
                    }
                    continue;
                }
                if dot >= 0.0 {
                    continue;
                }
                if mods.mask {
                    // Aku Aku: the wall is a guide rail, never a stop.
                    self.vel -= contact.normal * dot;
                    continue;
                }
                self.flags |= DRIVING_AGAINST_WALL;
                self.reserves = 0;
                self.turbo_outside = 0;
                if self.wall_rub == 0 {
                    self.events.push(Event::WallRub);
                }
                self.wall_rub = WALL_RUB_MS;
                let horizontal = Vec3::new(self.vel.x, 0.0, self.vel.z).length();
                let head_on = -dot / self.vel.length().max(1.0);
                if dot < -5119.0 && horizontal > 5120.0 && head_on > 0.879 {
                    slammed = true;
                    break 'outer;
                }
                self.vel -= contact.normal * dot;
            }
        }
        self.pos = centre - lift;
        if slammed {
            if self.state == State::Drifting {
                self.camera_spin -= self.turn_angle;
                self.angle = wrap(self.angle + self.turn_angle);
                self.turn_angle = 0;
                self.events.push(Event::DriftEnd);
            }
            self.events.push(Event::Slam {
                speed: self.speed.abs(),
            });
            self.state = State::Crashing(SLAM_MS);
            self.vel = Vec3::ZERO;
            self.speed = 0;
            self.reserves = 0;
            self.turbo_meter = 0;
        }
    }

    /// COLL_FIXED (9.3, 6.1, 6.5): the ground under the kart, landings, coyote time and airtime.
    fn ground_probe(&mut self, world: &Collision) {
        let was = self.flags_prev & TOUCH_GROUND != 0;
        let mut hit = world.ground(self.pos + Vec3::Y * 128.0, 384.0);
        // The water's floor, where it is above the ground (or there is none in reach).
        self.afloat = false;
        if let Some(f) = self.water.map(|s| s - DRAFT) {
            let from = self.pos.y + 128.0;
            if f <= from && hit.as_ref().is_none_or(|h| f > h.point.y) && from - f <= 384.0 {
                hit = Some(crate::collision::Hit {
                    point: Vec3::new(self.pos.x, f, self.pos.z),
                    normal: Vec3::Y,
                    distance: from - f,
                });
                self.afloat = self.pos.y <= f + 16.0;
            }
        }
        // `quadBlockHeight` (`COLL.c:1362-1373`): the ground found, else the search's bottom.
        self.ground = hit.as_ref().map_or(self.pos.y - 256.0, |h| h.point.y);
        let mut grounded = self.flags & TOUCH_GROUND != 0;
        if let Some(hit) = hit {
            if self.pos.y <= hit.point.y + 16.0 {
                grounded = true;
                if self.n1 == Vec3::Y {
                    self.n1 = hit.normal;
                }
            }
        }
        if grounded {
            self.flags |= TOUCH_GROUND;
            // `COLL.c:1476`: a cloud's ice effect makes any ground ice.
            let under = if self.cloud == Some(CloudEffect::Ice) {
                Terrain::ICE
            } else if self.afloat {
                if self.reserves > 0 {
                    Terrain::PLANING
                } else {
                    Terrain::AFLOAT
                }
            } else {
                self.ground_type
            };
            // Off ice the sideways grip is rebuilt over 320 ms (`COLL.c:1478-1481`).
            if self.terrain == Terrain::ICE && under != Terrain::ICE {
                self.grip_timer = ICE_EXIT_MS;
            }
            self.terrain = under;
            self.speed_terrain = under;
            if !was {
                self.flags |= STARTED_TOUCH_GROUND | TURBO_LATCH;
                self.grip_timer = LANDING_GRIP_MS;
                self.events.push(Event::Land {
                    air_ms: self.airtime,
                    fall: self.fall,
                });
            }
            self.coyote = COYOTE_MS;
            self.airtime = 0;
            self.fall = 0;
        } else {
            self.flags &= !TOUCH_GROUND;
            self.terrain = Terrain::NONE;
            // `quadBlockHeight + 0x8000 < posCurr.y` (`COLL.c:1431-1435`).
            if self.pos.y > self.ground + 128.0 {
                self.speed_terrain = Terrain::NONE;
            }
            self.coyote = (self.coyote - MS).max(0);
            if self.coyote == 0 {
                self.jump_cooldown = 0;
            }
            self.airtime += MS;
            self.fall = self.fall.max((-self.vel.y) as i32);
            if self.jump_cooldown != 0 {
                self.flags |= AIRBORNE;
            }
        }
        if self.grip_timer > 0 {
            self.grip_timer = (self.grip_timer - MS).max(0);
        }
    }

    /// JUMP_FRICTION (3.3 to 3.5, 6.2, 6.3): turn penalty, the engine, the hop, velocity to speed.
    fn jump_and_friction(&mut self, mods: &Modifiers) {
        if self.state != State::Drifting && !mods.mask && self.reserves == 0 {
            let t = (self.amp_turn >> 8).abs();
            let mut dec = map(t, 0, BACKWARD_TURN_RATE, 0, self.stats.turn_decrease)
                .min(self.base_speed.abs());
            // Deviates (the owner's call): CTR takes the turn's whole decrease off a reverse too
            // (`VehPhysGeneral_4_JumpAndFriction.c`), which at full lock leaves an Accel or Turn
            // kart a tenth of its reverse speed; here a turn takes at most half of it.
            if self.base_speed < 0 {
                dec = dec.min(self.base_speed.abs() / 2);
            }
            self.base_speed -= self.base_speed.signum() * dec;
        }
        if self.wall_rub != 0 {
            self.base_speed = self
                .base_speed
                .clamp(-WALL_RUB_SPEED_LIMIT, WALL_RUB_SPEED_LIMIT);
        }
        let mut mv = self.vel;
        let mut loss = 0.0;
        if self.grounded() {
            let acc = self.engine_accel(mods);
            let f = self.moving * Vec3::new(0.0, 0.0, acc as f32);
            if self.base_speed >= 0 {
                mv += f;
                self.fwd_accel_impulse = acc;
            } else {
                mv -= f;
                self.fwd_accel_impulse = -acc;
            }
            loss = (mv.length() - self.base_speed.abs() as f32).clamp(0.0, acc as f32);
        } else {
            self.fwd_accel_impulse = 0;
        }
        let lockout = self.blasted || matches!(self.state, State::Crashing(_) | State::Spinning(_));
        if self.grounded() && self.jump_forced != 0 {
            self.process_jump(&mut mv);
        } else if std::mem::take(&mut self.high_jump) {
            // `FORCED_JUMP_HIGH` (`VehPhysGeneral.c:1395-1409`): three times the jump force.
            self.jump_forced = JUMP_FORCED_MS;
            self.jump_vel = 3 * JUMP_FORCE;
            self.process_jump(&mut mv);
        } else if !lockout && self.coyote != 0 && self.jump_buffer != 0 && self.jump_cooldown == 0 {
            self.jump_forced = JUMP_FORCED_MS;
            self.jump_vel = JUMP_FORCE;
            self.process_jump(&mut mv);
            self.hopped = true;
            self.events.push(Event::Hop);
        }
        // `VehPhysCrash_ConvertVecToSpeed`.
        let speed = mv.length();
        let flat = Vec3::new(mv.x, 0.0, mv.z).length();
        if speed > 1e-3 {
            self.vel_yaw = atan2_units(mv.x, mv.z);
            self.vel_pitch = atan2_units(mv.y, flat);
        }
        let up = self.moving.y_axis;
        self.jump_height = mv.dot(up) as i32;
        let planar = mv - up * mv.dot(up);
        let along = planar.dot(self.moving.z_axis);
        self.speed_approx = (planar.length() * along.signum()) as i32;
        self.speed = ((speed - loss).max(0.0)) as i32;
        self.vel = mv;
    }

    /// The engine's push (3.4).
    fn engine_accel(&self, mods: &Modifiers) -> i32 {
        let base = self.base_speed;
        let sa = self.speed_approx;
        if base == 0 {
            return 0;
        }
        if self.on_pad && base > 0 {
            return TURBO_PAD_ACCEL;
        }
        let wrong_way = sa.abs() > 0x2FF && (base <= 0 || sa <= 0) && (base >= 0 || sa >= 0);
        let reverse_slide = self.terrain.flags & crate::terrain::flags::ACCEL_WHILE_REVERSE_SLIDING
            != 0
            && base > 0
            && sa < 0;
        if wrong_way && !reverse_slide {
            return 0;
        }
        let mut acc = self.stats.accel;
        if self.reserves != 0 && base > 0 {
            acc = ACCEL_RESERVES;
        }
        if self.terrain.accel != 256 && !mods.mask {
            acc = (acc * self.terrain.accel) >> 8;
        }
        acc
    }

    /// PROCESS_JUMP (6.3): the hop's vertical speed, plus the slope's.
    fn process_jump(&mut self, mv: &mut Vec3) {
        self.jump_cooldown = JUMP_COOLDOWN_MS;
        self.jump_buffer = 0;
        self.jump_started = true;
        let slope = |n: Vec3| {
            if (n.y * 4096.0).abs() < 21.0 {
                0.0
            } else {
                (mv.x * n.x + mv.z * n.z) / n.y
            }
        };
        let a = slope(self.n4);
        let b = slope(if self.grounded() { self.n1 } else { self.n2 });
        let best = if a.abs() >= b.abs() { a } else { b };
        let j = self.jump_vel as f32;
        let vy = (best * best + j * j).sqrt() - best;
        mv.y = mv.y.max(vy.min(JUMP_VERTICAL_CAP as f32));
    }

    /// TRANSLATE_MATRIX's squash and stretch (`VehPhysForce.c:1643-1772`): a hop crouches, a
    /// fall stretches, a hard landing squashes by the drop in up-speed, and `jumpSquishStretch`
    /// settles 300 a frame while the model's scale follows it 0xA0 a frame, the width and length
    /// giving back 0xA0/256 of it. A squish holds the height at 0; as it wears off the height
    /// snaps to the squash's and the baked recovery bounce plays (`:1740-1750`, then
    /// `VehPhysForce.c:1335-1343`). Not ported: the clock's and rain cloud's `hazardTimer` hold
    /// (`:1691-1695`) and a TNT's shrink on landing (`:1711-1714`).
    fn squash_stretch(&mut self) {
        let jh = self.jump_height;
        let mut target = SQUASH_DEFAULT;
        if !self.jump_started {
            let lag = self.squash_lag;
            target = (lag - ((lag * 9 + jh * 7) >> 4)) << 2;
            if target.abs() < SQUASH_DEADBAND {
                target = 0;
            }
            let landing = (self.flags | self.flags_prev) & STARTED_TOUCH_GROUND != 0;
            let least = if landing {
                SQUASH_LANDING_MIN
            } else {
                SQUASH_AIR_MIN
            };
            target = target.max(least).min(SQUASH_MAX);
        }
        if !self.grounded() && jh < 0 {
            let fall = map(
                -jh,
                0,
                FALL_STRETCH_SPEED,
                FALL_STRETCH_MIN,
                FALL_STRETCH_MAX,
            );
            target = target.max(fall);
            self.squash_lag = jh;
        }
        if self.squash.abs() < target.abs() {
            self.squash = target;
        }
        self.squash = approach_i(self.squash, SQUASH_EASE, 0);
        self.squash_lag = (self.squash_lag * 9 + jh * 7) >> 4;

        let height = self.squash + SCALE_BASE;
        let [x, y, z] = &mut self.model_scale;
        if self.squish != 0 {
            *y = 0;
        } else if *y == 0 {
            *y = height;
            self.squish_recover = Some(0);
            self.events.push(Event::SquishRestored);
        } else {
            *y = approach_i(*y, SCALE_EASE, height);
        }
        let across = SCALE_BASE - self.squash * SQUASH_XZ / 256;
        *x = approach_i(*x, SCALE_EASE, across);
        *z = approach_i(*z, SCALE_EASE, across);
        // `VehPhysForce_TranslateMatrix_UpdateMatrixAnimation`: the bounce steps before it is
        // drawn, so its entries 1 to 14 show.
        self.squish_recover = self
            .squish_recover
            .map(|i| i + 1)
            .filter(|&i| i < SQUISH_RECOVER.len());
    }

    /// `UI_JumpMeter_Update` (6.6): a long enough hop pays out on landing.
    fn hang_time(&mut self) {
        let airborne = self.flags & AIRBORNE != 0;
        if airborne && !self.grounded() {
            self.jump_meter = self.airtime.min(HANG_TIME_METER_MAX);
            return;
        }
        if airborne {
            self.flags &= !AIRBORNE;
            let m = self.jump_meter;
            self.jump_meter = 0;
            if m >= HANG_TIME_MIN_MS {
                let (fire, level) = if m >= HANG_TIME_HIGH_MS {
                    (0x100, 3)
                } else if m >= HANG_TIME_MID_MS {
                    (0x80, 2)
                } else {
                    (0, 1)
                };
                if self.fire_increment(HANG_TIME_RESERVES, FireKind::Stack, fire) {
                    self.events.push(Event::HangTime { level });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
