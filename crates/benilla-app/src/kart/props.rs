//! World of Kartcraft's props (`props/props.json`): shapes the kart drives on that the world has
//! not, the hub's stunt park and the Skyhop course's floating platforms among them. Built into
//! the client, so everyone sees the same ones: each a box (a platform) or a ramp (a wedge rising
//! along its heading), drawn on the map it stands on and handed to the kart's collision
//! ([`kartcraft::session::Session::set_props`]). On foot they are scenery: the body's collision
//! is the world's. Client-side, as the turbo pads are: the server knows nothing of them.

use std::sync::{Arc, LazyLock};

use benilla_assets::coords::wow_to_bevy;
use benilla_world::world_map::CurrentMap;
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

/// One prop: a box or a ramp standing on `(x, y, z)` (WoW), `length` along the heading `o`,
/// `width` across, `height` up; a ramp rises from its back edge to its front.
#[derive(serde::Deserialize, Clone)]
pub(crate) struct Prop {
    pub(crate) map: u32,
    pub(crate) kind: Kind,
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) z: f32,
    pub(crate) o: f32,
    pub(crate) length: f32,
    pub(crate) width: f32,
    pub(crate) height: f32,
    pub(crate) colour: [f32; 3],
}

#[derive(serde::Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Kind {
    Box,
    Ramp,
}

/// A turbo pad of the file's (as a course's, [`super::pads`]).
#[derive(serde::Deserialize, Clone, Copy)]
pub(crate) struct PropPad {
    pub(crate) map: u32,
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) z: f32,
    pub(crate) o: f32,
}

#[derive(serde::Deserialize)]
pub(crate) struct File {
    pub(crate) props: Vec<Prop>,
    #[serde(default)]
    pub(crate) pads: Vec<PropPad>,
    #[serde(default)]
    pub(crate) challenges: Vec<super::skyhop::Challenge>,
}

pub(crate) static FILE: LazyLock<File> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../../props/props.json")).expect("props/props.json")
});

/// A prop's faces, each a quad or triangle of WoW points, and the shade each takes (1 a top, less
/// for the sides and under it).
fn faces(p: &Prop) -> Vec<(Vec<[f32; 3]>, f32)> {
    let (f, l) = (
        Vec2::new(p.o.cos(), p.o.sin()),
        Vec2::new(-p.o.sin(), p.o.cos()),
    );
    let c = Vec2::new(p.x, p.y);
    let (hl, hw) = (p.length / 2.0, p.width / 2.0);
    let corner = |along: f32, across: f32, z: f32| {
        let q = c + f * along + l * across;
        [q.x, q.y, z]
    };
    let (z0, z1) = (p.z, p.z + p.height);
    let (bl, br, fl, fr) = (
        |z| corner(-hl, hw, z),
        |z| corner(-hl, -hw, z),
        |z| corner(hl, hw, z),
        |z| corner(hl, -hw, z),
    );
    match p.kind {
        Kind::Box => vec![
            (vec![bl(z1), br(z1), fr(z1), fl(z1)], 1.0),
            (vec![bl(z0), fl(z0), fr(z0), br(z0)], 0.45),
            (vec![fl(z0), fl(z1), fr(z1), fr(z0)], 0.75),
            (vec![br(z0), br(z1), bl(z1), bl(z0)], 0.75),
            (vec![bl(z0), bl(z1), fl(z1), fl(z0)], 0.65),
            (vec![fr(z0), fr(z1), br(z1), br(z0)], 0.65),
        ],
        Kind::Ramp => vec![
            // The slope, from the back edge on the ground to the front edge at the top.
            (vec![bl(z0), br(z0), fr(z1), fl(z1)], 1.0),
            (vec![fl(z0), fl(z1), fr(z1), fr(z0)], 0.75),
            (vec![bl(z0), fl(z0), fr(z0), br(z0)], 0.45),
            (vec![bl(z0), fl(z1), fl(z0)], 0.65),
            (vec![br(z0), fr(z0), fr(z1)], 0.65),
        ],
    }
}

/// A prop's collision triangles, Bevy world.
pub(crate) fn triangles(p: &Prop) -> Vec<[Vec3; 3]> {
    let mut out = Vec::new();
    for (face, _) in faces(p) {
        let v: Vec<Vec3> = face.iter().map(|&w| wow_to_bevy(w)).collect();
        for i in 1..v.len() - 1 {
            out.push([v[0], v[i], v[i + 1]]);
        }
    }
    out
}

/// A prop's mesh, unlit: each face its colour at its shade, double-sided.
fn mesh(p: &Prop) -> Mesh {
    let (mut pos, mut col, mut idx) = (Vec::new(), Vec::new(), Vec::new());
    for (face, shade) in faces(p) {
        let base = pos.len() as u32;
        let [r, g, b] = p.colour.map(|c| (c * shade).min(1.0));
        for &w in &face {
            pos.push(wow_to_bevy(w).to_array());
            col.push(LinearRgba::from(Color::srgb(r, g, b)).to_f32_array());
        }
        for i in 1..face.len() as u32 - 1 {
            idx.extend([base, base + i, base + i + 1]);
        }
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );
    let normals = vec![[0.0, 1.0, 0.0]; pos.len()];
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
    mesh.insert_indices(Indices::U32(idx));
    mesh
}

/// The props drawn, for the map they are of, and their faces for the kart.
#[derive(Resource, Default)]
pub(crate) struct Laid {
    map: Option<u32>,
    entities: Vec<Entity>,
    faces: kartcraft::session::Props,
}

/// Draws the map's props, and takes the last map's down; hands their faces to the kart.
pub(crate) fn lay(
    mut commands: Commands,
    map: Option<Res<CurrentMap>>,
    mut laid: ResMut<Laid>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut host: ResMut<super::Host>,
) {
    let now = map.map(|m| m.0);
    if laid.map != now {
        for e in laid.entities.drain(..) {
            commands.entity(e).despawn();
        }
        let here: Vec<&Prop> = FILE.props.iter().filter(|p| Some(p.map) == now).collect();
        let material = materials.add(StandardMaterial {
            base_color: Color::WHITE,
            unlit: true,
            double_sided: true,
            cull_mode: None,
            ..default()
        });
        laid.entities = here
            .iter()
            .map(|p| {
                commands
                    .spawn((
                        Mesh3d(meshes.add(mesh(p))),
                        MeshMaterial3d(material.clone()),
                        Transform::default(),
                        Name::new("kart prop"),
                    ))
                    .id()
            })
            .collect();
        laid.faces = Arc::new(here.iter().flat_map(|p| triangles(p)).collect());
        laid.map = now;
        info!("kart props: {} on map {now:?}", here.len());
    }
    host.session.set_props(laid.faces.clone());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prop(kind: Kind) -> Prop {
        Prop {
            map: 1,
            kind,
            x: 100.0,
            y: 50.0,
            z: 10.0,
            o: 0.0,
            length: 20.0,
            width: 8.0,
            height: 4.0,
            colour: [1.0, 0.5, 0.0],
        }
    }

    #[test]
    fn a_box_and_a_ramp_have_their_shapes() {
        let b = triangles(&prop(Kind::Box));
        assert_eq!(b.len(), 12);
        let top = b.iter().flatten().map(|v| v.y).fold(f32::MIN, f32::max);
        assert!(
            (top - 14.0).abs() < 1e-4,
            "the box's top at z + height: {top}"
        );
        // The ramp: its back edge on the ground, its front edge at the top, along +x (north).
        let r = faces(&prop(Kind::Ramp));
        let slope = &r[0].0;
        assert_eq!(slope[0], [90.0, 54.0, 10.0]);
        assert_eq!(slope[2], [110.0, 46.0, 14.0]);
        assert_eq!(triangles(&prop(Kind::Ramp)).len(), 8);
    }

    #[test]
    fn the_shipped_props_parse_and_stand_on_their_maps() {
        assert!(!FILE.props.is_empty());
        for p in &FILE.props {
            assert!(p.length > 0.0 && p.width > 0.0 && p.height > 0.0);
            assert!(p.map <= 1);
        }
        assert!(FILE.challenges.iter().any(|c| c.key == "skyhop"));
    }
}
