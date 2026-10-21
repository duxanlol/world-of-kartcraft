//! The track editor's placeable objects: models of the player's own install ([`KINDS`]), placed
//! with a turn in all three axes ([`Placed`]), drawn unlit with a shade by their normals, and
//! solid: each is a static collider in the world, so the kart's gathered world drives on it, the
//! aim lands on it and a body stands on it. Each kind's icon is drawn once on the CPU from its
//! own mesh and textures (an orthographic three-quarter view), for the editor's objects panel.
//! The editor's draft holds what is placed; [`lay`] keeps the world in step with it, the object
//! being dragged or about to be placed shown as a see-through ghost with no collider.

use std::collections::HashMap;

use avian3d::prelude::{Collider, RigidBody};
use benilla_assets::coords::{bevy_to_wow, wow_rotation_to_bevy, wow_to_bevy};
use benilla_assets::{LockRecover, WorldAssets};
use benilla_formats::{Chain, ModelBlend, RenderSubmesh};
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

use super::track_edit::TrackEdit;

/// A kind of object: its key in a saved track, its name and group in the objects panel, and its
/// model in the install.
pub(crate) struct Kind {
    pub(crate) key: &'static str,
    pub(crate) name: &'static str,
    pub(crate) group: &'static str,
    path: &'static str,
}

/// The turbo pad's key: no model of the install, CTR's pad as [`kartcraft::pads`] draws it, laid
/// and fired by the pad field (`kart/pads.rs`) on the track it is placed on, never a collider.
pub(crate) const TURBO_PAD: &str = "turbo-pad";

/// Whether `key` is the turbo pad's ([`TURBO_PAD`]).
pub(crate) fn is_pad(key: &str) -> bool {
    key == TURBO_PAD
}

/// The arrow sign's key: no model of the install, a blue panel drawn here ([`sign_part`]) whose
/// chevrons slide along it, pointing the way it is yawed.
pub(crate) const ARROW_SIGN: &str = "arrow-sign";

/// The kinds, in the panel's order: the planks of the prop catalog's ramps (`prop-catalog`), then
/// the turbo pad and the arrow sign.
pub(crate) const KINDS: [Kind; 12] = [
    Kind {
        key: "redridge-dock-plank",
        name: "Dock plank",
        group: "Ramps",
        path: "World\\Azeroth\\RedRidge\\PassiveDoodads\\DockPieces\\RedRidgeDocksPlank01.m2",
    },
    Kind {
        key: "excavation-plank",
        name: "Plank",
        group: "Ramps",
        path: "World\\Generic\\Dwarf\\Passive Doodads\\ExcavationBarrierPlank\\ExcavationBarrierPlank.m2",
    },
    Kind {
        key: "excavation-plank-2",
        name: "Plank pair",
        group: "Ramps",
        path: "World\\Generic\\Dwarf\\Passive Doodads\\ExcavationBarrierPlank\\ExcavationBarrierPlank02.m2",
    },
    Kind {
        key: "excavation-plank-2b",
        name: "Plank ramp",
        group: "Ramps",
        path: "World\\Generic\\Dwarf\\Passive Doodads\\ExcavationBarrierPlank\\ExcavationBarrierPlank02b.m2",
    },
    Kind {
        key: "excavation-plank-2c",
        name: "Long plank ramp",
        group: "Ramps",
        path: "World\\Generic\\Dwarf\\Passive Doodads\\ExcavationBarrierPlank\\ExcavationBarrierPlank02c.m2",
    },
    Kind {
        key: "excavation-plank-3",
        name: "Plank trio",
        group: "Ramps",
        path: "World\\Generic\\Dwarf\\Passive Doodads\\ExcavationBarrierPlank\\ExcavationBarrierPlank03.m2",
    },
    Kind {
        key: "excavation-plank-3b",
        name: "Wide plank ramp",
        group: "Ramps",
        path: "World\\Generic\\Dwarf\\Passive Doodads\\ExcavationBarrierPlank\\ExcavationBarrierPlank03b.m2",
    },
    Kind {
        key: "excavation-plank-3c",
        name: "Long wide ramp",
        group: "Ramps",
        path: "World\\Generic\\Dwarf\\Passive Doodads\\ExcavationBarrierPlank\\ExcavationBarrierPlank03c.m2",
    },
    Kind {
        key: "booty-bay-plank",
        name: "Booty Bay plank",
        group: "Ramps",
        path: "World\\wmo\\Azeroth\\Buildings\\Stranglethorn_BootyBay\\BootyBayPlank.wmo",
    },
    Kind {
        key: "pirate-gangplank",
        name: "Gangplank",
        group: "Ramps",
        path: "World\\wmo\\Dungeon\\MD_Pirateship\\Pirateship_plank.wmo",
    },
    Kind {
        key: TURBO_PAD,
        name: "Turbo pad",
        group: "Boosts",
        path: "",
    },
    Kind {
        key: ARROW_SIGN,
        name: "Arrow sign",
        group: "Signs",
        path: "",
    },
];

/// CTR's props as `ctr-extract --props` writes them from the player's own disc
/// (`kart-data/props.json`, `ctr_assets::props`): meters, +Y up, the kart's own frame.
#[derive(serde::Deserialize)]
struct CtrProps {
    textures: Vec<CtrTexture>,
    props: Vec<CtrProp>,
}

#[derive(serde::Deserialize)]
struct CtrTexture {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

#[derive(serde::Deserialize)]
struct CtrProp {
    name: String,
    group: String,
    surfaces: Vec<CtrSurface>,
}

#[derive(serde::Deserialize)]
struct CtrSurface {
    texture: Option<usize>,
    blend: String,
    vertices: Vec<CtrVertex>,
    indices: Vec<u32>,
}

#[derive(serde::Deserialize)]
struct CtrVertex {
    position: [f32; 3],
    uv: [f32; 2],
}

/// The player's CTR props, read once; none without the file (an older conversion: the launcher
/// converts again when its converter changes).
static CTR_PROPS: std::sync::LazyLock<Option<CtrProps>> = std::sync::LazyLock::new(|| {
    let path = kartcraft::model::data_root()?.join("props.json");
    let bytes = std::fs::read(&path).ok()?;
    serde_json::from_slice(&bytes)
        .inspect_err(|e| warn!("kart objects: {}: {e}", path.display()))
        .ok()
});

/// A CTR kind's `path`: its place in `props.json`.
const CTR_PATH: &str = "ctr:";
/// A build piece's `path`: its key ([`super::pieces`]).
const PIECE_PATH: &str = "piece:";

/// The build piece `key` names, if it names one.
pub(crate) fn piece_of(
    key: &str,
) -> Option<(super::pieces::Shape, &'static super::pieces::Material)> {
    super::pieces::piece(key)
}

/// A build piece's parts at `size`: its faces in Bevy space, its material's texture tiled over them.
fn piece_parts(
    shape: super::pieces::Shape,
    m: &super::pieces::Material,
    size: [f32; 3],
    texture: Option<(u32, u32, Vec<u8>)>,
) -> Part {
    let g = super::pieces::geometry(shape, size, m.tile);
    Part {
        pos: g.pos.iter().map(|p| wow_to_bevy(*p)).collect(),
        normals: g.normals.iter().map(|n| wow_to_bevy(*n)).collect(),
        uvs: g.uvs,
        idx: g.idx,
        texture,
        blend: ModelBlend::Opaque,
    }
}

/// A CTR prop's name as the objects panel shows it: `crate_question` → `Crate question`.
fn ctr_name(name: &str) -> String {
    let words = name.replace('_', " ");
    let mut c = words.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

/// Every kind: [`KINDS`], then the player's CTR props by group (`CTR Hazards`, `CTR Pickups`,
/// `CTR Scenery`), keyed `ctr-<name>`.
pub(crate) fn kinds() -> &'static [Kind] {
    static ALL: std::sync::LazyLock<Vec<Kind>> = std::sync::LazyLock::new(|| {
        let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
        let own = KINDS.iter().map(|k| Kind {
            key: k.key,
            name: k.name,
            group: k.group,
            path: k.path,
        });
        let ctr = CTR_PROPS
            .iter()
            .flat_map(|f| f.props.iter().enumerate())
            .map(|(i, p)| Kind {
                key: leak(format!("ctr-{}", p.name)),
                name: leak(ctr_name(&p.name)),
                group: leak(format!("CTR {}", p.group)),
                path: leak(format!("{CTR_PATH}{i}")),
            });
        let pieces = super::pieces::Shape::ALL
            .into_iter()
            .flat_map(|shape| super::pieces::MATERIALS.iter().map(move |m| (shape, m)));
        let pieces = pieces.map(|(shape, m)| Kind {
            key: leak(format!("{}-{}", shape.key(), m.key)),
            name: leak(format!("{} {}", m.name, shape.name())),
            group: "Build",
            path: leak(format!("{PIECE_PATH}{}-{}", shape.key(), m.key)),
        });
        own.chain(pieces).chain(ctr).collect()
    });
    &ALL
}

/// The kind of `key`, and its index among [`kinds`].
pub(crate) fn kind(key: &str) -> Option<(usize, &'static Kind)> {
    kinds().iter().enumerate().find(|(_, k)| k.key == key)
}

/// A CTR prop's parts, in yards at the kart's own size (CTR's props stand to its kart as CTR
/// drew them), each face shaded by its normal.
fn ctr_parts(index: usize) -> Option<Vec<Part>> {
    let file = CTR_PROPS.as_ref()?;
    let prop = file.props.get(index)?;
    let to_yards = kartcraft::model_scale_at(1.0);
    let parts = prop
        .surfaces
        .iter()
        .filter(|s| !s.indices.is_empty())
        .map(|s| {
            let pos: Vec<Vec3> = s
                .vertices
                .iter()
                .map(|v| Vec3::from(v.position) * to_yards)
                .collect();
            // Each vertex's normal: its faces' summed.
            let mut normals = vec![Vec3::ZERO; pos.len()];
            for t in s.indices.as_chunks::<3>().0 {
                let [a, b, c] = t.map(|i| pos.get(i as usize).copied().unwrap_or_default());
                let n = (b - a).cross(c - a);
                for &i in t {
                    if let Some(v) = normals.get_mut(i as usize) {
                        *v += n;
                    }
                }
            }
            let texture = s.texture.and_then(|t| file.textures.get(t));
            Part {
                normals: normals.iter().map(|n| n.normalize_or(Vec3::Y)).collect(),
                uvs: s.vertices.iter().map(|v| v.uv).collect(),
                idx: s.indices.clone(),
                texture: texture.map(|t| (t.width, t.height, t.rgba.clone())),
                // PSX semi-transparency blends; an opaque surface cuts out its clear texels.
                blend: if s.blend == "opaque" {
                    ModelBlend::AlphaTest
                } else {
                    ModelBlend::Blend
                },
                pos,
            }
        })
        .collect();
    Some(parts)
}

/// A turn as one rotation, WoW space: `yaw` about the vertical (a gate's facing), then `pitch`
/// its front up, then `roll` about its length.
pub(crate) fn turn(yaw: f32, pitch: f32, roll: f32) -> Quat {
    Quat::from_rotation_z(yaw) * Quat::from_rotation_y(-pitch) * Quat::from_rotation_x(roll)
}

/// An object as placed: its kind, where its origin stands (WoW) and its turn (radians): `yaw`
/// about the vertical as a gate's facing, `pitch` its front up, `roll` about its length.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Placed {
    pub(crate) model: String,
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) z: f32,
    pub(crate) yaw: f32,
    #[serde(default)]
    pub(crate) pitch: f32,
    #[serde(default)]
    pub(crate) roll: f32,
    /// A build piece's length, width and height (yards, [`super::pieces`]); none for a model,
    /// whose size is its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) size: Option<[f32; 3]>,
}

impl Placed {
    /// Where it stands and how it is turned, Bevy world, the model baked in Bevy space.
    pub(crate) fn transform(&self) -> Transform {
        Transform {
            translation: wow_to_bevy([self.x, self.y, self.z]),
            rotation: wow_rotation_to_bevy(self.turn().to_array()),
            scale: Vec3::ONE,
        }
    }

    /// Its turn as one rotation, WoW space: yaw, then pitch, then roll.
    pub(crate) fn turn(&self) -> Quat {
        turn(self.yaw, self.pitch, self.roll)
    }

    /// Turned to `q` (WoW space), kept as its yaw, pitch and roll.
    pub(crate) fn set_turn(&mut self, q: Quat) {
        let (z, y, x) = q.normalize().to_euler(EulerRot::ZYX);
        (self.yaw, self.pitch, self.roll) = (z, -y, x);
    }

    /// The axes it turns about (Bevy world, unit): yaw about the vertical, pitch about its side
    /// as yawed, roll about its length as yawed and pitched.
    pub(crate) fn axes(&self) -> [Vec3; 3] {
        let yawed = Quat::from_rotation_z(self.yaw);
        let pitched = yawed * Quat::from_rotation_y(-self.pitch);
        let dir = |q: Quat, v: Vec3| wow_to_bevy((q * v).to_array()).normalize();
        [
            dir(Quat::IDENTITY, Vec3::Z),
            dir(yawed, Vec3::Y),
            dir(pitched, Vec3::X),
        ]
    }
}

/// Objects as a publish and a player track's line carry them:
/// `key,x,y,z,yaw,pitch,roll[,length,width,height];...`, the place at a tenth of a yard, the turn
/// at a thousandth of a radian and a build piece's size at a tenth of a yard (`Kartcraft.cpp`
/// `CleanTrackObjects`).
pub(crate) fn wire(objects: &[Placed]) -> String {
    let one = |o: &Placed| {
        let size = o
            .size
            .map_or_else(String::new, |[l, w, h]| format!(",{l:.1},{w:.1},{h:.1}"));
        format!(
            "{},{:.1},{:.1},{:.1},{:.3},{:.3},{:.3}{size}",
            o.model, o.x, o.y, o.z, o.yaw, o.pitch, o.roll
        )
    };
    objects.iter().map(one).collect::<Vec<_>>().join(";")
}

/// [`wire`] back into objects; one that is not numbers is passed over.
pub(crate) fn parse_wire(text: &str) -> Vec<Placed> {
    let one = |spot: &str| {
        let v: Vec<&str> = spot.split(',').collect();
        let n = |i: usize| v.get(i)?.trim().parse::<f32>().ok();
        ((v.len() == 7 || v.len() == 10) && !v[0].is_empty()).then_some(())?;
        let size = match v.len() {
            10 => Some([n(7)?, n(8)?, n(9)?]),
            _ => None,
        };
        Some(Placed {
            model: v[0].to_string(),
            x: n(1)?,
            y: n(2)?,
            z: n(3)?,
            yaw: n(4)?,
            pitch: n(5)?,
            roll: n(6)?,
            size,
        })
    };
    text.split(';').filter_map(one).collect()
}

/// A kind as loaded: its parts (mesh, material, the ghost's see-through material), its collider
/// and its icon.
pub(crate) struct Loaded {
    parts: Vec<(
        Handle<Mesh>,
        Handle<StandardMaterial>,
        Handle<StandardMaterial>,
    )>,
    collider: Option<Collider>,
    pub(crate) icon: Handle<Image>,
    /// Its box in its own frame (WoW yards about its origin), for snapping; none for the pad.
    bounds: Option<[Vec3; 2]>,
}

/// The kinds loaded so far, `None` for one that did not load.
#[derive(Resource, Default)]
pub(crate) struct Models(HashMap<&'static str, Option<Loaded>>);

impl Models {
    /// The icon of `key`, once loaded.
    pub(crate) fn icon(&self, key: &str) -> Option<Handle<Image>> {
        self.0.get(key)?.as_ref().map(|l| l.icon.clone())
    }

    /// `placed`'s box in its own frame (WoW yards): a build piece's at its size, a model's as
    /// loaded; none for the turbo pad or a kind not loaded.
    pub(crate) fn bounds(&self, placed: &Placed) -> Option<[Vec3; 2]> {
        if let Some((shape, m)) = piece_of(&placed.model) {
            let size =
                super::pieces::clamp_size(shape, placed.size.unwrap_or(shape.default_size()));
            return bounds_of(&[piece_parts(shape, m, size, None)]);
        }
        self.0.get(placed.model.as_str())?.as_ref()?.bounds
    }
}

/// Parts' box, back in WoW axes.
fn bounds_of(parts: &[Part]) -> Option<[Vec3; 2]> {
    let mut all = parts
        .iter()
        .flat_map(|p| &p.pos)
        .map(|v| Vec3::from(bevy_to_wow(*v)));
    let first = all.next()?;
    Some(all.fold([first, first], |[lo, hi], v| [lo.min(v), hi.max(v)]))
}

/// The objects a published track may have (`Kartcraft.cpp` `USER_TRACK_OBJECTS`); the editor
/// places more, which only its own saves keep.
pub(crate) const PUBLISH_OBJECTS: usize = 100;

/// How near (yards) a face must come to another's to snap onto it.
pub(crate) const SNAP_REACH: f32 = 1.5;
/// How near (degrees) a turn must be to another's, or to it a quarter turn about its vertical,
/// to take it.
pub(crate) const SNAP_ANGLE: f32 = 15.0;

/// A box's six face centres and their outward normals, turned and placed as `p` is (WoW).
fn faces(p: &Placed, [lo, hi]: [Vec3; 2]) -> [(Vec3, Vec3); 6] {
    let (q, at) = (p.turn(), Vec3::new(p.x, p.y, p.z));
    let mid = (lo + hi) / 2.0;
    std::array::from_fn(|i| {
        let (axis, out) = (i / 2, if i % 2 == 0 { -1.0 } else { 1.0 });
        let mut c = mid;
        c[axis] = if out < 0.0 { lo[axis] } else { hi[axis] };
        let mut n = Vec3::ZERO;
        n[axis] = out;
        (at + q * c, q * n)
    })
}

/// `ghost` snapped onto the nearest of `others`: turned as that one is when its turn is within
/// [`SNAP_ANGLE`] of it (or of it a quarter turn round), then moved so the nearest pair of
/// facing faces within [`SNAP_REACH`] meet. With where they meet (WoW), or none in reach.
pub(crate) fn snap(
    ghost: &Placed,
    bounds: [Vec3; 2],
    others: impl IntoIterator<Item = (Placed, [Vec3; 2])>,
) -> Option<(Placed, Vec3)> {
    let mut best: Option<(f32, Placed, Vec3)> = None;
    for (other, other_bounds) in others {
        let mut g = ghost.clone();
        let (gq, oq) = (ghost.turn(), other.turn());
        let quarter = |k: i32| oq * Quat::from_rotation_z(k as f32 * std::f32::consts::FRAC_PI_2);
        let near = (0..4)
            .map(quarter)
            .find(|q| gq.angle_between(*q) <= SNAP_ANGLE.to_radians());
        if let Some(q) = near {
            g.set_turn(q);
        }
        for (gp, gn) in faces(&g, bounds) {
            for (op, on) in faces(&other, other_bounds) {
                let d = gp.distance(op);
                let closer = best.as_ref().is_none_or(|(b, ..)| d < *b);
                if gn.dot(on) < -0.9 && d <= SNAP_REACH && closer {
                    let mut moved = g.clone();
                    let shift = op - gp;
                    (moved.x, moved.y, moved.z) = (g.x + shift.x, g.y + shift.y, g.z + shift.z);
                    best = Some((d, moved, op));
                }
            }
        }
    }
    best.map(|(_, p, at)| (p, at))
}

/// The side of an icon (px).
const ICON: u32 = 96;
/// How much a face turned away from the light darkens (0 none), drawn and in the icon.
const SHADE: f32 = 0.45;

/// A face's shade under a light from above and a little behind the viewer.
fn shade(normal: Vec3) -> f32 {
    let light = Vec3::new(0.35, 1.0, 0.55).normalize();
    1.0 - SHADE + SHADE * normal.normalize_or_zero().dot(light).max(0.0)
}

/// A submesh in Bevy space with its texture, as loaded from the install.
struct Part {
    pos: Vec<Vec3>,
    normals: Vec<Vec3>,
    uvs: Vec<[f32; 2]>,
    idx: Vec<u32>,
    texture: Option<(u32, u32, Vec<u8>)>,
    blend: ModelBlend,
}

fn read_parts(chain: &mut Chain, path: &str) -> anyhow::Result<Vec<Part>> {
    let subs: Vec<RenderSubmesh> = benilla_formats::load_object_model(chain, path)?;
    Ok(subs
        .into_iter()
        .filter(|s| !s.indices.is_empty())
        .map(|s| {
            let texture = s.texture.as_deref().and_then(|t| {
                let bytes = chain.read(t).ok()?;
                benilla_formats::blp_to_rgba(&bytes).ok()
            });
            Part {
                pos: s.positions.iter().map(|p| wow_to_bevy(*p)).collect(),
                normals: s.normals.iter().map(|n| wow_to_bevy(*n)).collect(),
                uvs: s.uvs,
                idx: s.indices,
                texture,
                blend: s.blend,
            }
        })
        .collect())
}

/// A part's mesh, its shade in the vertex colours (the material is unlit).
fn part_mesh(p: &Part) -> Mesh {
    let colors: Vec<[f32; 4]> = (0..p.pos.len())
        .map(|i| {
            let s = p.normals.get(i).map_or(1.0, |n| shade(*n));
            [s, s, s, 1.0]
        })
        .collect();
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );
    let normals = vec![[0.0, 1.0, 0.0]; p.pos.len()];
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        p.pos.iter().map(|v| v.to_array()).collect::<Vec<_>>(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, p.uvs.clone());
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(p.idx.clone()));
    mesh
}

/// The icon: the parts seen orthographically from the front-left and above, framed to fill it,
/// each texel shaded by its face; transparent where nothing is.
fn draw_icon(parts: &[Part], size: u32) -> Vec<u8> {
    let back = Vec3::new(-1.0, 0.8, -1.2).normalize();
    let right = Vec3::Y.cross(back).normalize();
    let up = back.cross(right);
    let all = parts.iter().flat_map(|p| &p.pos);
    let (lo, hi) = all.fold((Vec3::MAX, Vec3::MIN), |(lo, hi), v| {
        (lo.min(*v), hi.max(*v))
    });
    let centre = (lo + hi) * 0.5;
    let view = |v: Vec3| {
        let d = v - centre;
        Vec3::new(d.dot(right), d.dot(up), d.dot(back))
    };
    let corners = (0..8).map(|i| {
        let pick = |bit: u32, a: f32, b: f32| if i & bit == 0 { a } else { b };
        view(Vec3::new(
            pick(1, lo.x, hi.x),
            pick(2, lo.y, hi.y),
            pick(4, lo.z, hi.z),
        ))
    });
    let reach = corners
        .fold(0.0f32, |m, c| m.max(c.x.abs()).max(c.y.abs()))
        .max(1e-3);
    let n = size as f32;
    let scale = n * 0.46 / reach;
    let screen = |v: Vec3| Vec3::new(n * 0.5 + v.x * scale, n * 0.5 - v.y * scale, v.z);
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    let mut depth = vec![f32::MIN; (size * size) as usize];
    for p in parts {
        for t in p.idx.as_chunks::<3>().0 {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| i as usize);
            let world = [p.pos[a], p.pos[b], p.pos[c]];
            let normal = (world[1] - world[0]).cross(world[2] - world[0]);
            let normal = if normal.dot(back) < 0.0 {
                -normal
            } else {
                normal
            };
            let lit = shade(normal);
            let s = world.map(|v| screen(view(v)));
            let uv = [a, b, c].map(|i| p.uvs.get(i).copied().unwrap_or([0.0, 0.0]));
            let area =
                (s[1].x - s[0].x) * (s[2].y - s[0].y) - (s[2].x - s[0].x) * (s[1].y - s[0].y);
            if area.abs() < 1e-6 {
                continue;
            }
            let x0 = s
                .iter()
                .map(|v| v.x)
                .fold(f32::MAX, f32::min)
                .floor()
                .max(0.0) as u32;
            let x1 = s
                .iter()
                .map(|v| v.x)
                .fold(f32::MIN, f32::max)
                .ceil()
                .min(n - 1.0) as u32;
            let y0 = s
                .iter()
                .map(|v| v.y)
                .fold(f32::MAX, f32::min)
                .floor()
                .max(0.0) as u32;
            let y1 = s
                .iter()
                .map(|v| v.y)
                .fold(f32::MIN, f32::max)
                .ceil()
                .min(n - 1.0) as u32;
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                    let edge = |i: usize, j: usize| {
                        (s[j].x - s[i].x) * (py - s[i].y) - (s[j].y - s[i].y) * (px - s[i].x)
                    };
                    let w = [edge(1, 2), edge(2, 0), edge(0, 1)].map(|e| e / area);
                    if w.iter().any(|&e| e < 0.0) {
                        continue;
                    }
                    let z = w[0] * s[0].z + w[1] * s[1].z + w[2] * s[2].z;
                    let at = (y * size + x) as usize;
                    if z <= depth[at] {
                        continue;
                    }
                    let texel = match &p.texture {
                        Some((tw, th, tex)) => {
                            let u = w[0] * uv[0][0] + w[1] * uv[1][0] + w[2] * uv[2][0];
                            let v = w[0] * uv[0][1] + w[1] * uv[1][1] + w[2] * uv[2][1];
                            let tx = (u.rem_euclid(1.0) * *tw as f32) as u32 % tw;
                            let ty = (v.rem_euclid(1.0) * *th as f32) as u32 % th;
                            let o = ((ty * tw + tx) * 4) as usize;
                            [tex[o], tex[o + 1], tex[o + 2], tex[o + 3]]
                        }
                        None => [180, 180, 180, 255],
                    };
                    if p.blend != ModelBlend::Opaque && texel[3] < 128 {
                        continue;
                    }
                    depth[at] = z;
                    let o = at * 4;
                    for k in 0..3 {
                        rgba[o + k] = (texel[k] as f32 * lit) as u8;
                    }
                    rgba[o + 3] = 255;
                }
            }
        }
    }
    rgba
}

impl Models {
    /// `key` loaded from the install, once.
    fn load(
        &mut self,
        key: &'static str,
        chain: &mut Chain,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<StandardMaterial>,
        images: &mut Assets<Image>,
    ) -> Option<&Loaded> {
        self.0
            .entry(key)
            .or_insert_with(|| {
                let (_, kind) = kind(key)?;
                if is_pad(key) {
                    return Some(pad_kind(meshes, materials, images));
                }
                // A CTR prop from the player's own disc, a build piece of their install's
                // texture at its default size, else a model of their install.
                let ctr = kind
                    .path
                    .strip_prefix(CTR_PATH)
                    .and_then(|i| i.parse().ok());
                let parts = match (ctr, piece_of(key)) {
                    (Some(i), _) => ctr_parts(i)?,
                    (None, Some((shape, m))) => {
                        let texture = chain
                            .read(m.texture)
                            .ok()
                            .and_then(|b| benilla_formats::blp_to_rgba(&b).ok());
                        if texture.is_none() {
                            warn!("kart object {key}: no texture {}", m.texture);
                        }
                        vec![piece_parts(shape, m, shape.default_size(), texture)]
                    }
                    (None, None) if key == ARROW_SIGN => vec![sign_part()],
                    (None, None) => read_parts(chain, kind.path)
                        .inspect_err(|e| warn!("kart object {key}: {e:#}"))
                        .ok()?,
                };
                let mut verts = Vec::new();
                let mut tris = Vec::new();
                for p in &parts {
                    let base = verts.len() as u32;
                    verts.extend(&p.pos);
                    let t = p.idx.as_chunks::<3>().0.iter();
                    tris.extend(t.map(|t| [base + t[0], base + t[1], base + t[2]]));
                }
                let collider = (!tris.is_empty()).then(|| Collider::trimesh(verts, tris));
                let bounds = bounds_of(&parts);
                let icon = images.add(benilla_assets::sprite_image(
                    ICON,
                    ICON,
                    draw_icon(&parts, ICON),
                ));
                let parts = parts
                    .iter()
                    .map(|p| {
                        let texture = p.texture.as_ref().map(|(w, h, rgba)| {
                            images.add(benilla_assets::sprite_image_tiled(*w, *h, rgba.clone()))
                        });
                        let material = |ghost: bool| StandardMaterial {
                            base_color: Color::WHITE.with_alpha(if ghost { 0.55 } else { 1.0 }),
                            base_color_texture: texture.clone(),
                            unlit: true,
                            alpha_mode: match (ghost, p.blend) {
                                (true, _) => AlphaMode::Blend,
                                (false, ModelBlend::Opaque) => AlphaMode::Opaque,
                                (false, ModelBlend::AlphaTest) => AlphaMode::Mask(0.5),
                                _ => AlphaMode::Blend,
                            },
                            cull_mode: None,
                            double_sided: true,
                            ..default()
                        };
                        (
                            meshes.add(part_mesh(p)),
                            materials.add(material(false)),
                            materials.add(material(true)),
                        )
                    })
                    .collect();
                Some(Loaded {
                    parts,
                    collider,
                    icon,
                    bounds,
                })
            })
            .as_ref()
    }
}

/// The turbo pad as a kind: its flat chevrons as one part (the ghost's, while it is placed or
/// dragged; laid, the pad field draws it), no collider, and its icon.
fn pad_kind(
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
) -> Loaded {
    // Its own frame: centred, pointing along WoW +x, as a placed object's yaw turns it.
    let pad = kartcraft::pads::Pad::from_wow(0.0, 0.0, 0.0, 0.0);
    let mesh = kartcraft::pads::pad_mesh(&pad, |_| Some(0.0))
        .unwrap_or_else(|| Mesh::from(Plane3d::default()));
    let material = |alpha: f32| StandardMaterial {
        base_color: Color::WHITE.with_alpha(alpha),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        cull_mode: None,
        double_sided: true,
        ..default()
    };
    Loaded {
        parts: vec![(
            meshes.add(mesh),
            materials.add(material(1.0)),
            materials.add(material(0.55)),
        )],
        collider: None,
        icon: images.add(benilla_assets::sprite_image(ICON, ICON, pad_icon(ICON))),
        bounds: None,
    }
}

/// The turbo pad's icon: a dark green plate and three bright chevrons pointing up, as the pad.
fn pad_icon(size: u32) -> Vec<u8> {
    let n = size as f32;
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    for py in 0..size {
        for px in 0..size {
            let (x, y) = ((px as f32 + 0.5) / n, (py as f32 + 0.5) / n);
            let plate = (0.18..=0.82).contains(&x) && (0.06..=0.94).contains(&y);
            // A chevron's arms: |x - ½| rising as y falls, three of them a third apart.
            let arm = (0..3).any(|k| {
                let tip = 0.18 + 0.27 * k as f32;
                let d = (y - tip) - (x - 0.5).abs() * 0.9;
                (0.0..0.09).contains(&d) && (x - 0.5).abs() < 0.28
            });
            let colour = match (plate, arm) {
                (_, true) => [120, 255, 90, 255],
                (true, false) => [20, 70, 30, 230],
                _ => [0, 0, 0, 0],
            };
            let at = ((py * size + px) * 4) as usize;
            rgba[at..at + 4].copy_from_slice(&colour);
        }
    }
    rgba
}

/// The arrow sign's length, height and thickness (yards).
const SIGN_SIZE: [f32; 3] = [6.0, 2.0, 0.25];
/// How fast its chevrons slide along it (chevrons a second).
const SIGN_SPEED: f64 = 1.2;

/// The arrow sign: a thin panel standing on its origin, its length along WoW +x, both faces
/// tiled with chevrons pointing +x, its edges the frame's blue.
fn sign_part() -> Part {
    let [l, h, t] = SIGN_SIZE;
    let (l, t) = (l / 2.0, t / 2.0);
    let mut part = Part {
        pos: Vec::new(),
        normals: Vec::new(),
        uvs: Vec::new(),
        idx: Vec::new(),
        texture: Some((SIGN_TILE, SIGN_TILE, sign_texture(SIGN_TILE))),
        blend: ModelBlend::Opaque,
    };
    // Its faces lit as from above, so it reads alike from either side.
    let mut face = |corners: [[f32; 3]; 4], uvs: [[f32; 2]; 4], lit: Option<Vec3>| {
        let base = part.pos.len() as u32;
        let p = corners.map(wow_to_bevy);
        let normal = lit.unwrap_or_else(|| (p[1] - p[0]).cross(p[2] - p[0]));
        part.pos.extend(p);
        part.normals.extend([normal; 4]);
        part.uvs.extend(uvs);
        part.idx.extend([0, 1, 2, 0, 2, 3].map(|i| base + i));
    };
    // One chevron to a square of the face, `u` growing along +x on both sides.
    let tiled = [[-l / h, 1.0], [l / h, 1.0], [l / h, 0.0], [-l / h, 0.0]];
    for y in [-t, t] {
        let c = [[-l, y, 0.0], [l, y, 0.0], [l, y, h], [-l, y, h]];
        face(c, tiled, Some(Vec3::Y));
    }
    // The frame's row, the same at any `u` as the chevrons slide.
    let frame = [[0.0, 0.02]; 4];
    for z in [0.0, h] {
        face(
            [[-l, -t, z], [l, -t, z], [l, t, z], [-l, t, z]],
            frame,
            None,
        );
    }
    for x in [-l, l] {
        face(
            [[x, -t, 0.0], [x, t, 0.0], [x, t, h], [x, -t, h]],
            frame,
            None,
        );
    }
    part
}

/// The side of the arrow sign's texture (px): one chevron.
const SIGN_TILE: u32 = 64;

/// The arrow sign's texture: a blue plate framed darker above and below, a white chevron pointing
/// right (+u), tiling along `u`.
fn sign_texture(size: u32) -> Vec<u8> {
    let n = size as f32;
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    for py in 0..size {
        for px in 0..size {
            let (x, y) = ((px as f32 + 0.5) / n, (py as f32 + 0.5) / n);
            let frame = !(0.1..0.9).contains(&y);
            // The chevron's arms: its tip at the right, falling back as |y - ½| grows.
            let back = 0.72 - (y - 0.5).abs() * 0.8 - x;
            let arm = (0.0..0.2).contains(&back) && (y - 0.5).abs() < 0.32;
            let colour = match (frame, arm) {
                (true, _) => [12, 34, 110, 255],
                (false, true) => [240, 248, 255, 255],
                (false, false) => [36, 110, 235, 255],
            };
            let at = ((py * size + px) * 4) as usize;
            rgba[at..at + 4].copy_from_slice(&colour);
        }
    }
    rgba
}

/// A placed object in the world, the draft's `objects[i]`; its root carries the collider.
#[derive(Component)]
pub(crate) struct PlacedObject(pub(crate) usize);

/// The ghost: the object being dragged, or the one a click would place.
#[derive(Component)]
pub(crate) struct Ghost;

/// What is in the world: the objects as laid and their roots, and the ghost and its kind.
#[derive(Resource, Default)]
pub(crate) struct Laid {
    objects: Vec<(usize, Placed)>,
    roots: Vec<Entity>,
    ghost: Option<(String, Option<[f32; 3]>, Entity)>,
}

/// Spawns `placed` as `loaded`'s parts under one root; solid, unless a ghost.
fn spawn(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    placed: &Placed,
    loaded: &Loaded,
    ghost: bool,
) -> Entity {
    let mut root = commands.spawn((
        placed.transform(),
        Visibility::default(),
        Name::new(format!("kart object {}", placed.model)),
    ));
    // A build piece is built at its own size, its collider with it; its kind's part is the
    // material's.
    if let (Some((shape, m)), Some((_, solid, see_through))) =
        (piece_of(&placed.model), loaded.parts.first())
    {
        let size = super::pieces::clamp_size(shape, placed.size.unwrap_or(shape.default_size()));
        let part = piece_parts(shape, m, size, None);
        if !ghost {
            let tris = part.idx.as_chunks::<3>().0.to_vec();
            root.insert((RigidBody::Static, Collider::trimesh(part.pos.clone(), tris)));
        }
        let material = if ghost { see_through } else { solid };
        let mesh = meshes.add(part_mesh(&part));
        root.with_children(|r| {
            r.spawn((Mesh3d(mesh), MeshMaterial3d(material.clone())));
        });
        return root.id();
    }
    if !ghost {
        if let Some(c) = &loaded.collider {
            root.insert((RigidBody::Static, c.clone()));
        }
    }
    root.with_children(|r| {
        for (mesh, solid, see_through) in &loaded.parts {
            let material = if ghost { see_through } else { solid };
            r.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(material.clone())));
        }
    });
    root.id()
}

/// The install and the asset stores a kind is loaded into.
type Stores<'w> = (
    Option<Res<'w, WorldAssets>>,
    ResMut<'w, Assets<Mesh>>,
    ResMut<'w, Assets<StandardMaterial>>,
    ResMut<'w, Assets<Image>>,
);

/// Keeps the world's objects in step with the editor's draft, or the race's: every kind loaded
/// (for the icons) once the editor opens, the placed ones laid again when they change, the ghost
/// moved, the arrow signs' chevrons slid along.
pub(crate) fn lay(
    mut commands: Commands,
    (edit, races, time): (Res<TrackEdit>, Res<super::race::Races>, Res<Time>),
    mut laid: ResMut<Laid>,
    mut models: ResMut<Models>,
    (assets, mut meshes, mut materials, mut images): Stores,
    mut ghosts: Query<&mut Transform, With<Ghost>>,
) {
    let Some(assets) = assets else { return };
    // The draft's while editing, else the race's: a test lap's, or a player track's.
    let want: Vec<(usize, Placed)> = match (edit.view().state, &races.race) {
        ("edit", _) => edit.laid_objects(),
        (_, Some(race)) => race.objects().iter().cloned().enumerate().collect(),
        _ => Vec::new(),
    };
    // The turbo pads are the pad field's to lay and fire (`kart/pads.rs`).
    let want: Vec<(usize, Placed)> = want
        .into_iter()
        .filter(|(_, p)| !is_pad(&p.model))
        .collect();
    let ghost = edit.ghost();
    let mut chain = None;
    let mut load = |key: &'static str, models: &mut Models, meshes: &mut Assets<Mesh>| {
        let chain = chain.get_or_insert_with(|| assets.chain.lock_recover());
        models
            .load(key, chain, meshes, &mut materials, &mut images)
            .is_some()
    };
    if edit.view().state == "edit" && models.0.len() < kinds().len() {
        for k in kinds() {
            load(k.key, &mut models, &mut meshes);
        }
    }
    if laid.objects != want {
        for root in laid.roots.drain(..) {
            commands.entity(root).despawn();
        }
        for (i, placed) in &want {
            let Some((_, kind)) = kind(&placed.model) else {
                continue;
            };
            if !load(kind.key, &mut models, &mut meshes) {
                continue;
            }
            let loaded = models.0[kind.key].as_ref().unwrap();
            let root = spawn(&mut commands, &mut meshes, placed, loaded, false);
            commands.entity(root).insert(PlacedObject(*i));
            laid.roots.push(root);
        }
        laid.objects = want;
    }
    match (ghost, &laid.ghost) {
        (Some(g), Some((model, size, e))) if *model == g.model && *size == g.size => {
            if let Ok(mut t) = ghosts.get_mut(*e) {
                t.set_if_neq(g.transform());
            }
        }
        (g, _) => {
            if let Some((_, _, e)) = laid.ghost.take() {
                commands.entity(e).despawn();
            }
            let kind = g.as_ref().and_then(|g| kind(&g.model));
            if let (Some(g), Some((_, kind))) = (g, kind) {
                if load(kind.key, &mut models, &mut meshes) {
                    let loaded = models.0[kind.key].as_ref().unwrap();
                    let e = spawn(&mut commands, &mut meshes, &g, loaded, true);
                    commands.entity(e).insert(Ghost);
                    laid.ghost = Some((g.model, g.size, e));
                }
            }
        }
    }
    // Every sign shares its kind's materials: one slide moves them all.
    if let Some(Some(sign)) = models.0.get(ARROW_SIGN) {
        let along = -(time.elapsed_secs_f64() * SIGN_SPEED).fract() as f32;
        for (_, solid, see_through) in &sign.parts {
            for m in [solid, see_through] {
                if let Some(m) = materials.get_mut(m) {
                    m.uv_transform.translation.x = along;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placed(yaw: f32, pitch: f32, roll: f32) -> Placed {
        Placed {
            model: KINDS[0].key.into(),
            x: 10.0,
            y: 20.0,
            z: 5.0,
            yaw,
            pitch,
            roll,
            size: None,
        }
    }

    /// Where the model's local WoW point `p` lands, WoW.
    fn lands(placed: &Placed, p: [f32; 3]) -> [f32; 3] {
        benilla_assets::coords::bevy_to_wow(placed.transform().transform_point(wow_to_bevy(p)))
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-4)
    }

    #[test]
    fn a_placed_object_turns_about_its_origin_in_all_three_axes() {
        use std::f32::consts::FRAC_PI_2;
        // Unturned, its front (+x) is north of where it stands.
        assert!(close(
            lands(&placed(0.0, 0.0, 0.0), [1.0, 0.0, 0.0]),
            [11.0, 20.0, 5.0]
        ));
        // A quarter yaw: west (+y), as a gate's facing turns.
        assert!(close(
            lands(&placed(FRAC_PI_2, 0.0, 0.0), [1.0, 0.0, 0.0]),
            [10.0, 21.0, 5.0]
        ));
        // Pitched, its front rises; rolled, its left side does.
        assert!(close(
            lands(&placed(0.0, FRAC_PI_2, 0.0), [1.0, 0.0, 0.0]),
            [10.0, 20.0, 6.0]
        ));
        assert!(close(
            lands(&placed(0.0, 0.0, FRAC_PI_2), [0.0, 1.0, 0.0]),
            [10.0, 20.0, 6.0]
        ));
    }

    #[test]
    fn a_turn_kept_as_yaw_pitch_and_roll_is_the_same_turn() {
        let mut p = placed(0.4, -0.3, 0.2);
        let q = Quat::from_axis_angle(Vec3::new(1.0, 2.0, 0.5).normalize(), 0.7) * p.turn();
        p.set_turn(q);
        assert!(p.turn().angle_between(q) < 1e-4);
    }

    #[test]
    fn its_axes_turn_with_it_yaw_first() {
        use std::f32::consts::FRAC_PI_2;
        let near = |a: Vec3, b: Vec3| (a - b).length() < 1e-4;
        let [yaw, pitch, roll] = placed(0.0, 0.0, 0.0).axes();
        assert!(near(yaw, Vec3::Y) && near(pitch, wow_to_bevy([0.0, 1.0, 0.0])));
        assert!(near(roll, wow_to_bevy([1.0, 0.0, 0.0])));
        // A quarter yaw swings pitch and roll round with it; the yaw axis stays upright.
        let [yaw, pitch, roll] = placed(FRAC_PI_2, 0.0, 0.0).axes();
        assert!(near(yaw, Vec3::Y));
        assert!(near(pitch, wow_to_bevy([-1.0, 0.0, 0.0])));
        assert!(near(roll, wow_to_bevy([0.0, 1.0, 0.0])));
        // Pitched, the roll axis tilts up with its front.
        let [_, _, roll] = placed(0.0, FRAC_PI_2, 0.0).axes();
        assert!(near(roll, Vec3::Y));
    }

    #[test]
    fn objects_go_over_the_wire_and_back() {
        let objects = vec![placed(0.5, -0.25, 0.125), placed(0.0, 0.0, 0.0)];
        let text = wire(&objects);
        assert_eq!(
            text.split(';').next(),
            Some("redridge-dock-plank,10.0,20.0,5.0,0.500,-0.250,0.125")
        );
        assert_eq!(parse_wire(&text), objects);
        assert!(parse_wire("").is_empty());
        assert_eq!(
            parse_wire("plank,1,2,x,0,0,0;plank,1,2,3,0,0,0").len(),
            1,
            "a bad one passed over"
        );
        // A build piece's size goes after its turn; an object without one is as it was.
        let mut ramp = placed(0.0, 0.0, 0.0);
        ramp.model = "ramp-wood".into();
        ramp.size = Some([12.0, 8.0, 3.5]);
        let text = wire(std::slice::from_ref(&ramp));
        assert_eq!(
            text,
            "ramp-wood,10.0,20.0,5.0,0.000,0.000,0.000,12.0,8.0,3.5"
        );
        assert_eq!(parse_wire(&text), vec![ramp]);
        assert!(
            parse_wire("ramp-wood,1,2,3,0,0,0,1,2").is_empty(),
            "a size is whole"
        );
    }

    /// The build pieces are kinds, every shape in every material, in the Build tab, each
    /// material's texture in the install.
    #[test]
    fn every_build_piece_is_a_kind_with_its_texture_in_the_install() {
        let build: Vec<&Kind> = kinds().iter().filter(|k| k.group == "Build").collect();
        assert_eq!(
            build.len(),
            super::super::pieces::Shape::ALL.len() * super::super::pieces::MATERIALS.len()
        );
        assert!(
            build.iter().all(|k| piece_of(k.key).is_some()),
            "every one a piece"
        );
        assert_eq!(kind("block-grate").unwrap().1.name, "Grate block");
        let data = benilla_formats::wow_data_or_skip!();
        let chain = benilla_formats::open_chain(&data).expect("the install's archives");
        for m in &super::super::pieces::MATERIALS {
            let bytes = chain
                .read(m.texture)
                .unwrap_or_else(|e| panic!("{}: {e}", m.texture));
            let (w, h, _) = benilla_formats::blp_to_rgba(&bytes).expect("a texture");
            assert!(w >= 64 && h >= 64, "{} is {w}x{h}", m.key);
        }
    }

    #[test]
    fn every_kind_is_keyed_once_and_found_by_its_key() {
        for (i, k) in KINDS.iter().enumerate() {
            assert_eq!(kind(k.key).map(|(j, _)| j), Some(i), "{}", k.key);
        }
        assert!(kind("nothing").is_none());
    }

    #[test]
    fn an_icon_fills_its_middle_and_leaves_its_corners_clear() {
        // A unit cube's six faces, untextured.
        let c = [-0.5f32, 0.5];
        let pos: Vec<Vec3> = (0..8)
            .map(|i| Vec3::new(c[i & 1], c[(i >> 1) & 1], c[(i >> 2) & 1]))
            .collect();
        let idx = vec![
            0, 1, 3, 0, 3, 2, 4, 6, 7, 4, 7, 5, 0, 4, 5, 0, 5, 1, 2, 3, 7, 2, 7, 6, 0, 2, 6, 0, 6,
            4, 1, 5, 7, 1, 7, 3,
        ];
        let cube = Part {
            pos,
            normals: Vec::new(),
            uvs: Vec::new(),
            idx,
            texture: None,
            blend: ModelBlend::Opaque,
        };
        let rgba = draw_icon(&[cube], 32);
        let alpha = |x: u32, y: u32| rgba[((y * 32 + x) * 4 + 3) as usize];
        assert_eq!(alpha(16, 16), 255, "the cube in the middle");
        assert_eq!(alpha(0, 0), 0, "nothing in a corner");
        // Its faces are shaded apart: more than one grey in the icon.
        let greys: std::collections::HashSet<u8> = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[3] == 255)
            .map(|p| p[0])
            .collect();
        assert!(greys.len() >= 2, "{greys:?}");
    }

    /// The arrow sign is a kind of its own, drawn: a closed panel along +x, standing on its
    /// origin, whose chevron tiles along `u` and points +u (white right of blue along its middle).
    #[test]
    fn the_arrow_sign_is_a_panel_of_chevrons_pointing_along_it() {
        assert_eq!(kind(ARROW_SIGN).unwrap().1.group, "Signs");
        let part = sign_part();
        assert_eq!(part.pos.len(), 24, "six faces");
        let (lo, hi) = part.pos.iter().fold((Vec3::MAX, Vec3::MIN), |(lo, hi), v| {
            (lo.min(*v), hi.max(*v))
        });
        assert_eq!(lo.y, 0.0, "it stands on its origin");
        assert_eq!(hi.y, SIGN_SIZE[1]);
        // WoW +x is Bevy -z: its length.
        assert_eq!(hi.z - lo.z, SIGN_SIZE[0]);
        let tex = sign_texture(SIGN_TILE);
        let at = |x: u32, y: u32| tex[((y * SIGN_TILE + x) * 4) as usize];
        let mid = SIGN_TILE / 2;
        let row: Vec<u8> = (0..SIGN_TILE).map(|x| at(x, mid)).collect();
        let white = row.iter().position(|&r| r == 240).expect("a chevron");
        assert!(
            white as u32 > mid,
            "the chevron right of the middle: {white}"
        );
        assert_eq!(at(0, 0), at(SIGN_TILE - 1, 0), "the frame tiles");
        assert_eq!(at(0, 0), 12, "the frame row the edges show");
    }

    /// The player's CTR props (`kart-data/props.json`) are kinds after the install's, by group,
    /// each built from its own surfaces into textured triangles; without the file, none.
    #[test]
    fn the_ctr_props_are_kinds_with_textured_triangles() {
        let ctr: Vec<&Kind> = kinds()
            .iter()
            .filter(|k| k.path.starts_with(CTR_PATH))
            .collect();
        let build = kinds().iter().filter(|k| k.group == "Build").count();
        assert_eq!(kinds().len(), KINDS.len() + build + ctr.len());
        let Some(file) = CTR_PROPS.as_ref() else {
            assert!(ctr.is_empty(), "no props.json, no CTR kinds");
            // Pointed at a conversion, it must have its props.
            assert!(
                std::env::var_os("WOW_KART_DATA").is_none(),
                "WOW_KART_DATA has no props.json"
            );
            return;
        };
        assert_eq!(ctr.len(), file.props.len());
        for k in &ctr {
            assert!(
                k.key.starts_with("ctr-") && k.group.starts_with("CTR "),
                "{}",
                k.key
            );
            let i: usize = k.path[CTR_PATH.len()..].parse().unwrap();
            let parts = ctr_parts(i).unwrap_or_else(|| panic!("{}", k.key));
            let tris: usize = parts.iter().map(|p| p.idx.len() / 3).sum();
            assert!(tris > 0, "{} has no triangles", k.key);
            assert!(
                parts.iter().all(|p| p.pos.iter().all(|v| v.is_finite())),
                "{}",
                k.key
            );
        }
        // The ? crate is among them, a pickup, textured.
        let crate_q = kind("ctr-crate_question").expect("the ? crate");
        assert_eq!(crate_q.1.group, "CTR Pickups");
        assert_eq!(ctr_name("crate_question"), "Crate question");
    }

    #[test]
    fn every_kind_loads_from_the_install() {
        let data = benilla_formats::wow_data_or_skip!();
        let mut chain = benilla_formats::open_chain(&data).expect("the install's archives");
        // The turbo pad and the arrow sign are drawn, not read ([`pad_kind`], [`sign_part`]).
        for k in KINDS.iter().filter(|k| !k.path.is_empty()) {
            let parts =
                read_parts(&mut chain, k.path).unwrap_or_else(|e| panic!("{}: {e:#}", k.key));
            let tris: usize = parts.iter().map(|p| p.idx.len() / 3).sum();
            assert!(tris > 0, "{} has no triangles", k.key);
            assert!(
                parts.iter().any(|p| p.texture.is_some()),
                "{} is untextured",
                k.key
            );
        }
    }

    /// A 2-yard cube about its origin at `at`, turned.
    fn cube(at: [f32; 3], yaw: f32, pitch: f32) -> (Placed, [Vec3; 2]) {
        let p = Placed {
            model: "cube".into(),
            x: at[0],
            y: at[1],
            z: at[2],
            yaw,
            pitch,
            roll: 0.0,
            size: None,
        };
        (p, [Vec3::splat(-1.0), Vec3::ONE])
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    /// Near another's side, an object takes its turn and meets it face to face; on top, it
    /// stands on it; out of reach, it stays where it is.
    #[test]
    fn an_object_snaps_end_to_end_and_on_top_turned_as_its_neighbour() {
        let other = cube([0.0, 0.0, 0.0], 0.0, 0.0);
        let (ghost, b) = cube([2.8, 0.3, 0.0], 5f32.to_radians(), 0.0);
        let (p, meets) = snap(&ghost, b, [other.clone()]).expect("in reach");
        assert!(near(p.yaw, 0.0), "turned as it: {}", p.yaw);
        assert!(near(p.x, 2.0) && near(p.y, 0.0) && near(p.z, 0.0), "{p:?}");
        assert!(meets.distance(Vec3::X) < 1e-3, "{meets}");
        let (ghost, b) = cube([0.2, -0.1, 2.9], 0.0, 0.0);
        let (p, _) = snap(&ghost, b, [other.clone()]).expect("on top");
        assert!(near(p.z, 2.0) && near(p.x, 0.0), "{p:?}");
        let (ghost, b) = cube([6.0, 0.0, 0.0], 0.0, 0.0);
        assert!(snap(&ghost, b, [other]).is_none(), "out of reach");
    }

    /// A turn a quarter round from the neighbour's matches it; one between keeps its own and,
    /// its faces not facing, does not snap.
    #[test]
    fn a_quarter_turn_matches_and_a_turn_between_does_not() {
        let other = cube([0.0, 0.0, 0.0], 0.0, 0.0);
        let (ghost, b) = cube([2.6, 0.0, 0.0], 92f32.to_radians(), 0.0);
        let (p, _) = snap(&ghost, b, [other.clone()]).expect("a quarter round");
        assert!(
            near(p.yaw, std::f32::consts::FRAC_PI_2),
            "{}",
            p.yaw.to_degrees()
        );
        let (ghost, b) = cube([2.6, 0.0, 0.0], 45f32.to_radians(), 0.0);
        assert!(snap(&ghost, b, [other]).is_none());
    }

    /// A ramp pitched up chains on from its end, pitched as it: the climb goes on.
    #[test]
    fn a_pitched_ramp_chains_on_up_its_neighbour() {
        let pitch = 20f32.to_radians();
        let other = cube([0.0, 0.0, 0.0], 0.0, pitch);
        let end = other.0.turn() * Vec3::X;
        let at = end * 2.0 + Vec3::new(0.4, 0.0, 0.3);
        let (ghost, b) = cube(at.into(), 0.0, 22f32.to_radians());
        let (p, meets) = snap(&ghost, b, [other]).expect("in reach");
        assert!(near(p.pitch, pitch), "{}", p.pitch.to_degrees());
        assert!(Vec3::new(p.x, p.y, p.z).distance(end * 2.0) < 1e-3, "{p:?}");
        assert!(meets.distance(end) < 1e-3);
    }
}
