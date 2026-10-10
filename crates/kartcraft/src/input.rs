//! The kart's controls: a PlayStation pad laid out as CTR's (✕ accelerates, □ brakes and reverses,
//! R1 and L1 hop, powerslide and boost, ○ fires the held item), an Xbox pad by position (A as ✕,
//! X brakes and reverses as □, B fires as ○, the shoulders hop and powerslide as R1/L1, the left
//! stick steers), or the keyboard for want of one. While we drive these keys belong to the kart;
//! the D-pad's abilities and `J`/`K` are read by the update.

use bevy::input::gamepad::{Gamepad, GamepadAxis, GamepadButton};
use bevy::prelude::*;

/// One frame of controls, as the driving model reads them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Controls {
    /// −1 full left to 1 full right.
    pub steer: f32,
    pub accelerate: bool,
    pub brake: bool,
    /// Down on the stick: reverse (with the brake, CTR's brake-then-reverse).
    pub reverse: bool,
    /// Either shoulder: hop, and held through a turn, powerslide.
    pub hop: bool,
    /// The other shoulder during a powerslide fires its turbo; CTR takes either.
    pub hop_alt: bool,
    /// ○ pressed this frame: fire the held item, stop the roulette (`VehPhysProc.c:801-905`);
    /// with `reverse` held a bomb or missile goes backwards (`VehPickupItem.c:829-835`).
    pub fire: bool,
    /// D-pad presses this frame (up, right, down, left), for the abilities.
    pub dpad: [bool; 4],
    /// Whether a pad supplied these.
    pub pad: bool,
}

/// The first connected pad's controls, else the keyboard's (arrows or WASD, Space to hop,
/// Shift for the second shoulder, E to fire, digits 1–4 for the abilities).
pub fn read(keys: &ButtonInput<KeyCode>, pads: &Query<&Gamepad>) -> Controls {
    if let Some(pad) = pads.iter().next() {
        // The triggers accelerate and brake too, for those who drive a racing game that way.
        let trigger = |b| pad.get(b).unwrap_or(0.0) > 0.4;
        return Controls {
            // Raw: the driving model applies CTR's own dead zone and response curve.
            steer: pad
                .get(GamepadAxis::LeftStickX)
                .unwrap_or(0.0)
                .clamp(-1.0, 1.0),
            accelerate: pad.pressed(GamepadButton::South) || trigger(GamepadButton::RightTrigger2),
            brake: pad.pressed(GamepadButton::West) || trigger(GamepadButton::LeftTrigger2),
            reverse: pad.get(GamepadAxis::LeftStickY).unwrap_or(0.0) < -0.6,
            hop: pad.pressed(GamepadButton::RightTrigger),
            hop_alt: pad.pressed(GamepadButton::LeftTrigger),
            fire: pad.just_pressed(GamepadButton::East),
            dpad: [
                pad.just_pressed(GamepadButton::DPadUp),
                pad.just_pressed(GamepadButton::DPadRight),
                pad.just_pressed(GamepadButton::DPadDown),
                pad.just_pressed(GamepadButton::DPadLeft),
            ],
            pad: true,
        };
    }
    let any = |k: &[KeyCode]| keys.any_pressed(k.iter().copied());
    let left = any(&[KeyCode::ArrowLeft, KeyCode::KeyA]);
    let right = any(&[KeyCode::ArrowRight, KeyCode::KeyD]);
    Controls {
        steer: f32::from(i8::from(right)) - f32::from(i8::from(left)),
        accelerate: any(&[KeyCode::ArrowUp, KeyCode::KeyW]),
        brake: any(&[KeyCode::ArrowDown, KeyCode::KeyS]),
        reverse: any(&[KeyCode::ArrowDown, KeyCode::KeyS]),
        hop: keys.pressed(KeyCode::Space),
        hop_alt: any(&[KeyCode::ShiftLeft, KeyCode::ShiftRight]),
        fire: keys.just_pressed(KeyCode::KeyE),
        dpad: [
            keys.just_pressed(KeyCode::Digit1),
            keys.just_pressed(KeyCode::Digit2),
            keys.just_pressed(KeyCode::Digit3),
            keys.just_pressed(KeyCode::Digit4),
        ],
        pad: false,
    }
}

/// Names each controller as it connects or goes, so a pad that is not picked up shows in the log.
/// A DualSense or DualShock reads natively (gilrs over Windows.Gaming.Input with SDL's mappings,
/// evdev on Linux): no Xbox emulation is needed, and its buttons land where CTR had them.
pub fn log_pads(mut events: MessageReader<bevy::input::gamepad::GamepadConnectionEvent>) {
    use bevy::input::gamepad::GamepadConnection;
    for e in events.read() {
        match &e.connection {
            GamepadConnection::Connected {
                name,
                vendor_id,
                product_id,
            } => info!(
                "kart: controller connected: {name} (vendor {vendor_id:04x?}, product {product_id:04x?})"
            ),
            GamepadConnection::Disconnected => info!("kart: controller disconnected"),
        }
    }
}

/// `WOW_KART_AUTOPILOT`: drive by ourselves, for unattended runs.
pub fn autopilot_on() -> bool {
    std::env::var_os("WOW_KART_AUTOPILOT").is_some()
}

/// The autopilot's controls `t` seconds in: gas, a long sweeping powerslide with a boost now and
/// then in the meter's red, then straight again.
pub fn autopilot(t: f32, meter: Option<(f32, bool)>) -> Controls {
    let cycle = t % 8.0;
    let sliding = (2.0..6.0).contains(&cycle);
    Controls {
        accelerate: true,
        steer: if sliding { -1.0 } else { 0.0 },
        hop: sliding,
        hop_alt: meter.is_some_and(|(_, red)| red) && (t * 10.0) as i32 % 3 == 0,
        ..Default::default()
    }
}

/// The autopilot's ○: a press every [`AUTOFIRE_SECS`], on the frame `t` crosses into a new
/// period (`dt` the frame's length), so a held item goes off now and then and a roulette is
/// stopped early.
pub fn autofire(t: f32, dt: f32) -> bool {
    (t / AUTOFIRE_SECS).floor() != ((t - dt) / AUTOFIRE_SECS).floor()
}

/// Seconds between the autopilot's presses of ○.
const AUTOFIRE_SECS: f32 = 1.5;
