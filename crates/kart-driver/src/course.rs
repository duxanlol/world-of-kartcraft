//! A course file: the collision soup and the route, as JSON in lu:
//! `{ "triangles": [[[x,y,z],[x,y,z],[x,y,z]], ...], "waypoints": [[x,y,z], ...], "laps": n,
//! "gate_half_width": w }`, `laps` 0 (or absent) for an open sprint.

use ctr_kart::Collision;
use glam::Vec3;
use serde_json::Value;

use crate::route::Route;

/// The collision grid's cell, lu: the game session's.
pub const CELL: f32 = 160.0;

/// The gate half-width when the file gives none, lu: the game's 20 yards.
const DEFAULT_GATE_HALF_WIDTH: f32 = 20.0 * crate::LU_PER_YARD;

pub struct Course {
    pub triangles: Vec<[[f32; 3]; 3]>,
    pub route: Route,
}

impl Course {
    pub fn from_json(text: &str) -> Result<Self, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let point = |p: &Value| -> Result<[f32; 3], String> {
            let a = p
                .as_array()
                .filter(|a| a.len() == 3)
                .ok_or("a point is not [x, y, z]")?;
            let mut out = [0.0; 3];
            for (o, x) in out.iter_mut().zip(a) {
                *o = x.as_f64().ok_or("a coordinate is not a number")? as f32;
            }
            Ok(out)
        };
        let list = |key: &str| -> Result<&Vec<Value>, String> {
            v[key].as_array().ok_or_else(|| format!("no `{key}` array"))
        };
        let triangles = list("triangles")?
            .iter()
            .map(|t| {
                let a = t
                    .as_array()
                    .filter(|a| a.len() == 3)
                    .ok_or("a triangle is not 3 points")?;
                Ok([point(&a[0])?, point(&a[1])?, point(&a[2])?])
            })
            .collect::<Result<Vec<_>, String>>()?;
        let waypoints = list("waypoints")?
            .iter()
            .map(|p| point(p).map(Vec3::from_array))
            .collect::<Result<Vec<_>, String>>()?;
        if waypoints.len() < 2 {
            return Err("fewer than two waypoints".into());
        }
        let laps = v["laps"].as_u64().unwrap_or(0) as u32;
        let half_width = v["gate_half_width"]
            .as_f64()
            .map_or(DEFAULT_GATE_HALF_WIDTH, |w| w as f32);
        Ok(Self {
            triangles,
            route: Route::new(waypoints, laps, half_width),
        })
    }

    /// The triangles bucketed for the kart.
    pub fn world(&self) -> Collision {
        Collision::new(self.triangles.iter().copied(), CELL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_course_file_reads() {
        let c = Course::from_json(
            r#"{"triangles": [[[0,0,0],[0,0,100],[100,0,0]]],
                "waypoints": [[0,0,0],[0,0,50],[0,0,90]], "laps": 2, "gate_half_width": 300}"#,
        )
        .unwrap();
        assert_eq!(c.triangles.len(), 1);
        assert_eq!(c.route.laps, 2);
        assert_eq!(c.route.gates.len(), 3);
        assert_eq!(c.route.gates[1].half_width, 300.0);
        assert_eq!(c.world().len(), 1);
        assert!(Course::from_json(r#"{"triangles": [], "waypoints": [[0,0]]}"#).is_err());
    }
}
