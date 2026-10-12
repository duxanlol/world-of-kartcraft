//! The engine classes and their physics constants: CTR's MetaPhys table
//! (`zGlobal_DATA.c:7122-7189` in the decompilation), copied into each driver at spawn. Only the
//! entries that differ between classes are per class; the rest are shared constants here.

/// The four engine classes of NTSC-U CTR.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Class {
    /// Crash, Cortex, Komodo Joe, Fake Crash, N. Oxide.
    #[default]
    Balanced,
    /// Coco, N. Gin, Pinstripe.
    Accel,
    /// Tiny, Dingodile, Papu Papu, N. Tropy.
    Speed,
    /// Polar, Pura, Ripper Roo, Penta Penguin.
    Turn,
}

impl Class {
    pub const ALL: [Class; 4] = [Class::Balanced, Class::Accel, Class::Speed, Class::Turn];

    pub fn name(self) -> &'static str {
        match self {
            Class::Balanced => "Balanced",
            Class::Accel => "Acceleration",
            Class::Speed => "Speed",
            Class::Turn => "Turning",
        }
    }

    pub fn stats(self) -> Stats {
        // (Accel_ClassStat, Speed_ClassStat, TurnRate, TurnDecreaseRate, TurnInputDelay,
        //  DriftTurnBase)
        let (accel, speed, turn_rate, turn_decrease, turn_input_delay, drift_turn_base) = match self
        {
            Class::Balanced => (480, 13140, 28, 4080, 5000, 14),
            Class::Accel => (544, 13520, 26, 9080, 4500, 10),
            Class::Speed => (448, 13900, 24, 5666, 4000, 5),
            Class::Turn => (512, 12950, 30, 7252, 5500, 18),
        };
        Stats {
            accel,
            speed,
            turn_rate,
            turn_decrease,
            turn_input_delay,
            drift_turn_base,
            // AccelSpeed_ClassStat is Speed_ClassStat + 1500 in every class.
            accel_speed: speed + 1500,
            collision_weight: match self {
                Class::Speed => 300,
                Class::Turn => 200,
                _ => 256,
            },
        }
    }
}

/// The per-class constants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stats {
    /// Accel_ClassStat: engine push per frame.
    pub accel: i32,
    /// Speed_ClassStat: top speed.
    pub speed: i32,
    /// TurnRate: steer strength at full lock.
    pub turn_rate: i32,
    /// TurnDecreaseRate: top speed lost at full steer.
    pub turn_decrease: i32,
    /// TurnInputDelay: steering response per frame.
    pub turn_input_delay: i32,
    /// DriftTurnBase: the powerslide's base turn.
    pub drift_turn_base: i32,
    /// AccelSpeed_ClassStat: the engine sound's scale.
    pub accel_speed: i32,
    /// CollisionWeight: how hard this kart is to push about.
    pub collision_weight: i32,
}

// The shared MetaPhys entries (section 10 of `docs/ctr-physics-spec.md`).
pub const GRAVITY: i32 = 900;
pub const JUMP_FORCE: i32 = 4596;
pub const PEDAL_FRICTION_PERP: i32 = 350;
pub const PEDAL_FRICTION_FWD: i32 = 120;
pub const NO_PEDAL_FRICTION_PERP: i32 = 425;
pub const NO_PEDAL_FRICTION_FWD: i32 = 120;
pub const BRAKE_FRICTION: i32 = 260;
pub const DRIFT_CURVE: i32 = 120;
pub const DRIFT_FRICTION: i32 = 120;
pub const ACCEL_RESERVES: i32 = 1152;
pub const SINGLE_TURBO_SPEED: i32 = 2048;
pub const SACRED_FIRE_SPEED: i32 = 4096;
pub const BACKWARD_SPEED: i32 = 5120;
pub const MASK_SPEED: i32 = 3900;
pub const BACKWARD_TURN_RATE: i32 = 48;
pub const TERMINAL_VELOCITY: i32 = 20480;
pub const STEER_ACCEL_STAGE4_FIRST: i32 = 64;
pub const STEER_ACCEL_STAGE2_FIRST: i32 = 4;
pub const STEER_ACCEL_STAGE2_LEN: i32 = 8;
pub const STEER_ACCEL_MAX: i32 = 3072;
pub const STEER_ACCEL_TURN_VEL_SCALE: i32 = 128;
pub const MODEL_ROT_VEL_MAX: i32 = 350;
pub const MODEL_ROT_VEL_MIN: i32 = 450;
pub const MODEL_TURN_COUNTER_STEER: i32 = 30;
pub const TURN_RESIST_MIN: i32 = 32;
pub const TURN_RESIST_MAX: i32 = 224;
pub const DRIFT_SWITCH_WAY: i32 = -10;
pub const DRIFT_STANDARD: i32 = 22;
pub const DRIFT_TURN_STARTUP_SCALE: i32 = 100;
pub const DRIFT_TURN_RAMP_MS: i32 = 60 << 5;
pub const DRIFT_FRAMES_TILL_SPINOUT: i32 = 60;
pub const DRIFT_SPIN_RATE_ACCEL: i32 = 3000;
pub const DRIFT_SPIN_RATE_DECEL: i32 = 500;
pub const DRIFT_RELEASE_ASSIST_MS: i32 = 15 << 5;
pub const DRIFT_SAME_DIR_ANGLE: i32 = 300;
pub const DRIFT_OPPOSITE_DIR_ANGLE: i32 = -128;
pub const DRIFT_TURN_ANGLE_SCALE: i32 = 768;
/// TurboMaxRoom: the powerslide meter fills over 30 frames.
pub const TURBO_MAX_ROOM_MS: i32 = 30 << 5;
/// TurboLowRoomWarning: the red zone is its last 15 frames.
pub const TURBO_LOW_ROOM_MS: i32 = 15 << 5;
/// TurboFullBarReserveGain: a boost at the very end of the meter.
pub const TURBO_FULL_BAR_GAIN_MS: i32 = 60 << 5;
pub const DRIFT_BOOST_MS: i32 = 15 << 5;
pub const DRIFT_BOOST_AXIS_KICK: i32 = 36;
pub const SLOPE_FORWARD_SPEED_BONUS: i32 = 5120;
pub const SIDE_SPEED_CLAMP: i32 = 2560;
/// The speed field's own clamp (`VehPhysForce.c:929-932`).
pub const MAX_SPEED: i32 = 25600;
/// `0x6400`: the target speed's clamp.
pub const MAX_BASE_SPEED: i32 = 0x6400;
/// `(AccelSpeed_ClassStat − Speed_ClassStat) << 12) / 5 − 1`, the same for every class.
pub const WUMPA_NET_STAT: i64 = ((1500 << 12) / 5) - 1;
pub const WALL_RUB_MS: i32 = 240;
pub const WALL_RUB_SPEED_LIMIT: i32 = 3328;
pub const COYOTE_MS: i32 = 160;
pub const JUMP_COOLDOWN_MS: i32 = 384;
pub const JUMP_FORCED_MS: i32 = 160;
pub const JUMP_BUFFER_FRAMES: i32 = 10;
/// `0x3700`, the default cap on a jump's vertical speed.
pub const JUMP_VERTICAL_CAP: i32 = 0x3700;
pub const LANDING_GRIP_MS: i32 = 320;
/// The hang-time meter's thresholds (`UI_Meter.c:59-121`).
pub const HANG_TIME_MIN_MS: i32 = 640;
pub const HANG_TIME_MID_MS: i32 = 960;
pub const HANG_TIME_HIGH_MS: i32 = 1440;
pub const HANG_TIME_METER_MAX: i32 = 2400;
pub const HANG_TIME_RESERVES: i32 = 960;
/// The turbo item: reserves and fire, juiced fire (`VehPickupItem.c`).
pub const TURBO_ITEM_RESERVES: i32 = 2400;
/// A turbo pad: the reserves it tops up to (`VEH_PHYS_FORCE_TURBO_PAD_RESERVES`) and the
/// engine's acceleration on it (`VEH_PHYS_JUMP_TURBO_PAD_ACCEL`).
pub const TURBO_PAD_RESERVES: i32 = 0x3C0;
pub const TURBO_PAD_ACCEL: i32 = 8000;
/// Off ice, the sideways grip's timer starts here (`0xfec0`, `COLL.c:1478-1481`).
pub const ICE_EXIT_MS: i32 = -320;
/// Super Engine tops reserves up to this every frame while the gas is held.
pub const SUPER_ENGINE_RESERVES: i32 = 120;
/// The kart's collision sphere, radius and centre height above the origin (lu).
pub const SPHERE_RADIUS: f32 = 25.0;
/// Spin-out and wall-slam lockouts (CTR-ms); the slam's is the animation's length, estimated.
pub const SPIN_MS: i32 = 0x3C0;
pub const SLAM_MS: i32 = 640;
/// `THREAD_DRIVER_HIT_RADIUS`: each kart's sphere against other karts, lu.
pub const KART_HIT_RADIUS: f32 = 64.0;
/// `VEH_PHYS_CRASH_BOUNCE_Y_CLAMP`: a bump's upward speed at most.
pub const BOUNCE_Y_CLAMP: f32 = 0x3200 as f32;

// Hit reactions and the self-applied items (section 13 of `docs/ctr-physics-spec.md`).
/// DamagedSpeed (MetaPhys 0x11): the top speed a squish takes; half of it under a TNT.
pub const DAMAGED_SPEED: i32 = 4096;
/// `VehPickState.c:24-30`: the `NoInputTimer` each damage type sets, CTR-ms.
pub const SPIN_HIT_MS: i32 = 0x3C0;
pub const BLAST_MS: i32 = 0x960;
pub const SQUISH_NO_INPUT_MS: i32 = 0xF0;
/// `squishTimer`: drawn flat and slowed, CTR-ms.
pub const SQUISH_MS: i32 = 0xF00;
/// `invincibleTimer` after a shield pops, CTR-ms.
pub const SHIELD_POP_INVINCIBLE_MS: i32 = 0x2A0;
/// A drift spun out by rolling backwards (`VehPhysProc.c:86`), CTR-ms.
pub const REVERSE_SPIN_MS: i32 = 0x780;
/// SpinFirst's whirl, angle units a frame, and the speed under which it gives way to SpinLast
/// (`VehPhysProc.c:88-99`).
pub const SPIN_RATE: i32 = 300;
pub const SPIN_SLOW_SPEED: i32 = 0x2FF;
/// SpinLast recentres inside this angle at half the remaining angle a frame, at least 32, and
/// SpinStop takes over under 16.
pub const SPIN_RECENTER_ANGLE: i32 = 400;
pub const SPIN_MIN_RECENTER_RATE: i32 = 0x20;
pub const SPIN_STOP_ANGLE: i32 = 16;
/// SpinStop's return of the steering animation to centre (`VehPhysProc_SpinStop_Animate`): its
/// length is the model's frame count, which the code does not hold. An estimate.
pub const SPIN_STOP_ANIM_FRAMES: i32 = 8;
/// The blasted kart's bounce, `jump_InitialVelY = 6000 + 2 * NoInputTimer`, re-armed every frame
/// (`VehStuckProc.c:54-56`, `1376-1381`).
pub const BLAST_JUMP_BASE: i32 = 6000;
pub const BLAST_FORCED_JUMP_MS: i32 = 0x60;
/// `bakedGteMath[BLASTED]`: 27 tumble frames, the kart turning about a pivot 32 lu over its origin
/// (`Vector.c:155-232`, `regionsEXE.h:2246`).
pub const BLAST_TUMBLE_FRAMES: i32 = 0x1B;
pub const BLAST_PIVOT: f32 = 0x2000 as f32 / 256.0;
/// A squished kart is drawn this far up its normal, lu (`VehPhysForce.c:123`, `1453-1458`).
pub const SQUISH_LIFT: f32 = 0x13 as f32;
/// The squash and stretch (`VehPhysForce.c:105-122`, `1643-1772`): the instance's scale as built
/// (0xCCC), the squash's resting target and its dead band, its least on landing and in the air and
/// its most, the fall's stretch (0x280 to 0x320 over 0 to 0xA00 of fall speed), how fast
/// `jumpSquishStretch` settles a frame, how fast the scale follows it, and how much of it the
/// width and length give back (0xA0/256).
pub const SCALE_BASE: i32 = 0xCCC;
pub const SQUASH_DEFAULT: i32 = -800;
pub const SQUASH_DEADBAND: i32 = 0x960;
pub const SQUASH_LANDING_MIN: i32 = -0x640;
pub const SQUASH_AIR_MIN: i32 = -800;
pub const SQUASH_MAX: i32 = 800;
pub const FALL_STRETCH_SPEED: i32 = 0xA00;
pub const FALL_STRETCH_MIN: i32 = 0x280;
pub const FALL_STRETCH_MAX: i32 = 0x320;
pub const SQUASH_EASE: i32 = 300;
pub const SCALE_EASE: i32 = 0xA0;
pub const SQUASH_XZ: i32 = 0xA0;
/// `bakedGteMath[SQUISH_RECOVER]` (`zGlobal_DATA.c:5673-5795`): the bounce as a squished kart
/// springs back, a frame each: the lift up the kart (1/256 lu) and the X, Y, Z scale (4096ths).
pub const SQUISH_RECOVER: [[i32; 4]; 15] = [
    [3091, 4096, 2227, 4096],
    [3896, 3704, 4456, 3704],
    [4418, 3200, 5869, 3200],
    [4636, 3086, 6116, 3086],
    [4418, 3200, 5869, 3200],
    [3338, 3758, 5251, 3758],
    [1669, 4550, 4217, 4550],
    [0, 4952, 3246, 4952],
    [0, 4670, 2816, 4670],
    [0, 4049, 3073, 4049],
    [0, 3427, 3639, 3427],
    [0, 3145, 4205, 3145],
    [0, 3294, 4463, 3294],
    [0, 3620, 4367, 3620],
    [0, 3947, 4191, 3947],
];
/// `RB_Player_ToggleFlicker` (`231/RB_Player.c:231-257`): with more `invincibleTimer` than this
/// left (CTR-ms), the kart is not drawn on odd frames.
pub const FLICKER_INVINCIBLE_MS: i32 = 0x2A0;
/// Aku Aku / Uka Uka (`ovr_231.h:51-52`) and its fade, during which it still protects
/// (`RB_MaskShieldCloud.c:19-81`, `180-202`), CTR-ms.
pub const MASK_MS: i32 = 0x1E00;
pub const MASK_JUICED_MS: i32 = 0x2D00;
pub const MASK_FADE_MS: i32 = 0x200;
/// The green shield's life and the fade over its last 60 frames (`VehPickupItem.c:130`,
/// `RB_MaskShieldCloud.c:374-390`), CTR-ms.
pub const SHIELD_MS: i32 = 0x2D00;
pub const SHIELD_FADE_MS: i32 = 1920;
/// `clockReceive` on a clocked kart (`VehPickupItem.c:132-133`), CTR-ms.
pub const CLOCK_MS: i32 = 0x1E00;
pub const CLOCK_JUICED_MS: i32 = 0x2D00;
/// TNT on the head (`RB_TNT.c`): the throw onto the head (lu a frame, its gravity and terminal
/// speed), Crash's head height (lu, the table at `:78`), the 90-frame fuse, and the hops to shed
/// it, `8 − MixRNG % 2` more after the first.
pub const TNT_THROW_VY: i32 = 0x30;
pub const TNT_GRAVITY: i32 = 4;
pub const TNT_TERMINAL_VY: i32 = -0x60;
pub const TNT_HEAD_HEIGHT: i32 = 32;
pub const TNT_FUSE_FRAMES: i32 = 0x5A;
pub const TNT_JUMPS: i32 = 8;
