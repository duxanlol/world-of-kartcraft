//! Crash Team Racing's kart driving model, ported from the decompilation of the NTSC-U game
//! (CTR-tools/ctr-native, GPL-3.0; see `docs/ctr-physics-spec.md`), driven over any triangle soup
//! instead of CTR's quadblock tracks. Units are CTR's own: level units (lu) for length, 1/256 lu
//! per frame for speed, 4096ths of a turn for angles and 30 frames a second.

pub mod collision;
pub mod kart;
pub mod stats;

pub use collision::Collision;
pub use kart::{CameraInput, Event, Input, Kart, Modifiers, OtherKart, Pose, State, MS, TICK};
pub use stats::Class;
