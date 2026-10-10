//! End-to-end checks against the user's own disc image. Skipped unless `CTR_DISC` points at a
//! CTR NTSC-U `.bin`/`.iso`:
//!
//! `CTR_DISC="path/to/CTR - Crash Team Racing (USA).bin" cargo test -p ctr-assets --release`

use ctr_assets::big::BigFile;
use ctr_assets::disc::Disc;
use ctr_assets::kart;

fn disc() -> Option<Disc> {
    let path = std::env::var_os("CTR_DISC")?;
    Some(Disc::open(std::path::Path::new(&path)).expect("CTR_DISC does not open"))
}

#[test]
fn extracts_every_kart() {
    let Some(mut disc) = disc() else {
        eprintln!("CTR_DISC not set; skipping");
        return;
    };
    let mut bf = BigFile::open(&mut disc).expect("BIGFILE.BIG");
    assert!(bf.entries.len() > 400);
    for name in kart::KARTS {
        let k = kart::build(&mut bf, name).unwrap_or_else(|e| panic!("{name}: {e}"));
        // The kart is a minority of the racer mesh; the driver is the rest.
        assert!(
            (60..=100).contains(&k.source.triangles_kart),
            "{name}: {} kart triangles",
            k.source.triangles_kart
        );
        assert!(k.source.triangles_total > k.source.triangles_kart + 100);
        // Same kart geometry for everyone: ~1.75 m long, ~0.9 m wide, sitting on y = 0.
        let (lo, hi) = (k.bounds.min, k.bounds.max);
        assert!(
            (hi[2] - lo[2] - 1.75).abs() < 0.1,
            "{name}: length {}",
            hi[2] - lo[2]
        );
        assert!(
            (hi[0] - lo[0] - 0.88).abs() < 0.1,
            "{name}: width {}",
            hi[0] - lo[0]
        );
        assert!(lo[1] > 0.0 && lo[1] < 0.2, "{name}: floor {}", lo[1]);
        // Anchors inside the kart.
        let s = k.anchors.seat;
        assert!(
            s[1] > 0.1 && s[1] < 0.4 && s[2] > lo[2] && s[2] < hi[2],
            "{name}: seat {s:?}"
        );
        let w = k.anchors.steering_wheel;
        assert!(w[1] > s[1] && w[2] > s[2], "{name}: steering wheel {w:?}");
        // Textures decoded, and the 17 tire sprites.
        assert_eq!(k.wheel_sprites.textures.len(), 17);
        for t in &k.textures {
            assert_eq!(
                t.rgba.len() as u32,
                t.width * t.height * 4,
                "{name}: {}",
                t.name
            );
            assert!(
                t.rgba.chunks(4).any(|p| p[3] > 0),
                "{name}: {} is empty",
                t.name
            );
        }
        for s in &k.surfaces {
            assert_eq!(s.indices.len() % 3, 0);
            assert!(s.indices.iter().all(|&i| (i as usize) < s.vertices.len()));
            if let Some(t) = s.texture {
                assert!(t < k.textures.len());
            }
        }
    }
}

#[test]
fn extracts_kart_audio() {
    let Some(mut disc) = disc() else {
        eprintln!("CTR_DISC not set; skipping");
        return;
    };
    let dir = std::env::temp_dir().join(format!("ctr-audio-test-{}", std::process::id()));
    let out = ctr_assets::audio::extract(&mut disc, &dir).expect("audio");
    assert_eq!(out.len(), ctr_assets::audio::SOUNDS.len());
    for r in &out {
        let secs = r.samples as f64 / r.rate as f64;
        assert!((0.1..3.0).contains(&secs), "{}: {secs} s", r.file);
        assert!(r.peak > 8000, "{}: peak {}", r.file, r.peak);
        let looped = matches!(
            r.file,
            "engine.wav"
                | "skid.wav"
                | "turbo.wav"
                | "roulette.wav"
                | "bomb_roll.wav"
                | "missile_fly.wav"
                | "shield_roll.wav"
                | "warp_fly.wav"
        );
        assert_eq!(r.looped, looped, "{}", r.file);
        assert!(dir.join(r.file).is_file());
    }
    assert!(dir.join("sounds.json").is_file());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn extracts_weapons() {
    let Some(mut disc) = disc() else {
        eprintln!("CTR_DISC not set; skipping");
        return;
    };
    let mut bf = BigFile::open(&mut disc).expect("BIGFILE.BIG");
    let w = ctr_assets::weapons::build(&mut bf).expect("weapons");
    assert_eq!(w.models.len(), 23);
    for (key, m) in &w.models {
        assert!(!m.frames.is_empty(), "{key}");
        for f in &m.frames {
            let tris: usize = f.surfaces.iter().map(|s| s.indices.len() / 3).sum();
            assert!(tris >= 16, "{key}: {tris} triangles");
            for s in &f.surfaces {
                assert!(s.indices.iter().all(|&i| (i as usize) < s.vertices.len()));
                assert!(s.texture.is_none_or(|t| t < w.textures.len()), "{key}");
            }
        }
    }
    // Animated models keep every stored frame; the fruit's identical spin frames collapse.
    for (key, n) in [("missile", 10), ("warpball", 16), ("aku", 7), ("fruit", 1)] {
        assert_eq!(w.models[key].frames.len(), n, "{key}");
    }
    // The crates are textured from the level VRM and stand on y = 0, ~2.1 m on a side.
    for key in ["crate_question", "crate_fruit", "tnt", "nitro"] {
        let f = &w.models[key].frames[0];
        assert!(f.surfaces.iter().all(|s| s.texture.is_some()), "{key}");
        let lo = w.models[key].bounds["min"][1].as_f64().unwrap();
        let hi = w.models[key].bounds["max"][1].as_f64().unwrap();
        assert!(
            lo.abs() < 0.05 && (hi - 2.12).abs() < 0.1,
            "{key}: {lo}..{hi}"
        );
    }
    for t in &w.textures {
        assert_eq!(t.rgba.len() as u32, t.width * t.height * 4);
        assert!(t.rgba.chunks(4).any(|p| p[3] > 0), "{} is empty", t.name);
    }
}

#[test]
fn extracts_effects() {
    let Some(mut disc) = disc() else {
        eprintln!("CTR_DISC not set; skipping");
        return;
    };
    let mut bf = BigFile::open(&mut disc).expect("BIGFILE.BIG");
    let fx = ctr_assets::effects::build(&mut bf).expect("effects");
    let frames = fx.turbo_flame["frames"].as_array().unwrap();
    assert_eq!(frames.len(), 8);
    for f in frames {
        let surfaces = f["surfaces"].as_array().unwrap();
        assert!(surfaces.iter().any(|s| s["blend"] == "additive"));
        let tris: usize = surfaces
            .iter()
            .map(|s| s["indices"].as_array().unwrap().len() / 3)
            .sum();
        assert!(tris >= 20, "{tris} flame triangles");
    }
    for t in &fx.textures {
        assert_eq!(t.rgba.len() as u32, t.width * t.height * 4);
        assert!(t.rgba.chunks(4).any(|p| p[3] > 0), "{} is empty", t.name);
    }
    assert!(fx.exhaust_smoke["texture"].as_u64().is_some());
    assert!(fx.skid_marks["texture"].as_u64().is_some());
    // The exhaust anchors sit at the kart's pipe tips (top rear of the kart).
    let k = kart::build(&mut bf, "crash").unwrap();
    for a in k.anchors.exhaust {
        assert!(
            a[1] > k.bounds.max[1] - 0.15 && a[2] < k.bounds.min[2] + 0.15,
            "{a:?}"
        );
    }
}

#[test]
fn extracts_hud_and_music() {
    let Some(mut disc) = disc() else {
        eprintln!("CTR_DISC not set; skipping");
        return;
    };
    {
        let mut bf = BigFile::open(&mut disc).expect("BIGFILE.BIG");
        let hud = ctr_assets::hud::build(&mut bf).expect("hud");
        let big = hud.fonts.iter().find(|f| f.name == "big").unwrap();
        for ch in ["0", "9", "a", "z", "/"] {
            assert!(big.glyphs.iter().any(|g| g.ch == ch), "big font lacks {ch}");
        }
        assert_eq!(
            hud.meshes
                .iter()
                .filter(|m| m.name.starts_with("big"))
                .count(),
            8
        );
        assert!(hud.meshes.iter().any(|m| m.name == "fruitdisp"));
    }
    // One race track: Crash Cove is 34560 ticks at 190 BPM x 120 TPQN = 90.95 s.
    let (lba, size) = disc.find_file(ctr_assets::audio::HWL_PATH).unwrap();
    let howl =
        ctr_assets::audio::Howl::parse(&disc.read_bytes(lba, size as usize).unwrap()).unwrap();
    let cseq = ctr_assets::music::Cseq::parse(&howl.seqs[3]).unwrap();
    let r = ctr_assets::music::render_song(&howl, &cseq, 0, 11025).unwrap();
    assert!((r.loop_seconds - 90.95).abs() < 0.1, "{}", r.loop_seconds);
    assert_eq!(r.pcm.len() % 2, 0);
    assert!(r.pcm.iter().any(|s| s.abs() > 20000));
}

#[test]
fn hud_renderer_draws_real_hud_fast() {
    let Some(mut disc) = disc() else {
        eprintln!("CTR_DISC not set; skipping");
        return;
    };
    let mut bf = BigFile::open(&mut disc).expect("BIGFILE.BIG");
    let hud = ctr_assets::hud::build(&mut bf).expect("hud");
    let json = serde_json::to_value(&hud).unwrap();
    let r = ctr_assets::hud_draw::HudRenderer::from_json(&json).expect("renderer");
    let state = ctr_assets::hud_draw::HudState {
        speedometer: true,
        speed: 9000.0,
        accel_speed: 10000.0,
        sacred_fire_speed: 2000.0,
        drift_meter: Some((490, 1000, 500)),
        jump: Some(1250),
        wumpa: Some(7),
        juiced: true,
        lap: Some((2, 3)),
        place: Some(1),
        item: Some((10, 3)),
        ranks: ["crash", "coco", "cortex", "tiny"]
            .map(String::from)
            .to_vec(),
        time_seconds: 0.4,
        ..Default::default()
    };
    let mut px = vec![0u8; 512 * 216 * 4];
    r.render(&state, &mut px);
    let n = 100;
    let t = std::time::Instant::now();
    for _ in 0..n {
        r.render(&state, &mut px);
    }
    let ms = t.elapsed().as_secs_f64() * 1000.0 / n as f64;
    println!("HudRenderer::render: {ms:.3} ms per frame");
    if !cfg!(debug_assertions) {
        assert!(ms < 2.0, "{ms} ms");
    }
    // Speedometer, portraits, lap text, wumpa count and place digit are drawn.
    let a = |x: usize, y: usize| px[(y * 512 + x) * 4 + 3];
    assert_eq!(a(420, 189), 255);
    assert_eq!(a(30, 65), 255);
    assert_eq!(a(150, 100), 0);
    assert_eq!(a(50, 191), 255);
}
