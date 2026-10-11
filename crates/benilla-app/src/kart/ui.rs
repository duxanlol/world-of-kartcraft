//! What the kart puts on screen through the client's UI layer, as a model value: the race's
//! banner and results and the chain score. `crate::ui_kart` hands it to the
//! layer's frames (`assets/ui/KartcraftFrames.xml`), which draw it in WoW's own fonts.

use bevy::prelude::*;

use super::race::RaceText;
use super::score::KartScore;

/// The chain score's lines: text and colour for the trick and the points, the experience and
/// skill earned, and the shared alpha (in steps of 1/50, so a fade changes it at most fifty times).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ScoreLines {
    pub(crate) trick: (String, [f32; 3]),
    pub(crate) points: (String, [f32; 3]),
    pub(crate) xp: String,
    pub(crate) skill: String,
    pub(crate) alpha: f32,
}

#[derive(Resource, Default)]
pub(crate) struct KartOverlay {
    pub(crate) banner: String,
    pub(crate) results: Vec<String>,
    pub(crate) score: Option<ScoreLines>,
}

pub(super) fn publish(
    time: Res<Time>,
    text: Res<RaceText>,
    score: Res<KartScore>,
    mut overlay: ResMut<KartOverlay>,
) {
    if overlay.banner != text.banner {
        overlay.banner.clone_from(&text.banner);
    }
    if overlay.results != text.lines {
        overlay.results.clone_from(&text.lines);
    }
    let rgb = |c: Color| {
        let c = c.to_srgba();
        [c.red, c.green, c.blue]
    };
    let lines = score.shown(time.elapsed_secs()).map(|s| ScoreLines {
        trick: (s.trick.0, rgb(s.trick.1)),
        points: (s.points.0, rgb(s.points.1)),
        xp: s.xp,
        skill: s.skill,
        alpha: (s.alpha * 50.0).round() / 50.0,
    });
    if overlay.score != lines {
        overlay.score = lines;
    }
}
