//! The Mirage Raceway's racers as karts: `Creature\GnomeRocketCar` and `Creature\GoblinRocketCar`,
//! read from the player's own install at startup (their art is not ours to ship) and handed to
//! kartcraft as any kart model (`kartcraft::model::provide`), sold for tokens as the built-in
//! goods are (`kartcraft::goods`). Each keeps its body and loses its own pilot, whose seat our
//! character takes, and its own rocket fire, for which our turbo flames burn at its nozzles.
//!
//! The M2's frame (+Z up, model units; a creature faces +X, but the goblin car's rockets pull it
//! along −Y) turns into the kart's (+X left, +Y up, +Z forward, meters) by axis, scaled to the
//! kart's size; the unlit kart shader gets a fixed light baked into the vertex colours, so the
//! shapes read.

use benilla_assets::{LockRecover, WorldAssets};
use bevy::prelude::*;
use kartcraft::model::{Anchors, Model, Surface, Texture, Vertex};

/// Which way a model drives, in its own frame.
#[derive(Clone, Copy)]
enum Facing {
    /// +X, as a creature faces.
    PlusX,
    /// −Y: the goblin car, its rockets out to its side in a creature's frame.
    MinusY,
}

/// One racer car: its good's key, its model, the parts left out by texture (lowercase substrings),
/// which way it drives, the kart meters a model unit becomes, and its anchors in the model's own
/// units and axes.
struct Car {
    key: &'static str,
    model: &'static str,
    drop: &'static [&'static str],
    facing: Facing,
    scale: f32,
    seat: [f32; 3],
    steering_wheel: [f32; 3],
    /// The rocket nozzles: turbo flames and exhaust smoke.
    nozzles: &'static [[f32; 3]],
}

/// Whether `racer` is one of the cars built from the install at startup, after the CVars load.
pub(super) fn built_from_install(racer: &str) -> bool {
    CARS.iter().any(|c| c.key.eq_ignore_ascii_case(racer))
}

const CARS: [Car; 2] = [
    Car {
        key: "gnomecar",
        model: "Creature\\GnomeRocketCar\\GnomeRocketCar.m2",
        // The pilot's body, its boots and face.
        drop: &["gnomepounderpilotskin", "gnomemaleskin"],
        facing: Facing::PlusX,
        // Its cart a CTR kart's size, the rocket towering over it.
        scale: 0.34,
        seat: [-0.05, 0.0, 1.1],
        steering_wheel: [0.35, 0.0, 1.45],
        // The one rocket's tail, between its rear fins.
        nozzles: &[[-2.3, -0.12, 5.0]],
    },
    Car {
        key: "goblincar",
        model: "Creature\\GoblinRocketCar\\GoblinRocketCar.m2",
        // The pilot, and the rockets' own fire.
        drop: &["creature\\goblin\\goblin.blp", "firering"],
        facing: Facing::MinusY,
        // 8.4 units nose to wheels: 1.85 m.
        scale: 0.22,
        seat: [0.0, 0.0, 0.55],
        steering_wheel: [0.0, -0.4, 0.85],
        // The two rockets' tails, either side ahead of the driver, the harness between them.
        nozzles: &[[2.0, -0.95, 0.85], [-1.83, -0.95, 0.85]],
    },
];

/// The light baked into an unlit kart: from above, a little ahead and left.
const LIGHT: Vec3 = Vec3::new(0.3, 0.85, 0.42);

/// A point of the model's frame in the kart's: forward to +Z, its left to +X, up to +Y.
fn to_kart(p: [f32; 3], facing: Facing, scale: f32) -> [f32; 3] {
    let [x, y, z] = p;
    let [left, up, forward] = match facing {
        Facing::PlusX => [y, z, x],
        Facing::MinusY => [x, z, -y],
    };
    [left * scale, up * scale, forward * scale]
}

/// Each car, built and handed over; one the install lacks is left out of the garage and shop.
pub(super) fn build_cars(assets: Option<Res<WorldAssets>>) {
    let Some(assets) = assets else { return };
    let mut chain = assets.chain.lock_recover();
    for car in &CARS {
        match build(&mut chain, car) {
            Ok(model) => kartcraft::model::provide(model),
            Err(e) => warn!("kart: {} from {}: {e:#}", car.key, car.model),
        }
    }
}

fn build(chain: &mut benilla_formats::Chain, car: &Car) -> anyhow::Result<Model> {
    let parts = benilla_formats::load_m2_mesh(chain, car.model)?;
    let light = LIGHT.normalize();
    // Textures by path and whether opaque: an opaque part's alpha is not coverage, so it is made
    // whole, where a cut-out part keeps its holes.
    let mut textures: Vec<Texture> = Vec::new();
    let mut index: std::collections::HashMap<(String, bool), usize> = Default::default();
    let mut surfaces = Vec::new();
    for p in &parts {
        let Some(path) = p.texture.as_deref() else {
            continue;
        };
        let lower = path.to_ascii_lowercase();
        if car.drop.iter().any(|d| lower.contains(d)) {
            continue;
        }
        let cut = match p.blend {
            benilla_formats::ModelBlend::Opaque => false,
            benilla_formats::ModelBlend::AlphaTest => true,
            // Additive and blended parts are glows and fire, ours to draw.
            _ => continue,
        };
        let key = (lower.clone(), cut);
        let slot = match index.get(&key) {
            Some(&i) => i,
            None => {
                let bytes = chain.read_file(&lower)?;
                let (width, height, mut rgba) = benilla_formats::blp_to_rgba(&bytes)?;
                if !cut {
                    for px in rgba.as_chunks_mut::<4>().0 {
                        px[3] = 255;
                    }
                }
                textures.push(Texture {
                    width,
                    height,
                    rgba,
                });
                index.insert(key, textures.len() - 1);
                textures.len() - 1
            }
        };
        let vertices = (0..p.positions.len())
            .map(|i| {
                let n = p
                    .normals
                    .get(i)
                    .map_or(Vec3::Y, |n| Vec3::from_array(to_kart(*n, car.facing, 1.0)));
                let shade = 0.55 + 0.45 * n.normalize_or_zero().dot(light).max(0.0);
                Vertex {
                    position: to_kart(p.positions[i], car.facing, car.scale),
                    normal: n.to_array(),
                    uv: p.uvs.get(i).copied().unwrap_or_default(),
                    color: [shade, shade, shade, 1.0],
                }
            })
            .collect();
        surfaces.push(Surface {
            texture: Some(slot),
            double_sided: p.two_sided,
            blend: String::new(),
            vertices,
            indices: p.indices.clone(),
        });
    }
    anyhow::ensure!(!surfaces.is_empty(), "no parts kept");
    let at = |p: [f32; 3]| to_kart(p, car.facing, car.scale);
    Ok(Model {
        kart: car.key.to_string(),
        anchors: Anchors {
            seat: at(car.seat),
            steering_wheel: Some(at(car.steering_wheel)),
            exhaust: car.nozzles.iter().map(|&n| at(n)).collect(),
            turbo: car.nozzles.iter().map(|&n| at(n)).collect(),
        },
        wheels: Vec::new(),
        wheel_sprites: None,
        surfaces,
        textures,
        smooth: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every car is a good of its own, and builds from the install with its pilot gone.
    #[test]
    fn every_racer_car_builds_from_the_install() {
        let data = benilla_formats::wow_data_or_skip!();
        let mut chain = benilla_formats::open_chain(&data).expect("the install's archives");
        for car in &CARS {
            assert!(
                kartcraft::goods::good(car.key).is_some(),
                "{} is a good",
                car.key
            );
            let model = build(&mut chain, car).unwrap_or_else(|e| panic!("{}: {e:#}", car.key));
            assert_eq!(model.kart, car.key);
            assert_eq!(model.anchors.turbo.len(), car.nozzles.len());
            for t in &model.textures {
                assert_eq!(
                    t.rgba.len(),
                    (t.width * t.height * 4) as usize,
                    "{}",
                    car.key
                );
            }
            // `WOW_KART_DUMP=<dir>`: the kart as built (its frame, meters) with its anchors, for
            // drawing it from above and the side to place them. Blizzard data: never commit it.
            if let Some(dir) = std::env::var_os("WOW_KART_DUMP") {
                let surfaces: Vec<_> = model
                    .surfaces
                    .iter()
                    .map(|s| {
                        serde_json::json!({
                            "texture": s.texture,
                            "positions": s.vertices.iter().map(|v| v.position).collect::<Vec<_>>(),
                            "uvs": s.vertices.iter().map(|v| v.uv).collect::<Vec<_>>(),
                            "indices": s.indices,
                        })
                    })
                    .collect();
                let textures: Vec<_> = model
                    .textures
                    .iter()
                    .map(|t| serde_json::json!({ "width": t.width, "height": t.height, "rgba": t.rgba }))
                    .collect();
                let a = &model.anchors;
                let dump = serde_json::json!({
                    "surfaces": surfaces,
                    "textures": textures,
                    "seat": a.seat,
                    "steering_wheel": a.steering_wheel,
                    "turbo": a.turbo,
                });
                let path = std::path::Path::new(&dir).join(format!("{}.json", car.key));
                std::fs::write(path, dump.to_string()).expect("the dump");
            }
        }
    }
}
