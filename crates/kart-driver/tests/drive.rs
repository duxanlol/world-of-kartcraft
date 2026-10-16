//! The driver over synthetic tiled worlds: corridors of floor strips between low walls.

use std::f32::consts::PI;

use glam::Vec3;
use kart_driver::{drive, Class, Collision, DriveOpts, DriveReport, Route, CELL};

type Tri = [[f32; 3]; 3];

/// Top speed of the Balanced class, lu/s.
fn top() -> f32 {
    Class::Balanced.stats().speed as f32 * 30.0 / 256.0
}

fn quad(tris: &mut Vec<Tri>, a: Vec3, b: Vec3, c: Vec3, d: Vec3) {
    tris.push([a.to_array(), b.to_array(), c.to_array()]);
    tris.push([a.to_array(), c.to_array(), d.to_array()]);
}

/// A road `half` lu each side of `centre` (floor at the centre's height) between walls `wall` lu
/// high, one strip per centre segment; segments where `gap` holds have neither.
fn corridor(
    centre: &[Vec3],
    closed: bool,
    half: f32,
    wall: f32,
    gap: impl Fn(Vec3) -> bool,
) -> Vec<Tri> {
    let n = centre.len();
    let left = |i: usize| {
        let a = centre[if closed {
            (i + n - 1) % n
        } else {
            i.saturating_sub(1)
        }];
        let b = centre[if closed {
            (i + 1) % n
        } else {
            (i + 1).min(n - 1)
        }];
        let d = (b - a).with_y(0.0).normalize();
        Vec3::new(d.z, 0.0, -d.x)
    };
    let mut tris = Vec::new();
    let segs = if closed { n } else { n - 1 };
    let up = Vec3::Y * wall;
    for i in 0..segs {
        let j = (i + 1) % n;
        let (a, b) = (centre[i], centre[j]);
        if gap((a + b) / 2.0) {
            continue;
        }
        let (la, lb) = (a + left(i) * half, b + left(j) * half);
        let (ra, rb) = (a - left(i) * half, b - left(j) * half);
        quad(&mut tris, la, lb, rb, ra);
        quad(&mut tris, la, lb, lb + up, la + up);
        quad(&mut tris, ra, rb, rb + up, ra + up);
    }
    tris
}

fn world(tris: Vec<Tri>) -> Collision {
    Collision::new(tris, CELL)
}

/// Every `every`th point, and the last.
fn thin(points: &[Vec3], every: usize) -> Vec<Vec3> {
    let mut out: Vec<Vec3> = points.iter().step_by(every).copied().collect();
    if !(points.len() - 1).is_multiple_of(every) {
        out.push(points[points.len() - 1]);
    }
    out
}

fn line(from: Vec3, to: Vec3, step: f32) -> Vec<Vec3> {
    let n = (from.distance(to) / step).round() as usize;
    (0..n).map(|i| from.lerp(to, i as f32 / n as f32)).collect()
}

fn arc(centre: Vec3, r: f32, from: f32, to: f32, step: f32) -> Vec<Vec3> {
    let n = ((to - from).abs() * r / step).round() as usize;
    (0..n)
        .map(|i| {
            let a = from + (to - from) * i as f32 / n as f32;
            centre + Vec3::new(r * a.cos(), 0.0, r * a.sin())
        })
        .collect()
}

fn opts(time_limit: f32) -> DriveOpts {
    DriveOpts {
        time_limit,
        ..Default::default()
    }
}

fn show(name: &str, r: &DriveReport) {
    println!("{name}: {}", r.to_json(false));
}

/// A stadium: straights of 6000 lu joined by 1500-lu bends, 600 lu wide.
#[test]
fn an_oval_finishes_with_sensible_laps_and_powerslides_its_bends() {
    let (r, len) = (1500.0, 6000.0);
    let mut centre = line(Vec3::new(r, 0.0, 0.0), Vec3::new(r, 0.0, len), 100.0);
    centre.extend(arc(Vec3::new(0.0, 0.0, len), r, 0.0, PI, 100.0));
    centre.extend(line(
        Vec3::new(-r, 0.0, len),
        Vec3::new(-r, 0.0, 0.0),
        100.0,
    ));
    centre.extend(arc(Vec3::ZERO, r, PI, 2.0 * PI, 100.0));
    let w = world(corridor(&centre, true, 300.0, 60.0, |_| false));
    let route = Route::new(thin(&centre, 5), 2, 400.0);
    let report = drive(&w, &route, &opts(120.0));
    show("oval", &report);
    assert!(report.finished, "{report:?}");
    assert_eq!(report.lap_times.len(), 2);
    let lap = 2.0 * len + 2.0 * PI * r;
    for &t in &report.lap_times {
        // No faster than flat out all the way, no slower than at 60 % of it.
        assert!(
            t > lap / (top() * 1.3) && t < lap / (top() * 0.6),
            "lap {t}"
        );
    }
    assert!(report.powerslides >= 3, "{}", report.powerslides);
    assert!(report.boosts >= 2, "{}", report.boosts);
    assert_eq!(report.stuck_recoveries, 0);
    assert_eq!(report.off_track_events, 0);
    assert!(report.top_speed_pct > 30.0, "{}", report.top_speed_pct);
    assert!(report.max_slope_deg < 1.0);
    assert!(report.elevation_gain < 50.0, "{}", report.elevation_gain);
    let samples = report.time / 0.25;
    assert!((report.trace.len() as f32 - samples).abs() <= 2.0);
}

/// A straight with a ramp up 400 lu over 1200 and a 600-lu gap to a landing at the start's
/// height, a pit below the gap.
#[test]
fn a_ramp_over_a_gap_is_airtime() {
    let height = |z: f32| ((z - 3000.0) / 1200.0).clamp(0.0, 1.0) * 400.0;
    let gap = |p: Vec3| (4200.0..4800.0).contains(&p.z);
    let centre: Vec<Vec3> = line(
        Vec3::new(0.0, 0.0, -200.0),
        Vec3::new(0.0, 0.0, 10000.0),
        100.0,
    )
    .into_iter()
    .map(|p| {
        if p.z >= 4800.0 {
            p
        } else {
            p.with_y(height(p.z))
        }
    })
    .collect();
    let mut tris = corridor(&centre, false, 300.0, 60.0, gap);
    let pit = |z: f32| Vec3::new(0.0, -300.0, z);
    tris.extend(corridor(
        &[pit(4200.0), pit(4500.0), pit(4800.0)],
        false,
        300.0,
        300.0,
        |_| false,
    ));
    let x = Vec3::X * 300.0;
    let (lip, land) = (Vec3::new(0.0, 400.0, 4200.0), Vec3::new(0.0, 0.0, 4800.0));
    quad(
        &mut tris,
        lip - x,
        lip + x,
        pit(4200.0) + x,
        pit(4200.0) - x,
    );
    quad(
        &mut tris,
        land - x,
        land + x,
        pit(4800.0) + x,
        pit(4800.0) - x,
    );
    let route = Route::new(thin(&centre[2..centre.len() - 10], 5), 0, 400.0);
    let report = drive(&world(tris), &route, &opts(60.0));
    show("ramp", &report);
    assert!(report.finished, "{report:?}");
    assert!(report.longest_air > 0.5, "{}", report.longest_air);
    assert!(report.airtime >= report.longest_air);
    assert!(report.elevation_gain > 350.0, "{}", report.elevation_gain);
    assert!(
        (15.0..22.0).contains(&report.max_slope_deg),
        "{}",
        report.max_slope_deg
    );
    assert_eq!(report.powerslides, 0);
}

/// A corridor walled off at 3000 lu while the route runs on to 8000.
#[test]
fn a_dead_end_is_stuck_and_not_finished() {
    let centre = line(
        Vec3::new(0.0, 0.0, -200.0),
        Vec3::new(0.0, 0.0, 3000.0),
        100.0,
    );
    let mut tris = corridor(&centre, false, 300.0, 60.0, |_| false);
    let end = Vec3::new(0.0, 0.0, 2900.0);
    let (x, up) = (Vec3::X * 300.0, Vec3::Y * 200.0);
    quad(&mut tris, end - x, end + x, end + x + up, end - x + up);
    let route = Route::new(
        line(Vec3::ZERO, Vec3::new(0.0, 0.0, 8000.0), 500.0),
        0,
        400.0,
    );
    let report = drive(&world(tris), &route, &opts(60.0));
    show("dead end", &report);
    assert!(!report.finished);
    assert!(report.stuck_recoveries >= 1);
    assert!(report.gave_up || report.time >= 60.0);
    assert!(report.wall_contacts >= 1);
}

/// Up a straight, round a 400-lu hairpin and back down the next straight.
#[test]
fn a_hairpin_slows_the_kart_and_still_finishes() {
    let (r, len) = (400.0, 4000.0);
    let mut centre = line(Vec3::new(0.0, 0.0, -200.0), Vec3::new(0.0, 0.0, len), 100.0);
    centre.extend(arc(Vec3::new(-r, 0.0, len), r, 0.0, PI, 100.0));
    centre.extend(line(
        Vec3::new(-2.0 * r, 0.0, len),
        Vec3::new(-2.0 * r, 0.0, -200.0),
        100.0,
    ));
    let w = world(corridor(&centre, false, 250.0, 60.0, |_| false));
    let route = Route::new(thin(&centre[2..centre.len() - 5], 2), 0, 350.0);
    let report = drive(&w, &route, &opts(60.0));
    show("hairpin", &report);
    assert!(report.finished, "{report:?}");
    assert_eq!(report.stuck_recoveries, 0);
    let slowest_in_bend = report
        .trace
        .iter()
        .filter(|p| p.position.z > len - 100.0)
        .map(|p| p.speed)
        .fold(f32::INFINITY, f32::min);
    assert!(slowest_in_bend < top() * 0.6, "{slowest_in_bend}");
    assert!(report.max_speed > top() * 0.9);
}

/// A straight with a box across it `height` lu tall at 1500 lu: a log, a kerb or a wall.
fn across(height: f32) -> (Collision, Route) {
    let centre = line(
        Vec3::new(0.0, 0.0, -200.0),
        Vec3::new(0.0, 0.0, 4000.0),
        100.0,
    );
    let mut tris = corridor(&centre, false, 300.0, 300.0, |_| false);
    let (x, up) = (Vec3::X * 300.0, Vec3::Y * height);
    let (near, far) = (Vec3::Z * 1500.0, Vec3::Z * 1530.0);
    quad(&mut tris, near - x, near + x, near + x + up, near - x + up);
    quad(
        &mut tris,
        near - x + up,
        near + x + up,
        far + x + up,
        far - x + up,
    );
    quad(&mut tris, far - x, far + x, far + x + up, far - x + up);
    let route = Route::new(
        line(Vec3::ZERO, Vec3::new(0.0, 0.0, 3800.0), 500.0),
        0,
        400.0,
    );
    (world(tris), route)
}

#[test]
fn a_log_across_the_road_is_hopped_and_a_wall_is_not() {
    // 0.8 yd: under the hop's 1.4 yd apex.
    let (w, route) = across(29.0);
    let hopped = drive(&w, &route, &opts(30.0));
    show("log, hopping", &hopped);
    assert!(hopped.finished && hopped.hops >= 1, "the log is hopped");
    let no_hops = DriveOpts {
        hops: false,
        ..opts(30.0)
    };
    let blocked = drive(&w, &route, &no_hops);
    show("log, no hops", &blocked);
    assert!(!blocked.finished, "without a hop the log stops the kart");
    // 3 yd: no hop clears it, and none is tried.
    let (w, route) = across(110.0);
    let wall = drive(&w, &route, &opts(30.0));
    show("wall", &wall);
    assert!(!wall.finished && wall.hops == 0);
}

#[test]
#[ignore = "a measurement: run with --ignored --nocapture"]
fn measure_the_tallest_box_a_hop_clears() {
    for h in (10..=70).step_by(4) {
        let (w, route) = across(h as f32);
        let r = drive(&w, &route, &opts(30.0));
        println!(
            "box {h} lu ({:.2} yd): finished {} hops {}",
            h as f32 / 36.576,
            r.finished,
            r.hops
        );
    }
}
