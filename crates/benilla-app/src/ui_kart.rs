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
//! - `BENILLA_KART_TRACKS(list)`: the track browser's courses, once per VM, a line each:
//!   `key<TAB>name<TAB>zone<TAB>map<TAB>laps<TAB>description<TAB>x,y;x,y;...` (the checkpoints, WoW x, y,
//!   which `GetWorldLocMapPosition` puts on the world map).
//! - `BENILLA_KART_GARAGE(skill, classes, racers)`: the garage's Karting skill (0 before the line
//!   is ours), its classes one `key<TAB>name<TAB>skill needed<TAB>description` a line in the order
//!   they open, and the racers whose karts the kart data holds, one `key<TAB>name` a line; at load
//!   and whenever the skill moves.
//!
//! The Race Finder itself talks to the server over the addon channel, in Lua.

use benilla_ui::script::{ScriptValue, UiScript};
use bevy::prelude::*;

use crate::kart::{class_list, racer_list, KartOverlay, ScoreLines, COURSES, ZONES};
use crate::ui_script::VmMemo;

/// What this VM was last sent, so an unchanged frame sends nothing.
#[derive(Default)]
pub(crate) struct Sent {
    race: Option<(String, Vec<String>)>,
    score: Option<Option<ScoreLines>>,
    courses: bool,
    /// The skill the garage was last sent at.
    garage: Option<Option<u16>>,
}

pub(crate) fn feed(
    overlay: Res<KartOverlay>,
    script: Option<NonSendMut<UiScript>>,
    mut sent: Local<VmMemo<Sent>>,
) {
    let Some(mut script) = script else {
        return;
    };
    let sent = sent.get(&script);
    if !sent.courses {
        script.fire_event(
            "BENILLA_KART_COURSE_NAMES",
            vec![ScriptValue::Str(course_names())],
        );
        script.fire_event("BENILLA_KART_COURSES", vec![ScriptValue::Str(zone_list())]);
        script.fire_event("BENILLA_KART_TRACKS", vec![ScriptValue::Str(track_list())]);
        sent.courses = true;
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

/// The finder's list: each zone with courses, by its place (`z3`, as the server reads it) and
/// its name with how many courses it holds.
fn zone_list() -> String {
    ZONES
        .iter()
        .enumerate()
        .map(|(i, zone)| {
            let n = COURSES.iter().filter(|c| c.zone == *zone).count();
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
        .map(|c| {
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
        })
        .collect::<Vec<_>>()
        .join("\n")
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
        assert!(course_names().contains("sprint\tNorthshire Sprint"));
        let tracks = track_list();
        assert_eq!(tracks.lines().count(), COURSES.len());
        for line in tracks.lines() {
            let fields: Vec<&str> = line.split('\t').collect();
            assert_eq!(fields.len(), 7, "{line}");
            assert!(fields[3].parse::<u32>().is_ok(), "the map id: {line}");
            assert!(fields[4].parse::<u8>().is_ok(), "the laps: {line}");
            assert!(fields[6].split(';').count() >= 2, "a route: {line}");
        }
    }
}
