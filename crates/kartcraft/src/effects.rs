//! The kart's effects, as CTR draws them, from `kart-data/effects.json` (written by
//! `ctr-extract` from the player's disc; `docs/ctr-assets.md`, "Effects"):
//!
//! - **Turbo flames** (`VehTurbo.c`): while reserves burn, a flame out of each exhaust pipe, one
//!   of eight small models per game frame (the right flame three frames on from the left),
//!   scaled `size / 8` by the boost that lit it.
//! - **Exhaust smoke** (`VehEmitter_Exhaust`): a puff out of each pipe every game frame unless a
//!   flame burns, riding with the kart; additive grey, but **dark** (subtractive) while the drift
//!   meter is in its boost window, CTR's "boost now" cue, and puffing on through a flame for the
//!   eight frames after a boost pressed too early.
//! - **Skid marks** (`VehGroundSkids.c`): a skidding tyre lays a tread mark, the last eight game
//!   frames of it, darkening the ground: all four through a powerslide, the back pair under the
//!   brake or spinning off the line ([`ctr_kart::Kart::skids`]).
//! - **Sparks and dirt** (`VehEmitter_Sparks_Ground`, `VehEmitter_Sparks_Wall`,
//!   `VehEmitter_Terrain_Ground`): additive lines, sparks off a hard landing and along a wall
//!   scraped, dirt off the tyres at speed.
//!
//! The PSX blends map onto Bevy's: additive to `Add`, a 50/50 `mix` to `Blend` at half alpha,
//! subtractive to `Multiply` by the inverted texture. The flames' subtractive outer haze is left
//! out: drawn as a multiply it smothers the kart.
//!
//! Every [`KartRoot`] gets them, ours and other drivers', from its [`KartStatus`]; a ghost
//! ([`GhostKart`]) has none.

use std::collections::{HashMap, VecDeque};
use std::sync::OnceLock;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::image::ImageSampler;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::model::{GhostKart, KartRoot, KartStatus};

/// CTR updates its effects once a frame at 30 fps.
const FRAME: f32 = 1.0 / 30.0;
/// Exhaust puffs: kart-local launch velocity (m/s), gravity (m/s²), half-size and its growth (m,
/// m/s), lifetime (s), and grey at birth and its fall per second.
const PUFF_VELOCITY: Vec3 = Vec3::new(0.0, 2.34, -2.34);
const PUFF_GRAVITY: f32 = -17.6;
const PUFF_SIZE: f32 = 0.234;
const PUFF_GROWTH: f32 = 1.83;
const PUFF_LIFE: f32 = 0.2;
const PUFF_GREY: f32 = 0.251;
const PUFF_FADE: f32 = 1.176;
/// The pipes, when `kart.json` names none (m).
const DEFAULT_EXHAUST: [[f32; 3]; 2] = [[0.281, 0.875, -0.875], [-0.281, 0.875, -0.875]];
/// Skid marks: the tyres' ground points (m), the mark's width (m) and how many game frames of it
/// survive.
const TYRES: [[f32; 3]; 4] = [
    [0.586, 0.0, -0.39],
    [-0.586, 0.0, -0.39],
    [0.586, 0.0, 0.78],
    [-0.586, 0.0, 0.78],
];
const SKID_WIDTH: f32 = 0.39;
const SKID_FRAMES: usize = 8;

#[derive(serde::Deserialize)]
pub(crate) struct Texture {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) rgba: Vec<u8>,
}

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
struct Instance {
    origin: [f32; 3],
    #[serde(default)]
    mirror_x: bool,
}

#[derive(serde::Deserialize)]
struct Flame {
    frames: Vec<Frame>,
    instances: Vec<Instance>,
}

#[derive(serde::Deserialize)]
struct Effects {
    textures: Vec<Texture>,
    turbo_flame: Flame,
}

/// The effects, with the smoke's and the tread's texture indices.
type Loaded = (Effects, Option<usize>, Option<usize>);

fn effects() -> Option<&'static Loaded> {
    static FX: OnceLock<Option<Loaded>> = OnceLock::new();
    FX.get_or_init(|| {
        let path = crate::model::data_root()?.join("effects.json");
        let data = std::fs::read(&path).ok()?;
        let raw: serde_json::Value = serde_json::from_slice(&data)
            .map_err(|e| warn!("kart effects: {e}"))
            .ok()?;
        let fx: Effects = serde_json::from_value(raw.clone())
            .map_err(|e| warn!("kart effects: {e}"))
            .ok()?;
        let named = |name: &str| {
            raw["textures"]
                .as_array()?
                .iter()
                .position(|t| t["name"] == name)
        };
        let (smoke, tread) = (named("dustpuff00"), named("tread"));
        info!(
            "kart effects: {} flame frames from {}",
            fx.turbo_flame.frames.len(),
            path.display()
        );
        Some((fx, smoke, tread))
    })
    .as_ref()
}

/// `invert`: for a multiply standing in for the PSX subtract, `1 − strength·texel`, with the
/// transparent texels white (no change).
pub(crate) fn image(t: &Texture, invert: Option<f32>) -> Image {
    let mut rgba = t.rgba.clone();
    if let Some(strength) = invert {
        for px in rgba.chunks_mut(4) {
            let on = px[3] != 0;
            for c in &mut px[..3] {
                *c = if on {
                    255 - (f32::from(*c) * strength).round() as u8
                } else {
                    255
                };
            }
            px[3] = 255;
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
    image.sampler = ImageSampler::nearest();
    image
}

pub(crate) fn unlit(texture: Option<Handle<Image>>, alpha_mode: AlphaMode) -> StandardMaterial {
    StandardMaterial {
        base_color_texture: texture,
        unlit: true,
        alpha_mode,
        cull_mode: None,
        double_sided: true,
        ..default()
    }
}

fn flame_mesh(s: &Surface) -> Mesh {
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
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_COLOR,
        v.iter()
            .map(|v| [v.color[0], v.color[1], v.color[2], v.color[3]])
            .collect::<Vec<_>>(),
    );
    mesh.insert_indices(Indices::U32(s.indices.clone()));
    mesh
}

fn dynamic_mesh() -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, Vec::<[f32; 3]>::new());
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, Vec::<[f32; 3]>::new());
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, Vec::<[f32; 2]>::new());
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, Vec::<[f32; 4]>::new());
    mesh.insert_indices(Indices::U32(Vec::new()));
    mesh
}

/// The shared flame meshes and materials, and the three batched meshes the particles and marks
/// are rebuilt into each frame.
struct Built {
    frames: Vec<Vec<(Handle<Mesh>, Handle<StandardMaterial>)>>,
    instances: Vec<(Vec3, bool)>,
    light: Handle<Mesh>,
    dark: Handle<Mesh>,
    skids: Handle<Mesh>,
    /// The sparks and dirt, additive lines ([`Spark`]).
    lines: Handle<Mesh>,
}

#[derive(Resource, Default)]
pub struct KartEffects {
    built: Option<Option<Built>>,
    /// Sparks off the ground and walls and dirt off the tyres, in the world.
    sparks: Vec<Spark>,
    rng: u32,
    puffs: Vec<Puff>,
    /// Each kart's tyres' last cross-sections (two ground points), newest last.
    marks: HashMap<(Entity, usize), VecDeque<Option<Section>>>,
}

/// A skid mark's cross-section: its two ground points.
type Section = (Vec3, Vec3);

struct Puff {
    kart: Entity,
    /// From the kart's origin, yards: puffs ride with the kart.
    offset: Vec3,
    velocity: Vec3,
    age: f32,
    dark: bool,
}

/// A kart's flames and its game-frame clock.
#[derive(Component)]
pub struct KartFx {
    flames: Vec<(Entity, Vec<Entity>)>,
    clock: f32,
    frame: usize,
}

fn build(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
) -> Option<Built> {
    let (fx, smoke, tread) = effects()?;
    let plain: Vec<Handle<Image>> = fx
        .textures
        .iter()
        .map(|t| images.add(image(t, None)))
        .collect();
    let tex = |i: Option<usize>| i.and_then(|i| plain.get(i).cloned());
    let mut inverted = |i: Option<usize>, strength: f32| {
        i.and_then(|i| fx.textures.get(i))
            .map(|t| images.add(image(t, Some(strength))))
    };
    let mut cache: HashMap<(Option<usize>, String), Handle<StandardMaterial>> = HashMap::new();
    let frames = fx
        .turbo_flame
        .frames
        .iter()
        .map(|f| {
            f.surfaces
                .iter()
                .filter(|s| s.blend != "subtractive")
                .map(|s| {
                    let material = cache
                        .entry((s.texture, s.blend.clone()))
                        .or_insert_with(|| {
                            let mode = match s.blend.as_str() {
                                "additive" | "add_quarter" => AlphaMode::Add,
                                "mix" => AlphaMode::Blend,
                                _ => AlphaMode::Mask(0.5),
                            };
                            let mut m = unlit(tex(s.texture), mode);
                            m.base_color = match s.blend.as_str() {
                                "add_quarter" => Color::srgb(0.25, 0.25, 0.25),
                                "mix" => Color::srgba(1.0, 1.0, 1.0, 0.5),
                                _ => Color::WHITE,
                            };
                            materials.add(m)
                        })
                        .clone();
                    (meshes.add(flame_mesh(s)), material)
                })
                .collect()
        })
        .collect();
    let instances = fx
        .turbo_flame
        .instances
        .iter()
        .map(|i| (Vec3::from_array(i.origin), i.mirror_x))
        .collect();
    let mut batch = |material: StandardMaterial| {
        let mesh = meshes.add(dynamic_mesh());
        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(materials.add(material)),
            Transform::default(),
            NoFrustumCulling,
            Name::new("kart effects"),
        ));
        mesh
    };
    let light = batch(unlit(tex(*smoke), AlphaMode::Add));
    let dark = batch(unlit(inverted(*smoke, 1.0), AlphaMode::Multiply));
    // The marks subtract at CTR's intensity 0x7F: about half.
    let skids = batch(unlit(inverted(*tread, 0.5), AlphaMode::Multiply));
    let lines = batch(unlit(None, AlphaMode::Add));
    Some(Built {
        frames,
        instances,
        light,
        dark,
        skids,
        lines,
    })
}

/// Gives each kart its flames, once the effects are built.
#[allow(clippy::type_complexity)]
pub fn attach(
    mut commands: Commands,
    mut state: ResMut<KartEffects>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    karts: Query<Entity, (With<KartRoot>, Without<KartFx>, Without<GhostKart>)>,
) {
    if karts.is_empty() {
        return;
    }
    let built = state
        .built
        .get_or_insert_with(|| build(&mut commands, &mut meshes, &mut materials, &mut images));
    let Some(built) = built.as_ref() else {
        return;
    };
    for kart in &karts {
        let flames = built
            .instances
            .iter()
            .map(|&(origin, mirror)| {
                let scale = if mirror {
                    Vec3::new(-1.0, 1.0, 1.0)
                } else {
                    Vec3::ONE
                };
                let flame = commands
                    .spawn((
                        Transform::from_translation(origin).with_scale(scale),
                        Visibility::Hidden,
                    ))
                    .id();
                let frames = built
                    .frames
                    .iter()
                    .map(|parts| {
                        let frame = commands
                            .spawn((Transform::default(), Visibility::Hidden))
                            .id();
                        for (mesh, material) in parts {
                            let part = commands
                                .spawn((
                                    Mesh3d(mesh.clone()),
                                    MeshMaterial3d(material.clone()),
                                    Transform::default(),
                                    NoFrustumCulling,
                                ))
                                .id();
                            commands.entity(frame).add_child(part);
                        }
                        commands.entity(flame).add_child(frame);
                        frame
                    })
                    .collect();
                commands.entity(kart).add_child(flame);
                (flame, frames)
            })
            .collect();
        commands.entity(kart).insert(KartFx {
            flames,
            clock: 0.0,
            frame: 0,
        });
    }
}

/// Runs each kart's effects at CTR's 30 Hz: flames lit and animated, puffs emitted, marks laid.
pub fn tick(
    time: Res<Time>,
    mut state: ResMut<KartEffects>,
    mut karts: Query<
        (
            Entity,
            &Transform,
            &InheritedVisibility,
            &KartStatus,
            &mut KartFx,
        ),
        With<KartRoot>,
    >,
    mut visibility: Query<(&mut Visibility, &mut Transform), Without<KartRoot>>,
) {
    let dt = time.delta_secs().min(0.1);
    let state = &mut *state;
    let pipes: Vec<Vec3> = crate::model::model()
        .map(|m| m.anchors.exhaust.clone())
        .filter(|e| !e.is_empty())
        .unwrap_or_else(|| DEFAULT_EXHAUST.to_vec())
        .into_iter()
        .map(Vec3::from_array)
        .collect();
    let mut live = Vec::new();
    for (kart, t, shown, status, mut fx) in &mut karts {
        if !shown.get() {
            continue;
        }
        live.push(kart);
        fx.clock += dt;
        let mut frames = 0;
        while fx.clock >= FRAME {
            fx.clock -= FRAME;
            fx.frame = (fx.frame + 1) % 8;
            frames += 1;
        }
        // Flames.
        let lit = status.fire > 0;
        let size = f32::from(status.fire) / 8.0;
        for (i, (flame, parts)) in fx.flames.iter().enumerate() {
            if let Ok((mut vis, mut ft)) = visibility.get_mut(*flame) {
                *vis = if lit {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                };
                let mirror = ft.scale.x.signum();
                ft.scale = Vec3::new(mirror, 1.0, 1.0) * size.max(0.01);
            }
            let shown_frame = (fx.frame + 3 * i) % parts.len().max(1);
            for (j, part) in parts.iter().enumerate() {
                if let Ok((mut vis, _)) = visibility.get_mut(*part) {
                    *vis = if j == shown_frame {
                        Visibility::Inherited
                    } else {
                        Visibility::Hidden
                    };
                }
            }
        }
        // Puffs and marks, once per game frame elapsed.
        let to_yards = t.scale.x;
        for _ in 0..frames {
            let smoking = !lit || status.boost_window || status.failed_boost;
            if smoking {
                for pipe in &pipes {
                    state.puffs.push(Puff {
                        kart,
                        offset: t.rotation * (*pipe * to_yards),
                        velocity: t.rotation * (PUFF_VELOCITY * to_yards),
                        age: 0.0,
                        dark: status.boost_window,
                    });
                }
            }
            let right = t.rotation * Vec3::X;
            for (i, tyre) in TYRES.iter().enumerate() {
                let marks = state.marks.entry((kart, i)).or_default();
                // The back pair marks on `ACTION_BACK_SKID`, the front on `ACTION_FRONT_SKID`
                // (`VehEmitter.c:812`, `:948`).
                let back = TYRES[i][2] < 0.0;
                let flag = if back { status.skids.0 } else { status.skids.1 };
                let skidding = status.grounded && flag;
                let section = skidding.then(|| {
                    let at = t.transform_point(Vec3::from_array(*tyre)) + Vec3::Y * 0.03;
                    let half = right * (SKID_WIDTH * 0.5 * to_yards);
                    (at - half, at + half)
                });
                marks.push_back(section);
                while marks.len() > SKID_FRAMES {
                    marks.pop_front();
                }
            }
            state.dirt_and_wall_sparks(t, status, to_yards);
        }
        state.landing_sparks(t, status, to_yards);
    }
    state.marks.retain(|(kart, _), _| live.contains(kart));
    state.puffs.retain(|p| live.contains(&p.kart));
    for p in &mut state.puffs {
        p.age += dt;
        p.velocity.y += PUFF_GRAVITY * dt / crate::METERS_PER_YARD;
        p.offset += p.velocity * dt;
    }
    state.puffs.retain(|p| p.age < PUFF_LIFE);
    for s in &mut state.sparks {
        s.step(dt);
    }
    state.sparks.retain(|s| s.age < s.kind.life());
}

// ---------------------------------------------------------------------------------------------
// Sparks and dirt: CTR's line particles (`effects.json` "landing_sparks" and "dirt_dust", whose
// numbers these are; `zGlobal_DATA.c` emSet_GroundSparks, emSet_WallSparks, emSet_DirtLR).
// Meters and seconds in the kart model's frame, scaled with the kart.
// ---------------------------------------------------------------------------------------------

/// A line's width against its distance from the eye: about a pixel of the PSX's 240 lines.
const LINE_WIDTH: f32 = 0.003;

/// FP8 world units a frame to m/s: `/ 256 / 51.2 * 30`.
const fn fp8_per_frame(v: f32) -> f32 {
    v / 256.0 / 51.2 * 30.0
}

/// What a line particle is.
#[derive(Clone, Copy, Debug, PartialEq)]
enum SparkKind {
    /// `emSet_GroundSparks` / `emSet_WallSparks`: yellow-white, 8 frames, a line two frames of
    /// travel long.
    Spark,
    /// `emSet_DirtLR`: orange-brown, 10 frames.
    Dirt,
    /// `emSet_GrassL/R` (`zGlobal_DATA.c:6642-6899`): green, 10 frames, thrown higher.
    Grass,
}

impl SparkKind {
    fn life(self) -> f32 {
        match self {
            SparkKind::Spark => 8.0 * FRAME,
            SparkKind::Dirt | SparkKind::Grass => 10.0 * FRAME,
        }
    }

    /// The head's colour `age` seconds in; the tail is black (additive: unseen).
    fn color(self, age: f32) -> [f32; 3] {
        let (start, fall) = match self {
            SparkKind::Spark => ([1.0, 1.0, 0.5], [7680.0, 7680.0, 3840.0]),
            SparkKind::Dirt => ([1.0, 0.5, 0.0], [2048.0, 1024.0, 0.0]),
            SparkKind::Grass => ([15360.0 / 65280.0, 1.0, 0.0], [0.0; 3]),
        };
        std::array::from_fn(|i| (start[i] - fall[i] / 65280.0 * 30.0 * age).max(0.0))
    }

    /// How many frames of travel the line spans.
    fn length(self) -> f32 {
        match self {
            SparkKind::Spark => 2.0,
            SparkKind::Dirt | SparkKind::Grass => 1.0,
        }
    }
}

/// One line particle, in the world (yards).
struct Spark {
    kind: SparkKind,
    at: Vec3,
    velocity: Vec3,
    gravity: f32,
    age: f32,
}

impl Spark {
    fn step(&mut self, dt: f32) {
        self.age += dt;
        self.velocity.y += self.gravity * dt;
        self.at += self.velocity * dt;
    }
}

/// `VEH_EMITTER_LANDING_SPARK_MIN_SPEED`, `_MIN_JUMP` (`VehEmitter.c:92-93`), speed units.
const SPARK_LANDING_SPEED: i32 = 6 * 256;
const SPARK_LANDING_FALL: i32 = 22 * 256;
/// `VEH_EMITTER_GROUND_SPARK_COUNT` (`VehEmitter.c:70`).
const SPARK_LANDING_COUNT: usize = 10;
/// `VEH_EMITTER_WALL_SPARK_MIN_SPEED` (`VehEmitter.c:112`), speed units.
const SPARK_WALL_SPEED: i32 = 2 * 256;
/// `VEH_EMITTER_TERRAIN_MIN_SPEED` (`VehEmitter.c:72`): the tyres throw dirt from this speed.
const DIRT_SPEED: i32 = 0x300;
/// The wall sparks' corners (`VehEmitter.c:117-121`: ±0x22, 0xA, +0x28 world units), meters.
const WALL_CORNERS: [[f32; 3]; 2] = [
    [34.0 / 51.2, 10.0 / 51.2, 40.0 / 51.2],
    [-34.0 / 51.2, 10.0 / 51.2, 40.0 / 51.2],
];

impl KartEffects {
    fn random(&mut self) -> f32 {
        if self.rng == 0 {
            self.rng = 0x9E37_79B9;
        }
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng as f32 / u32::MAX as f32
    }

    /// A particle at `local` (model meters) on the kart: the emitter set's random spread, its
    /// kart-local velocity turned into the world.
    fn spawn(&mut self, kind: SparkKind, t: &Transform, local: Vec3, to_yards: f32) {
        let spread = |r: f32, lo: f32, hi: f32| lo + (hi - lo) * r;
        let (side, up, gravity, offset) = match kind {
            // Ground and wall sparks: x/z ±500, y 1000..1800, gravity -300..-200 (FP8).
            SparkKind::Spark => (
                fp8_per_frame(500.0),
                (fp8_per_frame(1000.0), fp8_per_frame(1800.0)),
                spread(self.random(), -300.0, -200.0),
                0.0,
            ),
            // Dirt: x/z ±500, y 1500..1900, gravity -300, ±2560 FP8 of jitter.
            SparkKind::Dirt => (
                fp8_per_frame(500.0),
                (fp8_per_frame(1500.0), fp8_per_frame(1900.0)),
                -300.0,
                2560.0 / 256.0 / 51.2,
            ),
            // Grass: as dirt, but y 2000..2400.
            SparkKind::Grass => (
                fp8_per_frame(500.0),
                (fp8_per_frame(2000.0), fp8_per_frame(2400.0)),
                -300.0,
                2560.0 / 256.0 / 51.2,
            ),
        };
        let jitter = Vec3::new(
            spread(self.random(), -offset, offset),
            0.0,
            spread(self.random(), -offset, offset),
        );
        let velocity = Vec3::new(
            spread(self.random(), -side, side),
            spread(self.random(), up.0, up.1),
            spread(self.random(), -side, side),
        );
        self.sparks.push(Spark {
            kind,
            at: t.transform_point(local + jitter),
            velocity: t.rotation * velocity * to_yards,
            gravity: gravity / 256.0 / 51.2 * 900.0 * to_yards,
            age: 0.0,
        });
    }

    /// One game frame of a kart's dirt and wall sparks. Dirt (`VehEmitter_Terrain_Ground`,
    /// `VehEmitter.c:1342-1353`): on the ground at 0x300 or faster, one off each rear tyre, each
    /// of the four while drifting, on a ground whose `MetaDataTerrain` row has an emitter: dirt
    /// (`emSet_DirtLR`) and grass (`emSet_GrassL/R`), slow or not (`zGlobal_DATA.c:6982-7116`).
    /// The ground is the one read under the kart, whether or not it drives as it (`KartTerrain`).
    /// Wall sparks (`VehEmitter_Sparks_Wall`, `VehEmitter.c:506-675`): one a frame while
    /// scraping faster than 2.0, off a front corner; CTR picks the corner nearer the wall,
    /// which our collision does not report, so the corners take turns.
    fn dirt_and_wall_sparks(&mut self, t: &Transform, status: &KartStatus, to_yards: f32) {
        use ctr_kart::Ground;
        let speed = (status.speed * crate::lu_per_yard() * 256.0 / 30.0) as i32;
        let thrown = match status.ground {
            Ground::Dirt | Ground::SlowDirt => Some(SparkKind::Dirt),
            Ground::Grass | Ground::SlowGrass => Some(SparkKind::Grass),
            _ => None,
        };
        if let Some(kind) = thrown.filter(|_| status.grounded && speed >= DIRT_SPEED) {
            let tyres = if status.drifting { 0..4 } else { 0..2 };
            for tyre in &TYRES[tyres] {
                let at = Vec3::new(tyre[0], 10.0 / 51.2, tyre[2]);
                self.spawn(kind, t, at, to_yards);
            }
        }
        if status.feedback.wall_frames > 0 && speed > SPARK_WALL_SPEED {
            let corner = WALL_CORNERS[(self.sparks.len() + self.rng as usize) % 2];
            self.spawn(SparkKind::Spark, t, Vec3::from_array(corner), to_yards);
        }
    }

    /// A landing's sparks (`VehEmitter.c:1320-1340`, `VehEmitter_Sparks_Ground`): ten around
    /// the kart when it lands faster than 6.0 from a jump faster than 22.0 on a sparking
    /// terrain (`TERRAIN_FLAG_LANDING_SPARKS`: asphalt, stone, icy road).
    fn landing_sparks(&mut self, t: &Transform, status: &KartStatus, to_yards: f32) {
        let Some((fall, speed)) = status.feedback.landing else {
            return;
        };
        if status.ground.terrain().flags & ctr_kart::terrain::flags::LANDING_SPARKS == 0 {
            return;
        }
        if speed.abs() <= SPARK_LANDING_SPEED || fall <= SPARK_LANDING_FALL {
            return;
        }
        for _ in 0..SPARK_LANDING_COUNT {
            let at = Vec3::new(
                (self.random() - 0.5) * 1.2,
                10.0 / 51.2,
                (self.random() - 0.5) * 1.6,
            );
            self.spawn(SparkKind::Spark, t, at, to_yards);
        }
    }
}

/// A batch's vertex streams.
#[derive(Default)]
struct Streams {
    positions: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    colors: Vec<[f32; 4]>,
    indices: Vec<u32>,
}

impl Streams {
    fn quad(&mut self, corners: [Vec3; 4], color: [f32; 4]) {
        let base = self.positions.len() as u32;
        for (c, uv) in corners
            .iter()
            .zip([[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]])
        {
            self.positions.push(c.to_array());
            self.uvs.push(uv);
            self.colors.push(color);
        }
        self.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    /// A line's quad, `[tail, head, head, tail]`: black at the tail, `color` at the head.
    fn line(&mut self, corners: [Vec3; 4], color: [f32; 4]) {
        let base = self.positions.len() as u32;
        let black = [0.0, 0.0, 0.0, 1.0];
        for (c, (uv, col)) in corners.iter().zip([
            ([0.0, 0.0], black),
            ([1.0, 0.0], color),
            ([1.0, 1.0], color),
            ([0.0, 1.0], black),
        ]) {
            self.positions.push(c.to_array());
            self.uvs.push(uv);
            self.colors.push(col);
        }
        self.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    fn write(self, mesh: &mut Mesh) {
        let n = self.positions.len();
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, self.positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; n]);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, self.uvs);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, self.colors);
        mesh.insert_indices(Indices::U32(self.indices));
    }
}

/// Rebuilds the puff and mark batches for this frame's camera.
pub fn draw(
    state: Res<KartEffects>,
    karts: Query<&Transform, With<KartRoot>>,
    camera: Query<&GlobalTransform, With<benilla_world::view::WorldCamera>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let Some(Some(built)) = &state.built else {
        return;
    };
    let Ok(eye) = camera.single() else {
        return;
    };
    let (cam_right, cam_up) = (eye.right().as_vec3(), eye.up().as_vec3());
    let (mut light, mut dark, mut skids) =
        (Streams::default(), Streams::default(), Streams::default());
    for p in &state.puffs {
        let Ok(kart) = karts.get(p.kart) else {
            continue;
        };
        let at = kart.translation + p.offset;
        let half = (PUFF_SIZE + PUFF_GROWTH * p.age) * kart.scale.x;
        let (r, u) = (cam_right * half, cam_up * half);
        let corners = [at - r + u, at + r + u, at + r - u, at - r - u];
        let grey = (PUFF_GREY - PUFF_FADE * p.age).max(0.0);
        if p.dark {
            dark.quad(corners, [1.0; 4]);
        } else {
            light.quad(corners, [grey, grey, grey, 1.0]);
        }
    }
    for marks in state.marks.values() {
        for pair in marks.iter().collect::<Vec<_>>().windows(2) {
            if let (Some((a0, a1)), Some((b0, b1))) = (pair[0], pair[1]) {
                skids.quad([*a0, *a1, *b1, *b0], [1.0; 4]);
            }
        }
    }
    // The sparks and dirt as CTR's lines: from where the particle was a frame or two ago (the
    // tail, black) to where it is (the head, its colour), a pixel or so wide.
    let mut lines = Streams::default();
    let eye_at = eye.translation();
    for s in &state.sparks {
        let tail = s.at - s.velocity * (s.kind.length() * FRAME);
        let along = s.at - tail;
        let width = LINE_WIDTH * (eye_at - s.at).length();
        let side = along.cross(eye_at - s.at).normalize_or_zero() * (width * 0.5);
        let [r, g, b] = s.kind.color(s.age);
        lines.line(
            [tail - side, s.at - side, s.at + side, tail + side],
            [r, g, b, 1.0],
        );
    }
    for (handle, streams) in [
        (&built.light, light),
        (&built.dark, dark),
        (&built.skids, skids),
        (&built.lines, lines),
    ] {
        if let Some(mesh) = meshes.get_mut(handle) {
            streams.write(mesh);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Feedback;

    /// yd/s for a speed in CTR's units.
    fn yards(speed: i32) -> f32 {
        speed as f32 * 30.0 / 256.0 / crate::lu_per_yard()
    }

    #[test]
    fn sparks_and_dirt_spawn_as_ctr_s_emitters_do() {
        let t = Transform::IDENTITY;
        let mut fx = KartEffects::default();
        let mut s = KartStatus {
            grounded: true,
            speed: yards(DIRT_SPEED - 0x20),
            ground: ctr_kart::Ground::Dirt,
            ..Default::default()
        };
        // Below 0x300 the tyres throw nothing; above, one off each rear tyre, four drifting.
        fx.dirt_and_wall_sparks(&t, &s, 1.0);
        assert!(fx.sparks.is_empty());
        s.speed = yards(0x1000);
        fx.dirt_and_wall_sparks(&t, &s, 1.0);
        assert_eq!(fx.sparks.len(), 2);
        s.drifting = true;
        fx.dirt_and_wall_sparks(&t, &s, 1.0);
        assert_eq!(fx.sparks.len(), 6);
        assert!(fx.sparks.iter().all(|p| p.kind == SparkKind::Dirt));
        // Grass throws its own; asphalt, stone and snow throw nothing.
        fx.sparks.clear();
        s.ground = ctr_kart::Ground::SlowGrass;
        fx.dirt_and_wall_sparks(&t, &s, 1.0);
        assert_eq!(fx.sparks.len(), 4);
        assert!(fx.sparks.iter().all(|p| p.kind == SparkKind::Grass));
        fx.sparks.clear();
        for ground in [
            ctr_kart::Ground::Asphalt,
            ctr_kart::Ground::Stone,
            ctr_kart::Ground::Snow,
        ] {
            s.ground = ground;
            fx.dirt_and_wall_sparks(&t, &s, 1.0);
        }
        assert!(fx.sparks.is_empty());
        s.ground = ctr_kart::Ground::Asphalt;
        // Scraping a wall: one spark a frame.
        fx.sparks.clear();
        s.grounded = false;
        s.feedback.wall_frames = 1;
        fx.dirt_and_wall_sparks(&t, &s, 1.0);
        assert_eq!(fx.sparks.len(), 1);
        // A landing from a jump faster than 22.0 at a speed over 6.0: ten sparks.
        fx.sparks.clear();
        s.feedback = Feedback {
            landing: Some((SPARK_LANDING_FALL - 1, 0x2000)),
            ..Default::default()
        };
        fx.landing_sparks(&t, &s, 1.0);
        assert!(fx.sparks.is_empty());
        s.feedback.landing = Some((SPARK_LANDING_FALL + 1, 0x2000));
        // Landing on grass makes none (no TERRAIN_FLAG_LANDING_SPARKS); on asphalt, ten.
        s.ground = ctr_kart::Ground::Grass;
        fx.landing_sparks(&t, &s, 1.0);
        assert!(fx.sparks.is_empty());
        s.ground = ctr_kart::Ground::Asphalt;
        fx.landing_sparks(&t, &s, 1.0);
        assert_eq!(fx.sparks.len(), SPARK_LANDING_COUNT);
        // They fly up and fall, and are gone after their 8 frames.
        assert!(fx.sparks[0].velocity.y > 0.0 && fx.sparks[0].gravity < 0.0);
        for p in &mut fx.sparks {
            p.step(9.0 * FRAME);
        }
        fx.sparks.retain(|p| p.age < p.kind.life());
        assert!(fx.sparks.is_empty());
        // The head fades from CTR's start colour.
        assert_eq!(SparkKind::Spark.color(0.0), [1.0, 1.0, 0.5]);
        assert!(SparkKind::Dirt.color(0.2)[0] < 1.0);
    }
}
