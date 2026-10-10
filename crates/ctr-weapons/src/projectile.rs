//! The weapons in flight and on the ground: the bowling bomb, the shield bomb and the tracking
//! missile (`RB_MovingExplosive_ThTick`, `231/RB_MovingExplosive.c:19-367`), the warp orb's homing
//! (`231/RB_Warpball.c:428-783`), dropped TNT, Nitro and beakers (`RB_GenericMine_ThTick`,
//! `231/RB_GenericMine.c:100-420`), the blasts they end in (`RB_Burst_Init`,
//! `231/RB_Burst.c:270-317`; `RB_Blowup_Init`, `231/RB_Blowup.c:131-182`), and the [`Field`] that
//! holds them under CTR's caps.
//!
//! Positions are whole lu stored as `f32`, velocities lu per frame; one `step` is one 30 Hz frame.
//! CTR snaps weapons to quadblocks and blows them up on "weapon react" trigger quads; WoW terrain
//! has neither, so the ground is [`Collision::ground`] and a wall is any face steeper than
//! [`GROUND_NORMAL_Y`] crossed by the frame's move ([`Collision::ray`]), the port contract of
//! `docs/ctr-weapons-spec.md` 3.1.

use ctr_kart::collision::{Collision, GROUND_NORMAL_Y};
use glam::Vec3;

use crate::item::Item;

/// A racer, as the network names it.
pub type KartId = u32;

/// `GAME_TRACKER->numMissiles`' cap: at most 12 bombs and missiles in flight; another shot is
/// dropped (`Vehicle/VehPickupItem.c:641-648`).
pub const MAX_MOVING: usize = 12;
/// The race mine pool: 10 TNT, Nitro and beakers; one more sets off the oldest
/// (`231/RB_MinePool.c:9-35`, `:76-94`).
pub const MINE_POOL: usize = 10;

/// A bomb's launch speed, `facing * 3 >> 7` (`VehPickupItem.c:822-823`).
pub const BOMB_SPEED: f32 = 96.0;
/// Down held: `-((96 >> 1) * 3 / 5)` (`VehPickupItem.c:829-835`).
pub const BOMB_BACK_SPEED: f32 = 28.0;
/// A bomb turns toward the lock-on target 4 angle units a frame (`RB_MovingExplosive.c:129-143`).
pub const BOMB_TURN: i32 = 4;
/// A missile's speed, `facing * 5 >> 8`; juiced, the bomb's 96 (`VehPickupItem.c:837-847`).
pub const MISSILE_SPEED: f32 = 80.0;
pub const MISSILE_SPEED_JUICED: f32 = 96.0;
/// A missile's turn per frame: 64, juiced 128 (`RB_MovingExplosive.c:145-163`).
pub const MISSILE_TURN: i32 = 64;
pub const MISSILE_TURN_JUICED: i32 = 128;
/// A warp orb's speed, `facing * 7 >> 8` (`VehPickupItem.c:1000-1122`, `RB_Warpball.c:497-498`).
pub const WARP_SPEED: f32 = 112.0;

/// Bombs, missiles and shield bombs touch a kart within 96 lu in XZ (`0x2400` squared) and 192
/// in Y (`RB_MovingExplosive.c:325`, `RB_Hazard.c:25-85`); and a mine within 96 lu in 3D.
pub const CONTACT_RADIUS: f32 = 96.0;
/// A warp orb touches within 192 lu in XZ (`0x9000`) and 384 in Y (`RB_Warpball.c:701`).
pub const WARP_CONTACT_RADIUS: f32 = 192.0;
/// TNT and Nitro go off within 120 lu in 3D (`0x3840`); beakers within 80 (`0x1900`)
/// (`RB_GenericMine.c:154`).
pub const TNT_RADIUS: f32 = 120.0;
pub const BEAKER_RADIUS: f32 = 80.0;
/// A bomb's or shield bomb's blast, 320 lu; a juiced bomb's 512 (`RB_Burst.c:291-305`).
pub const BLAST_RADIUS: f32 = 320.0;
pub const BLAST_RADIUS_JUICED: f32 = 512.0;
/// A missile's blast, 128 lu juiced or not (`RB_Burst.c:291-295`).
pub const MISSILE_BLAST_RADIUS: f32 = 128.0;
/// TNT's blowup, 320 lu to everyone; Nitro's, 128 to players and 320 to other mines
/// (`RB_Blowup.c:157-178`).
pub const TNT_BLOWUP_RADIUS: f32 = 320.0;
pub const NITRO_PLAYER_RADIUS: f32 = 128.0;

/// The owner is ignored by its own bomb or missile for 60 frames (`VehPickupItem.c:849`); by its
/// warp orb, shield bomb and mines for 10 (`:1108`, `:1189`, `RB_MaskShieldCloud.c:455`).
pub const SAFETY_FRAMES: u8 = 60;
pub const SHORT_SAFETY_FRAMES: u8 = 10;

/// A grounded bomb or missile rides 48 lu over the face found (`RB_MovingExplosive.c:266-268`).
pub const RIDE_HEIGHT: f32 = 48.0;
/// The ground probe spans 256 lu above to 64 below (`RB_MovingExplosive.c:220-226`) …
const PROBE_UP: f32 = 256.0;
const PROBE_DOWN: f32 = 64.0;
/// … then, with nothing there, 2304 below; nothing there either is a fall into the void
/// (`:273-284`).
const DEEP_PROBE: f32 = 2304.0;
/// Gravity, lu per frame²: 4, missiles 8; terminal fall 96 (`:286-291`).
const GRAVITY: f32 = 4.0;
const MISSILE_GRAVITY: f32 = 8.0;
const TERMINAL: f32 = 96.0;
/// A dropped mine finds the ground 400 lu below to 64 above (`VehPickupItem.c:119-120`).
const MINE_PROBE_DOWN: f32 = 400.0;
const MINE_PROBE_UP: f32 = 64.0;

/// What the weapons need of a racer. `position` is the kart's origin (the bottom centre), lu.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct KartView {
    pub id: KartId,
    pub position: Vec3,
    pub shielded: bool,
    pub masked: bool,
    /// A TNT sits on the kart's head (`driver->instTntRecv`).
    pub tnt_on_head: bool,
    /// `squishTimer` is running.
    pub squished: bool,
    /// Race position, 0 = the leader (`driverRank`).
    pub rank: usize,
}

/// A damage attempt, as the protocol's hit kinds. Whether it lands is the victim's gatekeeping
/// (mask, shield, invincibility, `Vehicle/VehPickState.c:65-114`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HitKind {
    /// Damage type 1 (`VehPickState.c:238-246`).
    SpinOut,
    /// Damage type 2, the tumble (`VehPickState.c:151-184`).
    Blasted,
    /// Damage type 3 (`VehPickState.c:227-236`).
    Squished,
    /// A TNT jumped onto the head (`RB_GenericMine.c:263-301`).
    TntOnHead,
}

impl HitKind {
    /// The protocol letter (`docs/kartcraft-weapons-protocol.md`).
    pub fn code(self) -> char {
        match self {
            HitKind::SpinOut => 'S',
            HitKind::Blasted => 'B',
            HitKind::Squished => 'Q',
            HitKind::TntOnHead => 'T',
        }
    }

    pub fn from_code(c: char) -> Option<Self> {
        Some(match c {
            'S' => HitKind::SpinOut,
            'B' => HitKind::Blasted,
            'Q' => HitKind::Squished,
            'T' => HitKind::TntOnHead,
            _ => return None,
        })
    }

    /// Wumpa lost by an unmasked victim: spin-out 1 (`VehPhysProc.c:2868-2871`), blasted 3
    /// (`VehStuckProc.c:1466-1469`); a squish is a spin-out.
    pub fn wumpa_lost(self) -> u8 {
        match self {
            HitKind::SpinOut | HitKind::Squished => 1,
            HitKind::Blasted => 3,
            HitKind::TntOnHead => 0,
        }
    }
}

/// Something a weapon did this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Outcome {
    Hit {
        kart: KartId,
        kind: HitKind,
    },
    /// A red beaker's rain cloud on the kart (`RB_GenericMine.c:172-176`).
    RainCloud {
        kart: KartId,
    },
    /// A damaging blast centred `at`, `radius` lu to players; its hits are listed beside it. The
    /// weapon is gone.
    Exploded {
        at: Vec3,
        radius: f32,
    },
    /// Set off without damage: a mine or missile caught by a blast or a contact, a beaker
    /// shattering (`RB_Hazard.c:125-240`). The weapon is gone.
    Destroyed {
        at: Vec3,
    },
    /// Ended quietly (a warp orb's death, `RB_Warpball.c:12-73`). The weapon is gone.
    Expired,
}

fn rad(angle: i32) -> f32 {
    angle as f32 * std::f32::consts::TAU / 4096.0
}

/// The unit heading of a yaw: 0 is +Z, 1024 is +X (`MATH_Sin` on X, `MATH_Cos` on Z).
fn heading(yaw: i32) -> Vec3 {
    let a = rad(yaw);
    Vec3::new(a.sin(), 0.0, a.cos())
}

/// `ratan2(dx, dz)` in 4096ths.
fn yaw_to(d: Vec3) -> i32 {
    ((d.x.atan2(d.z) * 4096.0 / std::f32::consts::TAU).round() as i32) & 0xFFF
}

/// `RB_Hazard_InterpolateValue` at a 32 ms frame: `rot` toward `desired` by `speed`, the short
/// way round, landing on it when closer than `speed` (`RB_Hazard.c:284-313`).
fn turn(rot: i32, desired: i32, speed: i32) -> i32 {
    let mut diff = (desired - rot) & 0xFFF;
    if diff > 0x7FF {
        diff -= 0x1000;
    }
    if diff.abs() < speed {
        desired & 0xFFF
    } else if diff > 0 {
        (rot + speed) & 0xFFF
    } else {
        (rot - speed) & 0xFFF
    }
}

/// `RB_Hazard_CollideWithDrivers` for bombs, missiles and orbs: XZ within `r`, Y within `2r`.
fn touches_xz(p: Vec3, k: Vec3, r: f32) -> bool {
    let d = k - p;
    d.x * d.x + d.z * d.z < r * r && d.y * d.y < 4.0 * r * r
}

/// The same for mines: a 3D sphere (`RB_Hazard.c:52-62`).
fn touches_3d(p: Vec3, k: Vec3, r: f32) -> bool {
    let d = k - p;
    d.length_squared() < r * r && d.y * d.y < 4.0 * r * r
}

/// The first wall crossed moving from `from` to `to`: a face steeper than ground. Ground faces
/// on the way (a ramp) are passed through; the ground probe lifts the weapon onto them.
fn wall(world: &Collision, from: Vec3, to: Vec3) -> bool {
    let d = to - from;
    let len = d.length();
    if len < 1e-3 {
        return false;
    }
    let dir = d / len;
    let mut origin = from;
    let mut left = len;
    for _ in 0..4 {
        let Some(hit) = world.ray(origin, dir, left) else {
            return false;
        };
        if hit.normal.y.abs() < GROUND_NORMAL_Y {
            return true;
        }
        let skip = hit.distance + 0.01;
        origin += dir * skip;
        left -= skip;
        if left <= 0.0 {
            return false;
        }
    }
    false
}

/// A blast: everyone within `radius` in 3D but `spare` is blasted (`PROC_CollideHitboxWithBucket`,
/// `PROC.c:480-545`).
fn blast(at: Vec3, radius: f32, karts: &[KartView], spare: Option<KartId>) -> Vec<Outcome> {
    let mut out = vec![Outcome::Exploded { at, radius }];
    for k in karts {
        if Some(k.id) != spare && (k.position - at).length_squared() < radius * radius {
            out.push(Outcome::Hit {
                kart: k.id,
                kind: HitKind::Blasted,
            });
        }
    }
    out
}

/// What bombs, shield bombs and missiles share: `struct TrackerWeapon`'s motion.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Tracker {
    owner: KartId,
    pos: Vec3,
    yaw: i32,
    vel: Vec3,
    target: Option<KartId>,
    safety: u8,
    done: bool,
}

impl Tracker {
    fn new(owner: KartId, pos: Vec3, yaw: i32, speed: f32, target: Option<KartId>) -> Self {
        Self {
            owner,
            pos,
            yaw: yaw & 0xFFF,
            vel: heading(yaw) * speed,
            target,
            safety: SAFETY_FRAMES,
            done: false,
        }
    }

    /// Turn toward the target and fly along the yaw at `speed` (`RB_MovingExplosive.c:61-171`).
    /// A target that left the race is lost for good, and the weapon flies straight on.
    fn steer(&mut self, karts: &[KartView], rate: i32, speed: f32, backward: bool) {
        let Some(id) = self.target else {
            return;
        };
        let Some(k) = karts.iter().find(|k| k.id == id) else {
            self.target = None;
            return;
        };
        self.yaw = turn(self.yaw, yaw_to(k.position - self.pos), rate);
        let v = heading(self.yaw) * if backward { -speed } else { speed };
        self.vel.x = v.x;
        self.vel.z = v.z;
    }

    /// Move one frame, then follow the ground (`RB_MovingExplosive.c:199-291`). False when the
    /// weapon must explode: it met a wall (and steps back one frame, `:236-248`) or the void.
    fn advance(&mut self, world: &Collision, gravity: f32) -> bool {
        let from = self.pos;
        self.pos += self.vel;
        if wall(world, from, self.pos) {
            self.pos = from;
            return false;
        }
        let top = self.pos + Vec3::Y * PROBE_UP;
        if let Some(hit) = world.ground(top, PROBE_UP + PROBE_DOWN) {
            self.vel.y = 0.0;
            self.pos.y = hit.point.y + RIDE_HEIGHT;
        } else if world.ground(top, PROBE_UP + DEEP_PROBE).is_some() {
            self.vel.y = (self.vel.y - gravity).max(-TERMINAL);
        } else {
            return false;
        }
        true
    }

    /// The first kart in contact; the owner is spared while the safety frames run.
    fn contact(&self, karts: &[KartView]) -> bool {
        karts.iter().any(|k| {
            !(self.safety != 0 && k.id == self.owner)
                && touches_xz(self.pos, k.position, CONTACT_RADIUS)
        })
    }
}

/// The bowling bomb (ids 1 and 10), or a fired shield (`RB_MaskShieldCloud.c:409-463`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bomb {
    core: Tracker,
    juiced: bool,
    backward: bool,
    /// A shield bomb: no target, no remote detonation, never juiced.
    shield: bool,
    /// `flags |= 2`: explode at the end of the next tick (`RB_MovingExplosive.c:357-362`).
    detonate: bool,
}

impl Bomb {
    /// A bomb from the kart at `pos` facing `yaw`, aimed at the missile lock-on `target` if any;
    /// `backward` (Down held) rolls it back at 28 lu a frame (`VehPickupItem.c:818-835`).
    pub fn new(
        owner: KartId,
        pos: Vec3,
        yaw: i32,
        juiced: bool,
        backward: bool,
        target: Option<KartId>,
    ) -> Self {
        let mut core = Tracker::new(owner, pos, yaw, BOMB_SPEED, target);
        if backward {
            core.vel = -heading(yaw) * BOMB_BACK_SPEED;
        }
        Self {
            core,
            juiced,
            backward,
            shield: false,
            detonate: false,
        }
    }

    /// A shield fired straight ahead at 96 lu a frame, 10 safety frames, a 320 lu blast
    /// (`RB_MaskShieldCloud.c:436-458`).
    pub fn shield(owner: KartId, pos: Vec3, yaw: i32) -> Self {
        let mut core = Tracker::new(owner, pos, yaw, BOMB_SPEED, None);
        core.safety = SHORT_SAFETY_FRAMES;
        Self {
            core,
            juiced: false,
            backward: false,
            shield: true,
            detonate: false,
        }
    }

    /// The owner pressed Fire again (`VehPhysProc.c:812-819`).
    pub fn detonate(&mut self) {
        if !self.shield {
            self.detonate = true;
        }
    }

    pub fn is_shield(&self) -> bool {
        self.shield
    }

    pub fn owner(&self) -> KartId {
        self.core.owner
    }

    pub fn position(&self) -> Vec3 {
        self.core.pos
    }

    pub fn done(&self) -> bool {
        self.core.done
    }

    pub fn radius(&self) -> f32 {
        if self.juiced {
            BLAST_RADIUS_JUICED
        } else {
            BLAST_RADIUS
        }
    }

    /// `RB_MovingExplosive_Explode` → `RB_Burst_Init`: the owner is never hurt.
    pub fn explode(&mut self, karts: &[KartView]) -> Vec<Outcome> {
        self.core.done = true;
        blast(self.core.pos, self.radius(), karts, Some(self.core.owner))
    }

    pub fn step(&mut self, world: &Collision, karts: &[KartView]) -> Vec<Outcome> {
        if self.core.done {
            return Vec::new();
        }
        self.core.steer(karts, BOMB_TURN, BOMB_SPEED, self.backward);
        if !self.core.advance(world, GRAVITY) || self.core.contact(karts) || self.detonate {
            return self.explode(karts);
        }
        self.core.safety = self.core.safety.saturating_sub(1);
        Vec::new()
    }
}

/// The tracking missile (ids 2 and 11).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Missile {
    core: Tracker,
    juiced: bool,
}

impl Missile {
    /// A missile from `pos` facing `yaw`, homing on `target`; none flies straight
    /// (`VehPickupItem.c:654-733`).
    pub fn new(owner: KartId, pos: Vec3, yaw: i32, juiced: bool, target: Option<KartId>) -> Self {
        let speed = if juiced {
            MISSILE_SPEED_JUICED
        } else {
            MISSILE_SPEED
        };
        Self {
            core: Tracker::new(owner, pos, yaw, speed, target),
            juiced,
        }
    }

    pub fn owner(&self) -> KartId {
        self.core.owner
    }

    pub fn position(&self) -> Vec3 {
        self.core.pos
    }

    pub fn target(&self) -> Option<KartId> {
        self.core.target
    }

    pub fn done(&self) -> bool {
        self.core.done
    }

    pub fn explode(&mut self, karts: &[KartView]) -> Vec<Outcome> {
        self.core.done = true;
        blast(
            self.core.pos,
            MISSILE_BLAST_RADIUS,
            karts,
            Some(self.core.owner),
        )
    }

    /// Caught by a blast or rammed by a bomb: it dies without a blast (`RB_Hazard.c:125-156`).
    pub fn destroy(&mut self) -> Outcome {
        self.core.done = true;
        Outcome::Destroyed { at: self.core.pos }
    }

    pub fn step(&mut self, world: &Collision, karts: &[KartView]) -> Vec<Outcome> {
        if self.core.done {
            return Vec::new();
        }
        let (rate, speed) = if self.juiced {
            (MISSILE_TURN_JUICED, MISSILE_SPEED_JUICED)
        } else {
            (MISSILE_TURN, MISSILE_SPEED)
        };
        self.core.steer(karts, rate, speed, false);
        if !self.core.advance(world, MISSILE_GRAVITY) || self.core.contact(karts) {
            return self.explode(karts);
        }
        self.core.safety = self.core.safety.saturating_sub(1);
        Vec::new()
    }
}

/// The warp orb (id 9) in CTR's homing mode, without its track rail.
#[derive(Clone, Debug, PartialEq)]
pub struct WarpOrb {
    owner: KartId,
    pos: Vec3,
    yaw: i32,
    vel_y: f32,
    juiced: bool,
    target: Option<KartId>,
    /// `driversHit`: the karts it has blasted or passed (`RB_Warpball.c:744-757`).
    hit: Vec<KartId>,
    safety: u8,
    done: bool,
}

impl WarpOrb {
    /// An orb from `pos` facing `yaw` after `target`, the leader (the server's pick, as
    /// `RB_Warpball_GetDriverTarget`, `RB_Warpball.c:127-198`).
    pub fn new(owner: KartId, pos: Vec3, yaw: i32, juiced: bool, target: Option<KartId>) -> Self {
        Self {
            owner,
            pos,
            yaw: yaw & 0xFFF,
            vel_y: 0.0,
            juiced,
            target,
            hit: vec![owner],
            safety: SHORT_SAFETY_FRAMES,
            done: false,
        }
    }

    pub fn owner(&self) -> KartId {
        self.owner
    }

    pub fn position(&self) -> Vec3 {
        self.pos
    }

    pub fn target(&self) -> Option<KartId> {
        self.target
    }

    pub fn done(&self) -> bool {
        self.done
    }

    fn die(&mut self, out: &mut Vec<Outcome>) {
        self.done = true;
        out.push(Outcome::Expired);
    }

    pub fn step(&mut self, world: &Collision, karts: &[KartView]) -> Vec<Outcome> {
        let mut out = Vec::new();
        if self.done {
            return out;
        }
        // No target (fired from 1st, or it left): CTR turns it around and kills it.
        let Some(target) = self.target.and_then(|id| karts.iter().find(|k| k.id == id)) else {
            self.die(&mut out);
            return out;
        };
        // Homing (`RB_Warpball.c:465-528`): turn 0x100 a frame, up to 0x400 inside 768 lu, 0x40
        // in the safety frames; climb or sink 4 a frame toward the target, at most 96.
        let d = target.position - self.pos;
        let d2 = d.x * d.x + d.z * d.z;
        let mut rate = 0x100;
        if d2 < 0x90000 as f32 {
            rate = (0x400 - (d2 as i32 >> 9)).max(0x100);
        }
        if self.safety > 0 {
            rate = 0x40;
        }
        self.yaw = turn(self.yaw, yaw_to(d), rate);
        if d.y > 0.0 {
            self.vel_y = (self.vel_y + GRAVITY).min(d.y).min(TERMINAL);
        } else if d.y < 0.0 {
            self.vel_y = (self.vel_y - GRAVITY).max(d.y).max(-TERMINAL);
        }
        self.pos += heading(self.yaw) * WARP_SPEED + Vec3::Y * self.vel_y;
        // CTR keeps the orb on its rail; here it rides over the terrain instead.
        let top = self.pos + Vec3::Y * PROBE_UP;
        if let Some(g) = world.ground(top, PROBE_UP + PROBE_DOWN) {
            if self.pos.y < g.point.y + RIDE_HEIGHT {
                self.pos.y = g.point.y + RIDE_HEIGHT;
                self.vel_y = self.vel_y.max(0.0);
            }
        }
        // One kart a frame (`RB_Warpball.c:701-772`); never the owner.
        let victim = karts.iter().find(|k| {
            !self.hit.contains(&k.id) && touches_xz(self.pos, k.position, WARP_CONTACT_RADIUS)
        });
        if let Some(v) = victim {
            out.push(Outcome::Hit {
                kart: v.id,
                kind: HitKind::Blasted,
            });
            if (!self.juiced && Some(v.id) == self.target) || v.rank == 0 {
                self.die(&mut out);
                return out;
            }
            // Juiced: the victim and everyone behind are passed; on to the next kart ahead.
            for k in karts {
                if k.rank >= v.rank && !self.hit.contains(&k.id) {
                    self.hit.push(k.id);
                }
            }
            if Some(v.id) == self.target {
                self.target = karts
                    .iter()
                    .filter(|k| !self.hit.contains(&k.id))
                    .max_by_key(|k| k.rank)
                    .map(|k| k.id);
                if self.target.is_none() {
                    self.die(&mut out);
                    return out;
                }
            }
        }
        self.safety = self.safety.saturating_sub(1);
        out
    }
}

/// A dropped mine's kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MineKind {
    Tnt,
    /// TNT fired juiced (`VehPickupItem.c:1143-1150`).
    Nitro,
    /// Red when fired juiced (`VehPickupItem.c:1312-1334`).
    Beaker {
        red: bool,
    },
}

impl MineKind {
    /// The mine a shot of `item` drops, if it is one.
    pub fn from_shot(item: Item, juiced: bool) -> Option<Self> {
        match item {
            Item::Tnt if juiced => Some(MineKind::Nitro),
            Item::Tnt => Some(MineKind::Tnt),
            Item::Beaker => Some(MineKind::Beaker { red: juiced }),
            _ => None,
        }
    }

    pub fn radius(self) -> f32 {
        match self {
            MineKind::Beaker { .. } => BEAKER_RADIUS,
            _ => TNT_RADIUS,
        }
    }
}

/// A TNT, Nitro or beaker sitting where it was dropped.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mine {
    owner: KartId,
    kind: MineKind,
    pos: Vec3,
    /// `stopFallAtY`: the ground found at the drop.
    rest_y: f32,
    vel_y: f32,
    safety: u8,
    /// Pushed out of the full pool: it goes off on its next tick (`RB_GenericMine.c:375-418`).
    evicted: bool,
    done: bool,
}

impl Mine {
    /// Dropped at the kart's `pos`: it settles on the ground 400 lu below to 64 above, or stays
    /// where it is (`VehPickupItem.c:1203-1276`).
    pub fn drop(owner: KartId, kind: MineKind, pos: Vec3, world: &Collision) -> Self {
        let rest_y = world
            .ground(
                pos + Vec3::Y * MINE_PROBE_UP,
                MINE_PROBE_UP + MINE_PROBE_DOWN,
            )
            .map_or(pos.y, |g| g.point.y);
        Self {
            owner,
            kind,
            pos,
            rest_y,
            vel_y: 0.0,
            safety: SHORT_SAFETY_FRAMES,
            evicted: false,
            done: false,
        }
    }

    pub fn owner(&self) -> KartId {
        self.owner
    }

    pub fn kind(&self) -> MineKind {
        self.kind
    }

    pub fn position(&self) -> Vec3 {
        self.pos
    }

    pub fn done(&self) -> bool {
        self.done
    }

    /// `RB_GenericMine_ThDestroy` → `RB_Blowup_Init` (`RB_Blowup.c:131-182`): TNT blasts every
    /// kart within 320 lu, Nitro within 128, the owner included; a beaker only shatters.
    fn blow_up(&mut self, karts: &[KartView], spare: Option<KartId>) -> Vec<Outcome> {
        self.done = true;
        match self.kind {
            MineKind::Tnt => blast(self.pos, TNT_BLOWUP_RADIUS, karts, spare),
            MineKind::Nitro => blast(self.pos, NITRO_PLAYER_RADIUS, karts, spare),
            MineKind::Beaker { .. } => vec![Outcome::Destroyed { at: self.pos }],
        }
    }

    /// Set off by a blast or a weapon's contact: a visual explosion only (`RB_Hazard.c:158-240`).
    pub fn destroy(&mut self) -> Outcome {
        self.done = true;
        Outcome::Destroyed { at: self.pos }
    }

    pub fn step(&mut self, _world: &Collision, karts: &[KartView]) -> Vec<Outcome> {
        if self.done {
            return Vec::new();
        }
        // Fall to the rest height (`RB_GenericMine.c:121-136`).
        self.pos.y = (self.pos.y + self.vel_y).max(self.rest_y);
        self.vel_y = (self.vel_y - GRAVITY).max(-TERMINAL);
        let r = self.kind.radius();
        let toucher = karts.iter().find(|k| {
            !(self.safety != 0 && k.id == self.owner) && touches_3d(self.pos, k.position, r)
        });
        let mut out = Vec::new();
        if let Some(k) = toucher {
            out = match self.kind {
                // Spin-out, plus the rain cloud for red (`RB_GenericMine.c:172-206`).
                MineKind::Beaker { red } => {
                    self.done = true;
                    let mut out = vec![Outcome::Hit {
                        kart: k.id,
                        kind: HitKind::SpinOut,
                    }];
                    if red {
                        out.push(Outcome::RainCloud { kart: k.id });
                    }
                    out.push(Outcome::Destroyed { at: self.pos });
                    out
                }
                // A TNT already on the head, a squished kart, or Nitro: blasted, and the mine
                // blows up (`RB_GenericMine.c:211-258`).
                _ if k.tnt_on_head || k.squished || self.kind == MineKind::Nitro => {
                    let mut out = vec![Outcome::Hit {
                        kart: k.id,
                        kind: HitKind::Blasted,
                    }];
                    out.extend(self.blow_up(karts, Some(k.id)));
                    out
                }
                // Mask or shield: the TNT just blows up (`:261-366`, `RB_GenericMine_ThDestroy`).
                MineKind::Tnt if k.masked || k.shielded => self.blow_up(karts, None),
                // Otherwise it jumps onto the head; the kart carries it from here.
                MineKind::Tnt => {
                    self.done = true;
                    vec![Outcome::Hit {
                        kart: k.id,
                        kind: HitKind::TntOnHead,
                    }]
                }
                MineKind::Nitro => unreachable!(),
            };
        }
        self.safety = self.safety.saturating_sub(1);
        if self.evicted && !self.done {
            out.extend(self.blow_up(karts, None));
        }
        out
    }
}

/// One live weapon.
#[derive(Clone, Debug, PartialEq)]
pub enum Body {
    Bomb(Bomb),
    Missile(Missile),
    WarpOrb(WarpOrb),
    Mine(Mine),
}

impl Body {
    pub fn owner(&self) -> KartId {
        match self {
            Body::Bomb(b) => b.owner(),
            Body::Missile(m) => m.owner(),
            Body::WarpOrb(w) => w.owner(),
            Body::Mine(m) => m.owner(),
        }
    }

    pub fn position(&self) -> Vec3 {
        match self {
            Body::Bomb(b) => b.position(),
            Body::Missile(m) => m.position(),
            Body::WarpOrb(w) => w.position(),
            Body::Mine(m) => m.position(),
        }
    }

    pub fn done(&self) -> bool {
        match self {
            Body::Bomb(b) => b.done(),
            Body::Missile(m) => m.done(),
            Body::WarpOrb(w) => w.done(),
            Body::Mine(m) => m.done(),
        }
    }

    pub fn step(&mut self, world: &Collision, karts: &[KartView]) -> Vec<Outcome> {
        match self {
            Body::Bomb(b) => b.step(world, karts),
            Body::Missile(m) => m.step(world, karts),
            Body::WarpOrb(w) => w.step(world, karts),
            Body::Mine(m) => m.step(world, karts),
        }
    }

    /// Counted against [`MAX_MOVING`]: bombs and missiles from `ShootNow`, not shield bombs.
    fn counts_as_moving(&self) -> bool {
        matches!(self, Body::Bomb(b) if !b.is_shield()) || matches!(self, Body::Missile(_))
    }
}

/// Names a shot across the network: the shooter and its own shot counter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ShotKey {
    pub owner: KartId,
    pub shot: u16,
}

/// Every live weapon of a race, stepped together so they meet each other: a bomb or missile
/// sets off a mine it touches (`RB_MovingExplosive.c:330-336`), a bomb and a missile that touch
/// both go (`:338-353`), a warp orb sets off mines it touches (`RB_Warpball.c:774-782`), and a
/// bomb or missile blast sets off the mines and missiles in it (`RB_Burst.c:308-317`).
#[derive(Clone, Debug, Default)]
pub struct Field {
    live: Vec<(ShotKey, Body)>,
}

impl Field {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a weapon. False when the 12-bomb-and-missile cap refuses it. A mine into a full pool
    /// pushes out the oldest, which goes off on its next step.
    pub fn launch(&mut self, key: ShotKey, body: Body) -> bool {
        if body.counts_as_moving()
            && self
                .live
                .iter()
                .filter(|(_, b)| b.counts_as_moving())
                .count()
                >= MAX_MOVING
        {
            return false;
        }
        if matches!(body, Body::Mine(_)) {
            let pooled = |b: &Body| matches!(b, Body::Mine(m) if !m.evicted && !m.done);
            if self.live.iter().filter(|(_, b)| pooled(b)).count() >= MINE_POOL {
                if let Some((_, Body::Mine(oldest))) = self.live.iter_mut().find(|(_, b)| pooled(b))
                {
                    oldest.evicted = true;
                }
            }
        }
        self.live.push((key, body));
        true
    }

    /// The owner's rolling bomb goes off at the end of its next step. False if it has none.
    pub fn detonate(&mut self, owner: KartId) -> bool {
        for (_, b) in &mut self.live {
            if let Body::Bomb(bomb) = b {
                if bomb.owner() == owner && !bomb.is_shield() {
                    bomb.detonate();
                    return true;
                }
            }
        }
        false
    }

    /// Takes a weapon out (a relayed hit ended it).
    pub fn remove(&mut self, key: ShotKey) -> Option<Body> {
        let i = self.live.iter().position(|(k, _)| *k == key)?;
        Some(self.live.remove(i).1)
    }

    pub fn get(&self, key: ShotKey) -> Option<&Body> {
        self.live.iter().find(|(k, _)| *k == key).map(|(_, b)| b)
    }

    pub fn iter(&self) -> impl Iterator<Item = (ShotKey, &Body)> {
        self.live.iter().map(|(k, b)| (*k, b))
    }

    pub fn len(&self) -> usize {
        self.live.len()
    }

    pub fn is_empty(&self) -> bool {
        self.live.is_empty()
    }

    /// One frame of every weapon, in launch order.
    pub fn step(&mut self, world: &Collision, karts: &[KartView]) -> Vec<(ShotKey, Outcome)> {
        let mut out = Vec::new();
        for i in 0..self.live.len() {
            if self.live[i].1.done() {
                continue;
            }
            let key = self.live[i].0;
            let mut own = self.live[i].1.step(world, karts);
            let pos = self.live[i].1.position();
            if !self.live[i].1.done() {
                own.extend(self.meet(i, pos, karts, &mut out));
            }
            // A bomb's or missile's blast sets off the mines and missiles in it.
            let reach = own.iter().find_map(|o| match (o, &self.live[i].1) {
                (Outcome::Exploded { at, radius }, Body::Bomb(_) | Body::Missile(_)) => {
                    Some((*at, *radius))
                }
                (Outcome::Exploded { at, .. }, Body::Mine(_)) => Some((*at, TNT_BLOWUP_RADIUS)),
                _ => None,
            });
            let is_mine = matches!(self.live[i].1, Body::Mine(_));
            out.extend(own.into_iter().map(|o| (key, o)));
            if let Some((at, radius)) = reach {
                for j in 0..self.live.len() {
                    if j == i || self.live[j].1.done() {
                        continue;
                    }
                    let (k, b) = &mut self.live[j];
                    if (b.position() - at).length_squared() >= radius * radius {
                        continue;
                    }
                    match b {
                        Body::Mine(m) => out.push((*k, m.destroy())),
                        // A TNT or Nitro blowup reaches mines, not missiles (`RB_Blowup.c:157-160`).
                        Body::Missile(m) if !is_mine => out.push((*k, m.destroy())),
                        _ => {}
                    }
                }
            }
        }
        self.live.retain(|(_, b)| !b.done());
        out
    }

    /// Weapon `i`, still flying at `pos`, against the other weapons.
    fn meet(
        &mut self,
        i: usize,
        pos: Vec3,
        karts: &[KartView],
        out: &mut Vec<(ShotKey, Outcome)>,
    ) -> Vec<Outcome> {
        let (is_bomb, is_tracker, is_orb) = match &self.live[i].1 {
            Body::Bomb(_) => (true, true, false),
            Body::Missile(_) => (false, true, false),
            Body::WarpOrb(_) => (false, false, true),
            Body::Mine(_) => return Vec::new(),
        };
        let mut boom = false;
        for j in 0..self.live.len() {
            if j == i || self.live[j].1.done() {
                continue;
            }
            let (k, b) = &mut self.live[j];
            if !touches_3d(pos, b.position(), CONTACT_RADIUS) {
                continue;
            }
            match b {
                Body::Mine(m) if is_tracker || is_orb => {
                    out.push((*k, m.destroy()));
                    boom |= is_tracker;
                }
                Body::Missile(m) if is_bomb => {
                    out.push((*k, m.destroy()));
                    boom = true;
                }
                _ => {}
            }
            if boom {
                break;
            }
        }
        if !boom {
            return Vec::new();
        }
        match &mut self.live[i].1 {
            Body::Bomb(b) => b.explode(karts),
            Body::Missile(m) => m.explode(karts),
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests;
