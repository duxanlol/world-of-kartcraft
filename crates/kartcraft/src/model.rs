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

use crate::cosmetic::{repaint, Paint};
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
    /// The centre of the steering wheel's ring.
    #[serde(default)]
    pub steering_wheel: Option<[f32; 3]>,
    /// The exhaust pipes' tips, where the smoke comes out.
    #[serde(default)]
    pub exhaust: Vec<[f32; 3]>,
    /// Where its turbo flames burn, each pointing back (−Z); empty for CTR's own pair
    /// (`effects.json`), which every CTR kart shares. A kart whose nozzles stand elsewhere (a
    /// rocket car's) names its own.
    #[serde(default)]
    pub turbo: Vec<[f32; 3]>,
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
    /// Its textures filtered, not point-sampled: WoW's art, made for bilinear, where the PSX's is
    /// sampled as the PSX did.
    #[serde(default)]
    pub smooth: bool,
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
/// A racer picked before its kart exists (one built from the install, [`provide`]d at startup,
/// after the settings are read): taken once it is.
static WANTED: std::sync::RwLock<Option<String>> = std::sync::RwLock::new(None);

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
        *WANTED
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(racer.to_ascii_lowercase());
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
        .chain(
            crate::goods::karts()
                .map(|g| g.key)
                .filter(|k| model_named(k).is_some()),
        )
        .collect()
}

/// Every model read so far by key, `None` for one there is none for.
type Models = std::sync::Mutex<std::collections::HashMap<String, Option<&'static Model>>>;

fn models() -> &'static Models {
    static MODELS: OnceLock<Models> = OnceLock::new();
    MODELS.get_or_init(Default::default)
}

/// A kart built from the player's own install (a good with no shipped model,
/// [`crate::goods::Good`]), handed over to be found by its key like any other.
pub fn provide(model: Model) {
    let key = model.kart.to_ascii_lowercase();
    info!("kart: {}'s kart, from the install", model.kart);
    models()
        .lock()
        .unwrap()
        .insert(key.clone(), Some(&*Box::leak(Box::new(model))));
    let wanted = WANTED
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    if wanted.as_deref() == Some(key.as_str()) {
        set_our_kart(&key);
    }
}

/// A racer's kart (`kart-<name>.json`; `""` for `kart.json`), read once and kept.
pub fn model_named(name: &str) -> Option<&'static Model> {
    let name = name.to_ascii_lowercase();
    let models = models();
    if let Some(m) = models.lock().unwrap().get(&name) {
        return *m;
    }
    let loaded = (|| {
        // Our own karts are built in (crate::goods), the CTR racers' read off the disc's data.
        if let Some(good) = crate::goods::good(&name).filter(|g| g.kind == crate::goods::Kind::Kart)
        {
            let bytes = crate::goods::json(good)?;
            let model: Model = serde_json::from_slice(&bytes)
                .map_err(|e| warn!("kart: {}: {e}", good.key))
                .ok()?;
            info!("kart: {}'s kart, built in", model.kart);
            return Some(&*Box::leak(Box::new(model)));
        }
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

/// The median of CTR's drivers' seats, meters in the kart's frame: the seat without kart data.
pub const MEDIAN_SEAT: Vec3 = Vec3::new(0.0, 0.254, -0.029);

/// Where a seated driver's hips go, in the kart's frame, meters.
pub fn seat() -> Vec3 {
    model().map_or(MEDIAN_SEAT, |m| Vec3::from_array(m.anchors.seat))
}

/// The seat of a racer's kart (see [`model_named`]), else ours.
pub fn seat_of(racer: &str) -> Vec3 {
    model_named(racer).map_or_else(seat, |m| Vec3::from_array(m.anchors.seat))
}

/// The median of CTR's karts' steering-wheel centres, meters in the kart's frame.
pub const MEDIAN_STEERING_WHEEL: Vec3 = Vec3::new(0.0, 0.607, 0.407);

/// The centre of our kart's steering-wheel ring, in the kart's frame, meters.
pub fn steering_wheel() -> Vec3 {
    model()
        .and_then(|m| m.anchors.steering_wheel)
        .map_or(MEDIAN_STEERING_WHEEL, Vec3::from_array)
}

/// The steering wheel of a racer's kart (see [`model_named`]), else ours.
pub fn steering_wheel_of(racer: &str) -> Vec3 {
    model_named(racer)
        .and_then(|m| m.anchors.steering_wheel)
        .map_or_else(steering_wheel, Vec3::from_array)
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

fn image(t: &Texture, paint: Option<Paint>, smooth: bool) -> Image {
    let mut rgba = t.rgba.clone();
    if let Some(paint) = paint {
        for px in rgba.as_chunks_mut::<4>().0 {
            let c = repaint([px[0], px[1], px[2]].map(|v| f32::from(v) / 255.0), paint);
            for (v, c) in px.iter_mut().zip(c) {
                *v = (c * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: t.width,
            height: t.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = if smooth {
        ImageSampler::linear()
    } else {
        ImageSampler::nearest()
    };
    image
}

fn surface_mesh(s: &Surface, paint: Option<Paint>) -> Mesh {
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
        v.iter()
            .map(|v| {
                let [r, g, b, a] = v.color;
                let [r, g, b] = paint.map_or([r, g, b], |p| repaint([r, g, b], p));
                linear([r, g, b, a])
            })
            .collect::<Vec<_>>(),
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

/// Whose kart a root is (its model's racer key), for what hangs off its own shape: the turbo
/// flames and the exhaust smoke ([`crate::effects`]).
#[derive(Component, Clone, Copy)]
pub struct KartRacer(pub &'static str);

/// Ours, placed from [`KartView`].
#[derive(Component)]
pub struct LocalKart;

/// A time trial's ghost: a translucent kart replaying a recorded run, silent and solid to
/// nothing ([`KartSpawner::spawn_ghost`]).
#[derive(Component)]
pub struct GhostKart;

/// A ghost kart's opacity.
const GHOST_ALPHA: f32 = 0.4;

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
    /// The back and the front tyres skid, laying their marks (`VehGroundSkids.c`).
    pub skids: (bool, bool),
    /// The ground under it as the host read it (whether or not it drives as it), for the dirt
    /// its tyres throw; other drivers' karts relay none (asphalt).
    pub ground: ctr_kart::Ground,
    /// This frame's scrapes and landing ([`crate::session::Feedback`]); other drivers' karts
    /// relay none.
    pub feedback: crate::session::Feedback,
    /// The boost trail it wears ([`crate::cosmetic::TRAILS`]), 0 none.
    pub trail: u8,
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
    /// By racer, whether translucent for a ghost, and its paint ([`crate::cosmetic`]).
    assets: std::collections::HashMap<(String, bool, u8), KartAssets>,
    local: Option<Entity>,
    /// Whose kart `local` is, and in which paint.
    local_racer: &'static str,
    local_paint: u8,
}

/// Each of `surfaces` as a mesh and its unlit material over `textures` (a kart's, or a flag's),
/// under `paint`.
pub(crate) fn build_parts(
    textures: &[Texture],
    surfaces: &[Surface],
    paint: Option<Paint>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
) -> Vec<(Handle<Mesh>, Handle<StandardMaterial>)> {
    let textures: Vec<Handle<Image>> = textures
        .iter()
        .map(|t| images.add(image(t, paint, false)))
        .collect();
    parts_of(&textures, surfaces, paint, meshes, materials)
}

fn parts_of(
    textures: &[Handle<Image>],
    surfaces: &[Surface],
    paint: Option<Paint>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) -> Vec<(Handle<Mesh>, Handle<StandardMaterial>)> {
    surfaces
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
            (meshes.add(surface_mesh(s, paint)), material)
        })
        .collect()
}

fn build_assets(
    model: &Model,
    paint: Option<Paint>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
) -> KartAssets {
    let textures: Vec<Handle<Image>> = model
        .textures
        .iter()
        .map(|t| images.add(image(t, paint, model.smooth)))
        .collect();
    let parts = parts_of(&textures, &model.surfaces, paint, meshes, materials);
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
        self.spawn_as(racer, false, 0)
    }

    /// [`KartSpawner::spawn`]'s kart in a paint ([`crate::cosmetic::PAINTS`]); 0 or an unknown
    /// one is stock.
    pub fn spawn_painted(&mut self, racer: &str, paint: u8) -> Option<Entity> {
        self.spawn_as(racer, false, paint)
    }

    /// A ghost kart in `racer`'s colours: [`KartSpawner::spawn`]'s kart, translucent and marked
    /// [`GhostKart`].
    pub fn spawn_ghost(&mut self, racer: &str) -> Option<Entity> {
        let kart = self.spawn_as(racer, true, 0)?;
        self.commands
            .entity(kart)
            .insert((GhostKart, Name::new("ghost kart")));
        Some(kart)
    }

    fn spawn_as(&mut self, racer: &str, ghost: bool, paint: u8) -> Option<Entity> {
        let model = model_named(racer).or_else(model)?;
        // A built-in kart (crate::goods) wears its own livery: a paint is for CTR's karts.
        let painted =
            crate::cosmetic::paint(paint).filter(|_| crate::goods::good(&model.kart).is_none());
        let key = (
            model.kart.clone(),
            ghost,
            if painted.is_some() { paint } else { 0 },
        );
        if !self.model.assets.contains_key(&key) {
            let built = build_assets(
                model,
                painted,
                &mut self.meshes,
                &mut self.materials,
                &mut self.images,
            );
            if ghost {
                let handles = built
                    .parts
                    .iter()
                    .map(|(_, m)| m)
                    .chain([&built.disc_material, &built.tread_material]);
                for handle in handles {
                    if let Some(m) = self.materials.get_mut(handle) {
                        m.alpha_mode = AlphaMode::Blend;
                        m.base_color = m.base_color.with_alpha(GHOST_ALPHA);
                    }
                }
            }
            self.model.assets.insert(key.clone(), built);
        }
        let assets = self.model.assets.get(&key)?;
        let commands = &mut self.commands;
        let root = commands
            .spawn((
                KartRoot,
                KartRacer(model.kart.as_str()),
                KartWheels::default(),
                KartStatus::default(),
                crate::goods::KartFlag::default(),
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
        let paint = crate::cosmetic::ours().0;
        if let Some(e) = self.model.local {
            if self.model.local_racer == racer && self.model.local_paint == paint {
                return Some(e);
            }
            self.despawn(e);
            self.model.local = None;
        }
        let e = self.spawn_painted(racer, paint)?;
        self.commands.entity(e).insert(LocalKart);
        self.model.local = Some(e);
        self.model.local_racer = racer;
        self.model.local_paint = paint;
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
            &mut crate::goods::KartFlag,
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
    let Ok((mut t, mut visibility, mut wheels, mut status, mut flag)) = root.single_mut() else {
        return;
    };
    if !view.active {
        *visibility = Visibility::Hidden;
        return;
    }
    *t = view.transform;
    *visibility = if view.hidden {
        Visibility::Hidden
    } else {
        Visibility::Visible
    };
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
        skids: view.skids,
        ground: view.ground,
        feedback: view.feedback,
        trail: crate::cosmetic::ours().1,
    };
    flag.set_if_neq(crate::goods::KartFlag(crate::goods::our_flag()));
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
