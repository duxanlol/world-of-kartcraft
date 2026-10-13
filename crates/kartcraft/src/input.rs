//! The kart's controls: a PlayStation pad laid out as CTR's (✕ accelerates, □ brakes and reverses,
//! R1 and L1 hop, powerslide and boost, ○ fires the held item), an Xbox pad by position (A as ✕,
//! X brakes and reverses as □, B fires as ○, the shoulders hop and powerslide as R1/L1, the left
//! stick steers), or the keyboard for want of one, through [`Keys`]: the host's rebindable keys, or
//! the fixed layout. The D-pad's abilities are read by the host. Select swaps the camera's
//! distance and the right stick's click looks back, CTR's L2 and R2.

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
    /// Pressed this frame: the chase camera's near/far swap (CTR's L2).
    pub camera: bool,
    /// Held: the chase camera looks back (CTR's R2).
    pub look_back: bool,
    /// Whether a pad supplied these.
    pub pad: bool,
}

/// The keyboard's controls this frame, whichever keys drive them: the host resolves its own key
/// bindings into these, or [`Keys::from_keyboard`] reads the fixed layout.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Keys {
    pub accelerate: bool,
    pub brake: bool,
    pub left: bool,
    pub right: bool,
    /// The first shoulder: hop, and held through a turn, powerslide.
    pub hop: bool,
    /// The second shoulder, the powerslide's turbo.
    pub boost: bool,
    /// Pressed this frame.
    pub fire: bool,
    /// Pressed this frame, the D-pad's four abilities (up, right, down, left).
    pub abilities: [bool; 4],
    /// Pressed this frame: the camera's near/far swap.
    pub camera: bool,
    /// Held: the camera looks back.
    pub look_back: bool,
}

impl Keys {
    /// The fixed layout: arrows or WASD, Space to hop, Shift for the second shoulder, E to fire,
    /// digits 1–4 for the abilities, C to swap the camera's distance, X held to look back.
    pub fn from_keyboard(keys: &ButtonInput<KeyCode>) -> Keys {
        let any = |k: &[KeyCode]| keys.any_pressed(k.iter().copied());
        Keys {
            accelerate: any(&[KeyCode::ArrowUp, KeyCode::KeyW]),
            brake: any(&[KeyCode::ArrowDown, KeyCode::KeyS]),
            left: any(&[KeyCode::ArrowLeft, KeyCode::KeyA]),
            right: any(&[KeyCode::ArrowRight, KeyCode::KeyD]),
            hop: keys.pressed(KeyCode::Space),
            boost: any(&[KeyCode::ShiftLeft, KeyCode::ShiftRight]),
            fire: keys.just_pressed(KeyCode::KeyE),
            abilities: [
                keys.just_pressed(KeyCode::Digit1),
                keys.just_pressed(KeyCode::Digit2),
                keys.just_pressed(KeyCode::Digit3),
                keys.just_pressed(KeyCode::Digit4),
            ],
            camera: keys.just_pressed(KeyCode::KeyC),
            look_back: keys.pressed(KeyCode::KeyX),
        }
    }
}

/// The first connected pad's controls, else the keyboard's.
/// `held`: the buttons the interface holds while its menu or popup is up, which do not drive;
/// `all`: it holds the whole pad.
pub fn read(keys: &Keys, pads: &Query<&Gamepad>, held: &[GamepadButton], all: bool) -> Controls {
    let keyboard = from_keys(keys);
    match pads.iter().next().filter(|_| !all) {
        Some(pad) => merge(from_pad(pad, held), keyboard),
        None => keyboard,
    }
}

/// The buttons [`from_pad`] drives with, which are no binding's while we drive.
pub const DRIVING_PADS: &[GamepadButton] = &[
    GamepadButton::South,
    GamepadButton::West,
    GamepadButton::East,
    GamepadButton::RightTrigger,
    GamepadButton::LeftTrigger,
    GamepadButton::RightTrigger2,
    GamepadButton::LeftTrigger2,
    GamepadButton::DPadUp,
    GamepadButton::DPadRight,
    GamepadButton::DPadDown,
    GamepadButton::DPadLeft,
    GamepadButton::Select,
    GamepadButton::RightThumb,
];

/// How far the stick must lean before it steers over the keys.
const STICK_OVER_KEYS: f32 = 0.15;

/// The pad and the keys together, so a paired pad lying idle does not lock out the keyboard: a
/// button counts from either, and the stick steers once it leans, else the keys do.
fn merge(pad: Controls, keys: Controls) -> Controls {
    let stick = pad.steer.abs() > STICK_OVER_KEYS;
    Controls {
        steer: if stick { pad.steer } else { keys.steer },
        accelerate: pad.accelerate || keys.accelerate,
        brake: pad.brake || keys.brake,
        reverse: pad.reverse || keys.reverse,
        hop: pad.hop || keys.hop,
        hop_alt: pad.hop_alt || keys.hop_alt,
        fire: pad.fire || keys.fire,
        dpad: std::array::from_fn(|i| pad.dpad[i] || keys.dpad[i]),
        camera: pad.camera || keys.camera,
        look_back: pad.look_back || keys.look_back,
        pad: stick,
    }
}

/// The first pad's controls.
fn from_pad(pad: &Gamepad, held: &[GamepadButton]) -> Controls {
    // A button the interface holds does not drive.
    let free = |b: GamepadButton| !held.contains(&b);
    let pressed = |b| free(b) && pad.pressed(b);
    let just = |b| free(b) && pad.just_pressed(b);
    // The triggers accelerate and brake too, for those who drive a racing game that way.
    let trigger = |b| free(b) && pad.get(b).unwrap_or(0.0) > 0.4;
    Controls {
        // Raw: the driving model applies CTR's own dead zone and response curve.
        steer: pad
            .get(GamepadAxis::LeftStickX)
            .unwrap_or(0.0)
            .clamp(-1.0, 1.0),
        accelerate: pressed(GamepadButton::South) || trigger(GamepadButton::RightTrigger2),
        brake: pressed(GamepadButton::West) || trigger(GamepadButton::LeftTrigger2),
        reverse: pad.get(GamepadAxis::LeftStickY).unwrap_or(0.0) < -0.6,
        hop: pressed(GamepadButton::RightTrigger),
        hop_alt: pressed(GamepadButton::LeftTrigger),
        fire: just(GamepadButton::East),
        dpad: [
            just(GamepadButton::DPadUp),
            just(GamepadButton::DPadRight),
            just(GamepadButton::DPadDown),
            just(GamepadButton::DPadLeft),
        ],
        // CTR swaps the camera with L2 and looks back with R2, but the triggers here are the gas
        // and the brake (above); Select and the right stick's click are free on either pad.
        camera: just(GamepadButton::Select),
        look_back: pressed(GamepadButton::RightThumb),
        pad: true,
    }
}

/// The keys' controls.
fn from_keys(keys: &Keys) -> Controls {
    Controls {
        steer: f32::from(i8::from(keys.right)) - f32::from(i8::from(keys.left)),
        accelerate: keys.accelerate,
        brake: keys.brake,
        reverse: keys.brake,
        hop: keys.hop,
        hop_alt: keys.boost,
        fire: keys.fire,
        dpad: keys.abilities,
        camera: keys.camera,
        look_back: keys.look_back,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_idle_pad_leaves_the_keyboard_driving() {
        let idle_pad = Controls {
            pad: true,
            ..Default::default()
        };
        let keys = Controls {
            accelerate: true,
            steer: -1.0,
            dpad: [true, false, false, false],
            ..Default::default()
        };
        let both = merge(idle_pad, keys);
        assert!(both.accelerate, "the keys' gas");
        assert_eq!(both.steer, -1.0, "the keys steer while the stick rests");
        assert!(both.dpad[0], "and use the abilities");
        assert!(!both.pad);
    }

    #[test]
    fn a_leaning_stick_steers_over_the_keys_and_either_presses_buttons() {
        let pad = Controls {
            steer: 0.6,
            hop: true,
            pad: true,
            ..Default::default()
        };
        let keys = Controls {
            steer: -1.0,
            accelerate: true,
            ..Default::default()
        };
        let both = merge(pad, keys);
        assert_eq!(both.steer, 0.6);
        assert!(both.hop && both.accelerate);
        assert!(both.pad);
    }

    #[test]
    fn the_camera_s_swap_and_look_back_come_from_the_keys_or_the_pad() {
        let keys = from_keys(&Keys {
            camera: true,
            look_back: true,
            ..Default::default()
        });
        assert!(keys.camera && keys.look_back);
        let pad = Controls {
            look_back: true,
            pad: true,
            ..Default::default()
        };
        let both = merge(pad, Controls::default());
        assert!(both.look_back && !both.camera);
    }
}
