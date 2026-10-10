//! The roulette's result: `VehPhysGeneral_SetHeldItem` (`Vehicle/VehPhysGeneral.c:1651-1962`),
//! its weighted tables (`zGlobal_DATA.c:8108-8174`) and the one random number generator behind
//! them, `MixRNG_Scramble` (`MixRNG/MixRNG.c:3-10`). This is the reference the server ports:
//! [`tables_cpp`] prints the tables as C++ arrays.

use crate::item::Item;

/// CTR's 16-bit LCG, `s = (s * 0x6255 + 0x3619) & 0xFFFF` (`MixRNG/MixRNG.c:3-10`). One global
/// generator feeds item rolls, fruit-crate amounts, TNT hop counts and rain-cloud effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rng(pub u16);

impl Rng {
    /// `sdata->randomNumber`'s boot value (`zGlobal_SDATA.c:307`).
    pub const BOOT: u16 = 100;

    pub fn new(seed: u16) -> Self {
        Self(seed)
    }

    /// `MixRNG_Scramble`: advance, then return the new state.
    pub fn scramble(&mut self) -> u16 {
        self.0 = (u32::from(self.0) * 0x6255 + 0x3619) as u16;
        self.0
    }
}

impl Default for Rng {
    fn default() -> Self {
        Self(Self::BOOT)
    }
}

/// `ITEMSET_RNG_BUCKET_COUNT`: the roll is bucketed to 200 before it is scaled to a table.
const BUCKETS: u32 = 200;
/// `ITEMSET_RNG_RANDOM_SHIFT`.
const SHIFT: u32 = 3;

use Item::{
    Beaker as P, Bomb as B1, Bomb3 as B3, Clock as C, Invisibility as I, Mask as K, Missile as M1,
    Missile3 as M3, Shield as S, SuperEngine as E, Tnt as T, Turbo as U, WarpOrb as W,
};

/// `RNG_itemSetRace1` (`zGlobal_DATA.c:8108-8116`).
pub const RACE1: [Item; 20] = [P, P, P, P, P, P, P, P, T, T, T, T, T, T, T, T, B1, B1, U, S];
/// `RNG_itemSetRace2` (`zGlobal_DATA.c:8118-8134`).
pub const RACE2: [Item; 52] = [
    P, P, P, P, P, P, P, P, P, P, T, T, T, T, T, T, T, T, B1, B1, B1, B1, B1, B3, B3, B3, U, U, U,
    U, U, S, S, S, M1, M1, M1, M3, M3, W, W, W, W, W, K, K, K, K, K, K, K, C,
];
/// `RNG_itemSetRace3` (`zGlobal_DATA.c:8136-8151`).
pub const RACE3: [Item; 20] = [
    P, T, B1, B3, B3, U, U, S, M1, M1, M3, W, W, W, K, K, K, K, K, C,
];
/// `RNG_itemSetRace4` (`zGlobal_DATA.c:8153-8164`): 20 stored, only the first 19 sampled
/// (`ITEMSET_WEAPON_COUNT_RACE4`, `VehPhysGeneral.c:1827-1828`).
pub const RACE4: [Item; 20] = [U, S, M1, M3, W, W, W, W, W, K, K, K, K, K, K, K, K, C, C, U];
/// `RNG_itemSetBossrace` (`zGlobal_DATA.c:8166-8169`); also any 2-driver race's non-leader.
pub const BOSS_RACE: [Item; 20] = [
    P, T, B1, B3, B3, U, U, S, M1, M3, W, W, W, K, K, K, K, K, C, C,
];
/// `RNG_itemSetBattleDefault` (`zGlobal_DATA.c:8171-8174`).
pub const BATTLE_DEFAULT: [Item; 20] = [
    P, T, B1, B1, B1, B3, B3, S, S, M1, M1, M1, M1, M1, K, K, K, E, E, I,
];

/// The weighted tables, in `enum ItemSet` order where they appear there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Table {
    Race1,
    Race2,
    Race3,
    Race4,
    BattleDefault,
    BossRace,
}

impl Table {
    pub const ALL: [Table; 6] = [
        Table::Race1,
        Table::Race2,
        Table::Race3,
        Table::Race4,
        Table::BossRace,
        Table::BattleDefault,
    ];

    /// The entries the roll samples (`ITEMSET_WEAPON_COUNT_*`).
    pub fn entries(self) -> &'static [Item] {
        match self {
            Table::Race1 => &RACE1,
            Table::Race2 => &RACE2,
            Table::Race3 => &RACE3,
            Table::Race4 => &RACE4[..19],
            Table::BossRace => &BOSS_RACE,
            Table::BattleDefault => &BATTLE_DEFAULT,
        }
    }

    /// `item = table[rng * N / 200]` (`VehPhysGeneral.c:1807-1841`).
    pub fn pick(self, rng: u16) -> Item {
        let entries = self.entries();
        entries[(u32::from(rng) * entries.len() as u32 / BUCKETS) as usize]
    }

    pub fn name(self) -> &'static str {
        match self {
            Table::Race1 => "Race1",
            Table::Race2 => "Race2",
            Table::Race3 => "Race3",
            Table::Race4 => "Race4",
            Table::BossRace => "BossRace",
            Table::BattleDefault => "BattleDefault",
        }
    }
}

/// The race-wide state the post-roll overrides read. The server owns it: after a roll it records
/// the result with [`RaceItems::take`], and releases it with [`RaceItems::release`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RaceItems {
    /// Human players in the race (`numPlyrCurrGame`).
    pub humans: usize,
    /// `WARPBALL_HELD`: a warp orb is held or in flight (`VehPhysGeneral.c:1913-1929`).
    pub warp_orb_held: bool,
    /// `numPlayersWith3Missiles` (`VehPhysGeneral.c:1932-1953`).
    pub three_missile_holders: usize,
}

/// `ITEMSET_THREE_MISSILES_MIN_PLAYERS`, `ITEMSET_THREE_MISSILES_HELD_LIMIT`.
const THREE_MISSILES_MIN_HUMANS: usize = 3;
const THREE_MISSILES_LIMIT: usize = 2;

impl RaceItems {
    pub fn new(humans: usize) -> Self {
        Self {
            humans,
            ..Self::default()
        }
    }

    /// Record a rolled item, as `SetHeldItem` does as it overrides: a warp orb sets
    /// `WARPBALL_HELD`; Missile ×3 with 3+ humans counts a holder.
    pub fn take(&mut self, item: Item) {
        match item {
            Item::WarpOrb => self.warp_orb_held = true,
            Item::Missile3 if self.humans >= THREE_MISSILES_MIN_HUMANS => {
                self.three_missile_holders += 1
            }
            _ => {}
        }
    }

    /// The item is spent: the warp orb died (`231/RB_Warpball.c:33`), or the last of a Missile ×3
    /// was fired (`VehPhysProc.c:622-629`).
    pub fn release(&mut self, item: Item) {
        match item {
            Item::WarpOrb => self.warp_orb_held = false,
            Item::Missile3 if self.humans >= THREE_MISSILES_MIN_HUMANS => {
                self.three_missile_holders = self.three_missile_holders.saturating_sub(1)
            }
            _ => {}
        }
    }
}

/// The table for `rank` (0 = 1st) in a field of `racers` drivers, before the lap-1 downgrade;
/// `None` is the uniform fallback over ids 0–11 (`VehPhysGeneral.c:1692-1791`). The 3-driver
/// field draws once more from `rng` for its 2nd place.
pub fn table_for(rng: &mut Rng, rank: usize, racers: usize) -> Option<Table> {
    let set = |n: usize| match n {
        0 => Some(Table::Race1),
        1 => Some(Table::Race2),
        2 => Some(Table::Race3),
        3 => Some(Table::Race4),
        // Ranks past the field are unreachable in CTR; set 4 would be BattleDefault.
        _ => None,
    };
    match racers {
        // 1P Arcade: 1st Race1; 2nd–4th Race2; 5th–6th Race3; 7th–8th Race4.
        8 if rank == 1 => Some(Table::Race2),
        8 => set(rank / 2),
        // 2P Arcade: 1st Race1; 2nd–3rd Race2; 4th–5th Race3; 6th Race4.
        6 if rank == 0 => Some(Table::Race1),
        6 if rank == 5 => Some(Table::Race4),
        6 => set((rank - 1) / 2 + 1),
        5 if rank == 4 => Some(Table::Race4),
        5 | 4 => set(rank),
        3 => match rank {
            0 => Some(Table::Race1),
            1 if rng.scramble() & 1 != 0 => Some(Table::Race2),
            1 => Some(Table::Race3),
            _ => Some(Table::Race4),
        },
        2 if rank == 0 => Some(Table::Race1),
        2 => Some(Table::BossRace),
        _ => None,
    }
}

/// `VehPhysGeneral_SetHeldItem` for a race (not battle, Crystal Challenge or an Adventure boss):
/// the table by `position` (0 = 1st) and field size, Race4 downgraded to Race3 on lap 1
/// (`lap == 0`, `:1801-1804`), `(MixRNG_Scramble() >> 3) % 200` bucketed into it (`:1807-1841`),
/// or `MixRNG_Scramble() % 12` for a field size without tables (`:1862-1866`). Then the overrides
/// (`:1907-1953`): Spring → Turbo; a warp orb while one is held → Missile ×3; Missile ×3 with 3+
/// humans and 2 holders already → Missile ×1. Record the result with [`RaceItems::take`].
pub fn roll(rng: &mut Rng, position: usize, racers: usize, lap: u32, ctx: &RaceItems) -> Item {
    let rank = position.min(racers.saturating_sub(1));
    let mut table = table_for(rng, rank, racers);
    if table == Some(Table::Race4) && lap == 0 {
        table = Some(Table::Race3);
    }
    let bucket = (rng.scramble() >> SHIFT) % BUCKETS as u16;
    let mut item = match table {
        Some(t) => t.pick(bucket),
        None => Item::RACE[(rng.scramble() % Item::RACE.len() as u16) as usize],
    };
    if item == Item::Spring {
        item = Item::Turbo;
    }
    if item == Item::WarpOrb && ctx.warp_orb_held {
        item = Item::Missile3;
    }
    if item == Item::Missile3
        && ctx.humans >= THREE_MISSILES_MIN_HUMANS
        && ctx.three_missile_holders >= THREE_MISSILES_LIMIT
    {
        item = Item::Missile;
    }
    item
}

/// The tables as C++ arrays of CTR item ids, with the sampled length, for the server's port.
pub fn tables_cpp() -> String {
    let mut out = String::new();
    for t in Table::ALL {
        let ids: Vec<String> = t.entries().iter().map(|i| i.id().to_string()).collect();
        out.push_str(&format!(
            "static const uint8 k{}[{}] = {{ {} }};\n",
            t.name(),
            ids.len(),
            ids.join(", ")
        ));
    }
    out
}

/// Each table's exact odds over the LCG's full 65536-state period, as a Markdown table.
pub fn odds_markdown() -> String {
    let mut out = String::from("| Item |");
    for t in Table::ALL {
        out.push_str(&format!(" {} |", t.name()));
    }
    out.push_str("\n|---|");
    out.push_str(&"---|".repeat(Table::ALL.len()));
    out.push('\n');
    let items = [
        Item::Beaker,
        Item::Tnt,
        Item::Bomb,
        Item::Bomb3,
        Item::Turbo,
        Item::Shield,
        Item::Missile,
        Item::Missile3,
        Item::WarpOrb,
        Item::Mask,
        Item::Clock,
        Item::SuperEngine,
        Item::Invisibility,
    ];
    for item in items {
        out.push_str(&format!("| {} |", item.name()));
        for t in Table::ALL {
            let p = odds(t, item);
            if p == 0.0 {
                out.push_str(" – |");
            } else {
                out.push_str(&format!(" {:.1}% |", p * 100.0));
            }
        }
        out.push('\n');
    }
    out
}

/// The exact chance `table` gives `item`, over every LCG output.
pub fn odds(table: Table, item: Item) -> f64 {
    let hits = (0..=u16::MAX)
        .filter(|&s| table.pick((s >> SHIFT) % BUCKETS as u16) == item)
        .count();
    hits as f64 / 65536.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first states from the boot seed, by hand: 100 * 0x6255 + 0x3619 = 2_531_149 → 40781 (0x9F4D).
    #[test]
    fn lcg_sequence() {
        let mut rng = Rng::default();
        let mut expect = 100u32;
        for _ in 0..1000 {
            expect = (expect * 25173 + 13849) % 65536;
            assert_eq!(u32::from(rng.scramble()), expect);
        }
        let mut rng = Rng::default();
        assert_eq!(rng.scramble(), 0x9F4D);
        // Full period: every state is visited once.
        let mut seen = vec![false; 65536];
        let mut rng = Rng::new(0);
        for _ in 0..65536 {
            let s = rng.scramble() as usize;
            assert!(!seen[s]);
            seen[s] = true;
        }
    }

    /// Rolls `n` times at one position and returns the share of `item`.
    fn share(position: usize, racers: usize, lap: u32, item: Item, n: usize) -> f64 {
        let mut rng = Rng::default();
        let ctx = RaceItems::new(1);
        let hits = (0..n)
            .filter(|_| roll(&mut rng, position, racers, lap, &ctx) == item)
            .count();
        hits as f64 / n as f64
    }

    #[test]
    fn race1_is_beakers_and_tnt() {
        // Over the full period the shares are exact.
        assert!((odds(Table::Race1, Item::Beaker) - 0.40).abs() < 0.001);
        assert!((odds(Table::Race1, Item::Tnt) - 0.40).abs() < 0.001);
        // And the roll from 1st of 8 sees them.
        let n = 65536;
        assert!((share(0, 8, 1, Item::Beaker, n) - 0.40).abs() < 0.01);
        assert!((share(0, 8, 1, Item::Tnt, n) - 0.40).abs() < 0.01);
        assert_eq!(share(0, 8, 1, Item::Mask, n), 0.0);
    }

    #[test]
    fn race4_is_mostly_masks() {
        assert!((odds(Table::Race4, Item::Mask) - 0.42).abs() < 0.005);
        assert!((odds(Table::Race4, Item::WarpOrb) - 0.26).abs() < 0.005);
        // 8th of 8 on lap 2 rolls Race4 (warp orbs are rolled freely: nobody holds one).
        assert!((share(7, 8, 1, Item::Mask, 65536) - 0.42).abs() < 0.01);
    }

    #[test]
    fn spec_odds_match() {
        // The spec's table (`docs/ctr-weapons-spec.md` 1.5), exact to its rounding.
        let cases = [
            (Table::Race2, Item::Beaker, 19.5),
            (Table::Race2, Item::Clock, 1.5),
            (Table::Race3, Item::Mask, 25.0),
            (Table::Race3, Item::Clock, 4.9),
            (Table::Race4, Item::Turbo, 5.5),
            (Table::Race4, Item::Clock, 10.4),
            (Table::BossRace, Item::Clock, 9.9),
            (Table::Race1, Item::Shield, 4.9),
        ];
        for (t, item, pct) in cases {
            let p = odds(t, item) * 100.0;
            assert!((p - pct).abs() < 0.06, "{t:?} {item:?}: {p:.2} vs {pct}");
        }
    }

    #[test]
    fn lap_one_downgrades_race4_to_race3() {
        // Race3 has beakers; Race4 has none.
        assert!(share(7, 8, 0, Item::Beaker, 20000) > 0.03);
        assert_eq!(share(7, 8, 1, Item::Beaker, 20000), 0.0);
        assert!((share(7, 8, 0, Item::Mask, 65536) - 0.25).abs() < 0.01);
    }

    #[test]
    fn tables_by_field_size() {
        let mut rng = Rng::default();
        let t = |rng: &mut Rng, rank, racers| table_for(rng, rank, racers);
        let eight: Vec<_> = (0..8).map(|r| t(&mut rng, r, 8).unwrap()).collect();
        use Table::*;
        assert_eq!(
            eight,
            [Race1, Race2, Race2, Race2, Race3, Race3, Race4, Race4]
        );
        let six: Vec<_> = (0..6).map(|r| t(&mut rng, r, 6).unwrap()).collect();
        assert_eq!(six, [Race1, Race2, Race2, Race3, Race3, Race4]);
        let five: Vec<_> = (0..5).map(|r| t(&mut rng, r, 5).unwrap()).collect();
        assert_eq!(five, [Race1, Race2, Race3, Race4, Race4]);
        assert_eq!(t(&mut rng, 1, 2), Some(BossRace));
        assert_eq!(t(&mut rng, 0, 7), None);
        // 3 drivers: 2nd is Race2 or Race3 by one extra draw.
        let mut r = Rng::default();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..100 {
            seen.insert(t(&mut r, 1, 3).unwrap());
        }
        assert_eq!(seen.len(), 2);
    }

    #[test]
    fn uniform_fallback_for_other_fields() {
        // 7 racers have no tables: ids 0–11 uniformly, Spring folded into Turbo.
        let turbo = share(3, 7, 1, Item::Turbo, 65536);
        assert!((turbo - 2.0 / 12.0).abs() < 0.01, "{turbo}");
        assert_eq!(share(3, 7, 1, Item::Spring, 65536), 0.0);
    }

    #[test]
    fn one_warp_orb_at_a_time() {
        let mut ctx = RaceItems::new(4);
        let mut rng = Rng::default();
        let mut orbs = 0;
        let mut m3 = 0;
        for _ in 0..2000 {
            let item = roll(&mut rng, 7, 8, 1, &ctx);
            if item == Item::WarpOrb {
                orbs += 1;
                ctx.take(item);
            } else if item == Item::Missile3 {
                m3 += 1;
            }
        }
        assert_eq!(orbs, 1);
        // With 4 humans and nobody counted as a holder, the converted orbs stay ×3.
        assert!(m3 > 400, "{m3}");
        ctx.release(Item::WarpOrb);
        assert!(!ctx.warp_orb_held);
    }

    #[test]
    fn at_most_two_missile3_holders_with_three_humans() {
        let mut ctx = RaceItems::new(3);
        ctx.three_missile_holders = 2;
        ctx.warp_orb_held = true;
        let mut rng = Rng::default();
        for _ in 0..5000 {
            assert_ne!(roll(&mut rng, 7, 8, 1, &ctx), Item::Missile3);
        }
        // Two humans are not capped.
        let mut two = ctx;
        two.humans = 2;
        let mut rng = Rng::default();
        assert!((0..5000).any(|_| roll(&mut rng, 7, 8, 1, &two) == Item::Missile3));
    }

    /// `cargo test -p ctr-weapons print_tables -- --nocapture` prints the tables for the server.
    #[test]
    fn print_tables() {
        let cpp = tables_cpp();
        assert!(cpp.contains("kRace4[19]"));
        println!("{cpp}\n{}", odds_markdown());
    }
}
