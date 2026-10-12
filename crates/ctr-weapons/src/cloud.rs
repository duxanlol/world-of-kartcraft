//! The red beaker's rain cloud as whoever owns the held items rules it (`RB_RainCloud_Init` and
//! `RB_RainCloud_ThTick`, `231/RB_MaskShieldCloud.c:496-641`): 7.68 s over the kart that a red
//! beaker's spin-out took on (`231/RB_GenericMine.c:172-176`). A first cloud locks a held item
//! into the roulette until it goes and then rerolls it, or with nothing held slows the kart; a
//! second beaker under the cloud restarts the time and draws the effect anew. The effects on the
//! kart itself are `ctr-kart`'s ([`CloudEffect`]).

pub use ctr_kart::CloudEffect;

use crate::roulette::Rng;

/// `rcloud->timeMS = 0x1e00` (`RB_MaskShieldCloud.c:615`, `:633`): 7680 ms.
pub const CLOUD_MS: u32 = 0x1E00;
/// The same, in 30 Hz frames.
pub const CLOUD_FRAMES: u32 = CLOUD_MS / 32;

/// A cloud over one kart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RainCloud {
    pub effect: CloudEffect,
    /// The held item went into the roulette under it, to be rerolled when it goes.
    pub locked: bool,
}

impl RainCloud {
    /// A red beaker's spin-out took on a kart under `cloud` (or none), which `holds` an item with
    /// a shot left (`heldItemID != NONE && noItemTimer == 0`). A first cloud spins that item
    /// (`ITEM_ROLL`), or slows a kart holding nothing (`:617-622`); under a cloud the effect is
    /// drawn again, `(MixRNG % 400) / 100` (`:635-638`), and an `ITEM_ROLL` takes a held item into
    /// the roulette as its tick would (`:543-551`). A roulette locked before stays locked.
    pub fn rain(cloud: Option<RainCloud>, holds: bool, rng: &mut Rng) -> RainCloud {
        let effect = match cloud {
            None if holds => CloudEffect::ItemRoll,
            None => CloudEffect::Slow,
            Some(_) => {
                let draw = (rng.scramble() % 400) / 100;
                CloudEffect::from_id(draw as u8).unwrap_or(CloudEffect::Slow)
            }
        };
        let was_locked = cloud.is_some_and(|c| c.locked);
        RainCloud {
            effect,
            locked: was_locked || (effect == CloudEffect::ItemRoll && holds),
        }
    }

    /// Whether the roulette locked under this cloud spins out now: when the cloud goes, or when a
    /// redraw leaves `ITEM_ROLL` and the roll timer, no longer held at 5, runs down
    /// (`RB_MaskShieldCloud.c:550`, `:556-561`; `VehPhysProc.c:579-607`). Then the item is rolled.
    pub fn rerolls(&self, ended: bool) -> bool {
        self.locked && (ended || self.effect != CloudEffect::ItemRoll)
    }

    /// Whether a weapon crate gives nothing under it (`231/RB_Crate.c:185-191`).
    pub fn blocks_crates(&self) -> bool {
        self.effect == CloudEffect::ItemRoll
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_first_cloud_spins_a_held_item_or_slows_an_empty_kart() {
        let mut rng = Rng::default();
        let before = rng;
        let c = RainCloud::rain(None, true, &mut rng);
        assert_eq!(
            c,
            RainCloud {
                effect: CloudEffect::ItemRoll,
                locked: true
            }
        );
        assert_eq!(rng, before, "a first cloud draws no number");
        assert!(c.blocks_crates());
        assert!(!c.rerolls(false));
        assert!(c.rerolls(true));
        let c = RainCloud::rain(None, false, &mut rng);
        assert_eq!(
            c,
            RainCloud {
                effect: CloudEffect::Slow,
                locked: false
            }
        );
        assert!(!c.blocks_crates());
        assert!(!c.rerolls(true), "nothing to reroll");
        assert_eq!(CLOUD_FRAMES, 240);
    }

    #[test]
    fn a_second_beaker_draws_the_effect_from_mixrng() {
        // Every effect comes up, each as `(s % 400) / 100` of the next state says.
        let mut seen = [false; 4];
        let mut rng = Rng::new(1234);
        for _ in 0..64 {
            let mut peek = rng;
            let want = (peek.scramble() % 400) / 100;
            let c = RainCloud::rain(
                Some(RainCloud {
                    effect: CloudEffect::Slow,
                    locked: false,
                }),
                false,
                &mut rng,
            );
            assert_eq!(u16::from(c.effect.id()), want);
            assert!(!c.locked, "nothing held: nothing to lock");
            seen[usize::from(c.effect.id())] = true;
        }
        assert_eq!(seen, [true; 4]);
    }

    #[test]
    fn a_locked_roulette_rolls_when_the_redraw_leaves_item_roll() {
        let locked = RainCloud {
            effect: CloudEffect::ItemRoll,
            locked: true,
        };
        let mut rng = Rng::new(7);
        let next = loop {
            let c = RainCloud::rain(Some(locked), false, &mut rng);
            if c.effect != CloudEffect::ItemRoll {
                break c;
            }
        };
        assert!(next.locked);
        assert!(next.rerolls(false), "the roll timer runs down at once");
        // A redraw that lands on `ITEM_ROLL` takes an item held under a slow cloud.
        let mut rng = Rng::new(7);
        let slow = RainCloud {
            effect: CloudEffect::Slow,
            locked: false,
        };
        let rolled = loop {
            let c = RainCloud::rain(Some(slow), true, &mut rng);
            if c.effect == CloudEffect::ItemRoll {
                break c;
            }
        };
        assert!(rolled.locked && !rolled.rerolls(false) && rolled.rerolls(true));
    }
}
