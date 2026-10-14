//! Which controller drives, when several are connected (`KartPad`, the Kartcraft settings
//! window's Controller dropdown): any (the setting empty), or the one named by its key
//! ([`kartcraft::input::pad_key`]). Every other connected controller is marked
//! [`PadIgnored`], and every reader of the pads (the kart, the bindings, the interface's pad feed,
//! the rumble, the controller test) leaves it out. A chosen controller not connected leaves them
//! all read, so a pad swapped for another still drives. The connected ones are listed for the
//! dropdown in `KartPads`, `key|name` joined by `;`, rewritten as they come and go.

use bevy::input::gamepad::Gamepad;
use bevy::prelude::*;
use kartcraft::input::{pad_key, PadIgnored};

/// `KartPad`: the chosen controller's key, empty for any.
#[derive(Resource, Default)]
pub(crate) struct PadChoice {
    key: String,
    /// The `KartPads` listing last written.
    listed: Option<String>,
}

pub(crate) fn on_cvar(ev: On<crate::cvars::CvarChanged>, mut choice: ResMut<PadChoice>) {
    if ev.is("KartPad") {
        choice.key = ev.new.trim().to_string();
    }
}

/// The connected controllers as `(key, name)`, in connection order; a nameless one is called by
/// its ids.
fn connected<'a>(
    pads: impl Iterator<Item = (&'a Gamepad, Option<&'a Name>)>,
) -> Vec<(String, String)> {
    pads.map(|(pad, name)| {
        let name = name.map(|n| n.as_str()).unwrap_or("").trim();
        let key = pad_key(name, pad.vendor_id(), pad.product_id());
        let label = if name.is_empty() {
            format!("Controller {}", &key[..9])
        } else {
            name.replace([';', '|'], " ")
        };
        (key, label)
    })
    .collect()
}

/// Whether a controller with `key` is read: the choice is empty, it is the one chosen, or the one
/// chosen is not among the `connected`.
fn read(choice: &str, key: &str, connected: &[(String, String)]) -> bool {
    choice.is_empty() || key == choice || !connected.iter().any(|(k, _)| k == choice)
}

/// Marks the controllers not chosen, and lists the connected ones for the dropdown.
pub(crate) fn update(
    mut commands: Commands,
    mut choice: ResMut<PadChoice>,
    pads: Query<(Entity, &Gamepad, Option<&Name>, Has<PadIgnored>)>,
    mut cvars: ResMut<crate::cvars::Cvars>,
) {
    let list = connected(pads.iter().map(|(_, p, n, _)| (p, n)));
    for ((entity, _, _, ignored), (key, _)) in pads.iter().zip(&list) {
        let wanted = read(&choice.key, key, &list);
        if wanted && ignored {
            commands.entity(entity).remove::<PadIgnored>();
        } else if !wanted && !ignored {
            commands.entity(entity).insert(PadIgnored);
        }
    }
    let listing = list
        .iter()
        .map(|(k, n)| format!("{k}|{n}"))
        .collect::<Vec<_>>()
        .join(";");
    if choice.listed.as_deref() != Some(listing.as_str()) {
        cvars.set("KartPads", &listing);
        choice.listed = Some(listing);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pads() -> Vec<(String, String)> {
        vec![
            ("0000:0000:".into(), "Controller 0000:0000".into()),
            ("054c:0ce6:PS5 Controller".into(), "PS5 Controller".into()),
        ]
    }

    #[test]
    fn only_the_chosen_controller_is_read_and_any_when_none_or_gone() {
        let list = pads();
        let ps5 = "054c:0ce6:PS5 Controller";
        // Any: both.
        assert!(read("", "0000:0000:", &list) && read("", ps5, &list));
        // The PS5 chosen: the phantom is left out.
        assert!(read(ps5, ps5, &list));
        assert!(!read(ps5, "0000:0000:", &list));
        // A chosen pad not connected: every one is read, so another pad still drives.
        assert!(read("045e:02ea:Xbox Controller", ps5, &list));
    }

    #[test]
    fn a_key_names_the_ids_and_the_name() {
        assert_eq!(
            pad_key("PS5 Controller", Some(0x054c), Some(0x0ce6)),
            "054c:0ce6:PS5 Controller"
        );
        assert_eq!(pad_key(" a;b|c ", None, None), "0000:0000:a b c");
    }
}
