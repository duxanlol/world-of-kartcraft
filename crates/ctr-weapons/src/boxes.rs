//! Weapons breaking crates. A crate never looks for weapons: a weapon's own level search finds
//! the crate's hitbox and calls its `LInC` → `ThCollide` (`231/RB_Crate.c:107-271`), which
//! breaks a full-grown crate whatever touched it and starts the same 30-frame cooldown as a
//! driver's touch (`:123-131`).
//!
//! - A bomb, missile or shield bomb breaks a crate by touching it: a vertical probe 64 lu below
//!   to 256 above it, against the crate's box (`RB_MovingExplosive.c:220-239`), and by its burst,
//!   the weapon within the blast radius of the box's centre (`RB_Burst.c:286-317`,
//!   `PROC.c:419-454`). The crate does not stop it (`RB_Hazard.c:265-272`).
//! - A warp orb breaks one by touching it, its probe 128 lu either side (`RB_Warpball.c:607-640`),
//!   and flies on.
//! - A TNT, Nitro or beaker breaks the crate it is dropped on: one probe 400 lu below to 64
//!   above, at the drop (`VehPickupItem.c:1199-1251`, `:1395-1447`). A TNT or Nitro going off
//!   breaks none: `RB_Blowup_Init` searches the level with the thread callback
//!   (`RB_Blowup.c:159-180`).
//! - The shooter of a bomb, missile or shield bomb gets what the crate gives, as if it had
//!   driven into it (`RB_Crate.c:152-226`, `:315-355`); a warp orb or a mine gives no one
//!   anything.

use glam::Vec3;

use crate::projectile::{Body, MINE_PROBE_DOWN, MINE_PROBE_UP};
use crate::Item;

/// A moving explosive's level probe: 64 lu below it to 256 above (`RB_MovingExplosive.c:220-226`).
pub const PROBE_BELOW: f32 = 64.0;
pub const PROBE_ABOVE: f32 = 256.0;
/// The warp orb's: 128 lu either side (`RB_Warpball.c:607-612`).
pub const WARP_PROBE: f32 = 128.0;

/// A crate's hitbox: a cube of half-size `half` lu standing on `ground`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CrateBox {
    pub ground: Vec3,
    pub half: f32,
}

impl CrateBox {
    pub fn centre(&self) -> Vec3 {
        self.ground + Vec3::Y * self.half
    }

    /// A vertical probe at `at`, from `below` lu under it to `above` over it, meets the box
    /// (`COLL.c:336-345`, `:574-587`: with `FORCE_INSTANCE_HIT` any overlap counts).
    fn probed(&self, at: Vec3, below: f32, above: f32) -> bool {
        let d = at - self.ground;
        d.x.abs() < self.half
            && d.z.abs() < self.half
            && d.y - below < 2.0 * self.half
            && d.y + above > 0.0
    }
}

/// Whether a live weapon touches the crate this frame: bombs, shield bombs, missiles and warp
/// orbs do; a mine meets crates only where it is dropped ([`dropped_on`]).
pub fn touches(body: &Body, b: &CrateBox) -> bool {
    match body {
        Body::Bomb(_) | Body::Missile(_) => b.probed(body.position(), PROBE_BELOW, PROBE_ABOVE),
        Body::WarpOrb(_) => b.probed(body.position(), WARP_PROBE, WARP_PROBE),
        Body::Mine(_) => false,
    }
}

/// Whether a TNT, Nitro or beaker dropped at `at` lands on the crate.
pub fn dropped_on(at: Vec3, b: &CrateBox) -> bool {
    b.probed(at, MINE_PROBE_DOWN, MINE_PROBE_UP)
}

/// Whether a bomb's, shield bomb's or missile's burst at `at`, `radius` lu (its blast to
/// players), reaches the crate: the box's centre inside it.
pub fn blast_reaches(at: Vec3, radius: f32, b: &CrateBox) -> bool {
    (b.centre() - at).length_squared() < radius * radius
}

/// Whether a crate broken by a shot of this item gives its shooter the item or fruit: bombs,
/// missiles and shield bombs (`DYNAMIC_BOMB`, `DYNAMIC_ROCKET`, `DYNAMIC_SHIELD`), as their
/// single or ×3 ids; a warp orb, TNT, Nitro or beaker gives nothing.
pub fn rewards(item: Item) -> bool {
    matches!(item.single(), Item::Bomb | Item::Missile | Item::Shield)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projectile::{BLAST_RADIUS, BLAST_RADIUS_JUICED, MISSILE_BLAST_RADIUS};
    use crate::{Bomb, Mine, MineKind, Missile, WarpOrb};
    use ctr_kart::Collision;

    const HALF: f32 = 48.0;

    fn crate_at(x: f32) -> CrateBox {
        CrateBox {
            ground: Vec3::new(x, 0.0, 0.0),
            half: HALF,
        }
    }

    fn flat() -> Collision {
        let r = 40_000.0;
        Collision::new(
            [
                [[-r, 0.0, -r], [-r, 0.0, r], [r, 0.0, r]],
                [[-r, 0.0, -r], [r, 0.0, r], [r, 0.0, -r]],
            ],
            4096.0,
        )
    }

    #[test]
    fn bombs_missiles_shields_and_orbs_break_what_they_touch_and_mines_only_where_dropped() {
        let at = Vec3::new(0.0, 48.0, 0.0);
        let world = flat();
        let bodies = [
            Body::Bomb(Bomb::new(1, at, 0, false, false, None)),
            Body::Bomb(Bomb::shield(1, at, 0)),
            Body::Missile(Missile::new(1, at, 0, false, None)),
            Body::WarpOrb(WarpOrb::new(1, at, 0, false, None)),
        ];
        for body in &bodies {
            assert!(touches(body, &crate_at(40.0)), "{body:?}");
            assert!(!touches(body, &crate_at(50.0)), "{body:?}: beside it");
        }
        let mine = Body::Mine(Mine::drop(1, MineKind::Tnt, at, &world));
        assert!(!touches(&mine, &crate_at(0.0)), "a mine lying there");
        assert!(dropped_on(at, &crate_at(40.0)));
        assert!(!dropped_on(at, &crate_at(50.0)));
        // A bomb's probe reaches 64 lu down: a crate on a ledge below (its top 70 lu under the
        // bomb) is missed, but the orb's reaches 128.
        let low = CrateBox {
            ground: Vec3::new(0.0, -118.0, 0.0),
            half: HALF,
        };
        assert!(!touches(&bodies[0], &low));
        assert!(touches(&bodies[3], &low), "the orb probes 128 lu down");
    }

    #[test]
    fn a_burst_reaches_crates_within_its_blast_radius() {
        let at = Vec3::new(0.0, HALF, 0.0);
        for radius in [BLAST_RADIUS, BLAST_RADIUS_JUICED, MISSILE_BLAST_RADIUS] {
            assert!(
                blast_reaches(at, radius, &crate_at(radius - 1.0)),
                "{radius}"
            );
            assert!(
                !blast_reaches(at, radius, &crate_at(radius + 1.0)),
                "{radius}"
            );
        }
        assert_eq!((BLAST_RADIUS, BLAST_RADIUS_JUICED), (320.0, 512.0));
        assert_eq!(MISSILE_BLAST_RADIUS, 128.0);
    }

    #[test]
    fn only_bombs_missiles_and_shields_reward_their_shooter() {
        for item in [
            Item::Bomb,
            Item::Bomb3,
            Item::Missile,
            Item::Missile3,
            Item::Shield,
        ] {
            assert!(rewards(item), "{item:?}");
        }
        for item in [Item::WarpOrb, Item::Tnt, Item::Beaker, Item::Turbo] {
            assert!(!rewards(item), "{item:?}");
        }
    }
}
