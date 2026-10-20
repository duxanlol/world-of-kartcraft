//! World of Kartcraft's kart over the benilla world, shared by the client and the offline demo:
//! the driving [`session`] (CTR's driving model from `ctr-kart` over the streamed world's
//! collision), the [`input`] it reads, the kart's [`model`], converted from the player's own
//! CTR disc into `kart-data/kart.json`, and CTR's [`weapons`].

pub mod camera;
pub mod chain;
pub mod cosmetic;
pub mod effects;
pub mod goods;
pub mod hud;
pub mod input;
pub mod model;
pub mod pads;
pub mod rumble;
pub mod session;
pub mod sound;
pub mod voice;
pub mod weapons;

use bevy::prelude::*;
pub use ctr_kart::{wheelie_frame, Class, Event, Ground, Input, Kart, Modifiers, State};
pub use session::{KartView, NearbyKart, Session};

pub const METERS_PER_YARD: f32 = 0.9144;
/// Meters per CTR level unit at scale 1 (`docs/ctr-physics-spec.md` section 0); with it CTR's
/// kart model (in meters, from `kart.json`) fits CTR's collision sphere.
const METERS_PER_LU: f32 = 0.025;

/// How much bigger than CTR's own the kart is: `WOW_KART_SCALE` (default 1) times the race's scale
/// as the session took it on ([`race_scale`]). The model and the physics scale together, so a
/// smaller kart is also a slower one in the world's yards, and the world drives bigger round it.
pub fn kart_scale() -> f32 {
    static SCALE: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    let env = *SCALE.get_or_init(|| {
        std::env::var("WOW_KART_SCALE")
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|v: &f32| *v > 0.0)
            .unwrap_or(1.0)
    });
    env * race_scale()
}

/// The race's scale the host wants (a course's `scale`, 1 out of a race), and the one the session
/// has taken on: the session's world is gathered at its scale, so a new one waits for the
/// session to start again ([`latch_scale`]), as at a race's grid.
static WANTED_SCALE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3f80_0000);
static RACE_SCALE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3f80_0000);

/// The smallest and largest race scale: below, a kart is too small to climb a kerb; above, too
/// big for a street.
pub const MIN_RACE_SCALE: f32 = 0.2;
pub const MAX_RACE_SCALE: f32 = 3.0;

/// Asks for the race's scale `scale` (1 the kart's own size), clamped to the bounds; the session
/// takes it on as it starts again ([`scale_pending`]).
pub fn set_race_scale(scale: f32) {
    let s = if scale.is_finite() {
        scale.clamp(MIN_RACE_SCALE, MAX_RACE_SCALE)
    } else {
        1.0
    };
    WANTED_SCALE.store(s.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

/// The race's scale the session drives at.
pub fn race_scale() -> f32 {
    f32::from_bits(RACE_SCALE.load(std::sync::atomic::Ordering::Relaxed))
}

/// Whether a scale other than the session's is asked for: the host starts the session again
/// where the kart stands.
pub fn scale_pending() -> bool {
    WANTED_SCALE.load(std::sync::atomic::Ordering::Relaxed)
        != RACE_SCALE.load(std::sync::atomic::Ordering::Relaxed)
}

/// The scale asked for becomes the session's: called as the session starts, before its world
/// is gathered; returns the one it had, for [`restore_scale`] where the start comes to nothing.
pub(crate) fn latch_scale() -> u32 {
    let wanted = WANTED_SCALE.load(std::sync::atomic::Ordering::Relaxed);
    RACE_SCALE.swap(wanted, std::sync::atomic::Ordering::Relaxed)
}

/// The session's scale back to `was` ([`latch_scale`]'s).
pub(crate) fn restore_scale(was: u32) {
    RACE_SCALE.store(was, std::sync::atomic::Ordering::Relaxed);
}

/// Level units per yard.
pub fn lu_per_yard() -> f32 {
    METERS_PER_YARD / (METERS_PER_LU * kart_scale())
}

/// The model's meters to yards, at the kart's scale.
pub fn model_scale() -> f32 {
    kart_scale() * model_size() / METERS_PER_YARD
}

/// How the host frames the kart: how much bigger than its physics the model is drawn (a WoW
/// character is taller than the CTR racer the kart was built for), how much of the window's
/// bottom (a fraction) the HUD keeps clear of, and how far above CTR's racer's head (meters, the
/// model's frame) the driver's head is, where a TNT sits. The chase camera's own shape is the
/// session's [`camera::CameraTuning`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Framing {
    pub model_size: f32,
    pub hud_bottom_inset: f32,
    pub head_lift: f32,
}

impl Framing {
    /// CTR's own: the demo.
    pub const CTR: Framing = Framing {
        model_size: 1.0,
        hud_bottom_inset: 0.0,
        head_lift: 0.0,
    };
}

static FRAMING: std::sync::OnceLock<Framing> = std::sync::OnceLock::new();

/// Sets the host's [`Framing`] once, before anything is drawn.
pub fn set_framing(framing: Framing) {
    let _ = FRAMING.set(framing);
}

pub fn framing() -> Framing {
    *FRAMING.get_or_init(|| Framing::CTR)
}

/// The model's size against its physics: `WOW_KART_SIZE`, else the host's.
fn model_size() -> f32 {
    static SIZE: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *SIZE.get_or_init(|| {
        std::env::var("WOW_KART_SIZE")
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|v: &f32| *v > 0.0)
            .unwrap_or_else(|| framing().model_size)
    })
}

/// Our world (Bevy yards) to the kart's: lu from the session's `origin`, same axes (both are
/// right-handed, +Y up), which keeps the floats small anywhere on a continent.
pub fn to_lu(p: Vec3, origin: Vec3) -> Vec3 {
    (p - origin) * lu_per_yard()
}

/// Inverse of [`to_lu`].
pub fn from_lu(p: Vec3, origin: Vec3) -> Vec3 {
    p / lu_per_yard() + origin
}

/// CTR's heading (forward `(sin h, 0, cos h)`, 4096ths) to Bevy's yaw (facing
/// `(−sin θ, 0, −cos θ)`).
pub fn yaw_of(heading: i32) -> f32 {
    heading as f32 * std::f32::consts::TAU / 4096.0 + std::f32::consts::PI
}

/// Inverse of [`yaw_of`].
pub fn heading_of(yaw: f32) -> i32 {
    ((yaw - std::f32::consts::PI) * 4096.0 / std::f32::consts::TAU).round() as i32 & 0xFFF
}

/// The engine class: `WOW_KART_CLASS` (balanced, accel, speed or turn), default Balanced.
pub fn class() -> Class {
    std::env::var("WOW_KART_CLASS")
        .ok()
        .and_then(|v| class_named(&v))
        .unwrap_or(Class::Balanced)
}

/// A class by its key ([`class_key`]) or its name, any case.
pub fn class_named(key: &str) -> Option<Class> {
    match key.trim().to_ascii_lowercase().as_str() {
        "balanced" => Some(Class::Balanced),
        "accel" | "acceleration" => Some(Class::Accel),
        "speed" => Some(Class::Speed),
        "turn" | "turning" => Some(Class::Turn),
        _ => None,
    }
}

/// A class's key, as `WOW_KART_CLASS` spells it.
pub fn class_key(class: Class) -> &'static str {
    match class {
        Class::Balanced => "balanced",
        Class::Accel => "accel",
        Class::Speed => "speed",
        Class::Turn => "turn",
    }
}

/// The controls as the driving model reads them.
pub fn kart_input(c: &input::Controls) -> Input {
    Input {
        steer: c.steer,
        gas: c.accelerate,
        brake: c.brake,
        down: c.reverse,
        r1: c.hop,
        l1: c.hop_alt,
    }
}

/// Meters per CTR world unit in the models the converter writes (`docs/ctr-assets.md`,
/// "Coordinates and units": a model unit is 1/64 m and racers are drawn at 0.8, so 1/51.2 m), for
/// placing things against the drawn kart (a mask's orbit, a TNT on the head).
pub const METERS_PER_UNIT: f32 = 1.0 / 51.2;

#[cfg(test)]
mod tests {
    use super::*;

    /// A race's scale asked for waits for the session to start to take it on, bounded. (The
    /// session's own scale is left alone here: every other test's lengths are measured by it.)
    #[test]
    fn a_race_s_scale_waits_for_the_session_and_is_bounded() {
        set_race_scale(0.5);
        assert!(scale_pending());
        assert_eq!(race_scale(), 1.0, "not before the session starts");
        set_race_scale(f32::NAN);
        assert_eq!(
            f32::from_bits(WANTED_SCALE.load(std::sync::atomic::Ordering::Relaxed)),
            1.0
        );
        set_race_scale(0.01);
        assert_eq!(
            f32::from_bits(WANTED_SCALE.load(std::sync::atomic::Ordering::Relaxed)),
            MIN_RACE_SCALE
        );
        set_race_scale(1.0);
    }

    #[test]
    fn headings_and_yaws_agree_on_forward() {
        for heading in [0, 512, 1024, 2048, 3000] {
            let yaw = yaw_of(heading);
            let h = heading as f32 * std::f32::consts::TAU / 4096.0;
            let ctr = Vec3::new(h.sin(), 0.0, h.cos());
            let bevy = Vec3::new(-yaw.sin(), 0.0, -yaw.cos());
            assert!(
                (ctr - bevy).length() < 1e-4,
                "{heading}: {ctr:?} vs {bevy:?}"
            );
            assert_eq!(heading_of(yaw), heading);
        }
    }

    #[test]
    fn every_class_s_key_and_name_name_it() {
        for class in Class::ALL {
            assert_eq!(class_named(class_key(class)), Some(class));
            assert_eq!(class_named(class.name()), Some(class));
        }
        assert_eq!(class_named("hover"), None);
    }

    #[test]
    fn lu_round_trips() {
        let origin = Vec3::new(-8900.0, 50.0, 300.0);
        let p = Vec3::new(-8910.0, 52.0, 320.0);
        assert!((from_lu(to_lu(p, origin), origin) - p).length() < 1e-3);
    }
}
