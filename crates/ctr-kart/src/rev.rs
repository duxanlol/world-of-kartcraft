//! Revving on the grid before GO: CTR's `KS_ENGINE_REVVING` (`VehStuckProc.c:944-1340`). The kart
//! stands; the gas fills a rev level toward a target, the boost meter, at half the distance a
//! frame; held at the target too long the engine over-revs and is locked out until the gas is let
//! go and the revs die. Let go above the class's acceleration speed and the revs fall back under
//! it, the target steps a third of the way to its top: pumping the gas raises it. At GO an engine
//! revved above the acceleration speed and not locked out gets the start boost, big with the
//! target at its top, else small (`VehStuckProc_RevEngine_Update`).

use crate::stats::SACRED_FIRE_SPEED;

/// `VehStuckProc.c:17-23`: the fill step's bounds, the over-rev's time at the target, the
/// cooldown after letting go, the decay's bounds and the cooldown once the revs are dead.
const FILL_STEP_MAX: i32 = 5000;
const STEP_MIN: i32 = 0x100;
const OVERREV_TIMEOUT_MS: i32 = 192;
const RELEASE_COOLDOWN_MS: i32 = 0x100;
const DECAY_STEP_MAX_NORMAL: i32 = 1000;
const DECAY_STEP_MAX_LOCKED: i32 = 3000;
const EMPTY_COOLDOWN_MS: i32 = 0xC0;
/// `VehStuckProc.c:11-12`: the start boost's fire levels; `:13` its reserves.
pub const SMALL_BOOST_FIRE: i32 = 0x20;
pub const BIG_BOOST_FIRE: i32 = 0x80;
pub const START_BOOST_RESERVES: i32 = 960;
/// `VehStuckProc.c:26-30`: the kart's squash while revving, against its model scale 3276.
const SQUISH_SHIFT: i32 = 6;
const SQUISH_MAX: i32 = 0x400;
const MODEL_BASE_SCALE: f32 = 3276.0;

/// `KartStates.RevEngine.chargeState` (`namespace_Vehicle.h:73-75`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Charge {
    Idle,
    ReleasedAboveAccel,
    Active,
}

/// `lockoutFlags` (`namespace_Vehicle.h:81-83`).
const LOCKOUT_PEDAL_HELD: u8 = 0x1;
const LOCKOUT_REV_DECAY: u8 = 0x2;
const LOCKOUT_ALL: u8 = LOCKOUT_PEDAL_HELD | LOCKOUT_REV_DECAY;

/// `revEngineState` (`VehStuckProc.c:1209-1230`), read by the exhaust (`VehEmitter.c:205-213`,
/// `:1620`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RevState {
    /// Below the acceleration speed: the exhaust puffs as usual.
    Low,
    /// At or above it: the exhaust puffs dark, CTR's "boost now".
    Ready,
    /// Cooling down (the revs just died, or locked out after an over-rev): no exhaust.
    Busy,
}

/// The start boost a release at GO earns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartBoost {
    Small,
    Big,
}

/// `KartStates.RevEngine`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RevEngine {
    accel: i32,
    fire_level: i32,
    boost_meter: i32,
    over_rev_ms: i32,
    release_cooldown_ms: i32,
    empty_cooldown_ms: i32,
    charge: Charge,
    lockout: u8,
    /// The gas this frame (`fireSpeed > 0`).
    gas: bool,
}

/// `VehCalc_InterpBySpeed` (`VehCalc.c:11-34`): `val` toward `desired` by `speed`, no overshoot.
fn interp(val: i32, speed: i32, desired: i32) -> i32 {
    if val > desired {
        (val - speed).max(desired)
    } else {
        (val + speed).min(desired)
    }
}

impl RevEngine {
    /// `VehStuckProc_RevEngine_Init` (`VehStuckProc.c:1296-1340`) for a class accelerating to
    /// `accel`: no revs, nothing locked. The boost meter starts where the first idle frame puts it
    /// (`VehStuckProc.c:1187`).
    pub fn new(accel: i32) -> Self {
        Self {
            accel,
            fire_level: 0,
            boost_meter: accel + SACRED_FIRE_SPEED / 3,
            over_rev_ms: 0,
            release_cooldown_ms: 0,
            empty_cooldown_ms: 0,
            charge: Charge::Idle,
            lockout: 0,
            gas: false,
        }
    }

    /// One 30 Hz frame of `ms` (CTR-ms) with the gas held or not: `_PhysLinear`'s cooldowns, then
    /// `_Animate`'s revs (`VehStuckProc.c:994-1013`, `:1036-1207`). True when the engine
    /// over-revs this frame (OtherFX 0xF, `VehStuckProc.c:1098`).
    pub fn tick(&mut self, gas: bool, ms: i32) -> bool {
        self.gas = gas;
        self.release_cooldown_ms = (self.release_cooldown_ms - ms).max(0);
        self.empty_cooldown_ms = (self.empty_cooldown_ms - ms).max(0);
        if gas && self.release_cooldown_ms == 0 && self.lockout & LOCKOUT_ALL == 0 {
            let step =
                ((self.fire_level - self.boost_meter).abs() >> 1).clamp(STEP_MIN, FILL_STEP_MAX);
            self.fire_level = interp(self.fire_level, step, self.boost_meter);
            self.charge = Charge::Active;
            if self.boost_meter <= self.fire_level {
                self.over_rev_ms += ms;
                if self.over_rev_ms > OVERREV_TIMEOUT_MS {
                    self.charge = Charge::Idle;
                    self.lockout |= LOCKOUT_ALL;
                    return true;
                }
            } else {
                self.over_rev_ms = 0;
            }
            return false;
        }
        self.over_rev_ms = 0;
        if self.charge == Charge::Active {
            self.release_cooldown_ms = RELEASE_COOLDOWN_MS;
            self.charge = if self.accel < self.fire_level {
                Charge::ReleasedAboveAccel
            } else {
                Charge::Idle
            };
        }
        // Let go above the acceleration speed and fallen back under it: the target steps up.
        if self.charge != Charge::Idle && self.fire_level < self.accel {
            self.charge = Charge::Idle;
            let top = self.accel + SACRED_FIRE_SPEED;
            self.boost_meter = interp(self.boost_meter, SACRED_FIRE_SPEED / 3 + 3, top);
        }
        if self.fire_level > 0 {
            let max = if self.lockout & LOCKOUT_REV_DECAY != 0 {
                DECAY_STEP_MAX_LOCKED
            } else {
                DECAY_STEP_MAX_NORMAL
            };
            let decay = (self.fire_level >> 1).min(max).max(STEP_MIN);
            self.fire_level -= decay;
            if self.fire_level < 1 {
                self.empty_cooldown_ms = EMPTY_COOLDOWN_MS;
                self.fire_level = 0;
            }
        } else {
            self.lockout &= !LOCKOUT_REV_DECAY;
            self.boost_meter = self.accel + SACRED_FIRE_SPEED / 3;
        }
        if !gas {
            self.lockout &= !LOCKOUT_PEDAL_HELD;
        }
        false
    }

    /// The revs: the speedometer's needle while revving (`VehStuckProc.c:1234`).
    pub fn level(&self) -> i32 {
        self.fire_level
    }

    pub fn gas(&self) -> bool {
        self.gas
    }

    pub fn state(&self) -> RevState {
        if self.empty_cooldown_ms != 0 || self.lockout & LOCKOUT_REV_DECAY != 0 {
            RevState::Busy
        } else if self.fire_level >= self.accel {
            RevState::Ready
        } else {
            RevState::Low
        }
    }

    /// `turbo_MeterRoomLeft` while revving (`VehStuckProc.c:1236-1257`): the powerslide meter
    /// shows the revs, its red zone from the acceleration speed up. `max_room` and `low_room` in
    /// CTR-ms (`const_turboMaxRoom << 5`, `const_turboLowRoomWarning << 5`).
    pub fn meter_room(&self, max_room: i32, low_room: i32) -> i32 {
        let map = |v: i32, omin: i32, omax: i32, nmin: i32, nmax: i32| {
            if v <= omin {
                nmin
            } else if v >= omax {
                nmax
            } else {
                nmin + ((i64::from(v - omin) * i64::from(nmax - nmin)) / i64::from(omax - omin))
                    as i32
            }
        };
        if self.fire_level < self.accel {
            map(self.fire_level, 0, self.accel, max_room, low_room + 1)
        } else {
            map(
                self.fire_level,
                self.accel,
                self.accel + SACRED_FIRE_SPEED,
                low_room,
                1,
            )
        }
    }

    /// The model's scale while revving (`VehStuckProc.c:1259-1293`): squashed down and widened by
    /// the revs, `(x, y, z)` against the model's own.
    pub fn scale(&self) -> (f32, f32, f32) {
        let squish = (self.fire_level >> SQUISH_SHIFT).clamp(0, SQUISH_MAX);
        let wide = (MODEL_BASE_SCALE + (squish * 6 / 10) as f32) / MODEL_BASE_SCALE;
        let tall = (MODEL_BASE_SCALE - squish as f32) / MODEL_BASE_SCALE;
        (wide, tall, wide)
    }

    /// GO (`VehStuckProc_RevEngine_Update`, `VehStuckProc.c:968-983`): revved above the
    /// acceleration speed and not locked out, a boost, big with the target at its top.
    pub fn release(&self) -> Option<StartBoost> {
        if self.accel >= self.fire_level || self.lockout & LOCKOUT_ALL != 0 {
            return None;
        }
        Some(if self.accel + SACRED_FIRE_SPEED <= self.boost_meter {
            StartBoost::Big
        } else {
            StartBoost::Small
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kart::MS;

    const ACCEL: i32 = 9000;

    fn run(rev: &mut RevEngine, gas: bool, frames: usize) -> bool {
        let mut over = false;
        for _ in 0..frames {
            over |= rev.tick(gas, MS);
        }
        over
    }

    #[test]
    fn the_gas_fills_the_revs_at_half_the_distance_a_frame() {
        let mut rev = RevEngine::new(ACCEL);
        let target = ACCEL + SACRED_FIRE_SPEED / 3;
        rev.tick(true, MS);
        assert_eq!(rev.level(), FILL_STEP_MAX);
        rev.tick(true, MS);
        assert_eq!(rev.level(), FILL_STEP_MAX + (target - FILL_STEP_MAX) / 2);
        // In the red once past the acceleration speed: the exhaust's "boost now".
        run(&mut rev, true, 2);
        assert_eq!(rev.state(), RevState::Ready);
        assert!(rev.meter_room(960, 480) <= 480);
        // Let go: it falls by half (at most 1000) a frame, at least 0x100.
        let before = rev.level();
        rev.tick(false, MS);
        assert_eq!(rev.level(), before - 1000);
    }

    #[test]
    fn held_at_the_target_the_engine_over_revs_and_locks_out() {
        let mut rev = RevEngine::new(ACCEL);
        // Seven frames to the target, then over 192 ms at it.
        let mut frames = 0;
        while !rev.tick(true, MS) {
            frames += 1;
            assert!(frames < 30);
        }
        assert!((10..=16).contains(&frames), "{frames}");
        assert_eq!(rev.release(), None);
        assert_eq!(rev.state(), RevState::Busy);
        // Locked while the gas stays down; the revs die and it comes back once let go.
        run(&mut rev, true, 10);
        assert_eq!(rev.level(), 0);
        assert_eq!(rev.state(), RevState::Busy);
        run(&mut rev, false, 1);
        run(&mut rev, true, 3);
        assert!(rev.level() > 0);
    }

    #[test]
    fn revved_at_go_a_small_boost_and_pumped_first_a_big_one() {
        // Not revved, or revved under the acceleration speed: nothing.
        let mut rev = RevEngine::new(ACCEL);
        assert_eq!(rev.release(), None);
        rev.tick(true, MS);
        assert_eq!(rev.release(), None);
        // Held into the red, short of an over-rev: small.
        let mut rev = RevEngine::new(ACCEL);
        run(&mut rev, true, 6);
        assert_eq!(rev.release(), Some(StartBoost::Small));
        // Two pumps (into the red, let go, back under) raise the target to its top: big.
        let mut rev = RevEngine::new(ACCEL);
        for _ in 0..2 {
            run(&mut rev, true, 6);
            // Let go: back under the acceleration speed, then the cooldown before the gas bites.
            run(&mut rev, false, 8);
            assert!(rev.level() > 0, "the revs died: the target would reset");
        }
        assert_eq!(rev.boost_meter, ACCEL + SACRED_FIRE_SPEED);
        run(&mut rev, true, 8);
        assert_eq!(rev.release(), Some(StartBoost::Big));
        // Let the revs die and the target falls back.
        run(&mut rev, false, 30);
        assert_eq!(rev.boost_meter, ACCEL + SACRED_FIRE_SPEED / 3);
    }

    #[test]
    fn the_kart_squashes_with_the_revs() {
        let mut rev = RevEngine::new(ACCEL);
        assert_eq!(rev.scale(), (1.0, 1.0, 1.0));
        run(&mut rev, true, 5);
        let (x, y, z) = rev.scale();
        assert!(x > 1.0 && y < 1.0 && x == z);
    }
}
