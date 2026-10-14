//! The kart's key bindings: the layer's `Bindings.xml` under World of Kartcraft, rebindable in Key
//! Bindings as an addon's are. The driving controls are an overlay plane
//! ([`crate::bindings::BindingOverlay`]): their default keys are the ones WoW walks and casts with
//! (W, A, S, D, the arrows, Space, 1–4, C and X), which still walk, cast, open the character pane
//! and sit out of the kart; while we drive they are the kart's alone. The controller's buttons are
//! bindings too, Kartcraft's deviation: each driving control ships with its pad button beside its
//! keys ([`kartcraft::input::PAD_LAYOUT`]), rebindable on the Key Bindings page's Controller
//! column: the D-pad steers and reverses as the left stick does, and R2 is the pad's Shift and L2
//! its Ctrl, so the abilities are R2 with the D-pad (`SHIFT-PADDUP`, ...). Summon Kart is an ordinary binding,
//! read by its press.

use kartcraft::input::Keys;

use crate::bindings::BindingOverlay;

pub(super) const ACCELERATE: &str = "KARTCRAFT_ACCELERATE";
pub(super) const BRAKE: &str = "KARTCRAFT_BRAKE";
pub(super) const STEER_LEFT: &str = "KARTCRAFT_STEERLEFT";
pub(super) const STEER_RIGHT: &str = "KARTCRAFT_STEERRIGHT";
/// CTR's down: reverse, full speed alone and three quarters with the brake (the keyboard's brake
/// is both).
pub(super) const REVERSE: &str = "KARTCRAFT_REVERSE";
pub(super) const HOP: &str = "KARTCRAFT_HOP";
pub(super) const BOOST: &str = "KARTCRAFT_BOOST";
pub(super) const FIRE: &str = "KARTCRAFT_FIRE";
/// CTR's L2 and R2: the chase camera's near/far swap (pressed) and its look-back (held).
pub(super) const CAMERA: &str = "KARTCRAFT_CAMERA";
pub(super) const LOOK_BACK: &str = "KARTCRAFT_LOOKBACK";
/// The D-pad's abilities in its order (up, right, down, left): Wumpa Juice, Turbo Boost, Super
/// Engine, Aku Aku (`profession::DPAD`).
pub(super) const ABILITIES: [&str; 4] = [
    "KARTCRAFT_ABILITY1",
    "KARTCRAFT_ABILITY2",
    "KARTCRAFT_ABILITY3",
    "KARTCRAFT_ABILITY4",
];
/// Summon Kart, or on a stock server in and out of the kart.
pub(super) const SUMMON: &str = "KARTCRAFT_SUMMON";
/// A time trial of ours starts over from its line ([`super::race::restart_line`]).
pub(super) const RESTART: &str = "KARTCRAFT_RESTART";

/// The overlay plane: the controls the kart reads while it drives.
const DRIVING: &[&str] = &[
    ACCELERATE,
    BRAKE,
    STEER_LEFT,
    STEER_RIGHT,
    REVERSE,
    HOP,
    BOOST,
    FIRE,
    CAMERA,
    LOOK_BACK,
    ABILITIES[0],
    ABILITIES[1],
    ABILITIES[2],
    ABILITIES[3],
];

/// The default keys, the kart's fixed layout before it had bindings. `SHIFT-` is Shift alone, a
/// key string 1.12's `SetBinding` accepts. Summon and the Race Finder keep theirs only where the
/// stock defaults bind nothing; the Leave Race, settings and garage bindings start unbound.
const DEFAULTS: &[(&str, &str)] = &[
    ("W", ACCELERATE),
    ("UP", ACCELERATE),
    ("S", BRAKE),
    ("DOWN", BRAKE),
    ("A", STEER_LEFT),
    ("LEFT", STEER_LEFT),
    ("D", STEER_RIGHT),
    ("RIGHT", STEER_RIGHT),
    ("SPACE", HOP),
    ("SHIFT-", BOOST),
    ("E", FIRE),
    ("C", CAMERA),
    ("X", LOOK_BACK),
    ("1", ABILITIES[0]),
    ("2", ABILITIES[1]),
    ("3", ABILITIES[2]),
    ("4", ABILITIES[3]),
    ("J", SUMMON),
    ("I", "KARTCRAFT_RACEFINDER"),
    // The controller's driving buttons, CTR's layout ([`kartcraft::input::PAD_LAYOUT`]): the D-pad
    // drives, R2 with it uses the abilities.
    ("PAD1", ACCELERATE),
    ("PAD3", BRAKE),
    ("PADDLEFT", STEER_LEFT),
    ("PADDRIGHT", STEER_RIGHT),
    ("PADDDOWN", REVERSE),
    ("PADRSHOULDER", HOP),
    ("PADLSHOULDER", BOOST),
    ("PAD2", FIRE),
    ("SHIFT-PADDUP", ABILITIES[0]),
    ("SHIFT-PADDRIGHT", ABILITIES[1]),
    ("SHIFT-PADDDOWN", ABILITIES[2]),
    ("SHIFT-PADDLEFT", ABILITIES[3]),
    ("PADBACK", CAMERA),
    ("PADRSTICK", LOOK_BACK),
    // And its free ones: △ or Y summons the kart, Options or Menu opens the Kartcraft menu.
    ("PAD4", SUMMON),
    ("PADFORWARD", "KARTCRAFT_MENU"),
];

/// The command a [`kartcraft::input::PadControl`] is.
#[cfg(test)]
fn command_of(control: kartcraft::input::PadControl) -> &'static str {
    use kartcraft::input::PadControl;
    match control {
        PadControl::Accelerate => ACCELERATE,
        PadControl::Brake => BRAKE,
        PadControl::SteerLeft => STEER_LEFT,
        PadControl::SteerRight => STEER_RIGHT,
        PadControl::Reverse => REVERSE,
        PadControl::Hop => HOP,
        PadControl::Boost => BOOST,
        PadControl::Fire => FIRE,
        PadControl::Ability(i) => ABILITIES[i.min(3)],
        PadControl::Camera => CAMERA,
        PadControl::LookBack => LOOK_BACK,
    }
}

pub(super) fn plugin(app: &mut bevy::prelude::App) {
    app.insert_resource(BindingOverlay::new(DRIVING, DEFAULTS));
}

/// The keyboard's controls this frame, off the keys bound to the driving controls.
pub(super) fn keys(overlay: &BindingOverlay) -> Keys {
    Keys {
        accelerate: overlay.held(ACCELERATE),
        brake: overlay.held(BRAKE),
        left: overlay.held(STEER_LEFT),
        right: overlay.held(STEER_RIGHT),
        hop: overlay.held(HOP),
        boost: overlay.held(BOOST),
        fire: overlay.just_pressed(FIRE),
        abilities: ABILITIES.map(|a| overlay.just_pressed(a)),
        camera: overlay.just_pressed(CAMERA),
        look_back: overlay.held(LOOK_BACK),
        reverse: overlay.held(REVERSE),
    }
}

/// The controller's buttons' controls this frame, off the buttons bound to the driving controls.
pub(super) fn pad_keys(overlay: &BindingOverlay) -> Keys {
    Keys {
        accelerate: overlay.pad_held(ACCELERATE),
        brake: overlay.pad_held(BRAKE),
        left: overlay.pad_held(STEER_LEFT),
        right: overlay.pad_held(STEER_RIGHT),
        hop: overlay.pad_held(HOP),
        boost: overlay.pad_held(BOOST),
        fire: overlay.pad_just_pressed(FIRE),
        abilities: ABILITIES.map(|a| overlay.pad_just_pressed(a)),
        camera: overlay.pad_just_pressed(CAMERA),
        look_back: overlay.pad_held(LOOK_BACK),
        reverse: overlay.pad_held(REVERSE),
    }
}

#[cfg(test)]
mod tests {
    use benilla_ui::script::UiScript;

    use super::*;

    /// The controller buttons the driving controls ship with are the layout the kart's own fixed
    /// reader drives with ([`kartcraft::input::PAD_LAYOUT`], the offline demo's), button for button.
    #[test]
    fn the_pad_defaults_are_the_kart_s_pad_layout() {
        let shipped: Vec<(&str, &str)> = DEFAULTS
            .iter()
            .copied()
            .filter(|(k, c)| crate::bindings::store::is_pad_key(k) && DRIVING.contains(c))
            .collect();
        let layout: Vec<(&str, &str)> = kartcraft::input::PAD_LAYOUT
            .iter()
            .map(|&(k, control)| (k, command_of(control)))
            .collect();
        assert_eq!(shipped, layout);
    }

    /// The kart's rows over the install's own defaults, seeded as `seed_bindings_for_vm` seeds
    /// them: every control in the table with its default keys beside the stock commands' own.
    #[test]
    fn the_kart_s_bindings_register_with_their_defaults_beside_wow_s() {
        benilla_formats::wow_data_or_skip!();
        let overlay = BindingOverlay::new(DRIVING, DEFAULTS);
        let mut s = UiScript::new().unwrap();
        s.set_binding_overlay(DRIVING);
        let stock = crate::ui_script::default_bindings();
        s.set_default_bindings(overlay.defaults_over(stock.clone()));
        s.load_binding_set(1);
        let xml = include_str!("../../assets/ui/Bindings.xml");
        s.register_addon_bindings("benilla", &benilla_ui::bindings_xml::parse(xml).unwrap());
        let keys = |name: &str| -> Vec<String> {
            s.keybind_snapshot()
                .into_iter()
                .find(|(n, _)| n == name)
                .map(|(_, k)| k)
                .unwrap_or_default()
        };
        // Each driving control: its keys, then its controller buttons.
        assert_eq!(keys(ACCELERATE), ["W", "UP", "PAD1"]);
        assert_eq!(keys(BRAKE), ["S", "DOWN", "PAD3"]);
        assert_eq!(keys(STEER_LEFT), ["A", "LEFT", "PADDLEFT"]);
        assert_eq!(keys(STEER_RIGHT), ["D", "RIGHT", "PADDRIGHT"]);
        assert_eq!(keys(REVERSE), ["PADDDOWN"]);
        assert_eq!(keys(HOP), ["SPACE", "PADRSHOULDER"]);
        assert_eq!(keys(BOOST), ["SHIFT-", "PADLSHOULDER"]);
        assert_eq!(keys(FIRE), ["E", "PAD2"]);
        assert_eq!(keys(CAMERA), ["C", "PADBACK"]);
        assert_eq!(keys(LOOK_BACK), ["X", "PADRSTICK"]);
        for (i, a) in ABILITIES.iter().enumerate() {
            let dpad = [
                "SHIFT-PADDUP",
                "SHIFT-PADDRIGHT",
                "SHIFT-PADDDOWN",
                "SHIFT-PADDLEFT",
            ][i];
            assert_eq!(keys(a), [format!("{}", i + 1), dpad.to_string()]);
        }
        // WoW's keep theirs.
        assert!(keys("MOVEFORWARD").contains(&"W".to_string()));
        assert!(keys("JUMP").contains(&"SPACE".to_string()));
        assert_eq!(keys("ACTIONBUTTON1"), ["1"]);
        assert_eq!(keys("TOGGLECHARACTER0"), ["C"]);
        assert_eq!(keys("SITORSTAND"), ["X"]);
        // The ordinary ones take a key only where the stock defaults leave it free.
        for (key, command) in [("J", SUMMON), ("I", "KARTCRAFT_RACEFINDER")] {
            let stock_holds = stock.iter().any(|(k, _)| k == key);
            assert_eq!(keys(command).is_empty(), stock_holds, "{command} on {key}");
        }
        for command in [
            "KARTCRAFT_LEAVERACE",
            "KARTCRAFT_UNSTUCK",
            RESTART,
            "KARTCRAFT_SETTINGS",
            "KARTCRAFT_GARAGE",
            "KARTCRAFT_CLEANSCREEN",
            "KARTCRAFT_FREECAM",
            "KARTCRAFT_NEXTKART",
            "KARTCRAFT_GUIDE",
        ] {
            assert!(keys(command).is_empty(), "{command} starts unbound");
        }
        // Every row the file declares is a command the kart names or a Lua one.
        assert_eq!(
            benilla_ui::bindings_xml::parse(xml).unwrap().len(),
            DRIVING.len() + 12
        );
    }
}
