//! Crash Team Racing's weapons, ported from the decompilation of the NTSC-U game
//! (CTR-tools/ctr-native, GPL-3.0; see `docs/ctr-weapons-spec.md`): the items and their ids, the
//! roulette's weighted tables, the held-item state machine, and the projectiles, simulated over
//! any triangle soup ([`ctr_kart::Collision`]) instead of CTR's quadblock tracks. Units are CTR's
//! own: whole level units (lu) and lu per frame for weapons, 4096ths of a turn for angles, and 30
//! frames a second. Every function here is pure and deterministic.

pub mod boxes;
pub mod cloud;
pub mod held;
pub mod item;
pub mod projectile;
pub mod roulette;

pub use boxes::CrateBox;
pub use cloud::{CloudEffect, RainCloud};
pub use held::{Fire, Held, HeldView, Shot};
pub use item::Item;
pub use projectile::{
    Body, Bomb, Field, HitKind, KartId, KartView, Mine, MineKind, Missile, Outcome, ShotKey,
    WarpOrb,
};
pub use roulette::{roll, RaceItems, Rng, Table};
