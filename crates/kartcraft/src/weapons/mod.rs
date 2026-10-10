//! CTR's weapons in the kart: the crates and Wumpa fruit of a race, the held item and its
//! roulette ([`ctr_weapons::Held`]), the shots in flight ([`ctr_weapons::Field`], stepped at
//! CTR's 30 Hz in lockstep with our kart over the session's own collision), and every kart's side
//! of a hit: ours through `ctr-kart`'s [`Kart::hit`], other drivers' through a ghost kart of
//! theirs that shows their shield, mask, TNT, tumble and squish.
//!
//! [`Weapons`] speaks the protocol of `docs/kartcraft-weapons-protocol.md` and nothing else: the
//! host hands it the server's lines ([`wire::Down`], [`Weapons::receive`]) and sends the lines it
//! queues ([`Weapons::outbox`]). The client sends them to the Kartcraft server; the offline demo
//! to a [`referee::Referee`] that rules as the server does. Shots launch when they are relayed,
//! ours included, so every client starts each one from the same place; the victim's own client
//! decides a hit and reports it, and everyone shows it from the relay.

pub mod look;
pub mod referee;
pub mod wire;

use std::collections::{HashMap, VecDeque};
use std::sync::OnceLock;

use benilla_assets::coords::{bevy_to_wow, wow_to_bevy};
use bevy::prelude::*;
use ctr_kart::{Collision, Kart};
use ctr_weapons::held::ROLL_FRAMES;
use ctr_weapons::projectile::TNT_BLOWUP_RADIUS;
use ctr_weapons::{
    Body, Bomb, Field, Fire, Held, HitKind, KartId, Mine, MineKind, Missile, Outcome, ShotKey,
    WarpOrb,
};

pub use ctr_weapons::{HeldView, Item};

use crate::sound::{KartAudio, Sfx, WeaponLoops};
use crate::{from_lu, heading_of, lu_per_yard, to_lu, yaw_of};
use wire::{Aim, CrateKind, Down, Up};

/// How near (yards, in the ground plane) our kart takes a crate: CTR breaks it when the kart's
/// sphere touches its hitbox (`COLL.c:2186-2236`); the server allows 6.
const TOUCH_YARDS: f32 = 2.5;
const TOUCH_HEIGHT: f32 = 3.0;
/// A claim the server has not answered is asked again after this many frames.
const CLAIM_RETRY: u64 = 30;
/// A crate regrows from nothing +0x100 a frame (`231/RB_Crate.c:48-75`): 16 frames.
const CRATE_GROW: u64 = 16;
/// A fruit reaches the counter every 5 frames (`UI/UI_RenderFrame.c:212-257`).
const CREDIT_FRAMES: u64 = 5;
/// Frames past the spin's end that a roulette waits for the server's roll before it gives up.
const RESULT_WAIT: u64 = 60;
/// Frames our shield or bomb waits for its relay before the held item lets go of it.
const RELAY_WAIT: u32 = 60;
/// The rain cloud's 7680 ms (`231/RB_MaskShieldCloud.c:466-641`).
const CLOUD_FRAMES: u64 = 240;
/// Effects (explosions, debris) are kept this many frames; the longest, `warpedburst`, plays 30.
const EFFECT_FRAMES: u64 = 32;
/// Weapons farther than this (yards) are not heard.
const HEARD_YARDS: f32 = 80.0;
/// A TNT thrown off the head flies up at 48 lu a frame, falls 4 a frame² to −96, and goes off
/// where it lands (`231/RB_TNT.c:5-66`).
const THROWN_VY: f32 = 48.0;
const THROWN_GRAVITY: f32 = 4.0;
const THROWN_TERMINAL: f32 = 96.0;
/// A racer that sent a self-inflicted blast within this many frames of carrying a TNT blew it up.
const TNT_RECENT: u64 = 45;
/// A burst is drawn 0x30 lu under the weapon (`231/RB_Burst.c:203-206`).
const BURST_DROP: f32 = 48.0;

/// One crate spot of the race.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Crate {
    pub kind: CrateKind,
    /// The ground under it, Bevy world.
    pub at: Vec3,
    pub up: bool,
    /// When we asked for it, frames, until the server answers.
    claimed: Option<u64>,
    /// When it came back, frames: it regrows from there.
    back_at: u64,
}

/// What a kart wears: its shield, its mask, a TNT on its head (height in lu over its origin
/// and frames since it sat down), a rain cloud.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Look {
    pub shield: Option<ctr_kart::Shield>,
    /// When the shield went up, frames, for its grow.
    pub shield_since: u64,
    pub masked: bool,
    pub tnt: Option<(f32, i32)>,
    pub cloud: bool,
}

impl Look {
    fn of(kart: &Kart, was: &Look, frame: u64, cloud: bool) -> Look {
        let shield = kart.shield();
        Look {
            shield,
            shield_since: if shield.is_some() && was.shield.is_none() {
                frame
            } else {
                was.shield_since
            },
            masked: kart.masked(),
            tnt: kart.tnt_height().map(|h| (h, tnt_elapsed(kart))),
            cloud,
        }
    }
}

/// Frames the TNT has sat on the head (0 while it is thrown on).
fn tnt_elapsed(kart: &Kart) -> i32 {
    kart.tnt_fuse()
        .map_or(0, |left| (TNT_FUSE_FRAMES - left).max(0))
}

/// `numFramesOnHead` counts to 90 (`231/RB_TNT.c:194-239`).
const TNT_FUSE_FRAMES: i32 = 90;
/// Where the TNT sits over the kart's origin, lu: Crash's head (`RB_TNT.c:78`).
const TNT_HEAD: f32 = 32.0;
/// The fuse honks at these frames on the head (`RB_TNT.c:208`).
const HONKS: [i32; 6] = [0, 20, 40, 60, 70, 80];

/// A hit's look on another driver's kart, in its own frame: the tumble's turn and its pivot
/// (yards), and the squash and squish.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reaction {
    pub rotation: Quat,
    pub offset: Vec3,
    pub scale: Vec3,
}

/// Another driver's kart, as the weapons see it.
pub struct Other {
    /// Its origin (bottom centre) and facing, Bevy world and yaw.
    pub position: Vec3,
    pub yaw: f32,
    /// Race position, 0 the leader.
    pub rank: u8,
    /// Whose kart (`crash`, `cortex`, ...), for its mask.
    pub racer: String,
    /// Its hits are decided here (the demo's dummies) rather than by its own client.
    local: bool,
    /// A kart of its own on a flat plane, standing still, for its weapon state and its look.
    ghost: Kart,
    home: Vec3,
    look: Look,
    /// The last frame it carried a TNT.
    tnt_at: Option<u64>,
    seen: bool,
}

/// What a shot is, for its look and its sounds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShotInfo {
    pub item: Item,
    pub juiced: bool,
    /// Frame launched, and the facing it left with (4096ths).
    pub born: u64,
    pub heading: i32,
    pub backward: bool,
}

/// A one-off on screen: an explosion, crate debris, a shockwave.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Effect {
    pub id: u64,
    /// A `weapons.json` model.
    pub model: &'static str,
    /// Bevy world.
    pub at: Vec3,
    pub born: u64,
    /// Turned a quarter about its facing: CTR's second burst (`231/RB_Burst.c:250-266`).
    pub turned: bool,
}

/// A TNT thrown off a head, on its way down (visual: where it lands it only goes off).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Thrown {
    pub id: u64,
    /// lu from the session origin, and its fall speed.
    pub at: Vec3,
    vy: f32,
    born: u64,
}

#[derive(Resource, Default)]
pub struct Weapons {
    /// Our racer: our low guid on a server, the demo's own id offline.
    pub me: KartId,
    /// Our race position, 0 the leader.
    pub rank: u8,
    /// The offline referee also takes `W D` and `W Z` (a bomb's remote detonation, a fired
    /// shield), which the server's protocol has no line for: online those presses do nothing.
    pub extended: bool,
    /// Lines for the server, by sender (ours, or a demo dummy's).
    pub outbox: Vec<(KartId, Up)>,
    /// The fraction of a tick drawn past the last, for drawing between ticks.
    pub blend: f32,
    /// The camera we drive with, for the missile's lock-on.
    pub camera: Option<Transform>,
    field: Field,
    held: Held,
    /// The server's Wumpa total, and the count the HUD has credited so far.
    wumpa_total: u8,
    wumpa: u8,
    credited_at: u64,
    crates: Vec<Crate>,
    others: HashMap<KartId, Other>,
    shots: HashMap<ShotKey, ShotInfo>,
    /// Last tick's positions of the shots, lu, to draw between.
    previous: HashMap<ShotKey, Vec3>,
    next_shot: u16,
    inbox: VecDeque<Down>,
    fire: bool,
    backward: bool,
    /// Frames into the roulette, and how long our shield and bomb have waited for their relay.
    rolling_for: u64,
    shield_wait: Option<u32>,
    bomb_wait: Option<u32>,
    effects: Vec<Effect>,
    thrown: Vec<Thrown>,
    clouds: HashMap<KartId, u64>,
    ours: Look,
    honked: Option<i32>,
    origin: Option<Vec3>,
    /// Ticks since the start, CTR's frame counter.
    frame: u64,
    serial: u64,
    sounds: Vec<(Sfx, f32)>,
    loops: WeaponLoops,
    /// Where our kart was at the last tick, Bevy world.
    at: Vec3,
}

/// CTR's damage types as `ctr-kart` takes them.
fn kart_hit(kind: HitKind) -> ctr_kart::HitKind {
    match kind {
        HitKind::SpinOut => ctr_kart::HitKind::SpinOut,
        HitKind::Blasted => ctr_kart::HitKind::Blasted,
        HitKind::Squished => ctr_kart::HitKind::Squished,
        HitKind::TntOnHead => ctr_kart::HitKind::TntOnHead,
    }
}

/// A flat plane for the ghost karts to stand on.
fn plane() -> &'static Collision {
    static PLANE: OnceLock<Collision> = OnceLock::new();
    PLANE.get_or_init(|| {
        let r = 4096.0;
        Collision::new(
            [
                [[-r, 0.0, -r], [-r, 0.0, r], [r, 0.0, r]],
                [[-r, 0.0, -r], [r, 0.0, r], [r, 0.0, -r]],
            ],
            512.0,
        )
    })
}

/// Aku Aku guards Crash, Coco, Polar, Pura and Penta; Uka Uka the rest (`MaskBoolGoodGuy`,
/// `Vehicle/VehPickupItem.c:178-190`).
pub fn good_guy(racer: &str) -> bool {
    matches!(racer, "" | "crash" | "coco" | "polar" | "pura" | "pen")
}

impl Weapons {
    /// Everything of a race goes: crates, shots, the held item, the fruit.
    pub fn clear(&mut self) {
        *self = Weapons {
            me: self.me,
            extended: self.extended,
            next_shot: self.next_shot,
            frame: self.frame,
            serial: self.serial,
            ..Default::default()
        };
    }

    /// Whether there is anything of a race: crates laid out or an item.
    pub fn live(&self) -> bool {
        !self.crates.is_empty() || self.held.view() != HeldView::Empty || !self.field.is_empty()
    }

    /// A line from the server, handled at the next tick.
    pub fn receive(&mut self, down: Down) {
        self.inbox.push_back(down);
    }

    /// This frame's ○ (and whether the stick is held back), for the next tick.
    pub fn press(&mut self, fire: bool, backward: bool) {
        self.fire |= fire;
        if fire {
            self.backward = backward;
        }
    }

    /// Our Wumpa as the HUD counts it.
    pub fn wumpa(&self) -> u8 {
        self.wumpa
    }

    pub fn held(&self) -> HeldView {
        self.held.view()
    }

    /// The item slot for the HUD: the roulette while it spins, the item with its shots when
    /// stacked, flickering once the last is gone (`UI/UI_Weapon.c:85-140`).
    pub fn hud_item(&self) -> Option<(u8, u8)> {
        match self.held.view() {
            HeldView::Empty => None,
            HeldView::Spinning => Some((0x10, 0)),
            HeldView::Item(item) => {
                let count = if item.stack() != 0 {
                    self.held.shots_left()
                } else {
                    0
                };
                Some((item.id(), count))
            }
            HeldView::Fading(item) => (self.frame / 2).is_multiple_of(2).then_some((item.id(), 0)),
        }
    }

    pub fn crates(&self) -> &[Crate] {
        &self.crates
    }

    /// A crate's size, 0 gone to 1 full, as it regrows.
    pub fn crate_scale(&self, c: &Crate) -> f32 {
        if !c.up {
            return 0.0;
        }
        ((self.frame.saturating_sub(c.back_at)) as f32 / CRATE_GROW as f32).min(1.0)
    }

    pub fn frame(&self) -> u64 {
        self.frame
    }

    pub fn origin(&self) -> Option<Vec3> {
        self.origin
    }

    /// The shots in flight, with where they were a tick ago (lu).
    pub fn shots(&self) -> impl Iterator<Item = (ShotKey, &Body, ShotInfo, Vec3)> {
        self.field.iter().filter_map(|(key, body)| {
            let info = *self.shots.get(&key)?;
            let was = self.previous.get(&key).copied().unwrap_or(body.position());
            Some((key, body, info, was))
        })
    }

    pub fn effects(&self) -> &[Effect] {
        &self.effects
    }

    pub fn thrown(&self) -> &[Thrown] {
        &self.thrown
    }

    /// What our kart wears.
    pub fn our_look(&self) -> Look {
        self.ours
    }

    pub fn others(&self) -> impl Iterator<Item = (KartId, &Other, Look)> {
        self.others.iter().map(|(id, o)| (*id, o, o.look))
    }

    /// The look of a hit on another driver's kart: a blast's tumble and a squish (its spin-out
    /// comes with the state it relays); a demo dummy's whole reaction, its bounce included.
    pub fn reaction(&self, id: KartId) -> Option<Reaction> {
        let o = self.others.get(&id)?;
        let pose = o.ghost.pose();
        let lift = |lu: Vec3| lu / lu_per_yard();
        if o.local {
            return Some(Reaction {
                rotation: Quat::from_mat3(&pose.basis).normalize(),
                offset: lift(pose.position - o.home + pose.offset),
                scale: pose.scale,
            });
        }
        o.ghost.reaction()?;
        let turn = (o.ghost.drawn_yaw_offset() * 4096.0 / std::f32::consts::TAU).round() as i32;
        let facing = ctr_kart::kart::rot_axis_angle(Vec3::Y, (pose.heading + turn) & 0xFFF);
        Some(Reaction {
            rotation: Quat::from_mat3(&(facing.transpose() * pose.basis)).normalize(),
            offset: lift(facing.transpose() * pose.offset),
            scale: pose.scale,
        })
    }

    /// Starts this frame's account of the other karts: each one still there is [`Self::other`]ed,
    /// then [`Self::end_others`] drops the rest.
    pub fn begin_others(&mut self) {
        for o in self.others.values_mut() {
            o.seen = false;
        }
    }

    /// Another driver's kart this frame: its origin and yaw (Bevy), race position and racer.
    /// `local`: its hits are decided here (a demo dummy).
    pub fn other(
        &mut self,
        id: KartId,
        position: Vec3,
        yaw: f32,
        rank: u8,
        racer: &str,
        local: bool,
    ) {
        let o = self.others.entry(id).or_insert_with(|| {
            let mut ghost = Kart::new(crate::class(), Vec3::Y * 4.0, 0);
            ghost.seed(id as u16 ^ 0x5A5A);
            Other {
                position,
                yaw,
                rank,
                racer: String::new(),
                local,
                home: ghost.pose().position,
                ghost,
                look: Look::default(),
                tnt_at: None,
                seen: true,
            }
        });
        o.position = position;
        o.yaw = yaw;
        o.rank = rank;
        o.local = local;
        o.seen = true;
        if o.racer != racer {
            o.racer = racer.to_string();
        }
    }

    pub fn end_others(&mut self) {
        self.others.retain(|_, o| o.seen);
    }

    /// This frame's weapon sounds into the kart's audio.
    pub fn feed(&mut self, audio: &mut KartAudio) {
        audio.cues.extend(
            self.sounds
                .drain(..)
                .map(|(s, gain)| crate::sound::Cue::Sfx(s, gain)),
        );
        audio.weapons = self.loops;
    }

    fn sound(&mut self, sfx: Sfx, at: Vec3) {
        let d = at.distance(self.at);
        let gain = (1.0 - d / HEARD_YARDS).clamp(0.0, 1.0);
        if gain > 0.0 {
            self.sounds.push((sfx, gain));
        }
    }

    fn effect(&mut self, model: &'static str, at: Vec3) {
        self.serial += 1;
        self.effects.push(Effect {
            id: self.serial,
            model,
            at,
            born: self.frame,
            turned: false,
        });
    }

    /// `RB_Burst_Init` (`231/RB_Burst.c:209-282`): a bomb's, missile's or shield's blast is two
    /// `warpedburst`s, one turned a quarter, and the red shockwave, 0x30 below the weapon.
    fn burst(&mut self, at: Vec3) {
        let at = at - Vec3::Y * (BURST_DROP / lu_per_yard());
        self.effect("warpedburst", at);
        self.effect("warpedburst", at);
        if let Some(e) = self.effects.last_mut() {
            e.turned = true;
        }
        self.effect("shockwave_red", at);
    }

    fn next_shot(&mut self) -> u16 {
        self.next_shot = self.next_shot.checked_add(1).unwrap_or(1);
        self.next_shot
    }

    /// One 30 Hz frame, after our kart's tick: the server's lines, the held item and ○, the other
    /// karts' ghosts, our crate touches, and every shot against every kart.
    pub fn tick(&mut self, world: &Collision, kart: &mut Kart, origin: Vec3) {
        self.frame += 1;
        if self.origin != Some(origin) {
            // A new session measures from elsewhere: what flew in the old one is gone.
            self.field = Field::new();
            self.previous.clear();
            self.thrown.clear();
            self.origin = Some(origin);
        }
        let pose = kart.pose();
        self.at = from_lu(pose.position, origin);
        while let Some(down) = self.inbox.pop_front() {
            self.apply(down, world, kart, origin);
        }
        self.our_kart(kart, origin);
        self.held_item(kart, origin);
        self.ghosts(kart, origin);
        self.touch_crates();
        self.step_field(world, kart, origin);
        self.fly_thrown(world, origin);
        self.credit_wumpa();
        let cloud = self.clouds.get(&self.me).is_some_and(|&t| t > self.frame);
        self.ours = Look::of(kart, &self.ours, self.frame, cloud);
        self.effects
            .retain(|e| self.frame.saturating_sub(e.born) < EFFECT_FRAMES);
        let frame = self.frame;
        self.clouds.retain(|_, until| *until > frame);
    }

    fn apply(&mut self, down: Down, world: &Collision, kart: &mut Kart, origin: Vec3) {
        match down {
            Down::Layout { first, crates } => {
                for (i, c) in crates.into_iter().enumerate() {
                    let at = wow_to_bevy(c.at);
                    let spot = Crate {
                        kind: c.kind,
                        at,
                        up: true,
                        claimed: None,
                        back_at: 0,
                    };
                    let i = first + i;
                    if i >= self.crates.len() {
                        self.crates.resize(i + 1, spot);
                    }
                    self.crates[i] = spot;
                }
            }
            Down::Taken { index, by } => {
                let Some(c) = self.crates.get_mut(index) else {
                    return;
                };
                let was_up = c.up;
                c.up = false;
                c.claimed = None;
                let (kind, at) = (c.kind, c.at);
                if was_up {
                    if kind == CrateKind::Wumpa {
                        self.sound(Sfx::Wumpa, at);
                    } else {
                        self.effect("crate_explosion", at);
                        self.sound(Sfx::CrateBreak, at);
                    }
                }
                // Our weapon crate starts the roulette now; the roll comes as `I`
                // (`231/RB_Crate.c:152-226`: holding an item, the crate gives nothing).
                if by == self.me {
                    info!("kart weapons: took crate {index} ({})", kind.code());
                }
                if by == self.me && kind == CrateKind::Weapon && self.held.can_take_crate() {
                    self.held.start_roulette();
                    self.rolling_for = 0;
                }
            }
            Down::Back { index } => {
                if let Some(c) = self.crates.get_mut(index) {
                    c.up = true;
                    c.back_at = self.frame;
                }
            }
            Down::Item { item, .. } => {
                if self.held.view() != HeldView::Spinning {
                    self.held.start_roulette();
                    self.rolling_for = 0;
                }
                self.held.set_result(item);
            }
            Down::Wumpa(n) => self.wumpa_total = n.min(ctr_weapons::item::WUMPA_MAX),
            Down::Shot(s) => self.launch(s, world, kart, origin),
            Down::Hit {
                victim,
                shooter,
                shot,
                kind,
                wumpa,
            } => self.relayed_hit(victim, shooter, shot, kind, wumpa, kart, origin),
            Down::Detonate { shooter } => {
                self.field.detonate(shooter);
            }
            Down::ShieldShot { shooter, shot, aim } => {
                let key = ShotKey {
                    owner: shooter,
                    shot,
                };
                let at = to_lu(wow_to_bevy(aim.at), origin);
                let heading = heading_of(aim.yaw);
                if shooter != self.me {
                    if let Some(o) = self.others.get_mut(&shooter) {
                        o.ghost.fire_shield();
                    }
                }
                self.field
                    .launch(key, Body::Bomb(Bomb::shield(shooter, at, heading)));
                self.shots.insert(
                    key,
                    ShotInfo {
                        item: Item::Shield,
                        juiced: false,
                        born: self.frame,
                        heading,
                        backward: false,
                    },
                );
                self.sound(Sfx::ShieldPop, wow_to_bevy(aim.at));
            }
        }
    }

    /// A relayed shot (`VehPickupItem_ShootNow`, `Vehicle/VehPickupItem.c:579-1535`): the
    /// self-applied items on the shooter's kart (the clock on everyone else's), the rest into
    /// the field from where the shooter fired.
    fn launch(&mut self, s: wire::Shot, world: &Collision, kart: &mut Kart, origin: Vec3) {
        let key = ShotKey {
            owner: s.shooter,
            shot: s.shot,
        };
        let from = wow_to_bevy(s.aim.at);
        let at = to_lu(from, origin);
        let heading = heading_of(s.aim.yaw);
        let target = (s.aim.target != 0).then_some(s.aim.target);
        let mine = s.shooter == self.me;
        let item = s.item.single();
        let j = s.juiced;
        info!(
            "kart weapons: {} shot {} by {}{}",
            s.item.name(),
            s.shot,
            s.shooter,
            if j { ", juiced" } else { "" }
        );
        let body = match item {
            Item::Turbo => {
                if mine {
                    kart.fire_turbo(j);
                }
                None
            }
            Item::Shield => {
                if mine {
                    kart.shield_up(j);
                    self.shield_wait = None;
                } else if let Some(o) = self.others.get_mut(&s.shooter) {
                    o.ghost.shield_up(j);
                }
                self.sound(Sfx::Shield, from);
                None
            }
            Item::Mask => {
                let racer = if mine {
                    crate::model::our_kart().to_string()
                } else {
                    self.others
                        .get(&s.shooter)
                        .map(|o| o.racer.clone())
                        .unwrap_or_default()
                };
                if mine {
                    kart.mask(j);
                } else if let Some(o) = self.others.get_mut(&s.shooter) {
                    o.ghost.mask(j);
                }
                let sfx = if good_guy(&racer) { Sfx::Aku } else { Sfx::Uka };
                self.sound(sfx, from);
                None
            }
            Item::Clock => {
                // Every other kart spins and, if that takes, is slowed by its rank
                // (`VehPickupItem.c:855-908`); the victim's client reports the spin.
                self.sounds.push((Sfx::Clock, 1.0));
                if !mine && kart.clock(j, ctr_kart::clock_slowdown(self.rank)) {
                    self.outbox.push((
                        self.me,
                        Up::Hit {
                            shooter: s.shooter,
                            shot: s.shot,
                            kind: HitKind::SpinOut,
                        },
                    ));
                }
                let frame = self.frame;
                for (&id, o) in &mut self.others {
                    if o.local
                        && id != s.shooter
                        && o.ghost.clock(j, ctr_kart::clock_slowdown(o.rank))
                    {
                        o.tnt_at = o.tnt_at.or(o.ghost.tnt_fuse().map(|_| frame));
                        self.outbox.push((
                            id,
                            Up::Hit {
                                shooter: s.shooter,
                                shot: s.shot,
                                kind: HitKind::SpinOut,
                            },
                        ));
                    }
                }
                None
            }
            Item::Bomb => {
                self.sound(Sfx::BombLaunch, from);
                Some(Body::Bomb(Bomb::new(
                    s.shooter,
                    at,
                    heading,
                    j,
                    s.aim.backward,
                    target,
                )))
            }
            Item::Missile => {
                self.sound(Sfx::MissileLaunch, from);
                Some(Body::Missile(Missile::new(
                    s.shooter, at, heading, j, target,
                )))
            }
            Item::WarpOrb => {
                self.sound(Sfx::WarpLaunch, from);
                Some(Body::WarpOrb(WarpOrb::new(
                    s.shooter, at, heading, j, target,
                )))
            }
            Item::Tnt | Item::Beaker => {
                self.sound(Sfx::MineDrop, from);
                MineKind::from_shot(item, j)
                    .map(|kind| Body::Mine(Mine::drop(s.shooter, kind, at, world)))
            }
            _ => None,
        };
        let Some(body) = body else {
            return;
        };
        if self.field.launch(key, body) {
            self.shots.insert(
                key,
                ShotInfo {
                    item,
                    juiced: j,
                    born: self.frame,
                    heading,
                    backward: s.aim.backward,
                },
            );
            if mine && item == Item::Bomb {
                self.bomb_wait = None;
            }
        }
    }

    /// The server's word on a hit (`W H`): the victim's look, our Wumpa, and the shot's end.
    #[allow(clippy::too_many_arguments)]
    fn relayed_hit(
        &mut self,
        victim: KartId,
        shooter: KartId,
        shot: u16,
        kind: HitKind,
        wumpa: u8,
        kart: &mut Kart,
        origin: Vec3,
    ) {
        info!(
            "kart weapons: {victim} hit ({}) by {shooter}'s shot {shot}, {wumpa} Wumpa left",
            kind.code()
        );
        if victim == self.me {
            self.wumpa_total = wumpa;
            self.wumpa = self.wumpa.min(wumpa);
        } else if let Some(o) = self.others.get_mut(&victim) {
            // A demo dummy took it here already; another driver's own client did.
            if !o.local {
                let had_tnt = o.ghost.tnt_fuse().is_some()
                    || o.tnt_at
                        .is_some_and(|t| self.frame.saturating_sub(t) < TNT_RECENT);
                o.ghost.hit(kart_hit(kind));
                // Its TNT went off on its head: the 320 lu blast reaches us too.
                if shooter == victim && shot == 0 && kind == HitKind::Blasted && had_tnt {
                    o.tnt_at = None;
                    let at = to_lu(o.position, origin);
                    self.tnt_blast(victim, at, kart, origin);
                }
            }
        }
        if shot == 0 {
            return;
        }
        let key = ShotKey {
            owner: shooter,
            shot,
        };
        // A juiced warp orb flies on through its victims; anything else ends here.
        let goes_on = matches!(self.field.get(key), Some(Body::WarpOrb(_)))
            && self.shots.get(&key).is_some_and(|i| i.juiced);
        if !goes_on {
            if let Some(body) = self.field.remove(key) {
                self.ended(key, from_lu(body.position(), origin));
            }
        }
    }

    /// A TNT going off on `carrier`'s head at `at` (lu): 320 lu to everyone, the carrier's own
    /// blast its own (`RB_TNT.c:194-239`, `RB_Blowup.c:131-157`). We report our own blast as
    /// self-inflicted: the server only knows shots, and a TNT on a head is no longer one.
    fn tnt_blast(&mut self, carrier: KartId, at: Vec3, kart: &mut Kart, origin: Vec3) {
        let world_at = from_lu(at, origin);
        self.effect("crate_explosion", world_at);
        self.effect("shockwave_red", world_at);
        self.sound(Sfx::TntExplode, world_at);
        let reach = TNT_BLOWUP_RADIUS * TNT_BLOWUP_RADIUS;
        if carrier != self.me
            && (kart.pose().position - at).length_squared() < reach
            && kart.hit(ctr_kart::HitKind::Blasted)
        {
            self.outbox.push((
                self.me,
                Up::Hit {
                    shooter: self.me,
                    shot: 0,
                    kind: HitKind::Blasted,
                },
            ));
        }
        for (&id, o) in &mut self.others {
            let near = (to_lu(o.position, origin) - at).length_squared() < reach;
            if o.local && id != carrier && near && o.ghost.hit(ctr_kart::HitKind::Blasted) {
                self.outbox.push((
                    id,
                    Up::Hit {
                        shooter: id,
                        shot: 0,
                        kind: HitKind::Blasted,
                    },
                ));
            }
        }
    }

    /// Our kart's own news this tick: a TNT on our head going off (a self-inflicted blast) or
    /// thrown off, and the fuse's honks.
    fn our_kart(&mut self, kart: &mut Kart, origin: Vec3) {
        let at = kart.pose().position;
        for e in kart.events.clone() {
            match e {
                ctr_kart::Event::TntExploded => {
                    if kart.hit(ctr_kart::HitKind::Blasted) {
                        self.outbox.push((
                            self.me,
                            Up::Hit {
                                shooter: self.me,
                                shot: 0,
                                kind: HitKind::Blasted,
                            },
                        ));
                    }
                    let me = self.me;
                    self.tnt_blast(me, at, kart, origin);
                }
                ctr_kart::Event::TntThrownOff => {
                    self.serial += 1;
                    self.thrown.push(Thrown {
                        id: self.serial,
                        at: at + Vec3::Y * TNT_HEAD,
                        vy: THROWN_VY,
                        born: self.frame,
                    });
                }
                _ => {}
            }
        }
        // The honks at frames 0, 20, 40, 60, 70 and 80 on the head.
        let elapsed = kart
            .tnt_fuse()
            .filter(|_| kart.tnt_height().is_some_and(|h| h <= TNT_HEAD))
            .map(|left| TNT_FUSE_FRAMES - left);
        if elapsed != self.honked {
            if elapsed.is_some_and(|e| HONKS.contains(&e)) {
                self.sounds.push((Sfx::TntHonk, 1.0));
            }
            self.honked = elapsed;
        }
    }

    /// The held item's frame (`Vehicle/VehPhysProc.c:576-646`), then ○ (`:801-905`).
    fn held_item(&mut self, kart: &mut Kart, origin: Vec3) {
        // The result lands with 0x5E, or 0x41 juiced (`VehPhysProc.c:594-606`).
        if self.held.tick().is_some() {
            let juiced = ctr_weapons::item::juiced(self.wumpa_total);
            let sfx = if juiced {
                Sfx::RouletteJuiced
            } else {
                Sfx::RouletteStop
            };
            self.sounds.push((sfx, 1.0));
        }
        if self.held.view() == HeldView::Spinning {
            self.rolling_for += 1;
            // No roll from the server: the crate gave nothing after all.
            if self.rolling_for > u64::from(ROLL_FRAMES) + RESULT_WAIT {
                self.held = Held::new();
            }
        }
        self.loops.roulette = self.held.view() == HeldView::Spinning;
        // Our shield and bomb live from their relay; one never relayed lets go, and so does a
        // shield that popped or ran out.
        for w in [&mut self.shield_wait, &mut self.bomb_wait]
            .into_iter()
            .flatten()
        {
            *w = w.saturating_sub(1);
        }
        if self.held.shield_up()
            && kart.shield().is_none()
            && self.shield_wait.is_none_or(|w| w == 0)
        {
            self.held.shield_gone();
            self.shield_wait = None;
        }
        if self.held.bomb_out() {
            let me = self.me;
            let rolling = self
                .field
                .iter()
                .any(|(_, b)| matches!(b, Body::Bomb(x) if x.owner() == me && !x.is_shield()));
            if !rolling && self.bomb_wait.is_none_or(|w| w == 0) {
                self.held.bomb_gone();
                self.bomb_wait = None;
            }
        }
        if !std::mem::take(&mut self.fire) {
            return;
        }
        // The server's protocol has no line to detonate a bomb or fire a shield: online those
        // presses do nothing.
        if (self.held.bomb_out() || self.held.shield_up()) && !self.extended {
            return;
        }
        let can = if self.held.shield_up() {
            kart.fire_shield()
        } else {
            kart.can_fire()
        };
        let pose = kart.pose();
        let aim = |target: KartId, backward: bool, dropped: bool| Aim {
            at: bevy_to_wow(from_lu(pose.position, origin)),
            yaw: yaw_of(pose.heading),
            target,
            backward,
            dropped,
        };
        match self.held.fire(self.wumpa_total, can) {
            Fire::Nothing | Fire::StopRoulette => {}
            Fire::Detonate => self.outbox.push((self.me, Up::Detonate)),
            Fire::ShieldShot => {
                let shot = self.next_shot();
                self.outbox.push((
                    self.me,
                    Up::ShieldShot {
                        shot,
                        aim: aim(0, false, false),
                    },
                ));
            }
            Fire::Launch(fired) => {
                let item = fired.item;
                // The bomb steers for the missile's lock-on too (`RB_MovingExplosive.c:129-143`);
                // the warp orb's target is the server's pick.
                let target = match item {
                    Item::Bomb | Item::Missile => self.lock_on().unwrap_or(0),
                    _ => 0,
                };
                let backward = matches!(item, Item::Bomb | Item::Missile) && self.backward;
                let dropped = matches!(item, Item::Tnt | Item::Beaker);
                match item {
                    Item::Shield => self.shield_wait = Some(RELAY_WAIT),
                    Item::Bomb => self.bomb_wait = Some(RELAY_WAIT),
                    _ => {}
                }
                let shot = self.next_shot();
                self.outbox.push((
                    self.me,
                    Up::Fire {
                        shot,
                        aim: aim(target, backward, dropped),
                    },
                ));
            }
        }
    }

    /// The missile's lock-on (`VehPickupItem_MissileGetTargetDriver`, `VehPickupItem.c:353-506`):
    /// of the karts inside our view (inset a little, as CTR insets 31 of 512 pixels), the
    /// nearest in the ground plane. With none on screen a human's missile flies straight.
    pub fn lock_on(&self) -> Option<KartId> {
        let cam = self.camera?;
        let to_view = cam.compute_affine().inverse();
        let tan_y = (crate::camera::FOV_Y * 0.5).tan() * 0.9;
        let tan_x = tan_y * 16.0 / 9.0;
        self.others
            .iter()
            .filter_map(|(&id, o)| {
                let p = to_view.transform_point3(o.position + Vec3::Y);
                let ahead = -p.z;
                let seen =
                    ahead > 1.0 && (p.x / ahead).abs() < tan_x && (p.y / ahead).abs() < tan_y;
                let d = o.position - self.at;
                seen.then_some((id, d.x * d.x + d.z * d.z))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(id, _)| id)
    }

    /// The ghosts' frame: each stands on its plane with no input, its weapon state and look
    /// running; a demo dummy's TNT goes off on its own head.
    fn ghosts(&mut self, kart: &mut Kart, origin: Vec3) {
        let frame = self.frame;
        let mut blasts = Vec::new();
        let mut pops = Vec::new();
        for (&id, o) in &mut self.others {
            o.ghost.tick(
                plane(),
                &ctr_kart::Input::default(),
                &ctr_kart::Modifiers::default(),
            );
            if o.ghost.tnt_fuse().is_some() {
                o.tnt_at = Some(frame);
            }
            for e in std::mem::take(&mut o.ghost.events) {
                match e {
                    ctr_kart::Event::TntExploded if o.local => {
                        blasts.push((id, to_lu(o.position, origin)));
                        if o.ghost.hit(ctr_kart::HitKind::Blasted) {
                            self.outbox.push((
                                id,
                                Up::Hit {
                                    shooter: id,
                                    shot: 0,
                                    kind: HitKind::Blasted,
                                },
                            ));
                        }
                    }
                    ctr_kart::Event::ShieldPopped | ctr_kart::Event::ShieldExpired => {
                        pops.push(o.position);
                    }
                    _ => {}
                }
            }
            let cloud = self.clouds.get(&id).is_some_and(|&t| t > frame);
            o.look = Look::of(&o.ghost, &o.look, frame, cloud);
        }
        for at in pops {
            self.sound(Sfx::ShieldPop, at);
        }
        for (id, at) in blasts {
            self.tnt_blast(id, at, kart, origin);
        }
    }

    /// Our crate touches: each crate up within reach is asked for (`W C`), and again if the
    /// server stays quiet.
    fn touch_crates(&mut self) {
        let frame = self.frame;
        let reach = TOUCH_YARDS * crate::kart_scale();
        for (i, c) in self.crates.iter_mut().enumerate() {
            let d = c.at - self.at;
            let near = Vec2::new(d.x, d.z).length() < reach && d.y.abs() < TOUCH_HEIGHT;
            let asked = c.claimed.is_some_and(|t| frame - t < CLAIM_RETRY);
            if c.up && near && !asked {
                c.claimed = Some(frame);
                self.outbox.push((self.me, Up::Claim(i)));
            }
        }
    }

    /// Every kart as the weapons meet it: ours, and the others at their ghosts' state.
    fn targets(&self, kart: &Kart, origin: Vec3) -> Vec<ctr_weapons::KartView> {
        let view = |id, position, k: &Kart, rank: u8| ctr_weapons::KartView {
            id,
            position,
            shielded: k.shield().is_some(),
            masked: k.masked(),
            tnt_on_head: k.tnt_fuse().is_some(),
            squished: k.squish_scale() == 0.0,
            rank: usize::from(rank),
        };
        let mut views = vec![view(self.me, kart.pose().position, kart, self.rank)];
        for (&id, o) in &self.others {
            let mut at = to_lu(o.position, origin);
            if o.local {
                at += o.ghost.pose().position - o.home;
            }
            views.push(view(id, at, &o.ghost, o.rank));
        }
        views
    }

    /// One frame of every shot (`Field::step`) and what came of it.
    fn step_field(&mut self, world: &Collision, kart: &mut Kart, origin: Vec3) {
        self.previous = self.field.iter().map(|(k, b)| (k, b.position())).collect();
        let views = self.targets(kart, origin);
        for (key, outcome) in self.field.step(world, &views) {
            self.outcome(key, outcome, kart, origin);
        }
        // The flight loops, as loud as the nearest of each.
        let mut loops = (0.0f32, 0.0f32, 0.0f32);
        let gain = |at: Vec3| (1.0 - at.distance(self.at) / HEARD_YARDS).clamp(0.0, 1.0);
        for (_, body) in self.field.iter() {
            let g = gain(from_lu(body.position(), origin));
            match body {
                Body::Missile(_) => loops.0 = loops.0.max(g),
                Body::WarpOrb(_) => loops.1 = loops.1.max(g),
                Body::Bomb(_) => loops.2 = loops.2.max(g),
                Body::Mine(_) => {}
            }
        }
        (self.loops.missile, self.loops.warp, self.loops.bomb) = loops;
    }

    fn outcome(&mut self, key: ShotKey, outcome: Outcome, kart: &mut Kart, origin: Vec3) {
        match outcome {
            Outcome::Hit { kart: id, kind } => {
                let report = |kind| Up::Hit {
                    shooter: key.owner,
                    shot: key.shot,
                    kind,
                };
                if id == self.me {
                    // Ours to decide (`VehPickState_NewState`): a mask, shield or the
                    // invincibility after a pop keep it off, and then nothing is reported.
                    if kart.hit(kart_hit(kind)) {
                        let kind = if kind == HitKind::TntOnHead && kart.tnt_fuse().is_none() {
                            HitKind::Blasted
                        } else {
                            kind
                        };
                        self.outbox.push((self.me, report(kind)));
                    }
                } else if let Some(o) = self.others.get_mut(&id) {
                    if o.local {
                        if o.ghost.hit(kart_hit(kind)) {
                            let kind = if kind == HitKind::TntOnHead && o.ghost.tnt_fuse().is_none()
                            {
                                HitKind::Blasted
                            } else {
                                kind
                            };
                            self.outbox.push((id, report(kind)));
                        }
                    } else if o.ghost.shield().is_some() {
                        // Its own client decides; a shield pops there as it does here.
                        o.ghost.hit(kart_hit(kind));
                    }
                }
            }
            Outcome::RainCloud { kart: id } => {
                self.clouds.insert(id, self.frame + CLOUD_FRAMES);
            }
            Outcome::Exploded { at, .. } => self.ended(key, from_lu(at, origin)),
            Outcome::Destroyed { at } => self.ended(key, from_lu(at, origin)),
            Outcome::Expired => {
                let at = self.previous.get(&key).copied().unwrap_or_default();
                self.ended(key, from_lu(at, origin));
            }
        }
    }

    /// A shot's end on screen and in the ears: a burst for a bomb, missile or shield
    /// (`RB_Burst_Init`), crate debris and a red or green ring for TNT and Nitro
    /// (`RB_Blowup_Init`, `231/RB_Blowup.c:80-130`), glass for a beaker, the orb's death sound
    /// (`RB_Warpball.c:12-73`). A mine set off without damage still goes up as it would.
    fn ended(&mut self, key: ShotKey, at: Vec3) {
        self.previous.remove(&key);
        let Some(info) = self.shots.remove(&key) else {
            return;
        };
        match info.item {
            Item::Bomb | Item::Shield => {
                self.burst(at);
                self.sound(Sfx::BombExplode, at);
            }
            Item::Missile => {
                self.burst(at);
                self.sound(Sfx::MissileExplode, at);
            }
            Item::WarpOrb => self.sound(Sfx::WarpHit, at),
            Item::Tnt => {
                self.effect("crate_explosion", at);
                if info.juiced {
                    self.effect("shockwave_green", at);
                    self.sound(Sfx::Glass, at);
                } else {
                    self.effect("shockwave_red", at);
                }
                self.sound(Sfx::TntExplode, at);
            }
            Item::Beaker => self.sound(Sfx::Glass, at),
            _ => {}
        }
    }

    /// TNT thrown off a head, falling until it lands, where it goes off.
    fn fly_thrown(&mut self, world: &Collision, origin: Vec3) {
        let mut landed = Vec::new();
        let frame = self.frame;
        self.thrown.retain_mut(|t| {
            t.at.y += t.vy;
            t.vy = (t.vy - THROWN_GRAVITY).max(-THROWN_TERMINAL);
            let ground = world
                .ground(t.at + Vec3::Y * 256.0, 512.0)
                .map(|g| g.point.y);
            let down = t.vy < 0.0 && ground.is_none_or(|g| t.at.y <= g);
            if down || frame - t.born > 120 {
                landed.push(t.at);
                return false;
            }
            true
        });
        for at in landed {
            let at = from_lu(at, origin);
            self.effect("crate_explosion", at);
            self.effect("shockwave_red", at);
            self.sound(Sfx::TntExplode, at);
        }
    }

    /// The fruit reach the counter one every 5 frames (0x42), and 10 is juiced (0x41)
    /// (`UI/UI_RenderFrame.c:212-257`, `231/RB_Player.c:188-197`).
    fn credit_wumpa(&mut self) {
        if self.wumpa > self.wumpa_total {
            self.wumpa = self.wumpa_total;
        } else if self.wumpa < self.wumpa_total
            && self.frame.saturating_sub(self.credited_at) >= CREDIT_FRAMES
        {
            self.wumpa += 1;
            self.credited_at = self.frame;
            self.sounds.push((Sfx::WumpaCount, 1.0));
            if self.wumpa == ctr_weapons::item::JUICED_WUMPA {
                self.sounds.push((Sfx::RouletteJuiced, 1.0));
            }
        }
    }
}

#[cfg(test)]
mod tests;
