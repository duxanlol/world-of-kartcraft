//! The course pipeline over made-up terrain: no install needed.

use std::collections::HashMap;

use course_gen::cost::{Block, CostField, CostParams};
use course_gen::drivetest::{test_drive, Frame};
use course_gen::generate::{self, Candidate, GenParams, Hub, Kind, Planner, STEP};
use course_gen::metrics::{self, GRAVITY};
use course_gen::select::checkpoints;
use course_gen::{geom, Grid, ZoneTerrain};
use glam::{Vec2, Vec3};

/// A flat, open zone `size` pixels a side at 2 yd, north-west corner at world (0, 0).
fn flat(size: usize) -> ZoneTerrain {
    let grid = Grid {
        north_x: 0.0,
        west_y: 0.0,
        yards_per_px: 2.0,
        width: size,
        height: size,
    };
    let n = grid.len();
    ZoneTerrain {
        zone: "Testland".into(),
        area_id: 1,
        map_id: 0,
        map: "Test".into(),
        grid,
        height: vec![10.0; n],
        road: vec![0.0; n],
        water: vec![false; n],
        in_zone: vec![true; n],
        slope: vec![0.0; n],
        area: vec![2; n],
        area_names: HashMap::from([(2, "Test Fields".to_string())]),
        solid: vec![false; n],
        obstacles: Vec::new(),
        textures: Vec::new(),
        probes: Vec::new(),
    }
}

/// World point of pixel `(col, row)`'s centre.
fn at(t: &ZoneTerrain, c: usize, r: usize) -> Vec2 {
    let (x, y) = t.grid.world_of(c as f32 + 0.5, r as f32 + 0.5);
    Vec2::new(x, y)
}

#[test]
fn the_field_blocks_obstacles_holes_water_and_cliffs_with_margins() {
    let mut t = flat(100);
    let w = t.grid.width;
    t.solid[50 * w + 50] = true;
    t.height[20 * w + 20] = f32::NAN;
    t.water[80 * w + 80] = true;
    t.slope[80 * w + 20] = 40.0;
    t.road[10 * w + 70] = 1.0;
    let f = CostField::build(&t, &CostParams::default());
    assert_eq!(f.block[50 * w + 50], Block::Obstacle);
    // The 2-yd margin reaches the next pixel but not three away.
    assert_eq!(f.block[50 * w + 51], Block::Obstacle);
    assert_eq!(f.block[50 * w + 53], Block::Clear);
    assert_eq!(f.block[20 * w + 20], Block::Outside);
    assert_eq!(f.block[20 * w + 21], Block::Obstacle);
    assert_eq!(f.block[80 * w + 80], Block::Water);
    assert_eq!(f.block[80 * w + 20], Block::Steep);
    assert!(f.cost[10 * w + 70] < f.cost[10 * w + 60]);
    assert_eq!(f.clearance[50 * w + 50], 0.0);
    assert!(f.clearance[50 * w + 60] > 10.0);
}

#[test]
fn a_route_goes_through_the_gap_in_a_wall() {
    let mut t = flat(200);
    let w = t.grid.width;
    // A wall across the middle with one gap.
    for c in 0..200 {
        if !(140..160).contains(&c) {
            t.solid[100 * w + c] = true;
        }
    }
    let f = CostField::build(&t, &CostParams::default());
    let p = Planner::new(&f, 2);
    let (a, b) = (at(&t, 30, 30), at(&t, 30, 170));
    let leg = p.route(a, b, &HashMap::new(), None, None).expect("a route");
    assert!(leg.points.iter().all(|q| f.passable_at(q.x, q.y)));
    // Where the line crosses the wall's row (pixel centres at row 100.5).
    let px: Vec<(f32, f32)> = leg
        .points
        .iter()
        .map(|q| f.grid.pixel_of(q.x, q.y))
        .collect();
    let col = px
        .windows(2)
        .find(|w| (w[0].1 - 100.5) * (w[1].1 - 100.5) <= 0.0)
        .map(|w| (w[0].0 + w[1].0) / 2.0)
        .expect("crosses the wall's row");
    assert!((140.0..160.0).contains(&col), "{col}");
}

#[test]
fn the_kart_frame_keeps_handedness_and_round_trips() {
    let f = Frame {
        origin: Vec3::new(-9000.0, 100.0, 50.0),
    };
    let p = f.to_lu(-8990.0, 100.0, 50.0);
    // North (+x) is CTR +z, up is +y, west (+y) is CTR +x.
    assert!(p.z > 0.0 && p.x.abs() < 1e-3 && p.y.abs() < 1e-3);
    assert!(f.to_lu(-9000.0, 110.0, 50.0).x > 0.0);
    assert!(f.to_lu(-9000.0, 100.0, 60.0).y > 0.0);
    let back = f.to_wow(f.to_lu(-8950.5, 120.25, 47.0));
    assert!((back - Vec3::new(-8950.5, 120.25, 47.0)).length() < 1e-2);
    // A right-handed basis stays right-handed: x × y = z in both frames.
    let o = f.to_lu(-9000.0, 100.0, 50.0);
    let (ex, ey, ez) = (
        f.to_lu(-8999.0, 100.0, 50.0) - o,
        f.to_lu(-9000.0, 101.0, 50.0) - o,
        f.to_lu(-9000.0, 100.0, 51.0) - o,
    );
    assert!(ex.cross(ey).dot(ez) > 0.0);
}

fn ring(t: &ZoneTerrain, centre: Vec2, r: f32) -> Candidate {
    let raw: Vec<Vec2> = (0..96)
        .map(|i| centre + Vec2::from_angle(i as f32 / 96.0 * std::f32::consts::TAU) * r)
        .collect();
    let _ = t;
    Candidate {
        kind: Kind::Circuit,
        name: "ring".into(),
        pts: geom::resample(&raw, true, STEP),
        closed: true,
        seed: 0,
    }
}

#[test]
fn a_ring_measures_as_a_long_left_bend_with_gates_at_most_180_yd_apart() {
    let t = flat(400);
    let f = CostField::build(&t, &CostParams::default());
    let c = ring(&t, at(&t, 200, 200), 150.0);
    let m = metrics::measure(&c, &t, &f);
    assert!((m.length_yd - std::f32::consts::TAU * 150.0).abs() < 5.0);
    assert!(m.jumps.is_empty());
    assert!(m.elev_range < 1e-3);
    assert_eq!(m.subzones, vec!["Test Fields".to_string()]);
    let z = metrics::heights(&t, &c.pts, true);
    let gates = checkpoints(&c, &m, &z, &f);
    assert_eq!(gates[0].index, 0);
    assert_eq!(gates[0].kind, "start");
    let mut s: Vec<f32> = gates.iter().map(|g| g.s).collect();
    s.push(m.length_yd);
    assert!(s.windows(2).all(|w| w[1] - w[0] <= 181.0), "{s:?}");
    assert!(gates.iter().all(|g| (15.0..=25.0).contains(&g.half_width)));
}

#[test]
fn a_sharp_crest_throws_the_kart_and_a_flat_road_does_not() {
    let n = 200;
    let flat = vec![0.0; n];
    let speed = vec![40.0; n];
    assert!(metrics::jumps(&flat, &speed, false).is_empty());
    // Up a 20 % ramp to a lip, then a drop.
    let ramp: Vec<f32> = (0..n)
        .map(|i| {
            let s = i as f32 * STEP;
            if s < 300.0 {
                0.2 * s
            } else {
                60.0 - 0.4 * (s - 300.0)
            }
        })
        .collect();
    let j = metrics::jumps(&ramp, &speed, false);
    assert_eq!(j.len(), 1, "{j:?}");
    assert!((j[0].s - 300.0).abs() < 10.0);
    assert!(j[0].airtime > 0.2 && j[0].airtime < 2.0 * 40.0 * 0.2 / GRAVITY + 1.0);
}

#[test]
fn a_ring_on_flat_ground_drives_clean() {
    let t = flat(300);
    let c = ring(&t, at(&t, 150, 150), 120.0);
    let (r, rep) = test_drive(&t, &c, 2);
    assert!(r.ok, "{r:?}");
    assert_eq!(r.lap_times.len(), 2);
    assert_eq!(r.wall_contacts, 0);
    assert!(rep.trace.iter().all(|p| p.position.is_finite()));
}

#[test]
fn circuits_generate_on_open_ground_round_a_tree() {
    let mut t = flat(500);
    let w = t.grid.width;
    // A trunk in the middle, so the loops go round something.
    for r in 245..255 {
        for c in 245..255 {
            t.solid[r * w + c] = true;
        }
    }
    let f = CostField::build(&t, &CostParams::default());
    let planner = Planner::new(&f, 2);
    let centre = at(&t, 250, 250);
    let hubs = vec![Hub {
        name: "Middle".into(),
        x: centre.x,
        y: centre.y,
    }];
    let p = GenParams {
        attempts: 12,
        sprint_share: 0.0,
        threads: 4,
        ..Default::default()
    };
    let (cands, why) = generate::generate(&planner, &hubs, &p);
    assert!(!cands.is_empty(), "{why:?}");
    for c in &cands {
        assert!(c.closed);
        assert!(c.pts.iter().all(|q| f.passable_at(q.x, q.y)));
        assert!(!geom::self_intersects(&c.pts, true));
        let len = geom::length(&c.pts, true);
        assert!((p.circuit_len.0..=p.circuit_len.1).contains(&len));
        let k = geom::curvature(&c.pts, true, 3);
        assert!(k.iter().all(|k| k.abs() <= 1.0 / p.min_bend_radius + 1e-4));
    }
    // The same seed gives the same courses.
    let (again, _) = generate::generate(&planner, &hubs, &p);
    assert_eq!(again.len(), cands.len());
    assert_eq!(again[0].pts, cands[0].pts);
}
