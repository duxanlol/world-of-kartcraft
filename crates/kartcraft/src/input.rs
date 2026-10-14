//! The kart's controls, through [`Keys`] twice over: the keyboard's and the controller's buttons,
//! each the host's rebindable bindings or a fixed layout ([`Keys::from_keyboard`],
//! [`Keys::from_pad`]), and the controller's left stick, which steers and, held down, reverses.
//! The pad's default layout is CTR's on a PlayStation pad (✕ or R2 accelerates, □ or L2 brakes,
//! R1 and L1 hop, powerslide and boost, ○ fires the held item, the D-pad's abilities), an Xbox pad
//! by position; Select swaps the camera's distance and the right stick's click looks back, CTR's L2
//! and R2. A pad's brake is CTR's □: it reverses with the stick held down, where the keyboard's
//! brake reverses alone.

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
    /// The controller's fixed layout ([`PAD_LAYOUT`]), for a host with no bindings.
    pub fn from_pad(pad: &Gamepad) -> Keys {
        let pressed = |b| pad.pressed(b);
        let just = |b| pad.just_pressed(b);
        Keys {
            accelerate: pressed(GamepadButton::South) || pressed(GamepadButton::RightTrigger2),
            brake: pressed(GamepadButton::West) || pressed(GamepadButton::LeftTrigger2),
            left: false,
            right: false,
            hop: pressed(GamepadButton::RightTrigger),
            boost: pressed(GamepadButton::LeftTrigger),
            fire: just(GamepadButton::East),
            abilities: [
                just(GamepadButton::DPadUp),
                just(GamepadButton::DPadRight),
                just(GamepadButton::DPadDown),
                just(GamepadButton::DPadLeft),
            ],
            camera: just(GamepadButton::Select),
            look_back: pressed(GamepadButton::RightThumb),
        }
    }

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

/// The keyboard's controls and the first connected pad's together: `keys` the keyboard's buttons,
/// `pad_keys` the controller's (the host's bindings resolve both, leaving out a button the
/// interface holds); `all`: the interface holds the whole pad, its stick too.
pub fn read(keys: &Keys, pad_keys: &Keys, pads: &Query<&Gamepad>, all: bool) -> Controls {
    let keyboard = from_keys(keys);
    match pads.iter().next().filter(|_| !all) {
        Some(pad) => merge(from_pad(pad, pad_keys), keyboard),
        None => keyboard,
    }
}

/// The controller's default layout by binding name (`PAD1` ✕ or A, `PAD2` ○ or B, `PAD3` □ or
/// X, the shoulders, the triggers, the D-pad, `PADBACK` Select, `PADRSTICK` the right stick's
/// click), as the host ships it beside the keyboard's: what [`Keys::from_pad`] reads.
pub const PAD_LAYOUT: &[(&str, PadControl)] = &[
    ("PAD1", PadControl::Accelerate),
    ("PADRTRIGGER", PadControl::Accelerate),
    ("PAD3", PadControl::Brake),
    ("PADLTRIGGER", PadControl::Brake),
    ("PADRSHOULDER", PadControl::Hop),
    ("PADLSHOULDER", PadControl::Boost),
    ("PAD2", PadControl::Fire),
    ("PADDUP", PadControl::Ability(0)),
    ("PADDRIGHT", PadControl::Ability(1)),
    ("PADDDOWN", PadControl::Ability(2)),
    ("PADDLEFT", PadControl::Ability(3)),
    ("PADBACK", PadControl::Camera),
    ("PADRSTICK", PadControl::LookBack),
];

/// A control a pad button drives in [`PAD_LAYOUT`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadControl {
    Accelerate,
    Brake,
    Hop,
    Boost,
    Fire,
    Ability(usize),
    Camera,
    LookBack,
}

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

/// The first pad's controls: its stick, and its buttons as `keys` resolves them. The stick steers
/// once it leans (raw: the driving model applies CTR's own dead zone and response curve), else
/// buttons bound to steering do; down on it reverses, with the brake (CTR's □).
fn from_pad(pad: &Gamepad, keys: &Keys) -> Controls {
    let stick = pad
        .get(GamepadAxis::LeftStickX)
        .unwrap_or(0.0)
        .clamp(-1.0, 1.0);
    let buttons = f32::from(i8::from(keys.right)) - f32::from(i8::from(keys.left));
    Controls {
        steer: if stick.abs() > STICK_OVER_KEYS {
            stick
        } else {
            buttons
        },
        accelerate: keys.accelerate,
        brake: keys.brake,
        reverse: pad.get(GamepadAxis::LeftStickY).unwrap_or(0.0) < -0.6,
        hop: keys.hop,
        hop_alt: keys.boost,
        fire: keys.fire,
        dpad: keys.abilities,
        camera: keys.camera,
        look_back: keys.look_back,
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
    fn a_pad_s_buttons_drive_as_bound_and_its_brake_reverses_only_with_the_stick() {
        let pad = Gamepad::default();
        let c = from_pad(
            &pad,
            &Keys {
                brake: true,
                left: true,
                fire: true,
                ..Default::default()
            },
        );
        assert!(c.brake && c.fire && c.pad);
        assert!(
            !c.reverse,
            "CTR's square brakes; down on the stick reverses"
        );
        assert_eq!(
            c.steer, -1.0,
            "buttons bound to steering steer while the stick rests"
        );
        // The keyboard's brake reverses on its own.
        assert!(
            from_keys(&Keys {
                brake: true,
                ..Default::default()
            })
            .reverse
        );
    }

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
