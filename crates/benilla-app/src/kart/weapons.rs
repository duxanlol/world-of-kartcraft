//! CTR's weapons in a Kartcraft race: the client half of `docs/kartcraft-weapons-protocol.md`
//! over [`kartcraft::weapons::Weapons`]. The server's `W` lines go in as they come; the weapons
//! are bound to the race we are in (cleared when we finish, leave it or leave the kart); the
//! other racers' karts, an arena's creatures (karts our shots can hit, whose hits are decided
//! here), our race position and the HUD's item slot are handed over each frame; and the lines the
//! weapons queue (claims, shots, our hits, our shots' hits on creatures) go out at no more than 20
//! a second.
//! The 30 Hz step itself runs inside our kart's session (`super::update`).

use benilla_protocol::SessionEvent;
use bevy::prelude::*;
use kartcraft::weapons::wire::{Down, Pacer, Up, PREFIX};
use kartcraft::weapons::Weapons;

use super::profession::Profession;
use super::race::Races;
use super::remote::RemoteKarts;

/// A guid as the weapons name racers: its low 32 bits (players' high bits are 0).
pub(crate) fn low_guid(guid: u64) -> u32 {
    guid as u32
}

/// The race the weapons belong to, and our lines waiting to go.
#[derive(Resource, Default)]
pub(crate) struct WeaponLink {
    race: Option<u32>,
    pacer: Pacer,
    /// A race with no crates after GO: when we last asked for its layout again, and how often.
    asked_at: f32,
    asks: u8,
}

/// Seconds between asks for a layout that has not come, and how many before giving up.
const LAYOUT_ASK_SECS: f32 = 3.0;
const LAYOUT_ASKS: u8 = 5;

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
    races: Res<Races>,
    guid: Res<crate::net::SelfGuid>,
    names: Option<Res<crate::names::NameCache>>,
    remotes: Res<RemoteKarts>,
    index: Res<crate::net::GuidIndex>,
    units: Query<(&Transform, &crate::net::ObjectStore)>,
    mut weapons: ResMut<Weapons>,
    mut link: ResMut<WeaponLink>,
    mut extras: ResMut<kartcraft::hud::HudExtras>,
    profession: Profession,
) {
    // Not tied to the kart running: a Race Finder race's crates come while the teleport to the
    // start has the kart stopped, and rebinding when it moves again would clear them. Leaving
    // the kart still ends the race (`race.rs`), and with it the weapons.
    let race = races.weapons_race();
    if race != link.race {
        if weapons.live() {
            info!("kart weapons: race over, weapons cleared");
        }
        // A new race keeps the lines already waiting: a Race Finder race's layout comes in the
        // frame its roster binds it. A race's end drops them with the rest.
        if race.is_some() {
            weapons.rebind();
        } else {
            weapons.clear();
        }
        link.pacer.clear();
        link.race = race;
        link.asks = 0;
    }
    // A layout lost on its way (a loading screen, a relog): past GO with no crates, ask again.
    let now = time.elapsed_secs();
    if race.is_some()
        && weapons.crates().is_empty()
        && races.racing_with_crates(now)
        && link.asks < LAYOUT_ASKS
        && now - link.asked_at >= LAYOUT_ASK_SECS
    {
        link.asks += 1;
        link.asked_at = now;
        info!(
            "kart weapons: no crates after GO, asking for the layout again ({})",
            link.asks
        );
        profession.send("KARTCRAFT\tW\tL?\t0".to_string());
    }
    if race.is_none() {
        // A time trial has no weapons: nothing a stray line laid out stays drawn or pickable.
        if races.time_trial() && weapons.live() {
            weapons.clear();
        }
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
    // An arena's creatures still standing, their hits decided here (`Kartcraft.cpp` "Arena").
    let foes = races.foes();
    for &(kart, guid) in foes {
        let Some((at, store)) = index.0.get(&guid).and_then(|&e| units.get(e).ok()) else {
            continue;
        };
        if store.0.unit_health() == Some(0) {
            continue;
        }
        let yaw = at.rotation.to_euler(EulerRot::YXZ).0;
        weapons.other(kart, at.translation, yaw, 255, "", true);
    }
    weapons.end_others();
    // Ours only: no one else's hits are ours to report, and a creature's only by our shot.
    let me = weapons.me;
    for (from, up) in std::mem::take(&mut weapons.outbox) {
        if from == me {
            link.pacer.push(up.line());
        } else if let Up::Hit {
            shooter,
            shot,
            kind,
        } = up
        {
            if shooter == me && foes.iter().any(|f| f.0 == from) {
                link.pacer
                    .push(format!("{PREFIX}F\t{from}\t{shot}\t{}", kind.code()));
            }
        }
    }
    for line in link.pacer.due(time.elapsed_secs()) {
        profession.send(line);
    }
    extras.item = weapons.hud_item();
}
