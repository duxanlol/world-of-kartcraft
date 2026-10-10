//! The held items by CTR's own ids (`driver->heldItemID`, `include/namespace_Vehicle.h:641-657`),
//! the Wumpa cap and the juiced rule.

/// Wumpa fruit are clamped to `[0, 10]` (`231/RB_Player.c:141-197`).
pub const WUMPA_MAX: u8 = 10;
/// `DRIVER_WUMPA_JUICED_COUNT`: 10 fruit is juiced (`include/namespace_Vehicle.h:668`).
pub const JUICED_WUMPA: u8 = 10;
/// A stacked item's shots (`HELD_ITEM_STACK_COUNT`, `Vehicle/VehPhysGeneral.c:1956-1959`).
pub const STACK_SHOTS: u8 = 3;

/// Juiced is tested when the item is fired, not when it was rolled (`VehPickupItem.c:837-847`).
pub fn juiced(wumpa: u8) -> bool {
    wumpa >= JUICED_WUMPA
}

/// A held item. The discriminant is CTR's id.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Item {
    Turbo = 0,
    Bomb = 1,
    /// Missile ×1 (also the shared "bomb/missile" fire id, `VehPickupItem.c:1537-1569`).
    Missile = 2,
    /// TNT, or Nitro when fired juiced.
    Tnt = 3,
    /// Beaker (potion): green, or red when fired juiced.
    Beaker = 4,
    /// Unused; every roll of it becomes Turbo (`VehPhysGeneral.c:1907-1911`).
    Spring = 5,
    Shield = 6,
    /// Aku Aku / Uka Uka.
    Mask = 7,
    /// N. Tropy's clock.
    Clock = 8,
    WarpOrb = 9,
    Bomb3 = 10,
    Missile3 = 11,
    /// Battle only.
    Invisibility = 12,
    /// Battle only.
    SuperEngine = 13,
    #[default]
    None = 15,
}

impl Item {
    /// The ids 0–11, the fallback roll's range (`ITEMSET_FALLBACK_ITEM_COUNT`).
    pub const RACE: [Item; 12] = [
        Item::Turbo,
        Item::Bomb,
        Item::Missile,
        Item::Tnt,
        Item::Beaker,
        Item::Spring,
        Item::Shield,
        Item::Mask,
        Item::Clock,
        Item::WarpOrb,
        Item::Bomb3,
        Item::Missile3,
    ];

    pub fn id(self) -> u8 {
        self as u8
    }

    pub fn from_id(id: u8) -> Option<Item> {
        Some(match id {
            0..=11 => Item::RACE[id as usize],
            12 => Item::Invisibility,
            13 => Item::SuperEngine,
            15 => Item::None,
            _ => return None,
        })
    }

    /// Shots the item gives: 3 for Bomb ×3 and Missile ×3, none for None, else 1.
    pub fn shots(self) -> u8 {
        match self {
            Item::Bomb3 | Item::Missile3 => STACK_SHOTS,
            Item::None => 0,
            _ => 1,
        }
    }

    /// CTR's `numHeldItems` at roll time: 3 for the stacked items, 0 for the rest, whose one
    /// shot is the item itself (`VehPhysGeneral.c:1956-1959`).
    pub fn stack(self) -> u8 {
        if self.shots() == STACK_SHOTS {
            STACK_SHOTS
        } else {
            0
        }
    }

    /// What one shot of it launches: ×3 items fire their ×1 (`VehPickupItem.c:1537-1569`).
    pub fn single(self) -> Item {
        match self {
            Item::Bomb3 => Item::Bomb,
            Item::Missile3 => Item::Missile,
            other => other,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Item::Turbo => "Turbo",
            Item::Bomb => "Bomb",
            Item::Missile => "Missile",
            Item::Tnt => "TNT",
            Item::Beaker => "Beaker",
            Item::Spring => "Spring",
            Item::Shield => "Shield",
            Item::Mask => "Mask",
            Item::Clock => "Clock",
            Item::WarpOrb => "Warp orb",
            Item::Bomb3 => "Bomb x3",
            Item::Missile3 => "Missile x3",
            Item::Invisibility => "Invisibility",
            Item::SuperEngine => "Super Engine",
            Item::None => "none",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip() {
        for id in 0..=255u8 {
            if let Some(item) = Item::from_id(id) {
                assert_eq!(item.id(), id);
            }
        }
        assert_eq!(Item::from_id(14), None);
        assert_eq!(Item::Missile3.id(), 11);
        assert_eq!(Item::None.id(), 15);
    }

    #[test]
    fn stacked_items_give_three_shots_of_their_single() {
        assert_eq!(Item::Bomb3.shots(), 3);
        assert_eq!(Item::Missile3.single(), Item::Missile);
        assert_eq!(Item::Tnt.shots(), 1);
        assert_eq!(Item::Tnt.stack(), 0);
        assert!(juiced(10) && !juiced(9));
    }
}
