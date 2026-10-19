//! The track editor's placeable objects: models of the player's own install ([`KINDS`]), placed
//! with a turn in all three axes ([`Placed`]), drawn unlit with a shade by their normals, and
//! solid: each is a static collider in the world, so the kart's gathered world drives on it, the
//! aim lands on it and a body stands on it. Each kind's icon is drawn once on the CPU from its
//! own mesh and textures (an orthographic three-quarter view), for the editor's objects panel.
//! The editor's draft holds what is placed; [`lay`] keeps the world in step with it, the object
//! being dragged or about to be placed shown as a see-through ghost with no collider.

use std::collections::HashMap;

use avian3d::prelude::{Collider, RigidBody};
use benilla_assets::coords::{wow_rotation_to_bevy, wow_to_bevy};
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

/// The kinds, in the panel's order: the planks of the prop catalog's ramps (`prop-catalog`), then
/// the turbo pad.
pub(crate) const KINDS: [Kind; 11] = [
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
];

/// The kind of `key`, and its index.
pub(crate) fn kind(key: &str) -> Option<(usize, &'static Kind)> {
    KINDS.iter().enumerate().find(|(_, k)| k.key == key)
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
        Quat::from_rotation_z(self.yaw)
            * Quat::from_rotation_y(-self.pitch)
            * Quat::from_rotation_x(self.roll)
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

/// Objects as a publish and a player track's line carry them: `key,x,y,z,yaw,pitch,roll;...`,
/// the place at a tenth of a yard and the turn at a thousandth of a radian (`Kartcraft.cpp`
/// `CleanTrackObjects`).
pub(crate) fn wire(objects: &[Placed]) -> String {
    let one = |o: &Placed| {
        format!(
            "{},{:.1},{:.1},{:.1},{:.3},{:.3},{:.3}",
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
        (v.len() == 7 && !v[0].is_empty()).then_some(())?;
        Some(Placed {
            model: v[0].to_string(),
            x: n(1)?,
            y: n(2)?,
            z: n(3)?,
            yaw: n(4)?,
            pitch: n(5)?,
            roll: n(6)?,
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
}

/// The kinds loaded so far, `None` for one that did not load.
#[derive(Resource, Default)]
pub(crate) struct Models(HashMap<&'static str, Option<Loaded>>);

impl Models {
    /// The icon of `key`, once loaded.
    pub(crate) fn icon(&self, key: &str) -> Option<Handle<Image>> {
        self.0.get(key)?.as_ref().map(|l| l.icon.clone())
    }
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
                let parts = read_parts(chain, kind.path)
                    .inspect_err(|e| warn!("kart object {key}: {e:#}"))
                    .ok()?;
                let mut verts = Vec::new();
                let mut tris = Vec::new();
                for p in &parts {
                    let base = verts.len() as u32;
                    verts.extend(&p.pos);
                    let t = p.idx.as_chunks::<3>().0.iter();
                    tris.extend(t.map(|t| [base + t[0], base + t[1], base + t[2]]));
                }
                let collider = (!tris.is_empty()).then(|| Collider::trimesh(verts, tris));
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
    ghost: Option<(String, Entity)>,
}

/// Spawns `placed` as `loaded`'s parts under one root; solid, unless a ghost.
fn spawn(commands: &mut Commands, placed: &Placed, loaded: &Loaded, ghost: bool) -> Entity {
    let mut root = commands.spawn((
        placed.transform(),
        Visibility::default(),
        Name::new(format!("kart object {}", placed.model)),
    ));
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
/// moved.
pub(crate) fn lay(
    mut commands: Commands,
    (edit, races): (Res<TrackEdit>, Res<super::race::Races>),
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
    let mut load = |key: &'static str, models: &mut Models| {
        let chain = chain.get_or_insert_with(|| assets.chain.lock_recover());
        models
            .load(key, chain, &mut meshes, &mut materials, &mut images)
            .is_some()
    };
    if edit.view().state == "edit" && models.0.len() < KINDS.len() {
        for k in &KINDS {
            load(k.key, &mut models);
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
            if !load(kind.key, &mut models) {
                continue;
            }
            let loaded = models.0[kind.key].as_ref().unwrap();
            let root = spawn(&mut commands, placed, loaded, false);
            commands.entity(root).insert(PlacedObject(*i));
            laid.roots.push(root);
        }
        laid.objects = want;
    }
    match (ghost, &laid.ghost) {
        (Some(g), Some((model, e))) if *model == g.model => {
            if let Ok(mut t) = ghosts.get_mut(*e) {
                t.set_if_neq(g.transform());
            }
        }
        (g, _) => {
            if let Some((_, e)) = laid.ghost.take() {
                commands.entity(e).despawn();
            }
            let kind = g.as_ref().and_then(|g| kind(&g.model));
            if let (Some(g), Some((_, kind))) = (g, kind) {
                if load(kind.key, &mut models) {
                    let loaded = models.0[kind.key].as_ref().unwrap();
                    let e = spawn(&mut commands, &g, loaded, true);
                    commands.entity(e).insert(Ghost);
                    laid.ghost = Some((g.model, e));
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

    #[test]
    fn every_kind_loads_from_the_install() {
        let data = benilla_formats::wow_data_or_skip!();
        let mut chain = benilla_formats::open_chain(&data).expect("the install's archives");
        // The turbo pad is drawn, not read ([`pad_kind`]).
        for k in KINDS.iter().filter(|k| !is_pad(k.key)) {
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
}
