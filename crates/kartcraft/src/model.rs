//! The kart on screen: `kart-data/kart.json` (written by `ctr-extract` from the player's own CTR
//! disc; schema in `docs/ctr-assets.md`) drawn unlit, as the PSX drew it: its Gouraud colours
//! baked in, its textures point-sampled with black texels cut out. CTR draws its wheels as
//! camera-facing sprites; here each wheel is the game's side-on sprite on two discs over a dark
//! tread, the front pair steering and all four rolling. The whole kart is one entity tree under
//! [`KartRoot`], placed from [`KartView`] every frame.

use std::path::PathBuf;
use std::sync::OnceLock;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::ecs::system::SystemParam;
use bevy::image::ImageSampler;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, Face, TextureDimension, TextureFormat};

pub use ctr_assets::kart::KARTS;

use crate::session::KartView;

/// The tread band's width, meters (the game's sprite has no depth).
const WHEEL_WIDTH: f32 = 0.2;
/// Segments around a wheel.
const WHEEL_SEGMENTS: usize = 20;

#[derive(serde::Deserialize, Clone, Copy)]
pub struct Vertex {
    pub position: [f32; 3],
    #[serde(default)]
    pub normal: [f32; 3],
    #[serde(default)]
    pub uv: [f32; 2],
    #[serde(default = "white")]
    pub color: [f32; 4],
}

fn white() -> [f32; 4] {
    [1.0; 4]
}

#[derive(serde::Deserialize)]
pub struct Surface {
    pub texture: Option<usize>,
    #[serde(default)]
    pub double_sided: bool,
    #[serde(default)]
    pub blend: String,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

#[derive(serde::Deserialize)]
pub struct Texture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(serde::Deserialize)]
pub struct Wheel {
    pub center: [f32; 3],
    pub radius: f32,
    #[serde(default)]
    pub steers: bool,
}

#[derive(serde::Deserialize)]
pub struct WheelSprites {
    /// `tire00` (tread, edge-on) to `tire16` (side-on), indices into the textures.
    pub textures: Vec<usize>,
}

#[derive(serde::Deserialize, Default)]
pub struct Anchors {
    #[serde(default)]
    pub seat: [f32; 3],
    /// The exhaust pipes' tips, where the smoke comes out.
    #[serde(default)]
    pub exhaust: Vec<[f32; 3]>,
}

#[derive(serde::Deserialize)]
pub struct Model {
    pub kart: String,
    #[serde(default)]
    pub anchors: Anchors,
    #[serde(default)]
    pub wheels: Vec<Wheel>,
    pub wheel_sprites: Option<WheelSprites>,
    pub surfaces: Vec<Surface>,
    pub textures: Vec<Texture>,
}

/// `WOW_KART_DATA`, else a `kart-data` beside or above the working directory.
pub fn data_root() -> Option<PathBuf> {
    std::env::var_os("WOW_KART_DATA")
        .map(PathBuf::from)
        .or_else(|| {
            ["kart-data", "../kart-data"]
                .into_iter()
                .map(PathBuf::from)
                .find(|p| p.join("kart.json").is_file())
        })
}

/// The racer picked with [`set_our_kart`], over [`our_kart`]'s default.
static CHOSEN: std::sync::RwLock<Option<&'static str>> = std::sync::RwLock::new(None);

/// Our kart's racer: the one picked with [`set_our_kart`], else `WOW_KART` (crash, cortex, coco,
/// tiny, polar, pura, pinstripe, roo, joe, pen, fake), else whichever `kart.json` holds.
pub fn our_kart() -> &'static str {
    if let Some(racer) = *CHOSEN
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
    {
        return racer;
    }
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(|| {
        std::env::var("WOW_KART")
            .ok()
            .filter(|k| model_named(k).is_some())
            .or_else(|| model_named("").map(|m| m.kart.clone()))
            .unwrap_or_default()
    })
}

/// Picks our kart's racer; `false`, and no change, for one the kart data has no kart for. Our
/// kart on screen follows at its next frame ([`update`]), and so does the racer we relay.
pub fn set_our_kart(racer: &str) -> bool {
    let Some(model) = model_named(racer).filter(|_| !racer.is_empty()) else {
        return false;
    };
    *CHOSEN
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(model.kart.as_str());
    true
}

/// The racers whose karts the kart data holds (`kart-<name>.json`), in `ctr-extract`'s order.
pub fn racers() -> Vec<&'static str> {
    let Some(root) = data_root() else {
        return Vec::new();
    };
    KARTS
        .into_iter()
        .filter(|k| root.join(format!("kart-{k}.json")).is_file())
        .collect()
}

/// A racer's kart (`kart-<name>.json`; `""` for `kart.json`), read once and kept.
pub fn model_named(name: &str) -> Option<&'static Model> {
    static MODELS: OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, Option<&'static Model>>>,
    > = OnceLock::new();
    let name = name.to_ascii_lowercase();
    let models = MODELS.get_or_init(Default::default);
    if let Some(m) = models.lock().unwrap().get(&name) {
        return *m;
    }
    let loaded = (|| {
        let root = data_root()?;
        let file = if name.is_empty() {
            root.join("kart.json")
        } else {
            root.join(format!("kart-{name}.json"))
        };
        let data = std::fs::read(&file).ok()?;
        let model: Model = serde_json::from_slice(&data)
            .map_err(|e| warn!("kart: {}: {e}", file.display()))
            .ok()?;
        info!("kart: {}'s kart from {}", model.kart, file.display());
        Some(&*Box::leak(Box::new(model)))
    })();
    models.lock().unwrap().insert(name, loaded);
    loaded
}

/// Our kart's model.
pub fn model() -> Option<&'static Model> {
    model_named(our_kart()).or_else(|| model_named(""))
}

/// Where a seated driver's hips go, in the kart's frame, meters (the median of CTR's drivers).
pub fn seat() -> Vec3 {
    model().map_or(Vec3::new(0.0, 0.254, -0.029), |m| {
        Vec3::from_array(m.anchors.seat)
    })
}

/// Reads the model off the disk early, so the first `J` only spawns it.
pub fn preload() {
    let _ = std::thread::Builder::new()
        .name("kart-model".into())
        .spawn(|| {
            if model().is_none() {
                warn!("kart: no kart-data/kart.json (run setup, or set WOW_KART_DATA); the kart is invisible");
            }
        });
}

/// PSX colours are display-space; the shader multiplies in linear space.
fn linear(c: [f32; 4]) -> [f32; 4] {
    [c[0].powf(2.2), c[1].powf(2.2), c[2].powf(2.2), c[3]]
}

fn image(t: &Texture) -> Image {
    let mut image = Image::new(
        Extent3d {
            width: t.width,
            height: t.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        t.rgba.clone(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::nearest();
    image
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
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_NORMAL,
        v.iter().map(|v| v.normal).collect::<Vec<_>>(),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        v.iter().map(|v| v.uv).collect::<Vec<_>>(),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_COLOR,
        v.iter().map(|v| linear(v.color)).collect::<Vec<_>>(),
    );
    mesh.insert_indices(Indices::U32(s.indices.clone()));
    mesh
}

/// A wheel's two side discs (the side-on sprite, both faces), about +X, radius 1, meters.
fn disc_mesh() -> Mesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();
    for side in [-1.0f32, 1.0] {
        let x = side * WHEEL_WIDTH * 0.5;
        let base = positions.len() as u32;
        positions.push([x, 0.0, 0.0]);
        normals.push([side, 0.0, 0.0]);
        uvs.push([0.5, 0.5]);
        for i in 0..=WHEEL_SEGMENTS {
            let a = i as f32 / WHEEL_SEGMENTS as f32 * std::f32::consts::TAU;
            positions.push([x, a.sin(), a.cos()]);
            normals.push([side, 0.0, 0.0]);
            uvs.push([0.5 + 0.5 * a.cos(), 0.5 - 0.5 * a.sin()]);
        }
        for i in 0..WHEEL_SEGMENTS as u32 {
            let (a, b) = (base + 1 + i, base + 2 + i);
            // Wound to face out on each side.
            if side > 0.0 {
                indices.extend([base, a, b]);
            } else {
                indices.extend([base, b, a]);
            }
        }
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// A wheel's tread: an open cylinder about +X, radius 0.96 (inside the sprite's rim), meters.
fn tread_mesh() -> Mesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();
    let r = 0.96;
    for i in 0..=WHEEL_SEGMENTS {
        let a = i as f32 / WHEEL_SEGMENTS as f32 * std::f32::consts::TAU;
        let n = [0.0, a.sin(), a.cos()];
        for side in [-1.0f32, 1.0] {
            positions.push([side * WHEEL_WIDTH * 0.5, r * n[1], r * n[2]]);
            normals.push(n);
            uvs.push([(side + 1.0) * 0.5, i as f32 / WHEEL_SEGMENTS as f32 * 4.0]);
        }
    }
    for i in 0..WHEEL_SEGMENTS as u32 {
        let (a, b, c, d) = (i * 2, i * 2 + 1, i * 2 + 2, i * 2 + 3);
        indices.extend([a, c, b, b, c, d]);
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// Any kart's root entity: ours, or another player's.
#[derive(Component)]
pub struct KartRoot;

/// Ours, placed from [`KartView`].
#[derive(Component)]
pub struct LocalKart;

/// A kart's moving parts this frame: front-wheel steer (−1 right to 1 left) and the wheels' roll
/// (radians).
#[derive(Component, Default, Clone, Copy, Debug, PartialEq)]
pub struct KartWheels {
    pub steer: f32,
    pub spin: f32,
}

/// What a kart is doing, for its effects and sounds: the turbo flame's size (0 none, else 4 to
/// 8), the powerslide and its boost window (the meter's red), an early boost's puff, the ground,
/// the engine's (distortion, volume) and its speed (yd/s).
#[derive(Component, Default, Clone, Copy, Debug, PartialEq)]
pub struct KartStatus {
    pub fire: u8,
    pub drifting: bool,
    pub boost_window: bool,
    pub failed_boost: bool,
    pub grounded: bool,
    pub engine: (u8, u8),
    pub speed: f32,
}

/// A wheel's steering pivot and its rolling hub, each knowing its kart.
#[derive(Component)]
pub struct WheelPivot {
    steers: bool,
    root: Entity,
}

#[derive(Component)]
pub struct WheelHub {
    root: Entity,
}

/// The kart's meshes and materials, built once and shared by every kart drawn.
struct KartAssets {
    parts: Vec<(Handle<Mesh>, Handle<StandardMaterial>)>,
    disc: Handle<Mesh>,
    tread: Handle<Mesh>,
    disc_material: Handle<StandardMaterial>,
    tread_material: Handle<StandardMaterial>,
}

#[derive(Resource, Default)]
pub struct KartModel {
    /// By racer.
    assets: std::collections::HashMap<String, KartAssets>,
    local: Option<Entity>,
    /// Whose kart `local` is.
    local_racer: &'static str,
}

fn build_assets(
    model: &Model,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
) -> KartAssets {
    let textures: Vec<Handle<Image>> = model
        .textures
        .iter()
        .map(|t| images.add(image(t)))
        .collect();
    let parts = model
        .surfaces
        .iter()
        .map(|s| {
            let material = materials.add(StandardMaterial {
                base_color_texture: s.texture.and_then(|t| textures.get(t).cloned()),
                unlit: true,
                alpha_mode: if s.blend == "blend" {
                    AlphaMode::Blend
                } else {
                    AlphaMode::Mask(0.5)
                },
                cull_mode: if s.double_sided {
                    None
                } else {
                    Some(Face::Back)
                },
                double_sided: s.double_sided,
                ..default()
            });
            (meshes.add(surface_mesh(s)), material)
        })
        .collect();
    let sprites = model.wheel_sprites.as_ref().map(|w| w.textures.as_slice());
    let sprite = |i: Option<&usize>| i.and_then(|&t| textures.get(t).cloned());
    let disc_material = materials.add(StandardMaterial {
        base_color_texture: sprite(sprites.and_then(|t| t.last())),
        base_color: if sprites.is_some() {
            Color::WHITE
        } else {
            Color::srgb(0.12, 0.12, 0.12)
        },
        unlit: true,
        alpha_mode: AlphaMode::Mask(0.5),
        cull_mode: None,
        double_sided: true,
        ..default()
    });
    let tread_material = materials.add(StandardMaterial {
        base_color_texture: sprite(sprites.and_then(|t| t.first())),
        base_color: Color::srgb(0.6, 0.6, 0.6),
        unlit: true,
        cull_mode: None,
        double_sided: true,
        ..default()
    });
    KartAssets {
        parts,
        disc: meshes.add(disc_mesh()),
        tread: meshes.add(tread_mesh()),
        disc_material,
        tread_material,
    }
}

/// Spawns karts: ours, and one per other driver.
#[derive(SystemParam)]
pub struct KartSpawner<'w, 's> {
    commands: Commands<'w, 's>,
    model: ResMut<'w, KartModel>,
    meshes: ResMut<'w, Assets<Mesh>>,
    materials: ResMut<'w, Assets<StandardMaterial>>,
    images: ResMut<'w, Assets<Image>>,
}

impl KartSpawner<'_, '_> {
    /// A new kart in `racer`'s colours (ours if unknown), hidden at the origin; `None` without
    /// any `kart.json`.
    pub fn spawn(&mut self, racer: &str) -> Option<Entity> {
        let model = model_named(racer).or_else(model)?;
        if !self.model.assets.contains_key(&model.kart) {
            let built = build_assets(
                model,
                &mut self.meshes,
                &mut self.materials,
                &mut self.images,
            );
            self.model.assets.insert(model.kart.clone(), built);
        }
        let assets = self.model.assets.get(&model.kart)?;
        let commands = &mut self.commands;
        let root = commands
            .spawn((
                KartRoot,
                KartWheels::default(),
                KartStatus::default(),
                Transform::default(),
                Visibility::Hidden,
                Name::new("kart"),
            ))
            .id();
        for (mesh, material) in &assets.parts {
            let part = commands
                .spawn((
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material.clone()),
                    Transform::default(),
                    NoFrustumCulling,
                ))
                .id();
            commands.entity(root).add_child(part);
        }
        for w in &model.wheels {
            let pivot = commands
                .spawn((
                    WheelPivot {
                        steers: w.steers,
                        root,
                    },
                    Transform::from_translation(Vec3::from_array(w.center)),
                    Visibility::Inherited,
                ))
                .id();
            let hub = commands
                .spawn((
                    WheelHub { root },
                    Transform::from_scale(Vec3::new(1.0, w.radius, w.radius)),
                    Visibility::Inherited,
                ))
                .id();
            let disc = commands
                .spawn((
                    Mesh3d(assets.disc.clone()),
                    MeshMaterial3d(assets.disc_material.clone()),
                    Transform::default(),
                    NoFrustumCulling,
                ))
                .id();
            let band = commands
                .spawn((
                    Mesh3d(assets.tread.clone()),
                    MeshMaterial3d(assets.tread_material.clone()),
                    Transform::default(),
                    NoFrustumCulling,
                ))
                .id();
            commands.entity(hub).add_children(&[disc, band]);
            commands.entity(pivot).add_child(hub);
            commands.entity(root).add_child(pivot);
        }
        Some(root)
    }

    /// Despawns a kart spawned by [`KartSpawner::spawn`].
    pub fn despawn(&mut self, kart: Entity) {
        self.commands.entity(kart).despawn();
    }

    /// Our own kart, spawned the first time it is asked for and again when another racer's is
    /// picked.
    fn local(&mut self) -> Option<Entity> {
        let racer = our_kart();
        if let Some(e) = self.model.local {
            if self.model.local_racer == racer {
                return Some(e);
            }
            self.despawn(e);
            self.model.local = None;
        }
        let e = self.spawn(racer)?;
        self.commands.entity(e).insert(LocalKart);
        self.model.local = Some(e);
        self.model.local_racer = racer;
        Some(e)
    }
}

/// Places our kart from this frame's [`KartView`], or hides it.
pub fn update(
    view: Res<KartView>,
    mut spawner: KartSpawner,
    mut root: Query<
        (
            &mut Transform,
            &mut Visibility,
            &mut KartWheels,
            &mut KartStatus,
        ),
        With<LocalKart>,
    >,
) {
    if !view.active && spawner.model.local.is_none() {
        return;
    }
    if spawner.local().is_none() {
        return;
    }
    let Ok((mut t, mut visibility, mut wheels, mut status)) = root.single_mut() else {
        return;
    };
    if !view.active {
        *visibility = Visibility::Hidden;
        return;
    }
    *t = view.transform;
    *visibility = Visibility::Visible;
    *wheels = KartWheels {
        steer: view.steer,
        spin: view.wheel_spin,
    };
    *status = KartStatus {
        fire: view.fire,
        drifting: view.state == crate::State::Drifting,
        boost_window: boost_window(view.meter),
        failed_boost: view.failed_boost,
        grounded: view.grounded,
        engine: view.engine,
        speed: view.speed,
    };
}

/// CTR's "boost now" cue: the drift meter between 129 and `(TurboLowRoomWarning + 2)·32` CTR-ms
/// from full (`VehEmitter_Exhaust`), when the exhaust smokes dark.
pub fn boost_window(meter: Option<(f32, bool)>) -> bool {
    meter.is_some_and(|(filled, _)| {
        let room = (1.0 - filled) * 960.0;
        (129.0..=544.0).contains(&room)
    })
}

/// Turns every kart's front wheels and rolls all of them.
pub fn animate_wheels(
    karts: Query<&KartWheels>,
    mut pivots: Query<(&WheelPivot, &mut Transform)>,
    mut hubs: Query<(&WheelHub, &mut Transform), Without<WheelPivot>>,
) {
    for (pivot, mut t) in &mut pivots {
        let Ok(w) = karts.get(pivot.root) else {
            continue;
        };
        // CTR's front wheels turn up to about 20° with the steer.
        t.rotation = if pivot.steers {
            Quat::from_rotation_y(w.steer * 0.35)
        } else {
            Quat::IDENTITY
        };
    }
    for (hub, mut t) in &mut hubs {
        if let Ok(w) = karts.get(hub.root) {
            t.rotation = Quat::from_rotation_x(w.spin);
        }
    }
}
