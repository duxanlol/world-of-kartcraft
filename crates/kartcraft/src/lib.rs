//! World of Kartcraft's kart over the benilla world, shared by the client and the offline demo:
//! the driving [`session`] (CTR's driving model from `ctr-kart` over the streamed world's
//! collision), the [`input`] it reads, the kart's [`model`], converted from the player's own
//! CTR disc into `kart-data/kart.json`, and CTR's [`weapons`].

pub mod camera;
pub mod chain;
pub mod effects;
pub mod hud;
pub mod input;
pub mod model;
pub mod session;
pub mod sound;
pub mod weapons;

use bevy::prelude::*;
pub use ctr_kart::{Class, Event, Input, Kart, Modifiers, State};
pub use session::{KartView, NearbyKart, Session};

pub const METERS_PER_YARD: f32 = 0.9144;
/// Meters per CTR level unit at scale 1 (`docs/ctr-physics-spec.md` section 0); with it CTR's
/// kart model (in meters, from `kart.json`) fits CTR's collision sphere.
const METERS_PER_LU: f32 = 0.025;

/// How much bigger than CTR's own the kart is: `WOW_KART_SCALE`, default 1. The model and the
/// physics scale together, so a bigger kart is also a faster one, as it would be.
pub fn kart_scale() -> f32 {
    static SCALE: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *SCALE.get_or_init(|| {
        std::env::var("WOW_KART_SCALE")
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|v: &f32| *v > 0.0)
            .unwrap_or(1.0)
    })
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
