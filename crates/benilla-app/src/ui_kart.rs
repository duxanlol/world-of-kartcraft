//! The kart's overlay ([`crate::kart::KartOverlay`]) into the client's UI layer
//! (`assets/ui/KartcraftFrames.xml`), as events when it changes:
//!
//! - `BENILLA_KART_RACE(banner, results)`: the banner, and the results one per line.
//! - `BENILLA_KART_SCORE(trick, r, g, b, points, r, g, b, xp, skill, alpha)`, or no arguments
//!   when there is nothing to show.
//! - `BENILLA_KART_COURSES(list)`: what the Race Finder offers, one `key<TAB>name` a line, once
//!   per VM: the zones that have courses, as `z<place>` and "Westfall (8)", which the server takes
//!   for every course in that zone. One argument, so the list can grow past what an event's
//!   `arg1`..`argN` would carry.
//! - `BENILLA_KART_COURSE_NAMES(list)`: every course, one `key<TAB>name` a line, once per VM, for
//!   naming the course a ready check is for.
//! - `BENILLA_KART_TRACKS(list, flight, leaps)`: the track browser's courses, once per VM, a line
//!   each: `key<TAB>name<TAB>zone<TAB>map<TAB>laps<TAB>description<TAB>x,y;x,y;...` (the checkpoints,
//!   WoW x, y, which `GetWorldLocMapPosition` puts on the world map); the flight courses' keys, one
//!   a line; the flight distances as the courses, their launch the one point; and the gauntlets'
//!   keys, one a line.
//! - `BENILLA_KART_GARAGE(skill, classes, racers)`: the garage's Karting skill (0 before the line
//!   is ours), its classes one `key<TAB>name<TAB>skill needed<TAB>description` a line in the order
//!   they open, and the racers whose karts the kart data holds, one `key<TAB>name` a line; at load
//!   and whenever the skill moves.
//!
//! - `BENILLA_KART_TRACKMAP(name, laps, points)`: the race's track when it changes, its
//!   checkpoints `x,y;x,y;...` (WoW x, y); an empty name when there is no race. The race map
//!   itself is the kart's own ([`crate::kart::race_map_ui`]).
//! - `BENILLA_KART_TRACKEDIT(state, note, notes)`: the track editor, when it changes (its
//!   panels and popups are the kart's own, [`crate::kart::track_edit_ui`]): `edit` (the
//!   interface hidden), `test` (a test lap of the draft), `run` (of a saved track) or empty when
//!   closed; the last word for the player and how many so far, a new one for the chat.
//! - `BENILLA_KART_MYTRACKS(list)`: the track editor's saved tracks, as `BENILLA_KART_TRACKS`
//!   lists courses with six fields more, the grid (`x,y,o` or empty), the gates with their
//!   facings (`x,y[,o];...`), the floor, the placed objects (`key,x,y,z,yaw,pitch,roll;...`),
//!   the scale (empty for 1) and `1` for a flying track, at load and after each save or delete.
//! - `BENILLA_KART_SEATED(seated)`: 1 when we climb into the kart, 0 when we climb out.
//! - `BENILLA_KART_LEAVE(percent)`: leaving the kart in a race, held for, how far; no arguments
//!   when let go or done.
//! - `BENILLA_KART_GPS(town, yards)`: a delivery's town and the yards left to it by road, in
//!   steps of ten; no arguments off its map, out of the kart or delivered.
//! - `BENILLA_KART_PAD(name, held, lx, ly, rx, ry, l2, r2)`: the controller test's pad while it
//!   is open (`KartPadTest`), when it changes: its name (empty with none), the buttons held as
//!   space-separated tokens, the sticks -1 to 1 and the triggers 0 to 1.
//!
//! The Race Finder itself talks to the server over the addon channel, in Lua.

use benilla_ui::script::{ScriptValue, UiScript};
use bevy::prelude::*;

use crate::kart::{class_list, racer_list, KartOverlay, RaceMap, ScoreLines, COURSES, ZONES};
use crate::ui_script::VmMemo;

/// What this VM was last sent, so an unchanged frame sends nothing.
#[derive(Default)]
pub(crate) struct Sent {
    race: Option<(String, Vec<String>)>,
    score: Option<Option<ScoreLines>>,
    courses: bool,
    seated: Option<bool>,
    leaving: Option<Option<u8>>,
    gps: Option<Option<(String, u32)>>,
    /// The skill the garage was last sent at.
    garage: Option<Option<u16>>,
    /// The race map's track and racers as last sent, and when the racers were.
    track: Option<Option<RaceMap>>,
    pad: Option<Option<crate::kart::PadView>>,
    /// The held pad modifier as last sent ([`KartOverlay::pad_modifier`]).
    pad_modifier: Option<&'static str>,
    track_edit: Option<crate::kart::track_edit::EditView>,
    my_tracks: Option<u32>,
}

pub(crate) fn feed(
    time: Res<Time>,
    overlay: Res<KartOverlay>,
    (edit, mut keys, mut menu): (
        Res<crate::kart::track_edit::TrackEdit>,
        ResMut<crate::kart::track_edit_ui::EditorKeysText>,
        ResMut<crate::kart::menu_ui::KartMenu>,
    ),
    script: Option<NonSendMut<UiScript>>,
    mut sent: Local<VmMemo<Sent>>,
) {
    let Some(mut script) = script else {
        return;
    };
    let sent = sent.get(&script);
    // The Kartcraft menu (`kart::menu_ui`): told shown or closed, its rows read as it opens and
    // each second while up (the queue, the race), and the entry chosen run.
    let now = time.elapsed_secs();
    if menu.shown != menu.open {
        menu.shown = menu.open;
        menu.read_at = None;
        let edge = if menu.open {
            "BenillaKartMenu_OnShow()"
        } else {
            "PlaySound(\"igMainMenuClose\")"
        };
        let _ = script.run(edge);
    }
    // Waiting on the server's dungeons, each frame until they come.
    let due = menu.read_at.is_none_or(|t| now - t >= 1.0) || menu.waiting();
    if menu.open && due {
        let rows: String = script
            .eval("return BenillaKartMenu_Rows ~= nil and BenillaKartMenu_Rows() or ''")
            .unwrap_or_default();
        menu.set_rows(&rows);
        menu.read_at = Some(now);
    }
    if let Some(id) = menu.take_run() {
        let _ = script.run(&format!("BenillaKartMenu_Run(\"{id}\")"));
    }
    let view = edit.view();
    if sent.track_edit.as_ref() != Some(&view) {
        script.fire_event(
            "BENILLA_KART_TRACKEDIT",
            vec![
                ScriptValue::Str(view.state.into()),
                ScriptValue::Str(edit.note().into()),
                ScriptValue::Number(f64::from(view.notes)),
            ],
        );
        // The keys card's, as bound when the editor opens.
        let opened = sent.track_edit.as_ref().is_none_or(|v| v.state != "edit");
        if view.state == "edit" && opened {
            let text = crate::kart::track_edit_ui::keys_text(&script.keybind_snapshot());
            keys.0 = text;
        }
        sent.track_edit = Some(view);
    }
    let (mine, saves) = edit.saved();
    if sent.my_tracks != Some(saves) {
        // A course's line, then its grid, its gates with their facings, its floor, its objects,
        // its scale (empty for its own size) and `1` for a flying track, for a publish.
        let list: Vec<String> = mine
            .map(|(c, grid, gates, z, objects)| {
                let scale = match c.scale() {
                    s if s != 1.0 => format!("{s:.2}"),
                    _ => String::new(),
                };
                let flight = if c.flight() { "1" } else { "" };
                format!(
                    "{}\t{grid}\t{gates}\t{z}\t{objects}\t{scale}\t{flight}",
                    track_line(c)
                )
            })
            .collect();
        script.fire_event(
            "BENILLA_KART_MYTRACKS",
            vec![ScriptValue::Str(list.join("\n"))],
        );
        sent.my_tracks = Some(saves);
    }
    if !sent.courses {
        script.fire_event(
            "BENILLA_KART_COURSE_NAMES",
            vec![ScriptValue::Str(course_names())],
        );
        script.fire_event("BENILLA_KART_COURSES", vec![ScriptValue::Str(zone_list())]);
        script.fire_event(
            "BENILLA_KART_TRACKS",
            vec![
                ScriptValue::Str(track_list()),
                ScriptValue::Str(flight_list()),
                ScriptValue::Str(leap_list()),
                ScriptValue::Str(gauntlet_list()),
            ],
        );
        sent.courses = true;
    }
    // The race's track when it changes.
    if sent.track.as_ref() != Some(&overlay.track) {
        script.fire_event("BENILLA_KART_TRACKMAP", track_args(overlay.track.as_ref()));
        sent.track = Some(overlay.track.clone());
    }
    let race = (overlay.banner.clone(), overlay.results.clone());
    if sent.race.as_ref() != Some(&race) {
        script.fire_event(
            "BENILLA_KART_RACE",
            vec![
                ScriptValue::Str(race.0.clone()),
                ScriptValue::Str(race.1.join("\n")),
            ],
        );
        sent.race = Some(race);
    }

    if sent.score.as_ref() != Some(&overlay.score) {
        script.fire_event(
            "BENILLA_KART_SCORE",
            overlay.score.clone().map(score_args).unwrap_or_default(),
        );
        sent.score = Some(overlay.score.clone());
    }
    if sent.seated != Some(overlay.seated) {
        script.fire_event(
            "BENILLA_KART_SEATED",
            vec![ScriptValue::Number(f64::from(u8::from(overlay.seated)))],
        );
        sent.seated = Some(overlay.seated);
    }
    if sent.leaving != Some(overlay.leaving) {
        script.fire_event(
            "BENILLA_KART_LEAVE",
            overlay
                .leaving
                .map(|p| vec![ScriptValue::Number(f64::from(p))])
                .unwrap_or_default(),
        );
        sent.leaving = Some(overlay.leaving);
    }
    if sent.gps.as_ref() != Some(&overlay.gps) {
        script.fire_event(
            "BENILLA_KART_GPS",
            overlay
                .gps
                .as_ref()
                .map(|(town, yards)| {
                    vec![
                        ScriptValue::Str(town.clone()),
                        ScriptValue::Number(f64::from(*yards)),
                    ]
                })
                .unwrap_or_default(),
        );
        sent.gps = Some(overlay.gps.clone());
    }
    if sent.pad.as_ref() != Some(&overlay.pad) {
        if let Some(pad) = &overlay.pad {
            let mut args = vec![
                ScriptValue::Str(pad.name.clone()),
                ScriptValue::Str(pad.held.clone()),
            ];
            args.extend(
                pad.axes
                    .iter()
                    .map(|v| ScriptValue::Number(f64::from(*v) / 100.0)),
            );
            script.fire_event("BENILLA_KART_PAD", args);
        }
        sent.pad = Some(overlay.pad.clone());
    }
    // R2 or L2 held a moment: the card of its bindings (`BenillaPadMods`), and gone with it.
    if sent.pad_modifier != Some(overlay.pad_modifier) {
        script.fire_event(
            "BENILLA_PAD_MODIFIER",
            vec![ScriptValue::Str(overlay.pad_modifier.to_string())],
        );
        sent.pad_modifier = Some(overlay.pad_modifier);
    }
    if sent.garage != Some(overlay.skill) {
        script.fire_event("BENILLA_KART_GARAGE", garage_args(overlay.skill));
        sent.garage = Some(overlay.skill);
    }
}

/// The garage's event: the skill, the classes and the racers.
fn garage_args(skill: Option<u16>) -> Vec<ScriptValue> {
    static RACERS: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let racers = RACERS.get_or_init(|| racer_list(&kartcraft::model::racers()));
    vec![
        ScriptValue::Number(f64::from(skill.unwrap_or(0))),
        ScriptValue::Str(class_list()),
        ScriptValue::Str(racers.clone()),
    ]
}

/// The finder's list: each zone (or series) with courses, by its place (`z3`, as the server
/// reads it) and its name with how many courses it holds.
fn zone_list() -> String {
    ZONES
        .iter()
        .enumerate()
        .map(|(i, zone)| {
            let n = COURSES.iter().filter(|c| c.finder_group() == zone).count();
            format!("z{i}\t{zone} ({n})")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every course, one `key<TAB>name` a line.
fn course_names() -> String {
    COURSES
        .iter()
        .map(|c| format!("{}\t{}", c.key, c.name))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every course for the track browser, a line each (see the module's list).
fn track_list() -> String {
    COURSES
        .iter()
        .map(track_line)
        .collect::<Vec<_>>()
        .join("\n")
}

/// The flight courses' keys, one a line: the track browser's Flight tab.
fn flight_list() -> String {
    COURSES
        .iter()
        .filter(|c| c.flight())
        .map(|c| c.key.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The gauntlets' keys, one a line: the track browser's Gauntlet tab.
fn gauntlet_list() -> String {
    COURSES
        .iter()
        .filter(|c| c.gauntlet())
        .map(|c| c.key.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The flight distances as the track browser lists courses, a line each, the launch its route.
fn leap_list() -> String {
    let clean = |s: &str| s.replace(['\t', '\n'], " ");
    crate::kart::LEAPS
        .iter()
        .map(|l| {
            format!(
                "{}\t{}\t{}\t{}\t0\t{}\t{:.1},{:.1}",
                l.key,
                clean(&l.name),
                clean(&l.zone),
                l.map,
                clean(&l.description()),
                l.launch.x,
                l.launch.y
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A course's line in the track browser's list.
fn track_line(c: &crate::kart::Course) -> String {
    let points: Vec<String> = c
        .route()
        .iter()
        .map(|p| format!("{:.1},{:.1}", p[0], p[1]))
        .collect();
    let clean = |s: &str| s.replace(['\t', '\n'], " ");
    format!(
        "{}\t{}\t{}\t{}\t{}\t{}\t{}",
        c.key,
        clean(&c.name),
        clean(&c.zone),
        c.map,
        c.laps,
        clean(&c.description()),
        points.join(";")
    )
}

/// The race map's track as its event carries it; no race is an empty name.
fn track_args(track: Option<&RaceMap>) -> Vec<ScriptValue> {
    let Some(t) = track else {
        return vec![ScriptValue::Str(String::new())];
    };
    let points: Vec<String> = t
        .points
        .iter()
        .map(|p| format!("{:.1},{:.1}", p[0], p[1]))
        .collect();
    vec![
        ScriptValue::Str(t.name.clone()),
        ScriptValue::Number(f64::from(t.laps)),
        ScriptValue::Str(points.join(";")),
    ]
}

fn score_args(lines: ScoreLines) -> Vec<ScriptValue> {
    let rgb = |c: [f32; 3]| c.map(|v| ScriptValue::Number(f64::from(v)));
    let mut args = vec![ScriptValue::Str(lines.trick.0)];
    args.extend(rgb(lines.trick.1));
    args.push(ScriptValue::Str(lines.points.0));
    args.extend(rgb(lines.points.1));
    args.push(ScriptValue::Str(lines.xp));
    args.push(ScriptValue::Str(lines.skill));
    args.push(ScriptValue::Number(f64::from(lines.alpha)));
    args
}

/// The shop's goods pane's yaw, which its rotate buttons turn, for its kart
/// (`crate::kart::goods_preview`).
pub(crate) fn good_yaw(
    script: Option<NonSend<UiScript>>,
    mut preview: ResMut<crate::kart::goods_preview::GoodPreview>,
) {
    if let Some(script) = script {
        preview.yaw = script.model_pane_facing("BenillaKartShopFrameGood");
    }
}

/// The skin shop pane's yaw, which its rotate buttons turn, for its booth (`crate::kart::skins`).
pub(crate) fn skin_yaw(
    script: Option<NonSend<UiScript>>,
    mut booth: ResMut<crate::portrait::SkinBooth>,
) {
    if let Some(script) = script {
        booth.yaw = script.model_pane_facing("BenillaKartShopFrameModel");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_finder_offers_every_zone_by_place_and_names_every_course() {
        let zones = zone_list();
        let lines: Vec<&str> = zones.lines().collect();
        assert_eq!(lines.len(), ZONES.len());
        for (i, line) in lines.iter().enumerate() {
            let (key, name) = line.split_once('\t').unwrap();
            assert_eq!(key, format!("z{i}"));
            assert!(name.starts_with(&ZONES[i]) && name.ends_with(')'), "{line}");
        }
        assert_eq!(course_names().lines().count(), COURSES.len());
        assert!(course_names().contains("gp-azora\tAzora Hill Climb"));
        let tracks = track_list();
        assert_eq!(tracks.lines().count(), COURSES.len());
        for line in tracks.lines() {
            let fields: Vec<&str> = line.split('\t').collect();
            assert_eq!(fields.len(), 7, "{line}");
            assert!(fields[3].parse::<u32>().is_ok(), "the map id: {line}");
            assert!(fields[4].parse::<u8>().is_ok(), "the laps: {line}");
            assert!(fields[6].split(';').count() >= 2, "a route: {line}");
        }
        let list = flight_list();
        let flights: Vec<&str> = list.lines().collect();
        assert!(
            flights.contains(&"tb-skyway")
                && flights.len() == COURSES.iter().filter(|c| c.flight()).count()
        );
        let leaps = leap_list();
        assert_eq!(leaps.lines().count(), crate::kart::LEAPS.len());
        for line in leaps.lines() {
            let fields: Vec<&str> = line.split('\t').collect();
            assert_eq!(fields.len(), 7, "{line}");
            assert_eq!(fields[6].split(';').count(), 1, "the launch: {line}");
        }
        assert!(leaps.starts_with("tb-leap\tThunder Bluff Leap\tMulgore\t1\t0\t"));
    }
}
