//! The kart's overlay ([`crate::kart::KartOverlay`]) into the client's UI layer
//! (`assets/ui/KartcraftFrames.xml`), as events when it changes:
//!
//! - `BENILLA_KART_RACE(banner, results)`: the banner, and the results one per line.
//! - `BENILLA_KART_SCORE(trick, r, g, b, points, r, g, b, xp, skill, alpha)`, or no arguments
//!   when there is nothing to show.
//! - `BENILLA_KART_FINDER_TOGGLE`: `I` was pressed.
//! - `BENILLA_KART_COURSES(list)`: what the Race Finder offers, one `key<TAB>name` a line, once
//!   per VM: the zones that have courses, as `z<place>` and "Westfall (8)", which the server takes
//!   for every course in that zone. One argument, so the list can grow past what an event's
//!   `arg1`..`argN` would carry.
//! - `BENILLA_KART_COURSE_NAMES(list)`: every course, one `key<TAB>name` a line, once per VM, for
//!   naming the course a ready check is for.
//!
//! The Race Finder itself talks to the server over the addon channel, in Lua.

use benilla_ui::script::{ScriptValue, UiScript};
use bevy::prelude::*;

use crate::kart::{KartOverlay, ScoreLines, COURSES, ZONES};
use crate::ui_script::VmMemo;

/// What this VM was last sent, so an unchanged frame sends nothing.
#[derive(Default)]
pub(crate) struct Sent {
    race: Option<(String, Vec<String>)>,
    score: Option<Option<ScoreLines>>,
    courses: bool,
}

pub(crate) fn feed(
    mut overlay: ResMut<KartOverlay>,
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
        sent.courses = true;
    }
    if std::mem::take(&mut overlay.toggle_finder) {
        script.fire_event("BENILLA_KART_FINDER_TOGGLE", Vec::new());
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
    }
}
