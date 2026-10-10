//! Renders CTR's 1P race HUD from `hud.json` through the library renderer
//! ([`ctr_assets::hud_draw`]) with sample values (speed near the top, drift meter half full in
//! the red zone, 1.2 s airtime, 7 wumpa, lap 2/3, 1st place, 3 bombs), composited over a
//! simple sky/road backdrop, at the native 512×216 and as a 4:3-corrected 576×432 image.
//! Also writes the raw transparent overlay (`*-overlay.png`, RGBA) and prints render timing.
//!
//! `cargo run -p ctr-assets --release --example render_hud -- kart-data/hud.json out.png`

use ctr_assets::hud_draw::{HudRenderer, HudState, HEIGHT, WIDTH};

fn save(path: &str, w: usize, h: usize, color: png::ColorType, data: &[u8]) {
    let file = std::fs::File::create(path).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(color);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(data).unwrap();
    println!("wrote {path}");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let hud = HudRenderer::load(args[1].as_ref()).expect("hud.json");
    let state = HudState {
        speedometer: true,
        speed: 9000.0,
        accel_speed: 10000.0,
        sacred_fire_speed: 2000.0,
        drift_meter: Some((490, 1000, 500)),
        jump: Some(1250),
        wumpa: Some(7),
        lap: Some((2, 3)),
        place: Some(1),
        item: Some((10, 3)),
        ranks: ["crash", "coco", "cortex", "tiny"]
            .map(String::from)
            .to_vec(),
        time_seconds: 0.4,
        ..Default::default()
    };
    let mut overlay = vec![0u8; WIDTH * HEIGHT * 4];
    let t = std::time::Instant::now();
    let n = 200;
    for _ in 0..n {
        hud.render(&state, &mut overlay);
    }
    println!(
        "render: {:.3} ms per frame",
        t.elapsed().as_secs_f64() * 1000.0 / n as f64
    );
    save(
        &args[2].replace(".png", "-overlay.png"),
        WIDTH,
        HEIGHT,
        png::ColorType::Rgba,
        &overlay,
    );

    // Composite over a backdrop.
    let mut rgb = vec![0u8; WIDTH * HEIGHT * 3];
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let t = y as f32 / HEIGHT as f32;
            let bg = if t < 0.45 {
                [0.35 + 0.3 * t, 0.55 + 0.3 * t, 0.85]
            } else {
                [0.32, 0.30, 0.28 + 0.1 * ((x % 64 < 32) as i32 as f32)]
            };
            let o = (y * WIDTH + x) * 4;
            let a = overlay[o + 3] as f32 / 255.0;
            for k in 0..3 {
                rgb[(y * WIDTH + x) * 3 + k] =
                    ((overlay[o + k] as f32 / 255.0 * a + bg[k] * (1.0 - a)) * 255.0) as u8;
            }
        }
    }
    save(&args[2], WIDTH, HEIGHT, png::ColorType::Rgb, &rgb);
    let (cw, ch) = (576, 432);
    let mut corrected = Vec::with_capacity(cw * ch * 3);
    for y in 0..ch {
        for x in 0..cw {
            let o = ((y * HEIGHT / ch) * WIDTH + x * WIDTH / cw) * 3;
            corrected.extend_from_slice(&rgb[o..o + 3]);
        }
    }
    save(
        &args[2].replace(".png", "-4x3.png"),
        cw,
        ch,
        png::ColorType::Rgb,
        &corrected,
    );
    for r in hud.regions() {
        println!("region {:12} {:?} {:?}", r.name, r.rect, r.anchor);
    }
}
