//! World of Kartcraft's own cosmetics (`cosmetics/` at the repository's root): karts and flags
//! built into the client, so every player has every one and sees the ones others wear, unlike
//! the CTR karts converted from each player's own disc. A Kartcraft server sells them for
//! tokens (`Kartcraft.cpp` "Goods", whose catalogue a test holds to this one) and says what each
//! racer owns and wears. A kart is a racer key ([`crate::model::model_named`] finds it here); a
//! flag is drawn on its pole at the back of whatever kart its driver drives, and travels on the
//! wire by its index in [`FLAGS`].

use std::io::Read;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

use bevy::prelude::*;

use crate::model::{KartRoot, Surface, Texture};

/// What a good is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Kart,
    Flag,
}

/// One cosmetic: its key (the server's, and a kart's racer key), what it is, its name, and its
/// gzipped JSON.
pub struct Good {
    pub key: &'static str,
    pub kind: Kind,
    pub name: &'static str,
    gz: &'static [u8],
}

pub const GOODS: [Good; 4] = [
    Good {
        key: "student",
        kind: Kind::Flag,
        name: "Student Flag",
        gz: include_bytes!("../../../cosmetics/flag-student.json.gz"),
    },
    Good {
        key: "fel",
        kind: Kind::Kart,
        name: "Fel Kart",
        gz: include_bytes!("../../../cosmetics/kart-fel.json.gz"),
    },
    Good {
        key: "alliance",
        kind: Kind::Kart,
        name: "Alliance Kart",
        gz: include_bytes!("../../../cosmetics/kart-alliance.json.gz"),
    },
    Good {
        key: "wok",
        kind: Kind::Kart,
        name: "WoK Kart",
        gz: include_bytes!("../../../cosmetics/kart-wok.json.gz"),
    },
];

/// The flags by wire index: index `i + 1` is `FLAGS[i]`, 0 none.
pub const FLAGS: [&str; 1] = ["student"];

/// The good with `key` (ASCII case aside).
pub fn good(key: &str) -> Option<&'static Good> {
    GOODS.iter().find(|g| g.key.eq_ignore_ascii_case(key))
}

/// The custom karts' keys, in the catalogue's order.
pub fn karts() -> impl Iterator<Item = &'static Good> {
    GOODS.iter().filter(|g| g.kind == Kind::Kart)
}

/// A flag's wire index by key; 0 for none or an unknown one.
pub fn flag_index(key: &str) -> u8 {
    FLAGS
        .iter()
        .position(|f| f.eq_ignore_ascii_case(key))
        .map_or(0, |i| u8::try_from(i + 1).unwrap_or(0))
}

/// A good's JSON, unpacked.
pub(crate) fn json(good: &Good) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(good.gz)
        .read_to_end(&mut out)
        .map_err(|e| warn!("kart goods: {}: {e}", good.key))
        .ok()?;
    Some(out)
}

/// A flag's model: its textures and surfaces, meters, Y up, the pole's foot at the origin and the
/// cloth along +X.
#[derive(serde::Deserialize)]
pub struct FlagModel {
    pub textures: Vec<Texture>,
    pub surfaces: Vec<Surface>,
}

/// The flag with wire index `index`, read once and kept.
pub fn flag(index: u8) -> Option<&'static FlagModel> {
    static FLAG_MODELS: OnceLock<Vec<Option<FlagModel>>> = OnceLock::new();
    let models = FLAG_MODELS.get_or_init(|| {
        FLAGS
            .iter()
            .map(|key| {
                let bytes = json(good(key)?)?;
                serde_json::from_slice(&bytes)
                    .map_err(|e| warn!("kart goods: flag {key}: {e}"))
                    .ok()
            })
            .collect()
    });
    models.get(usize::from(index).checked_sub(1)?)?.as_ref()
}

static OUR_FLAG: AtomicU8 = AtomicU8::new(0);

/// The flag we wear, as the server last said (`KartFlag`, a key; "" none).
pub fn set_our_flag(key: &str) {
    OUR_FLAG.store(flag_index(key), Ordering::Relaxed);
}

/// Our flag's wire index, 0 none.
pub fn our_flag() -> u8 {
    OUR_FLAG.load(Ordering::Relaxed)
}

/// Where a flag stands on a kart, in the kart's frame (meters; +X left, +Z forward): its pole's
/// foot on the kart's left side beside the seat, so the cloth streams back over the side above
/// the driver's head and ends ahead of a rear wing (the Fel kart's stands 0.8 m back).
const MOUNT: Vec3 = Vec3::new(0.45, 0.42, 0.05);
/// The flag's size on a kart: a 2.95 m pole made a metre, its cloth 0.66 m by 0.34.
const SCALE: f32 = 0.34;

/// The flag a kart flies (its wire index, 0 none): set by the host on our kart and each other
/// driver's, as [`crate::model::KartStatus`] is.
#[derive(Component, Clone, Copy, Default, PartialEq, Eq)]
pub struct KartFlag(pub u8);

/// The flag drawn on a kart, and which.
#[derive(Component)]
pub struct FlagShown {
    index: u8,
    entity: Entity,
}

/// One flag's parts: each surface's mesh and material.
type FlagParts = Vec<(Handle<Mesh>, Handle<StandardMaterial>)>;

/// Each flag's meshes and materials, built once.
#[derive(Resource, Default)]
pub struct FlagAssets(Vec<Option<FlagParts>>);

/// Puts each kart's flag on it, and takes a changed one down.
#[allow(clippy::type_complexity)]
pub fn attach(
    mut commands: Commands,
    mut assets: ResMut<FlagAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    karts: Query<(Entity, &KartFlag, Option<&FlagShown>), (With<KartRoot>, Changed<KartFlag>)>,
) {
    for (kart, &KartFlag(index), shown) in &karts {
        if shown.is_some_and(|s| s.index == index) {
            continue;
        }
        if let Some(old) = shown {
            commands.entity(old.entity).despawn();
            commands.entity(kart).remove::<FlagShown>();
        }
        let Some(model) = flag(index) else {
            continue;
        };
        let slot = usize::from(index - 1);
        if assets.0.len() <= slot {
            assets.0.resize_with(slot + 1, || None);
        }
        let parts = assets.0[slot].get_or_insert_with(|| {
            crate::model::build_parts(
                &model.textures,
                &model.surfaces,
                None,
                &mut meshes,
                &mut materials,
                &mut images,
            )
        });
        // The cloth (+X) streams back over the kart's tail (−Z).
        let pole = commands
            .spawn((
                Transform::from_translation(MOUNT)
                    .with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2))
                    .with_scale(Vec3::splat(SCALE)),
                Visibility::Inherited,
                Name::new("kart flag"),
            ))
            .id();
        for (mesh, material) in parts.iter() {
            let part = commands
                .spawn((
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material.clone()),
                    Transform::default(),
                    bevy::camera::visibility::NoFrustumCulling,
                ))
                .id();
            commands.entity(pole).add_child(part);
        }
        commands.entity(kart).add_child(pole);
        commands.entity(kart).insert(FlagShown {
            index,
            entity: pole,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_good_unpacks_and_reads() {
        for g in &GOODS {
            let bytes = json(g).unwrap_or_else(|| panic!("{} unpacks", g.key));
            match g.kind {
                Kind::Kart => {
                    let m: crate::model::Model = serde_json::from_slice(&bytes).expect(g.key);
                    assert_eq!(m.kart, g.key, "the kart's own key");
                    assert!(!m.surfaces.is_empty() && m.wheels.len() == 4, "{}", g.key);
                    // The seat and the steering wheel where CTR's are, for the driver.
                    assert!(m.anchors.steering_wheel.is_some(), "{}", g.key);
                }
                Kind::Flag => {
                    let f: FlagModel = serde_json::from_slice(&bytes).expect(g.key);
                    assert!(!f.surfaces.is_empty(), "{}", g.key);
                    for t in &f.textures {
                        assert_eq!(t.rgba.len(), (t.width * t.height * 4) as usize, "{}", g.key);
                    }
                }
            }
        }
    }

    #[test]
    fn flags_go_by_their_index_and_none_is_zero() {
        assert_eq!(flag_index("student"), 1);
        assert_eq!(flag_index(""), 0);
        assert_eq!(flag_index("nope"), 0);
        assert!(flag(1).is_some() && flag(0).is_none() && flag(9).is_none());
        for f in FLAGS {
            assert_eq!(good(f).map(|g| g.kind), Some(Kind::Flag));
        }
    }
}
