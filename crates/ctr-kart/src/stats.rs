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
