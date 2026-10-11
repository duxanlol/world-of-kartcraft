//! The kart's key bindings: the layer's `Bindings.xml` under World of Kartcraft, rebindable in Key
//! Bindings as an addon's are. The driving controls are an overlay plane
//! ([`crate::bindings::BindingOverlay`]): their default keys are the ones WoW walks and casts with
//! (W, A, S, D, the arrows, Space, 1–4), which still walk and cast out of the kart; while we drive
//! they are the kart's alone. Summon Kart is an ordinary binding, read by its press.

use kartcraft::input::Keys;

use crate::bindings::BindingOverlay;

pub(super) const ACCELERATE: &str = "KARTCRAFT_ACCELERATE";
pub(super) const BRAKE: &str = "KARTCRAFT_BRAKE";
pub(super) const STEER_LEFT: &str = "KARTCRAFT_STEERLEFT";
pub(super) const STEER_RIGHT: &str = "KARTCRAFT_STEERRIGHT";
pub(super) const HOP: &str = "KARTCRAFT_HOP";
pub(super) const BOOST: &str = "KARTCRAFT_BOOST";
pub(super) const FIRE: &str = "KARTCRAFT_FIRE";
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

/// The overlay plane: the controls the kart reads while it drives.
const DRIVING: &[&str] = &[
    ACCELERATE,
    BRAKE,
    STEER_LEFT,
    STEER_RIGHT,
    HOP,
    BOOST,
    FIRE,
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
    ("1", ABILITIES[0]),
    ("2", ABILITIES[1]),
    ("3", ABILITIES[2]),
    ("4", ABILITIES[3]),
    ("J", SUMMON),
    ("I", "KARTCRAFT_RACEFINDER"),
];

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
    }
}

#[cfg(test)]
mod tests {
    use benilla_ui::script::UiScript;

    use super::*;

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
        assert_eq!(keys(ACCELERATE), ["W", "UP"]);
        assert_eq!(keys(BRAKE), ["S", "DOWN"]);
        assert_eq!(keys(STEER_LEFT), ["A", "LEFT"]);
        assert_eq!(keys(STEER_RIGHT), ["D", "RIGHT"]);
        assert_eq!(keys(HOP), ["SPACE"]);
        assert_eq!(keys(BOOST), ["SHIFT-"]);
        assert_eq!(keys(FIRE), ["E"]);
        for (i, a) in ABILITIES.iter().enumerate() {
            assert_eq!(keys(a), [format!("{}", i + 1)]);
        }
        // WoW's keep theirs.
        assert!(keys("MOVEFORWARD").contains(&"W".to_string()));
        assert!(keys("JUMP").contains(&"SPACE".to_string()));
        assert_eq!(keys("ACTIONBUTTON1"), ["1"]);
        // The ordinary ones take a key only where the stock defaults leave it free.
        for (key, command) in [("J", SUMMON), ("I", "KARTCRAFT_RACEFINDER")] {
            let stock_holds = stock.iter().any(|(k, _)| k == key);
            assert_eq!(keys(command).is_empty(), stock_holds, "{command} on {key}");
        }
        for command in [
            "KARTCRAFT_LEAVERACE",
            "KARTCRAFT_SETTINGS",
            "KARTCRAFT_GARAGE",
        ] {
            assert!(keys(command).is_empty(), "{command} starts unbound");
        }
        // Every row the file declares is a command the kart names or a Lua one.
        assert_eq!(
            benilla_ui::bindings_xml::parse(xml).unwrap().len(),
            DRIVING.len() + 5
        );
    }
}
