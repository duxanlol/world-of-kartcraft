//! A creature model's parts as the M2 loader yields them: each submesh's geoset id, texture,
//! vertex count and bounds (model units, X forward, Y left, Z up), the displays that use the model
//! and their skins. For picking a part (a vehicle's driver) and an anchor (its exhausts) by number.
//! Output is Blizzard data: never commit it.
//! `cargo run -p benilla-formats --example m2_parts -- 'Creature\GnomeRocketCar\GnomeRocketCar.m2'`

fn main() -> anyhow::Result<()> {
    let path = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: m2_parts <model path>"))?;
    let data = benilla_formats::wow_data().expect("no WoW install found (set $WOW_DATA)");
    let mut chain = benilla_formats::open_chain(&data)?;
    let catalog = benilla_formats::load_creature_catalog(&mut chain)?;
    let stem = |p: &str| {
        p.to_ascii_lowercase()
            .trim_end_matches(".m2")
            .trim_end_matches(".mdx")
            .to_string()
    };
    let mut skins = Vec::new();
    for (display, _) in catalog.display_models() {
        if let Some(m) = catalog.model(display) {
            if stem(&m.model_path) == stem(&path) {
                println!(
                    "display {display}: scale {} skins {:?}",
                    m.scale, m.textures
                );
                if skins.is_empty() {
                    skins = m.textures.to_vec();
                }
            }
        }
    }
    let parts = benilla_formats::load_m2_mesh_skinned(&mut chain, &path, &skins)?;
    // `m2_parts <path> <part>`: that part's Y and Z extent in half-unit slices along X, its shape
    // from the side and above, for an anchor at a nozzle.
    if let Some(i) = std::env::args()
        .nth(2)
        .and_then(|v| v.parse::<usize>().ok())
    {
        let p = &parts[i];
        let mut slices: std::collections::BTreeMap<i32, ([f32; 2], [f32; 2])> = Default::default();
        for v in &p.positions {
            let s = slices
                .entry((v[0] * 2.0).floor() as i32)
                .or_insert(([f32::MAX, f32::MIN], [f32::MAX, f32::MIN]));
            s.0 = [s.0[0].min(v[1]), s.0[1].max(v[1])];
            s.1 = [s.1[0].min(v[2]), s.1[1].max(v[2])];
        }
        for (k, (y, z)) in slices {
            println!(
                "x {:5.1}..{:5.1}: y {:5.2}..{:5.2}  z {:5.2}..{:5.2}",
                k as f32 / 2.0,
                (k + 1) as f32 / 2.0,
                y[0],
                y[1],
                z[0],
                z[1]
            );
        }
        return Ok(());
    }
    for (i, p) in parts.iter().enumerate() {
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for v in &p.positions {
            for k in 0..3 {
                lo[k] = lo[k].min(v[k]);
                hi[k] = hi[k].max(v[k]);
            }
        }
        println!(
            "part {i}: geoset {} texture {:?} slot {:?} {} verts {} tris blend {:?}\n  min {lo:.2?}\n  max {hi:.2?}",
            p.geoset_id,
            p.texture,
            p.skin_slot,
            p.positions.len(),
            p.indices.len() / 3,
            p.blend,
        );
    }
    Ok(())
}
