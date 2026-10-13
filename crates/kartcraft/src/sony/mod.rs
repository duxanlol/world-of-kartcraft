//! PlayStation pads read and rumbled directly over HID, on Windows. Windows.Gaming.Input reads a
//! DualSense or a DualShock 4 but cannot rumble it (it vibrates only Xbox-class pads), and a
//! Bluetooth one switched to its full report by a rumble report stops reading there. So the pads
//! are read here ([`report`], a thread in `hid`), each one a Bevy gamepad of its own marked
//! [`SonyPad`], which [`crate::input::read`] prefers over the Windows copy of the same pad; and
//! the kart's rumble ([`crate::rumble`]) reaches them through [`SonyPads::shake`].
//!
//! `WOW_SONY_HID=0` leaves them to Windows (no rumble). Elsewhere this is inert: Linux's
//! `hid-playstation` driver rumbles them through evdev.

pub mod report;

#[cfg(windows)]
mod hid;

use bevy::input::gamepad::{
    GamepadAxis, GamepadButton, GamepadConnection, GamepadConnectionEvent,
    RawGamepadAxisChangedEvent, RawGamepadButtonChangedEvent, RawGamepadEvent,
};
use bevy::prelude::*;

use report::{Button, PadState};

/// A gamepad read here, not through Windows.
#[derive(Component)]
pub struct SonyPad;

/// The pads read here and the rumble they are given.
#[derive(Resource, Default)]
pub struct SonyPads {
    #[cfg(windows)]
    link: Option<hid::Link>,
    /// Each pad's entity and the state last handed to Bevy (`None` before its first).
    pads: std::collections::HashMap<u64, (Entity, Option<PadState>)>,
    /// The shakes under way: when each ends (`Time::elapsed_secs_f64`) and its strengths.
    shakes: Vec<(f64, f32, f32)>,
    sent: (u8, u8),
}

impl SonyPads {
    /// Adds a shake, `seconds` long from `now`.
    pub fn shake(&mut self, now: f64, pulse: crate::rumble::Pulse) {
        self.shakes
            .push((now + f64::from(pulse.seconds), pulse.strong, pulse.weak));
    }
}

pub struct SonyPadsPlugin;

impl Plugin for SonyPadsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SonyPads>();
        #[cfg(windows)]
        if std::env::var("WOW_SONY_HID").map_or(true, |v| v != "0") {
            app.add_systems(Startup, |mut pads: ResMut<SonyPads>| {
                pads.link = hid::start();
            })
            .add_systems(PreUpdate, poll.before(bevy::input::InputSystems))
            .add_systems(PostUpdate, drive_motors);
        }
    }
}

/// Bevy's buttons for the pad's, Xbox-named as gilrs names a PlayStation pad's: ✕ South, ○ East,
/// □ West, △ North.
const BUTTONS: [(Button, GamepadButton); 16] = [
    (Button::Cross, GamepadButton::South),
    (Button::Circle, GamepadButton::East),
    (Button::Square, GamepadButton::West),
    (Button::Triangle, GamepadButton::North),
    (Button::L1, GamepadButton::LeftTrigger),
    (Button::R1, GamepadButton::RightTrigger),
    (Button::Share, GamepadButton::Select),
    (Button::Options, GamepadButton::Start),
    (Button::L3, GamepadButton::LeftThumb),
    (Button::R3, GamepadButton::RightThumb),
    (Button::Ps, GamepadButton::Mode),
    (Button::Touchpad, GamepadButton::Other(0)),
    (Button::Up, GamepadButton::DPadUp),
    (Button::Right, GamepadButton::DPadRight),
    (Button::Down, GamepadButton::DPadDown),
    (Button::Left, GamepadButton::DPadLeft),
];

const AXES: [GamepadAxis; 4] = [
    GamepadAxis::LeftStickX,
    GamepadAxis::LeftStickY,
    GamepadAxis::RightStickX,
    GamepadAxis::RightStickY,
];

/// The changes from `old` to `new` as Bevy's raw gamepad events (all of `new` when `old` is none):
/// the buttons 0 or 1, the triggers as analog buttons, the sticks as axes.
pub fn changes(gamepad: Entity, old: Option<&PadState>, new: &PadState) -> Vec<RawGamepadEvent> {
    let mut out = Vec::new();
    for (b, bevy) in BUTTONS {
        let on = new.pressed(b);
        if old.is_none_or(|o| o.pressed(b) != on) {
            out.push(
                RawGamepadButtonChangedEvent::new(gamepad, bevy, if on { 1.0 } else { 0.0 }).into(),
            );
        }
    }
    for (value, was, bevy) in [
        (new.l2, old.map(|o| o.l2), GamepadButton::LeftTrigger2),
        (new.r2, old.map(|o| o.r2), GamepadButton::RightTrigger2),
    ] {
        if was != Some(value) {
            out.push(RawGamepadButtonChangedEvent::new(gamepad, bevy, value).into());
        }
    }
    for (i, axis) in AXES.into_iter().enumerate() {
        if old.map(|o| o.sticks[i]) != Some(new.sticks[i]) {
            out.push(RawGamepadAxisChangedEvent::new(gamepad, axis, new.sticks[i]).into());
        }
    }
    out
}

/// The pads' news into Bevy, as `bevy_gilrs` hands it a pad's: a connection, the changed buttons
/// and axes, a disconnection.
#[cfg(windows)]
fn poll(
    mut commands: Commands,
    mut pads: ResMut<SonyPads>,
    mut raw: MessageWriter<RawGamepadEvent>,
    mut connections: MessageWriter<GamepadConnectionEvent>,
    mut buttons: MessageWriter<RawGamepadButtonChangedEvent>,
    mut axes: MessageWriter<RawGamepadAxisChangedEvent>,
) {
    let SonyPads { link, pads, .. } = &mut *pads;
    let Some(link) = link else { return };
    let read: Vec<(u64, hid::Slot)> = {
        let shared = link.shared.lock().unwrap_or_else(|e| e.into_inner());
        shared.pads.iter().map(|(id, s)| (*id, s.clone())).collect()
    };
    for (id, slot) in &read {
        let Some(state) = slot.state else { continue };
        let entry = pads.entry(*id).or_insert_with(|| {
            let entity = commands.spawn(SonyPad).id();
            let event = GamepadConnectionEvent::new(
                entity,
                GamepadConnection::Connected {
                    name: slot.model.name().to_string(),
                    vendor_id: Some(report::SONY),
                    product_id: Some(slot.product),
                },
            );
            raw.write(event.clone().into());
            connections.write(event);
            (entity, None)
        });
        for e in changes(entry.0, entry.1.as_ref(), &state) {
            match &e {
                RawGamepadEvent::Button(b) => {
                    buttons.write(*b);
                }
                RawGamepadEvent::Axis(a) => {
                    axes.write(*a);
                }
                RawGamepadEvent::Connection(_) => {}
            }
            raw.write(e);
        }
        entry.1 = Some(state);
    }
    let gone: Vec<u64> = pads
        .keys()
        .filter(|id| !read.iter().any(|(r, _)| r == *id))
        .copied()
        .collect();
    for id in gone {
        if let Some((entity, _)) = pads.remove(&id) {
            let event = GamepadConnectionEvent::new(entity, GamepadConnection::Disconnected);
            raw.write(event.clone().into());
            connections.write(event);
        }
    }
}

/// The strongest shake under way on each motor, sent to the pads when it changes.
#[cfg(windows)]
fn drive_motors(time: Res<Time>, mut pads: ResMut<SonyPads>) {
    let now = time.elapsed_secs_f64();
    pads.shakes.retain(|(end, ..)| *end > now);
    let (strong, weak) = pads
        .shakes
        .iter()
        .fold((0.0f32, 0.0f32), |(s, w), (_, ps, pw)| {
            (s.max(*ps), w.max(*pw))
        });
    let want = (
        (strong.clamp(0.0, 1.0) * 255.0).round() as u8,
        (weak.clamp(0.0, 1.0) * 255.0).round() as u8,
    );
    if want != pads.sent {
        if let Some(link) = &pads.link {
            let _ = link.motors.send(want);
        }
        pads.sent = want;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pad_s_first_state_is_whole_and_then_only_its_changes() {
        let e = Entity::from_raw_u32(7).unwrap();
        let mut s = PadState::default();
        // Everything once: 16 buttons, 2 triggers, 4 axes.
        assert_eq!(changes(e, None, &s).len(), 22);
        let old = s;
        s.buttons |= Button::Cross.bit();
        s.sticks[0] = 0.5;
        s.r2 = 1.0;
        let c = changes(e, Some(&old), &s);
        assert_eq!(c.len(), 3);
        assert!(c.iter().any(|e| matches!(e, RawGamepadEvent::Button(b)
            if b.button == GamepadButton::South && b.value == 1.0)));
        assert!(c.iter().any(|e| matches!(e, RawGamepadEvent::Button(b)
            if b.button == GamepadButton::RightTrigger2 && b.value == 1.0)));
        assert!(c.iter().any(|e| matches!(e, RawGamepadEvent::Axis(a)
            if a.axis == GamepadAxis::LeftStickX && a.value == 0.5)));
        assert!(changes(e, Some(&s), &s).is_empty());
    }
}
