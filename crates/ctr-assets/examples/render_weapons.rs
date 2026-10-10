//! Renders `weapons.json` for visual verification: one PNG per model with up to 8 of its
//! animation frames (evenly spaced), seen from the front-left and the back-right, a quarter
//! view from above (orthographic, z-buffered, PSX blend modes on a gray background; texel
//! alpha 255 draws opaque, 128 blends, 0 is skipped).
//!
//! `cargo run -p ctr-assets --example render_weapons -- kart-data/weapons.json out_dir`

use serde_json::Value;

const CELL: usize = 200;
const MAX_FRAMES: usize = 8;

struct Img {
    w: usize,
    h: usize,
    px: Vec<[f32; 3]>,
    z: Vec<f32>,
}

struct Tex {
    w: usize,
    h: usize,
    rgba: Vec<u8>,
}

fn f(v: &Value) -> f32 {
    v.as_f64().unwrap_or(0.0) as f32
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

/// Draws one triangle; `p` is screen x, y and depth (smaller is nearer). Blended surfaces
/// test depth without writing it.
fn tri(
    img: &mut Img,
    p: [[f32; 3]; 3],
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
            let i = y as usize * img.w + x as usize;
            let depth = p[0][2] * w0 + p[1][2] * w1 + p[2][2] * w2;
            if depth > img.z[i] {
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
            if a == 255 || mode == "opaque" {
                img.z[i] = depth;
            }
            blend(&mut img.px[i], src, mode, a);
        }
    }
}

/// Orthographic view: yaw about +Y, then pitch down; returns screen right, up and depth
/// (distance away from the camera, which sits towards +Z and above).
fn view(q: [f32; 3], yaw: f32, pitch: f32) -> [f32; 3] {
    let (sy, cy) = yaw.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    let x = q[0] * cy - q[2] * sy;
    let z = q[0] * sy + q[2] * cy;
    let y = q[1] * cp - z * sp;
    let d = -(q[1] * sp + z * cp);
    [x, y, d]
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let j: Value = serde_json::from_slice(&std::fs::read(&args[1]).unwrap()).unwrap();
    let dir = std::path::Path::new(&args[2]);
    std::fs::create_dir_all(dir).unwrap();
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
    let views = [
        (35f32.to_radians(), 25f32.to_radians()),
        (215f32.to_radians(), 25f32.to_radians()),
    ];
    for (key, model) in j["models"].as_object().unwrap() {
        let frames = model["frames"].as_array().unwrap();
        let shown: Vec<usize> = if frames.len() <= MAX_FRAMES {
            (0..frames.len()).collect()
        } else {
            (0..MAX_FRAMES)
                .map(|i| i * frames.len() / MAX_FRAMES)
                .collect()
        };
        // Fit the model's bounds (over all frames) into a cell.
        let (lo, hi) = (&model["bounds"]["min"], &model["bounds"]["max"]);
        let centre: [f32; 3] = std::array::from_fn(|k| (f(&lo[k]) + f(&hi[k])) / 2.0);
        let radius = (0..3)
            .map(|k| (f(&hi[k]) - f(&lo[k])) / 2.0)
            .map(|r| r * r)
            .sum::<f32>()
            .sqrt()
            .max(1e-3);
        let scale = CELL as f32 * 0.45 / radius;
        let (w, h) = (CELL * shown.len(), CELL * views.len());
        let mut img = Img {
            w,
            h,
            px: vec![[0.45, 0.45, 0.47]; w * h],
            z: vec![f32::MAX; w * h],
        };
        for (col, &fi) in shown.iter().enumerate() {
            for (row, &(yaw, pitch)) in views.iter().enumerate() {
                let (ox, oy) = (
                    (col * CELL + CELL / 2) as f32,
                    (row * CELL + CELL / 2) as f32,
                );
                let surfaces = frames[fi]["surfaces"].as_array().unwrap();
                // Opaque surfaces first so blended ones test against them.
                for pass in 0..2 {
                    for s in surfaces {
                        let mode = s["blend"].as_str().unwrap();
                        if (mode == "opaque") != (pass == 0) {
                            continue;
                        }
                        let t = s["texture"].as_u64().map(|i| &texs[i as usize]);
                        let v = s["vertices"].as_array().unwrap();
                        for idx in s["indices"].as_array().unwrap().chunks(3) {
                            let g = |k: usize| &v[idx[k].as_u64().unwrap() as usize];
                            let p = [0, 1, 2].map(|k| {
                                let q = &g(k)["position"];
                                let q = [0, 1, 2].map(|a| f(&q[a]) - centre[a]);
                                let [x, y, d] = view(q, yaw, pitch);
                                [ox + x * scale, oy - y * scale, d]
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
        }
        let path = dir.join(format!("{key}.png"));
        let file = std::fs::File::create(&path).unwrap();
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let data: Vec<u8> = img
            .px
            .iter()
            .flat_map(|p| p.map(|c| (c.clamp(0.0, 1.0) * 255.0) as u8))
            .collect();
        enc.write_header().unwrap().write_image_data(&data).unwrap();
        println!(
            "wrote {} ({} of {} frames)",
            path.display(),
            shown.len(),
            frames.len()
        );
    }
}
