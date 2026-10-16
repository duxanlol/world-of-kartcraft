//! A zone's terrain as rasters over its bounding box, north up: height, road weight, water, slope,
//! the zone footprint, and the placed doodads and buildings as round obstacles.
//!
//! Pixel `(col, row)` covers world `x = north_x − row·res`, `y = west_y − col·res` (WoW +X north,
//! +Y west), so columns step east and rows step south, as MCNK rows and columns do.

use std::collections::{HashMap, HashSet};

use anyhow::{anyhow, bail, Context, Result};
use benilla_bytes::{chunks, ByteExt};
use benilla_formats::{
    adt_to_tile_mesh, load_area_table_catalog, load_map_catalog, load_world_map_area_catalog,
    world_to_chunk, AreaTableCatalog, Chain, ChunkMesh, MapCatalog, TileMesh, WorldMapArea,
    WorldMapAreaCatalog, CHUNK_SIZE, TILE_SIZE,
};
use serde::{Deserialize, Serialize};

/// The finest an instance's streets are rasterized, yards per pixel ([`crate::wmo_floor`]).
pub const STREET_YARDS_PER_PX: f32 = 1.0;

/// Raster resolution the tools default to, in yards per pixel.
pub const DEFAULT_YARDS_PER_PX: f32 = 2.0;

/// World yards from the map corner to its centre: 32 tiles.
const MAP_CENTER: f32 = 32.0 * TILE_SIZE;

/// Lower-cased substrings of a ground texture path that mark it as a road or path surface.
/// Elwynn and Westfall pave their roads with `ElwynnCobbleStoneBase` and `WestFallCobbleStoneBase`
/// (Redridge with `RedridgeRockRoadBase`); `ElwynnDirtBase2` and `WestFallDirtBase` also paint
/// shoulders, banks and open ground, so dirt is not a road.
pub const ROAD_KEYWORDS: &[&str] = &["road", "path", "cobble", "trail"];

/// Substrings that veto a [`ROAD_KEYWORDS`] match.
pub const ROAD_EXCLUDE: &[&str] = &[];

/// Whether a ground texture path is a road surface under `keywords`, vetoed by `exclude`.
pub fn is_road_texture(path: &str, keywords: &[String], exclude: &[String]) -> bool {
    let p = path.to_ascii_lowercase();
    keywords.iter().any(|k| p.contains(k.as_str()))
        && !exclude.iter().any(|k| p.contains(k.as_str()))
}

/// The raster's placement in the world.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct Grid {
    /// World x of the raster's north edge.
    pub north_x: f32,
    /// World y of the raster's west edge.
    pub west_y: f32,
    pub yards_per_px: f32,
    /// Columns (west to east).
    pub width: usize,
    /// Rows (north to south).
    pub height: usize,
}

impl Grid {
    /// World `(x, y)` of a continuous pixel coordinate; pixel centres are at `+0.5`.
    pub fn world_of(&self, col: f32, row: f32) -> (f32, f32) {
        (
            self.north_x - row * self.yards_per_px,
            self.west_y - col * self.yards_per_px,
        )
    }

    /// Continuous pixel `(col, row)` of a world `(x, y)`.
    pub fn pixel_of(&self, x: f32, y: f32) -> (f32, f32) {
        (
            (self.west_y - y) / self.yards_per_px,
            (self.north_x - x) / self.yards_per_px,
        )
    }

    /// The flat index of pixel `(col, row)`, `None` off the raster.
    pub fn index(&self, col: i64, row: i64) -> Option<usize> {
        (col >= 0 && row >= 0 && (col as usize) < self.width && (row as usize) < self.height)
            .then(|| row as usize * self.width + col as usize)
    }

    pub fn len(&self) -> usize {
        self.width * self.height
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// What an obstacle is.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ObstacleKind {
    /// A placed M2 (tree, rock, fence, prop).
    Doodad,
    /// A placed WMO (building, bridge, wall).
    Wmo,
}

/// A placed model as a vertical cylinder in world coordinates.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Obstacle {
    pub kind: ObstacleKind,
    pub model: String,
    /// Centre in WoW world yards: the doodad origin, or a WMO's MODF extents centre.
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// Horizontal radius in yards: the reach of an M2's collision hull near the ground times its
    /// scale (a tree's trunk, not its canopy), or a WMO's MODF extents half-diagonal.
    pub radius: f32,
    /// Whether a mover collides with it: an M2 with a non-empty collision box, every WMO.
    pub solid: bool,
}

/// One ground texture's share of the zone, in pixels of fully visible texture.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TextureCoverage {
    pub path: String,
    pub pixels: f64,
    pub is_road: bool,
}

/// Choices for [`load_zone`].
#[derive(Clone, Debug)]
pub struct TerrainParams {
    pub yards_per_px: f32,
    /// Yards of terrain kept around the zone's chunks.
    pub margin: f32,
    pub road_keywords: Vec<String>,
    pub road_exclude: Vec<String>,
    /// Read M2 headers for doodad radii; off, every doodad gets radius 1 and counts as solid.
    pub doodad_bounds: bool,
    /// Lower-cased substrings of texture paths to keep a weight raster for ([`ZoneTerrain::probes`]).
    pub probe_textures: Vec<String>,
    /// Drive on the walkable floor of the zone's WMOs as well as its terrain ([`crate::wmo_floor`]):
    /// bridges, tunnels and city streets, as an instance's streets always are.
    pub wmo_floors: bool,
}

impl Default for TerrainParams {
    fn default() -> Self {
        Self {
            yards_per_px: DEFAULT_YARDS_PER_PX,
            margin: 64.0,
            road_keywords: ROAD_KEYWORDS.iter().map(|s| s.to_string()).collect(),
            road_exclude: ROAD_EXCLUDE.iter().map(|s| s.to_string()).collect(),
            doodad_bounds: true,
            probe_textures: Vec::new(),
            wmo_floors: false,
        }
    }
}

/// A zone's terrain rasters (row-major over [`Grid`]) and obstacles.
pub struct ZoneTerrain {
    pub zone: String,
    pub area_id: u32,
    pub map_id: u32,
    /// The map's directory name (`Azeroth`).
    pub map: String,
    pub grid: Grid,
    /// Terrain height in yards; NaN in a hole or off the loaded tiles.
    pub height: Vec<f32>,
    /// Visible weight of road textures, 0..1.
    pub road: Vec<f32>,
    /// Liquid covers the pixel's MCLQ cell.
    pub water: Vec<bool>,
    /// The pixel's chunk belongs to the zone or one of its subzones.
    pub in_zone: Vec<bool>,
    /// Ground slope in degrees.
    pub slope: Vec<f32>,
    /// The pixel's chunk's AreaTable id (the subzone), 0 off the loaded tiles.
    pub area: Vec<u32>,
    /// Display names of the areas in [`ZoneTerrain::area`].
    pub area_names: HashMap<u32, String>,
    /// A solid model stands on the pixel at kart height: a doodad's collision hull cut between
    /// half a yard under and [`FOOTPRINT_HEIGHT`] over its base, or a building's footprint.
    pub solid: Vec<bool>,
    pub obstacles: Vec<Obstacle>,
    /// Every ground texture seen on the zone's chunks, most coverage first.
    pub textures: Vec<TextureCoverage>,
    /// One visible-weight raster per [`TerrainParams::probe_textures`] entry, in order.
    pub probes: Vec<Vec<f32>>,
    /// The ground is the walkable floor of the map's WMOs (an instance's streets,
    /// [`crate::wmo_floor`]), and `road` marks that floor.
    pub wmo_ground: bool,
    /// An open WMO floor is laid over the terrain on the pixel ([`TerrainParams::wmo_floors`]).
    pub laid: Vec<bool>,
    /// A WMO's wall stands on the pixel's ground (continent zones with laid floors): part of
    /// [`ZoneTerrain::solid`], kept apart for what must never be crossed.
    pub walls: Vec<bool>,
}

impl ZoneTerrain {
    /// Bilinear sample of `field` at world `(x, y)`, skipping NaN corners; `None` off the raster.
    fn bilinear(&self, field: &[f32], x: f32, y: f32) -> Option<f32> {
        let (c, r) = self.grid.pixel_of(x, y);
        let (c, r) = (c - 0.5, r - 0.5);
        let (c0, r0) = (c.floor() as i64, r.floor() as i64);
        let (fc, fr) = (c - c0 as f32, r - r0 as f32);
        let mut acc = 0.0;
        let mut wsum = 0.0;
        for (dc, dr, w) in [
            (0, 0, (1.0 - fc) * (1.0 - fr)),
            (1, 0, fc * (1.0 - fr)),
            (0, 1, (1.0 - fc) * fr),
            (1, 1, fc * fr),
        ] {
            if let Some(i) = self.grid.index(c0 + dc, r0 + dr) {
                let v = field[i];
                if v.is_finite() && w > 0.0 {
                    acc += v * w;
                    wsum += w;
                }
            }
        }
        (wsum > 0.0).then(|| acc / wsum)
    }

    fn nearest(&self, x: f32, y: f32) -> Option<usize> {
        let (c, r) = self.grid.pixel_of(x, y);
        self.grid.index(c.floor() as i64, r.floor() as i64)
    }

    /// Terrain height at world `(x, y)`, bilinear over the raster.
    pub fn height_at(&self, x: f32, y: f32) -> Option<f32> {
        self.bilinear(&self.height, x, y)
    }

    /// Road weight 0..1 at world `(x, y)`, bilinear over the raster.
    pub fn road_at(&self, x: f32, y: f32) -> Option<f32> {
        self.bilinear(&self.road, x, y)
    }

    /// Slope in degrees at world `(x, y)`, bilinear over the raster.
    pub fn slope_at(&self, x: f32, y: f32) -> Option<f32> {
        self.bilinear(&self.slope, x, y)
    }

    /// Whether a doodad's hull or a building stands on world `(x, y)`.
    pub fn solid_at(&self, x: f32, y: f32) -> Option<bool> {
        self.nearest(x, y).map(|i| self.solid[i])
    }

    /// Whether liquid covers world `(x, y)`.
    pub fn water_at(&self, x: f32, y: f32) -> Option<bool> {
        self.nearest(x, y).map(|i| self.water[i])
    }

    /// The area (subzone) id at world `(x, y)`.
    pub fn area_at(&self, x: f32, y: f32) -> Option<u32> {
        self.nearest(x, y).map(|i| self.area[i])
    }

    /// Whether world `(x, y)` is on the zone's own chunks.
    pub fn in_zone_at(&self, x: f32, y: f32) -> Option<bool> {
        self.nearest(x, y).map(|i| self.in_zone[i])
    }
}

/// One loaded MCNK and the top-level zone of its area.
struct LoadedChunk {
    mesh: ChunkMesh,
    top_zone: Option<u32>,
}

/// Open the install's patch chain: `$WOW_DATA`, else benilla's usual places.
pub fn open_install() -> Result<Chain> {
    let data = benilla_formats::wow_data().ok_or_else(|| {
        anyhow!(
            "no WoW install; set WOW_DATA (looked in {:?})",
            benilla_formats::candidates()
        )
    })?;
    Chain::open(&data).with_context(|| format!("opening {}", data.display()))
}

/// The zone named `zone` (any case) that has a world map: `AreaTable` can repeat a name across
/// maps, and only a zone with a `WorldMapArea` rect is bounded.
pub fn find_zone<'a>(
    areas: &AreaTableCatalog,
    wma: &'a WorldMapAreaCatalog,
    zone: &str,
) -> Result<(u32, &'a WorldMapArea)> {
    wma.iter()
        .filter(|(_, a)| a.area_id != 0)
        .find(|(_, a)| {
            areas
                .name(a.area_id)
                .is_some_and(|n| n.eq_ignore_ascii_case(zone))
        })
        .map(|(_, a)| (a.area_id, a))
        .ok_or_else(|| anyhow!("no zone named {zone:?} with a world map"))
}

/// The instance map named `name` (any case): a `Map.dbc` row with a non-zero `InstanceType`, a
/// dungeon such as "Stratholme" (map 329), which has no world map of its own in 1.12.
pub fn find_instance_map(maps: &MapCatalog, name: &str) -> Option<u32> {
    (0..MAX_MAP_ID).find(|&id| {
        maps.instance_type(id).is_some_and(|t| t != 0)
            && maps.name(id).is_some_and(|n| n.eq_ignore_ascii_case(name))
    })
}

/// The highest map id [`find_instance_map`] looks at; 1.12's run to 533.
const MAX_MAP_ID: u32 = 1024;

/// Load `zone` (an `AreaTable` name such as "Elwynn Forest", subzones included) and rasterize it.
/// A name with no world map that names an instance map ("Stratholme") loads that whole map: every
/// ADT tile its WDT has is the zone.
pub fn load_zone(chain: &mut Chain, zone: &str, params: &TerrainParams) -> Result<ZoneTerrain> {
    let areas = load_area_table_catalog(chain)?;
    // The world map's rect for the zone bounds which tiles to read; a tile past it is read too.
    let wma = load_world_map_area_catalog(chain)?;
    let maps = load_map_catalog(chain)?;
    let tile_of = |v: f32| ((MAP_CENTER - v) / TILE_SIZE).floor() as i64;
    let (zone_area, zone_name, map_id, (tx0, tx1), (ty0, ty1), whole_map) =
        match find_zone(&areas, &wma, zone) {
            Ok((zone_area, rect)) => {
                let zone_row = areas.get(zone_area).context("zone row")?;
                let (x_lo, x_hi) = minmax(rect.loc_top, rect.loc_bottom);
                let (y_lo, y_hi) = minmax(rect.loc_left, rect.loc_right);
                (
                    zone_area,
                    zone_row.name.clone(),
                    zone_row.map_id,
                    (tile_of(y_hi) - 1, tile_of(y_lo) + 1),
                    (tile_of(x_hi) - 1, tile_of(x_lo) + 1),
                    false,
                )
            }
            Err(e) => {
                let Some(map_id) = find_instance_map(&maps, zone) else {
                    return Err(e);
                };
                let name = maps.name(map_id).unwrap_or(zone).to_string();
                (0, name, map_id, (0, 63), (0, 63), true)
            }
        };
    let map = maps
        .directory(map_id)
        .ok_or_else(|| anyhow!("no Map.dbc row {map_id}"))?
        .to_string();

    let mut tiles: Vec<(TileMesh, Vec<ModfExtent>)> = Vec::new();
    for tx in tx0.max(0)..=tx1.min(63) {
        for ty in ty0.max(0)..=ty1.min(63) {
            let path = format!("World\\Maps\\{map}\\{map}_{tx}_{ty}.adt");
            let Ok(bytes) = chain.read_file(&path) else {
                continue; // no tile there
            };
            let mesh = adt_to_tile_mesh(&bytes).with_context(|| format!("meshing {path}"))?;
            tiles.push((mesh, read_modf_extents(&bytes)));
        }
    }
    if tiles.is_empty() {
        bail!("no ADT tiles under {zone:?}");
    }

    // Chunks keyed by their global chunk index, and the zone's bounding box from its own chunks.
    let mut by_chunk: HashMap<(u32, u32), LoadedChunk> = HashMap::new();
    let (mut bx0, mut bx1, mut by0, mut by1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for (tile, _) in &tiles {
        for c in &tile.chunks {
            let Some(&nw) = c.positions.first() else {
                continue;
            };
            let top_zone = areas.top_zone(c.area_id);
            if whole_map || top_zone == Some(zone_area) {
                bx1 = bx1.max(nw[0]);
                bx0 = bx0.min(nw[0] - CHUNK_SIZE);
                by1 = by1.max(nw[1]);
                by0 = by0.min(nw[1] - CHUNK_SIZE);
            }
            let centre = (nw[0] - CHUNK_SIZE / 2.0, nw[1] - CHUNK_SIZE / 2.0);
            by_chunk.insert(
                world_to_chunk(centre.0, centre.1),
                LoadedChunk {
                    mesh: c.clone(),
                    top_zone,
                },
            );
        }
    }
    if bx0 > bx1 {
        bail!("no chunk of the loaded tiles is in {zone:?}");
    }
    // A whole map's own area is the top zone most of its chunks name, so its subzones read as
    // places on it ("King's Square").
    let zone_area = if whole_map {
        let mut count: HashMap<u32, usize> = HashMap::new();
        for lc in by_chunk.values() {
            if let Some(z) = lc.top_zone {
                *count.entry(z).or_default() += 1;
            }
        }
        count
            .into_iter()
            .max_by_key(|&(z, n)| (n, std::cmp::Reverse(z)))
            .map_or(0, |(z, _)| z)
    } else {
        zone_area
    };

    // A city's alleys are a few yards wide: its streets are rasterized finer.
    let res = if whole_map {
        params.yards_per_px.min(STREET_YARDS_PER_PX)
    } else {
        params.yards_per_px
    };
    let north_x = bx1 + params.margin;
    let west_y = by1 + params.margin;
    let grid = Grid {
        north_x,
        west_y,
        yards_per_px: res,
        width: ((west_y - (by0 - params.margin)) / res).ceil() as usize,
        height: ((north_x - (bx0 - params.margin)) / res).ceil() as usize,
    };

    // Which of each chunk's layers are road, resolved once per texture name.
    let mut road_tex: HashMap<String, bool> = HashMap::new();
    let mut coverage: HashMap<String, f64> = HashMap::new();
    let n = grid.len();
    let mut height = vec![f32::NAN; n];
    let mut road = vec![0.0f32; n];
    let mut probes = vec![vec![0.0f32; n]; params.probe_textures.len()];
    let mut water = vec![false; n];
    let mut in_zone = vec![false; n];
    let mut area = vec![0u32; n];
    // Road weight is the mean of a 2×2 sub-sample: an alpha texel is half a yard.
    const SUB: [f32; 2] = [0.25, 0.75];
    for row in 0..grid.height {
        for col in 0..grid.width {
            let i = row * grid.width + col;
            let (x, y) = grid.world_of(col as f32 + 0.5, row as f32 + 0.5);
            let Some(lc) = by_chunk.get(&world_to_chunk(x, y)) else {
                continue;
            };
            let c = &lc.mesh;
            let zone_px = whole_map || lc.top_zone == Some(zone_area);
            in_zone[i] = zone_px;
            area[i] = c.area_id;
            height[i] = c.height_at([x, y, 0.0]).unwrap_or(f32::NAN);
            let nw = c.positions[0];
            let mut rsum = 0.0;
            for sr in SUB {
                for sc in SUB {
                    let (sx, sy) = grid.world_of(col as f32 + sc, row as f32 + sr);
                    let south = ((nw[0] - sx) / CHUNK_SIZE).clamp(0.0, 0.9999);
                    let east = ((nw[1] - sy) / CHUNK_SIZE).clamp(0.0, 0.9999);
                    let w = layer_weights(c, south, east);
                    for (li, tex) in c.layer_textures.iter().enumerate() {
                        let wl = w[li];
                        if wl <= 0.0 {
                            continue;
                        }
                        let is_road = *road_tex.entry(tex.clone()).or_insert_with(|| {
                            is_road_texture(tex, &params.road_keywords, &params.road_exclude)
                        });
                        if is_road {
                            rsum += wl;
                        }
                        if zone_px {
                            *coverage.entry(tex.clone()).or_default() += f64::from(wl) / 4.0;
                        }
                        for (k, probe) in params.probe_textures.iter().enumerate() {
                            if tex.to_ascii_lowercase().contains(probe.as_str()) {
                                probes[k][i] += wl / 4.0;
                            }
                        }
                    }
                }
            }
            road[i] = (rsum / 4.0).min(1.0);
            let south = ((nw[0] - x) / CHUNK_SIZE).clamp(0.0, 0.9999);
            let east = ((nw[1] - y) / CHUNK_SIZE).clamp(0.0, 0.9999);
            water[i] = liquid_at(c, south, east, height[i]);
        }
    }

    // An instance's streets are WMO faces over filler terrain: its ground is the walkable floor.
    let floor = (whole_map || params.wmo_floors).then(|| {
        let wmos: Vec<benilla_formats::WmoInstance> = tiles
            .iter()
            .flat_map(|(t, _)| t.wmos.iter().cloned())
            .collect();
        crate::wmo_floor::walkable_floor(
            chain,
            &grid,
            &wmos,
            &height,
            placement_rotation,
            !whole_map,
        )
    });
    // On a continent the terrain stays the ground, and an open WMO floor (a bridge's deck, a
    // tunnel's, a city's street) is laid over it where there is one.
    let mut laid = vec![false; grid.len()];
    if let Some(f) = &floor {
        if whole_map {
            height.clone_from(&f.height);
        }
        for i in 0..grid.len() {
            if whole_map {
                in_zone[i] = height[i].is_finite();
            }
            if f.paved[i] && !f.solid[i] {
                road[i] = 1.0;
                if !whole_map && f.height[i].is_finite() {
                    height[i] = f.height[i];
                    laid[i] = true;
                }
            }
        }
    }

    let mut slope = slope_raster(&grid, &height);
    // A laid floor's slope is its own: beside a tunnel's floor is the mountain over it.
    if laid.iter().any(|&l| l) {
        let floor_only: Vec<f32> = height
            .iter()
            .zip(&laid)
            .map(|(&z, &l)| if l { z } else { f32::NAN })
            .collect();
        let own = slope_raster(&grid, &floor_only);
        for i in 0..grid.len() {
            if laid[i] {
                slope[i] = own[i];
            }
        }
    }

    // Obstacles: every placement inside the raster, once per uniqueId.
    let mut obstacles = Vec::new();
    let mut seen_doodad = HashSet::new();
    let mut seen_wmo = HashSet::new();
    let mut m2_hulls: HashMap<String, Option<Hull>> = HashMap::new();
    // The WMOs' walls: an instance's are its floor's solid (off its streets is a wall); a
    // continent zone's, with WMO floors laid over its terrain, are where a WMO face stands in the
    // kart's band over the ground (its buildings are then not stamped as discs below).
    let mut solid = match &floor {
        Some(f) if whole_map => f.solid.clone(),
        Some(f) => f.walls.clone(),
        None => vec![false; grid.len()],
    };
    let inside = |x: f32, y: f32| {
        let (c, r) = grid.pixel_of(x, y);
        grid.index(c.floor() as i64, r.floor() as i64).is_some()
    };
    for (tile, extents) in &tiles {
        for d in &tile.doodads {
            if !inside(d.position[0], d.position[1]) || !seen_doodad.insert(d.unique_id) {
                continue;
            }
            let r = if params.doodad_bounds {
                let hull = m2_hulls
                    .entry(d.model.clone())
                    .or_insert_with(|| m2_hull(chain, &d.model));
                hull.as_ref().map(|h| {
                    stamp_hull(
                        &grid,
                        &mut solid,
                        h,
                        placement_rotation(d.rotation),
                        glam::Vec3::from_array(d.position),
                        d.scale,
                        (d.position[2] - 0.5, d.position[2] + FOOTPRINT_HEIGHT),
                    );
                    hull_footprint(&h.points)
                })
            } else {
                Some(1.0)
            };
            obstacles.push(Obstacle {
                kind: ObstacleKind::Doodad,
                model: d.model.clone(),
                x: d.position[0],
                y: d.position[1],
                z: d.position[2],
                radius: r.unwrap_or(0.0) * d.scale,
                solid: r.is_some_and(|r| r > 0.0),
            });
        }
        for w in &tile.wmos {
            if !seen_wmo.insert(w.unique_id) {
                continue;
            }
            let ext = extents.iter().find(|e| e.unique_id == w.unique_id);
            let (x, y, z, radius) = match ext {
                Some(e) => e.centre_radius(),
                None => (w.position[0], w.position[1], w.position[2], 10.0),
            };
            if !inside(x, y) {
                continue;
            }
            // A WMO-built map's walls are in its floor's solid already.
            if radius <= MAX_WMO_RADIUS && floor.is_none() {
                stamp_disc(&grid, &mut solid, x, y, radius * WMO_FOOTPRINT_SHARE);
            }
            obstacles.push(Obstacle {
                kind: ObstacleKind::Wmo,
                model: w.model.clone(),
                x,
                y,
                z,
                radius,
                solid: true,
            });
        }
    }

    let mut textures: Vec<TextureCoverage> = coverage
        .into_iter()
        .map(|(path, pixels)| TextureCoverage {
            is_road: is_road_texture(&path, &params.road_keywords, &params.road_exclude),
            path,
            pixels,
        })
        .collect();
    textures.sort_by(|a, b| b.pixels.total_cmp(&a.pixels));

    // The WMOs' own doodads (carts, barrels, lamp posts), stood on as the ADT's are, and the
    // instance's area triggers, kept clear: one is an exit (Stratholme's back gate, 2221) or
    // starts a script (Aurius, 2187), and a kart passing it would leave the race.
    if let Some(f) = &floor {
        let triggers = benilla_formats::load_area_trigger_catalog(chain)?;
        for t in triggers.on_map(map_id) {
            let reach = if t.radius > 0.0 {
                t.radius
            } else {
                0.5 * t.box_size[0].hypot(t.box_size[1])
            };
            stamp_disc(
                &grid,
                &mut solid,
                t.position[0],
                t.position[1],
                reach + TRIGGER_MARGIN,
            );
        }
        for d in &f.doodads {
            let (model, at) = (&d.model, d.at);
            if !inside(at.x, at.y) {
                continue;
            }
            // Only what stands on the ground: a lamp on a wall, a banner on a roof or a crate in
            // a cellar is out of a kart's way.
            let (c, r) = grid.pixel_of(at.x, at.y);
            let ground = grid
                .index(c.floor() as i64, r.floor() as i64)
                .map_or(f32::NAN, |i| height[i]);
            if !ground.is_finite() || (at.z - ground).abs() > DOODAD_ON_GROUND {
                continue;
            }
            let hull = m2_hulls
                .entry(model.clone())
                .or_insert_with(|| m2_hull(chain, model));
            if let Some(h) = hull.as_ref() {
                // A kart rides over what stays under a kerb's height: a rubble mound, a broken
                // barrel.
                let band = (ground + KERB, ground + FOOTPRINT_HEIGHT);
                stamp_hull(&grid, &mut solid, h, d.rotation, at, d.scale, band);
            }
        }
    }

    // A hull's cut is its outline; fill what it encloses (a trunk's inside).
    let solid = crate::roads::Mask {
        width: grid.width,
        height: grid.height,
        data: solid,
    }
    .fill_holes(SOLID_HOLE_PX)
    .data;
    // A laid floor is driven on, whatever footprint its own building stamped.
    let solid: Vec<bool> = solid.iter().zip(&laid).map(|(&s, &l)| s && !l).collect();
    let walls: Vec<bool> = match &floor {
        Some(f) if !whole_map => f.walls.iter().zip(&laid).map(|(&w, &l)| w && !l).collect(),
        _ => vec![false; grid.len()],
    };
    let area_names = area
        .iter()
        .copied()
        .collect::<HashSet<u32>>()
        .into_iter()
        .filter_map(|id| areas.name(id).map(|name| (id, name.to_string())))
        .collect();

    Ok(ZoneTerrain {
        zone: zone_name,
        area_id: zone_area,
        map_id,
        map,
        grid,
        height,
        road,
        water,
        in_zone,
        slope,
        area,
        area_names,
        solid,
        obstacles,
        textures,
        probes,
        wmo_ground: whole_map,
        laid,
        walls,
    })
}

fn minmax(a: f32, b: f32) -> (f32, f32) {
    (a.min(b), a.max(b))
}

/// Each layer's visible weight at chunk-relative `(south, east)` in `0..1`: layer `i ≥ 1` is drawn
/// over the layers below it with its alpha, so a lower layer keeps `∏(1 − a)` of what is above.
fn layer_weights(c: &ChunkMesh, south: f32, east: f32) -> [f32; 4] {
    let mut w = [1.0, 0.0, 0.0, 0.0];
    let Some(alpha) = &c.alpha_map else {
        return w;
    };
    let n = 64usize;
    let row = ((south * n as f32) as usize).min(n - 1);
    let col = ((east * n as f32) as usize).min(n - 1);
    let texel = &alpha[(row * n + col) * 4..(row * n + col) * 4 + 4];
    for layer in 1..c.layer_textures.len().min(4) {
        let a = f32::from(texel[layer - 1]) / 255.0;
        for wl in w.iter_mut().take(layer) {
            *wl *= 1.0 - a;
        }
        w[layer] = a;
    }
    w
}

/// Whether a wet MCLQ cell covers chunk-relative `(south, east)` above the terrain `ground`.
fn liquid_at(c: &ChunkMesh, south: f32, east: f32, ground: f32) -> bool {
    let row = ((south * 8.0) as usize).min(7);
    let col = ((east * 8.0) as usize).min(7);
    c.liquids.iter().any(|l| {
        if l.grid != [9, 9] || !l.wet.get(row * 8 + col).copied().unwrap_or(false) {
            return false;
        }
        let corners = [
            row * 9 + col,
            row * 9 + col + 1,
            (row + 1) * 9 + col,
            (row + 1) * 9 + col + 1,
        ];
        let top = corners
            .iter()
            .map(|&k| l.positions[k][2])
            .fold(f32::MIN, f32::max);
        !ground.is_finite() || top > ground
    })
}

/// Slope in degrees from central differences; NaN neighbours fall back to the centre.
pub(crate) fn slope_raster(grid: &Grid, height: &[f32]) -> Vec<f32> {
    let (w, h) = (grid.width, grid.height);
    let res = grid.yards_per_px;
    let mut out = vec![0.0; w * h];
    for r in 0..h {
        for c in 0..w {
            let i = r * w + c;
            let hc = height[i];
            if !hc.is_finite() {
                out[i] = f32::NAN;
                continue;
            }
            let at = |cc: usize, rr: usize| {
                let v = height[rr * w + cc];
                if v.is_finite() {
                    v
                } else {
                    hc
                }
            };
            let dx = (at((c + 1).min(w - 1), r) - at(c.saturating_sub(1), r)) / (2.0 * res);
            let dy = (at(c, (r + 1).min(h - 1)) - at(c, r.saturating_sub(1))) / (2.0 * res);
            out[i] = (dx * dx + dy * dy).sqrt().atan().to_degrees();
        }
    }
    out
}

/// A MODF placement's world-space extents, which the WMO reader does not keep.
struct ModfExtent {
    unique_id: u32,
    /// WoW world `[x, y, z]` corners.
    lo: [f32; 3],
    hi: [f32; 3],
}

impl ModfExtent {
    fn centre_radius(&self) -> (f32, f32, f32, f32) {
        let half = [
            (self.hi[0] - self.lo[0]) / 2.0,
            (self.hi[1] - self.lo[1]) / 2.0,
        ];
        (
            (self.lo[0] + self.hi[0]) / 2.0,
            (self.lo[1] + self.hi[1]) / 2.0,
            self.lo[2],
            (half[0] * half[0] + half[1] * half[1]).sqrt(),
        )
    }
}

/// MODF's 64-byte entries: the extents are two corner-origin `C3`s at `+0x20`, mapped to world as
/// MDDF positions are (`x = 32·TILE − z`, `y = 32·TILE − x`, `z = y`).
fn read_modf_extents(adt: &[u8]) -> Vec<ModfExtent> {
    let mut out = Vec::new();
    for (magic, data) in chunks(adt) {
        if &magic != b"FDOM" {
            continue;
        }
        for e in data.as_chunks::<64>().0 {
            let f = |o: usize| e.f32_at(o).unwrap_or(0.0);
            let a = [MAP_CENTER - f(0x28), MAP_CENTER - f(0x20), f(0x24)];
            let b = [MAP_CENTER - f(0x34), MAP_CENTER - f(0x2c), f(0x30)];
            out.push(ModfExtent {
                unique_id: e.u32_at(4).unwrap_or(0),
                lo: [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])],
                hi: [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])],
            });
        }
    }
    out
}

/// An M2's collision box horizontal half-diagonal; `Some(0)` with no collision, `None` unreadable.
/// An M2's collision hull in model space.
struct Hull {
    points: Vec<[f32; 3]>,
    indices: Vec<u32>,
}

fn m2_hull(chain: &mut Chain, model: &str) -> Option<Hull> {
    let lower = model.to_ascii_lowercase();
    let path = if lower.ends_with(".mdx") || lower.ends_with(".mdl") {
        format!("{}.m2", &model[..model.len() - 4])
    } else {
        model.to_string()
    };
    let bytes = chain.read_file(&path).ok()?;
    let hull = benilla_formats::parse_m2_collision_hull(&bytes).ok()?;
    Some(Hull {
        points: hull.positions,
        indices: hull.indices,
    })
}

/// Enclosed open patches of a solid footprint smaller than this (pixels) are inside it.
const SOLID_HOLE_PX: usize = 200;

/// The height over a street (yards) under which a WMO doodad's hull is ridden over, not hit: a
/// rubble mound (Stratholme's `KarazahnRockRubble02`, a yard high) or a broken barrel, which a
/// player walks over too; a crate stands taller.
const KERB: f32 = 1.1;

/// A WMO doodad whose origin is further than this (yards) above or below the ground under it stands
/// on a wall, a roof or a floor below, not in the street.
const DOODAD_ON_GROUND: f32 = 2.0;

/// Yards kept between an instance's area trigger and the ground a course may use.
pub const TRIGGER_MARGIN: f32 = 12.0;

/// A WMO wider than this (yards) is a cave or a city, mostly underground or walled by terrain; its
/// extents are not stamped as a footprint.
pub const MAX_WMO_RADIUS: f32 = 100.0;
/// A WMO's extents half-diagonal overstates its footprint; it is stamped at this share.
pub const WMO_FOOTPRINT_SHARE: f32 = 0.8;

/// An MDDF placement's rotation in the WoW frame (wowdev.wiki's `createPlacementMatrix`, as
/// `benilla_assets::coords::placement_rotation` before its Bevy conjugation).
fn placement_rotation(rotation_deg: [f32; 3]) -> glam::Quat {
    use glam::Quat;
    use std::f32::consts::{FRAC_PI_2, PI};
    let [rx, ry, rz] = rotation_deg.map(f32::to_radians);
    Quat::from_rotation_x(FRAC_PI_2)
        * Quat::from_rotation_y(ry - PI)
        * Quat::from_rotation_z(-rx)
        * Quat::from_rotation_x(rz - FRAC_PI_2)
}

/// Mark the pixels a placed doodad's hull covers between heights `lo` and `hi` (world yards).
fn stamp_hull(
    grid: &Grid,
    solid: &mut [bool],
    hull: &Hull,
    q: glam::Quat,
    origin: glam::Vec3,
    scale: f32,
    (lo, hi): (f32, f32),
) {
    use glam::Vec3;
    let world: Vec<Vec3> = hull
        .points
        .iter()
        .map(|p| origin + q * (Vec3::from_array(*p) * scale))
        .collect();
    for t in hull.indices.as_chunks::<3>().0 {
        let tri = [
            world[t[0] as usize],
            world[t[1] as usize],
            world[t[2] as usize],
        ];
        let poly = clip_slab(&tri, lo, hi);
        if poly.len() >= 2 {
            stamp_polygon(grid, solid, &poly);
        }
    }
}

/// The part of a triangle between heights `lo` and `hi` (Sutherland–Hodgman on two planes).
fn clip_slab(tri: &[glam::Vec3; 3], lo: f32, hi: f32) -> Vec<glam::Vec3> {
    fn clip(poly: Vec<glam::Vec3>, inside: impl Fn(f32) -> f32) -> Vec<glam::Vec3> {
        let mut out = Vec::with_capacity(poly.len() + 2);
        for i in 0..poly.len() {
            let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
            let (da, db) = (inside(a.z), inside(b.z));
            if da >= 0.0 {
                out.push(a);
            }
            if (da >= 0.0) != (db >= 0.0) {
                out.push(a + (b - a) * (da / (da - db)));
            }
        }
        out
    }
    let p = clip(tri.to_vec(), |z| z - lo);
    if p.is_empty() {
        return p;
    }
    clip(p, |z| hi - z)
}

/// Mark the pixels under a convex polygon's ground projection, and every pixel its edges cross
/// so a thin fence rail still blocks.
fn stamp_polygon(grid: &Grid, solid: &mut [bool], poly: &[glam::Vec3]) {
    use glam::Vec2;
    let px: Vec<Vec2> = poly
        .iter()
        .map(|p| {
            let (c, r) = grid.pixel_of(p.x, p.y);
            Vec2::new(c, r)
        })
        .collect();
    let mut mark = |c: f32, r: f32| {
        if let Some(i) = grid.index(c.floor() as i64, r.floor() as i64) {
            solid[i] = true;
        }
    };
    for i in 0..px.len() {
        let (a, b) = (px[i], px[(i + 1) % px.len()]);
        let n = (a.distance(b) * 3.0).ceil().max(1.0) as usize;
        for k in 0..=n {
            let p = a.lerp(b, k as f32 / n as f32);
            mark(p.x, p.y);
        }
    }
    if px.len() < 3 {
        return;
    }
    let lo = px.iter().fold(Vec2::MAX, |m, p| m.min(*p));
    let hi = px.iter().fold(Vec2::MIN, |m, p| m.max(*p));
    for r in lo.y.floor() as i64..=hi.y.ceil() as i64 {
        for c in lo.x.floor() as i64..=hi.x.ceil() as i64 {
            let p = Vec2::new(c as f32 + 0.5, r as f32 + 0.5);
            // Inside a convex polygon of either winding: no edge has it on the other side.
            let (mut pos, mut neg) = (false, false);
            for i in 0..px.len() {
                let s = (px[(i + 1) % px.len()] - px[i]).perp_dot(p - px[i]);
                pos |= s > 0.0;
                neg |= s < 0.0;
            }
            if !(pos && neg) {
                mark(p.x, p.y);
            }
        }
    }
}

fn stamp_disc(grid: &Grid, solid: &mut [bool], x: f32, y: f32, radius: f32) {
    let (c, r) = grid.pixel_of(x, y);
    let k = (radius / grid.yards_per_px).ceil() as i64 + 1;
    for dr in -k..=k {
        for dc in -k..=k {
            let (pc, pr) = (c.floor() as i64 + dc, r.floor() as i64 + dr);
            let Some(i) = grid.index(pc, pr) else {
                continue;
            };
            let (wx, wy) = grid.world_of(pc as f32 + 0.5, pr as f32 + 0.5);
            if (wx - x).hypot(wy - y) <= radius {
                solid[i] = true;
            }
        }
    }
}

/// Height above a doodad's lowest hull point (model yards) that counts as its footprint: a kart's
/// height and a little, so a tree's trunk counts and its canopy does not.
const FOOTPRINT_HEIGHT: f32 = 2.5;

/// The horizontal reach from the origin of a collision hull's low part: the hull's points and its
/// edges' crossings at [`FOOTPRINT_HEIGHT`] above its base; 0 with no hull.
fn hull_footprint(points: &[[f32; 3]]) -> f32 {
    let Some(base) = points.iter().map(|p| p[2]).reduce(f32::min) else {
        return 0.0;
    };
    let top = base.max(0.0) + FOOTPRINT_HEIGHT;
    points
        .iter()
        .filter(|p| p[2] <= top)
        .map(|p| p[0].hypot(p[1]))
        .fold(0.0, f32::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_round_trips_and_faces_north_up() {
        let g = Grid {
            north_x: 100.0,
            west_y: 50.0,
            yards_per_px: 2.0,
            width: 10,
            height: 10,
        };
        let (x, y) = g.world_of(3.0, 4.0);
        assert_eq!((x, y), (92.0, 44.0));
        assert_eq!(g.pixel_of(x, y), (3.0, 4.0));
        // A later row is further south (smaller x), a later column further east (smaller y).
        assert!(g.world_of(0.0, 1.0).0 < g.world_of(0.0, 0.0).0);
        assert!(g.world_of(1.0, 0.0).1 < g.world_of(0.0, 0.0).1);
    }

    #[test]
    fn road_keywords_match_case_insensitively() {
        let k: Vec<String> = ROAD_KEYWORDS.iter().map(|s| s.to_string()).collect();
        assert!(is_road_texture(
            "Tileset\\Elwynn\\ElwynnCobblestoneBase.blp",
            &k,
            &[]
        ));
        assert!(!is_road_texture(
            "Tileset\\Elwynn\\ElwynnGrassBase.blp",
            &k,
            &[]
        ));
        assert!(!is_road_texture(
            "Tileset\\Elwynn\\ElwynnCobblestoneBase.blp",
            &k,
            &["cobble".to_string()]
        ));
    }
}
