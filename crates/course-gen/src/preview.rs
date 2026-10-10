//! Course previews drawn from the terrain rasters alone: a hillshaded map of the course's region
//! with roads, water and obstacles, the line with arrows, start and checkpoints; and its
//! elevation profile with the jumps marked.

use glam::Vec2;
use image::{imageops, Rgb, RgbImage};

use crate::cost::{Block, CostField};
use crate::generate::STEP;
use crate::select::Course;
use crate::terrain::ZoneTerrain;

type Px = Rgb<u8>;

const ROUTE: Px = Rgb([220, 40, 30]);
const OUTLINE: Px = Rgb([255, 255, 255]);
const DARK: Px = Rgb([20, 20, 20]);
const JUMP: Px = Rgb([255, 210, 0]);
const CHECK: Px = Rgb([30, 110, 230]);

/// The map preview: the course's bounding box plus a margin, at up to ~1100 px a side.
pub fn map(t: &ZoneTerrain, field: &CostField, c: &Course) -> RgbImage {
    let g = t.grid;
    let pts: Vec<Vec2> = c.route.iter().map(|p| Vec2::new(p[0], p[1])).collect();
    let (mut c0, mut r0, mut c1, mut r1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in &pts {
        let (pc, pr) = g.pixel_of(p.x, p.y);
        c0 = c0.min(pc);
        c1 = c1.max(pc);
        r0 = r0.min(pr);
        r1 = r1.max(pr);
    }
    let pad = 110.0 / g.yards_per_px;
    let c0 = (c0 - pad).max(0.0) as usize;
    let r0 = (r0 - pad).max(0.0) as usize;
    let c1 = ((c1 + pad) as usize).min(g.width - 1);
    let r1 = ((r1 + pad) as usize).min(g.height - 1);
    let (w, h) = (c1 - c0 + 1, r1 - r0 + 1);
    let base = base_map(t, field, c0, r0, w, h);
    let scale = (1100.0 / w.max(h) as f32).clamp(1.0, 4.0);
    let (sw, sh) = ((w as f32 * scale) as u32, (h as f32 * scale) as u32);
    let mut img = imageops::resize(&base, sw, sh, imageops::FilterType::Triangle);
    let to_px = |p: Vec2| {
        let (pc, pr) = g.pixel_of(p.x, p.y);
        Vec2::new((pc - c0 as f32) * scale, (pr - r0 as f32) * scale)
    };

    // The line: a white casing, then red, closing a circuit.
    let mut line_pts: Vec<Vec2> = pts.iter().map(|&p| to_px(p)).collect();
    if c.closed {
        line_pts.push(line_pts[0]);
    }
    let thick = (2.2 * scale.sqrt()).max(2.0);
    polyline(&mut img, &line_pts, thick + 2.0, OUTLINE);
    polyline(&mut img, &line_pts, thick, ROUTE);

    // Direction chevrons every ~150 yd.
    let every = (150.0 / STEP) as usize;
    for i in (every / 2..pts.len()).step_by(every) {
        let a = to_px(pts[i.saturating_sub(2)]);
        let b = to_px(pts[(i + 2).min(pts.len() - 1)]);
        let d = (b - a).normalize_or_zero();
        let tip = to_px(pts[i]) + d * 7.0;
        let side = d.perp() * 6.0;
        let back = tip - d * 10.0;
        thick_line(&mut img, back + side, tip, 2.0, OUTLINE);
        thick_line(&mut img, back - side, tip, 2.0, OUTLINE);
    }

    // Jumps.
    for j in &c.metrics.jumps {
        let q = to_px(pts[j.at.min(pts.len() - 1)]);
        triangle(&mut img, q + Vec2::new(0.0, -12.0), 7.0, JUMP);
    }

    // Checkpoints with their numbers; the start as a chequered bar.
    let fs = 2;
    for (k, cp) in c.checkpoints.iter().enumerate() {
        let q = to_px(Vec2::new(cp.x, cp.y));
        if k == 0 {
            continue;
        }
        disc(&mut img, q, 5.0, DARK);
        disc(&mut img, q, 4.0, CHECK);
        text(
            &mut img,
            q + Vec2::new(7.0, -14.0),
            &k.to_string(),
            fs,
            DARK,
            OUTLINE,
        );
    }
    if let Some(s) = c.checkpoints.first() {
        let q = to_px(Vec2::new(s.x, s.y));
        let d = Vec2::new(s.heading[0], s.heading[1]);
        // The heading is in world (x, y); on the image x runs down (−row) and y left (−col).
        let dir = Vec2::new(-d.y, -d.x).normalize_or_zero();
        let across = dir.perp();
        let half = (s.half_width / g.yards_per_px * scale).max(10.0);
        for k in -6..6 {
            for row in 0..2 {
                let a = q + across * (k as f32 / 6.0 * half) + dir * (row as f32 * 4.0 - 4.0);
                let b = a + across * (half / 6.0);
                let colour = if (k + row) % 2 == 0 { DARK } else { OUTLINE };
                thick_line(&mut img, a, b, 2.5, colour);
            }
        }
        text(
            &mut img,
            q + Vec2::new(10.0, 10.0),
            "START",
            2,
            DARK,
            OUTLINE,
        );
    }

    // The test drive's path, thin.
    if let Some(d) = &c.drive {
        let tr: Vec<Vec2> = d
            .trace
            .iter()
            .map(|p| to_px(Vec2::new(p[0], p[1])))
            .collect();
        polyline(&mut img, &tr, 0.8, Rgb([20, 40, 200]));
    }

    // Where the test drive stalled.
    for s in c.drive.iter().flat_map(|d| &d.stalls) {
        let q = to_px(Vec2::new(s[0], s[1]));
        thick_line(
            &mut img,
            q - Vec2::splat(8.0),
            q + Vec2::splat(8.0),
            2.0,
            Rgb([255, 0, 255]),
        );
        thick_line(
            &mut img,
            q + Vec2::new(-8.0, 8.0),
            q + Vec2::new(8.0, -8.0),
            2.0,
            Rgb([255, 0, 255]),
        );
    }

    // Title and key numbers.
    let title = format!(
        "{} - {:.2} KM {} - LAP {:.0}S",
        c.name.to_ascii_uppercase(),
        c.metrics.length_m / 1000.0,
        if c.closed { "CIRCUIT" } else { "SPRINT" },
        c.drive.as_ref().map_or(c.metrics.est_time, |d| d.best_lap)
    );
    let bw = (title.len() * 8 + 8) as u32;
    fill_rect(&mut img, 0, 0, bw.min(sw), 20, Rgb([250, 250, 245]));
    text(
        &mut img,
        Vec2::new(4.0, 3.0),
        &title,
        2,
        DARK,
        Rgb([250, 250, 245]),
    );
    img
}

/// The terrain under the preview at raster resolution.
fn base_map(
    t: &ZoneTerrain,
    field: &CostField,
    c0: usize,
    r0: usize,
    w: usize,
    h: usize,
) -> RgbImage {
    let g = t.grid;
    let res = g.yards_per_px;
    let hz = |c: usize, r: usize, fb: f32| {
        let v = t.height[r * g.width + c];
        if v.is_finite() {
            v
        } else {
            fb
        }
    };
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for r in r0..r0 + h {
        for c in c0..c0 + w {
            let v = t.height[r * g.width + c];
            if v.is_finite() {
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
    }
    let span = (hi - lo).max(1.0);
    let light = Vec2::new(-1.0, -1.0).normalize();
    let mut img = RgbImage::new(w as u32, h as u32);
    for r in 0..h {
        for c in 0..w {
            let (gc, gr) = (c0 + c, r0 + r);
            let i = gr * g.width + gc;
            let v = t.height[i];
            let mut rgb = if v.is_finite() {
                let dx = (hz((gc + 1).min(g.width - 1), gr, v) - hz(gc.saturating_sub(1), gr, v))
                    / (2.0 * res);
                let dy = (hz(gc, (gr + 1).min(g.height - 1), v) - hz(gc, gr.saturating_sub(1), v))
                    / (2.0 * res);
                let n = glam::Vec3::new(-dx, -dy, 1.0).normalize();
                let l = glam::Vec3::new(light.x, light.y, 1.4).normalize();
                let shade = n.dot(l).clamp(0.0, 1.0);
                // Low ground green, high ground tan.
                let e = (v - lo) / span;
                let tint = [120.0 + 80.0 * e, 160.0 + 30.0 * e, 100.0 + 40.0 * e];
                tint.map(|k| k * (0.35 + 0.85 * shade))
            } else {
                [0.0, 0.0, 0.0]
            };
            match field.block[i] {
                Block::Outside => rgb = rgb.map(|k| k * 0.45),
                Block::Steep => rgb = [rgb[0] * 0.8 + 20.0, rgb[1] * 0.7, rgb[2] * 0.7],
                _ => {}
            }
            if t.water[i] {
                rgb = [
                    rgb[0] * 0.25 + 30.0,
                    rgb[1] * 0.35 + 80.0,
                    rgb[2] * 0.3 + 170.0,
                ];
            }
            if t.solid[i] {
                rgb = [
                    rgb[0] * 0.55 + 30.0,
                    rgb[1] * 0.5 + 20.0,
                    rgb[2] * 0.45 + 10.0,
                ];
            }
            let rw = t.road[i].clamp(0.0, 1.0) * 0.75;
            if rw > 0.0 {
                rgb = [
                    rgb[0] * (1.0 - rw) + 225.0 * rw,
                    rgb[1] * (1.0 - rw) + 175.0 * rw,
                    rgb[2] * (1.0 - rw) + 70.0 * rw,
                ];
            }
            img.put_pixel(
                c as u32,
                r as u32,
                Rgb(rgb.map(|k| k.clamp(0.0, 255.0) as u8)),
            );
        }
    }
    img
}

/// The elevation profile: height against distance, jumps and checkpoints marked.
pub fn profile(c: &Course) -> RgbImage {
    let (w, h) = (1000u32, 300u32);
    let (ml, mr, mt, mb) = (60.0, 20.0, 34.0, 34.0);
    let mut img = RgbImage::from_pixel(w, h, Rgb([250, 250, 245]));
    let z: Vec<f32> = c.route.iter().map(|p| p[2]).collect();
    let n = z.len();
    let total = c.metrics.length_yd.max(1.0);
    let (lo, hi) = (c.metrics.elev_min, c.metrics.elev_max);
    let pad = ((hi - lo) * 0.15).max(3.0);
    let (lo, hi) = (lo - pad, hi + pad);
    let pw = w as f32 - ml - mr;
    let ph = h as f32 - mt - mb;
    let sx = |s: f32| ml + s / total * pw;
    let sy = |v: f32| mt + (1.0 - (v - lo) / (hi - lo)) * ph;
    // Grid lines every 5 or 10 yd of height.
    let stepz = if hi - lo > 60.0 { 10.0 } else { 5.0 };
    let mut gz = (lo / stepz).ceil() * stepz;
    while gz < hi {
        let y = sy(gz);
        thick_line(
            &mut img,
            Vec2::new(ml, y),
            Vec2::new(w as f32 - mr, y),
            0.5,
            Rgb([215, 215, 210]),
        );
        text(
            &mut img,
            Vec2::new(4.0, y - 5.0),
            &format!("{gz:.0}"),
            2,
            Rgb([90, 90, 90]),
            Rgb([250, 250, 245]),
        );
        gz += stepz;
    }
    // Filled terrain under the line.
    for px in 0..pw as usize {
        let s = px as f32 / pw * total;
        let i = ((s / STEP) as usize).min(n - 1);
        let y = sy(z[i]);
        for yy in y as u32..(h - mb as u32) {
            img.put_pixel(ml as u32 + px as u32, yy, Rgb([170, 200, 150]));
        }
    }
    let line: Vec<Vec2> = (0..n)
        .map(|i| Vec2::new(sx(i as f32 * STEP), sy(z[i])))
        .collect();
    polyline(&mut img, &line, 1.6, Rgb([40, 90, 40]));
    for (k, cp) in c.checkpoints.iter().enumerate() {
        let x = sx(cp.s);
        thick_line(
            &mut img,
            Vec2::new(x, h as f32 - mb),
            Vec2::new(x, h as f32 - mb + 6.0),
            1.0,
            CHECK,
        );
        if k > 0 {
            text(
                &mut img,
                Vec2::new(x - 4.0, h as f32 - mb + 9.0),
                &k.to_string(),
                2,
                CHECK,
                Rgb([250, 250, 245]),
            );
        }
    }
    for j in &c.metrics.jumps {
        let x = sx(j.s);
        let y = sy(z[j.at.min(n - 1)]);
        triangle(&mut img, Vec2::new(x, y - 10.0), 6.0, Rgb([230, 150, 0]));
    }
    let title = format!(
        "{} - ELEVATION {:.0}-{:.0} YD, CLIMB {:.0} YD/LAP, {} JUMPS",
        c.name.to_ascii_uppercase(),
        c.metrics.elev_min,
        c.metrics.elev_max,
        c.metrics.climb,
        c.metrics.jumps.len()
    );
    text(
        &mut img,
        Vec2::new(ml, 8.0),
        &title,
        2,
        DARK,
        Rgb([250, 250, 245]),
    );
    img
}

fn put(img: &mut RgbImage, x: i64, y: i64, c: Px) {
    if x >= 0 && y >= 0 && (x as u32) < img.width() && (y as u32) < img.height() {
        img.put_pixel(x as u32, y as u32, c);
    }
}

fn fill_rect(img: &mut RgbImage, x: u32, y: u32, w: u32, h: u32, c: Px) {
    for yy in y..(y + h).min(img.height()) {
        for xx in x..(x + w).min(img.width()) {
            img.put_pixel(xx, yy, c);
        }
    }
}

fn disc(img: &mut RgbImage, p: Vec2, r: f32, c: Px) {
    let k = r.ceil() as i64;
    for dy in -k..=k {
        for dx in -k..=k {
            if (dx * dx + dy * dy) as f32 <= r * r {
                put(img, p.x as i64 + dx, p.y as i64 + dy, c);
            }
        }
    }
}

fn thick_line(img: &mut RgbImage, a: Vec2, b: Vec2, r: f32, c: Px) {
    let steps = (a.distance(b) * 2.0).ceil().max(1.0) as usize;
    for s in 0..=steps {
        disc(img, a.lerp(b, s as f32 / steps as f32), r, c);
    }
}

fn polyline(img: &mut RgbImage, pts: &[Vec2], r: f32, c: Px) {
    for w in pts.windows(2) {
        thick_line(img, w[0], w[1], r, c);
    }
}

/// A filled downward-pointing triangle under `top`'s row, `r` across.
fn triangle(img: &mut RgbImage, top: Vec2, r: f32, c: Px) {
    let k = r.ceil() as i64;
    for dy in 0..=2 * k {
        let half = r * (1.0 - dy as f32 / (2.0 * r));
        for dx in -k..=k {
            if (dx as f32).abs() <= half {
                put(img, top.x as i64 + dx, top.y as i64 + dy, c);
            }
        }
    }
}

/// 3×5 glyphs, rows top to bottom, three bits a row (MSB left).
fn glyph(ch: char) -> [u8; 5] {
    match ch.to_ascii_uppercase() {
        '0' => [7, 5, 5, 5, 7],
        '1' => [2, 6, 2, 2, 7],
        '2' => [7, 1, 7, 4, 7],
        '3' => [7, 1, 7, 1, 7],
        '4' => [5, 5, 7, 1, 1],
        '5' => [7, 4, 7, 1, 7],
        '6' => [7, 4, 7, 5, 7],
        '7' => [7, 1, 1, 1, 1],
        '8' => [7, 5, 7, 5, 7],
        '9' => [7, 5, 7, 1, 7],
        'A' => [2, 5, 7, 5, 5],
        'B' => [6, 5, 6, 5, 6],
        'C' => [3, 4, 4, 4, 3],
        'D' => [6, 5, 5, 5, 6],
        'E' => [7, 4, 6, 4, 7],
        'F' => [7, 4, 6, 4, 4],
        'G' => [3, 4, 5, 5, 3],
        'H' => [5, 5, 7, 5, 5],
        'I' => [7, 2, 2, 2, 7],
        'J' => [1, 1, 1, 5, 2],
        'K' => [5, 5, 6, 5, 5],
        'L' => [4, 4, 4, 4, 7],
        'M' => [5, 7, 7, 5, 5],
        'N' => [6, 5, 5, 5, 5],
        'O' => [2, 5, 5, 5, 2],
        'P' => [6, 5, 6, 4, 4],
        'Q' => [2, 5, 5, 6, 3],
        'R' => [6, 5, 6, 5, 5],
        'S' => [3, 4, 2, 1, 6],
        'T' => [7, 2, 2, 2, 2],
        'U' => [5, 5, 5, 5, 7],
        'V' => [5, 5, 5, 5, 2],
        'W' => [5, 5, 7, 7, 5],
        'X' => [5, 5, 2, 5, 5],
        'Y' => [5, 5, 2, 2, 2],
        'Z' => [7, 1, 2, 4, 7],
        '.' => [0, 0, 0, 0, 2],
        ',' => [0, 0, 0, 2, 4],
        '-' => [0, 0, 7, 0, 0],
        ':' => [0, 2, 0, 2, 0],
        '/' => [1, 1, 2, 4, 4],
        '%' => [5, 1, 2, 4, 5],
        '\'' => [2, 2, 0, 0, 0],
        '&' => [2, 5, 2, 5, 3],
        '(' => [2, 4, 4, 4, 2],
        ')' => [2, 1, 1, 1, 2],
        _ => [0; 5],
    }
}

/// Text at `at` (top left), each font pixel `s`×`s`, with a one-pixel halo in `bg`.
fn text(img: &mut RgbImage, at: Vec2, s: &str, scale: i64, fg: Px, bg: Px) {
    for pass in 0..2 {
        for (k, ch) in s.chars().enumerate() {
            let g = glyph(ch);
            let x0 = at.x as i64 + k as i64 * 4 * scale;
            for (row, bits) in g.iter().enumerate() {
                for col in 0..3 {
                    if bits & (4 >> col) == 0 {
                        continue;
                    }
                    for dy in 0..scale {
                        for dx in 0..scale {
                            let (x, y) =
                                (x0 + col * scale + dx, at.y as i64 + row as i64 * scale + dy);
                            if pass == 0 {
                                for (ox, oy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                                    put(img, x + ox, y + oy, bg);
                                }
                            } else {
                                put(img, x, y, fg);
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The whole zone's drivability field: grey by cost, red steep, blue water, brown obstacle
/// margins, black outside, with the hubs.
pub fn field(f: &CostField, hubs: &[crate::generate::Hub]) -> RgbImage {
    let g = f.grid;
    let mut img = RgbImage::new(g.width as u32, g.height as u32);
    for (i, b) in f.block.iter().enumerate() {
        let c = match b {
            Block::Clear => {
                let v = (255.0 - (f.cost[i] - 0.35) * 70.0).clamp(60.0, 255.0) as u8;
                Rgb([v, v, v])
            }
            Block::Outside => Rgb([0, 0, 0]),
            Block::Water => Rgb([40, 90, 200]),
            Block::Steep => Rgb([170, 50, 40]),
            Block::Obstacle => Rgb([120, 80, 30]),
        };
        img.put_pixel((i % g.width) as u32, (i / g.width) as u32, c);
    }
    for h in hubs {
        let (c, r) = g.pixel_of(h.x, h.y);
        disc(&mut img, Vec2::new(c, r), 4.0, Rgb([255, 0, 255]));
    }
    img
}
