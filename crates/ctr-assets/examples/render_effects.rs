//! Renders `effects.json` for visual verification: the 8 turbo flame frames from the side and
//! from behind (orthographic, PSX blend modes on a gray background; texel alpha 255 draws
//! opaque, 128 blends, 0 is skipped), an exhaust-smoke puff over its lifetime in both blend
//! modes, and every texture enlarged.
//!
//! `cargo run -p ctr-assets --example render_effects -- kart-data/effects.json out.png`

use serde_json::Value;

const CELL: usize = 160;

struct Img {
    w: usize,
    h: usize,
    px: Vec<[f32; 3]>,
}

fn f(v: &Value) -> f32 {
    v.as_f64().unwrap_or(0.0) as f32
}

struct Tex {
    w: usize,
    h: usize,
    rgba: Vec<u8>,
}

fn blend(dst: &mut [f32; 3], src: [f32; 3], mode: &str, alpha: u8) {
    if alpha == 255 || mode == "opaque" {
        *dst = src;
        return;
    }
    for k in 0..3 {
        dst[k] = match mode {
            "additive" => dst[k] + src[k],
            "subtractive" => dst[k] - src[k],
            "add_quarter" => dst[k] + src[k] * 0.25,
            _ => dst[k] * 0.5 + src[k] * 0.5,
        }
        .clamp(0.0, 1.0);
    }
}

/// Draws one triangle; `proj` maps meters to pixels.
fn tri(
    img: &mut Img,
    p: [[f32; 2]; 3],
    uv: [[f32; 2]; 3],
    col: [[f32; 3]; 3],
    tex: Option<&Tex>,
    mode: &str,
) {
    let area =
        (p[1][0] - p[0][0]) * (p[2][1] - p[0][1]) - (p[1][1] - p[0][1]) * (p[2][0] - p[0][0]);
    if area.abs() < 1e-6 {
        return;
    }
    let minx = p.iter().map(|q| q[0]).fold(f32::MAX, f32::min).max(0.0) as usize;
    let maxx = (p
        .iter()
        .map(|q| q[0])
        .fold(f32::MIN, f32::max)
        .min(img.w as f32 - 1.0)) as isize;
    let miny = p.iter().map(|q| q[1]).fold(f32::MAX, f32::min).max(0.0) as usize;
    let maxy = (p
        .iter()
        .map(|q| q[1])
        .fold(f32::MIN, f32::max)
        .min(img.h as f32 - 1.0)) as isize;
    for y in miny as isize..=maxy {
        for x in minx as isize..=maxx {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let w0 = ((p[1][0] - px) * (p[2][1] - py) - (p[1][1] - py) * (p[2][0] - px)) / area;
            let w1 = ((p[2][0] - px) * (p[0][1] - py) - (p[2][1] - py) * (p[0][0] - px)) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                continue;
            }
            let c: [f32; 3] =
                std::array::from_fn(|k| col[0][k] * w0 + col[1][k] * w1 + col[2][k] * w2);
            let (src, a) = match tex {
                Some(t) => {
                    let u = uv[0][0] * w0 + uv[1][0] * w1 + uv[2][0] * w2;
                    let v = uv[0][1] * w0 + uv[1][1] * w1 + uv[2][1] * w2;
                    let tu = ((u * t.w as f32) as usize).min(t.w - 1);
                    let tv = ((v * t.h as f32) as usize).min(t.h - 1);
                    let o = (tv * t.w + tu) * 4;
                    if t.rgba[o + 3] == 0 {
                        continue;
                    }
                    (
                        std::array::from_fn(|k| t.rgba[o + k] as f32 / 255.0 * c[k]),
                        t.rgba[o + 3],
                    )
                }
                None => (c, 255),
            };
            blend(&mut img.px[y as usize * img.w + x as usize], src, mode, a);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let j: Value = serde_json::from_slice(&std::fs::read(&args[1]).unwrap()).unwrap();
    let texs: Vec<Tex> = j["textures"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| Tex {
            w: t["width"].as_u64().unwrap() as usize,
            h: t["height"].as_u64().unwrap() as usize,
            rgba: t["rgba"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_u64().unwrap() as u8)
                .collect(),
        })
        .collect();
    let frames = j["turbo_flame"]["frames"].as_array().unwrap();
    let tex_rows = 2;
    let mut img = Img {
        w: CELL * 8,
        h: CELL * 3 + 140 * tex_rows,
        px: vec![[0.45, 0.45, 0.47]; CELL * 8 * (CELL * 3 + 140 * tex_rows)],
    };
    // Row 0: side view (camera on +X: screen x = -z (rear to the right), y up).
    // Row 1: rear view (camera behind: screen x = -x... mirrored so +X (left) is screen left).
    let scale = 60.0; // pixels per meter
    for (fi, fr) in frames.iter().enumerate() {
        for row in 0..2 {
            let (ox, oy) = (fi * CELL + CELL / 2, row * CELL + CELL / 2);
            for s in fr["surfaces"].as_array().unwrap() {
                let mode = s["blend"].as_str().unwrap();
                let t = s["texture"].as_u64().map(|i| &texs[i as usize]);
                let v = s["vertices"].as_array().unwrap();
                for idx in s["indices"].as_array().unwrap().chunks(3) {
                    let g = |k: usize| &v[idx[k].as_u64().unwrap() as usize];
                    let p = [0, 1, 2].map(|k| {
                        let q = &g(k)["position"];
                        let sx = if row == 0 { -f(&q[2]) } else { -f(&q[0]) };
                        [
                            ox as f32 + sx * scale - 30.0,
                            oy as f32 - f(&q[1]) * scale + 40.0,
                        ]
                    });
                    let uv = [0, 1, 2].map(|k| [f(&g(k)["uv"][0]), f(&g(k)["uv"][1])]);
                    let col = [0, 1, 2].map(|k| {
                        [
                            f(&g(k)["color"][0]),
                            f(&g(k)["color"][1]),
                            f(&g(k)["color"][2]),
                        ]
                    });
                    tri(&mut img, p, uv, col, t, mode);
                }
            }
        }
    }
    // Row 2: exhaust puff over its life (6 frames), additive then subtractive.
    let ex = &j["exhaust_smoke"];
    let t = &texs[ex["texture"].as_u64().unwrap() as usize];
    for (mi, mode) in ["additive", "subtractive"].iter().enumerate() {
        for k in 0..6 {
            let half = (f(&ex["size"]["half_extent_m_start"])
                + f(&ex["size"]["half_extent_m_per_second"]) * k as f32 / 30.0)
                * 110.0;
            let gray =
                f(&ex["color"]["start_gray"]) + f(&ex["color"]["per_second"]) * k as f32 / 30.0;
            let (cx, cy) = (
                (mi * 6 + k) as f32 * 105.0 + 60.0,
                (2 * CELL + CELL / 2) as f32,
            );
            let c = [[gray * 255.0 / 128.0 * 2.0; 3]; 3];
            let q = [
                [cx - half, cy - half],
                [cx + half, cy - half],
                [cx + half, cy + half],
                [cx - half, cy + half],
            ];
            tri(
                &mut img,
                [q[0], q[1], q[2]],
                [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]],
                c,
                Some(t),
                mode,
            );
            tri(
                &mut img,
                [q[0], q[2], q[3]],
                [[0.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
                c,
                Some(t),
                mode,
            );
        }
    }
    // Texture strip, enlarged (4x), on black; alpha 0 shown as magenta.
    let mut x0 = 4;
    let mut y0 = CELL * 3 + 4;
    for t in &texs {
        let z = if t.w <= 16 && t.h <= 8 { 8 } else { 4 };
        if x0 + t.w * z > img.w {
            x0 = 4;
            y0 += 140;
        }
        for y in 0..t.h * z {
            for x in 0..t.w * z {
                let o = ((y / z) * t.w + x / z) * 4;
                let c = if t.rgba[o + 3] == 0 {
                    [0.6, 0.0, 0.6]
                } else {
                    [
                        t.rgba[o] as f32 / 255.0,
                        t.rgba[o + 1] as f32 / 255.0,
                        t.rgba[o + 2] as f32 / 255.0,
                    ]
                };
                if y0 + y < img.h {
                    img.px[(y0 + y) * img.w + x0 + x] = c;
                }
            }
        }
        x0 += t.w * z + 8;
    }
    let file = std::fs::File::create(&args[2]).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), img.w as u32, img.h as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let data: Vec<u8> = img
        .px
        .iter()
        .flat_map(|p| p.map(|c| (c.clamp(0.0, 1.0) * 255.0) as u8))
        .collect();
    enc.write_header().unwrap().write_image_data(&data).unwrap();
    println!("wrote {}", args[2]);
}
