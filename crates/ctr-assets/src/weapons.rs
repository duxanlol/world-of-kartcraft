//! `weapons.json`: CTR's weapon, crate and Wumpa fruit models with every animation frame and
//! their textures, from the player's disc, plus the pickup/roulette timing constants.
//!
//! Weapon models come from `shared.mpk` / `shared.vrm` (the pack `effects.rs` reads); the
//! weapon crate, fruit crate and Wumpa fruit exist only in the track level files, so they are
//! taken from one track (Crash Cove 1P, `levels\tracks\coco1\1P`), whose `data.vrm` holds the
//! crate art; every track carries the same three models. Model ids are `gGT->modelPtr` indices
//! (CTR-native `include/namespace_Instance.h`); each entry cites where the game spawns it.
//!
//! Units follow `effects.json`: meters = world units / 51.2, +Y up. Positions are the model at
//! instance scale 0x1000 (`INSTANCE_Birth`'s default, `INSTANCE.c:31`); where the game spawns
//! an instance at another scale, the entry's `instance_scale` says so (1.0 = 0x1000).

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{json, Value};

use crate::big::{self, BigFile};
use crate::ctr::{unwrap_patched, Bytes, Model};
use crate::effects::{mesh_frame_at, MeshFrame, Textures, FPS, WORLD_PER_METER};
use crate::kart::Texture;
use crate::mpk;
use crate::vram::Vram;
use crate::{Error, Result};

/// Where a model is read from.
#[derive(Clone, Copy)]
enum Pack {
    /// `packs\shared.mpk`.
    Shared,
    /// The track level file (`data.lev`).
    Level,
}

struct Spec {
    /// Key in `weapons.json` `models`.
    key: &'static str,
    pack: Pack,
    /// Model name in the pack.
    name: &'static str,
    /// `gGT->modelPtr` id.
    id: i16,
    /// Instance scale the game spawns it at, 4096 = 1.0.
    scale: u16,
    source: &'static str,
    animation: &'static str,
}

const LOOP: &str = "loops: frame index advances by 1 every game frame and wraps (animFrame + 1 < INSTANCE_GetNumAnimFrames, else 0)";
const ONCE: &str = "plays once: frame index advances by 1 every game frame; the instance dies after the last frame";
const STATIC: &str = "static (one frame)";

const MODELS: [Spec; 23] = [
    Spec {
        key: "crate_question",
        pack: Pack::Level,
        name: "crate_question",
        id: 0x08,
        scale: 0x1000,
        source: "level model crate_question (PU_RANDOM_CRATE 0x08); RB_Crate.c: broken on touch, gives a roulette roll (itemRollTimer = 90, OtherFX 0x5D), regrows after a cooldown",
        animation: STATIC,
    },
    Spec {
        key: "crate_fruit",
        pack: Pack::Level,
        name: "crate_fruit",
        id: 0x07,
        scale: 0x1000,
        source: "level model crate_fruit (PU_FRUIT_CRATE 0x07); RB_Crate.c: broken on touch, gives rand % 4 + 5 Wumpa",
        animation: STATIC,
    },
    Spec {
        key: "fruit",
        pack: Pack::Level,
        name: "fruit",
        id: 0x02,
        scale: 0x1000,
        source: "level model fruit (PU_WUMPA_FRUIT 0x02), mesh fruit_hi; untextured Gouraud; RB_Fruit.c:55-60: hidden for the rest of the race once picked up (PlaySound3D 0x43)",
        animation: "one frame: the model's 32-frame `spin` animation (looped, RB_Fruit.c:68 RB_Fruit_LInB sets ANIM_LOOP) has byte-identical vertex frames on the disc, so it is exported once; the turning seen in game is not in the vertex frames",
    },
    Spec {
        key: "bomb",
        pack: Pack::Shared,
        name: "bomb",
        id: 0x3B,
        scale: 0x1000,
        source: "shared.mpk bomb (DYNAMIC_BOMB 0x3B); VehPickupItem.c:742 INSTANCE_BirthWithThread(DYNAMIC_BOMB, .., RB_MovingExplosive_ThTick)",
        animation: STATIC,
    },
    Spec {
        key: "missile",
        pack: Pack::Shared,
        name: "rocket",
        id: 0x29,
        scale: 0x1000,
        source: "shared.mpk rocket (DYNAMIC_ROCKET 0x29); VehPickupItem.c:738 INSTANCE_BirthWithThread(DYNAMIC_ROCKET, \"bombtracker1\", .., RB_MovingExplosive_ThTick)",
        animation: LOOP,
    },
    Spec {
        key: "tnt",
        pack: Pack::Shared,
        name: "crate_tnt",
        id: 0x27,
        scale: 0x1000,
        source: "shared.mpk crate_tnt (STATIC_CRATE_TNT 0x27); VehPickupItem.c:1149, dropped with < 10 Wumpa",
        animation: STATIC,
    },
    Spec {
        key: "nitro",
        pack: Pack::Shared,
        name: "crate_nitro",
        id: 0x06,
        scale: 0x1000,
        source: "shared.mpk crate_nitro (stored under id 0x06 = PU_EXPLOSIVE_CRATE); VehPickupItem.c:1145, the juiced TNT (>= 10 Wumpa)",
        animation: STATIC,
    },
    Spec {
        key: "crate_explosion",
        pack: Pack::Shared,
        name: "crate_explosion",
        id: 0x26,
        scale: 0x1000,
        source: "shared.mpk crate_explosion (STATIC_CRATE_EXPLOSION 0x26); RB_Crate.c: spawned at a broken crate's matrix with colorRGBA 0xF2953A0, RB_CrateAny_ThTick_Explode",
        animation: ONCE,
    },
    Spec {
        key: "beaker_green",
        pack: Pack::Shared,
        name: "beakergreen",
        id: 0x47,
        scale: 0x1000,
        source: "shared.mpk beakergreen (STATIC_BEAKER_GREEN 0x47); VehPickupItem.c:1327, the potion dropped with < 10 Wumpa",
        animation: LOOP,
    },
    Spec {
        key: "beaker_red",
        pack: Pack::Shared,
        name: "beakerred",
        id: 0x46,
        scale: 0x1000,
        source: "shared.mpk beakerred (STATIC_BEAKER_RED 0x46); VehPickupItem.c:1314, the juiced potion (>= 10 Wumpa)",
        animation: LOOP,
    },
    Spec {
        key: "cloud",
        pack: Pack::Shared,
        name: "cloud",
        id: 0x42,
        scale: 0x1000,
        source: "shared.mpk cloud (STATIC_CLOUD 0x42); RB_MaskShieldCloud.c:577, the rain cloud over a driver that hit a potion",
        animation: LOOP,
    },
    Spec {
        key: "shockwave_red",
        pack: Pack::Shared,
        name: "shockwavered",
        id: 0x44,
        scale: 0x1000,
        source: "shared.mpk shockwavered (STATIC_SHOCKWAVE_RED 0x44); RB_Blowup.c:112, the TNT blast ring (others use the green one)",
        animation: ONCE,
    },
    Spec {
        key: "shockwave_green",
        pack: Pack::Shared,
        name: "shockwavegreen",
        id: 0x45,
        scale: 0x1000,
        source: "shared.mpk shockwavegreen (STATIC_SHOCKWAVE_GREEN 0x45); RB_Blowup.c:112, the blast ring of non-TNT explosions",
        animation: ONCE,
    },
    Spec {
        key: "shield_dark",
        pack: Pack::Shared,
        name: "shielddark",
        id: 0x5A,
        scale: 0x700,
        source: "shared.mpk shielddark (DYNAMIC_SHIELD_DARK 0x5A); VehPickupItem.c:929, the shield's base instance at SHIELD_SCALE 0x700 with alphaScale 0x400",
        animation: STATIC,
    },
    Spec {
        key: "shield_green",
        pack: Pack::Shared,
        name: "shield_green",
        id: 0x5E,
        scale: 0x700,
        source: "shared.mpk shield_green (DYNAMIC_SHIELD_GREEN 0x5E); VehPickupItem.c:943, the colour layer of the normal shield (< 10 Wumpa), scale 0x700",
        animation: STATIC,
    },
    Spec {
        key: "shield_blue",
        pack: Pack::Shared,
        name: "shield",
        id: 0x56,
        scale: 0x700,
        source: "shared.mpk shield (DYNAMIC_SHIELD 0x56); VehPickupItem.c:939, the colour layer of the juiced shield (>= 10 Wumpa, SHIELD_FLAG_BLUE, no time limit), scale 0x700",
        animation: STATIC,
    },
    Spec {
        key: "shield_highlight",
        pack: Pack::Shared,
        name: "highlight",
        id: 0x5D,
        scale: 0x700,
        source: "shared.mpk highlight (DYNAMIC_HIGHLIGHT 0x5D); VehPickupItem.c:957, the shield's rotating highlight, scale 0x700",
        animation: STATIC,
    },
    Spec {
        key: "aku",
        pack: Pack::Shared,
        name: "akumouth",
        id: 0x39,
        scale: 0x1000,
        source: "shared.mpk akumouth (STATIC_AKUAKU 0x39); VehPickupItem.c:281, the mask over a hero driver (MASK_HEAD_SCALE_NORMAL = 1.0)",
        animation: "speak: the mouth while the mask talks; stored at half rate (see game_frames)",
    },
    Spec {
        key: "akubeam",
        pack: Pack::Shared,
        name: "akubeam",
        id: 0x3E,
        scale: 0x1000,
        source: "shared.mpk akubeam (STATIC_AKUBEAM 0x3E); VehPickupItem.c:297/323, the beam cone under the Aku Aku mask",
        animation: LOOP,
    },
    Spec {
        key: "uka",
        pack: Pack::Shared,
        name: "ukamouth",
        id: 0x3A,
        scale: 0x1000,
        source: "shared.mpk ukamouth (STATIC_UKAUKA 0x3A); VehPickupItem.c:302, the mask over a villain driver",
        animation: "speak: the mouth while the mask talks; stored at half rate (see game_frames)",
    },
    Spec {
        key: "ukabeam",
        pack: Pack::Shared,
        name: "ukabeam",
        id: 0x40,
        scale: 0x1000,
        source: "shared.mpk ukabeam (STATIC_UKABEAM 0x40); VehPickupItem.c:318/323, the beam cone under the Uka Uka mask",
        animation: LOOP,
    },
    Spec {
        key: "warpball",
        pack: Pack::Shared,
        name: "warpball",
        id: 0x36,
        scale: 0x1000,
        source: "shared.mpk warpball (DYNAMIC_WARPBALL 0x36); VehPickupItem.c:1015, RB_Warpball_ThTick",
        animation: LOOP,
    },
    Spec {
        key: "warpedburst",
        pack: Pack::Shared,
        name: "warpedburst",
        id: 0x2B,
        scale: 0x1000,
        source: "shared.mpk warpedburst (STATIC_WARPEDBURST 0x2B, instance name \"explosion2\"); RB_Burst.c:251, the bomb/missile/warp orb explosion",
        animation: ONCE,
    },
];

#[derive(Serialize)]
pub struct WeaponModel {
    pub source: &'static str,
    pub model: String,
    pub model_id: i16,
    /// The `_hi` mesh the frames come from.
    pub mesh: String,
    /// The animation the frames come from (`None` for a static mesh).
    pub animation_name: Option<String>,
    pub frames: Vec<MeshFrame>,
    /// Animation length in game frames. Equals `frames.len()` unless the model stores only
    /// every other frame (`frames.len() == game_frames / 2 + 1`); the game then interpolates
    /// between stored frames, so stored frame `i` shows at game frame `2 i`.
    pub game_frames: usize,
    pub frame_rate_fps: f32,
    pub animation: &'static str,
    pub instance_scale: f32,
    /// Axis-aligned bounds over all frames, meters at scale 1.0.
    pub bounds: Value,
}

#[derive(Serialize)]
pub struct WeaponsFile {
    pub format: &'static str,
    pub version: u32,
    pub units: Value,
    pub textures: Vec<Texture>,
    pub models: BTreeMap<&'static str, WeaponModel>,
    pub constants: Value,
}

/// Offsets (into the unwrapped level data) of a level file's models, with names and ids.
/// Level header (CTR-tools `SceneHeader`): `u32 num_models @0x14`, `u32 ptr_models @0x18`
/// pointing at `num_models` model pointers.
pub fn level_models(data: &[u8]) -> Result<Vec<(usize, String, i16)>> {
    let b = Bytes(data);
    let n = b.u32(0x14)? as usize;
    let ptr = b.u32(0x18)? as usize;
    if n > 256 {
        return Err(Error(format!("level: implausible model count {n}")));
    }
    (0..n)
        .map(|i| {
            let at = b.u32(ptr + i * 4)? as usize;
            Ok((at, b.name(at)?, b.i16(at + 0x10)?))
        })
        .collect()
}

fn bounds(frames: &[MeshFrame]) -> Value {
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    for v in frames
        .iter()
        .flat_map(|f| &f.surfaces)
        .flat_map(|s| &s.vertices)
    {
        for k in 0..3 {
            lo[k] = lo[k].min(v.position[k]);
            hi[k] = hi[k].max(v.position[k]);
        }
    }
    json!({ "min": lo, "max": hi })
}

pub fn build(bf: &mut BigFile) -> Result<WeaponsFile> {
    // The level's VRM goes on top of shared.vrm, as the game loads them.
    let mut vram = Vram::new();
    vram.load_vrm(&bf.read(big::index::SHARED_VRM)?)?;
    vram.load_vrm(&bf.read(big::index::CRASH_COVE_1P_VRM)?)?;
    let mpk_file = bf.read(big::index::SHARED_MPK)?;
    let shared = unwrap_patched(&mpk_file)?;
    let shared_models = mpk::models(&mpk_file)?;
    let lev_file = bf.read(big::index::CRASH_COVE_1P_LEV)?;
    let level = unwrap_patched(&lev_file)?;
    let lev_models = level_models(level)?;
    let mut tex = Textures::default();

    let mut models = BTreeMap::new();
    for spec in &MODELS {
        let (data, list, pack) = match spec.pack {
            Pack::Shared => (shared, &shared_models, "shared.mpk"),
            Pack::Level => (level, &lev_models, "Crash Cove data.lev"),
        };
        let (at, _, id) = list
            .iter()
            .find(|m| m.1 == spec.name)
            .ok_or_else(|| Error(format!("{pack} has no model {}", spec.name)))?;
        if *id != spec.id {
            return Err(Error(format!(
                "{pack} {}: model id {id:#x}, expected {:#x}",
                spec.name, spec.id
            )));
        }
        let model = Model::parse_at(data, *at)?;
        let mesh = model
            .meshes
            .first()
            .ok_or_else(|| Error(format!("{}: no mesh", spec.name)))?;
        if mesh.anims.len() > 1 {
            return Err(Error(format!(
                "{}: {} animations, expected at most one",
                spec.name,
                mesh.anims.len()
            )));
        }
        // An animation whose frames are all the same vertices (the fruit's `spin`) is exported
        // as its one distinct frame.
        let anim = mesh
            .anims
            .first()
            .filter(|a| a.frames.iter().any(|f| *f != a.frames[0]));
        let frames = match anim {
            None => vec![mesh_frame_at(spec.key, mesh, &mesh.frame, &vram, &mut tex)?],
            Some(a) => a
                .frames
                .iter()
                .enumerate()
                .map(|(i, f)| {
                    mesh_frame_at(
                        &format!("{}_{}{i}", spec.key, a.name),
                        mesh,
                        f,
                        &vram,
                        &mut tex,
                    )
                })
                .collect::<Result<_>>()?,
        };
        models.insert(
            spec.key,
            WeaponModel {
                source: spec.source,
                model: model.name.clone(),
                model_id: spec.id,
                mesh: mesh.name.clone(),
                animation_name: anim.map(|a| a.name.clone()),
                bounds: bounds(&frames),
                game_frames: anim.map_or(1, |a| a.game_frames),
                frames,
                frame_rate_fps: FPS,
                animation: spec.animation,
                instance_scale: spec.scale as f32 / 4096.0,
            },
        );
    }

    let constants = json!({
        "roulette": {
            "seconds": 90.0 / FPS,
            "frames": 90,
            "fast_stop_after_frames": 70,
            "source": "RB_Crate.c:198 itemRollTimer = 90; VehPhysProc.c:13 ITEM_ROLL_FAST_STOP_FRAMES = 70 (a fire press after 70 frames stops it early)",
            "sounds": "OtherFX 0x5D loops while it spins (RB_Crate.c:202), stopped by UI_RenderFrame.c:875; then 0x5E (normal) or 0x41 (>= 10 Wumpa) when the item lands (VehPhysProc.c:21-22)"
        },
        "crate_respawn": {
            "cooldown_seconds": 0x1E as f32 / FPS,
            "regrow_seconds": 16.0 / FPS,
            "source": "RB_Crate.c:125 cooldown = 0x1E frames with the crate at scale 0, then scale grows +0x100 per frame back to 0x1000 (RB_Crate.c:59-69); the cooldown pauses while a driver or mine sits in the crate"
        },
        "fruit_crate_wumpa": { "min": 5, "max": 8, "source": "RB_Crate.c:316 rand % 4 + 5" },
        "loose_fruit": { "wumpa": 1, "respawns": false, "source": "RB_Fruit.c: scale set to 0 and the thread dies on pickup" },
        "wumpa_juiced": { "count": 10, "max": 10, "source": "namespace_Vehicle.h:668 DRIVER_WUMPA_JUICED_COUNT = 10 = DRIVER_WUMPA_MAX_COUNT" },
        "item_choice": "rank-weighted random: VehPhysGeneral_SetHeldItem (VehPhysGeneral.c)"
    });

    Ok(WeaponsFile {
        format: "ctr-weapons",
        version: 1,
        units: json!({
            "length": format!("meters, +Y up; 1 CTR world unit = 1/{WORLD_PER_METER} m; positions are at instance scale 1.0 (multiply by instance_scale to get the in-game size)"),
            "time": "seconds; the game advances animations once per frame at 30 fps",
            "colors": "0..1 multipliers; for textured meshes 1.0 = PSX 0x80 (neutral); untextured (Gouraud) vertex colours are the colour, 1.0 = 0xFF",
            "textures": "RGBA8 rows top-first; alpha 0 = transparent (PSX black), 128 = texel blends with the surface's blend mode (STP bit), 255 = texel drawn opaque even on a blended surface"
        }),
        textures: tex.list,
        models,
        constants,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_keys_are_unique() {
        let mut keys: Vec<_> = MODELS.iter().map(|m| m.key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), MODELS.len());
    }

    #[test]
    fn reads_level_model_list() {
        // Header: num_models @0x14 = 1, ptr_models @0x18 = 0x20 -> model at 0x24.
        let mut d = vec![0u8; 0x24 + 0x18];
        d[0x14..0x18].copy_from_slice(&1u32.to_le_bytes());
        d[0x18..0x1C].copy_from_slice(&0x20u32.to_le_bytes());
        d[0x20..0x24].copy_from_slice(&0x24u32.to_le_bytes());
        d[0x24..0x29].copy_from_slice(b"fruit");
        d[0x34..0x36].copy_from_slice(&2i16.to_le_bytes());
        assert_eq!(level_models(&d).unwrap(), vec![(0x24, "fruit".into(), 2)]);
    }
}
