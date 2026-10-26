//! The kart's horn: Fire with no item held sounds it, and the drivers around hear it at our kart.
//! The horn is the one we wear (`kartcraft::goods::HORNS`, a sound of the player's own install;
//! CTR's honk none worn). Ours plays at once and goes to the server as "H", which sounds it to
//! the drivers within its range as "KARTCRAFT\tH\t<horn>" from us (`Kartcraft.cpp` RelayHorn),
//! at most once in [`HORN_GAP`]: a tapped honk, each press cutting the last one short.

use std::collections::HashMap;

use benilla_assets::WorldAssets;
use benilla_protocol::SessionEvent;
use bevy::prelude::*;
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle};

use super::profession::Profession;
use super::remote::RemoteKarts;

/// The longest a horn plays (s).
const HORN_SECS: f32 = 2.5;
/// The least between two of ours (s): the server's HORN_INTERVAL, and a little more.
const HORN_GAP: f32 = 0.3;
/// A horn cut short by the same driver's next fades this long (ms).
const HORN_CUT_MS: u64 = 40;
/// A horn cut at [`HORN_SECS`] fades this long (ms).
const HORN_FADE_MS: u64 = 400;
/// Another driver's horn is heard this far (yards), the server's HORN_RANGE, fading with it.
const HEARD_WITHIN: f32 = 100.0;
/// A horn's level at its kart, of the effects volume.
const LEVEL: f32 = 0.8;

#[derive(Resource, Default)]
pub(crate) struct Horn {
    /// Fire pressed this frame with no item held, in the kart (`super::update`).
    pub(crate) pressed: bool,
    /// When ours last sounded.
    last: Option<f32>,
    /// Other drivers' horns come over the wire: the driver's guid and the horn's index.
    heard: Vec<(u64, u8)>,
}

/// Other drivers' horns.
pub(crate) fn on_chat(In(ev): In<SessionEvent>, mut horn: ResMut<Horn>) {
    let SessionEvent::Chat(m) = ev else {
        return;
    };
    if !m.is_addon() {
        return;
    }
    let Some(index) = m.text.strip_prefix("KARTCRAFT\tH\t") else {
        return;
    };
    horn.heard
        .push((m.sender_guid, index.trim().parse().unwrap_or(0)));
}

/// One horn playing: whose kart (`None` ours), its spatial track, the sound, and when it began.
pub(super) struct Playing {
    kart: Option<Entity>,
    track: Option<kira::track::SpatialTrackHandle>,
    sound: StaticSoundHandle,
    began: f32,
}

/// Each horn's sound, decoded once; `None` for one the install lacks.
pub(super) type Decoded = HashMap<u8, Option<StaticSoundData>>;

#[allow(clippy::too_many_arguments)]
pub(super) fn update(
    time: Res<Time>,
    mut horn: ResMut<Horn>,
    profession: Profession,
    remotes: Res<RemoteKarts>,
    karts: Query<&GlobalTransform>,
    camera: Query<&GlobalTransform, With<benilla_world::view::WorldCamera>>,
    assets: Option<Res<WorldAssets>>,
    config: Option<Res<crate::sound::SoundConfig>>,
    output: Option<NonSendMut<crate::sound::SoundOutput>>,
    (mut decoded, mut playing): (Local<Decoded>, Local<Vec<Playing>>),
) {
    let now = time.elapsed_secs();
    let horn = &mut *horn;
    let ours = std::mem::take(&mut horn.pressed)
        && horn.last.is_none_or(|t| now - t >= HORN_GAP)
        && profession.server_kart();
    if ours {
        horn.last = Some(now);
        profession.send("KARTCRAFT\tH".to_string());
        info!("kart horn: sounded horn {}", kartcraft::goods::our_horn());
    }
    let heard = std::mem::take(&mut horn.heard);
    for (guid, index) in &heard {
        info!("kart horn: heard horn {index} from {guid:#x}");
    }
    let Some(mixer) = output.and_then(|o| o.into_inner().mixer.as_mut()) else {
        return;
    };
    let sfx = config.map_or(1.0, |c| if c.enabled { c.sfx } else { 0.0 });
    let listener = camera.single().map(|c| c.translation()).unwrap_or_default();
    let mut sound = |index: u8| -> Option<StaticSoundData> {
        decoded
            .entry(index)
            .or_insert_with(|| {
                let Some(path) = kartcraft::goods::horn_sound(index) else {
                    let honk = kartcraft::sound::honk();
                    if honk.is_none() {
                        warn!("kart horn: no CTR honk (kart-data/audio/tnt_honk.wav)");
                    }
                    return honk;
                };
                let bytes = assets.as_ref()?.read_file_or_loose(path)?;
                crate::sound::sfx_from_bytes(bytes)
                    .map_err(|e| warn!("kart horn: {path}: {e:#}"))
                    .ok()
            })
            .clone()
    };
    if ours {
        if let Some(data) = sound(kartcraft::goods::our_horn()) {
            let data = data.volume(crate::sound::amp_to_db(LEVEL * sfx));
            cut(&mut playing, None);
            if let Ok(handle) = mixer.play_2d(data) {
                playing.push(Playing {
                    kart: None,
                    track: None,
                    sound: handle,
                    began: now,
                });
            }
        }
    }
    for (guid, index) in heard {
        let Some(kart) = remotes
            .karts
            .values()
            .find(|r| r.guid == guid)
            .map(|r| r.kart)
        else {
            continue;
        };
        let Ok(at) = karts.get(kart).map(|t| t.translation()) else {
            continue;
        };
        let near = 1.0 - at.distance(listener) / HEARD_WITHIN;
        let Some(data) = sound(index).filter(|_| near > 0.0) else {
            continue;
        };
        let data = data.volume(crate::sound::amp_to_db(LEVEL * near * sfx));
        cut(&mut playing, Some(kart));
        if let Ok((track, handle)) = mixer.play_3d(data, at, false) {
            playing.push(Playing {
                kart: Some(kart),
                track: Some(track),
                sound: handle,
                began: now,
            });
        }
    }
    // Each follows its kart, and stops at HORN_SECS.
    playing.retain_mut(|p| {
        if p.sound.state() == kira::sound::PlaybackState::Stopped {
            return false;
        }
        if let (Some(track), Some(at)) = (
            p.track.as_mut(),
            p.kart
                .and_then(|k| karts.get(k).ok())
                .map(|t| t.translation()),
        ) {
            crate::sound::set_track_position(track, at);
        }
        if now - p.began >= HORN_SECS {
            p.sound.stop(crate::sound::fade(HORN_FADE_MS));
            return false;
        }
        true
    });
}

/// Stops the horn `kart` is playing (`None` ours): its driver's next one takes over.
fn cut(playing: &mut Vec<Playing>, kart: Option<Entity>) {
    playing.retain_mut(|p| {
        if p.kart != kart {
            return true;
        }
        p.sound.stop(crate::sound::fade(HORN_CUT_MS));
        false
    });
}
