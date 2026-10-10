//! Races between karts, refereed by the Kartcraft server (`Kartcraft.cpp`, "Races"). At a
//! course's start F5 hosts a race; drivers near it see the offer and F5 joins; the host's F5
//! starts it. The server sends the course's checkpoints and a countdown: CTR's 3-2-1-GO, the kart
//! held on the line, with CTR's start boost for an engine revved just before GO. Each checkpoint
//! passed is reported, and the server checks it against where we really are; it relays everyone's
//! progress, from which our place is worked out, and the finishes with their times. Checkpoints
//! stand in the world as beams of light, the next one bright. F6 leaves a race.

use std::collections::HashMap;

use benilla_assets::coords::{bevy_to_wow, wow_to_bevy};
use benilla_protocol::SessionEvent;
use benilla_world::collision::WorldCollision;
use bevy::prelude::*;

/// The courses' starts, so a driver at one may host (the server's table, `Kartcraft.cpp`,
/// holds the whole course and sends it at the start).
const COURSES: [(&str, &str, [f32; 2]); 3] = [
    ("sprint", "Northshire Sprint", [-8930.0, -118.0]),
    ("elwynn", "Elwynn Loop", [-9436.0, 59.0]),
    ("stormwind", "Road to Stormwind", [-9436.0, 59.0]),
];
/// How near a start (yards) a driver may host or join, and how near a checkpoint counts as
/// passing it (the server allows 45).
const START_RADIUS: f32 = 55.0;
const CHECKPOINT_RADIUS: f32 = 38.0;
/// The start boost: revving this long before GO (s) is big, a little earlier small, earlier
/// still bogs the engine down.
const BOOST_BIG: std::ops::Range<f32> = 0.15..0.6;
const BOOST_SMALL: std::ops::Range<f32> = 0.6..1.2;
/// A checkpoint's beam: its height (yards) and radius.
const BEAM_HEIGHT: f32 = 40.0;
const BEAM_RADIUS: f32 = 1.2;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Offer {
    id: u32,
    name: String,
    host: String,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Active {
    id: u32,
    laps: u8,
    /// Checkpoints in WoW x, y; the first is the start (and finish of a lap).
    points: Vec<[f32; 2]>,
    /// When GO falls, local seconds; `None` until the countdown arrives.
    go_at: Option<f32>,
    roster: Vec<String>,
    host: bool,
    lap: u8,
    checkpoint: usize,
    /// Each racer's (laps, checkpoint, when) as the server relays it.
    progress: HashMap<String, (u8, u8, f32)>,
    /// Finishes: (name, place, ms).
    finished: Vec<(String, u8, u32)>,
    done: bool,
    /// When the gas went down during the countdown (local s), for the start boost.
    revved_at: Option<f32>,
    beeped: i32,
}

#[derive(Resource, Default)]
pub(crate) struct Races {
    pub(crate) offer: Option<Offer>,
    pub(crate) race: Option<Active>,
    /// Our character's name, to find ourselves in rosters.
    pub(crate) me: String,
    /// Lines from the server not yet handled.
    inbox: Vec<Vec<String>>,
}

impl Races {
    /// The next checkpoint of a race under way, Bevy world (ground height unknown: y 0).
    pub(crate) fn next_target(&self, now: f32) -> Option<Vec3> {
        let race = self.race.as_ref()?;
        if race.done || race.go_at.is_none_or(|go| now < go) || race.points.len() < 2 {
            return None;
        }
        let n = race.points.len();
        let next = if race.laps > 0 {
            (race.checkpoint + 1) % n
        } else {
            (race.checkpoint + 1).min(n - 1)
        };
        let p = race.points[next];
        Some(wow_to_bevy([p[0], p[1], 0.0]))
    }

    /// Whether the kart must hold on the line: a countdown is running.
    pub(crate) fn holding(&self, now: f32) -> bool {
        self.race
            .as_ref()
            .is_some_and(|r| r.go_at.is_some_and(|go| now < go))
    }
}

/// Race lines from the server.
pub(crate) fn on_chat(In(ev): In<SessionEvent>, mut races: ResMut<Races>) {
    let SessionEvent::Chat(m) = ev else {
        return;
    };
    if !m.is_addon() {
        return;
    }
    if let Some(body) = m.text.strip_prefix("KARTCRAFT\tR\t") {
        races
            .inbox
            .push(body.split('\t').map(str::to_string).collect());
    }
}

/// Where a racer stands in the race, for ordering: laps, then checkpoints, then who got there
/// first.
fn standing(progress: &(u8, u8, f32), points: usize) -> (i64, f32) {
    (
        i64::from(progress.0) * points as i64 + i64::from(progress.1),
        -progress.2,
    )
}

/// What the race shows on screen this frame: a prompt or countdown at the top, results below.
#[derive(Resource, Default)]
pub(crate) struct RaceText {
    pub(crate) banner: String,
    pub(crate) lines: Vec<String>,
}

/// Handles the server's lines, the keys, the countdown, checkpoints and standings.
#[allow(clippy::too_many_arguments)]
pub(crate) fn update(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    ui: Res<crate::ui_script::UiKeyboardCapture>,
    drive: Res<super::KartDrive>,
    mut races: ResMut<Races>,
    mut text: ResMut<RaceText>,
    mut extras: ResMut<kartcraft::hud::HudExtras>,
    mut audio: ResMut<kartcraft::sound::KartAudio>,
    profession: super::profession::Profession,
    mut auto: Local<AutoRace>,
) {
    let now = time.elapsed_secs();
    let races = &mut *races;
    let me = races.me.clone();

    // The server's lines.
    for f in std::mem::take(&mut races.inbox) {
        let id = f.get(1).and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
        match f.first().map(String::as_str) {
            Some("O") if f.len() >= 5 => {
                if races.race.is_none() {
                    races.offer = Some(Offer {
                        id,
                        name: f[3].clone(),
                        host: f[4].clone(),
                    });
                }
            }
            Some("L") if f.len() >= 3 => {
                let roster: Vec<String> = f[2].split(',').map(str::to_string).collect();
                if roster.contains(&me) {
                    let race = races.race.get_or_insert_with(|| Active {
                        id,
                        ..Default::default()
                    });
                    race.host = roster.first() == Some(&me);
                    race.roster = roster;
                    races.offer = None;
                } else if races.race.as_ref().is_some_and(|r| r.id == id) {
                    races.race = None;
                }
            }
            Some("C") if f.len() >= 5 => {
                if let Some(race) = races.race.as_mut().filter(|r| r.id == id) {
                    race.laps = f[3].parse().unwrap_or(0);
                    race.points = f[4]
                        .split(';')
                        .filter_map(|p| {
                            let (x, y) = p.split_once(':')?;
                            Some([x.parse().ok()?, y.parse().ok()?])
                        })
                        .collect();
                    info!(
                        "kart race: course {} — {} checkpoints, {} laps",
                        f[2],
                        race.points.len(),
                        race.laps
                    );
                }
            }
            Some("G") if f.len() >= 3 => {
                if let Some(race) = races.race.as_mut().filter(|r| r.id == id) {
                    let ms: f32 = f[2].parse().unwrap_or(4000.0);
                    race.go_at = Some(now + ms / 1000.0);
                    race.lap = 0;
                    race.checkpoint = 0;
                    race.progress.clear();
                    race.finished.clear();
                    race.revved_at = None;
                    race.beeped = 0;
                    info!("kart race: countdown, {} racers", race.roster.len());
                }
            }
            Some("P") if f.len() >= 5 => {
                if let Some(race) = races.race.as_mut().filter(|r| r.id == id) {
                    let lap = f[3].parse().unwrap_or(0);
                    let cp = f[4].parse().unwrap_or(0);
                    race.progress.insert(f[2].clone(), (lap, cp, now));
                }
            }
            Some("F") if f.len() >= 5 => {
                if let Some(race) = races.race.as_mut().filter(|r| r.id == id) {
                    let place = f[3].parse().unwrap_or(0);
                    let ms = f[4].parse().unwrap_or(0);
                    info!("kart race: {} finished {place} in {ms} ms", f[2]);
                    race.finished.push((f[2].clone(), place, ms));
                    if f[2] == me {
                        race.done = true;
                    }
                }
            }
            _ => {}
        }
    }

    // Out of the kart, out of the race.
    if !drive.active {
        if let Some(race) = races.race.take() {
            profession.send(format!("KARTCRAFT\tR\tQ\t{}", race.id));
        }
        races.offer = None;
        *text = RaceText::default();
        extras.lap = None;
        extras.place = None;
        audio.music_rate = 1.0;
        return;
    }
    let wow = bevy_to_wow(drive.pos);
    let at = [wow[0], wow[1]];
    let near = |p: [f32; 2], r: f32| (p[0] - at[0]).hypot(p[1] - at[1]) <= r;

    // F5: join, host, or start; F6: leave. `WOW_KART_RACE` presses F5 for unattended runs.
    let f5 = (keys.just_pressed(KeyCode::F5) && !ui.typing) || auto.press(now, races);
    let f6 = keys.just_pressed(KeyCode::F6) && !ui.typing;
    let here = COURSES.iter().find(|c| near(c.2, START_RADIUS));
    if f6 {
        if let Some(race) = races.race.take() {
            profession.send(format!("KARTCRAFT\tR\tQ\t{}", race.id));
        }
    } else if f5 {
        match (&races.race, &races.offer, here) {
            (Some(race), _, _) if race.host && race.go_at.is_none() => {
                profession.send(format!("KARTCRAFT\tR\tS\t{}", race.id));
            }
            (None, Some(offer), _) => {
                profession.send(format!("KARTCRAFT\tR\tJ\t{}", offer.id));
            }
            (None, None, Some(course)) => {
                profession.send(format!("KARTCRAFT\tR\tH\t{}", course.0));
            }
            _ => {}
        }
    }

    // The countdown, the start boost and the race.
    *text = RaceText::default();
    let Some(race) = races.race.as_mut() else {
        extras.lap = None;
        extras.place = None;
        audio.music_rate = 1.0;
        text.banner = match (&races.offer, here) {
            (Some(o), _) => format!("{}'s {} — F5 to join", o.host, o.name),
            (None, Some(c)) => format!("{} starts here — F5 to host a race", c.1),
            _ => String::new(),
        };
        return;
    };
    let Some(go) = race.go_at else {
        text.banner = format!(
            "Waiting to race: {}{}",
            race.roster.join(", "),
            if race.host {
                " — F5 to start, F6 to leave"
            } else {
                " — F6 to leave"
            }
        );
        return;
    };
    if now < go {
        let left = go - now;
        let beat = left.ceil() as i32;
        if beat <= 3 && beat != race.beeped {
            race.beeped = beat;
            audio.cues.push(kartcraft::sound::Cue::Countdown(false));
        }
        text.banner = if beat <= 3 {
            beat.to_string()
        } else {
            "Ready".into()
        };
        // The gas held from when it went down.
        if drive.revving {
            race.revved_at.get_or_insert(now);
        } else {
            race.revved_at = None;
        }
        return;
    }
    if race.beeped != 0 {
        race.beeped = 0;
        audio.cues.push(kartcraft::sound::Cue::Countdown(true));
        let early = race.revved_at.map(|t| go - t);
        let boost = early.and_then(|e| {
            if BOOST_BIG.contains(&e) {
                Some(true)
            } else if BOOST_SMALL.contains(&e) {
                Some(false)
            } else {
                None
            }
        });
        races_start_boost(boost);
    }
    if now - go < 1.0 {
        text.banner = "GO!".into();
    }

    // Checkpoints: the next one passed is reported (the server checks it).
    let n = race.points.len();
    if n > 1 && !race.done {
        let next = if race.laps > 0 {
            (race.checkpoint + 1) % n
        } else {
            (race.checkpoint + 1).min(n - 1)
        };
        if std::env::var_os("WOW_KART_DEBUG").is_some()
            && (now * 2.0) as i32 != ((now - time.delta_secs()) * 2.0) as i32
        {
            let p = race.points[next];
            info!(
                "kart race debug: next {next} at {p:?}, {:.0} yd away (we are at {at:?})",
                (p[0] - at[0]).hypot(p[1] - at[1])
            );
        }
        if near(race.points[next], CHECKPOINT_RADIUS) {
            info!("kart race: checkpoint {next}, lap {}", race.lap);
            profession.send(format!("KARTCRAFT\tR\tK\t{}\t{next}", race.id));
            race.checkpoint = next;
            if race.laps > 0 && next == 0 {
                race.lap += 1;
            }
            race.progress
                .insert(me.clone(), (race.lap, next as u8, now));
        }
    }

    // Standings: the finished in order, then the rest by progress.
    let mut order: Vec<String> = race.finished.iter().map(|f| f.0.clone()).collect();
    let mut running: Vec<(&String, (i64, f32))> = race
        .roster
        .iter()
        .filter(|r| !order.contains(r))
        .map(|r| {
            let p = race.progress.get(r).copied().unwrap_or((0, 0, go));
            (r, standing(&p, n))
        })
        .collect();
    running.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    order.extend(running.into_iter().map(|(r, _)| r.clone()));
    extras.place = order.iter().position(|r| *r == me).map(|i| i as u8 + 1);
    extras.lap = (race.laps > 0).then(|| ((race.lap + 1).min(race.laps), race.laps));
    audio.music_rate = if race.laps > 1 && race.lap + 1 == race.laps && !race.done {
        // CTR's final lap: 20 BPM on its tempo (about 180).
        200.0 / 180.0
    } else {
        1.0
    };
    if race.done {
        text.banner = match race.finished.iter().find(|f| f.0 == me) {
            Some((_, place, ms)) => format!("{} — {}", ordinal(*place), clock(*ms)),
            None => String::new(),
        };
    }
    text.lines = race
        .finished
        .iter()
        .map(|(name, place, ms)| format!("{}. {name}  {}", place, clock(*ms)))
        .collect();
}

/// `WOW_KART_RACE=host:<course>` hosts at that course's start and starts once another racer is
/// in; `WOW_KART_RACE=join` joins the first race offered. A test hook: it presses F5.
#[derive(Default)]
pub(crate) struct AutoRace {
    since: Option<f32>,
    hosted: bool,
    started: bool,
    joined: bool,
}

impl AutoRace {
    fn press(&mut self, now: f32, races: &Races) -> bool {
        let Ok(spec) = std::env::var("WOW_KART_RACE") else {
            return false;
        };
        let since = *self.since.get_or_insert(now);
        if now - since < 6.0 {
            return false;
        }
        if spec == "join" {
            if !self.joined && races.offer.is_some() && races.race.is_none() {
                self.joined = true;
                return true;
            }
            return false;
        }
        match &races.race {
            None if !self.hosted => {
                self.hosted = true;
                true
            }
            Some(r) if r.host && !self.started && r.roster.len() >= 2 && r.go_at.is_none() => {
                self.started = true;
                true
            }
            _ => false,
        }
    }
}

/// The start boost is fired by the kart update, which holds the session.
pub(crate) static START_BOOST: std::sync::Mutex<Option<bool>> = std::sync::Mutex::new(None);

fn races_start_boost(boost: Option<bool>) {
    if let Some(big) = boost {
        *START_BOOST.lock().unwrap() = Some(big);
    }
}

fn ordinal(place: u8) -> String {
    let suffix = match place {
        1 => "ST",
        2 => "ND",
        3 => "RD",
        _ => "TH",
    };
    format!("{place}{suffix}")
}

fn clock(ms: u32) -> String {
    format!(
        "{}:{:02}.{:02}",
        ms / 60000,
        (ms / 1000) % 60,
        (ms % 1000) / 10
    )
}

/// The checkpoints' beams.
#[derive(Component)]
pub(crate) struct Beam(usize);

/// Stands a beam at each checkpoint of the race we are in, the next one bright.
#[allow(clippy::type_complexity)]
pub(crate) fn beams(
    mut commands: Commands,
    races: Res<Races>,
    collide: WorldCollision,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut beams: Query<(
        Entity,
        &Beam,
        &mut Transform,
        &mut MeshMaterial3d<StandardMaterial>,
    )>,
    mut looks: Local<
        Option<(
            Handle<Mesh>,
            Handle<StandardMaterial>,
            Handle<StandardMaterial>,
        )>,
    >,
) {
    let race = races.race.as_ref().filter(|r| r.go_at.is_some() && !r.done);
    let Some(race) = race else {
        for (e, ..) in &beams {
            commands.entity(e).despawn();
        }
        return;
    };
    let (mesh, next_look, other_look) = looks
        .get_or_insert_with(|| {
            let mesh = meshes.add(Cylinder::new(BEAM_RADIUS, BEAM_HEIGHT));
            let look = |c: Color| StandardMaterial {
                base_color: c,
                unlit: true,
                alpha_mode: AlphaMode::Add,
                cull_mode: None,
                double_sided: true,
                ..default()
            };
            (
                mesh,
                materials.add(look(Color::srgb(1.0, 0.85, 0.2))),
                materials.add(look(Color::srgb(0.18, 0.12, 0.04))),
            )
        })
        .clone();
    let n = race.points.len();
    let next = if race.laps > 0 {
        (race.checkpoint + 1) % n.max(1)
    } else {
        (race.checkpoint + 1).min(n.saturating_sub(1))
    };
    let mut have = vec![false; n];
    for (_, beam, mut t, mut material) in &mut beams {
        let Some(p) = race.points.get(beam.0) else {
            continue;
        };
        have[beam.0] = true;
        let column = wow_to_bevy([p[0], p[1], 0.0]);
        let ground = collide
            .ray_body(Vec3::new(column.x, 2000.0, column.z), Dir3::NEG_Y, 4000.0)
            .map_or(t.translation.y - BEAM_HEIGHT / 2.0, |hit| {
                2000.0 - hit.distance
            });
        t.translation = Vec3::new(column.x, ground + BEAM_HEIGHT / 2.0, column.z);
        material.0 = if beam.0 == next {
            next_look.clone()
        } else {
            other_look.clone()
        };
    }
    for (i, p) in race.points.iter().enumerate() {
        if !have[i] {
            let at = wow_to_bevy([p[0], p[1], 0.0]);
            commands.spawn((
                Beam(i),
                Mesh3d(mesh.clone()),
                MeshMaterial3d(other_look.clone()),
                Transform::from_translation(at),
                Name::new("race checkpoint"),
            ));
        }
    }
}

/// The race's banner and results on screen.
#[derive(Component)]
pub(crate) struct RaceBanner;

#[derive(Component)]
pub(crate) struct RaceResults;

pub(crate) fn spawn_text(mut commands: Commands) {
    commands.spawn((
        RaceBanner,
        Text::new(String::new()),
        TextFont {
            font_size: 40.0,
            ..default()
        },
        TextColor(Color::srgb(1.0, 0.82, 0.0)),
        TextShadow::default(),
        TextLayout::new_with_justify(Justify::Center),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Percent(26.0),
            width: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            ..default()
        },
        Pickable::IGNORE,
        GlobalZIndex(11),
    ));
    commands.spawn((
        RaceResults,
        Text::new(String::new()),
        TextFont {
            font_size: 22.0,
            ..default()
        },
        TextColor(Color::WHITE),
        TextShadow::default(),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Percent(34.0),
            right: Val::Percent(4.0),
            ..default()
        },
        Pickable::IGNORE,
        GlobalZIndex(11),
    ));
}

pub(crate) fn draw_text(
    race: Res<RaceText>,
    mut banner: Query<&mut Text, (With<RaceBanner>, Without<RaceResults>)>,
    mut results: Query<&mut Text, (With<RaceResults>, Without<RaceBanner>)>,
) {
    if let Ok(mut t) = banner.single_mut() {
        if t.0 != race.banner {
            t.0.clone_from(&race.banner);
        }
    }
    if let Ok(mut t) = results.single_mut() {
        let lines = race.lines.join("\n");
        if t.0 != lines {
            t.0 = lines;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clocks_and_ordinals_read_like_ctr() {
        assert_eq!(clock(83_456), "1:23.45");
        assert_eq!(clock(5_000), "0:05.00");
        assert_eq!(ordinal(1), "1ST");
        assert_eq!(ordinal(3), "3RD");
        assert_eq!(ordinal(7), "7TH");
    }

    #[test]
    fn standings_order_by_laps_then_checkpoints_then_first_there() {
        let a = standing(&(1, 2, 10.0), 6);
        let b = standing(&(1, 3, 11.0), 6);
        let c = standing(&(1, 3, 12.0), 6);
        assert!(b > a);
        assert!(b > c, "the earlier of two at the same checkpoint leads");
    }
}
