//! Software-renders a `kart.json` to a PNG for visual verification.
//!
//! `cargo run -p ctr-assets --example render_kart -- <kart.json> <out.png> [--disc <disc.bin>]`
//!
//! Row 1: the exported kart (what the client gets) from four views, with wheels drawn as
//! side-on sprite discs and anchor markers (seat = magenta, steering wheel = cyan, wheel
//! centres = yellow). With `--disc`, row 2 renders the same racer's full model (kart + driver)
//! for comparison. The last strip shows the 17 tire sprites.

use std::path::PathBuf;

use serde_json::Value;

const VIEW: usize = 400;

#[derive(Clone)]
struct Tex {
    w: usize,
    h: usize,
    rgba: Vec<u8>,
}

struct Tri {
    p: [[f32; 3]; 3],
    uv: [[f32; 2]; 3],
    c: [[f32; 4]; 3],
    tex: Option<usize>,
    double_sided: bool,
    blend: bool,
}

struct Scene {
    tris: Vec<Tri>,
    textures: Vec<Tex>,
    markers: Vec<([f32; 3], [u8; 3])>,
}

fn f(v: &Value) -> f32 {
    v.as_f64().unwrap_or(0.0) as f32
}
fn v3(v: &Value) -> [f32; 3] {
    [f(&v[0]), f(&v[1]), f(&v[2])]
}

fn load_scene(j: &Value, wheels: bool) -> Scene {
    let textures: Vec<Tex> = j["textures"]
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
    let mut tris = Vec::new();
    for s in j["surfaces"].as_array().unwrap() {
        let verts = s["vertices"].as_array().unwrap();
        let idx: Vec<usize> = s["indices"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_u64().unwrap() as usize)
            .collect();
        for t in idx.chunks(3) {
            let get = |k: usize| &verts[t[k]];
            tris.push(Tri {
                p: [
                    v3(&get(0)["position"]),
                    v3(&get(1)["position"]),
                    v3(&get(2)["position"]),
                ],
                uv: [0, 1, 2].map(|k| [f(&get(k)["uv"][0]), f(&get(k)["uv"][1])]),
                c: [0, 1, 2].map(|k| {
                    let c = &get(k)["color"];
                    [f(&c[0]), f(&c[1]), f(&c[2]), f(&c[3])]
                }),
                tex: s["texture"].as_u64().map(|x| x as usize),
                double_sided: s["double_sided"].as_bool().unwrap_or(true),
                blend: s["blend"] == "blend",
            });
        }
    }
    let mut markers = Vec::new();
    if wheels {
        let side = j["wheel_sprites"]["textures"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()
            .as_u64()
            .unwrap() as usize;
        for w in j["wheels"].as_array().unwrap() {
            let c = v3(&w["center"]);
            let r = f(&w["radius"]);
            // Quad in the plane perpendicular to the axle (X).
            let q = [
                [c[0], c[1] + r, c[2] - r],
                [c[0], c[1] + r, c[2] + r],
                [c[0], c[1] - r, c[2] + r],
                [c[0], c[1] - r, c[2] - r],
            ];
            let uv = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
            for (a, b, cc) in [(0, 1, 2), (0, 2, 3)] {
                tris.push(Tri {
                    p: [q[a], q[b], q[cc]],
                    uv: [uv[a], uv[b], uv[cc]],
                    c: [[1.0; 4]; 3],
                    tex: Some(side),
                    double_sided: true,
                    blend: false,
                });
            }
            markers.push((c, [255, 255, 0]));
        }
        markers.push((v3(&j["anchors"]["seat"]), [255, 0, 255]));
        markers.push((v3(&j["anchors"]["steering_wheel"]), [0, 255, 255]));
    }
    // Ground plane checker at y = 0.
    let g = 1.4f32;
    let n = 7;
    for i in 0..n {
        for k in 0..n {
            let x0 = -g + 2.0 * g * i as f32 / n as f32;
            let z0 = -g + 2.0 * g * k as f32 / n as f32;
            let s = 2.0 * g / n as f32;
            let shade = if (i + k) % 2 == 0 { 0.55 } else { 0.45 };
            let col = [[shade, shade, shade * 1.05, 1.0]; 3];
            let a = [x0, 0.0, z0];
            let b = [x0 + s, 0.0, z0];
            let c = [x0 + s, 0.0, z0 + s];
            let d = [x0, 0.0, z0 + s];
            tris.push(Tri {
                p: [a, c, b],
                uv: [[0.0; 2]; 3],
                c: col,
                tex: None,
                double_sided: true,
                blend: false,
            });
            tris.push(Tri {
                p: [a, d, c],
                uv: [[0.0; 2]; 3],
                c: col,
                tex: None,
                double_sided: true,
                blend: false,
            });
        }
    }
    Scene {
        tris,
        textures,
        markers,
    }
}

struct Image {
    w: usize,
    h: usize,
    px: Vec<[u8; 3]>,
}

impl Image {
    fn new(w: usize, h: usize) -> Self {
        Self {
            w,
            h,
            px: vec![[40, 44, 56]; w * h],
        }
    }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn norm(a: [f32; 3]) -> [f32; 3] {
    let l = dot(a, a).sqrt();
    [a[0] / l, a[1] / l, a[2] / l]
}

/// Renders `scene` into the `VIEW`-sized cell at (ox, oy) from `eye` looking at `target`.
fn render(img: &mut Image, ox: usize, oy: usize, scene: &Scene, eye: [f32; 3], target: [f32; 3]) {
    let fwd = norm(sub(target, eye));
    let right = norm(cross(fwd, [0.0, 1.0, 0.0]));
    let up = cross(right, fwd);
    let focal = 1.0 / (30f32.to_radians()).tan();
    let project = |p: [f32; 3]| -> Option<[f32; 3]> {
        let d = sub(p, eye);
        let (x, y, z) = (dot(d, right), dot(d, up), dot(d, fwd));
        if z < 0.05 {
            return None;
        }
        let sx = (x * focal / z * 0.5 + 0.5) * VIEW as f32;
        let sy = (0.5 - y * focal / z * 0.5) * VIEW as f32;
        Some([sx, sy, z])
    };
    let mut depth = vec![f32::MAX; VIEW * VIEW];
    // Two passes: opaque, then blended.
    for pass in 0..2 {
        for t in &scene.tris {
            if (pass == 1) != t.blend {
                continue;
            }
            let (Some(a), Some(b), Some(c)) = (project(t.p[0]), project(t.p[1]), project(t.p[2]))
            else {
                continue;
            };
            // Signed area on a y-down screen: CCW (front) faces are negative.
            let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
            if area.abs() < 1e-6 || (!t.double_sided && area > 0.0) {
                continue;
            }
            let minx = a[0].min(b[0]).min(c[0]).floor().max(0.0) as usize;
            let maxx = (a[0].max(b[0]).max(c[0]).ceil() as isize).min(VIEW as isize - 1);
            let miny = a[1].min(b[1]).min(c[1]).floor().max(0.0) as usize;
            let maxy = (a[1].max(b[1]).max(c[1]).ceil() as isize).min(VIEW as isize - 1);
            if maxx < 0 || maxy < 0 {
                continue;
            }
            for y in miny..=maxy as usize {
                for x in minx..=maxx as usize {
                    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                    let w0 = ((b[0] - px) * (c[1] - py) - (b[1] - py) * (c[0] - px)) / area;
                    let w1 = ((c[0] - px) * (a[1] - py) - (c[1] - py) * (a[0] - px)) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    // Perspective-correct weights.
                    let (i0, i1, i2) = (w0 / a[2], w1 / b[2], w2 / c[2]);
                    let iz = i0 + i1 + i2;
                    let z = 1.0 / iz;
                    let di = y * VIEW + x;
                    if z >= depth[di] {
                        continue;
                    }
                    let (k0, k1, k2) = (i0 * z, i1 * z, i2 * z);
                    let mut col: [f32; 4] = std::array::from_fn(|ch| {
                        t.c[0][ch] * k0 + t.c[1][ch] * k1 + t.c[2][ch] * k2
                    });
                    let mut alpha = 1.0;
                    if let Some(ti) = t.tex {
                        let tx = &scene.textures[ti];
                        let u = t.uv[0][0] * k0 + t.uv[1][0] * k1 + t.uv[2][0] * k2;
                        let v = t.uv[0][1] * k0 + t.uv[1][1] * k1 + t.uv[2][1] * k2;
                        let tu = ((u * tx.w as f32).floor() as isize).clamp(0, tx.w as isize - 1)
                            as usize;
                        let tv = ((v * tx.h as f32).floor() as isize).clamp(0, tx.h as isize - 1)
                            as usize;
                        let o = (tv * tx.w + tu) * 4;
                        let texel = &tx.rgba[o..o + 4];
                        if texel[3] == 0 {
                            continue;
                        }
                        for ch in 0..3 {
                            col[ch] *= texel[ch] as f32 / 255.0;
                        }
                        if texel[3] < 255 && t.blend {
                            alpha = 0.5;
                        }
                    }
                    let pi = (oy + y) * img.w + ox + x;
                    let dst = img.px[pi];
                    for ch in 0..3 {
                        let s = (col[ch].clamp(0.0, 1.0) * 255.0) * alpha
                            + dst[ch] as f32 * (1.0 - alpha);
                        img.px[pi][ch] = s as u8;
                    }
                    if alpha >= 1.0 {
                        depth[di] = z;
                    }
                }
            }
        }
    }
    for (p, col) in &scene.markers {
        if let Some(s) = project(*p) {
            for dy in -3i32..=3 {
                for dx in -3i32..=3 {
                    let (x, y) = (s[0] as i32 + dx, s[1] as i32 + dy);
                    if x >= 0
                        && y >= 0
                        && (x as usize) < VIEW
                        && (y as usize) < VIEW
                        && (dx.abs() == 3 || dy.abs() == 3 || dx == 0 || dy == 0)
                    {
                        img.px[(oy + y as usize) * img.w + ox + x as usize] = *col;
                    }
                }
            }
        }
    }
}

fn views() -> [([f32; 3], [f32; 3]); 4] {
    let t = [0.0, 0.3, 0.05];
    [
        ([1.7, 1.1, 2.2], t),   // front three-quarter, from the kart's left (+X)
        ([3.0, 0.45, 0.05], t), // left side (front = +Z = screen left)
        ([-1.7, 1.2, -2.2], t), // rear three-quarter, from the kart's right
        ([0.01, 3.2, 0.3], t),  // top (front = +Z = screen bottom)
    ]
}

fn main() {
    let mut pos = Vec::new();
    let mut disc = None;
    let mut effects = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--disc" {
            disc = args.next().map(PathBuf::from);
        } else if a == "--effects" {
            effects = args.next().map(PathBuf::from);
        } else {
            pos.push(PathBuf::from(a));
        }
    }
    if pos.first().is_some_and(|p| p.as_os_str() == "--sheet") {
        // Contact sheet: `--sheet <a.json> <b.json> ... <out.png>`, one front three-quarter
        // view per kart, four per row.
        let out = pos.pop().unwrap();
        let files = &pos[1..];
        let cols = 4;
        let rows = files.len().div_ceil(cols);
        let mut img = Image::new(VIEW * cols, VIEW * rows);
        for (i, f) in files.iter().enumerate() {
            let j: Value = serde_json::from_slice(&std::fs::read(f).expect("read")).expect("parse");
            let (eye, target) = views()[0];
            render(
                &mut img,
                (i % cols) * VIEW,
                (i / cols) * VIEW,
                &load_scene(&j, true),
                eye,
                target,
            );
        }
        save(&img, &out);
        return;
    }
    let kart: Value =
        serde_json::from_slice(&std::fs::read(&pos[0]).expect("read kart.json")).expect("parse");
    let mut rows = vec![load_scene(&kart, true)];
    if let Some(e) = effects {
        // Turbo flames (fire size 8) at their origins and smoke spawn points as white markers.
        let fx: Value = serde_json::from_slice(&std::fs::read(e).expect("effects")).expect("parse");
        let scene = &mut rows[0];
        let base = scene.textures.len();
        for t in fx["textures"].as_array().unwrap() {
            scene.textures.push(Tex {
                w: t["width"].as_u64().unwrap() as usize,
                h: t["height"].as_u64().unwrap() as usize,
                rgba: t["rgba"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|x| x.as_u64().unwrap() as u8)
                    .collect(),
            });
        }
        let frames = fx["turbo_flame"]["frames"].as_array().unwrap();
        for (inst, frame) in fx["turbo_flame"]["instances"]
            .as_array()
            .unwrap()
            .iter()
            .zip([0usize, 3])
        {
            let o = v3(&inst["origin"]);
            let mx = if inst["mirror_x"].as_bool().unwrap() {
                -1.0
            } else {
                1.0
            };
            for srf in frames[frame]["surfaces"].as_array().unwrap() {
                let v = srf["vertices"].as_array().unwrap();
                for idx in srf["indices"].as_array().unwrap().chunks(3) {
                    let g = |k: usize| &v[idx[k].as_u64().unwrap() as usize];
                    scene.tris.push(Tri {
                        p: [0, 1, 2].map(|k| {
                            let q = v3(&g(k)["position"]);
                            [o[0] + mx * q[0], o[1] + q[1], o[2] + q[2]]
                        }),
                        uv: [0, 1, 2].map(|k| [f(&g(k)["uv"][0]), f(&g(k)["uv"][1])]),
                        c: [0, 1, 2].map(|k| {
                            let c = &g(k)["color"];
                            [f(&c[0]), f(&c[1]), f(&c[2]), 1.0]
                        }),
                        tex: srf["texture"].as_u64().map(|t| base + t as usize),
                        double_sided: true,
                        blend: true,
                    });
                }
            }
        }
        for a in kart["anchors"]["exhaust"].as_array().unwrap() {
            scene.markers.push((v3(a), [255, 255, 255]));
        }
    }
    if let Some(d) = disc {
        let mut disc = ctr_assets::disc::Disc::open(&d).expect("disc");
        let mut bf = ctr_assets::big::BigFile::open(&mut disc).expect("bigfile");
        let full = ctr_assets::kart::build_with(&mut bf, kart["kart"].as_str().unwrap(), true)
            .expect("full");
        let full: Value = serde_json::to_value(&full).unwrap();
        rows.push(load_scene(&full, true));
    }
    let sprite_h = 80;
    let mut img = Image::new(VIEW * 4, VIEW * rows.len() + sprite_h);
    for (r, scene) in rows.iter().enumerate() {
        for (i, (eye, target)) in views().iter().enumerate() {
            render(&mut img, i * VIEW, r * VIEW, scene, *eye, *target);
        }
    }
    // Tire sprites strip, 2x scale.
    let textures = &rows[0].textures;
    let sprites: Vec<usize> = kart["wheel_sprites"]["textures"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_u64().unwrap() as usize)
        .collect();
    let mut x0 = 4;
    let y0 = VIEW * rows.len() + 4;
    for s in sprites {
        let t = &textures[s];
        for y in 0..t.h * 2 {
            for x in 0..t.w * 2 {
                let o = ((y / 2) * t.w + x / 2) * 4;
                let (px, py) = (x0 + x, y0 + y);
                if px < img.w && py < img.h && t.rgba[o + 3] > 0 {
                    img.px[py * img.w + px] = [t.rgba[o], t.rgba[o + 1], t.rgba[o + 2]];
                }
            }
        }
        x0 += t.w * 2 + 6;
    }
    save(&img, &pos[1]);
}

fn save(img: &Image, path: &std::path::Path) {
    let file = std::fs::File::create(path).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), img.w as u32, img.h as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().unwrap();
    let data: Vec<u8> = img.px.iter().flatten().copied().collect();
    w.write_image_data(&data).unwrap();
    println!("wrote {}", path.display());
}
