//! Builds `kart.json`: one racer's kart without its driver, plus wheels and anchors.
//!
//! CTR draws each racer as a single animated mesh (`models\racers\hi\<name>.ctr`) holding both
//! the kart and the character. Two independent signals separate them (see [`classify`]):
//!
//! 1. **Shared geometry.** Every racer sits in the same kart geometry (only colours differ), so
//!    a kart triangle's corners also exist, at the same model-space positions, in most other
//!    racers' meshes. Driver corners are unique to the character.
//! 2. **Animation.** In the `turn` animation the driver leans and steers while the kart body
//!    does not move at all (bit-identical vertices in every frame).
//!
//! A triangle is kart when it is static and shared by at least [`MIN_SHARED_STATIC`] other
//! racers. Animated kart parts (the steering wheel turns, a rear engine piece shakes) are kept
//! when shared by at least [`MIN_SHARED_MOVING`] racers, or [`MIN_SHARED_STEERING`] in front of
//! the seat (the wheel ring sits closer or further for big/small drivers, so fewer match).
//!
//! The wheels are not part of the mesh: `DrawTires` draws each one as a sprite (17 frames,
//! `shared.mpk` icon group 0 `tireAnim`) at fixed instance-local positions; see [`wheels`].

use serde::Serialize;

use crate::big::{self, BigFile};
use crate::ctr::{Model, ModelMesh, TexLayout, Triangle};
use crate::mpk;
use crate::vram::Vram;
use crate::{Error, Result};

/// CTR model units per meter in `kart.json`. A model unit is `(byte + frame.pos) * scale /
/// 4096` (before the racer instance's own 0.8 scale, which we drop). At 64 units/m the kart is
/// ~1.75 m long and its wheels are 0.5 m across: cartoon go-kart proportions.
pub const UNITS_PER_METER: f32 = 64.0;

/// Kart choices: the racers whose kart separates completely (see [`check_complete`]). N. Gin,
/// Dingodile, Papu Papu and N. Tropy have a hood/steering column fitted to their body that is
/// not shared, so their kart would come out without it; Oxide drives a hovercraft.
pub const KARTS: [&str; 11] = [
    "crash",
    "cortex",
    "tiny",
    "coco",
    "polar",
    "pura",
    "pinstripe",
    "roo",
    "joe",
    "pen",
    "fake",
];

/// Racers compared against when classifying (Fake Crash is excluded because he shares Crash's
/// driver geometry; Oxide because his vehicle differs).
const REFERENCE: [&str; 14] = [
    "crash",
    "cortex",
    "tiny",
    "coco",
    "ngin",
    "dingo",
    "polar",
    "pura",
    "pinstripe",
    "papu",
    "roo",
    "joe",
    "ntropy",
    "pen",
];

/// Per-axis tolerance (model units) for two vertices to count as the same position. Models
/// are quantized to 8 bits per axis with per-model scales, so the same kart vertex lands up to
/// a couple of units apart between racers.
const MATCH_TOLERANCE: f32 = 3.0;
/// Static triangles need this many other racers sharing all three corners.
pub const MIN_SHARED_STATIC: usize = 3;
/// Animated triangles need this many anywhere...
pub const MIN_SHARED_MOVING: usize = 8;
/// ...or this many when in front of the seat (`z > 0`, the steering wheel).
pub const MIN_SHARED_STEERING: usize = 5;

/// Which animation frame to export: the middle of `turn` (steering centred).
fn neutral_frame(mesh: &ModelMesh) -> (String, crate::ctr::Frame) {
    if let Some(a) = mesh
        .anims
        .iter()
        .find(|a| a.name.starts_with("turn"))
        .or(mesh.anims.first())
    {
        let i = a.frames.len() / 2;
        return (format!("{}[{i}]", a.name), a.frames[i].clone());
    }
    ("static".into(), mesh.frame.clone())
}

pub struct Racer {
    pub name: String,
    pub mesh: ModelMesh,
    pub frame_name: String,
    pub tris: Vec<Triangle>,
    /// Per triangle: no corner moves in any frame of the `turn` animation.
    pub is_static: Vec<bool>,
}

impl Racer {
    pub fn new(name: &str, mesh: ModelMesh) -> Result<Self> {
        let (frame_name, frame) = neutral_frame(&mesh);
        let tris = mesh.triangles(&frame)?;
        // Per raw vertex: does it move during `turn`?
        let mut moves = vec![false; frame.verts.len()];
        if let Some(turn) = mesh.anims.iter().find(|a| a.name.starts_with("turn")) {
            let base: Vec<[f32; 3]> = frame
                .verts
                .iter()
                .map(|v| mesh.position(&frame, *v))
                .collect();
            for f in &turn.frames {
                for (i, v) in f.verts.iter().enumerate().take(base.len()) {
                    let p = mesh.position(f, *v);
                    if (0..3).any(|k| (p[k] - base[i][k]).abs() > 1e-3) {
                        moves[i] = true;
                    }
                }
            }
        }
        let is_static = tris
            .iter()
            .map(|t| {
                t.vert_ids
                    .iter()
                    .all(|&i| !moves.get(i).copied().unwrap_or(true))
            })
            .collect();
        Ok(Self {
            name: name.into(),
            mesh,
            frame_name,
            tris,
            is_static,
        })
    }
}

pub fn load_racer(bf: &mut BigFile, name: &str) -> Result<Racer> {
    let id = big::CHARACTERS
        .iter()
        .position(|c| *c == name)
        .ok_or_else(|| {
            Error(format!(
                "unknown racer {name}; choose one of {}",
                KARTS.join(", ")
            ))
        })?;
    let model = Model::parse_file(&bf.read(big::index::RACER_HI + id)?)?;
    let mesh = model
        .meshes
        .into_iter()
        .next()
        .ok_or_else(|| Error(format!("{name}: no meshes")))?;
    Racer::new(name, mesh)
}

fn close(a: [f32; 3], b: [f32; 3]) -> bool {
    (0..3).all(|k| (a[k] - b[k]).abs() <= MATCH_TOLERANCE)
}

/// For each triangle of `racer`, how many of the `others` have vertices at all three of its
/// corners (anywhere in their mesh, so differing triangulation still matches).
pub fn shared_counts(racer: &Racer, others: &[&Racer]) -> Vec<usize> {
    let other_points: Vec<Vec<[f32; 3]>> = others
        .iter()
        .map(|o| o.tris.iter().flat_map(|t| t.positions).collect())
        .collect();
    racer
        .tris
        .iter()
        .map(|t| {
            other_points
                .iter()
                .filter(|pts| {
                    t.positions
                        .iter()
                        .all(|p| pts.iter().any(|q| close(*p, *q)))
                })
                .count()
        })
        .collect()
}

/// `true` per triangle of `racer` that belongs to the kart.
pub fn classify(racer: &Racer, others: &[&Racer]) -> Vec<bool> {
    shared_counts(racer, others)
        .into_iter()
        .zip(&racer.is_static)
        .zip(&racer.tris)
        .map(|((c, &st), t)| {
            let front = t.positions.iter().map(|p| p[2]).sum::<f32>() > 0.0;
            if st {
                c >= MIN_SHARED_STATIC
            } else {
                c >= MIN_SHARED_MOVING || (front && c >= MIN_SHARED_STEERING)
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Output schema
// ---------------------------------------------------------------------------------------------

#[derive(Serialize)]
pub struct KartFile {
    pub format: &'static str,
    pub version: u32,
    pub kart: String,
    pub source: Source,
    pub units: Units,
    pub axes: Axes,
    pub bounds: Bounds,
    pub anchors: Anchors,
    pub wheels: Vec<Wheel>,
    pub wheel_sprites: WheelSprites,
    pub surfaces: Vec<Surface>,
    pub textures: Vec<Texture>,
}

#[derive(Serialize)]
pub struct Source {
    pub game: &'static str,
    pub model: String,
    pub frame: String,
    pub triangles_total: usize,
    pub triangles_kart: usize,
}

#[derive(Serialize)]
pub struct Units {
    pub length: &'static str,
    pub ctr_units_per_meter: f32,
    pub ctr_instance_scale: f32,
}

#[derive(Serialize)]
pub struct Axes {
    pub up: &'static str,
    pub forward: &'static str,
    pub left: &'static str,
    pub handedness: &'static str,
    pub front_face: &'static str,
    pub origin: &'static str,
}

#[derive(Serialize)]
pub struct Bounds {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

#[derive(Serialize)]
pub struct Anchors {
    /// Where a seated driver's hips (pelvis) go.
    pub seat: [f32; 3],
    /// Centre of the steering-wheel ring (hands go on its rim).
    pub steering_wheel: [f32; 3],
    /// Exhaust outlets (left, right) where smoke puffs spawn; see `effects.json` for the
    /// turbo flame origins.
    pub exhaust: [[f32; 3]; 2],
}

#[derive(Serialize)]
pub struct Wheel {
    pub name: &'static str,
    pub center: [f32; 3],
    pub radius: f32,
    /// Front wheels steer (`DrawTires` rotates their rim vector by the steering angle).
    pub steers: bool,
}

#[derive(Serialize)]
pub struct WheelSprites {
    /// Texture indices of `tire00..tire16`.
    pub textures: Vec<usize>,
    pub rule: &'static str,
    /// Vertex colour the game modulates the sprites with (0x80 = 1.0).
    pub tint: [f32; 4],
}

#[derive(Serialize)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub color: [f32; 4],
}

#[derive(Serialize)]
pub struct Surface {
    /// Index into `textures`, or `null` for flat (Gouraud-only) triangles.
    pub texture: Option<usize>,
    /// `false`: back faces are culled (front faces wind counter-clockwise).
    pub double_sided: bool,
    /// `"opaque"` or `"blend"` (PSX semi-transparency; texels with alpha 128 blend 50/50).
    pub blend: &'static str,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

#[derive(Serialize, Clone)]
pub struct Texture {
    pub name: String,
    pub width: u32,
    pub height: u32,
    /// Row-major RGBA8, top row first.
    pub rgba: Vec<u8>,
}

// ---------------------------------------------------------------------------------------------

fn to_m(p: [f32; 3]) -> [f32; 3] {
    [
        p[0] / UNITS_PER_METER,
        p[1] / UNITS_PER_METER,
        p[2] / UNITS_PER_METER,
    ]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l > 1e-12 {
        [v[0] / l, v[1] / l, v[2] / l]
    } else {
        [0.0, 1.0, 0.0]
    }
}

/// A texture region shared by all triangles with the same page + CLUT.
struct TexGroup {
    layout: TexLayout,
    min: [u32; 2],
    max: [u32; 2],
}

/// Texture key: page/CLUT/depth/blend, ignoring UVs.
fn tex_key(t: &TexLayout) -> (u16, u16) {
    (t.tpage, t.clut)
}

/// Wheel placement from `DrawTires_Solid` (instance-local, in quarter world units, already
/// multiplied by the 0.8 instance scale there): centres at `x = ±0x90`, `y = 0x40`,
/// `z = 0xC7` (front) / `-0x60` (rear) times `scale / 4096 / 4`, sprite half-size
/// `wheelSize (0xCCC) * 4096 >> 18`. Dividing out the instance scale gives model units.
pub fn wheels() -> Vec<Wheel> {
    let unit = |v: f32| v / 4.0 / UNITS_PER_METER;
    let radius = (0xCCC as f32 * 4096.0 / 262144.0) / 4.0 / 0.8 / UNITS_PER_METER;
    let (x, y, zf, zr) = (unit(144.0), unit(64.0), unit(199.0), unit(-96.0));
    vec![
        Wheel {
            name: "front_left",
            center: [x, y, zf],
            radius,
            steers: true,
        },
        Wheel {
            name: "front_right",
            center: [-x, y, zf],
            radius,
            steers: true,
        },
        Wheel {
            name: "rear_left",
            center: [x, y, zr],
            radius,
            steers: false,
        },
        Wheel {
            name: "rear_right",
            center: [-x, y, zr],
            radius,
            steers: false,
        },
    ]
}

/// Builds the kart for `name` from the bigfile.
pub fn build(bf: &mut BigFile, name: &str) -> Result<KartFile> {
    build_with(bf, name, false)
}

/// Like [`build`]; `include_driver` keeps every triangle (for debugging the separation; with
/// the `CTR_CLASSIFY_COLORS` environment variable set, kart triangles are drawn green and
/// driver triangles red instead of textured).
pub fn build_with(bf: &mut BigFile, name: &str, include_driver: bool) -> Result<KartFile> {
    if !KARTS.contains(&name) {
        return Err(Error(format!(
            "unknown kart {name}; choose one of {}",
            KARTS.join(", ")
        )));
    }
    let mut racers = Vec::new();
    for r in REFERENCE {
        racers.push(load_racer(bf, r)?);
    }
    let chosen = match REFERENCE.iter().position(|r| *r == name) {
        Some(i) => i,
        None => {
            racers.push(load_racer(bf, name)?);
            racers.len() - 1
        }
    };
    // Classify every racer against the reference racers other than itself (and, for Fake
    // Crash, other than Crash, whose driver he shares).
    let twin = |a: &str, b: &str| a == b || (a == "fake" && b == "crash");
    let classes: Vec<Vec<bool>> = racers
        .iter()
        .map(|r| {
            let others: Vec<&Racer> = racers[..REFERENCE.len()]
                .iter()
                .filter(|o| !twin(&r.name, &o.name))
                .collect();
            classify(r, &others)
        })
        .collect();
    let anchors = anchors(&racers, &classes, chosen);

    let mut racer = racers.swap_remove(chosen);
    let is_kart = &classes[chosen];
    if include_driver && std::env::var_os("CTR_CLASSIFY_COLORS").is_some() {
        for (t, k) in racer.tris.iter_mut().zip(is_kart) {
            t.colors = [if *k {
                [40, 220, 60, 0]
            } else {
                [230, 40, 40, 0]
            }; 3];
            t.tex = None;
        }
    }

    let mut vram = Vram::new();
    vram.load_vrm(&bf.read(big::index::SHARED_VRM)?)?;
    let groups = mpk::icon_groups(&bf.read(big::index::SHARED_MPK)?)?;
    let tire = groups
        .iter()
        .find(|g| g.id == 0)
        .ok_or_else(|| Error("shared.mpk has no tire icon group (id 0)".into()))?;

    let mut k = assemble(&racer, is_kart, include_driver, &vram, &tire.icons, anchors)?;
    k.source.triangles_kart = is_kart.iter().filter(|k| **k).count();
    Ok(k)
}

fn assemble(
    racer: &Racer,
    is_kart: &[bool],
    include_driver: bool,
    vram: &Vram,
    tire_icons: &[mpk::Icon],
    anchors: Anchors,
) -> Result<KartFile> {
    let mesh = &racer.mesh;
    let kart_tris: Vec<&Triangle> = racer
        .tris
        .iter()
        .zip(is_kart)
        .filter(|(_, k)| **k || include_driver)
        .map(|(t, _)| t)
        .collect();
    if !include_driver {
        check_complete(&racer.name, &kart_tris)?;
    }

    // Texture groups over the kart's textured triangles.
    let mut tex_groups: Vec<TexGroup> = Vec::new();
    let mut tex_of_layout = vec![usize::MAX; mesh.tex_layouts.len()];
    for t in &kart_tris {
        let Some(li) = t.tex else { continue };
        let l = mesh.tex_layouts[li];
        let gi = match tex_groups
            .iter()
            .position(|g| tex_key(&g.layout) == tex_key(&l))
        {
            Some(gi) => gi,
            None => {
                tex_groups.push(TexGroup {
                    layout: l,
                    min: [u32::MAX; 2],
                    max: [0; 2],
                });
                tex_groups.len() - 1
            }
        };
        tex_of_layout[li] = gi;
        let g = &mut tex_groups[gi];
        for uv in &l.uv[..3] {
            for (k, &c) in uv.iter().enumerate() {
                g.min[k] = g.min[k].min(c as u32);
                g.max[k] = g.max[k].max(c as u32);
            }
        }
    }
    let mut textures: Vec<Texture> = tex_groups
        .iter()
        .map(|g| {
            let (w, h) = (g.max[0] - g.min[0] + 1, g.max[1] - g.min[1] + 1);
            Texture {
                name: format!(
                    "page{}_{}_clut{}_{}_{}bpp",
                    g.layout.page_x(),
                    g.layout.page_y(),
                    g.layout.clut_x(),
                    g.layout.clut_y(),
                    [4, 8, 16, 16][g.layout.bpp() as usize]
                ),
                width: w,
                height: h,
                rgba: vram.decode_rgba(&g.layout, g.min[0], g.min[1], w, h),
            }
        })
        .collect();

    // Surfaces keyed by (texture, double sided, blend).
    let mut surfaces: Vec<Surface> = Vec::new();
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    for t in &kart_tris {
        let tex = t.tex.map(|li| tex_of_layout[li]);
        let layout = t.tex.map(|li| mesh.tex_layouts[li]);
        let blend = match layout {
            Some(l) if l.semi_transparent() => "blend",
            _ => "opaque",
        };
        let double_sided = !t.cull;
        let si = match surfaces
            .iter()
            .position(|s| s.texture == tex && s.double_sided == double_sided && s.blend == blend)
        {
            Some(i) => i,
            None => {
                surfaces.push(Surface {
                    texture: tex,
                    double_sided,
                    blend,
                    vertices: vec![],
                    indices: vec![],
                });
                surfaces.len() - 1
            }
        };
        // The game's cull test keeps a face when its NCLIP sign, xor the `n` flag, matches the
        // instance's; for these models that makes the stored order (v0, v1, v2) clockwise
        // seen from the front in our right-handed, y-up space. We emit counter-clockwise
        // fronts, so swap two corners unless `n` is set. (Verified visually: culled renders
        // show outer surfaces, and Cortex's "N" reads unmirrored from the front.)
        let order: [usize; 3] = if t.flip { [0, 1, 2] } else { [0, 2, 1] };
        let p: Vec<[f32; 3]> = order.iter().map(|&i| to_m(t.positions[i])).collect();
        let n = normalize(cross(sub(p[1], p[0]), sub(p[2], p[0])));
        let s = &mut surfaces[si];
        let base = s.vertices.len() as u32;
        for (k, &i) in order.iter().enumerate() {
            let c = t.colors[i];
            let uv = match (layout, tex) {
                (Some(l), Some(gi)) => {
                    let g = &tex_groups[gi];
                    let w = (g.max[0] - g.min[0] + 1) as f32;
                    let h = (g.max[1] - g.min[1] + 1) as f32;
                    [
                        (l.uv[i][0] as f32 - g.min[0] as f32 + 0.5) / w,
                        (l.uv[i][1] as f32 - g.min[1] as f32 + 0.5) / h,
                    ]
                }
                _ => [0.0, 0.0],
            };
            // Textured: texel * colour / 128 (0x80 = neutral). Flat: colour / 255.
            let div = if tex.is_some() { 128.0 } else { 255.0 };
            let color = [c[0] as f32 / div, c[1] as f32 / div, c[2] as f32 / div, 1.0];
            for a in 0..3 {
                lo[a] = lo[a].min(p[k][a]);
                hi[a] = hi[a].max(p[k][a]);
            }
            s.vertices.push(Vertex {
                position: p[k],
                normal: n,
                uv,
                color,
            });
        }
        s.indices.extend([base, base + 1, base + 2]);
    }

    // Wheel sprites.
    let mut sprite_ids = Vec::new();
    for icon in tire_icons {
        let l = icon.layout;
        let (u0, v0) = (l.uv[0][0] as u32, l.uv[0][1] as u32);
        let (u1, v1) = (l.uv[3][0] as u32, l.uv[3][1] as u32);
        let (w, h) = (u1.saturating_sub(u0) + 1, v1.saturating_sub(v0) + 1);
        sprite_ids.push(textures.len());
        textures.push(Texture {
            name: icon.name.clone(),
            width: w,
            height: h,
            rgba: vram.decode_rgba(&l, u0, v0, w, h),
        });
    }

    Ok(KartFile {
        format: "ctr-kart",
        version: 1,
        kart: racer.name.clone(),
        source: Source {
            game: "Crash Team Racing (NTSC-U, SCUS-94426)",
            model: format!("{} ({})", racer.mesh.name, racer.name),
            frame: racer.frame_name.clone(),
            triangles_total: racer.tris.len(),
            triangles_kart: kart_tris.len(),
        },
        units: Units { length: "meters", ctr_units_per_meter: UNITS_PER_METER, ctr_instance_scale: 0.8 },
        axes: Axes {
            up: "+Y",
            forward: "+Z",
            left: "+X",
            handedness: "right",
            front_face: "ccw",
            origin: "kart centre on the ground plane (CTR driver instance origin)",
        },
        bounds: Bounds { min: lo, max: hi },
        anchors,
        wheels: wheels(),
        wheel_sprites: WheelSprites {
            textures: sprite_ids,
            rule: "sprite index = table[min(|dot(view_dir, axle)| * 128, 128)], 0 = edge-on (tread), 16 = side-on (full disc); see docs/ctr-assets.md",
            tint: [1.0, 1.0, 1.0, 1.0],
        },
        surfaces,
        textures,
    })
}

/// Sanity check of a separated kart: the surface area (model units²) of its triangles in four
/// regions must reach what every complete kart has. Regions by triangle centroid: nose
/// `z > 45`; hood/steering column `10 < z <= 45, y > 20`; floor `-20 < z <= 10 or y <= 20`
/// below `y < 15`; rear engine `z < -30`. Complete karts measure ~1700 / 750-1550 / 1450-3000 /
/// 3700-4150; racers whose hood is fitted to their body (N. Gin, Papu Papu, N. Tropy,
/// Dingodile) measure 0-30 for the hood.
pub fn check_complete(name: &str, kart: &[&Triangle]) -> Result<()> {
    let mut area = [0.0f32; 4];
    for t in kart {
        let p = t.positions;
        let a = 0.5 * {
            let c = cross(sub(p[1], p[0]), sub(p[2], p[0]));
            (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt()
        };
        let c = [0, 1, 2].map(|k| (p[0][k] + p[1][k] + p[2][k]) / 3.0);
        let region = if c[2] > 45.0 {
            0
        } else if c[2] > 10.0 && c[1] > 20.0 {
            1
        } else if c[2] > -20.0 && c[1] < 15.0 {
            2
        } else if c[2] < -30.0 {
            3
        } else {
            continue;
        };
        area[region] += a;
    }
    const NEED: [(&str, f32); 4] = [
        ("nose", 1000.0),
        ("hood", 500.0),
        ("floor", 1000.0),
        ("engine", 3000.0),
    ];
    for ((what, need), got) in NEED.iter().zip(area) {
        if got < *need {
            return Err(Error(format!(
                "{name}: separated kart is incomplete ({what} area {got:.0} < {need:.0})"
            )));
        }
    }
    Ok(())
}

fn centroid<'a>(pts: impl Iterator<Item = &'a [f32; 3]>) -> Option<[f32; 3]> {
    let mut sum = [0.0f32; 3];
    let mut n = 0.0;
    for p in pts {
        for k in 0..3 {
            sum[k] += p[k];
        }
        n += 1.0;
    }
    (n > 0.0).then(|| sum.map(|s| s / n))
}

fn median3(mut v: Vec<[f32; 3]>) -> Option<[f32; 3]> {
    if v.is_empty() {
        return None;
    }
    let mut out = [0.0; 3];
    for (k, o) in out.iter_mut().enumerate() {
        v.sort_by(|a, b| a[k].total_cmp(&b[k]));
        *o = v[v.len() / 2][k];
    }
    Some(out)
}

/// Seat: where the drivers' hips are. Per racer, the centroid of the driver's vertices in the
/// pelvis box above the seat pan (`y < 25`, `-25 < z < 8` model units, behind the knees and in
/// front of the seat back); the anchor is the per-axis median over all racers, since every
/// racer sits in the same kart. Steering wheel: the centroid of the chosen kart's animated
/// triangles in front of and above the seat (`z > 0`, `y > 25`: the wheel ring), or the
/// median over racers that have one.
fn anchors(racers: &[Racer], classes: &[Vec<bool>], chosen: usize) -> Anchors {
    let mut seats = Vec::new();
    let mut wheels = Vec::new();
    for (r, cls) in racers.iter().zip(classes) {
        let driver = r
            .tris
            .iter()
            .zip(cls)
            .filter(|(_, k)| !**k)
            .flat_map(|(t, _)| t.positions.iter());
        if let Some(c) = centroid(driver.filter(|p| p[1] < 25.0 && p[2] > -25.0 && p[2] < 8.0)) {
            seats.push(c);
        }
        let ring = r
            .tris
            .iter()
            .zip(cls)
            .zip(&r.is_static)
            .filter(|((t, k), st)| {
                let c = centroid(t.positions.iter()).unwrap_or_default();
                **k && !**st && c[2] > 0.0 && c[1] > 25.0
            })
            .flat_map(|((t, _), _)| t.positions.iter());
        wheels.push(centroid(ring));
    }
    let mut seat = median3(seats).unwrap_or([0.0, 15.0, -2.0]);
    seat[0] = 0.0;
    let mut wheel = wheels[chosen]
        .or_else(|| median3(wheels.iter().flatten().copied().collect()))
        .unwrap_or([0.0, 38.0, 25.0]);
    wheel[0] = 0.0;
    Anchors {
        seat: to_m(seat),
        steering_wheel: to_m(wheel),
        exhaust: crate::effects::exhaust_anchors(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tri(p: [[f32; 3]; 3]) -> Triangle {
        Triangle {
            vert_ids: [0, 1, 2],
            positions: p,
            colors: [[0; 4]; 3],
            tex: None,
            cull: true,
            flip: false,
        }
    }

    #[test]
    fn classification_uses_shared_geometry_and_motion() {
        let kart = tri([[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 10.0, 0.0]]);
        let driver = tri([[0.0, 50.0, 0.0], [10.0, 50.0, 0.0], [0.0, 60.0, 0.0]]);
        let racer = |name: String, tris: Vec<Triangle>, st: Vec<bool>| Racer {
            name,
            mesh: dummy_mesh(),
            frame_name: String::new(),
            tris,
            is_static: st,
        };
        // The kart triangle twice: once static (body), once animated (steering wheel).
        let me = racer(
            "a".into(),
            vec![kart, driver, kart],
            vec![true, true, false],
        );
        let mut others = Vec::new();
        for i in 0..MIN_SHARED_MOVING {
            // Same kart corners (requantized, other triangulation), different driver.
            let k = tri([[10.4, 0.0, 0.0], [0.0, 9.6, 0.3], [5.0, 5.0, 0.0]]);
            let k2 = tri([[0.2, 0.0, 0.0], [10.4, 0.0, 0.0], [5.0, 5.0, 0.0]]);
            let d = tri([
                [i as f32 * 5.0, 70.0, 0.0],
                [5.0, 80.0, 0.0],
                [0.0, 90.0, 0.0],
            ]);
            others.push(racer(format!("o{i}"), vec![k, k2, d], vec![true; 3]));
        }
        let refs: Vec<&Racer> = others.iter().collect();
        assert_eq!(classify(&me, &refs), vec![true, false, true]);
        // Static parts need fewer matches than moving ones.
        assert_eq!(
            classify(&me, &refs[..MIN_SHARED_STATIC]),
            vec![true, false, false]
        );
        assert_eq!(
            classify(&me, &refs[..MIN_SHARED_STATIC - 1]),
            vec![false, false, false]
        );
    }

    fn dummy_mesh() -> ModelMesh {
        ModelMesh {
            name: String::new(),
            flags: 0,
            scale: [4096; 3],
            commands: vec![],
            colors: vec![],
            tex_layouts: vec![],
            frame: crate::ctr::Frame {
                pos: [0; 3],
                verts: vec![],
            },
            anims: vec![],
        }
    }

    #[test]
    fn wheels_touch_the_ground() {
        for w in wheels() {
            assert!(
                (w.center[1] - w.radius).abs() < 1e-4,
                "{} {}",
                w.center[1],
                w.radius
            );
        }
    }
}
