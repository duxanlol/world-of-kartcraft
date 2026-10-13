//! The offline referee: the Kartcraft server's weapon rules (`setup/vmangos/src/Kartcraft.cpp`,
//! `HandleWeapon`; `docs/kartcraft-weapons-protocol.md`) for a race with no server, so the demo
//! drives the same [`super::Weapons`] the client does. It owns the crates and their respawns,
//! rolls the roulette ([`ctr_weapons::roll`]), keeps each racer's item, shots and Wumpa, and
//! relays shots, detonations, shield bombs, hits, rams and rain clouds.

use std::collections::HashMap;

use ctr_kart::Ram;
use ctr_weapons::cloud::CLOUD_MS;
use ctr_weapons::{roll, HitKind, Item, KartId, RaceItems, RainCloud, Rng, ShotKey};

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
/// A shot breaks a crate no farther from where it left than it can have flown at the warp orb's
/// 112 lu a frame, the fastest (`RB_Warpball.c`), 92 yd/s (`SHOT_SPEED`)...
const SHOT_SPEED: f32 = 92.0;
/// ...and a juiced bomb's 512 lu blast, with the crate's size and slack: 16 yd (`BREAK_REACH`).
const BREAK_REACH: f32 = 16.0;
/// The mask's 8 s, juiced 12 s: no Wumpa is lost under it (`231/RB_Player.c:153-161`).
const MASK_TIME: f32 = 7680.0 / 960.0;
const MASK_TIME_JUICED: f32 = 11520.0 / 960.0;
/// A green shield lasts 12 s (`VehPickupItem.c:976-983`); a second's slack for the line.
const SHIELD_TIME: f32 = 13.0;
/// The turbo item burns 2400 ms of reserves (`VehPickupItem.c`); a ram under it counts this long
/// after its relay, with slack for the line.
const TURBO_TIME: f32 = 3.0;
/// A rain cloud's 7.68 s (`RB_MaskShieldCloud.c:615`).
const CLOUD_TIME: f32 = CLOUD_MS as f32 / 1000.0;
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
    /// When the turbo item was last fired.
    turbo_at: Option<f32>,
    /// A rain cloud over the racer, and when it goes.
    cloud: Option<(RainCloud, f32)>,
}

struct Record {
    item: Item,
    juiced: bool,
    at: f32,
    hit: bool,
    dropped: bool,
    /// Where it left from, WoW world x and y.
    from: [f32; 2],
    /// A Shield launched (`W Z`), not one put up.
    shield_bomb: bool,
}

impl Record {
    /// Whether it can break crates: a bomb, missile, shield bomb or warp orb, or a TNT, Nitro or
    /// beaker where it is dropped (`ctr_weapons::boxes`).
    fn breaks_crates(&self) -> bool {
        match self.item.single() {
            Item::Bomb | Item::Missile | Item::WarpOrb => !self.dropped,
            Item::Shield => self.shield_bomb,
            Item::Tnt | Item::Beaker => self.dropped,
            _ => false,
        }
    }

    /// How far from where it left (yards) it can break a crate `elapsed` seconds on.
    fn crate_reach(&self, elapsed: f32) -> f32 {
        if self.dropped {
            CRATE_RADIUS
        } else {
            SHOT_SPEED * elapsed + BREAK_REACH
        }
    }
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

    /// The roulette a cloud locked spins out: a new roll, given at once (`RB_RainCloud_ThTick`,
    /// `RB_MaskShieldCloud.c:556-561`).
    fn reroll(&mut self, who: KartId, now: f32) -> Vec<Addressed> {
        let position = usize::from(self.rng.scramble()) % RACERS;
        let rolled = roll(&mut self.rng, position, RACERS, LAP, &RaceItems::new(1));
        let item = self.force.unwrap_or(rolled);
        let out = self.grant(who, item, now);
        // No roulette to wait for: it shows at once.
        if let Some(r) = self.racers.get_mut(&who) {
            r.claimed = now - ROLL_MIN;
        }
        out
    }

    /// A red beaker's spin-out took on `who` (`RB_GenericMine.c:172-176`): the cloud starts, or
    /// starts again with a new effect ([`RainCloud::rain`]); a held item goes into the roulette.
    fn rain(&mut self, who: KartId, now: f32) -> Vec<Addressed> {
        let r = self.racers.entry(who).or_default();
        let holds = r.held.is_some() && r.shots != 0;
        let cloud = RainCloud::rain(r.cloud.map(|c| c.0), holds, &mut self.rng);
        if cloud.locked {
            r.held = None;
            r.shots = 0;
        }
        r.cloud = Some((cloud, now + CLOUD_TIME));
        let mut out = vec![(
            None,
            Down::Cloud {
                victim: who,
                effect: cloud.effect,
                locked: cloud.locked,
            },
        )];
        if cloud.rerolls(false) {
            if let Some(r) = self.racers.get_mut(&who) {
                r.cloud = Some((
                    RainCloud {
                        locked: false,
                        ..cloud
                    },
                    now + CLOUD_TIME,
                ));
            }
            out.extend(self.reroll(who, now));
        }
        out
    }

    /// Crates due back come up; old shots are forgotten; clouds go, and the roulettes they held
    /// are rolled (`TickWeapons`).
    pub fn tick(&mut self, now: f32) -> Vec<Addressed> {
        let mut out = Vec::new();
        let gone: Vec<(KartId, bool)> = self
            .racers
            .iter_mut()
            .filter_map(|(&id, r)| {
                let (cloud, until) = r.cloud?;
                (now >= until).then(|| {
                    r.cloud = None;
                    (id, cloud.rerolls(true))
                })
            })
            .collect();
        for (id, reroll) in gone {
            if reroll {
                out.extend(self.reroll(id, now));
            }
        }
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

    /// What a crate of `kind` broken by `to` (its kart or its shot) gives them.
    fn give(&mut self, to: KartId, kind: CrateKind, now: f32) -> Vec<Addressed> {
        if kind == CrateKind::Weapon {
            // Holding an item, or under a cloud spinning one, the crate breaks for nothing
            // (`RB_Crate.c:152-226`).
            let clouded = self
                .racers
                .get(&to)
                .and_then(|r| r.cloud)
                .is_some_and(|(c, _)| c.blocks_crates());
            if clouded {
                return Vec::new();
            }
            // Holding one already, as the client claims a weapon crate only with its slot empty:
            // the two disagree, so its roulette lands on what is held.
            if self.holds(to) {
                let r = self.racers.entry(to).or_default();
                return match r.held {
                    Some(item) => vec![(
                        Some(to),
                        Down::Holding {
                            item,
                            shots: r.shots,
                        },
                    )],
                    None => Vec::new(),
                };
            }
            // Anywhere in the field, for variety.
            let position = usize::from(self.rng.scramble()) % RACERS;
            let rolled = roll(&mut self.rng, position, RACERS, LAP, &RaceItems::new(1));
            let item = self.force.unwrap_or(rolled);
            return self.grant(to, item, now);
        }
        // A fruit crate gives `MixRNG % 4 + 5` (`RB_Crate.c:316`), a fruit one.
        let gain = if kind == CrateKind::Fruit {
            self.rng.scramble() % 4 + 5
        } else {
            1
        };
        let r = self.racers.entry(to).or_default();
        r.wumpa = (u16::from(r.wumpa) + gain).min(u16::from(ctr_weapons::item::WUMPA_MAX)) as u8;
        vec![(Some(to), Down::Wumpa(r.wumpa))]
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
                out.extend(self.give(from, kind, now));
            }
            Up::Break { shot, index } => {
                // Our shot broke a crate (`ctr_weapons::boxes`): a live shot of ours that breaks
                // crates, a full-grown crate (no single fruit) it can have reached by now.
                let Some(s) = self.shots.get(&ShotKey { owner: from, shot }) else {
                    return out;
                };
                let Some(c) = self.crates.get_mut(index) else {
                    return out;
                };
                let d = (c.spot.at[0] - s.from[0]).hypot(c.spot.at[1] - s.from[1]);
                if !s.breaks_crates()
                    || now - s.at >= SHOT_LIFE
                    || d > s.crate_reach(now - s.at)
                    || c.spot.kind == CrateKind::Wumpa
                    || !c.up
                    || now < c.grown_at
                {
                    return out;
                }
                c.up = false;
                c.back_at = Some(now + CRATE_RESPAWN);
                let kind = c.spot.kind;
                // A bomb, missile or shield bomb gives its shooter the crate; a warp orb or a
                // mine breaks it for no one (`RB_Crate.c:152-226`, `:315-355`).
                let by = if ctr_weapons::boxes::rewards(s.item) && !s.dropped {
                    from
                } else {
                    0
                };
                out.push((None, Down::Taken { index, by }));
                if by != 0 {
                    out.extend(self.give(from, kind, now));
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
                if now - r.claimed < ROLL_MIN {
                    return out;
                }
                // The client spent the shot when it fired: so does this, relayed or not.
                r.shots -= 1;
                if r.shots == 0 {
                    r.held = None;
                }
                if is_projectile(item) && live >= MAX_LIVE_SHOTS {
                    return out;
                }
                let juiced = ctr_weapons::item::juiced(r.wumpa);
                if item == Item::Mask {
                    r.mask_until = now + if juiced { MASK_TIME_JUICED } else { MASK_TIME };
                    // `ACTION_MASK_WEAPON` blows the cloud away (`RB_MaskShieldCloud.c:527-532`).
                    r.cloud = None;
                }
                if item == Item::Turbo {
                    r.turbo_at = Some(now);
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
                        juiced,
                        at: now,
                        hit: false,
                        dropped: aim.dropped,
                        from: [aim.at[0], aim.at[1]],
                        shield_bomb: false,
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
                let mut red = false;
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
                    red = s.item == Item::Beaker && s.juiced && kind == HitKind::SpinOut;
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
                if red {
                    out.extend(self.rain(from, now));
                }
            }
            Up::Ram { attacker, ram } => {
                // `VehPhysCrash_Attack`: only with the attacker's mask on, its shield up (spent
                // now), or its turbo item burning.
                let Some(a) = self.racers.get_mut(&attacker).filter(|_| attacker != from) else {
                    return out;
                };
                let armed = match ram {
                    Ram::Mask => now < a.mask_until,
                    Ram::Shield => a
                        .shield
                        .take()
                        .is_some_and(|(at, juiced)| juiced || now - at <= SHIELD_TIME),
                    Ram::Turbo => a.turbo_at.is_some_and(|t| now - t <= TURBO_TIME),
                };
                if !armed {
                    return out;
                }
                let r = self.racers.entry(from).or_default();
                // As the hit it is: a blast costs 3, a squish 1.
                if now >= r.mask_until {
                    r.wumpa = r
                        .wumpa
                        .saturating_sub(if ram == Ram::Turbo { 1 } else { 3 });
                }
                out.push((
                    None,
                    Down::Ram {
                        victim: from,
                        attacker,
                        ram,
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
                        juiced,
                        at: now,
                        hit: false,
                        dropped: false,
                        from: [aim.at[0], aim.at[1]],
                        shield_bomb: true,
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
            Up::Lose(loss) => {
                // Taken as hits take it, none while masked; the total goes to the loser alone.
                let r = self.racers.entry(from).or_default();
                if now >= r.mask_until {
                    r.wumpa = r.wumpa.saturating_sub(loss.wumpa());
                }
                out.push((Some(from), Down::Wumpa(r.wumpa)));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::super::wire::Aim;
    use super::*;
    use ctr_weapons::CloudEffect;

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
        // Holding an item, the crate breaks for nothing but saying what is held, for a client
        // that thought its slot empty.
        let out = r.handle(1, Up::Claim(0), [10.0, 20.0, 0.0], 1.6);
        assert_eq!(out[0], (None, Down::Taken { index: 0, by: 1 }));
        assert!(matches!(out[1], (Some(1), Down::Holding { .. })), "{out:?}");
    }

    #[test]
    fn a_fire_the_full_pool_refuses_still_spends_the_item() {
        let (mut r, _) = Referee::new(vec![spot(CrateKind::Weapon)], 100);
        for shot in 1..=12 {
            fire_from(
                &mut r,
                Item::Missile,
                shot,
                false,
                10.0 + f32::from(shot) * 0.01,
            );
        }
        // Twelve in flight: the next shot goes nowhere, but its item is spent, as the client spent
        // it when it fired.
        r.grant(1, Item::Bomb, 9.0);
        let aim = Aim {
            at: [10.0, 0.0, 0.0],
            ..Aim::default()
        };
        assert!(r
            .handle(1, Up::Fire { shot: 13, aim }, [0.0; 3], 10.5)
            .is_empty());
        assert!(!r.holds(1));
        // So the next weapon crate rolls, not an empty wheel.
        let out = r.handle(1, Up::Claim(0), [11.0, 20.0, 0.0], 10.6);
        assert!(matches!(out[1], (Some(1), Down::Item { .. })), "{out:?}");
    }

    /// Racer 1 fires `item` from (10, 0) at `now` as shot `shot`, `dropped` behind or not.
    fn fire_from(r: &mut Referee, item: Item, shot: u16, dropped: bool, now: f32) {
        r.grant(1, item, now - 1.0);
        let aim = Aim {
            at: [10.0, 0.0, 0.0],
            dropped,
            ..Aim::default()
        };
        let out = r.handle(1, Up::Fire { shot, aim }, [0.0; 3], now);
        assert_eq!(out.len(), 1, "{out:?}");
    }

    fn broke(shot: u16, index: usize) -> Up {
        Up::Break { shot, index }
    }

    #[test]
    fn a_bomb_breaks_a_crate_for_its_shooter_and_it_comes_back_as_any_crate() {
        // Weapon crate 0 at (10, 20), 20 yd from where the shots leave.
        let (mut r, _) = Referee::new(vec![spot(CrateKind::Weapon)], 100);
        fire_from(&mut r, Item::Bomb, 1, false, 1.0);
        // Not the shooter's to report, nor a shot that is not.
        assert!(r.handle(2, broke(1, 0), [0.0; 3], 1.2).is_empty());
        assert!(r.handle(1, broke(2, 0), [0.0; 3], 1.2).is_empty());
        assert!(r.handle(1, broke(1, 7), [0.0; 3], 1.2).is_empty());
        let out = r.handle(1, broke(1, 0), [0.0; 3], 1.2);
        assert_eq!(out[0], (None, Down::Taken { index: 0, by: 1 }));
        assert!(
            matches!(out[1], (Some(1), Down::Item { .. })),
            "the shooter rolls: {out:?}"
        );
        // Broken: neither the shot nor a kart breaks it again until it is back, 1 s on.
        assert!(r.handle(1, broke(1, 0), [0.0; 3], 1.3).is_empty());
        assert!(r.handle(2, Up::Claim(0), [10.0, 20.0, 0.0], 1.3).is_empty());
        assert!(r.tick(2.15).is_empty());
        assert_eq!(r.tick(2.2), vec![(None, Down::Back { index: 0 })]);
        // Regrowing, it cannot be broken.
        assert!(r.handle(1, broke(1, 0), [0.0; 3], 2.5).is_empty());
        // Full again; the shooter holds an item now, so it breaks for nothing but saying what is
        // held.
        let out = r.handle(1, broke(1, 0), [0.0; 3], 2.8);
        assert_eq!(out[0], (None, Down::Taken { index: 0, by: 1 }));
        assert!(matches!(out[1], (Some(1), Down::Holding { .. })), "{out:?}");
    }

    #[test]
    fn a_shot_breaks_only_crates_it_can_have_reached() {
        let far = CrateSpot {
            kind: CrateKind::Fruit,
            at: [10.0, 200.0, 0.0],
        };
        let (mut r, _) = Referee::new(vec![far, spot(CrateKind::Wumpa)], 100);
        fire_from(&mut r, Item::Missile, 1, false, 1.0);
        // 200 yd off: not 0.5 s after the shot (92 yd/s and 16 yd), but 2.5 s after.
        assert!(r.handle(1, broke(1, 0), [0.0; 3], 1.5).is_empty());
        let out = r.handle(1, broke(1, 0), [0.0; 3], 3.5);
        assert_eq!(out[0], (None, Down::Taken { index: 0, by: 1 }));
        assert!(
            matches!(out[1], (Some(1), Down::Wumpa(5..=8))),
            "the shooter gets the fruit: {out:?}"
        );
        // A single fruit is the karts' alone.
        assert!(r.handle(1, broke(1, 1), [0.0; 3], 1.5).is_empty());
        // A shot forgotten breaks nothing.
        r.tick(40.0);
        assert!(r.handle(1, broke(1, 0), [0.0; 3], 40.0).is_empty());
    }

    #[test]
    fn a_warp_orb_or_a_dropped_mine_breaks_a_crate_for_no_one_and_a_shield_up_breaks_none() {
        let near = CrateSpot {
            kind: CrateKind::Weapon,
            at: [12.0, 5.0, 0.0],
        };
        let (mut r, _) = Referee::new(vec![spot(CrateKind::Weapon), near], 100);
        fire_from(&mut r, Item::WarpOrb, 1, false, 1.0);
        let out = r.handle(1, broke(1, 0), [0.0; 3], 1.2);
        assert_eq!(out, vec![(None, Down::Taken { index: 0, by: 0 })]);
        // A TNT breaks only the crate it is dropped on: 20 yd off is too far.
        fire_from(&mut r, Item::Tnt, 2, true, 3.0);
        let out = r.handle(1, broke(2, 1), [0.0; 3], 3.1);
        assert_eq!(out, vec![(None, Down::Taken { index: 1, by: 0 })]);
        r.tick(4.5);
        assert!(r.handle(1, broke(2, 0), [0.0; 3], 5.5).is_empty());
        // A shield put up is no shot at crates; launched, it gives as a bomb does.
        fire_from(&mut r, Item::Shield, 3, false, 6.0);
        assert!(r.handle(1, broke(3, 0), [0.0; 3], 6.5).is_empty());
        let aim = Aim {
            at: [10.0, 0.0, 0.0],
            ..Aim::default()
        };
        assert_eq!(
            r.handle(1, Up::ShieldShot { shot: 4, aim }, [0.0; 3], 6.6)
                .len(),
            1
        );
        let out = r.handle(1, broke(4, 0), [0.0; 3], 6.8);
        assert_eq!(out[0], (None, Down::Taken { index: 0, by: 1 }));
        assert!(matches!(out[1], (Some(1), Down::Item { .. })), "{out:?}");
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
    fn a_spin_out_costs_one_wumpa_and_the_mask_grab_two_but_not_under_a_mask() {
        use super::super::wire::Loss;
        let (mut r, _) = Referee::new(vec![], 100);
        r.racers.entry(1).or_default().wumpa = 4;
        let lose = |r: &mut Referee, loss, now| r.handle(1, Up::Lose(loss), [0.0; 3], now);
        assert_eq!(
            lose(&mut r, Loss::SpinOut, 1.0),
            vec![(Some(1), Down::Wumpa(3))]
        );
        assert_eq!(
            lose(&mut r, Loss::MaskGrab, 2.0),
            vec![(Some(1), Down::Wumpa(1))]
        );
        assert_eq!(
            lose(&mut r, Loss::MaskGrab, 3.0),
            vec![(Some(1), Down::Wumpa(0))]
        );
        r.racers.entry(1).or_default().wumpa = 4;
        r.racers.entry(1).or_default().mask_until = 10.0;
        assert_eq!(
            lose(&mut r, Loss::SpinOut, 4.0),
            vec![(Some(1), Down::Wumpa(4))]
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

    /// Racer 1 drops a beaker, juiced (red) or not, as shot `shot` at `now`.
    fn drop_beaker(r: &mut Referee, shot: u16, red: bool, now: f32) {
        r.racers.entry(1).or_default().wumpa = if red { 10 } else { 0 };
        r.grant(1, Item::Beaker, now - 1.0);
        let aim = Aim {
            dropped: true,
            ..Aim::default()
        };
        let out = r.handle(1, Up::Fire { shot, aim }, [0.0; 3], now);
        assert!(matches!(&out[..], [(None, Down::Shot(s))] if s.juiced == red));
    }

    fn spun(shot: u16) -> Up {
        Up::Hit {
            shooter: 1,
            shot,
            kind: HitKind::SpinOut,
        }
    }

    fn cloud_of(out: &[Addressed]) -> Option<(CloudEffect, bool)> {
        out.iter().find_map(|(_, d)| match d {
            Down::Cloud {
                victim: 2,
                effect,
                locked,
            } => Some((*effect, *locked)),
            _ => None,
        })
    }

    #[test]
    fn a_red_beaker_spins_the_held_item_for_seven_seconds_then_rerolls_it() {
        let (mut r, _) = Referee::new(vec![spot(CrateKind::Weapon)], 100);
        // A green one only spins.
        drop_beaker(&mut r, 1, false, 1.0);
        assert_eq!(cloud_of(&r.handle(2, spun(1), [0.0; 3], 1.5)), None);
        r.grant(2, Item::Missile3, 0.0);
        drop_beaker(&mut r, 2, true, 2.0);
        let out = r.handle(2, spun(2), [0.0; 3], 2.5);
        assert!(matches!(out[0].1, Down::Hit { victim: 2, .. }));
        assert_eq!(cloud_of(&out), Some((CloudEffect::ItemRoll, true)));
        // The item is gone into the roulette: no shot, and a crate breaks for nothing.
        let fire = Up::Fire {
            shot: 1,
            aim: Aim::default(),
        };
        assert!(r.handle(2, fire, [0.0; 3], 3.0).is_empty());
        let out = r.handle(2, Up::Claim(0), [10.0, 20.0, 0.0], 3.0);
        assert_eq!(out, vec![(None, Down::Taken { index: 0, by: 2 })]);
        // 7.68 s on, a new roll comes, and fires at once.
        let item = |out: &[Addressed]| {
            out.iter()
                .any(|(to, d)| *to == Some(2) && matches!(d, Down::Item { .. }))
        };
        assert!(!item(&r.tick(2.5 + 7.6)));
        let out = r.tick(2.5 + 7.7);
        assert!(item(&out), "{out:?}");
        assert!(r.holds(2));
        assert_eq!(r.handle(2, fire, [0.0; 3], 2.5 + 7.7).len(), 1);
        assert!(!item(&r.tick(20.0)), "once");
    }

    #[test]
    fn a_red_beaker_on_an_empty_kart_slows_it_and_a_second_redraws_the_effect() {
        let (mut r, _) = Referee::new(vec![], 100);
        drop_beaker(&mut r, 1, true, 1.0);
        let out = r.handle(2, spun(1), [0.0; 3], 1.5);
        assert_eq!(cloud_of(&out), Some((CloudEffect::Slow, false)));
        // A second one under it: the effect is MixRNG's (`(s % 400) / 100`), and the time
        // starts again.
        let mut peek = r.rng;
        let want = CloudEffect::from_id(((peek.scramble() % 400) / 100) as u8).unwrap();
        drop_beaker(&mut r, 2, true, 5.0);
        let out = r.handle(2, spun(2), [0.0; 3], 5.5);
        assert_eq!(cloud_of(&out), Some((want, false)));
        assert!(r.racers[&2].cloud.is_some_and(|(_, until)| until > 13.0));
        r.tick(13.2);
        assert!(r.racers[&2].cloud.is_none());
        // A mask blows a cloud away.
        drop_beaker(&mut r, 3, true, 14.0);
        r.handle(2, spun(3), [0.0; 3], 14.5);
        r.grant(2, Item::Mask, 13.0);
        r.handle(
            2,
            Up::Fire {
                shot: 9,
                aim: Aim::default(),
            },
            [0.0; 3],
            15.0,
        );
        assert!(r.racers[&2].cloud.is_none());
    }

    #[test]
    fn rams_need_the_mask_shield_or_turbo_and_a_shield_is_spent() {
        let (mut r, _) = Referee::new(vec![], 100);
        let rammed = |ram| Up::Ram { attacker: 1, ram };
        r.racers.entry(2).or_default().wumpa = 8;
        for ram in [Ram::Mask, Ram::Shield, Ram::Turbo] {
            assert!(
                r.handle(2, rammed(ram), [0.0; 3], 0.0).is_empty(),
                "{ram:?}"
            );
        }
        let fire = |r: &mut Referee, item, shot, now| {
            r.grant(1, item, now - 1.0);
            r.handle(
                1,
                Up::Fire {
                    shot,
                    aim: Aim::default(),
                },
                [0.0; 3],
                now,
            )
        };
        fire(&mut r, Item::Mask, 1, 1.0);
        let out = r.handle(2, rammed(Ram::Mask), [0.0; 3], 2.0);
        assert_eq!(
            out,
            vec![(
                None,
                Down::Ram {
                    victim: 2,
                    attacker: 1,
                    ram: Ram::Mask,
                    wumpa: 5
                }
            )]
        );
        assert!(
            r.handle(2, rammed(Ram::Mask), [0.0; 3], 10.0).is_empty(),
            "worn off"
        );
        assert!(
            r.handle(1, rammed(Ram::Mask), [0.0; 3], 2.0).is_empty(),
            "not itself"
        );
        fire(&mut r, Item::Shield, 2, 11.0);
        assert_eq!(r.handle(2, rammed(Ram::Shield), [0.0; 3], 12.0).len(), 1);
        assert!(
            r.handle(2, rammed(Ram::Shield), [0.0; 3], 12.1).is_empty(),
            "spent"
        );
        fire(&mut r, Item::Turbo, 3, 20.0);
        let out = r.handle(2, rammed(Ram::Turbo), [0.0; 3], 21.0);
        assert!(
            matches!(out[..], [(None, Down::Ram { wumpa: 1, .. })]),
            "{out:?}"
        );
        assert!(r.handle(2, rammed(Ram::Turbo), [0.0; 3], 23.5).is_empty());
    }
}
