//! The shop's preview of World of Kartcraft's own goods: the layer's shop window
//! (`KartcraftFrames.xml`) names the picked good's key in `KartGoodPreview` (empty none), and its
//! goods pane (`BenillaKartShopFrameGood`) shows it. A good is no WoW display, so the skin booth
//! (`kart::skins`) cannot mirror it: a custom kart stands up as itself, a flag on our own kart,
//! far below the world on a layer of its own, and a camera of its own draws it into the pane's
//! image.

use bevy::camera::visibility::RenderLayers;
use bevy::camera::PerspectiveProjection;
use bevy::prelude::*;
use kartcraft::goods::{self, KartFlag, Kind};
use kartcraft::model::{GhostKart, KartSpawner};

use crate::portrait::{PortraitImages, PortraitSource, KART_GOOD_LAYER, KART_GOOD_SLOT};

/// Where the preview kart stands: far below any world, out of every view, range and sound.
const PREVIEW_AT: Vec3 = Vec3::new(0.0, -20_000.0, 0.0);
/// The pane's image, its 230×210 aspect at twice the size.
const SIZE: (u32, u32) = (460, 420);
/// The camera's aim over the kart's origin, its distance and height, metres, and its vertical
/// field of view: a kart and a flag on it fill the pane.
const AIM: Vec3 = Vec3::new(0.0, 0.5, 0.0);
const EYE: Vec3 = Vec3::new(0.0, 1.9, 5.4);
const FOV: f32 = 0.55;

/// The good the shop asks for, the kart showing it, the booth's camera, and the pane's image.
#[derive(Resource, Default)]
pub(crate) struct GoodPreview {
    wanted: Option<String>,
    shown: Option<(String, Entity)>,
    camera: Option<Entity>,
    image: Option<Handle<Image>>,
    /// The pane's yaw, which its rotate buttons turn (fed by `crate::ui_kart::good_yaw`).
    pub(crate) yaw: f32,
    turned: f32,
}

/// `KartGoodPreview` as it moves: the good to preview, empty (the shop closed, or a skin picked)
/// none.
pub(crate) fn on_cvar(ev: On<crate::cvars::CvarChanged>, mut preview: ResMut<GoodPreview>) {
    if ev.is("KartGoodPreview") {
        let key = ev.new.trim();
        preview.wanted = goods::good(key).map(|g| g.key.to_string());
    }
}

/// The pane's image, made once at startup and named for the pane's slot.
pub(crate) fn setup(
    mut preview: ResMut<GoodPreview>,
    mut images: ResMut<Assets<Image>>,
    mut portraits: ResMut<PortraitImages>,
) {
    let image = images.add(crate::portrait::new_target_image_sized(SIZE.0, SIZE.1));
    portraits.0.insert(
        KART_GOOD_SLOT.to_string(),
        PortraitSource::Live(image.clone()),
    );
    preview.image = Some(image);
}

/// Stand the wanted good up (and the last one down) with the booth's camera, and turn it to the
/// pane's yaw. Every write is a command, so the preview, far below the world on its own layer,
/// orders against nothing that moves or draws the world's own.
pub(crate) fn update(
    // Before `commands`: a system's command buffers apply in parameter order, and the kart this
    // spawns must stand before `commands` dresses it.
    mut spawner: KartSpawner,
    mut commands: Commands,
    mut preview: ResMut<GoodPreview>,
    children: Query<&Children>,
    layered: Query<(), With<RenderLayers>>,
) {
    let layer = RenderLayers::layer(KART_GOOD_LAYER);
    let preview = &mut *preview;
    if preview.shown.as_ref().map(|(k, _)| k) != preview.wanted.as_ref() {
        if let Some((_, old)) = preview.shown.take() {
            spawner.despawn(old);
        }
        preview.shown = preview.wanted.clone().and_then(|key| {
            let good = goods::good(&key)?;
            // A kart as itself; a flag on the kart we drive.
            let (racer, flag) = match good.kind {
                Kind::Kart => (good.key, 0),
                Kind::Flag => (kartcraft::model::our_kart(), goods::flag_index(good.key)),
                // A horn is heard, not seen (the shop plays it): our kart as it is.
                Kind::Horn => (kartcraft::model::our_kart(), goods::our_flag()),
            };
            let kart = spawner.spawn(racer)?;
            commands.entity(kart).insert((
                // Silent and smokeless, as a ghost is.
                GhostKart,
                KartFlag(flag),
                Transform::from_translation(PREVIEW_AT)
                    .with_rotation(Quat::from_rotation_y(preview.yaw)),
                Visibility::Visible,
                layer.clone(),
                Name::new("kart goods preview"),
            ));
            debug!("kart goods: previewing {key}");
            Some((key, kart))
        });
        preview.turned = preview.yaw;
    }

    // The camera draws only while there is a good to show.
    match (
        preview.shown.is_some(),
        preview.camera,
        preview.image.clone(),
    ) {
        (true, None, Some(image)) => {
            let camera = commands
                .spawn((
                    crate::portrait::booth_view_shape(),
                    Camera {
                        order: -100 + KART_GOOD_LAYER as isize,
                        // Transparent: the window's art shows behind the kart.
                        clear_color: ClearColorConfig::Custom(Color::NONE),
                        ..default()
                    },
                    bevy::camera::RenderTarget::Image(image.into()),
                    Projection::from(PerspectiveProjection {
                        fov: FOV,
                        near: 0.05,
                        far: 50.0,
                        ..default()
                    }),
                    Transform::from_translation(PREVIEW_AT + EYE)
                        .looking_at(PREVIEW_AT + AIM, Vec3::Y),
                    layer.clone(),
                    Name::new("kart goods booth"),
                ))
                .id();
            preview.camera = Some(camera);
        }
        (false, Some(camera), _) => {
            commands.entity(camera).despawn();
            preview.camera = None;
        }
        _ => {}
    }

    let Some((_, kart)) = preview.shown else {
        return;
    };
    if preview.turned != preview.yaw {
        preview.turned = preview.yaw;
        commands.entity(kart).insert(
            Transform::from_translation(PREVIEW_AT)
                .with_rotation(Quat::from_rotation_y(preview.yaw)),
        );
    }
    // Every part on the booth's layer, the flag's too once it is put on.
    for part in children.iter_descendants(kart) {
        if !layered.contains(part) {
            commands.entity(part).insert(layer.clone());
        }
    }
}
