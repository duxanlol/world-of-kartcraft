//! The kart's overlay ([`crate::kart::KartOverlay`]) into the client's UI layer
//! (`assets/ui/KartcraftFrames.xml`), as events when it changes:
//!
//! - `BENILLA_KART_RACE(banner, results)`: the banner, and the results one per line.
//! - `BENILLA_KART_SCORE(trick, r, g, b, points, r, g, b, xp, skill, alpha)`, or no arguments
//!   when there is nothing to show.
//! - `BENILLA_KART_FINDER_TOGGLE`: `I` was pressed.
//!
//! The Race Finder itself talks to the server over the addon channel, in Lua.

use benilla_ui::script::{ScriptValue, UiScript};
use bevy::prelude::*;

use crate::kart::{KartOverlay, ScoreLines};
use crate::ui_script::VmMemo;

/// What this VM was last sent, so an unchanged frame sends nothing.
#[derive(Default)]
pub(crate) struct Sent {
    race: Option<(String, Vec<String>)>,
    score: Option<Option<ScoreLines>>,
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
