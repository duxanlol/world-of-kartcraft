//! CTR's ground types (`enum TerrainType`, `namespace_Level.h:78-101`) and their rows of
//! `MetaDataTerrain` (`zGlobal_DATA.c:6976-7116`, struct `regionsEXE.h:146-196`, spec 8.2): the
//! physics multipliers a [`Terrain`] carries, and the skid and rolling sounds `VehEmitter`
//! plays over each (`VehEmitter.c:1187-1267`). The host says which one is under the kart
//! ([`crate::Kart::ground_type`]); this crate never learns where it came from.

use crate::kart::Terrain;

/// `TERRAIN_FLAG_*` (`regionsEXE.h:95-106`).
pub mod flags {
    pub const RAISE_GROUND_OFFSET: u32 = 0x1;
    pub const ACCEL_WHILE_REVERSE_SLIDING: u32 = 0x4;
    /// Both skid flags every frame on it (`VehEmitter.c:1413-1419`).
    pub const FORCE_SKIDMARKS: u32 = 0x8;
    pub const SKIP_TURN_ASSIST: u32 = 0x10;
    /// No rolling sound (`VehEmitter.c:1203-1210`).
    pub const ONESHOT_GROUND_SOUND: u32 = 0x20;
    pub const LANDING_SPARKS: u32 = 0x40;
    /// The minimum frictions of mud (`VehPhysForce.c:680-787`).
    pub const MUD_PHYSICS: u32 = 0x80;
    pub const SIDESLIP_FRICTION: u32 = 0x100;
}

/// CTR's ground types, in `enum TerrainType`'s order (the index into `MetaDataTerrain`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Ground {
    #[default]
    Asphalt,
    Dirt,
    Grass,
    Wood,
    Water,
    Stone,
    Ice,
    Track,
    IcyRoad,
    Snow,
    /// In the air.
    None,
    Hardpack,
    Metal,
    FastWater,
    Mud,
    Sideslip,
    RiverAsphalt,
    SteamAsphalt,
    OceanAsphalt,
    SlowGrass,
    SlowDirt,
}

/// One `MetaDataTerrain` row's fields this port uses.
struct Row {
    flags: u32,
    speed: i32,
    accel: i32,
    counter_steer: i32,
    lean: i32,
    ground_friction: i32,
    turn_angle: i32,
    turn_response: i32,
    skid_sound: i16,
    sound: i16,
}

#[allow(clippy::too_many_arguments)]
const fn row(
    flags: u32,
    speed: i32,
    accel: i32,
    counter_steer: i32,
    lean: i32,
    ground_friction: i32,
    turn_angle: i32,
    turn_response: i32,
    skid_sound: i16,
    sound: i16,
) -> Row {
    Row {
        flags,
        speed,
        accel,
        counter_steer,
        lean,
        ground_friction,
        turn_angle,
        turn_response,
        skid_sound,
        sound,
    }
}

impl Ground {
    pub const ALL: [Ground; 21] = [
        Ground::Asphalt,
        Ground::Dirt,
        Ground::Grass,
        Ground::Wood,
        Ground::Water,
        Ground::Stone,
        Ground::Ice,
        Ground::Track,
        Ground::IcyRoad,
        Ground::Snow,
        Ground::None,
        Ground::Hardpack,
        Ground::Metal,
        Ground::FastWater,
        Ground::Mud,
        Ground::Sideslip,
        Ground::RiverAsphalt,
        Ground::SteamAsphalt,
        Ground::OceanAsphalt,
        Ground::SlowGrass,
        Ground::SlowDirt,
    ];

    /// The row, field for field as `zGlobal_DATA.c` has it: flags, speedMultiplier,
    /// slowUntilSpeed (the accel multiplier), counterSteerRatio, turnLeanScale,
    /// groundFrictionScale, turnAngleScale, turnResponseScale, skidSound, sound.
    const fn row(self) -> Row {
        match self {
            Ground::Asphalt
            | Ground::Stone
            | Ground::RiverAsphalt
            | Ground::SteamAsphalt
            | Ground::OceanAsphalt => row(64, 256, 256, 256, 256, 256, 256, 256, 17, -1),
            Ground::Dirt | Ground::Grass => row(0, 140, 180, 256, 256, 256, 256, 256, 18, 24),
            Ground::Wood => row(33, 256, 256, 256, 256, 256, 256, 256, 17, 21),
            Ground::Water => row(0, 180, 200, 256, 256, 256, 256, 256, -1, 25),
            Ground::Ice => row(28, 256, 100, 0, 0, 32, 128, 32, -1, 22),
            Ground::Track => row(33, 140, 180, 256, 256, 256, 256, 256, 17, 21),
            Ground::IcyRoad => row(64, 256, 256, 256, 256, 128, 256, 256, 19, -1),
            Ground::Snow => row(0, 140, 180, 256, 256, 128, 256, 256, 19, 23),
            Ground::None => row(0, 256, 256, 0, 256, 256, 256, 256, -1, -1),
            Ground::Hardpack => row(0, 256, 256, 256, 256, 256, 256, 256, 18, -1),
            Ground::Metal => row(64, 256, 256, 256, 256, 256, 256, 256, 17, 26),
            Ground::FastWater => row(0, 256, 256, 256, 256, 256, 256, 256, -1, 25),
            Ground::Mud => row(128, 140, 180, 256, 256, 512, 256, 256, -1, 27),
            Ground::Sideslip => row(320, 256, 256, 256, 256, 256, 256, 256, 17, -1),
            Ground::SlowGrass | Ground::SlowDirt => {
                row(0, 100, 180, 256, 256, 256, 256, 256, 18, 24)
            }
        }
    }

    /// The physics this ground gives the kart.
    pub const fn terrain(self) -> Terrain {
        let r = self.row();
        Terrain {
            speed: r.speed,
            accel: r.accel,
            counter_steer: r.counter_steer,
            lean: r.lean,
            ground_friction: r.ground_friction,
            turn_angle: r.turn_angle,
            turn_response: r.turn_response,
            flags: r.flags,
        }
    }

    /// The OtherFX id `VehEmitter` loops while the kart skids here; `None` is CTR's −1, silence.
    pub const fn skid_sound(self) -> Option<u16> {
        let s = self.row().skid_sound;
        if s < 0 {
            None
        } else {
            Some(s as u16)
        }
    }

    /// The OtherFX id looped while the kart rolls over it (`terrain->sound`); none on a ground
    /// flagged `ONESHOT_GROUND_SOUND` or with −1.
    pub const fn rolling_sound(self) -> Option<u16> {
        let r = self.row();
        if r.flags & flags::ONESHOT_GROUND_SOUND != 0 || r.sound < 0 {
            None
        } else {
            Some(r.sound as u16)
        }
    }
}

/// The skid sound's volume, CTR's 0..255, at `speed` (|speedApprox|): `VehEmitter.c:1483-1497`
/// maps 2000..12000 to 0x14..0xAA, and plays none below 0x201 (`:1462-1465`). The steer's half
/// that CTR adds on top is left to the caller.
pub fn skid_volume(speed: i32) -> i32 {
    let speed = speed.abs();
    if speed < 0x201 {
        return 0;
    }
    crate::kart::map(speed, 2000, 12000, 0x14, 0xAA)
}

/// The rolling sound's volume (0..200) and OtherFX distortion (0x6C..0xD2) at `speed`
/// (`VehEmitter.c:1212-1243`: 0..5000 and 0..12000).
pub fn rolling_mix(speed: i32) -> (i32, i32) {
    let speed = speed.abs();
    (
        crate::kart::map(speed, 0, 5000, 0, 200),
        crate::kart::map(speed, 0, 12000, 0x6C, 0xD2),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Collision, Input, Kart, Modifiers};
    use glam::Vec3;

    #[test]
    fn the_presets_are_ctr_s_table() {
        // `zGlobal_DATA.c:6976-7116`, the fields in table order.
        let t = |g: Ground| {
            let t = g.terrain();
            [
                t.flags as i32,
                t.speed,
                t.accel,
                t.counter_steer,
                t.lean,
                t.ground_friction,
                t.turn_angle,
                t.turn_response,
            ]
        };
        assert_eq!(t(Ground::Asphalt), [64, 256, 256, 256, 256, 256, 256, 256]);
        assert_eq!(t(Ground::Dirt), [0, 140, 180, 256, 256, 256, 256, 256]);
        assert_eq!(t(Ground::Grass), t(Ground::Dirt));
        assert_eq!(t(Ground::Wood), [33, 256, 256, 256, 256, 256, 256, 256]);
        assert_eq!(t(Ground::Water), [0, 180, 200, 256, 256, 256, 256, 256]);
        assert_eq!(t(Ground::Stone), t(Ground::Asphalt));
        assert_eq!(t(Ground::Ice), [28, 256, 100, 0, 0, 32, 128, 32]);
        assert_eq!(t(Ground::Track), [33, 140, 180, 256, 256, 256, 256, 256]);
        assert_eq!(t(Ground::IcyRoad), [64, 256, 256, 256, 256, 128, 256, 256]);
        assert_eq!(t(Ground::Snow), [0, 140, 180, 256, 256, 128, 256, 256]);
        assert_eq!(t(Ground::None), [0, 256, 256, 0, 256, 256, 256, 256]);
        assert_eq!(t(Ground::Hardpack), [0, 256, 256, 256, 256, 256, 256, 256]);
        assert_eq!(t(Ground::Metal), t(Ground::Asphalt));
        assert_eq!(t(Ground::FastWater), t(Ground::Hardpack));
        assert_eq!(t(Ground::Mud), [128, 140, 180, 256, 256, 512, 256, 256]);
        assert_eq!(
            t(Ground::Sideslip),
            [320, 256, 256, 256, 256, 256, 256, 256]
        );
        assert_eq!(t(Ground::SlowGrass), [0, 100, 180, 256, 256, 256, 256, 256]);
        assert_eq!(t(Ground::SlowDirt), t(Ground::SlowGrass));
        // The kart's own presets are the table's rows.
        assert_eq!(Ground::Asphalt.terrain(), Terrain::ASPHALT);
        assert_eq!(Ground::Ice.terrain(), Terrain::ICE);
        assert_eq!(Ground::None.terrain(), Terrain::NONE);
        // Sounds: asphalt skids 17 and rolls silent, grass skids 18 and rolls 24, wood is
        // one-shot (no roll), mud rolls 27 with no skid.
        assert_eq!(
            (
                Ground::Asphalt.skid_sound(),
                Ground::Asphalt.rolling_sound()
            ),
            (Some(17), None)
        );
        assert_eq!(
            (Ground::Grass.skid_sound(), Ground::Grass.rolling_sound()),
            (Some(18), Some(24))
        );
        assert_eq!(Ground::Wood.rolling_sound(), None);
        assert_eq!(
            (Ground::Snow.skid_sound(), Ground::Snow.rolling_sound()),
            (Some(19), Some(23))
        );
        assert_eq!(
            (Ground::Mud.skid_sound(), Ground::Mud.rolling_sound()),
            (None, Some(27))
        );
        assert_eq!(Ground::Water.rolling_sound(), Some(25));
        assert_eq!(Ground::Ice.rolling_sound(), Some(22));
        assert_eq!(Ground::Metal.rolling_sound(), Some(26));
    }

    #[test]
    fn the_sound_mixes_follow_vehemitter() {
        assert_eq!(skid_volume(0x200), 0);
        assert_eq!(skid_volume(1000), 0x14);
        assert_eq!(skid_volume(-12000), 0xAA);
        assert_eq!(rolling_mix(0), (0, 0x6C));
        assert_eq!(rolling_mix(5000).0, 200);
        assert_eq!(rolling_mix(12000), (200, 0xD2));
    }

    /// A flat floor at y = 0 in 2000-lu tiles (the grid leaves out faces too large to drive on).
    fn flat() -> Collision {
        let (half, tile) = (40000.0, 2000.0);
        let mut tris = Vec::new();
        for i in 0..40 {
            for j in 0..40 {
                let (x0, z0) = (-half + i as f32 * tile, -half + j as f32 * tile);
                let (x1, z1) = (x0 + tile, z0 + tile);
                tris.push([[x0, 0.0, z0], [x0, 0.0, z1], [x1, 0.0, z1]]);
                tris.push([[x0, 0.0, z0], [x1, 0.0, z1], [x1, 0.0, z0]]);
            }
        }
        Collision::new(tris, 160.0)
    }

    fn kart() -> Kart {
        Kart::new(crate::Class::Balanced, Vec3::new(0.0, 1.0, 0.0), 0)
    }

    const GAS: Input = Input {
        steer: 0.0,
        gas: true,
        brake: false,
        down: false,
        r1: false,
        l1: false,
    };

    /// The speed after `frames` of full gas from a standstill on `ground`.
    fn top(ground: Ground, frames: usize) -> i32 {
        let w = flat();
        let mut kart = kart();
        kart.ground_type = ground.terrain();
        for _ in 0..frames {
            kart.tick(&w, &GAS, &Modifiers::default());
        }
        kart.speed()
    }

    #[test]
    fn dirt_holds_the_kart_to_140_256ths_and_asphalt_is_unchanged() {
        let asphalt = top(Ground::Asphalt, 150);
        let dirt = top(Ground::Dirt, 150);
        let slow = top(Ground::SlowGrass, 150);
        let stats = crate::Class::Balanced.stats();
        // Asphalt reaches the class's top speed; the default ground is asphalt.
        assert!((asphalt - stats.speed).abs() < 200, "{asphalt}");
        assert_eq!(kart().ground_type, Terrain::ASPHALT);
        let want = (stats.speed * 140) >> 8;
        assert!((dirt - want).abs() < 200, "dirt {dirt} want {want}");
        let want = (stats.speed * 100) >> 8;
        assert!((slow - want).abs() < 200, "slow grass {slow} want {want}");
        // Mud has dirt's cap, reached slower against twice the friction.
        assert!((top(Ground::Mud, 150) - dirt).abs() < 200);
        assert!(top(Ground::Mud, 15) < top(Ground::Dirt, 15));
        // Dirt's engine is 180/256 of asphalt's: slower off the line too.
        assert!(top(Ground::Dirt, 10) < top(Ground::Asphalt, 10));
    }

    #[test]
    fn leaving_asphalt_for_dirt_at_speed_bleeds_down_to_dirt_s_cap() {
        let w = flat();
        let mut kart = kart();
        for _ in 0..120 {
            kart.tick(&w, &GAS, &Modifiers::default());
        }
        let fast = kart.speed();
        kart.ground_type = Ground::Dirt.terrain();
        for _ in 0..90 {
            kart.tick(&w, &GAS, &Modifiers::default());
        }
        let want = (crate::Class::Balanced.stats().speed * 140) >> 8;
        assert!(fast > want + 2000);
        assert!(
            (kart.speed() - want).abs() < 300,
            "{} want {want}",
            kart.speed()
        );
    }

    #[test]
    fn a_turbo_pad_tops_reserves_up_fires_once_and_lifts_the_cap_to_4096_over() {
        let w = flat();
        let mut kart = kart();
        for _ in 0..90 {
            kart.tick(&w, &GAS, &Modifiers::default());
        }
        let cruise = kart.speed();
        kart.events.clear();
        for _ in 0..4 {
            kart.turbo_pad();
            kart.tick(&w, &GAS, &Modifiers::default());
        }
        let turbos = kart
            .events
            .iter()
            .filter(|e| **e == crate::Event::Turbo)
            .count();
        assert_eq!(turbos, 1, "one cue for driving onto it");
        // Frozen at 960 while on it: the 32 ms drained each tick is topped up again.
        assert_eq!(
            kart.reserves(),
            crate::stats::TURBO_PAD_RESERVES - crate::MS
        );
        // The pad's engine (8000) takes the kart well past its class's speed at once.
        assert!(kart.speed() > cruise + 2000, "{} {cruise}", kart.speed());
        assert!(kart.speed() <= cruise + 4096 + 300);
        // Braking on a pad refuses it (`VehFire.c:120-129`).
        let brake = Input {
            gas: false,
            brake: true,
            ..GAS
        };
        let mut kart = kart_at_rest(&w);
        kart.tick(&w, &brake, &Modifiers::default());
        kart.turbo_pad();
        assert_eq!(kart.reserves(), 0);
    }

    fn kart_at_rest(w: &Collision) -> Kart {
        let mut k = kart();
        k.tick(w, &Input::default(), &Modifiers::default());
        k
    }

    #[test]
    fn skids_come_from_braking_wheelspin_and_the_powerslide_not_cruising() {
        let w = flat();
        let mut kart = kart();
        // A standing start spins the wheels: speed under half the target (`VehPhysForce.c:522`).
        let mut spun = false;
        for _ in 0..120 {
            kart.tick(&w, &GAS, &Modifiers::default());
            spun |= kart.skids().0;
        }
        assert!(spun, "wheelspin off the line");
        assert_eq!(kart.skids(), (false, false), "cruising at top speed");
        // The brake at speed: the back tyres (`VehPhysForce.c:448-453`).
        let brake = Input {
            gas: false,
            brake: true,
            ..GAS
        };
        kart.tick(&w, &brake, &Modifiers::default());
        assert_eq!(kart.skids(), (true, false));
        // Stopped, no skid.
        for _ in 0..120 {
            kart.tick(&w, &brake, &Modifiers::default());
        }
        assert_eq!(kart.skids(), (false, false));
        // Ice forces both (`TERRAIN_FLAG_FORCE_SKIDMARKS`).
        kart.ground_type = Ground::Ice.terrain();
        kart.tick(&w, &Input::default(), &Modifiers::default());
        assert_eq!(kart.skids(), (true, true));
    }
}
