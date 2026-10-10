//! The kart's sounds (`kartcraft::sound`) through the client's mixer, at its effects volume.

use bevy::prelude::*;
pub(super) use kartcraft::sound::KartAudio;
use kartcraft::sound::Sounds;

use super::KartDrive;

pub(super) fn update(
    drive: Res<KartDrive>,
    mut audio: ResMut<KartAudio>,
    config: Option<Res<crate::sound::SoundConfig>>,
    output: Option<NonSendMut<crate::sound::SoundOutput>>,
    mut sounds: Local<Sounds>,
) {
    let Some(mixer) = output.and_then(|o| o.into_inner().mixer.as_mut()) else {
        audio.cues.clear();
        return;
    };
    let sfx = config
        .as_ref()
        .map_or(1.0, |c| if c.enabled { c.sfx } else { 0.0 });
    audio.music = config
        .as_ref()
        .map_or(1.0, |c| if c.enabled { c.music } else { 0.0 });
    sounds.update(&mut audio, drive.active, sfx, |data| {
        mixer.play_2d(data).ok()
    });
}

/// Other drivers' karts within this many yards are heard.
const HEARD_WITHIN: f32 = 60.0;

/// One remote kart's engine: its spatial track and the loop on it.
pub(super) struct RemoteEngine {
    track: kira::track::SpatialTrackHandle,
    sound: kira::sound::static_sound::StaticSoundHandle,
}

/// Other drivers' engines, placed at their karts, pitched from the state they relay (CTR's own
/// engine model on their side), fading out with distance.
#[allow(clippy::type_complexity)]
pub(super) fn remote_engines(
    config: Option<Res<crate::sound::SoundConfig>>,
    output: Option<NonSendMut<crate::sound::SoundOutput>>,
    karts: Query<
        (Entity, &Transform, &kartcraft::model::KartStatus),
        (
            With<kartcraft::model::KartRoot>,
            Without<kartcraft::model::LocalKart>,
        ),
    >,
    camera: Query<&GlobalTransform, With<benilla_world::view::WorldCamera>>,
    mut engines: Local<std::collections::HashMap<Entity, RemoteEngine>>,
) {
    let Some(mixer) = output.and_then(|o| o.into_inner().mixer.as_mut()) else {
        return;
    };
    let sfx = config.map_or(1.0, |c| if c.enabled { c.sfx } else { 0.0 });
    let listener = camera.single().map(|c| c.translation()).unwrap_or_default();
    let mut live = Vec::new();
    for (kart, t, status) in &karts {
        let distance = t.translation.distance(listener);
        if distance > HEARD_WITHIN {
            continue;
        }
        live.push(kart);
        let engine = match engines.get_mut(&kart) {
            Some(e) => e,
            None => {
                let data = kartcraft::sound::engine_loop()
                    .loop_region(..)
                    .volume(kira::Decibels::SILENCE);
                let Ok((track, sound)) = mixer.play_3d(data, t.translation, false) else {
                    continue;
                };
                engines.entry(kart).or_insert(RemoteEngine { track, sound })
            }
        };
        crate::sound::set_track_position(&mut engine.track, t.translation);
        let near = (1.0 - distance / HEARD_WITHIN).clamp(0.0, 1.0);
        let level = 0.6 * f32::from(status.engine.1) / 230.0 * near * near * sfx;
        let tween = kira::Tween {
            duration: std::time::Duration::from_millis(60),
            ..Default::default()
        };
        let db = if level <= 0.0001 {
            kira::Decibels::SILENCE
        } else {
            kira::Decibels(20.0 * level.log10())
        };
        engine.sound.set_volume(db, tween);
        engine
            .sound
            .set_playback_rate(kartcraft::sound::engine_rate(status.engine.0), tween);
    }
    engines.retain(|kart, engine| {
        let keep = live.contains(kart);
        if !keep {
            engine.sound.stop(kira::Tween::default());
        }
        keep
    });
}
