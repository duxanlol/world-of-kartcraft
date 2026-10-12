//! A bot that test-drives a kart course: it drives a [`Route`] over a [`Collision`] with CTR's
//! driving model (`ctr-kart`) the way a decent player would (pure-pursuit steering, braking for
//! corners, powerslides with boosts in long bends, backing out when stuck) and reports how the
//! course drove. Units are the kart's: lu for length, seconds, lu/s. The same driver authors the
//! [`showcase`] runs' input tapes.

mod course;
mod driver;
mod route;
pub mod showcase;

pub use course::{Course, CELL};
pub use ctr_kart::{Class, Collision};
pub use driver::{drive, DriveOpts, DriveReport, SlideTuning, TracePoint, LU_PER_YARD};
pub use route::{Gate, Route};
