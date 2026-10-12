//! The kart's sounds (`kartcraft::sound`) through the client's mixer, at its effects volume.

use bevy::prelude::*;
pub(super) use kartcraft::sound::KartAudio;
use kartcraft::sound::Sounds;

use super::KartDrive;

/// The pass line (`PlayLevel.c:391-415`): once a race is 0x4B00 ms old, when someone passes us
/// the racer now one ahead says it, in their kart's voice (Crash's when their kart is not
/// relayed); when we pass someone, we say it (CTR's check from the other player's side).
pub(super) fn overtakes(
    time: Res<Time>,
    races: Res<super::race::Races>,
    remotes: Res<super::remote::RemoteKarts>,
    names: Option<Res<crate::names::NameCache>>,
    mut audio: ResMut<KartAudio>,
    mut last: Local<Option<(u32, usize)>>,
) {
    use kartcraft::voice::{line, overtake, Overtake};
    let Some((run, go)) = races
        .run()
        .filter(|r| !r.time_trial && !r.done)
        .and_then(|r| Some((r.id, r.go_at?)))
    else {
        *last = None;
        return;
    };
    let order = races.order();
    let Some(place) = order.iter().position(|r| *r == races.me).map(|i| i + 1) else {
        return;
    };
    let before = last.filter(|(id, _)| *id == run).map_or(0, |(_, p)| p);
    *last = Some((run, place));
    match overtake(before, place, (time.elapsed_secs() - go) * 1000.0) {
        Some(Overtake::Passed(at)) => {
            let passer = order[at - 1].as_str();
            let racer = remotes
                .karts
                .values()
                .find(|k| names.as_ref().and_then(|n| n.peek(k.guid)) == Some(passer))
                .map_or("", |k| remotes.racer(k.guid));
            audio.say_as(line::OVERTAKE, racer);
        }
        Some(Overtake::Passing) => audio.say(line::OVERTAKE),
        None => {}
    }
}

pub(super) fn update(
    drive: Res<KartDrive>,
    mut audio: ResMut<KartAudio>,
    config: Option<Res<crate::sound::SoundConfig>>,
    settings: Res<super::settings::KartSettings>,
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
    // CTR's music at its own share (`KartMusic`, `KartMusicVolume`) of the client's music volume.
    audio.music = config
        .as_ref()
        .map_or(1.0, |c| if c.enabled { c.music } else { 0.0 })
        * settings.music_level();
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
            Without<kartcraft::model::GhostKart>,
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
