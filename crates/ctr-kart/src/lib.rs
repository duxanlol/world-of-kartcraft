//! Crash Team Racing's kart driving model, ported from the decompilation of the NTSC-U game
//! (CTR-tools/ctr-native, GPL-3.0; see `docs/ctr-physics-spec.md`), driven over any triangle soup
//! instead of CTR's quadblock tracks. Units are CTR's own: level units (lu) for length, 1/256 lu
//! per frame for speed, 4096ths of a turn for angles and 30 frames a second.

pub mod collision;
pub mod grab;
pub mod kart;
pub mod rev;
pub mod stats;
pub mod terrain;

pub use collision::Collision;
pub use grab::GrabMask;
pub use kart::{
    clock_slowdown, CameraInput, CloudEffect, Event, HitKind, Input, Kart, Modifiers, OtherKart,
    Pose, Ram, Shield, State, Terrain, MS, RAM_TURBO_IMPACT, TICK,
};
pub use rev::{RevEngine, RevState, StartBoost};
pub use stats::Class;
pub use terrain::Ground;
