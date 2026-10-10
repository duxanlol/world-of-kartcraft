//! Turbo chains, Kartcraft's answer to Skate 3's trick lines: every powerslide boost and
//! hang-time boost scores, the chain's multiplier climbs with each one, and the chain banks once
//! we stop boosting for a moment. A wall slam or a spin-out loses it. Banked chains are what the
//! profession pays out on.

use ctr_kart::Event;

/// Seconds without a new boost, outside a powerslide, before the chain banks.
const BANK_AFTER: f32 = 2.0;
/// Each boost in a chain is worth this much more than the last (1.0, 1.25, 1.5, ...).
const STEP: f32 = 0.25;

/// A chain in progress or banked: its points and a label for the screen.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub points: f32,
    pub label: String,
}

/// How a chain ended when nothing new came.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Banked(Line),
    /// Lost, with the points it had.
    Lost(f32),
}

#[derive(Default)]
pub struct Chain {
    points: f32,
    count: u32,
    label: String,
    /// Sim time of the last scoring event.
    last: f32,
    /// A slam or spin-out since the last call to [`Chain::idle`].
    lost: bool,
}

/// What a boost is worth before the chain's multiplier, and its name.
fn worth(event: &Event) -> Option<(f32, &'static str)> {
    match event {
        Event::Boost { stage: 1, .. } => Some((100.0, "Turbo")),
        Event::Boost { stage: 2, .. } => Some((200.0, "Double Turbo")),
        Event::Boost { .. } => Some((400.0, "Triple Turbo")),
        Event::HangTime { level: 1 } => Some((150.0, "Hang Time")),
        Event::HangTime { level: 2 } => Some((300.0, "Big Hang Time")),
        Event::HangTime { .. } => Some((600.0, "Huge Hang Time")),
        _ => None,
    }
}

impl Chain {
    /// One event at sim time `now`. Never banks by itself; [`Chain::idle`] does.
    pub fn observe(&mut self, now: f32, event: &Event) -> Option<Line> {
        if let Some((points, name)) = worth(event) {
            let multiplier = 1.0 + STEP * self.count as f32;
            self.points += points * multiplier;
            self.count += 1;
            self.label = if self.count > 1 {
                format!("{name} x{}", self.count)
            } else {
                name.to_string()
            };
            self.last = now;
        }
        if matches!(event, Event::Slam { .. } | Event::SpinOut) && self.points > 0.0 {
            self.lost = true;
        }
        None
    }

    /// Ends the chain if it was lost, or if it has gone quiet while we are not drifting.
    pub fn idle(&mut self, now: f32, drifting: bool) -> Option<Outcome> {
        if self.lost {
            let points = self.points;
            *self = Chain::default();
            return Some(Outcome::Lost(points));
        }
        if self.points > 0.0 && !drifting && now - self.last >= BANK_AFTER {
            let line = Line {
                points: self.points.round(),
                label: std::mem::take(&mut self.label),
            };
            *self = Chain::default();
            return Some(Outcome::Banked(line));
        }
        None
    }

    /// The chain in progress, if any.
    pub fn live(&self) -> Option<Line> {
        (self.points > 0.0).then(|| Line {
            points: self.points.round(),
            label: self.label.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boost(stage: u8) -> Event {
        Event::Boost {
            stage,
            reserves: 1000,
        }
    }

    #[test]
    fn a_triple_turbo_chain_climbs_and_banks_when_quiet() {
        let mut c = Chain::default();
        c.observe(0.0, &boost(1));
        c.observe(1.0, &boost(2));
        c.observe(2.0, &boost(3));
        // 100 + 200×1.25 + 400×1.5
        assert_eq!(c.live().unwrap().points, 950.0);
        assert_eq!(c.live().unwrap().label, "Triple Turbo x3");
        assert_eq!(c.idle(3.0, false), None);
        assert_eq!(c.idle(10.0, true), None, "never mid-drift");
        let Some(Outcome::Banked(line)) = c.idle(10.0, false) else {
            panic!("did not bank");
        };
        assert_eq!(line.points, 950.0);
        assert!(c.live().is_none());
    }

    #[test]
    fn a_slam_loses_the_chain() {
        let mut c = Chain::default();
        c.observe(0.0, &boost(1));
        c.observe(0.5, &Event::Slam { speed: 9000 });
        assert_eq!(c.idle(0.5, false), Some(Outcome::Lost(100.0)));
        assert!(c.live().is_none());
        // A slam with nothing at stake loses nothing.
        c.observe(1.0, &Event::Slam { speed: 9000 });
        assert_eq!(c.idle(1.0, false), None);
    }

    #[test]
    fn other_events_do_not_score() {
        let mut c = Chain::default();
        c.observe(0.0, &Event::Hop);
        c.observe(0.0, &Event::BoostEarly);
        assert!(c.live().is_none());
    }
}
