//! Stratholme (map 329, an instance) from the player's own install: its streets are the faces of
//! one city WMO over filler terrain (`course_gen::wmo_floor`), and the Stratholme Street Circuit
//! of `courses/courses.json` runs on them.

use course_gen::generate::{self, GenParams, Planner};
use course_gen::{cost, load_zone, TerrainParams};
use glam::Vec2;

/// King's Square, where the circuit starts, and the street's height there (`courses.json`).
const KINGS_SQUARE: (f32, f32, f32) = (3544.9, -3388.9, 132.8);

#[test]
fn stratholme_loads_as_its_streets_and_a_street_circuit_runs_round_kings_square() {
    let data = benilla_formats::wow_data_or_skip!();
    let mut chain = benilla_formats::Chain::open(&data).expect("open the install");
    let t = load_zone(&mut chain, "Stratholme", &TerrainParams::default()).expect("Stratholme");
    assert_eq!(t.map_id, 329);
    assert!(t.wmo_ground, "an instance's ground is its WMO floor");
    assert_eq!(
        t.grid.yards_per_px,
        course_gen::terrain::STREET_YARDS_PER_PX
    );
    // The street, not the filler terrain sixty yards under it.
    let (x, y, z) = KINGS_SQUARE;
    let h = t.height_at(x, y).expect("ground at King's Square");
    assert!((h - z).abs() < 2.0, "King's Square at {h}");
    assert!(t.road_at(x, y).unwrap_or(0.0) > 0.5, "paved");
    // The houses the circuit goes round are not ground.
    assert_eq!(t.height_at(3630.0, -3420.0), None);

    let street = cost::CostParams {
        obstacle_margin: 0.0,
        lane_margin: 0.0,
        ..Default::default()
    };
    let f = cost::CostField::build(&t, &street);
    let planner = Planner::with_clearance(&f, 1, generate::STREET_ROUTE_CLEARANCE);
    // The course's waypoints: King's Square, the rat-trap alley, Market Row, Festival Lane, the
    // King's Square gate.
    let via: Vec<Vec2> = [
        (3539.0, -3389.0),
        (3553.0, -3334.0),
        (3613.0, -3336.0),
        (3668.0, -3331.0),
        (3713.0, -3429.0),
        (3660.0, -3477.0),
        (3633.0, -3501.0),
        (3572.0, -3452.0),
        (3556.0, -3435.0),
    ]
    .iter()
    .map(|&(x, y)| Vec2::new(x, y))
    .collect();
    let c = generate::street_circuit(&planner, &via, &GenParams::default()).expect("a circuit");
    let len = course_gen::geom::length(&c.pts, true);
    assert!((400.0..650.0).contains(&len), "{len} yd a lap");
    // Through the alley: the line passes the rat trap's gates.
    assert!(c
        .pts
        .iter()
        .any(|q| q.distance(Vec2::new(3612.3, -3335.4)) < 6.0));
}
