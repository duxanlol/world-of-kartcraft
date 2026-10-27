//! What the kart puts on screen through the client's UI layer, as a model value: the race's
//! banner and results, the chain score, the Karting skill (for the garage's tiers), and the race
//! map's track and racers.
//! `crate::ui_kart` hands it to the
//! layer's frames (`assets/ui/KartcraftFrames.xml`), which draw it in WoW's own fonts.

use bevy::prelude::*;

use benilla_assets::coords::bevy_to_wow;

use super::race::{RaceMap, RaceText, Races};
use super::remote::RemoteKarts;
use super::score::KartScore;
use super::KartDrive;

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

/// A racer on the race map: WoW x, y; their place (0 before the standings have them); whether
/// it is us; and whether the position is their kart's own (relayed, near) or only the last
/// checkpoint the server has them past (too far off for their kart to be relayed).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RacerDot {
    pub(crate) name: String,
    pub(crate) at: [f32; 2],
    pub(crate) place: u8,
    pub(crate) me: bool,
    pub(crate) near: bool,
}

#[derive(Resource, Default)]
pub(crate) struct KartOverlay {
    pub(crate) banner: String,
    pub(crate) results: Vec<String>,
    pub(crate) score: Option<ScoreLines>,
    /// Our Karting skill; `None` before the line is ours.
    pub(crate) skill: Option<u16>,
    /// The race map: the race's track, and its racers, us among them.
    pub(crate) track: Option<RaceMap>,
    pub(crate) racers: Vec<RacerDot>,
    /// Whether we sit in the kart: the layer hides the action bars and unit frames meanwhile.
    pub(crate) seated: bool,
    /// The controller test's view of the pad the kart reads (`/kartpad`), while it is open.
    pub(crate) pad: Option<PadView>,
    /// A pad modifier held a moment, for the card of its bindings: `SHIFT` (R2), `CTRL` (L2),
    /// else empty ([`pad_modifier`]).
    pub(crate) pad_modifier: &'static str,
    /// Leaving the kart in a race, held for: how far, in percent.
    pub(crate) leaving: Option<u8>,
    /// A delivery's town and the yards left to it by road (`super::gps`).
    pub(crate) gps: Option<(String, u32)>,
}

/// The pad the kart reads, as the controller test shows it: its name (empty with none), the
/// buttons held as the layer's tokens, and the sticks and triggers in hundredths (left x, y,
/// right x, y, L2, R2).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PadView {
    pub(crate) name: String,
    pub(crate) held: String,
    pub(crate) axes: [i16; 6],
}

/// Bevy's buttons and the tokens the controller test lists them by.
const PAD_TOKENS: [(bevy::input::gamepad::GamepadButton, &str); 18] = {
    use bevy::input::gamepad::GamepadButton as B;
    [
        (B::South, "South"),
        (B::East, "East"),
        (B::West, "West"),
        (B::North, "North"),
        (B::LeftTrigger, "LeftTrigger"),
        (B::RightTrigger, "RightTrigger"),
        (B::LeftTrigger2, "LeftTrigger2"),
        (B::RightTrigger2, "RightTrigger2"),
        (B::Select, "Select"),
        (B::Start, "Start"),
        (B::Mode, "Mode"),
        (B::LeftThumb, "LeftThumb"),
        (B::RightThumb, "RightThumb"),
        (B::DPadUp, "DPadUp"),
        (B::DPadRight, "DPadRight"),
        (B::DPadDown, "DPadDown"),
        (B::DPadLeft, "DPadLeft"),
        (B::Other(0), "Touchpad"),
    ]
};

/// How long R2 or L2 is held before the card of its bindings shows (s): a chord's quick press
/// never shows it.
const MODIFIER_CARD_SECS: f32 = 0.35;

/// R2 or L2 held as the pad's modifier ([`kartcraft::input::modifier_down`]) for
/// [`MODIFIER_CARD_SECS`]: the interface's card of every binding it modifies shows while it is.
pub(super) fn pad_modifier(
    time: Res<Time>,
    pads: Query<&bevy::input::gamepad::Gamepad, Without<kartcraft::input::PadIgnored>>,
    mut overlay: ResMut<KartOverlay>,
    mut since: Local<Option<(&'static str, f32)>>,
) {
    use kartcraft::input::{modifier_down, PAD_CTRL, PAD_MODIFIER};
    let held = pads.iter().find_map(|p| {
        if modifier_down(p, PAD_MODIFIER) {
            Some("SHIFT")
        } else if modifier_down(p, PAD_CTRL) {
            Some("CTRL")
        } else {
            None
        }
    });
    let now = time.elapsed_secs();
    *since = match (held, *since) {
        (Some(m), Some((was, at))) if m == was => Some((was, at)),
        (Some(m), _) => Some((m, now)),
        (None, _) => None,
    };
    let shown = since
        .filter(|(_, at)| now - at >= MODIFIER_CARD_SECS)
        .map_or("", |(m, _)| m);
    if overlay.pad_modifier != shown {
        overlay.pad_modifier = shown;
    }
}

/// The controller test's view, while `KartPadTest` holds it open: the pad
/// [`kartcraft::input::read`] would drive with.
pub(super) fn pad_view(
    settings: Res<super::settings::KartSettings>,
    pads: Query<
        (&bevy::input::gamepad::Gamepad, Option<&Name>),
        Without<kartcraft::input::PadIgnored>,
    >,
    mut overlay: ResMut<KartOverlay>,
) {
    use bevy::input::gamepad::{GamepadAxis, GamepadButton};
    let view = settings.pad_test.then(|| {
        let Some((pad, name)) = pads.iter().next() else {
            return PadView::default();
        };
        let held: Vec<&str> = PAD_TOKENS
            .iter()
            .filter(|(b, _)| pad.pressed(*b))
            .map(|(_, t)| *t)
            .collect();
        let hundredths = |v: f32| (v.clamp(-1.0, 1.0) * 100.0).round() as i16;
        let axis = |a| hundredths(pad.get(a).unwrap_or(0.0));
        let trigger = |b| hundredths(pad.get(b).unwrap_or(0.0));
        PadView {
            name: name.map_or_else(|| "Controller".to_string(), |n| n.to_string()),
            held: held.join(" "),
            axes: [
                axis(GamepadAxis::LeftStickX),
                axis(GamepadAxis::LeftStickY),
                axis(GamepadAxis::RightStickX),
                axis(GamepadAxis::RightStickY),
                trigger(GamepadButton::LeftTrigger2),
                trigger(GamepadButton::RightTrigger2),
            ],
        }
    });
    if overlay.pad != view {
        overlay.pad = view;
    }
}

/// Each racer's dot: us where our kart is, another at their relayed kart when it is `near`, else
/// at the last checkpoint the server has them past.
pub(crate) fn racer_dots(
    map: &RaceMap,
    me: &str,
    me_at: Option<[f32; 2]>,
    near: &[(String, [f32; 2])],
) -> Vec<RacerDot> {
    map.checkpoints
        .iter()
        .map(|(name, checkpoint)| {
            let place = map
                .order
                .iter()
                .position(|o| o == name)
                .map_or(0, |i| (i + 1).min(255) as u8);
            let mine = name == me;
            let seen = if mine {
                me_at
            } else {
                near.iter().find(|(n, _)| n == name).map(|(_, at)| *at)
            };
            RacerDot {
                name: name.clone(),
                at: seen.unwrap_or(*checkpoint),
                place,
                me: mine,
                near: seen.is_some(),
            }
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn publish(
    time: Res<Time>,
    text: Res<RaceText>,
    score: Res<KartScore>,
    races: Res<Races>,
    drive: Res<KartDrive>,
    remotes: Res<RemoteKarts>,
    names: Option<Res<crate::names::NameCache>>,
    gps: Res<super::gps::Gps>,
    mut overlay: ResMut<KartOverlay>,
    edit: Res<super::track_edit::TrackEdit>,
) {
    // The race map: the track once its course line is here (else the track editor's draft), and
    // every racer on it.
    let track = races
        .map()
        .or_else(|| edit.preview(&races.me).as_ref()?.map());
    let racers = track.as_ref().map_or_else(Vec::new, |map| {
        let to_wow = |p: Vec3| {
            let w = bevy_to_wow(p);
            [w[0], w[1]]
        };
        let near = super::race::relayed(&remotes, names.as_deref());
        let me_at = drive.active.then(|| to_wow(drive.pos));
        racer_dots(map, &races.me, me_at, &near)
    });
    if overlay.track != track {
        overlay.track = track;
    }
    if overlay.racers != racers {
        overlay.racers = racers;
    }
    if overlay.banner != text.banner {
        overlay.banner.clone_from(&text.banner);
    }
    if overlay.results != text.lines {
        overlay.results.clone_from(&text.lines);
    }
    if overlay.seated != drive.active {
        overlay.seated = drive.active;
    }
    if overlay.leaving != drive.leaving {
        overlay.leaving = drive.leaving;
    }
    // The yards left in steps of ten: the clock is not rewritten every frame the kart rolls.
    let gps_left = gps.left.as_ref().map(|(t, y)| (t.clone(), y / 10 * 10));
    if overlay.gps != gps_left {
        overlay.gps = gps_left;
    }
    if overlay.skill != score.skill() {
        overlay.skill = score.skill();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn racers_are_where_their_karts_are_or_at_their_last_checkpoint() {
        let map = RaceMap {
            name: "Elwynn Loop".into(),
            laps: 3,
            points: vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0]],
            order: vec!["Gnome".into(), "Probesix".into(), "Goblin".into()],
            checkpoints: vec![
                ("Probesix".into(), [100.0, 0.0]),
                ("Goblin".into(), [0.0, 0.0]),
                ("Gnome".into(), [100.0, 100.0]),
            ],
        };
        let near = [("Goblin".to_string(), [10.0, 5.0])];
        let dots = racer_dots(&map, "Probesix", Some([90.0, 2.0]), &near);
        let dot = |n: &str| dots.iter().find(|d| d.name == n).unwrap().clone();
        // We are where our kart is, second.
        assert_eq!(
            dot("Probesix"),
            RacerDot {
                name: "Probesix".into(),
                at: [90.0, 2.0],
                place: 2,
                me: true,
                near: true,
            }
        );
        // A racer near enough to be relayed is at their kart.
        assert!(dot("Goblin").near && dot("Goblin").at == [10.0, 5.0]);
        assert_eq!(dot("Goblin").place, 3);
        // One too far off is at the last checkpoint the server has them past.
        assert!(!dot("Gnome").near && dot("Gnome").at == [100.0, 100.0]);
        assert_eq!(dot("Gnome").place, 1);
        // Out of the kart, we are at our checkpoint too.
        let off = racer_dots(&map, "Probesix", None, &near);
        assert_eq!(off.iter().find(|d| d.me).unwrap().at, [100.0, 0.0]);
    }
}
