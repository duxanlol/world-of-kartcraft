//! The track editor's build pieces: a ramp and a block of any size, faced with a texture of the
//! player's own install that repeats by the yard (never stretched), to fill a gap in the world, bridge
//! a drop or lift a road onto a ledge. Each is an object ([`super::objects`]) keyed by its shape and
//! material (`ramp-wood`), its size kept with it ([`super::objects::Placed::size`]); its mesh and
//! its collider are built for that size, in its own frame (WoW axes: +x its length, +y its width,
//! +z up), standing on its origin.

/// A piece's shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Shape {
    /// A wedge rising along its length from nothing at its back (−x) to its height at its front.
    Ramp,
    /// A box, its top flat.
    Block,
}

impl Shape {
    pub(crate) const ALL: [Shape; 2] = [Shape::Ramp, Shape::Block];

    pub(crate) fn key(self) -> &'static str {
        match self {
            Shape::Ramp => "ramp",
            Shape::Block => "block",
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Shape::Ramp => "ramp",
            Shape::Block => "block",
        }
    }

    /// The size a new one is placed at: length, width, height (yards).
    pub(crate) fn default_size(self) -> [f32; 3] {
        match self {
            Shape::Ramp => [12.0, 8.0, 3.0],
            Shape::Block => [8.0, 8.0, 2.0],
        }
    }
}

/// A material: its key and name, its texture in the install, and how many yards one repeat of it
/// covers.
pub(crate) struct Material {
    pub(crate) key: &'static str,
    pub(crate) name: &'static str,
    pub(crate) texture: &'static str,
    pub(crate) tile: f32,
}

/// The materials, in the panel's order: tileable floors of the install.
pub(crate) const MATERIALS: [Material; 6] = [
    Material {
        key: "wood",
        name: "Wood",
        texture: "Dungeons\\Textures\\floor\\MM_WESTFALL_WOOD_FLOOR_01.blp",
        tile: 4.0,
    },
    Material {
        key: "cobble",
        name: "Cobble",
        texture: "Tileset\\Elwynn\\ElwynnCobbleStoneBase.blp",
        tile: 4.0,
    },
    Material {
        key: "dirt",
        name: "Dirt",
        texture: "Tileset\\Barrens\\BarrensRoad01.blp",
        tile: 4.0,
    },
    Material {
        key: "paving",
        name: "Paving",
        texture: "Tileset\\AraithiHighlands\\ArathiHighlandsCityFloor.blp",
        tile: 4.0,
    },
    Material {
        key: "grate",
        name: "Grate",
        texture: "Dungeons\\Textures\\metal\\BM_BRSPIRE_METALGRATE.blp",
        tile: 3.0,
    },
    Material {
        key: "stone",
        name: "Stone",
        texture: "Dungeons\\Textures\\floor\\BM_LDRN_COBBLESTONE01.blp",
        tile: 4.0,
    },
];

/// The smallest and largest a piece may be, each way (yards): the server's bounds too
/// (`Kartcraft.cpp` `CleanTrackObjects`).
pub(crate) const MIN_SIZE: f32 = 0.5;
pub(crate) const MAX_SIZE: f32 = 200.0;

/// `size` within the bounds; one not finite is the shape's default's.
pub(crate) fn clamp_size(shape: Shape, size: [f32; 3]) -> [f32; 3] {
    let d = shape.default_size();
    std::array::from_fn(|i| {
        let v = size[i];
        if v.is_finite() {
            v.clamp(MIN_SIZE, MAX_SIZE)
        } else {
            d[i]
        }
    })
}

/// The piece a key names (`ramp-wood`): its shape and material.
pub(crate) fn piece(key: &str) -> Option<(Shape, &'static Material)> {
    let (shape, material) = key.split_once('-')?;
    let shape = Shape::ALL.into_iter().find(|s| s.key() == shape)?;
    let material = MATERIALS.iter().find(|m| m.key == material)?;
    Some((shape, material))
}

/// A piece's geometry in its own frame (WoW axes, yards): each face its own vertices, so it shades
/// flat, its normal out, and its texture coordinates the face's yards over the material's tile.
#[derive(Debug, Default)]
pub(crate) struct Geometry {
    pub(crate) pos: Vec<[f32; 3]>,
    pub(crate) normals: Vec<[f32; 3]>,
    pub(crate) uvs: Vec<[f32; 2]>,
    pub(crate) idx: Vec<u32>,
}

impl Geometry {
    /// A flat face of `corners` (3 or 4, in order round it), its normal turned away from `inside`.
    fn face(&mut self, corners: &[[f32; 3]], inside: [f32; 3], tile: f32) {
        let v = |p: [f32; 3]| bevy::math::Vec3::from(p);
        let (a, b, c) = (v(corners[0]), v(corners[1]), v(corners[2]));
        let mut n = (b - a).cross(c - a).normalize_or(bevy::math::Vec3::Z);
        let centre = corners.iter().map(|&p| v(p)).sum::<bevy::math::Vec3>() / corners.len() as f32;
        if n.dot(centre - v(inside)) < 0.0 {
            n = -n;
        }
        // Mapped onto the face's plane by its two axes the normal leans least on.
        let (ua, va) = match (n.x.abs(), n.y.abs(), n.z.abs()) {
            (x, y, z) if z >= x && z >= y => (0, 1),
            (x, y, _) if x >= y => (1, 2),
            _ => (0, 2),
        };
        let base = self.pos.len() as u32;
        for &p in corners {
            self.pos.push(p);
            self.normals.push(n.to_array());
            self.uvs.push([p[ua] / tile, -p[va] / tile]);
        }
        let tris: &[u32] = if corners.len() == 4 {
            &[0, 1, 2, 0, 2, 3]
        } else {
            &[0, 1, 2]
        };
        self.idx.extend(tris.iter().map(|i| base + i));
    }
}

/// `shape` at `size` (length, width, height), standing on its origin, faced by a texture repeating
/// every `tile` yards.
pub(crate) fn geometry(shape: Shape, size: [f32; 3], tile: f32) -> Geometry {
    let [l, w, h] = size.map(|v| v.max(MIN_SIZE));
    let (x0, x1, y0, y1) = (-l / 2.0, l / 2.0, -w / 2.0, w / 2.0);
    let inside = [0.0, 0.0, h / 3.0];
    let mut g = Geometry::default();
    let tile = tile.max(0.1);
    match shape {
        Shape::Ramp => {
            // The slope, from its back edge on the ground to its front edge at its height.
            g.face(
                &[[x0, y0, 0.0], [x1, y0, h], [x1, y1, h], [x0, y1, 0.0]],
                inside,
                tile,
            );
            g.face(
                &[[x0, y0, 0.0], [x0, y1, 0.0], [x1, y1, 0.0], [x1, y0, 0.0]],
                inside,
                tile,
            );
            g.face(
                &[[x1, y0, 0.0], [x1, y1, 0.0], [x1, y1, h], [x1, y0, h]],
                inside,
                tile,
            );
            g.face(&[[x0, y0, 0.0], [x1, y0, 0.0], [x1, y0, h]], inside, tile);
            g.face(&[[x0, y1, 0.0], [x1, y1, h], [x1, y1, 0.0]], inside, tile);
        }
        Shape::Block => {
            let c = |x: f32, y: f32, z: f32| [x, y, z];
            g.face(
                &[c(x0, y0, h), c(x1, y0, h), c(x1, y1, h), c(x0, y1, h)],
                inside,
                tile,
            );
            g.face(
                &[
                    c(x0, y0, 0.0),
                    c(x0, y1, 0.0),
                    c(x1, y1, 0.0),
                    c(x1, y0, 0.0),
                ],
                inside,
                tile,
            );
            g.face(
                &[c(x0, y0, 0.0), c(x1, y0, 0.0), c(x1, y0, h), c(x0, y0, h)],
                inside,
                tile,
            );
            g.face(
                &[c(x0, y1, 0.0), c(x0, y1, h), c(x1, y1, h), c(x1, y1, 0.0)],
                inside,
                tile,
            );
            g.face(
                &[c(x0, y0, 0.0), c(x0, y0, h), c(x0, y1, h), c(x0, y1, 0.0)],
                inside,
                tile,
            );
            g.face(
                &[c(x1, y0, 0.0), c(x1, y1, 0.0), c(x1, y1, h), c(x1, y0, h)],
                inside,
                tile,
            );
        }
    }
    g
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_names_its_shape_and_material() {
        let (shape, m) = piece("ramp-cobble").unwrap();
        assert_eq!((shape, m.key), (Shape::Ramp, "cobble"));
        assert_eq!(piece("block-grate").unwrap().0, Shape::Block);
        assert!(piece("ramp-gold").is_none());
        assert!(piece("excavation-plank").is_none());
    }

    #[test]
    fn a_ramp_rises_along_its_length_and_its_faces_point_out() {
        let g = geometry(Shape::Ramp, [10.0, 4.0, 2.0], 4.0);
        assert_eq!(
            g.idx.len() / 3,
            2 + 2 + 2 + 1 + 1,
            "slope, bottom, front, two sides"
        );
        let top = g.pos.iter().map(|p| p[2]).fold(f32::MIN, f32::max);
        assert_eq!(top, 2.0);
        // Its back edge on the ground, its front at its height.
        assert!(g.pos.iter().any(|p| p[0] == -5.0 && p[2] == 0.0));
        assert!(g.pos.iter().all(|p| p[0] > -5.0 + 1e-3 || p[2] == 0.0));
        // Every face's normal points away from inside.
        let inside = bevy::math::Vec3::new(0.0, 0.0, 2.0 / 3.0);
        for t in g.idx.chunks(3) {
            let p = |i: u32| bevy::math::Vec3::from(g.pos[i as usize]);
            let centre = (p(t[0]) + p(t[1]) + p(t[2])) / 3.0;
            let n = bevy::math::Vec3::from(g.normals[t[0] as usize]);
            assert!(n.dot(centre - inside) > 0.0, "{t:?}");
        }
    }

    /// The texture repeats by the yard: a longer block has more repeats, not a stretched one.
    #[test]
    fn the_texture_repeats_by_the_yard() {
        let span = |l: f32| {
            let g = geometry(Shape::Block, [l, 4.0, 1.0], 4.0);
            let us = g.uvs.iter().map(|uv| uv[0]);
            us.clone().fold(f32::MIN, f32::max) - us.fold(f32::MAX, f32::min)
        };
        assert!((span(8.0) - 2.0).abs() < 1e-4);
        assert!((span(40.0) - 10.0).abs() < 1e-4);
    }

    #[test]
    fn sizes_are_bounded() {
        assert_eq!(
            clamp_size(Shape::Block, [0.0, 999.0, f32::NAN]),
            [MIN_SIZE, MAX_SIZE, 2.0]
        );
    }
}
