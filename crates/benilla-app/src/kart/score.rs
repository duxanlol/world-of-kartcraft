//! The kart's score on screen: the turbo chain in progress (its latest boost, running points) at
//! the top of the screen, then the banked chain's points with what it earned, the experience
//! (`SMSG_LOG_XPGAIN` without a victim) and the Karting skill gained, or CRASH! when a wall or a
//! spin-out loses it. CTR's own HUD (speedometer, turbo meter) is `kartcraft::hud`.

use benilla_formats::kartcraft::SKILL_KARTING;
use benilla_protocol::SessionEvent;
use bevy::prelude::*;

use crate::net::{ObjectStore, SelfPlayer};
use kartcraft::chain::Line;

/// How long a banked chain stays up, then how long it fades (seconds).
const BANKED_HOLD: f32 = 2.5;
const BANKED_FADE: f32 = 1.0;
/// How long CRASH! stays up (seconds).
const CRASH_HOLD: f32 = 1.5;
/// Experience arriving this long after a bank is the bank's (seconds).
const XP_WINDOW: f32 = 4.0;

const TRICK_COLOR: Color = Color::srgb(1.0, 1.0, 1.0);
const POINTS_COLOR: Color = Color::srgb(1.0, 0.82, 0.0);
const BANKED_COLOR: Color = Color::srgb(0.35, 1.0, 0.35);
const CRASH_COLOR: Color = Color::srgb(1.0, 0.25, 0.2);

/// The score feed, written by the kart update and the experience handler.
#[derive(Resource, Default)]
pub(super) struct KartScore {
    /// The chain in progress.
    live: Option<Line>,
    banked: Option<Banked>,
    /// When the last chain was lost, and its points.
    crashed: Option<(f32, f32)>,
    /// The Karting skill last seen, for the gain under a bank.
    skill: Option<u16>,
}

struct Banked {
    at: f32,
    points: f32,
    label: String,
    xp: u32,
    skill_gain: u16,
}

impl KartScore {
    pub(super) fn live(&mut self, line: Option<Line>) {
        self.live = line;
    }

    pub(super) fn bank(&mut self, now: f32, points: f32, label: &str) {
        self.banked = Some(Banked {
            at: now,
            points,
            label: label.to_string(),
            xp: 0,
            skill_gain: 0,
        });
        self.crashed = None;
        self.live = None;
    }

    pub(super) fn lose(&mut self, now: f32, points: f32) {
        self.crashed = Some((now, points));
        self.live = None;
    }

    /// Out of the kart: nothing in progress.
    pub(super) fn clear_live(&mut self) {
        self.live = None;
    }
}

/// `SMSG_LOG_XPGAIN` without a victim shortly after a bank is the chain's experience.
pub(super) fn on_xp_gain(In(ev): In<SessionEvent>, time: Res<Time>, mut score: ResMut<KartScore>) {
    let SessionEvent::XpGain(x) = ev else {
        return;
    };
    let now = time.elapsed_secs();
    if let Some(banked) = score.banked.as_mut() {
        if !x.kill && now - banked.at < XP_WINDOW {
            banked.xp += x.total;
        }
    }
}

/// What the score shows this frame: per line its text and colour, and the shared alpha. The
/// client's UI layer draws it (`super::ui`).
pub(super) struct Shown {
    pub(super) trick: (String, Color),
    pub(super) points: (String, Color),
    pub(super) xp: String,
    pub(super) skill: String,
    pub(super) alpha: f32,
}

/// Our Karting skill, from our skill fields; `None` before we have the line.
pub(super) fn karting_skill(store: &ObjectStore) -> Option<u16> {
    (0..benilla_protocol::messages::PLAYER_SKILL_SLOTS)
        .filter_map(|slot| store.0.player_skill(slot))
        .find(|s| u32::from(s.skill_id) == SKILL_KARTING)
        .map(|s| s.value)
}

/// The Karting skill's rise lands on the chain it came from.
pub(super) fn track_skill(
    self_q: Query<&ObjectStore, With<SelfPlayer>>,
    mut score: ResMut<KartScore>,
) {
    let skill = self_q.single().ok().and_then(karting_skill);
    if let (Some(new), Some(old)) = (skill, score.skill) {
        if new > old {
            if let Some(banked) = score.banked.as_mut() {
                banked.skill_gain += new - old;
            }
        }
    }
    score.skill = skill.or(score.skill);
}

impl KartScore {
    /// The Karting skill last seen.
    pub(super) fn skill(&self) -> Option<u16> {
        self.skill
    }

    /// What to show at `now`, or nothing.
    pub(super) fn shown(&self, now: f32) -> Option<Shown> {
        shown(self, now, self.skill)
    }
}

fn shown(score: &KartScore, now: f32, skill: Option<u16>) -> Option<Shown> {
    if let Some(live) = &score.live {
        return Some(Shown {
            trick: (live.label.clone(), TRICK_COLOR),
            points: (grouped(live.points), POINTS_COLOR),
            xp: String::new(),
            skill: String::new(),
            alpha: 1.0,
        });
    }
    if let Some(banked) = &score.banked {
        let age = now - banked.at;
        if age < BANKED_HOLD + BANKED_FADE {
            let xp = match banked.xp {
                0 => String::new(),
                xp => format!("+{xp} XP"),
            };
            let skill = match banked.skill_gain {
                0 => String::new(),
                gain => {
                    let at = skill.map(|v| format!(" ({v})")).unwrap_or_default();
                    format!("+{gain} Karting{at}")
                }
            };
            return Some(Shown {
                trick: (banked.label.clone(), TRICK_COLOR),
                points: (format!("+{}", grouped(banked.points)), BANKED_COLOR),
                xp,
                skill,
                alpha: fade(age, BANKED_HOLD, BANKED_FADE),
            });
        }
    }
    if let Some((at, lost)) = score.crashed {
        let age = now - at;
        if age < CRASH_HOLD + BANKED_FADE {
            return Some(Shown {
                trick: ("CRASH!".into(), CRASH_COLOR),
                points: (grouped(lost), CRASH_COLOR.with_alpha(0.6)),
                xp: String::new(),
                skill: String::new(),
                alpha: fade(age, CRASH_HOLD, BANKED_FADE),
            });
        }
    }
    None
}

/// Full alpha for `hold` seconds, then a linear fade over `fade`.
fn fade(age: f32, hold: f32, fade: f32) -> f32 {
    (1.0 - (age - hold).max(0.0) / fade).clamp(0.0, 1.0)
}

/// `points` rounded, with thousands separators: 12,345.
fn grouped(points: f32) -> String {
    let digits = (points.max(0.0).round() as u64).to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(points: f32, label: &str) -> Option<Line> {
        Some(Line {
            points,
            label: label.into(),
        })
    }

    #[test]
    fn points_group_by_thousands() {
        assert_eq!(grouped(0.4), "0");
        assert_eq!(grouped(999.0), "999");
        assert_eq!(grouped(12345.0), "12,345");
        assert_eq!(grouped(1234567.0), "1,234,567");
    }

    #[test]
    fn a_bank_shows_then_fades_and_a_lost_chain_shows_crash() {
        let mut score = KartScore::default();
        score.live(line(250.0, "Double Turbo x2"));
        let live = shown(&score, 0.0, None).expect("live chain");
        assert_eq!(live.points.0, "250");
        score.bank(1.0, 300.0, "Triple Turbo x3");
        let banked = shown(&score, 1.5, None).expect("banked");
        assert_eq!(banked.points.0, "+300");
        assert_eq!(banked.trick.0, "Triple Turbo x3");
        assert!(shown(&score, 1.0 + BANKED_HOLD + BANKED_FADE + 0.1, None).is_none());
        score.lose(10.0, 120.0);
        assert_eq!(shown(&score, 10.1, None).expect("crash").trick.0, "CRASH!");
    }
}
