//! CTR's weapons in a Kartcraft race: the client half of `docs/kartcraft-weapons-protocol.md`
//! over [`kartcraft::weapons::Weapons`]. The server's `W` lines go in as they come; the weapons
//! are bound to the race we are in (cleared when we finish, leave it or leave the kart); the
//! other racers' karts, our race position and the HUD's item slot are handed over each frame;
//! and the lines the weapons queue (claims, shots, our hits) go out at no more than 20 a second.
//! The 30 Hz step itself runs inside our kart's session (`super::update`).

use benilla_protocol::SessionEvent;
use bevy::prelude::*;
use kartcraft::weapons::wire::{Down, Pacer, PREFIX};
use kartcraft::weapons::Weapons;

use super::profession::Profession;
use super::race::Races;
use super::remote::RemoteKarts;
use super::KartDrive;

/// A guid as the weapons name racers: its low 32 bits (players' high bits are 0).
pub(crate) fn low_guid(guid: u64) -> u32 {
    guid as u32
}

/// The race the weapons belong to, and our lines waiting to go.
#[derive(Resource, Default)]
pub(crate) struct WeaponLink {
    race: Option<u32>,
    pacer: Pacer,
}

/// The server's weapon lines.
pub(crate) fn on_chat(In(ev): In<SessionEvent>, mut weapons: ResMut<Weapons>) {
    let SessionEvent::Chat(m) = ev else {
        return;
    };
    if !m.is_addon() || !m.text.starts_with(PREFIX) {
        return;
    }
    match Down::parse(&m.text) {
        Some(down) => weapons.receive(down),
        None => debug!("kart weapons: unread line {:?}", m.text),
    }
}

/// After the race's update: the weapons' race, who we and the others are, our lines out, the
/// item slot.
#[allow(clippy::too_many_arguments)]
pub(crate) fn update(
    time: Res<Time>,
    drive: Res<KartDrive>,
    races: Res<Races>,
    guid: Res<crate::net::SelfGuid>,
    names: Option<Res<crate::names::NameCache>>,
    remotes: Res<RemoteKarts>,
    mut weapons: ResMut<Weapons>,
    mut link: ResMut<WeaponLink>,
    mut extras: ResMut<kartcraft::hud::HudExtras>,
    profession: Profession,
) {
    let race = races.weapons_race().filter(|_| drive.active);
    if race != link.race {
        if weapons.live() {
            info!("kart weapons: race over, weapons cleared");
        }
        weapons.clear();
        link.pacer.clear();
        link.race = race;
    }
    if race.is_none() {
        extras.item = None;
        return;
    }
    if let Some(g) = guid.0 {
        weapons.me = low_guid(g);
    }
    weapons.rank = extras.place.map_or(0, |p| p.saturating_sub(1));
    // The other racers' karts (drivers outside the race are not in it), by the standings.
    let order = races.order();
    weapons.begin_others();
    for remote in remotes.karts.values() {
        let name = names.as_ref().and_then(|n| n.peek(remote.guid));
        let Some(rank) = name.and_then(|n| order.iter().position(|o| o == n)) else {
            continue;
        };
        weapons.other(
            low_guid(remote.guid),
            remote.position,
            remote.yaw,
            rank.min(255) as u8,
            remotes.racer(remote.guid),
            false,
        );
    }
    weapons.end_others();
    // Ours only: no one else's hits are ours to report.
    let me = weapons.me;
    for (from, up) in std::mem::take(&mut weapons.outbox) {
        if from == me {
            link.pacer.push(up.line());
        }
    }
    for line in link.pacer.due(time.elapsed_secs()) {
        profession.send(line);
    }
    extras.item = weapons.hud_item();
}
