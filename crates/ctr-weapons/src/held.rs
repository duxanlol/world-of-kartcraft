//! The held item as the driver's own client sees it: the roulette spin, the result, the shots
//! left and the gap between them, as `VehPhysProc_Driving_PhysLinear` runs them
//! (`Vehicle/VehPhysProc.c:576-646` the timers, `:801-905` the Circle press). The result itself
//! comes from the server (the roll is [`crate::roulette::roll`]); this only times it.

use crate::item::{juiced, Item};

/// `itemRollTimer` at a crate break (`231/RB_Crate.c:198`): 90 frames, one a frame.
pub const ROLL_FRAMES: u16 = 90;
/// `VEH_PHYS_PROC_ITEM_ROLL_FAST_STOP_FRAMES`: Fire stops the roll once the timer is below 70.
pub const FAST_STOP_BELOW: u16 = 70;
/// `VEH_PHYS_PROC_WEAPON_COOLDOWN_STACK_FRAMES`: the gap after a shot with shots still stacked.
pub const STACK_GAP_FRAMES: u16 = 5;
/// `VEH_PHYS_PROC_WEAPON_COOLDOWN_EMPTY_FRAMES`: the icon lingers this long after the last shot.
pub const LINGER_FRAMES: u16 = 30;

/// One shot to launch: the ×1 item and whether it was fired juiced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shot {
    pub item: Item,
    pub juiced: bool,
}

/// What a Fire press did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fire {
    Nothing,
    /// The roulette was told to stop; the result shows on the next tick.
    StopRoulette,
    /// A bomb was out: this press detonates it (`VehPhysProc.c:812-819`).
    Detonate,
    /// A shield was up: this press fires it as a shield bomb (`VehPhysProc.c:822-829`).
    ShieldShot,
    Launch(Shot),
}

/// What the HUD shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeldView {
    Empty,
    Spinning,
    Item(Item),
    /// The last shot is fired; the icon flickers until it goes (`UI/UI_Weapon.c:85-89`).
    Fading(Item),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Held {
    /// `heldItemID`; `None` with `rolling` is CTR's roulette id 16.
    item: Item,
    rolling: bool,
    /// `itemRollTimer`.
    roll_timer: u16,
    /// The server's answer, waiting for the spin to end.
    result: Option<Item>,
    /// `numHeldItems`: 3 for a stacked item, counting down per shot; 0 for a single.
    stack: u8,
    /// `noItemTimer`, frames.
    no_item_timer: u16,
    /// `instBombThrow`: our bomb is rolling.
    bomb_out: bool,
    /// `instBubbleHold`: our shield is up.
    shield_up: bool,
    /// A rain cloud holds the roulette at 5 frames from its end (`RB_MaskShieldCloud.c:543-551`).
    locked: bool,
}

/// `d->itemRollTimer = 5` under an `ITEM_ROLL` cloud, every frame.
const CLOUD_ROLL_TIMER: u16 = 5;

impl Held {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a crate would start a roulette now (`231/RB_Crate.c:152-226`, checks 2–3).
    pub fn can_take_crate(&self) -> bool {
        !self.rolling && (self.item == Item::None || self.no_item_timer != 0) && self.stack == 0
    }

    /// A weapon crate was taken: spin for 90 frames (`RB_Crate.c:196-205`).
    pub fn start_roulette(&mut self) {
        self.item = Item::None;
        self.rolling = true;
        self.roll_timer = ROLL_FRAMES;
        self.result = None;
        self.stack = 0;
        self.no_item_timer = 0;
    }

    /// The server's roll for the running roulette. A roulette a rain cloud held is let go: the
    /// item shows on the next tick (`VehPhysGeneral_SetHeldItem` as the cloud goes,
    /// `RB_MaskShieldCloud.c:556-561`).
    pub fn set_result(&mut self, item: Item) {
        if self.rolling {
            self.result = Some(item);
            if std::mem::take(&mut self.locked) {
                self.roll_timer = 0;
            }
        }
    }

    /// An `ITEM_ROLL` rain cloud takes the held item into the roulette (`HELD_ITEM_ROULETTE`,
    /// shots gone), held 5 frames from its end while the cloud lasts: no Fire stops it
    /// (`RB_MaskShieldCloud.c:543-551`). A running roulette's result is dropped; the new roll
    /// comes as the cloud goes.
    pub fn cloud_lock(&mut self) {
        self.item = Item::None;
        self.rolling = true;
        self.roll_timer = CLOUD_ROLL_TIMER;
        self.result = None;
        self.stack = 0;
        self.no_item_timer = 0;
        self.locked = true;
    }

    /// The cloud went with no roll yet: the roulette runs down and waits for the result.
    pub fn cloud_unlock(&mut self) {
        if std::mem::take(&mut self.locked) {
            self.roll_timer = 0;
        }
    }

    /// Whether a rain cloud holds the roulette.
    pub fn locked(&self) -> bool {
        self.locked
    }

    /// One frame's timers, before the Fire press is read: the roll counts down and, at 0, shows
    /// the result (`VehPhysProc.c:580-607`); `noItemTimer` counts down and takes the item away at 1
    /// with no shots stacked (`:612-636`). Returns the item revealed this frame.
    pub fn tick(&mut self) -> Option<Item> {
        let mut revealed = None;
        if self.locked {
            self.roll_timer = CLOUD_ROLL_TIMER;
        } else if self.rolling {
            if self.roll_timer != 0 {
                self.roll_timer -= 1;
            } else if let Some(item) = self.result.take() {
                self.rolling = false;
                self.item = item;
                self.stack = item.stack();
                revealed = Some(item);
            }
        }
        if self.no_item_timer != 0 {
            if self.no_item_timer == 1 && self.stack == 0 {
                self.item = Item::None;
            }
            self.no_item_timer -= 1;
        }
        revealed
    }

    /// A Fire press this frame, after [`Held::tick`]. `can_fire` is the kart's side of the gate:
    /// driving or drifting, no TNT on the head, no clock on us (`VehPhysProc.c:804-861`). `wumpa`
    /// decides juiced at this moment.
    pub fn fire(&mut self, wumpa: u8, can_fire: bool) -> Fire {
        if !can_fire {
            return Fire::Nothing;
        }
        if self.bomb_out {
            self.bomb_out = false;
            return Fire::Detonate;
        }
        if self.shield_up {
            self.shield_up = false;
            return Fire::ShieldShot;
        }
        // Under the cloud the press does stop the roll, but the cloud sets it back to 5 before
        // the next frame (`VehPhysProc.c:857-858`, `RB_MaskShieldCloud.c:550`).
        if self.locked {
            return Fire::Nothing;
        }
        if self.rolling || self.item == Item::None || self.no_item_timer != 0 {
            // `CheckItemRollTimer`: any refused press stops a roll that has spun 20 frames.
            if self.rolling && self.roll_timer < FAST_STOP_BELOW && self.roll_timer != 0 {
                self.roll_timer = 0;
                return Fire::StopRoulette;
            }
            return Fire::Nothing;
        }
        if self.stack != 0 {
            self.stack -= 1;
            self.no_item_timer = STACK_GAP_FRAMES;
        } else {
            self.no_item_timer = LINGER_FRAMES;
        }
        let shot = Shot {
            item: self.item.single(),
            juiced: juiced(wumpa),
        };
        match shot.item {
            Item::Bomb => self.bomb_out = true,
            Item::Shield => self.shield_up = true,
            _ => {}
        }
        Fire::Launch(shot)
    }

    /// Our bomb exploded on its own (`RB_MovingExplosive.c:313-317` clears `instBombThrow`).
    pub fn bomb_gone(&mut self) {
        self.bomb_out = false;
    }

    /// Our shield popped or ran out.
    pub fn shield_gone(&mut self) {
        self.shield_up = false;
    }

    pub fn bomb_out(&self) -> bool {
        self.bomb_out
    }

    pub fn shield_up(&self) -> bool {
        self.shield_up
    }

    /// The item held (the stacked id while shots remain), `None` while spinning or empty.
    pub fn item(&self) -> Item {
        self.item
    }

    /// Shots left of the held item.
    pub fn shots_left(&self) -> u8 {
        match self.item {
            Item::None => 0,
            _ if self.no_item_timer != 0 && self.stack == 0 => 0,
            item if item.stack() != 0 => self.stack,
            _ => 1,
        }
    }

    pub fn view(&self) -> HeldView {
        if self.rolling {
            HeldView::Spinning
        } else if self.item == Item::None {
            HeldView::Empty
        } else if self.no_item_timer != 0 && self.stack == 0 {
            HeldView::Fading(self.item)
        } else {
            HeldView::Item(self.item)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ticks until the result shows; returns the frames it took.
    fn spin(held: &mut Held, press_at: Option<u32>) -> u32 {
        for frame in 1..200 {
            if held.tick().is_some() {
                return frame;
            }
            if press_at.is_some_and(|p| frame >= p) {
                held.fire(0, true);
            }
        }
        panic!("never revealed");
    }

    #[test]
    fn the_roulette_spins_90_frames() {
        let mut held = Held::new();
        held.start_roulette();
        held.set_result(Item::Mask);
        assert_eq!(held.view(), HeldView::Spinning);
        // 90 frames counting down, the result on the 91st.
        assert_eq!(spin(&mut held, None), 91);
        assert_eq!(held.view(), HeldView::Item(Item::Mask));
    }

    #[test]
    fn fire_stops_it_after_20_frames() {
        // Pressed from the first frame: ignored until the timer is below 70 (frame 21), and the
        // result shows on the next frame.
        let mut held = Held::new();
        held.start_roulette();
        held.set_result(Item::Tnt);
        assert_eq!(spin(&mut held, Some(1)), 22);
        // A press at frame 50 stops it at 51.
        let mut held = Held::new();
        held.start_roulette();
        held.set_result(Item::Tnt);
        assert_eq!(spin(&mut held, Some(50)), 51);
    }

    #[test]
    fn the_result_waits_for_the_server() {
        let mut held = Held::new();
        held.start_roulette();
        for _ in 0..120 {
            assert_eq!(held.tick(), None);
        }
        held.set_result(Item::Turbo);
        assert_eq!(held.tick(), Some(Item::Turbo));
    }

    #[test]
    fn three_shots_five_frames_apart() {
        let mut held = Held::new();
        held.start_roulette();
        held.set_result(Item::Missile3);
        while held.tick().is_none() {}
        let mut shots = Vec::new();
        for frame in 0..60 {
            if frame > 0 {
                held.tick();
            }
            // Fire held down every frame.
            if let Fire::Launch(shot) = held.fire(10, true) {
                assert_eq!(shot.item, Item::Missile);
                assert!(shot.juiced);
                shots.push(frame);
            }
        }
        assert_eq!(shots, [0, 5, 10]);
        // The last shot's 5-frame gap ends the item.
        assert_eq!(held.view(), HeldView::Empty);
    }

    #[test]
    fn a_single_item_lingers_30_frames() {
        let mut held = Held::new();
        held.start_roulette();
        held.set_result(Item::Beaker);
        while held.tick().is_none() {}
        assert_eq!(held.shots_left(), 1);
        assert_eq!(
            held.fire(3, true),
            Fire::Launch(Shot {
                item: Item::Beaker,
                juiced: false
            })
        );
        for frame in 1..=30 {
            assert_eq!(held.view(), HeldView::Fading(Item::Beaker), "{frame}");
            assert_eq!(held.fire(3, true), Fire::Nothing);
            held.tick();
        }
        assert_eq!(held.view(), HeldView::Empty);
    }

    #[test]
    fn a_second_press_detonates_the_bomb() {
        let mut held = Held::new();
        held.start_roulette();
        held.set_result(Item::Bomb3);
        while held.tick().is_none() {}
        assert!(matches!(held.fire(0, true), Fire::Launch(_)));
        for _ in 0..5 {
            held.tick();
        }
        assert_eq!(held.fire(0, true), Fire::Detonate);
        assert!(matches!(held.fire(0, true), Fire::Launch(_)));
        assert_eq!(held.shots_left(), 1);
    }

    #[test]
    fn a_rain_cloud_holds_the_roulette_until_the_new_roll() {
        let mut held = Held::new();
        held.start_roulette();
        held.set_result(Item::Bomb3);
        while held.tick().is_none() {}
        held.cloud_lock();
        assert_eq!(held.view(), HeldView::Spinning);
        assert_eq!(held.shots_left(), 0);
        assert!(!held.can_take_crate());
        // The cloud's 240 frames: Fire never stops it, nothing shows.
        for _ in 0..240 {
            assert_eq!(held.tick(), None);
            assert_eq!(held.fire(10, true), Fire::Nothing);
        }
        held.set_result(Item::Mask);
        assert!(!held.locked());
        assert_eq!(held.tick(), Some(Item::Mask));
        assert!(matches!(held.fire(0, true), Fire::Launch(_)));
        // A cloud gone with no roll yet: the roulette waits for it, then shows it.
        let mut held = Held::new();
        held.start_roulette();
        held.set_result(Item::Tnt);
        held.cloud_lock();
        assert_eq!(held.tick(), None, "the crate's roll is dropped");
        held.cloud_unlock();
        assert_eq!(held.tick(), None);
        held.set_result(Item::Turbo);
        assert_eq!(held.tick(), Some(Item::Turbo));
    }

    #[test]
    fn blocked_presses_do_nothing() {
        let mut held = Held::new();
        held.start_roulette();
        held.set_result(Item::Shield);
        while held.tick().is_none() {}
        assert_eq!(held.fire(0, false), Fire::Nothing);
        assert!(matches!(held.fire(0, true), Fire::Launch(_)));
        held.tick();
        assert_eq!(held.fire(0, true), Fire::ShieldShot);
    }
}
