//! The drivability cost field over a zone's raster: what a kart may cross, how much it costs, and
//! how far each pixel is from anything it may not cross.

use crate::roads::Mask;
use crate::terrain::{Grid, ZoneTerrain};

/// Choices for [`CostField::build`].
#[derive(Clone, Debug)]
pub struct CostParams {
    /// Cost of a fully paved pixel against open ground's 1.0.
    pub road_cost: f32,
    /// Steeper ground (degrees) is impassable.
    pub max_slope: f32,
    /// Slope (degrees) at which the slope factor reaches 2.
    pub slope_scale: f32,
    /// Yards kept clear around every solid footprint pixel and hole.
    pub obstacle_margin: f32,
    /// The margin on paved ground.
    pub lane_margin: f32,
    /// Clearance (yards) below which a pixel costs extra, rising as the square.
    pub want_clearance: f32,
    pub clearance_penalty: f32,
    /// Extra cost within `water_near` yards of water.
    pub water_near: f32,
    pub water_penalty: f32,
}

impl Default for CostParams {
    fn default() -> Self {
        Self {
            road_cost: 0.35,
            max_slope: 32.0,
            slope_scale: 22.0,
            obstacle_margin: 2.0,
            lane_margin: 0.5,
            want_clearance: 14.0,
            clearance_penalty: 5.0,
            water_near: 12.0,
            water_penalty: 0.6,
        }
    }
}

/// Why a pixel is impassable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Block {
    Clear,
    Outside,
    Water,
    Steep,
    Obstacle,
}

/// Per-pixel cost (`f32::INFINITY` impassable), clearance and water distance over a [`Grid`].
pub struct CostField {
    pub grid: Grid,
    pub block: Vec<Block>,
    pub cost: Vec<f32>,
    /// Yards to the nearest impassable pixel (0 on one).
    pub clearance: Vec<f32>,
    /// Yards to the nearest water pixel.
    pub water_dist: Vec<f32>,
    /// Road weight, 0..1, as the terrain's.
    pub road: Vec<f32>,
}

impl CostField {
    pub fn build(t: &ZoneTerrain, p: &CostParams) -> Self {
        let g = t.grid;
        let res = g.yards_per_px;
        let n = g.len();
        let mut block = vec![Block::Clear; n];
        for (i, b) in block.iter_mut().enumerate() {
            *b = if !t.in_zone[i] || !t.height[i].is_finite() {
                Block::Outside
            } else if t.water[i] {
                Block::Water
            } else if t.slope[i] > p.max_slope {
                Block::Steep
            } else {
                Block::Clear
            };
        }
        // Within the margin of a solid footprint pixel or a hole in the ground (a cave or mine
        // mouth). A paved lane keeps only a sliver of margin: the fences lining it are its kerbs.
        let open = Mask {
            width: g.width,
            height: g.height,
            data: (0..n)
                .map(|i| !t.solid[i] && (t.height[i].is_finite() || !t.in_zone[i]))
                .collect(),
        };
        for (i, d) in open.distance_transform().into_iter().enumerate() {
            let margin = if t.road[i] >= 0.5 {
                p.lane_margin
            } else {
                p.obstacle_margin
            };
            if d * res <= margin + res * 0.5 && block[i] == Block::Clear {
                block[i] = Block::Obstacle;
            }
        }
        let clear = Mask {
            width: g.width,
            height: g.height,
            data: block.iter().map(|&b| b == Block::Clear).collect(),
        };
        let clearance: Vec<f32> = clear
            .distance_transform()
            .into_iter()
            .map(|d| d * res)
            .collect();
        let dry = Mask {
            width: g.width,
            height: g.height,
            data: t.water.iter().map(|&w| !w).collect(),
        };
        // Off the raster counts as water to the transform; nothing is near that edge that matters.
        let water_dist: Vec<f32> = dry
            .distance_transform()
            .into_iter()
            .map(|d| d * res)
            .collect();
        let cost = (0..n)
            .map(|i| {
                if block[i] != Block::Clear {
                    return f32::INFINITY;
                }
                let road = t.road[i].clamp(0.0, 1.0);
                let base = 1.0 + (p.road_cost - 1.0) * road;
                let slope = 1.0 + (t.slope[i] / p.slope_scale).powi(3);
                let short = ((p.want_clearance - clearance[i]) / p.want_clearance).max(0.0);
                let wet = if water_dist[i] < p.water_near {
                    p.water_penalty
                } else {
                    0.0
                };
                // A paved lane between fences is meant to be driven: room matters less there.
                base * slope + p.clearance_penalty * (1.0 - 0.75 * road) * short * short + wet
            })
            .collect();
        Self {
            grid: g,
            block,
            cost,
            clearance,
            water_dist,
            road: t.road.clone(),
        }
    }

    fn nearest(&self, x: f32, y: f32) -> Option<usize> {
        let (c, r) = self.grid.pixel_of(x, y);
        self.grid.index(c.floor() as i64, r.floor() as i64)
    }

    pub fn clearance_at(&self, x: f32, y: f32) -> f32 {
        self.nearest(x, y).map_or(0.0, |i| self.clearance[i])
    }

    pub fn water_dist_at(&self, x: f32, y: f32) -> f32 {
        self.nearest(x, y).map_or(0.0, |i| self.water_dist[i])
    }

    pub fn block_at(&self, x: f32, y: f32) -> Block {
        self.nearest(x, y).map_or(Block::Outside, |i| self.block[i])
    }

    pub fn passable_at(&self, x: f32, y: f32) -> bool {
        self.block_at(x, y) == Block::Clear
    }
}
