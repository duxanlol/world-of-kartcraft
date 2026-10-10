//! The weapons on screen, from `kart-data/weapons.json` (written by `ctr-extract` from the
//! player's disc; `docs/ctr-assets.md`, "Weapons"), drawn unlit with the PSX blends as the kart's
//! effects draw them:
//!
//! - **Crates and fruit** where the race laid them, regrowing when they come back; the fruit
//!   turns (its 32 `spin` frames are identical on the disc; the turn is the instance's).
//! - **Shots**: the bowling bomb rolling (`dir.x += 0x200` a frame,
//!   `RB_MovingExplosive.c:204-218`), the missile's `thrust` animation facing where it flies, the
//!   warp orb's `warp` turned to the camera (its model always points north), dropped
//!   TNT, Nitro and beakers growing in over 8 frames (`RB_GenericMine.c:139-152`), a fired shield
//!   as its bubble.
//! - **Blasts**: `warpedburst`, the shockwaves and the crate debris, played once.
//! - **On a kart**: the shield (dark, then green or blue, then the turning highlight) growing and
//!   pulsing (`RB_ShieldDark_ThTick_Grow`, `231/RB_MaskShieldCloud.c:291-420`), the mask circling
//!   it with its beam (`RB_MaskWeapon_ThTick`, `:84-216`), a TNT on the head pulsing through its
//!   fuse (`231/RB_TNT.c:241-247`), a rain cloud over it.
//!
//! The shield's bubble is a half shell: it is turned to face the camera, as the player sees it.
//! The green shield's fade over its last two seconds is not drawn.

use std::collections::HashMap;
use std::sync::OnceLock;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use ctr_weapons::{Body, KartId, MineKind};

use super::{Look, Weapons};
use crate::effects::{image, unlit, Texture};
use crate::session::KartView;
use crate::{from_lu, model_scale, METERS_PER_UNIT};

/// `RB_MASK_POSITION`: the mask bobs down and up with its beam's 24 frames (`231/R231.c:82-85`).
const MASK_BOB: [i16; 24] = [
    0, 0, -2, -4, -8, -12, -16, -19, -23, -27, -29, -31, -32, -31, -29, -27, -23, -19, -16, -12,
    -8, -4, -2, 0,
];
/// The mask circles 64 units out and 64 up, turning 0x100 a frame (`RB_MaskShieldCloud.c:136-200`).
const MASK_ORBIT: f32 = 64.0;
const MASK_TURN: i32 = 0x100;
/// The shield's scale (x and z, y) growing in over 8 frames, then pulsing over 6
/// (`231/R231.c:86-118`), 0x1000 = 1.
const SHIELD_GROW: [[i16; 2]; 8] = [
    [977, 1835],
    [1792, 2936],
    [2205, 2095],
    [2335, 1254],
    [1884, 1612],
    [1433, 1971],
    [1612, 1881],
    [1792, 1792],
];
const SHIELD_PULSE: [[i16; 2]; 6] = [
    [1845, 1756],
    [1899, 1720],
    [1971, 1684],
    [1899, 1720],
    [1845, 1756],
    [1792, 1792],
];
/// The highlight turns 0x100 a frame from 0xC00 until 0x400, then hides 30 frames
/// (`RB_MaskShieldCloud.c:300-318`).
const HIGHLIGHT_START: i32 = 0xC00;
const HIGHLIGHT_SHOWN: u64 = 8;
const HIGHLIGHT_HIDDEN: u64 = 30;
/// A TNT on the head is drawn at half size, its x/z and y scale through the fuse's 90 frames
/// (`231/RB_TNT.c:70-78`, `:241-247`), 0x1000 = 1.
const TNT_SCALE: [[i16; 2]; 90] = [
    [2048, 2048],
    [2048, 2048],
    [2048, 2048],
    [2048, 2048],
    [2048, 2048],
    [2048, 2048],
    [2048, 2048],
    [2048, 2048],
    [2048, 2048],
    [2048, 2048],
    [2048, 2048],
    [2048, 2048],
    [2355, 2148],
    [2539, 2248],
    [2462, 2293],
    [2280, 2273],
    [2061, 2220],
    [1878, 2148],
    [1802, 2070],
    [1815, 1998],
    [1850, 1945],
    [1898, 1925],
    [1951, 1934],
    [1999, 1957],
    [2034, 1986],
    [2048, 2038],
    [2048, 2048],
    [2048, 2048],
    [2048, 2048],
    [2048, 2048],
    [2048, 2048],
    [2048, 2048],
    [2048, 2048],
    [2048, 2048],
    [2048, 2048],
    [2048, 2048],
    [2048, 2048],
    [2048, 2048],
    [2560, 2214],
    [2867, 2381],
    [2739, 2457],
    [2434, 2423],
    [2070, 2335],
    [1766, 2215],
    [1638, 2085],
    [1649, 1965],
    [1679, 1877],
    [1728, 1843],
    [1791, 1857],
    [1867, 1899],
    [1954, 1964],
    [2048, 2048],
    [2150, 2150],
    [2048, 2048],
    [1945, 1945],
    [2048, 2048],
    [2150, 2150],
    [2048, 2048],
    [1945, 1945],
    [2048, 2048],
    [2150, 2150],
    [2123, 2123],
    [2074, 2074],
    [2048, 2048],
    [2728, 2270],
    [3137, 2491],
    [2967, 2592],
    [2562, 2532],
    [2078, 2381],
    [1673, 2184],
    [1503, 1987],
    [1620, 1836],
    [1878, 1775],
    [2136, 2014],
    [2253, 2253],
    [2052, 2052],
    [1851, 1851],
    [2067, 2067],
    [2283, 2283],
    [1943, 1943],
    [2331, 2331],
    [1926, 1926],
    [2344, 2344],
    [1899, 1899],
    [2379, 2379],
    [1904, 1904],
    [2327, 2327],
    [1926, 1926],
    [2379, 2379],
    [3072, 3072],
];
/// A bowling bomb rolls 0x200 a frame (`RB_MovingExplosive.c:204-218`).
const BOMB_ROLL: f32 = 0x200 as f32;
/// Dropped mines grow in +0x200 a frame (`RB_GenericMine.c:139-152`).
const MINE_GROW: f32 = 0x200 as f32;
/// The fruit turns once in its 32-frame spin.
const FRUIT_TURN_FRAMES: f32 = 32.0;
/// The "?" and fruit crates at half CTR's size beside the kart (Kartcraft's own: at full size
/// they stand as tall as a WoW character).
pub(crate) const CRATE_SIZE: f32 = 0.5;
/// The rain cloud hangs this far over the kart, meters.
const CLOUD_HEIGHT: f32 = 1.9;

#[derive(serde::Deserialize)]
struct Vertex {
    position: [f32; 3],
    uv: [f32; 2],
    color: [f32; 4],
}

#[derive(serde::Deserialize)]
struct Surface {
    texture: Option<usize>,
    blend: String,
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
}

#[derive(serde::Deserialize)]
struct Frame {
    surfaces: Vec<Surface>,
}

#[derive(serde::Deserialize)]
struct Model {
    frames: Vec<Frame>,
    #[serde(default)]
    game_frames: usize,
}

#[derive(serde::Deserialize)]
struct File {
    textures: Vec<Texture>,
    models: HashMap<String, Model>,
}

fn data() -> Option<&'static File> {
    static DATA: OnceLock<Option<File>> = OnceLock::new();
    DATA.get_or_init(|| {
        let path = crate::model::data_root()?.join("weapons.json");
        let bytes = std::fs::read(&path)
            .map_err(|e| warn!("kart weapons: {}: {e}", path.display()))
            .ok()?;
        let file: File = serde_json::from_slice(&bytes)
            .map_err(|e| warn!("kart weapons: {e}"))
            .ok()?;
        info!(
            "kart weapons: {} models from {}",
            file.models.len(),
            path.display()
        );
        Some(file)
    })
    .as_ref()
}

/// Reads `weapons.json` off the disk early, so the first crate only builds meshes.
pub fn preload() {
    let _ = std::thread::Builder::new()
        .name("kart-weapons".into())
        .spawn(|| {
            data();
        });
}

/// PSX colours are display-space; the shader multiplies in linear space.
fn linear(c: [f32; 4]) -> [f32; 4] {
    [c[0].powf(2.2), c[1].powf(2.2), c[2].powf(2.2), c[3]]
}

/// The PSX subtract (`dst − texel·colour`) as black laid over the scene by `texel·colour`: RGB 0,
/// alpha the texel's strength (none where the texel is transparent). It darkens in proportion to
/// what is behind rather than by a fixed amount, and evenly across the channels.
fn shade(t: &Texture) -> Image {
    let rgba: Vec<u8> = t
        .rgba
        .chunks(4)
        .flat_map(|px| {
            let a = if px[3] == 0 {
                0
            } else {
                px[0].max(px[1]).max(px[2])
            };
            [0, 0, 0, a]
        })
        .collect();
    image(
        &Texture {
            width: t.width,
            height: t.height,
            rgba,
        },
        None,
    )
}

fn surface_mesh(s: &Surface) -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );
    let v = &s.vertices;
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        v.iter().map(|v| v.position).collect::<Vec<_>>(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; v.len()]);
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        v.iter().map(|v| v.uv).collect::<Vec<_>>(),
    );
    // Opaque surfaces as the kart's (display-space colours made linear); blended ones as the
    // kart's effects, whose additive glow was matched to CTR's with the colours as they are; a
    // subtract's colour is how much it takes away, in alpha (see [`shade`]).
    let colour = |c: [f32; 4]| match s.blend.as_str() {
        "opaque" => linear(c),
        "subtractive" => [0.0, 0.0, 0.0, c[0].max(c[1]).max(c[2]).min(1.0)],
        _ => c,
    };
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_COLOR,
        v.iter().map(|v| colour(v.color)).collect::<Vec<_>>(),
    );
    mesh.insert_indices(Indices::U32(s.indices.clone()));
    mesh
}

/// One model's frames, each its surfaces' meshes and materials.
struct Built {
    frames: Vec<Vec<(Handle<Mesh>, Handle<StandardMaterial>)>>,
    /// The most surfaces any frame has: the parts each drawn instance keeps.
    parts: usize,
    /// Game frames per stored frame (2 for an animation stored at half rate).
    step: usize,
}

fn build(
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
) -> HashMap<String, Built> {
    let Some(file) = data() else {
        return HashMap::new();
    };
    let mut plain: HashMap<usize, Handle<Image>> = HashMap::new();
    let mut shaded: HashMap<usize, Handle<Image>> = HashMap::new();
    let mut cache: HashMap<(Option<usize>, String), Handle<StandardMaterial>> = HashMap::new();
    let mut material = |s: &Surface| {
        cache
            .entry((s.texture, s.blend.clone()))
            .or_insert_with(|| {
                let texture = s.texture.and_then(|t| file.textures.get(t).map(|x| (t, x)));
                let m = match s.blend.as_str() {
                    "subtractive" => unlit(
                        texture.map(|(i, t)| {
                            shaded
                                .entry(i)
                                .or_insert_with(|| images.add(shade(t)))
                                .clone()
                        }),
                        AlphaMode::Blend,
                    ),
                    blend => {
                        let tex = texture.map(|(i, t)| {
                            plain
                                .entry(i)
                                .or_insert_with(|| images.add(image(t, None)))
                                .clone()
                        });
                        let mode = match blend {
                            "additive" | "add_quarter" => AlphaMode::Add,
                            "mix" => AlphaMode::Blend,
                            _ if tex.is_some() => AlphaMode::Mask(0.5),
                            _ => AlphaMode::Opaque,
                        };
                        // A mix blends only the texels with the PSX's semi-transparency bit,
                        // which the converter leaves at alpha 128; the rest (255) are opaque.
                        let mut m = unlit(tex, mode);
                        if blend == "add_quarter" {
                            m.base_color = Color::srgb(0.25, 0.25, 0.25);
                        }
                        m
                    }
                };
                materials.add(m)
            })
            .clone()
    };
    file.models
        .iter()
        .map(|(name, model)| {
            let frames: Vec<Vec<_>> = model
                .frames
                .iter()
                .map(|f| {
                    f.surfaces
                        .iter()
                        .map(|s| (meshes.add(surface_mesh(s)), material(s)))
                        .collect()
                })
                .collect();
            let parts = frames.iter().map(Vec::len).max().unwrap_or(0);
            let step = (model.game_frames / model.frames.len().max(1)).max(1);
            (
                name.clone(),
                Built {
                    frames,
                    parts,
                    step,
                },
            )
        })
        .collect()
}

/// Names a drawn thing across frames, so it keeps its entities.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Slot {
    what: u8,
    a: u64,
    b: u64,
    model: &'static str,
}

struct Placed {
    root: Entity,
    parts: Vec<Entity>,
}

#[derive(Resource, Default)]
pub struct WeaponLook {
    built: Option<HashMap<String, Built>>,
    placed: HashMap<Slot, Placed>,
}

#[derive(Component)]
pub struct WeaponRoot;

#[derive(Component)]
pub struct WeaponPart;

/// One thing to draw this frame: a model's game frame (looped), where.
struct Draw {
    slot: Slot,
    frame: usize,
    /// Play once: past the last frame it is not drawn.
    once: bool,
    transform: Transform,
}

/// The angle of 4096ths, radians.
fn turns(units: f32) -> f32 {
    units * std::f32::consts::TAU / 4096.0
}

/// A model's frame (`+Z` forward) facing along `dir` in the ground plane.
fn facing(dir: Vec3) -> Quat {
    Quat::from_rotation_y(dir.x.atan2(dir.z))
}

/// What a kart wears, drawn about its `at` and `rotation` (the model's frame).
#[allow(clippy::too_many_arguments)]
fn wear(
    draws: &mut Vec<Draw>,
    id: KartId,
    at: Vec3,
    rotation: Quat,
    look: &Look,
    racer: &str,
    frame: u64,
    eye: Vec3,
) {
    let size = model_scale();
    let local = |m: Vec3| at + rotation * (m * size);
    let slot = |what: u8, model: &'static str| Slot {
        what: 20 + what,
        a: u64::from(id),
        b: 0,
        model,
    };
    let to_eye = facing(eye - at);
    if let Some(shield) = look.shield {
        let age = frame.saturating_sub(look.shield_since) as usize;
        let [xz, y] = SHIELD_GROW
            .get(age)
            .copied()
            .unwrap_or(SHIELD_PULSE[frame as usize % SHIELD_PULSE.len()]);
        let scale = Vec3::new(xz.into(), y.into(), xz.into()) / 4096.0 * size;
        let layer = Transform {
            translation: at,
            rotation: to_eye,
            scale,
        };
        let colour = if shield.blue {
            "shield_blue"
        } else {
            "shield_green"
        };
        // CTR draws the dark layer, then the colour, then the highlight: each a hair nearer
        // the camera, so the transparent sort keeps that order.
        let nearer = |n: f32| (eye - at).normalize_or_zero() * (0.02 * n);
        for (what, model) in [(1, "shield_dark"), (2, colour)] {
            draws.push(Draw {
                slot: slot(what, model),
                frame: 0,
                once: false,
                transform: Transform {
                    translation: at + nearer(f32::from(what)),
                    ..layer
                },
            });
        }
        let phase = frame % (HIGHLIGHT_SHOWN + HIGHLIGHT_HIDDEN);
        if phase < HIGHLIGHT_SHOWN {
            let angle = turns((HIGHLIGHT_START + 0x100 * (phase as i32 + 1)) as f32);
            draws.push(Draw {
                slot: slot(3, "shield_highlight"),
                frame: 0,
                once: false,
                transform: Transform {
                    translation: at + nearer(3.0),
                    rotation: to_eye * Quat::from_rotation_y(angle),
                    ..layer
                },
            });
        }
    }
    if look.masked {
        let (mask, beam) = if super::good_guy(racer) {
            ("aku", "akubeam")
        } else {
            ("uka", "ukabeam")
        };
        let spin = turns(-((frame as i32 * MASK_TURN) & 0xFFF) as f32);
        let beat = frame as usize % MASK_BOB.len();
        let orbit = Vec3::new(
            spin.sin() * MASK_ORBIT,
            MASK_ORBIT + f32::from(MASK_BOB[beat]),
            spin.cos() * MASK_ORBIT,
        ) * METERS_PER_UNIT;
        let turned = rotation * Quat::from_rotation_y(spin);
        draws.push(Draw {
            slot: slot(4, mask),
            frame: (frame / 2) as usize,
            once: false,
            transform: Transform {
                translation: local(orbit),
                rotation: turned,
                scale: Vec3::splat(size),
            },
        });
        draws.push(Draw {
            slot: slot(5, beam),
            frame: beat,
            once: false,
            transform: Transform {
                translation: local(Vec3::Y * MASK_ORBIT * METERS_PER_UNIT),
                rotation: turned,
                scale: Vec3::splat(size),
            },
        });
    }
    if let Some((height, elapsed)) = look.tnt {
        let seated = height <= super::TNT_HEAD;
        let [xz, y] = if seated {
            TNT_SCALE[elapsed.clamp(0, 89) as usize]
        } else {
            [2048, 2048]
        };
        let lift = height * METERS_PER_UNIT + crate::framing().head_lift;
        draws.push(Draw {
            slot: slot(6, "tnt"),
            frame: 0,
            once: false,
            transform: Transform {
                translation: local(Vec3::Y * lift),
                rotation,
                scale: Vec3::new(xz.into(), y.into(), xz.into()) / 4096.0 * size,
            },
        });
    }
    if look.cloud {
        let lift = CLOUD_HEIGHT + crate::framing().head_lift;
        draws.push(Draw {
            slot: slot(7, "cloud"),
            frame: frame as usize,
            once: false,
            transform: Transform {
                translation: local(Vec3::Y * lift),
                rotation: to_eye,
                scale: Vec3::splat(size),
            },
        });
    }
}

/// Builds this frame's list of weapon things and brings their entities in line with it.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn draw(
    mut commands: Commands,
    weapons: Res<Weapons>,
    view: Res<KartView>,
    camera: Query<&GlobalTransform, With<benilla_world::view::WorldCamera>>,
    mut look: ResMut<WeaponLook>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut roots: Query<(&mut Transform, &mut Visibility), (With<WeaponRoot>, Without<WeaponPart>)>,
    mut parts: Query<
        (
            &mut Mesh3d,
            &mut MeshMaterial3d<StandardMaterial>,
            &mut Visibility,
        ),
        (With<WeaponPart>, Without<WeaponRoot>),
    >,
) {
    let look = &mut *look;
    let mut draws = Vec::new();
    if let (true, Some(origin)) = (view.active, weapons.origin()) {
        list(&weapons, &view, origin, &camera, &mut draws);
    }
    if draws.is_empty() && look.placed.is_empty() {
        return;
    }
    let built = look
        .built
        .get_or_insert_with(|| build(&mut meshes, &mut materials, &mut images));
    let mut used = std::collections::HashSet::new();
    for d in draws {
        let Some(model) = built.get(d.slot.model) else {
            continue;
        };
        let n = model.frames.len().max(1);
        let stored = d.frame / model.step;
        if d.once && stored >= n {
            continue;
        }
        let surfaces = &model.frames[stored % n];
        used.insert(d.slot);
        match look.placed.get(&d.slot) {
            Some(placed) => {
                if let Ok((mut t, mut vis)) = roots.get_mut(placed.root) {
                    *t = d.transform;
                    *vis = Visibility::Visible;
                }
                for (i, part) in placed.parts.iter().enumerate() {
                    let Ok((mut mesh, mut material, mut vis)) = parts.get_mut(*part) else {
                        continue;
                    };
                    match surfaces.get(i) {
                        Some((m, mat)) => {
                            if mesh.0 != *m {
                                mesh.0 = m.clone();
                            }
                            if material.0 != *mat {
                                material.0 = mat.clone();
                            }
                            vis.set_if_neq(Visibility::Inherited);
                        }
                        None => {
                            vis.set_if_neq(Visibility::Hidden);
                        }
                    }
                }
            }
            None => {
                let root = commands
                    .spawn((
                        WeaponRoot,
                        d.transform,
                        Visibility::Visible,
                        Name::new("kart weapon"),
                    ))
                    .id();
                let parts = (0..model.parts)
                    .map(|i| {
                        let (mesh, material) = surfaces
                            .get(i)
                            .or(model.frames[0].first())
                            .cloned()
                            .unwrap_or_default();
                        let shown = if i < surfaces.len() {
                            Visibility::Inherited
                        } else {
                            Visibility::Hidden
                        };
                        let part = commands
                            .spawn((
                                WeaponPart,
                                Mesh3d(mesh),
                                MeshMaterial3d(material),
                                Transform::default(),
                                shown,
                                NoFrustumCulling,
                            ))
                            .id();
                        commands.entity(root).add_child(part);
                        part
                    })
                    .collect();
                look.placed.insert(d.slot, Placed { root, parts });
            }
        }
    }
    look.placed.retain(|slot, placed| {
        let keep = used.contains(slot);
        if !keep {
            commands.entity(placed.root).despawn();
        }
        keep
    });
}

/// Everything of the weapons to draw this frame.
fn list(
    weapons: &Weapons,
    view: &KartView,
    origin: Vec3,
    camera: &Query<&GlobalTransform, With<benilla_world::view::WorldCamera>>,
    draws: &mut Vec<Draw>,
) {
    let frame = weapons.frame();
    let size = model_scale();
    let eye = camera
        .single()
        .map_or(view.transform.translation, |c| c.translation());
    let slot = |what: u8, a: u64, b: u64, model: &'static str| Slot { what, a, b, model };
    for (i, c) in weapons.crates().iter().enumerate() {
        let grown = weapons.crate_scale(c);
        if grown <= 0.0 {
            continue;
        }
        let (model, rotation, scale) = match c.kind {
            super::wire::CrateKind::Weapon => ("crate_question", Quat::IDENTITY, CRATE_SIZE),
            super::wire::CrateKind::Fruit => ("crate_fruit", Quat::IDENTITY, CRATE_SIZE),
            super::wire::CrateKind::Wumpa => (
                "fruit",
                Quat::from_rotation_y(frame as f32 / FRUIT_TURN_FRAMES * std::f32::consts::TAU),
                1.0,
            ),
        };
        draws.push(Draw {
            slot: slot(0, i as u64, 0, model),
            frame: 0,
            once: false,
            transform: Transform {
                translation: c.at,
                rotation,
                scale: Vec3::splat(size * grown * scale),
            },
        });
    }
    let blend = weapons.blend;
    for (key, body, info, was) in weapons.shots() {
        let now = body.position();
        let at = from_lu(was.lerp(now, blend), origin);
        let age = frame.saturating_sub(info.born);
        let moved = now - was;
        let heading = if Vec2::new(moved.x, moved.z).length() > 0.01 {
            facing(moved)
        } else {
            Quat::from_rotation_y(turns(info.heading as f32))
        };
        let id = (u64::from(key.owner) << 16) | u64::from(key.shot);
        let one = |what: u8, model: &'static str, frame: usize, transform: Transform| Draw {
            slot: slot(10 + what, id, 0, model),
            frame,
            once: false,
            transform,
        };
        let plain = |rotation: Quat, scale: f32| Transform {
            translation: at,
            rotation,
            scale: Vec3::splat(size * scale),
        };
        match body {
            Body::Bomb(b) if b.is_shield() => {
                let t = Transform {
                    translation: at,
                    rotation: facing(eye - at),
                    scale: Vec3::new(1792.0, 1792.0, 1792.0) / 4096.0 * size,
                };
                draws.push(one(0, "shield_dark", 0, t));
                draws.push(one(1, "shield_green", 0, t));
            }
            Body::Bomb(_) => {
                let roll = turns(BOMB_ROLL * age as f32) * if info.backward { -1.0 } else { 1.0 };
                let rotation = heading * Quat::from_rotation_x(roll);
                draws.push(one(0, "bomb", 0, plain(rotation, 1.0)));
            }
            Body::Missile(_) => {
                draws.push(one(0, "missile", age as usize, plain(heading, 1.0)));
            }
            // The orb's model always points north (its header's flag 1): it faces the camera
            // (`RB_MODEL_ALWAYS_POINT_NORTH`, `RenderBucket_QueueExecute.c:1458-1500`).
            Body::WarpOrb(_) => {
                let t = plain(facing(eye - at), 1.0);
                draws.push(one(0, "warpball", age as usize, t));
            }
            Body::Mine(m) => {
                let grow = (MINE_GROW * (age + 1) as f32 / 4096.0).min(1.0);
                let model = match m.kind() {
                    MineKind::Tnt => "tnt",
                    MineKind::Nitro => "nitro",
                    MineKind::Beaker { red: false } => "beaker_green",
                    MineKind::Beaker { red: true } => "beaker_red",
                };
                let rotation = Quat::from_rotation_y(turns(info.heading as f32));
                draws.push(one(0, model, age as usize, plain(rotation, grow)));
            }
        }
    }
    for e in weapons.effects() {
        // Bursts and shockwaves turn to face the camera (their header's billboard flag,
        // `RB_Burst.c:243-282`); debris keeps the crate's frame.
        let mut rotation = if e.model == "crate_explosion" {
            Quat::IDENTITY
        } else {
            facing(eye - e.at)
        };
        if e.turned {
            rotation *= Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        }
        draws.push(Draw {
            slot: slot(2, e.id, 0, e.model),
            frame: frame.saturating_sub(e.born) as usize,
            once: true,
            transform: Transform {
                translation: e.at,
                rotation,
                scale: Vec3::splat(size),
            },
        });
    }
    for t in weapons.thrown() {
        draws.push(Draw {
            slot: slot(3, t.id, 0, "tnt"),
            frame: 0,
            once: false,
            transform: Transform::from_translation(from_lu(t.at, origin))
                .with_scale(Vec3::splat(size * 0.5)),
        });
    }
    let ours = weapons.our_look();
    wear(
        draws,
        weapons.me,
        view.transform.translation,
        view.transform.rotation,
        &ours,
        crate::model::our_kart(),
        frame,
        eye,
    );
    for (id, other, look) in weapons.others() {
        let rotation = Quat::from_rotation_y(other.yaw + std::f32::consts::PI);
        let mut at = other.position;
        if let Some(r) = weapons.reaction(id) {
            at += rotation * r.offset;
        }
        wear(draws, id, at, rotation, &look, &other.racer, frame, eye);
    }
}
