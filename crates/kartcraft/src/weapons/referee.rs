//! The offline referee: the Kartcraft server's weapon rules (`setup/vmangos/src/Kartcraft.cpp`,
//! `HandleWeapon`; `docs/kartcraft-weapons-protocol.md`) for a race with no server, so the demo
//! drives the same [`super::Weapons`] the client does. It owns the crates and their respawns,
//! rolls the roulette ([`ctr_weapons::roll`]), keeps each racer's item, shots and Wumpa, and
//! relays shots, detonations, shield bombs and hits.

use std::collections::HashMap;

use ctr_weapons::{roll, HitKind, Item, KartId, RaceItems, Rng, ShotKey};

use super::wire::{CrateKind, CrateSpot, Down, Up};

/// The server's claim reach, yards (`CRATE_RADIUS`).
const CRATE_RADIUS: f32 = 12.0;
/// A broken crate's 30-frame cooldown, 1 s (`231/RB_Crate.c:125`), then it is back.
const CRATE_RESPAWN: f32 = 1.0;
/// It regrows 16 frames, +0x100 a frame, and cannot be hit until full (`231/RB_Crate.c:48-75`,
/// `:123`).
const CRATE_GROW: f32 = 16.0 / 30.0;
/// Shots of a rolled item are taken from 20 frames after the claim (`ROLL_MIN`).
const ROLL_MIN: f32 = 20.0 / 30.0;
/// A shot is remembered 30 s; CTR keeps 12 projectiles (`MAX_LIVE_SHOTS`).
const SHOT_LIFE: f32 = 30.0;
const MAX_LIVE_SHOTS: usize = 12;
/// The mask's 8 s, juiced 12 s: no Wumpa is lost under it (`231/RB_Player.c:153-161`).
const MASK_TIME: f32 = 7680.0 / 960.0;
const MASK_TIME_JUICED: f32 = 11520.0 / 960.0;
/// A green shield lasts 12 s (`VehPickupItem.c:976-983`); a second's slack for the line.
const SHIELD_TIME: f32 = 13.0;
/// The roulette's field: 8 racers on lap 2, CTR's 1P arcade table set.
const RACERS: usize = 8;
const LAP: u32 = 1;

struct Spot {
    spot: CrateSpot,
    up: bool,
    back_at: Option<f32>,
    /// Until when it regrows after coming back.
    grown_at: f32,
}

#[derive(Default)]
struct Racer {
    held: Option<Item>,
    shots: u8,
    claimed: f32,
    wumpa: u8,
    mask_until: f32,
    /// A shield fired and not yet launched: when, and whether juiced (blue, no time limit).
    shield: Option<(f32, bool)>,
}

struct Record {
    item: Item,
    at: f32,
    hit: bool,
    dropped: bool,
}

pub struct Referee {
    rng: Rng,
    crates: Vec<Spot>,
    racers: HashMap<KartId, Racer>,
    shots: HashMap<ShotKey, Record>,
    /// The race order, leader first, for the warp orb's target.
    pub order: Vec<KartId>,
    /// `WOW_KART_WEAPON_TEST`: every roll gives this.
    pub force: Option<Item>,
}

/// A line and whom it is for: `None` every racer, else that one.
pub type Addressed = (Option<KartId>, Down);

fn is_projectile(item: Item) -> bool {
    matches!(
        item,
        Item::Bomb
            | Item::Missile
            | Item::Tnt
            | Item::Beaker
            | Item::WarpOrb
            | Item::Bomb3
            | Item::Missile3
    )
}

impl Referee {
    /// A race over these crates; the layout goes to every racer.
    pub fn new(crates: Vec<CrateSpot>, seed: u16) -> (Self, Vec<Addressed>) {
        let layout = super::wire::layout(&crates)
            .into_iter()
            .map(|d| (None, d))
            .collect();
        let referee = Self {
            rng: Rng::new(seed),
            crates: crates
                .into_iter()
                .map(|spot| Spot {
                    spot,
                    up: true,
                    back_at: None,
                    grown_at: f32::NEG_INFINITY,
                })
                .collect(),
            racers: HashMap::new(),
            shots: HashMap::new(),
            order: Vec::new(),
            force: None,
        };
        (referee, layout)
    }

    /// Whether `who` holds an item or shots of one.
    pub fn holds(&self, who: KartId) -> bool {
        self.racers
            .get(&who)
            .is_some_and(|r| r.held.is_some() || r.shots != 0)
    }

    /// Crates due back come up; old shots are forgotten (`TickWeapons`).
    pub fn tick(&mut self, now: f32) -> Vec<Addressed> {
        let mut out = Vec::new();
        for (i, c) in self.crates.iter_mut().enumerate() {
            if !c.up && c.back_at.is_some_and(|t| now >= t) {
                c.up = true;
                c.back_at = None;
                c.grown_at = now + CRATE_GROW;
                out.push((None, Down::Back { index: i }));
            }
        }
        self.shots.retain(|_, s| now - s.at < SHOT_LIFE);
        out
    }

    /// Hands `to` an item as a weapon crate would (the demo's weapon test).
    pub fn grant(&mut self, to: KartId, item: Item, now: f32) -> Vec<Addressed> {
        let r = self.racers.entry(to).or_default();
        r.held = Some(item);
        r.shots = item.shots();
        r.claimed = now;
        vec![(
            Some(to),
            Down::Item {
                item,
                shots: r.shots,
            },
        )]
    }

    /// A racer's line; `at` is where the sender is (WoW world, yards).
    pub fn handle(&mut self, from: KartId, up: Up, at: [f32; 3], now: f32) -> Vec<Addressed> {
        let mut out = Vec::new();
        match up {
            Up::Claim(index) => {
                let Some(c) = self.crates.get_mut(index) else {
                    return out;
                };
                let d = (c.spot.at[0] - at[0]).hypot(c.spot.at[1] - at[1]);
                if !c.up || now < c.grown_at || d > CRATE_RADIUS {
                    return out;
                }
                c.up = false;
                c.back_at = (c.spot.kind != CrateKind::Wumpa).then_some(now + CRATE_RESPAWN);
                let kind = c.spot.kind;
                out.push((None, Down::Taken { index, by: from }));
                if kind == CrateKind::Weapon {
                    // Holding an item, the crate breaks for nothing (`RB_Crate.c:152-226`).
                    if self.holds(from) {
                        return out;
                    }
                    // Anywhere in the field, for variety.
                    let position = usize::from(self.rng.scramble()) % RACERS;
                    let rolled = roll(&mut self.rng, position, RACERS, LAP, &RaceItems::new(1));
                    let item = self.force.unwrap_or(rolled);
                    out.extend(self.grant(from, item, now));
                } else {
                    // A fruit crate gives `MixRNG % 4 + 5` (`RB_Crate.c:316`), a fruit one.
                    let gain = if kind == CrateKind::Fruit {
                        self.rng.scramble() % 4 + 5
                    } else {
                        1
                    };
                    let r = self.racers.entry(from).or_default();
                    r.wumpa = (u16::from(r.wumpa) + gain)
                        .min(u16::from(ctr_weapons::item::WUMPA_MAX))
                        as u8;
                    out.push((Some(from), Down::Wumpa(r.wumpa)));
                }
            }
            Up::Fire { shot, mut aim } => {
                let live = self
                    .shots
                    .values()
                    .filter(|s| is_projectile(s.item) && !s.hit && now - s.at < SHOT_LIFE)
                    .count();
                let order = self.order.clone();
                let r = self.racers.entry(from).or_default();
                let Some(item) = r.held.filter(|_| r.shots != 0) else {
                    return out;
                };
                if now - r.claimed < ROLL_MIN || (is_projectile(item) && live >= MAX_LIVE_SHOTS) {
                    return out;
                }
                let juiced = ctr_weapons::item::juiced(r.wumpa);
                r.shots -= 1;
                if r.shots == 0 {
                    r.held = None;
                }
                if item == Item::Mask {
                    r.mask_until = now + if juiced { MASK_TIME_JUICED } else { MASK_TIME };
                }
                if item == Item::Shield {
                    r.shield = Some((now, juiced));
                }
                if item == Item::WarpOrb {
                    // The leader, or the next racer when the shooter leads.
                    aim.target = order.into_iter().find(|&k| k != from).unwrap_or(0);
                }
                self.shots.insert(
                    ShotKey { owner: from, shot },
                    Record {
                        item,
                        at: now,
                        hit: false,
                        dropped: aim.dropped,
                    },
                );
                out.push((
                    None,
                    Down::Shot(super::wire::Shot {
                        shooter: from,
                        item,
                        juiced,
                        shot,
                        aim,
                    }),
                ));
            }
            Up::Hit {
                shooter,
                shot,
                kind,
            } => {
                let own = shooter == from && shot == 0;
                if !own {
                    let Some(s) = self.shots.get_mut(&ShotKey {
                        owner: shooter,
                        shot,
                    }) else {
                        return out;
                    };
                    let single = matches!(s.item, Item::Missile | Item::Missile3 | Item::WarpOrb);
                    if single && s.hit {
                        return out;
                    }
                    if single || s.dropped {
                        s.hit = true;
                    }
                }
                let r = self.racers.entry(from).or_default();
                let loss = if now < r.mask_until {
                    0
                } else {
                    match kind {
                        HitKind::Blasted => 3,
                        HitKind::TntOnHead => 0,
                        _ => 1,
                    }
                };
                r.wumpa = r.wumpa.saturating_sub(loss);
                out.push((
                    None,
                    Down::Hit {
                        victim: from,
                        shooter,
                        shot,
                        kind,
                        wumpa: r.wumpa,
                    },
                ));
            }
            Up::Detonate => {
                // Only with a bomb of ours still rolling.
                let rolling = self.shots.iter().any(|(k, s)| {
                    k.owner == from
                        && matches!(s.item, Item::Bomb | Item::Bomb3)
                        && !s.hit
                        && now - s.at < SHOT_LIFE
                });
                if rolling {
                    out.push((None, Down::Detonate { shooter: from }));
                }
            }
            Up::ShieldShot { shot, aim } => {
                // Only with a shield of ours up; it becomes a shot that hits may name.
                let r = self.racers.entry(from).or_default();
                let Some((at, juiced)) = r.shield.take() else {
                    return out;
                };
                if !juiced && now - at > SHIELD_TIME {
                    return out;
                }
                self.shots.insert(
                    ShotKey { owner: from, shot },
                    Record {
                        item: Item::Shield,
                        at: now,
                        hit: false,
                        dropped: false,
                    },
                );
                out.push((
                    None,
                    Down::ShieldShot {
                        shooter: from,
                        shot,
                        aim,
                    },
                ));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::super::wire::Aim;
    use super::*;

    fn spot(kind: CrateKind) -> CrateSpot {
        CrateSpot {
            kind,
            at: [10.0, 20.0, 0.0],
        }
    }

    #[test]
    fn a_weapon_crate_rolls_once_and_comes_back() {
        let (mut r, layout) = Referee::new(vec![spot(CrateKind::Weapon)], 100);
        assert_eq!(layout.len(), 1);
        let out = r.handle(1, Up::Claim(0), [11.0, 20.0, 0.0], 0.0);
        assert_eq!(out[0], (None, Down::Taken { index: 0, by: 1 }));
        assert!(matches!(out[1], (Some(1), Down::Item { .. })));
        // Gone until it is back: a second claim gets nothing.
        assert!(r.handle(2, Up::Claim(0), [10.0, 20.0, 0.0], 0.5).is_empty());
        // Back after its 30-frame cooldown, and not to be hit while it regrows 16 frames.
        assert!(r.tick(0.95).is_empty());
        assert_eq!(r.tick(1.0), vec![(None, Down::Back { index: 0 })]);
        assert!(r.handle(1, Up::Claim(0), [10.0, 20.0, 0.0], 1.5).is_empty());
        // Holding an item, the crate breaks for nothing.
        let out = r.handle(1, Up::Claim(0), [10.0, 20.0, 0.0], 1.6);
        assert_eq!(out, vec![(None, Down::Taken { index: 0, by: 1 })]);
    }

    #[test]
    fn claims_out_of_reach_are_refused() {
        let (mut r, _) = Referee::new(vec![spot(CrateKind::Fruit)], 100);
        assert!(r.handle(1, Up::Claim(0), [23.0, 20.0, 0.0], 0.0).is_empty());
        assert!(r.handle(1, Up::Claim(5), [10.0, 20.0, 0.0], 0.0).is_empty());
        let out = r.handle(1, Up::Claim(0), [10.0, 20.0, 0.0], 0.0);
        let Some((Some(1), Down::Wumpa(n))) = out.get(1).cloned() else {
            panic!("{out:?}");
        };
        assert!((5..=8).contains(&n));
    }

    #[test]
    fn shots_follow_the_held_item_and_hits_cost_wumpa() {
        let (mut r, _) = Referee::new(vec![], 100);
        r.grant(1, Item::Bomb3, 0.0);
        let fire = |shot| Up::Fire {
            shot,
            aim: Aim::default(),
        };
        assert!(r.handle(1, fire(1), [0.0; 3], 0.1).is_empty(), "too soon");
        for shot in 1..=3 {
            let out = r.handle(1, fire(shot), [0.0; 3], 1.0);
            assert!(
                matches!(&out[..], [(None, Down::Shot(s))] if s.item == Item::Bomb3),
                "{out:?}"
            );
        }
        assert!(
            r.handle(1, fire(4), [0.0; 3], 1.0).is_empty(),
            "out of shots"
        );
        r.racers.entry(2).or_default().wumpa = 5;
        let hit = |shot| Up::Hit {
            shooter: 1,
            shot,
            kind: HitKind::Blasted,
        };
        let out = r.handle(2, hit(2), [0.0; 3], 1.5);
        assert!(matches!(out[0].1, Down::Hit { wumpa: 2, .. }), "{out:?}");
        assert!(
            r.handle(2, hit(9), [0.0; 3], 1.5).is_empty(),
            "no such shot"
        );
    }

    #[test]
    fn a_bomb_detonates_and_a_shield_launches_only_while_ours() {
        let (mut r, _) = Referee::new(vec![], 100);
        let aim = Aim::default();
        assert!(
            r.handle(1, Up::Detonate, [0.0; 3], 0.0).is_empty(),
            "no bomb"
        );
        r.grant(1, Item::Bomb, 0.0);
        r.handle(1, Up::Fire { shot: 1, aim }, [0.0; 3], 1.0);
        assert!(
            r.handle(2, Up::Detonate, [0.0; 3], 1.5).is_empty(),
            "not theirs"
        );
        assert_eq!(
            r.handle(1, Up::Detonate, [0.0; 3], 1.5),
            vec![(None, Down::Detonate { shooter: 1 })]
        );
        let launch = |shot| Up::ShieldShot { shot, aim };
        assert!(
            r.handle(1, launch(2), [0.0; 3], 2.0).is_empty(),
            "no shield"
        );
        r.grant(1, Item::Shield, 2.0);
        r.handle(1, Up::Fire { shot: 3, aim }, [0.0; 3], 3.0);
        let out = r.handle(1, launch(4), [0.0; 3], 4.0);
        assert!(matches!(
            &out[..],
            [(
                None,
                Down::ShieldShot {
                    shooter: 1,
                    shot: 4,
                    ..
                }
            )]
        ));
        assert!(r.handle(1, launch(5), [0.0; 3], 4.5).is_empty(), "once");
        // The shield bomb is a shot a hit can name.
        let hit = Up::Hit {
            shooter: 1,
            shot: 4,
            kind: HitKind::Blasted,
        };
        assert_eq!(r.handle(2, hit, [0.0; 3], 5.0).len(), 1);
        // A green shield gone stale launches nothing.
        r.grant(1, Item::Shield, 6.0);
        r.handle(1, Up::Fire { shot: 6, aim }, [0.0; 3], 7.0);
        assert!(r.handle(1, launch(7), [0.0; 3], 30.0).is_empty());
    }
}
