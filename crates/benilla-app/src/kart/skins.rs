//! The skin shop's preview (World of Kartcraft): the layer's shop window (`KartcraftFrames.xml`)
//! names the picked skin's display in `KartSkinPreview` (`0` none), and its model pane
//! (`BenillaKartShopFrameModel`) shows it. The pane is a body booth (`crate::portrait`), which
//! mirrors a unit's dressed look, and nobody in the world wears the skin, so the preview stands
//! up a hidden local unit with that display, built by the world's own unit pipeline (a humanoid
//! NPC's `CreatureDisplayInfoExtra` look included), as the `fxview` fixture's unit is.
//!
//! Buying and wearing are the server's (`Kartcraft.cpp`, "Skins"): a worn skin is our native
//! display, and streams in like any other.

use bevy::prelude::*;

use crate::portrait::SkinBooth;

/// The preview unit's synthetic guid: a wire-shaped `0xF130` creature guid no server sends (entry
/// 0).
const PREVIEW_GUID: u64 = (0xF130u64 << 48) | 0x5C1E;

/// Where the preview unit stands: far below any world, out of every view, range and sound.
const PREVIEW_AT: Vec3 = Vec3::new(0.0, -20_000.0, 0.0);

/// The display the shop asks for, and the hidden unit wearing it.
#[derive(Resource, Default)]
pub(crate) struct SkinPreview {
    wanted: Option<u32>,
    shown: Option<(u32, Entity)>,
}

/// `KartSkinPreview` as it moves: the display to preview, `0` (the shop closed) none.
pub(crate) fn on_cvar(ev: On<crate::cvars::CvarChanged>, mut preview: ResMut<SkinPreview>) {
    if ev.is("KartSkinPreview") {
        preview.wanted = ev.new.trim().parse().ok().filter(|&d| d != 0);
    }
}

/// Stand the wanted skin's unit up (and the last one down), and point the shop's pane at it; the
/// pane's yaw is the feed's (`crate::ui_kart::skin_yaw`).
pub(crate) fn update(
    mut commands: Commands,
    mut preview: ResMut<SkinPreview>,
    mut booth: ResMut<SkinBooth>,
    units: Query<(), With<crate::net::NetEntity>>,
) {
    // A world change clears every unit, ours too: stand it up again.
    let alive = preview.shown.is_some_and(|(_, e)| units.contains(e));
    if alive && preview.shown.map(|(d, _)| d) == preview.wanted {
        return;
    }
    if let Some((_, old)) = preview.shown.take().filter(|_| alive) {
        commands.entity(old).despawn();
    }
    preview.shown = preview.wanted.map(|wanted| {
        let unit = commands
            .spawn((
                crate::net::Guid(PREVIEW_GUID),
                crate::net::NetEntity {
                    kind: benilla_protocol::EntityKind::Unit,
                    display_id: Some(wanted),
                    scale: 1.0,
                },
                crate::net::ObjectStore(benilla_protocol::messages::ObjectFields::from_pairs(&[
                    (22, 100), // UNIT_FIELD_HEALTH
                    (28, 100), // UNIT_FIELD_MAXHEALTH
                    (34, 60),  // UNIT_FIELD_LEVEL
                    (35, 35),  // UNIT_FIELD_FACTIONTEMPLATE: friendly, no combat pose
                ])),
                Transform::from_translation(PREVIEW_AT),
                // Hidden: the booth draws its own copies.
                Visibility::Hidden,
            ))
            .id();
        debug!("kart skins: previewing display {wanted}");
        (wanted, unit)
    });
    booth.unit = preview.shown.map(|(_, e)| e);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let mut app = App::new();
        app.init_resource::<SkinPreview>()
            .init_resource::<SkinBooth>()
            .add_systems(Update, update);
        app
    }

    fn want(app: &mut App, display: Option<u32>) {
        app.world_mut().resource_mut::<SkinPreview>().wanted = display;
        app.update();
    }

    fn units(app: &mut App) -> Vec<(Entity, Option<u32>)> {
        let mut q = app.world_mut().query::<(Entity, &crate::net::NetEntity)>();
        q.iter(app.world())
            .map(|(e, n)| (e, n.display_id))
            .collect()
    }

    #[test]
    fn one_hidden_unit_wears_the_picked_skin_and_goes_with_the_shop() {
        let mut app = app();
        want(&mut app, Some(3597));
        let first = units(&mut app);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].1, Some(3597));
        assert_eq!(app.world().resource::<SkinBooth>().unit, Some(first[0].0));

        // Another pick replaces it; the same pick again keeps it.
        want(&mut app, Some(4527));
        let second = units(&mut app);
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].1, Some(4527));
        want(&mut app, Some(4527));
        assert_eq!(units(&mut app), second);

        // A world change took it: it stands up again.
        app.world_mut().despawn(second[0].0);
        app.update();
        assert_eq!(units(&mut app).len(), 1);

        // The shop closed: no unit, an empty booth.
        want(&mut app, None);
        assert!(units(&mut app).is_empty());
        assert_eq!(app.world().resource::<SkinBooth>().unit, None);
    }
}
