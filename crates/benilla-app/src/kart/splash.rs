//! A kart landing on the water splashes: WoW's own splash of a body entering water, of the
//! player's install, its size by how hard the kart came down (CTR has no water to land in; its
//! landing thud is the road's, `kartcraft::sound`). Ours plays as we land; another driver's where
//! their kart is, when the line they relay first says they are on the water after the air.

use std::collections::HashMap;

use benilla_assets::WorldAssets;
use bevy::prelude::*;
use kira::sound::static_sound::StaticSoundData;

/// The splashes, small to big (`Sound\Character\Footsteps\EnterWaterSplash`).
const SPLASHES: [&str; 3] = [
    r"Sound\Character\Footsteps\EnterWaterSplash\EnterWaterSmallA.wav",
    r"Sound\Character\Footsteps\EnterWaterSplash\EnterWaterMediumA.wav",
    r"Sound\Character\Footsteps\EnterWaterSplash\EnterWaterGiantA.wav",
];
/// The falls (speed units downward) from which the medium and the big one play: CTR's landing
/// sparks' 22.0, and three times it.
const MEDIUM_FALL: i32 = 22 * 256;
const BIG_FALL: i32 = 66 * 256;
/// Another driver's splash is heard this far (yards), fading with it.
const HEARD_WITHIN: f32 = 60.0;
/// A splash's level, of the effects volume.
const LEVEL: f32 = 0.9;

/// The splashes due: whose kart (`None` ours) and which ([`SPLASHES`]).
#[derive(Resource, Default)]
pub(crate) struct Splashes(pub(crate) Vec<(Option<Entity>, usize)>);

/// The splash for a landing `fall` speed units hard.
pub(crate) fn size(fall: i32) -> usize {
    if fall >= BIG_FALL {
        2
    } else if fall >= MEDIUM_FALL {
        1
    } else {
        0
    }
}

pub(super) fn play(
    mut splashes: ResMut<Splashes>,
    karts: Query<&GlobalTransform>,
    camera: Query<&GlobalTransform, With<benilla_world::view::WorldCamera>>,
    assets: Option<Res<WorldAssets>>,
    config: Option<Res<crate::sound::SoundConfig>>,
    output: Option<NonSendMut<crate::sound::SoundOutput>>,
    mut decoded: Local<HashMap<usize, Option<StaticSoundData>>>,
) {
    let due = std::mem::take(&mut splashes.0);
    if due.is_empty() {
        return;
    }
    let Some(mixer) = output.and_then(|o| o.into_inner().mixer.as_mut()) else {
        return;
    };
    let sfx = config.map_or(1.0, |c| if c.enabled { c.sfx } else { 0.0 });
    let listener = camera.single().map(|c| c.translation()).unwrap_or_default();
    for (kart, which) in due {
        let Some(data) = decoded
            .entry(which)
            .or_insert_with(|| {
                let path = SPLASHES[which.min(SPLASHES.len() - 1)];
                let bytes = assets.as_ref()?.read_file_or_loose(path)?;
                crate::sound::sfx_from_bytes(bytes)
                    .map_err(|e| warn!("kart splash: {path}: {e:#}"))
                    .ok()
            })
            .clone()
        else {
            continue;
        };
        match kart
            .and_then(|k| karts.get(k).ok())
            .map(|t| t.translation())
        {
            None if kart.is_none() => {
                let _ = mixer.play_2d(data.volume(crate::sound::amp_to_db(LEVEL * sfx)));
            }
            Some(at) => {
                let near = 1.0 - at.distance(listener) / HEARD_WITHIN;
                if near > 0.0 {
                    let data = data.volume(crate::sound::amp_to_db(LEVEL * near * sfx));
                    let _ = mixer.play_3d(data, at, false);
                }
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_harder_landing_splashes_bigger() {
        assert_eq!(size(0), 0);
        assert_eq!(size(MEDIUM_FALL), 1);
        assert_eq!(size(BIG_FALL + 1), 2);
    }

    /// Every splash is a sound the install has.
    #[test]
    fn the_splashes_are_the_install_s() {
        let data = benilla_formats::wow_data_or_skip!();
        let mut chain = benilla_formats::open_chain(&data).expect("the install's archives");
        for path in SPLASHES {
            let bytes = chain
                .read_file(path)
                .unwrap_or_else(|e| panic!("{path}: {e:#}"));
            crate::sound::sfx_from_bytes(bytes).unwrap_or_else(|e| panic!("{path}: {e:#}"));
        }
    }
}
