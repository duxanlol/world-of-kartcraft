//! The game world for the website's replay viewer (`worldofkartcraft-site`, `/replay/<key>`): a
//! player's own 1.12.1 MPQs, picked in the browser and read in place, nothing uploaded and nothing
//! of the game's shipped with the site. [`World`] is the plain-Rust core over a [`Chain`]: a map's
//! folder by id (`Map.dbc`), the terrain tiles a course spans (its WDT), a tile's chunks as flat
//! mesh arrays with their texture layers, alpha and shadow maps and water, its trees and props
//! (M2) and buildings (WMO) as placements, a model's submeshes, and a texture as RGBA.
//! The `wasm` module binds it for a Web Worker, where `FileReaderSync` reads a picked `File` by
//! byte range, synchronously, as [`benilla_mpq::Source`] asks.

use anyhow::{Context, Result};
use benilla_formats::{
    load_m2_mesh, load_map_catalog, load_tile_mesh, load_wmo, Chain, MapCatalog, MapTiles,
    ModelBlend, TILE_SIZE,
};
use glam::{Mat3, Mat4, Quat, Vec3};

#[cfg(target_arch = "wasm32")]
mod wasm;

/// One MCNK, flat: WoW-yard positions (`x, y, z` each), normals, 0..1 uvs, triangle indices.
#[derive(Debug, Clone)]
pub struct Chunk {
    pub positions: Vec<f32>,
    pub normals: Vec<f32>,
    pub uvs: Vec<f32>,
    pub indices: Vec<u32>,
    /// Its MTEX layers, layer 0 first (up to 4).
    pub layers: Vec<String>,
    /// RGBA, `ALPHA_MAP_SIZE`², R/G/B the weights of layers 1/2/3; `None` with one layer.
    pub alpha: Option<Vec<u8>>,
    /// The baked shadow, a byte a texel over `SHADOW_MAP_SIZE`² (255 shadowed).
    pub shadow: Option<Vec<u8>>,
}

/// One liquid surface over a chunk: positions (`x, y, z`), triangle indices, and whether it is
/// the sea (drawn darker and wider than a lake).
#[derive(Debug, Clone)]
pub struct Water {
    pub positions: Vec<f32>,
    pub indices: Vec<u32>,
    pub ocean: bool,
    pub magma: bool,
}

/// A model placed in the world: its path, its `uniqueId` (the same in every tile it straddles), and
/// its model-to-world matrix in the render frame (`x` east, `y` up, `z` south), column-major, for
/// a mesh whose positions are WoW model coords taken into that frame as the ground's are.
#[derive(Debug, Clone)]
pub struct Placement {
    pub model: String,
    pub unique_id: u32,
    pub matrix: [f32; 16],
}

#[derive(Debug, Clone, Default)]
pub struct Tile {
    pub chunks: Vec<Chunk>,
    pub water: Vec<Water>,
    /// Trees, rocks and props (M2).
    pub doodads: Vec<Placement>,
    /// Buildings (WMO).
    pub wmos: Vec<Placement>,
}

/// One submesh of a model: WoW model-space positions and normals, uvs, indices, its texture, and
/// how it draws: 0 opaque, 1 cut out at half alpha, 2 blended.
#[derive(Debug, Clone)]
pub struct Submesh {
    pub positions: Vec<f32>,
    pub normals: Vec<f32>,
    pub uvs: Vec<f32>,
    pub indices: Vec<u32>,
    pub texture: Option<String>,
    pub blend: u8,
    pub two_sided: bool,
}

/// WoW `[x, y, z]` into the render frame, as the game's `benilla_assets::coords::wow_to_bevy`.
fn wow_to_render(p: [f32; 3]) -> Vec3 {
    Vec3::new(-p[1], p[2], -p[0])
}

/// An MDDF/MODF Euler rotation (degrees) in the render frame: the game's
/// `benilla_assets::coords::placement_rotation`, wowdev.wiki's ADT `createPlacementMatrix`,
/// `Rx(90°)·Ry(ry−180°)·Rz(−rx)·Rx(rz−90°)` in the WoW frame, conjugated into the render frame.
fn placement_rotation(rotation_deg: [f32; 3]) -> Quat {
    use std::f32::consts::{FRAC_PI_2, PI};
    let (rx, ry, rz) = (
        rotation_deg[0].to_radians(),
        rotation_deg[1].to_radians(),
        rotation_deg[2].to_radians(),
    );
    let in_wow = Quat::from_rotation_x(FRAC_PI_2)
        * Quat::from_rotation_y(ry - PI)
        * Quat::from_rotation_z(-rx)
        * Quat::from_rotation_x(rz - FRAC_PI_2);
    let to_render = Quat::from_mat3(&Mat3::from_cols(
        Vec3::new(0.0, 0.0, -1.0),
        Vec3::new(-1.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
    ));
    to_render * in_wow * to_render.inverse()
}

fn placement(
    model: &str,
    unique_id: u32,
    position: [f32; 3],
    rotation: [f32; 3],
    scale: f32,
) -> Placement {
    let m = Mat4::from_scale_rotation_translation(
        Vec3::splat(scale),
        placement_rotation(rotation),
        wow_to_render(position),
    );
    Placement {
        model: model.to_string(),
        unique_id,
        matrix: m.to_cols_array(),
    }
}

/// A texture's largest level no wider than asked, RGBA8.
#[derive(Debug, Clone)]
pub struct Texture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub struct World {
    chain: Chain,
    maps: Option<MapCatalog>,
    tiles: Option<(String, MapTiles)>,
}

impl World {
    pub fn new(chain: Chain) -> Self {
        World {
            chain,
            maps: None,
            tiles: None,
        }
    }

    /// A map's folder under `World\Maps` by its `Map.dbc` id ("Azeroth", "Kalimdor", ...).
    pub fn map_directory(&mut self, map_id: u32) -> Result<Option<String>> {
        if self.maps.is_none() {
            self.maps = Some(load_map_catalog(&mut self.chain)?);
        }
        Ok(self
            .maps
            .as_ref()
            .and_then(|m| m.directory(map_id))
            .map(str::to_string))
    }

    /// The map's existing ADT tiles `(x, y)` under a world rectangle (`min`, `max` as `x, y`).
    pub fn tiles_in(&mut self, map: &str, min: [f32; 2], max: [f32; 2]) -> Result<Vec<(u32, u32)>> {
        if self.tiles.as_ref().is_none_or(|(m, _)| m != map) {
            self.tiles = Some((map.to_string(), MapTiles::load(&mut self.chain, map)?));
        }
        let Some((_, tiles)) = self.tiles.as_ref() else {
            return Ok(Vec::new());
        };
        let (ax, ay) = tiles.tile_at(min[0], min[1]);
        let (bx, by) = tiles.tile_at(max[0], max[1]);
        let (cx, cy) = ((min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0);
        let span = (max[0] - min[0]).abs().max((max[1] - min[1]).abs());
        let radius = (span / TILE_SIZE).ceil() as u32 + 1;
        let (lo_x, hi_x) = (ax.min(bx), ax.max(bx));
        let (lo_y, hi_y) = (ay.min(by), ay.max(by));
        Ok(tiles
            .existing_in_radius(cx, cy, radius)
            .into_iter()
            .filter(|&(x, y)| (lo_x..=hi_x).contains(&x) && (lo_y..=hi_y).contains(&y))
            .collect())
    }

    /// Tile `(x, y)` of `map`, meshed.
    pub fn tile(&mut self, map: &str, x: u32, y: u32) -> Result<Tile> {
        let mesh = load_tile_mesh(&mut self.chain, map, x, y)?;
        let mut tile = Tile::default();
        for d in &mesh.doodads {
            tile.doodads.push(placement(
                &d.model,
                d.unique_id,
                d.position,
                d.rotation,
                d.scale,
            ));
        }
        for w in &mesh.wmos {
            tile.wmos.push(placement(
                &w.model,
                w.unique_id,
                w.position,
                w.rotation,
                1.0,
            ));
        }
        for c in mesh.chunks {
            for l in &c.liquids {
                tile.water.push(Water {
                    positions: l.positions.iter().flatten().copied().collect(),
                    indices: l.indices.clone(),
                    ocean: matches!(l.kind, benilla_formats::LiquidKind::Ocean),
                    magma: l.kind.is_fullbright(),
                });
            }
            let mut layers = c.layer_textures;
            if layers.is_empty() {
                layers.extend(c.base_texture);
            }
            tile.chunks.push(Chunk {
                positions: c.positions.iter().flatten().copied().collect(),
                normals: c.normals.iter().flatten().copied().collect(),
                uvs: c.uvs.iter().flatten().copied().collect(),
                indices: c.indices,
                layers,
                alpha: c.alpha_map,
                shadow: c.shadow,
            });
        }
        Ok(tile)
    }

    /// A model's submeshes: an M2 (a doodad) or, `wmo`, a WMO root with its groups.
    pub fn model(&mut self, path: &str, wmo: bool) -> Result<Vec<Submesh>> {
        let subs = if wmo {
            load_wmo(&mut self.chain, path)?
        } else {
            load_m2_mesh(&mut self.chain, path)?
        };
        Ok(subs
            .into_iter()
            .filter(|s| !s.indices.is_empty())
            .map(|s| Submesh {
                positions: s.positions.iter().flatten().copied().collect(),
                normals: s.normals.iter().flatten().copied().collect(),
                uvs: s.uvs.iter().flatten().copied().collect(),
                indices: s.indices,
                texture: s.texture,
                blend: match s.blend {
                    ModelBlend::Opaque => 0,
                    ModelBlend::AlphaTest => 1,
                    _ => 2,
                },
                two_sided: s.two_sided,
            })
            .collect())
    }

    /// A BLP texture by its internal path, at its largest level no wider than `max_width`.
    pub fn texture(&mut self, path: &str, max_width: u32) -> Result<Texture> {
        let bytes = self.chain.read_file(path)?;
        let blp = benilla_blp::decode(&bytes).with_context(|| format!("decoding {path}"))?;
        let level = blp
            .mips
            .iter()
            .find(|m| m.width <= max_width.max(1))
            .or(blp.mips.last())
            .with_context(|| format!("{path} has no levels"))?;
        Ok(Texture {
            width: level.width,
            height: level.height,
            rgba: level.rgba.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Elwynn by Goldshire (a course's span): the tiles under it, one meshed with its textures.
    #[test]
    fn a_course_span_reads_as_tiles_meshes_and_textures() {
        let data = benilla_formats::wow_data_or_skip!();
        let mut w = World::new(Chain::open(&data).unwrap());
        assert_eq!(w.map_directory(0).unwrap().as_deref(), Some("Azeroth"));
        let tiles = w
            .tiles_in("Azeroth", [-9806.0, -879.0], [-9082.0, 417.0])
            .unwrap();
        assert!(!tiles.is_empty() && tiles.len() <= 9, "{tiles:?}");
        let (x, y) = tiles[0];
        let tile = w.tile("Azeroth", x, y).unwrap();
        assert_eq!(tile.chunks.len(), 256);
        let c = &tile.chunks[0];
        assert_eq!(c.positions.len(), 145 * 3);
        assert_eq!(c.uvs.len(), 145 * 2);
        assert!(!c.indices.is_empty() && !c.layers.is_empty());
        let tex = w.texture(&c.layers[0], 256).unwrap();
        assert!(tex.width <= 256 && tex.rgba.len() == (tex.width * tex.height * 4) as usize);
        // Elwynn's trees: placed, and the first one's model read with a texture.
        let tile = tiles
            .iter()
            .map(|&(x, y)| w.tile("Azeroth", x, y).unwrap())
            .find(|t| !t.doodads.is_empty())
            .expect("a tile with doodads");
        let d = &tile.doodads[0];
        assert!(d.matrix.iter().all(|v| v.is_finite()));
        let subs = w.model(&d.model, false).unwrap();
        assert!(!subs.is_empty() && subs[0].positions.len().is_multiple_of(3));
        assert!(subs.iter().any(|s| s.texture.is_some()));
    }

    /// A placement turned only about the up axis keeps the model upright: its up stays up.
    #[test]
    fn a_heading_only_placement_stands_upright() {
        let p = placement("x", 1, [0.0, 0.0, 0.0], [0.0, 37.0, 0.0], 1.0);
        let m = Mat4::from_cols_array(&p.matrix);
        // A model's WoW up (+Z) into the render frame is +Y; it stays +Y after the placement.
        let up = m.transform_vector3(wow_to_render([0.0, 0.0, 1.0]));
        assert!((up - Vec3::Y).length() < 1e-4, "{up}");
    }
}
