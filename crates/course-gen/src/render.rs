//! The debug render: hillshaded terrain, water, the road mask and the traced graph, north up.

use image::{Rgb, RgbImage};

use crate::roads::{RoadGraph, RoadRasters};
use crate::terrain::ZoneTerrain;

/// A labelled world point drawn as a cross, to check the raster's orientation by eye.
pub struct Landmark {
    pub name: &'static str,
    pub x: f32,
    pub y: f32,
}

/// Render `terrain` with its road rasters and `graph` at one pixel per raster pixel.
pub fn render(
    terrain: &ZoneTerrain,
    rasters: &RoadRasters,
    graph: &RoadGraph,
    landmarks: &[Landmark],
) -> RgbImage {
    let g = terrain.grid;
    let (w, h) = (g.width, g.height);
    let res = g.yards_per_px;
    let mut img = RgbImage::new(w as u32, h as u32);
    let hgt = |c: usize, r: usize, fallback: f32| {
        let v = terrain.height[r * w + c];
        if v.is_finite() {
            v
        } else {
            fallback
        }
    };
    // Light from the north-west, 45° up; vectors are (east, south, up).
    let light = {
        let v = [-1.0f32, -1.0, 2.0f32.sqrt()];
        let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        [v[0] / n, v[1] / n, v[2] / n]
    };
    for r in 0..h {
        for c in 0..w {
            let i = r * w + c;
            let hc = terrain.height[i];
            let mut rgb = if hc.is_finite() {
                let dx = (hgt((c + 1).min(w - 1), r, hc) - hgt(c.saturating_sub(1), r, hc))
                    / (2.0 * res);
                let dy = (hgt(c, (r + 1).min(h - 1), hc) - hgt(c, r.saturating_sub(1), hc))
                    / (2.0 * res);
                let n = [-dx, -dy, 1.0];
                let len = (n[0] * n[0] + n[1] * n[1] + 1.0).sqrt();
                let shade =
                    ((n[0] * light[0] + n[1] * light[1] + n[2] * light[2]) / len).clamp(0.0, 1.0);
                let v = 40.0 + 190.0 * shade;
                [v, v, v]
            } else {
                [0.0, 0.0, 0.0]
            };
            if !terrain.in_zone[i] {
                rgb = rgb.map(|v| v * 0.55);
            }
            if terrain.water[i] {
                rgb = [
                    rgb[0] * 0.3 + 20.0,
                    rgb[1] * 0.4 + 60.0,
                    rgb[2] * 0.4 + 150.0,
                ];
            }
            // The raw road weight in amber, the cleaned mask in a stronger tint.
            let rw = terrain.road[i];
            if rw > 0.0 {
                let t = 0.55 * rw;
                rgb = [
                    rgb[0] * (1.0 - t) + 230.0 * t,
                    rgb[1] * (1.0 - t) + 170.0 * t,
                    rgb[2] * (1.0 - t) + 40.0 * t,
                ];
            }
            if rasters.mask.data[i] {
                rgb = [rgb[0] * 0.7 + 60.0, rgb[1] * 0.7 + 50.0, rgb[2] * 0.7];
            }
            img.put_pixel(
                c as u32,
                r as u32,
                Rgb(rgb.map(|v| v.clamp(0.0, 255.0) as u8)),
            );
        }
    }

    let to_px = |x: f32, y: f32| g.pixel_of(x, y);
    for e in &graph.edges {
        for s in e.points.windows(2) {
            let a = to_px(s[0].x, s[0].y);
            let b = to_px(s[1].x, s[1].y);
            line(&mut img, a, b, Rgb([230, 20, 20]));
        }
    }
    for n in &graph.nodes {
        let (c, r) = to_px(n.x, n.y);
        let colour = if n.degree == 1 {
            Rgb([255, 255, 255])
        } else {
            Rgb([20, 200, 255])
        };
        dot(&mut img, c, r, 2.5, colour);
    }
    for l in landmarks {
        let (c, r) = to_px(l.x, l.y);
        for d in -6..=6 {
            let d = d as f32;
            put(&mut img, c + d, r, Rgb([255, 0, 255]));
            put(&mut img, c, r + d, Rgb([255, 0, 255]));
        }
    }
    img
}

fn put(img: &mut RgbImage, c: f32, r: f32, colour: Rgb<u8>) {
    if c >= 0.0 && r >= 0.0 && (c as u32) < img.width() && (r as u32) < img.height() {
        img.put_pixel(c as u32, r as u32, colour);
    }
}

fn dot(img: &mut RgbImage, c: f32, r: f32, radius: f32, colour: Rgb<u8>) {
    let k = radius.ceil() as i32;
    for dr in -k..=k {
        for dc in -k..=k {
            if (dc * dc + dr * dr) as f32 <= radius * radius {
                put(img, c + dc as f32, r + dr as f32, colour);
            }
        }
    }
}

fn line(img: &mut RgbImage, a: (f32, f32), b: (f32, f32), colour: Rgb<u8>) {
    let steps = ((b.0 - a.0).abs().max((b.1 - a.1).abs()) * 2.0)
        .ceil()
        .max(1.0) as usize;
    for s in 0..=steps {
        let t = s as f32 / steps as f32;
        put(img, a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, colour);
    }
}
