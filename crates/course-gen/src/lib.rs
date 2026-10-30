//! Kart course generation from a zone's terrain, read from the player's own 1.12.1 install at run
//! time: [`terrain`] rasterizes a zone (height, road weight, water, slope, obstacles) and
//! [`roads`] traces its road network into a graph in WoW world coordinates. On those, [`cost`]
//! builds a drivability field, [`generate`] lays circuits and sprints over it, [`metrics`]
//! measures them, [`drivetest`] drives them with the CTR-physics bot, [`select`] keeps the best and
//! [`preview`] draws them; [`marshal`] places the Race Marshal at each course start.

pub mod arena;
pub mod audit;
pub mod cost;
pub mod drivetest;
pub mod export;
pub mod flight;
pub mod gauntlet;
pub mod generate;
pub mod geom;
pub mod gps;
pub mod gps_drive;
pub mod marshal;
pub mod metrics;
pub mod pads;
pub mod preview;
pub mod render;
pub mod roads;
pub mod scout;
pub mod select;
pub mod showcase;
pub mod soup;
pub mod terrain;
pub mod wmo_floor;

pub use roads::{extract_roads, RoadEdge, RoadGraph, RoadNode, RoadParams, RoadPoint};
pub use terrain::{
    load_zone, open_install, Grid, Obstacle, ObstacleKind, TerrainParams, TextureCoverage,
    ZoneTerrain,
};
