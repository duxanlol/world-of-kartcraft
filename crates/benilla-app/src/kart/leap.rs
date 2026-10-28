//! A flight distance (`Kartcraft.cpp` "Flight distance"): the Race Marshal puts us on a high
//! launch ("R\tLeap\t<key>" back from the server arms the run), we fly off it as far as we can,
//! and the first touchdown after the wings opened is told ("R\tLeapLand\t<key>"); the server
//! measures the distance from where it sees us and answers with the best ("R\tLeapBest", the
//! interface's to say). The kart flies through a run whatever `KartFlight` says
//! ([`super::settings::KartSettings::modes`]).

use benilla_protocol::SessionEvent;
use bevy::prelude::*;
use kartcraft::Event;

/// A run is given up this long (s) after it was armed: as the server's own longest leap.
const GIVE_UP: f32 = 180.0;

/// The run under way: its leap's key, when it was armed and whether the wings have opened.
#[derive(Resource, Default)]
pub(crate) struct LeapRun {
    run: Option<(String, f32, bool)>,
}

impl LeapRun {
    pub(crate) fn running(&self) -> bool {
        self.run.is_some()
    }

    /// Arms a run on `key`'s launch at `now`.
    fn arm(&mut self, key: &str, now: f32) {
        self.run = Some((key.to_string(), now, false));
    }

    /// A kart event at `now`: the wings opening, then the touchdown's line to send; a run past
    /// [`GIVE_UP`] ends unsent.
    pub(super) fn observe(&mut self, event: &Event, now: f32) -> Option<String> {
        let (key, since, flew) = self.run.as_mut()?;
        if now - *since > GIVE_UP {
            self.run = None;
            return None;
        }
        match event {
            Event::TakeOff => *flew = true,
            Event::Touchdown if *flew => {
                let line = format!("KARTCRAFT\tR\tLeapLand\t{key}");
                self.run = None;
                return Some(line);
            }
            _ => {}
        }
        None
    }
}

/// The server's word that we are on a launch.
pub(crate) fn on_chat(In(ev): In<SessionEvent>, time: Res<Time>, mut leap: ResMut<LeapRun>) {
    let SessionEvent::Chat(m) = ev else {
        return;
    };
    if !m.is_addon() {
        return;
    }
    if let Some(key) = m.text.strip_prefix("KARTCRAFT\tR\tLeap\t") {
        info!("kart leap: on the launch of {key}");
        leap.arm(key.trim(), time.elapsed_secs());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A run tells its first touchdown after the wings opened, once; a hop's landing before them
    /// is not one, and a run left too long tells nothing.
    #[test]
    fn a_leap_tells_its_touchdown_after_a_flight() {
        let mut leap = LeapRun::default();
        assert_eq!(leap.observe(&Event::Touchdown, 0.0), None, "not armed");
        leap.arm("tb-leap", 0.0);
        assert!(leap.running());
        assert_eq!(leap.observe(&Event::Touchdown, 1.0), None, "no flight yet");
        assert_eq!(leap.observe(&Event::TakeOff, 2.0), None);
        assert_eq!(
            leap.observe(&Event::Touchdown, 9.0).as_deref(),
            Some("KARTCRAFT\tR\tLeapLand\ttb-leap")
        );
        assert!(!leap.running());
        assert_eq!(leap.observe(&Event::Touchdown, 10.0), None, "once");
        leap.arm("tb-leap", 0.0);
        leap.observe(&Event::TakeOff, 1.0);
        assert_eq!(leap.observe(&Event::Touchdown, GIVE_UP + 1.0), None);
        assert!(!leap.running());
    }
}
