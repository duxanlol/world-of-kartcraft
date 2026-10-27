//! The kart's controls, through [`Keys`] twice over: the keyboard's and the controller's buttons,
//! each the host's rebindable bindings or a fixed layout ([`Keys::from_keyboard`],
//! [`Keys::from_pad`]), and the controller's left stick, which steers and, held down, reverses.
//! The pad's default layout is CTR's on a PlayStation pad (✕ accelerates, □ brakes, R1 and L1
//! hop, powerslide and boost, ○ fires the held item, the D-pad steers and its down reverses as the
//! stick does), an Xbox pad by position. R2 and L2 are the pad's modifiers, as Shift and Ctrl are
//! the keyboard's ([`PAD_MODIFIER`], [`PAD_CTRL`]): R2 with the D-pad uses the abilities, and L2
//! with a button is a binding of its own. Select swaps the camera's distance and
//! the right stick's click looks back, CTR's L2 and R2. A pad's brake is CTR's □: it reverses with
//! down held (stick or D-pad), where the keyboard's brake reverses alone.

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
    /// Held: CTR's down, reverse (full speed alone, three quarters with the brake).
    pub reverse: bool,
    /// Down on the stick reverses too: reverse is bound on the pad (a pad with it unbound
    /// reverses on neither).
    pub stick_reverses: bool,
}

impl Keys {
    /// The controller's fixed layout ([`PAD_LAYOUT`]), for a host with no bindings: the D-pad
    /// drives, and with R2 held uses the abilities instead.
    pub fn from_pad(pad: &Gamepad) -> Keys {
        let pressed = |b| pad_down(pad, b);
        let just = |b| pad.just_pressed(b);
        let modifier = modifier_down(pad, PAD_MODIFIER);
        Keys {
            accelerate: pressed(GamepadButton::South),
            brake: pressed(GamepadButton::West),
            left: !modifier && pressed(GamepadButton::DPadLeft),
            right: !modifier && pressed(GamepadButton::DPadRight),
            hop: pressed(GamepadButton::RightTrigger),
            boost: pressed(GamepadButton::LeftTrigger),
            fire: just(GamepadButton::East),
            abilities: [
                modifier && just(GamepadButton::DPadUp),
                modifier && just(GamepadButton::DPadRight),
                modifier && just(GamepadButton::DPadDown),
                modifier && just(GamepadButton::DPadLeft),
            ],
            camera: just(GamepadButton::Select),
            look_back: pressed(GamepadButton::RightThumb),
            reverse: !modifier && pressed(GamepadButton::DPadDown),
            stick_reverses: true,
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
            // The brake is down too ([`from_keys`]).
            reverse: false,
            stick_reverses: false,
        }
    }
}

/// The keyboard's controls and the first connected pad's together: `keys` the keyboard's buttons,
/// `pad_keys` the controller's (the host's bindings resolve both, leaving out a button the
/// interface holds), the stick of `sticks` (the pads whose stick has centred,
/// [`StickCentred`]); `all`: the interface holds the whole pad, its stick too.
pub fn read(
    keys: &Keys,
    pad_keys: &Keys,
    pads: &[&Gamepad],
    sticks: &[&Gamepad],
    all: bool,
) -> Controls {
    let keyboard = from_keys(keys);
    if all || pads.is_empty() {
        return keyboard;
    }
    merge(from_pad(stick(sticks), pad_keys), keyboard)
}

/// On a controller whose left stick has been seen at rest since it connected. A D-pad-only pad
/// (a USB SNES pad, an arcade stick) can report a stick it has not, held at one end: it never
/// rests, so it never steers, where a real stick rests the moment it is plugged in.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct StickCentred;

/// Marks each controller whose stick rests ([`StickCentred`]).
pub fn find_centred(
    mut commands: Commands,
    pads: Query<(Entity, &Gamepad), Without<StickCentred>>,
) {
    for (entity, pad) in &pads {
        let at = |a| pad.get(a).unwrap_or(0.0).abs();
        if at(GamepadAxis::LeftStickX) <= STICK_OVER_KEYS
            && at(GamepadAxis::LeftStickY) <= STICK_OVER_KEYS
        {
            commands.entity(entity).insert(StickCentred);
        }
    }
}

/// What [`stick_dpad`] has seen of a controller's left stick: a D-pad wired as the stick (a USB
/// SNES pad's) only ever jumps between rest and full tilt, where a real stick passes the values
/// between on its way and rests a little off centre.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct StickSeen {
    /// Whether it was at rest last frame.
    rested: bool,
    /// Jumps from rest straight to full tilt.
    jumps: u8,
    /// Seen between rest and full tilt: a real stick, never a D-pad.
    analog: bool,
    /// The D-pad buttons it holds (up, down, left, right), so it lets go only of its own.
    held: [bool; 4],
}

impl StickSeen {
    /// Whether the stick is a D-pad: only jumps, and enough of them to tell.
    pub fn digital(&self) -> bool {
        !self.analog && self.jumps >= DIGITAL_JUMPS
    }
}

/// How many jumps from rest to full tilt, with nothing between, tell a D-pad wired as a stick.
const DIGITAL_JUMPS: u8 = 3;
/// A stick component at rest under this, at full tilt over [`STICK_FULL`]; between, analog.
const STICK_REST: f32 = 0.05;
const STICK_FULL: f32 = 0.95;

/// A D-pad wired as the left stick (a USB SNES pad, some arcade sticks) presses the D-pad's
/// buttons too, so the menus, the D-pad's bindings and the modifier's (R2 with the D-pad) take it
/// as every other pad's D-pad: told by [`StickSeen`], after the input set has read the frame's
/// events and before anything reads the pads.
pub fn stick_dpad(
    mut commands: Commands,
    mut pads: Query<(Entity, &mut Gamepad, Option<&mut StickSeen>)>,
) {
    for (entity, mut pad, seen) in &mut pads {
        let at = [
            pad.get(GamepadAxis::LeftStickX).unwrap_or(0.0),
            pad.get(GamepadAxis::LeftStickY).unwrap_or(0.0),
        ];
        let Some(mut seen) = seen else {
            let rested = at.iter().all(|v| v.abs() < STICK_REST);
            commands.entity(entity).insert(StickSeen {
                rested,
                ..Default::default()
            });
            continue;
        };
        let rest = at.iter().all(|v| v.abs() < STICK_REST);
        let full = at.iter().any(|v| v.abs() > STICK_FULL);
        if at
            .iter()
            .any(|v| (STICK_REST..=STICK_FULL).contains(&v.abs()))
        {
            seen.analog = true;
        } else if seen.rested && full {
            seen.jumps = seen.jumps.saturating_add(1);
        }
        seen.rested = rest;
        if !seen.digital() {
            continue;
        }
        let [x, y] = at;
        let want = [y > 0.5, y < -0.5, x < -0.5, x > 0.5];
        let buttons = [
            GamepadButton::DPadUp,
            GamepadButton::DPadDown,
            GamepadButton::DPadLeft,
            GamepadButton::DPadRight,
        ];
        for ((button, want), held) in buttons.into_iter().zip(want).zip(&mut seen.held) {
            if want && !*held {
                pad.digital_mut().press(button);
            } else if !want && *held {
                pad.digital_mut().release(button);
            }
            *held = want;
        }
    }
}

/// A controller the player did not choose (the host's controller setting): nothing reads it.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct PadIgnored;

/// A controller's identity across runs, as the controller setting names it: its USB vendor and
/// product ids and the name the OS gives it (`054c:0ce6:PS5 Controller`).
pub fn pad_key(name: &str, vendor: Option<u16>, product: Option<u16>) -> String {
    format!(
        "{:04x}:{:04x}:{}",
        vendor.unwrap_or(0),
        product.unwrap_or(0),
        name.replace([';', '|'], " ").trim()
    )
}

/// The left stick of whichever controller leans furthest, and the furthest down any is held: a
/// second device that reports as a pad (a mapping-less virtual one) never hides the real one's.
pub fn stick(pads: &[&Gamepad]) -> Vec2 {
    let axis = |p: &Gamepad, a| p.get(a).unwrap_or(0.0).clamp(-1.0, 1.0);
    lean(pads.iter().map(|p| {
        Vec2::new(
            axis(p, GamepadAxis::LeftStickX),
            axis(p, GamepadAxis::LeftStickY),
        )
    }))
}

/// [`stick`]'s choice among the pads' sticks: the widest lean across, the furthest down.
fn lean(sticks: impl Iterator<Item = Vec2>) -> Vec2 {
    sticks.fold(Vec2::ZERO, |acc, s| {
        Vec2::new(
            if s.x.abs() > acc.x.abs() { s.x } else { acc.x },
            acc.y.min(s.y),
        )
    })
}

/// The pad's modifier, held as the keyboard's Shift is: a button pressed with it is its
/// `SHIFT-` binding (R2 with the D-pad, the abilities).
pub const PAD_MODIFIER: GamepadButton = GamepadButton::RightTrigger2;
/// The pad's second modifier, held as the keyboard's Ctrl is: a button pressed with it is its
/// `CTRL-` binding (none by default, the player's to bind).
pub const PAD_CTRL: GamepadButton = GamepadButton::LeftTrigger2;

static TRIGGERS_ARE_BUTTONS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// The host's setting (`KartPadTriggers`): R2 and L2 plain buttons, bound alone, instead of the
/// pad's modifiers.
pub fn set_triggers_are_buttons(on: bool) {
    TRIGGERS_ARE_BUTTONS.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Whether R2 and L2 are plain buttons ([`set_triggers_are_buttons`]).
pub fn triggers_are_buttons() -> bool {
    TRIGGERS_ARE_BUTTONS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Whether `pad` holds the modifier `m` ([`PAD_MODIFIER`], [`PAD_CTRL`]): never while the
/// triggers are plain buttons.
pub fn modifier_down(pad: &Gamepad, m: GamepadButton) -> bool {
    !triggers_are_buttons() && pad_down(pad, m)
}

/// How far a trigger (or any button with an analog travel) must go to count as pressed, as the
/// kart's triggers always took a light squeeze.
pub const ANALOG_PRESS: f32 = 0.4;

/// Whether `pad` holds `b`: pressed, or squeezed past [`ANALOG_PRESS`].
pub fn pad_down(pad: &Gamepad, b: GamepadButton) -> bool {
    pad.pressed(b) || pad.get(b).unwrap_or(0.0) > ANALOG_PRESS
}

/// The controller's default layout by binding name (`PAD1` ✕ or A, `PAD2` ○ or B, `PAD3` □ or
/// X, the shoulders, the triggers, the D-pad, `PADBACK` Select, `PADRSTICK` the right stick's
/// click), as the host ships it beside the keyboard's: what [`Keys::from_pad`] reads.
pub const PAD_LAYOUT: &[(&str, PadControl)] = &[
    ("PAD1", PadControl::Accelerate),
    ("PAD3", PadControl::Brake),
    ("PADDLEFT", PadControl::SteerLeft),
    ("PADDRIGHT", PadControl::SteerRight),
    ("PADDDOWN", PadControl::Reverse),
    ("PADRSHOULDER", PadControl::Hop),
    ("PADLSHOULDER", PadControl::Boost),
    ("PAD2", PadControl::Fire),
    ("SHIFT-PADDUP", PadControl::Ability(0)),
    ("SHIFT-PADDRIGHT", PadControl::Ability(1)),
    ("SHIFT-PADDDOWN", PadControl::Ability(2)),
    ("SHIFT-PADDLEFT", PadControl::Ability(3)),
    ("PADBACK", PadControl::Camera),
    ("PADRSTICK", PadControl::LookBack),
];

/// A control a pad button drives in [`PAD_LAYOUT`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadControl {
    Accelerate,
    Brake,
    SteerLeft,
    SteerRight,
    Reverse,
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

/// The pads' controls: the stick ([`stick`]), and the buttons as `keys` resolves them. Buttons
/// bound to steering steer while held, else the stick once it leans (raw: the driving model
/// applies CTR's own dead zone and response curve); down on it reverses, with the brake (CTR's □).
fn from_pad(stick_at: Vec2, keys: &Keys) -> Controls {
    let stick = stick_at.x;
    let buttons = f32::from(i8::from(keys.right)) - f32::from(i8::from(keys.left));
    Controls {
        steer: if buttons == 0.0 && stick.abs() > STICK_OVER_KEYS {
            stick
        } else {
            buttons
        },
        accelerate: keys.accelerate,
        brake: keys.brake,
        reverse: (keys.stick_reverses && stick_at.y < -0.6) || keys.reverse,
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
        reverse: keys.brake || keys.reverse,
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

/// `WOW_KART_AUTOPILOT=fly`: the autopilot drives straight and, in the air, taps the hop button
/// twice in a row now and then, which opens the wings ([`autopilot_fly`]).
pub fn autopilot_flies() -> bool {
    std::env::var("WOW_KART_AUTOPILOT").is_ok_and(|v| v == "fly")
}

/// The flying autopilot's controls `t` seconds in: full gas straight on, a hop every two seconds,
/// and off the ground a hop tapped every fourth frame, a double tap.
pub fn autopilot_fly(t: f32, grounded: bool) -> Controls {
    let frame = (t * 30.0) as i32;
    Controls {
        accelerate: true,
        hop: if grounded {
            frame % 60 < 2
        } else {
            frame % 4 < 2
        },
        ..Default::default()
    }
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
        let c = from_pad(
            Vec2::ZERO,
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
    fn the_d_pad_drives_and_with_r2_held_uses_the_abilities() {
        let mut pad = Gamepad::default();
        pad.digital_mut().press(GamepadButton::DPadLeft);
        pad.digital_mut().press(GamepadButton::DPadDown);
        let k = Keys::from_pad(&pad);
        assert!(k.left && k.reverse && !k.right);
        assert_eq!(k.abilities, [false; 4], "no ability without R2");
        let c = from_pad(Vec2::ZERO, &k);
        assert_eq!(c.steer, -1.0);
        assert!(c.reverse, "D-pad down is CTR's down");
        // R2 held: the same presses are the abilities, and do not drive.
        let mut pad = Gamepad::default();
        pad.digital_mut().press(PAD_MODIFIER);
        pad.digital_mut().press(GamepadButton::DPadLeft);
        let k = Keys::from_pad(&pad);
        assert_eq!(k.abilities, [false, false, false, true]);
        assert!(
            !k.left && !k.reverse && !k.accelerate,
            "R2 is no longer the gas"
        );
    }

    #[test]
    fn down_on_the_stick_reverses_only_while_reverse_is_bound_on_the_pad() {
        let down = Vec2::new(0.0, -1.0);
        let bound = Keys {
            stick_reverses: true,
            ..Default::default()
        };
        assert!(from_pad(down, &bound).reverse);
        assert!(!from_pad(down, &Keys::default()).reverse, "unbound");
    }

    #[test]
    fn the_d_pad_held_steers_over_the_stick() {
        let left = Keys {
            left: true,
            ..Default::default()
        };
        assert_eq!(from_pad(Vec2::new(1.0, 0.0), &left).steer, -1.0);
        // Let go, the leaning stick steers again.
        assert_eq!(from_pad(Vec2::new(0.7, 0.0), &Keys::default()).steer, 0.7);
    }

    /// A D-pad wired as the stick (a NEXT SNES pad's) presses the D-pad's buttons once it has
    /// jumped from rest to full tilt a few times with nothing between; a real stick never does.
    #[test]
    fn a_d_pad_wired_as_the_stick_presses_the_d_pad() {
        let mut app = App::new();
        app.add_systems(Update, stick_dpad);
        let snes = app.world_mut().spawn(Gamepad::default()).id();
        let real = app.world_mut().spawn(Gamepad::default()).id();
        let set = |app: &mut App, pad: Entity, x: f32| {
            let mut p = app.world_mut().get_mut::<Gamepad>(pad).unwrap();
            p.analog_mut().set(GamepadAxis::LeftStickX, x);
        };
        app.update();
        // The real stick passes through the middle on its way.
        for (sx, rx) in [
            (-1.0, -0.4),
            (0.0, 0.0),
            (1.0, 1.0),
            (0.0, 0.0),
            (-1.0, -1.0),
            (0.0, 0.0),
        ] {
            set(&mut app, snes, sx);
            set(&mut app, real, rx);
            app.update();
        }
        let seen = |app: &App, pad| *app.world().get::<StickSeen>(pad).unwrap();
        assert!(seen(&app, snes).digital(), "{:?}", seen(&app, snes));
        assert!(!seen(&app, real).digital());
        // Left on the SNES pad's D-pad: DPadLeft down, and up again as it lets go.
        set(&mut app, snes, -1.0);
        set(&mut app, real, -1.0);
        app.update();
        let pad = |app: &App, e| {
            app.world()
                .get::<Gamepad>(e)
                .unwrap()
                .pressed(GamepadButton::DPadLeft)
        };
        assert!(pad(&app, snes));
        assert!(!pad(&app, real), "a real stick never presses the D-pad");
        set(&mut app, snes, 0.0);
        app.update();
        assert!(!pad(&app, snes));
    }

    #[test]
    fn a_stick_held_at_one_end_since_it_connected_never_steers() {
        let mut app = App::new();
        app.add_systems(Update, find_centred);
        // A D-pad-only pad reporting a stick it has not, full right; a real one at rest.
        let mut retro = Gamepad::default();
        retro.analog_mut().set(GamepadAxis::LeftStickX, 1.0);
        let retro = app.world_mut().spawn(retro).id();
        let real = app.world_mut().spawn(Gamepad::default()).id();
        app.update();
        assert!(app.world().get::<StickCentred>(retro).is_none());
        assert!(app.world().get::<StickCentred>(real).is_some());
        // The retro pad's D-pad drives, its phantom stick read by nobody.
        let retro_pad = app.world().get::<Gamepad>(retro).unwrap();
        let k = Keys::default();
        let c = read(&k, &k, &[retro_pad], &[], false);
        assert_eq!(c.steer, 0.0);
        let c = read(&k, &Keys { right: true, ..k }, &[retro_pad], &[], false);
        assert_eq!(c.steer, 1.0);
    }

    #[test]
    fn a_resting_phantom_pad_does_not_hide_the_real_stick() {
        // A mapping-less device that reports as a pad, idle, beside the real one steering left
        // and held down; in either order.
        let phantom = Vec2::ZERO;
        let real = Vec2::new(-0.8, -0.7);
        assert_eq!(lean([phantom, real].into_iter()), real);
        assert_eq!(lean([real, phantom].into_iter()), real);
        assert_eq!(lean(std::iter::empty()), Vec2::ZERO);
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
