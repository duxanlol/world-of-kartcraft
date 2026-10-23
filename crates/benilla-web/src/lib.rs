//! The game world for the website's replay viewer (`worldofkartcraft-site`, `/replay/<key>`): a
//! player's own 1.12.1 MPQs, picked in the browser and read in place, nothing uploaded and nothing
//! of the game's shipped with the site. [`World`] is the plain-Rust core over a [`Chain`]: a map's
//! folder by id (`Map.dbc`), the terrain tiles a course spans (its WDT), a tile's chunks as flat
//! mesh arrays with their texture layers, alpha and shadow maps and water, and a texture as RGBA.
//! The `wasm` module binds it for a Web Worker, where `FileReaderSync` reads a picked `File` by
//! byte range, synchronously, as [`benilla_mpq::Source`] asks.

use anyhow::{Context, Result};
use benilla_formats::{load_map_catalog, load_tile_mesh, Chain, MapCatalog, MapTiles, TILE_SIZE};

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

#[derive(Debug, Clone, Default)]
pub struct Tile {
    pub chunks: Vec<Chunk>,
    pub water: Vec<Water>,
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
    }
}
