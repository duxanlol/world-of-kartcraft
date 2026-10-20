//! `prop-catalog <out-dir> [--wow <Data dir>] [--ctr <disc.bin>] [--list]`
//!
//! The track editor's placeable-object candidates: the WoW install's doodads (`World\*.m2`) and
//! world objects (`World\wmo\*.wmo` roots) whose file name says ramp, platform, bridge, obstacle
//! or prop ([`GROUPS`]), and every model the CTR disc's levels and `shared.mpk` carry. With
//! `--list` it prints them by group; else it writes `<out-dir>` as a local viewer: `index.html`,
//! `catalog.json`, one `models/<id>.json` each (Y-up yards) and their textures as PNG. The
//! install and the disc are only read; the output is the player's own art, so it stays local:
//! serve it with `python3 -m http.server -d <out-dir>` and open `http://localhost:8000`.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use benilla_formats::{Chain, ModelBlend, RenderSubmesh};
use serde::Serialize;
use serde_json::json;

/// The groups, in the viewer's order, and the words in a file's name that put it in one; the
/// first group with a match wins. CTR models no word names go to "Misc".
const GROUPS: [(&str, &[&str]); 5] = [
    (
        "Ramps",
        &[
            "ramp",
            "gangplank",
            "plank",
            "stair",
            "steps",
            "slope",
            "incline",
            "chute",
            "slide",
        ],
    ),
    (
        "Platforms",
        &[
            "platform", "dock", "pier", "scaffold", "deck", "raft", "stage", "pedestal", "dais",
            "walkway", "catwalk", "landing", "floor",
        ],
    ),
    ("Bridges", &["bridge"]),
    (
        "Obstacles",
        &[
            "fence",
            "barricade",
            "barrier",
            "wall",
            "pillar",
            "column",
            "post",
            "spike",
            "boulder",
            "rock",
            "tnt",
            "nitro",
            "bomb",
        ],
    ),
    (
        "Misc",
        &[
            "crate", "box", "barrel", "keg", "cask", "chest", "cart", "wagon", "hay", "bale",
            "log", "sack", "tent", "anvil", "table", "bench", "sign", "drum", "tire", "cone",
        ],
    ),
];

/// Words that make a name no prop, whatever else it says (a floor candle, a wall lamp).
const NOT_PROPS: [&str; 16] = [
    "candle", "brazier", "torch", "lamp", "light", "lantern", "web", "scroll", "vial", "tree",
    "shrub", "bush", "grass", "hologram", "banner", "fx",
];

/// A CTR model's group, or `None` for no prop.
/// CTR's own groups (`ctr_assets::props::group_of`: Hazards, Pickups), else the WoW groups
/// by its name, else Misc.
fn ctr_group_of(name: &str, level: Option<usize>) -> Option<&'static str> {
    match ctr_assets::props::group_of(name, level)? {
        "Scenery" => Some(group_of(name).unwrap_or("Misc")),
        g => Some(g),
    }
}

/// Past this many triangles a model is a building, not a prop to place.
const MAX_TRIS: usize = 40_000;

/// WoW yards per CTR meter.
const YARDS_PER_METER: f32 = 1.093_613_3;

/// A name's words, lowercase: split at `_`, `-`, spaces, digits and each capital after a small
/// letter (`RedRidgeDocksPlank01` is red, ridge, docks, plank).
fn words(name: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut prev_lower = false;
    for c in name.chars() {
        if !c.is_ascii_alphabetic() {
            out.push(String::new());
            prev_lower = false;
            continue;
        }
        if c.is_ascii_uppercase() && prev_lower {
            out.push(String::new());
        }
        prev_lower = c.is_ascii_lowercase();
        out.last_mut().unwrap().push(c.to_ascii_lowercase());
    }
    out.retain(|w| !w.is_empty());
    out
}

/// The group a name's words put it in: a word starting or ending with a group's word (`Docks`,
/// `boardwalk`-less `Docksboardwalk`, `oildrum`), so `Dustwallow` is no wall.
fn group_of(name: &str) -> Option<&'static str> {
    let words = words(name);
    let says = |w: &str| {
        let plural = format!("{w}s");
        (words.iter()).any(|t| t.starts_with(w) || t.ends_with(w) || t.ends_with(&plural))
    };
    if NOT_PROPS.iter().any(|w| says(w)) {
        return None;
    }
    GROUPS
        .iter()
        .find(|(_, ws)| ws.iter().any(|w| says(w)))
        .map(|(g, _)| *g)
}

/// A file's name without its folders and extension.
fn stem(path: &str) -> &str {
    let file = path.rsplit(['\\', '/']).next().unwrap_or(path);
    file.rsplit_once('.').map_or(file, |(s, _)| s)
}

/// Whether `path` (lowercase) is a world doodad or a WMO's root, not one of its `_000` groups.
fn is_world_model(path: &str) -> bool {
    if !path.starts_with("world\\") {
        return false;
    }
    if let Some(base) = path.strip_suffix(".wmo") {
        let group = base.len() > 4
            && base.as_bytes()[base.len() - 4] == b'_'
            && base[base.len() - 3..].bytes().all(|b| b.is_ascii_digit());
        return !group;
    }
    path.ends_with(".mdx") || path.ends_with(".m2")
}

/// A candidate before export.
struct Candidate {
    group: &'static str,
    path: String,
}

/// The install's candidates, by path.
fn wow_candidates(chain: &Chain) -> Result<Vec<Candidate>> {
    let mut out: Vec<Candidate> = chain
        .list()?
        .into_iter()
        .filter_map(|e| {
            let lower = e.name.to_ascii_lowercase();
            let group = group_of(stem(&e.name)).filter(|_| is_world_model(&lower))?;
            Some(Candidate {
                group,
                path: e.name,
            })
        })
        .collect();
    out.sort_by(|a, b| {
        a.path
            .to_ascii_lowercase()
            .cmp(&b.path.to_ascii_lowercase())
    });
    out.dedup_by(|a, b| a.path.eq_ignore_ascii_case(&b.path));
    Ok(out)
}

/// One drawn part of a model, Y-up yards.
#[derive(Serialize)]
struct Part {
    pos: Vec<f32>,
    uv: Vec<f32>,
    /// Vertex colours (RGBA), CTR's Gouraud shading; empty for WoW.
    col: Vec<f32>,
    idx: Vec<u32>,
    tex: Option<String>,
    /// `opaque`, `alpha` (cut out) or `blend`.
    blend: &'static str,
    two_sided: bool,
}

/// The viewer's row for one model.
#[derive(Serialize)]
struct Entry {
    id: String,
    source: &'static str,
    group: &'static str,
    name: String,
    path: String,
    tris: usize,
    /// Width, height, depth (yards).
    size: [f32; 3],
}

fn size_of(parts: &[Part]) -> [f32; 3] {
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    for p in parts.iter().flat_map(|p| p.pos.as_chunks::<3>().0) {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let round = |v: f32| (v * 100.0).round() / 100.0;
    [0, 1, 2].map(|k| round((hi[k] - lo[k]).max(0.0)))
}

/// A texture path as a file name in `tex/`.
fn tex_file(path: &str) -> String {
    let flat: String = path
        .to_ascii_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{}.png", flat.trim_end_matches(".blp"))
}

/// The WoW model at `path` as parts, its textures written to `tex/` (once each).
fn wow_parts(
    chain: &mut Chain,
    path: &str,
    out: &Path,
    written: &mut HashMap<String, bool>,
) -> Result<Vec<Part>> {
    let subs: Vec<RenderSubmesh> = benilla_formats::load_object_model(chain, path)?;
    let mut parts = Vec::new();
    for sub in subs {
        if sub.indices.is_empty() {
            continue;
        }
        let tex = sub.texture.as_deref().and_then(|t| {
            let file = tex_file(t);
            let ok = *written.entry(file.clone()).or_insert_with(|| {
                let png = out.join("tex").join(&file);
                chain
                    .read(t)
                    .ok()
                    .and_then(|bytes| benilla_formats::blp_to_png(&bytes, &png).ok())
                    .is_some()
            });
            ok.then(|| format!("tex/{file}"))
        });
        // Z up to Y up: (x, y, z) -> (x, z, -y).
        let pos = sub
            .positions
            .iter()
            .flat_map(|p| [p[0], p[2], -p[1]])
            .collect();
        parts.push(Part {
            pos,
            uv: sub.uvs.iter().flatten().copied().collect(),
            col: Vec::new(),
            idx: sub.indices,
            tex,
            blend: match sub.blend {
                ModelBlend::Opaque => "opaque",
                ModelBlend::AlphaTest => "alpha",
                _ => "blend",
            },
            two_sided: sub.two_sided,
        });
    }
    Ok(parts)
}

/// The disc's props as parts, keyed by name, their textures written to `tex/`.
#[allow(clippy::type_complexity)]
fn ctr_props(
    disc: &Path,
    out: &Path,
) -> Result<Vec<(&'static str, ctr_assets::props::Prop, Vec<Part>)>> {
    let mut disc = ctr_assets::disc::Disc::open(disc).map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut bf = ctr_assets::big::BigFile::open(&mut disc).map_err(|e| anyhow::anyhow!("{e}"))?;
    let props = ctr_assets::props::build(&mut bf).map_err(|e| anyhow::anyhow!("{e}"))?;
    for (i, t) in props.textures.iter().enumerate() {
        let png = out.join("tex").join(format!("ctr-{i}.png"));
        image::save_buffer(&png, &t.rgba, t.width, t.height, image::ColorType::Rgba8)
            .with_context(|| png.display().to_string())?;
    }
    Ok(props
        .props
        .into_iter()
        .filter_map(|p| Some((ctr_group_of(&p.name, p.level)?, p)))
        .map(|(group, p)| {
            let parts = p
                .frame
                .surfaces
                .iter()
                .filter(|s| !s.indices.is_empty())
                .map(|s| Part {
                    pos: (s.vertices.iter())
                        .flat_map(|v| v.position.map(|c| c * YARDS_PER_METER))
                        .collect(),
                    uv: s.vertices.iter().flat_map(|v| v.uv).collect(),
                    col: s.vertices.iter().flat_map(|v| v.color).collect(),
                    idx: s.indices.clone(),
                    tex: s.texture.map(|i| format!("tex/ctr-{i}.png")),
                    blend: if s.blend == "opaque" {
                        "alpha"
                    } else {
                        "blend"
                    },
                    two_sided: true,
                })
                .collect();
            (group, p, parts)
        })
        .collect())
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    std::fs::write(path, serde_json::to_vec(value)?).with_context(|| path.display().to_string())
}

struct Args {
    out: PathBuf,
    wow: Option<PathBuf>,
    ctr: Option<PathBuf>,
    list: bool,
}

fn args() -> Result<Args> {
    let mut it = std::env::args().skip(1);
    let mut a = Args {
        out: PathBuf::new(),
        wow: None,
        ctr: None,
        list: false,
    };
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--wow" => a.wow = it.next().map(PathBuf::from),
            "--ctr" => a.ctr = it.next().map(PathBuf::from),
            "--list" => a.list = true,
            _ if a.out.as_os_str().is_empty() => a.out = PathBuf::from(arg),
            other => bail!("unexpected argument {other}"),
        }
    }
    if a.out.as_os_str().is_empty() && !a.list {
        bail!("usage: prop-catalog <out-dir> [--wow <Data dir>] [--ctr <disc.bin>] [--list]");
    }
    a.wow = a.wow.or_else(benilla_formats::wow_data);
    Ok(a)
}

fn main() -> Result<()> {
    let a = args()?;
    let mut chain = match &a.wow {
        Some(data) => Some(Chain::open(data).context("the WoW install's archives")?),
        None => None,
    };
    let candidates = match &chain {
        Some(c) => wow_candidates(c)?,
        None => Vec::new(),
    };
    if a.list {
        let mut by: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for c in &candidates {
            by.entry(c.group).or_default().push(&c.path);
        }
        for (group, paths) in &by {
            println!("{group}: {}", paths.len());
            for p in paths {
                println!("  {p}");
            }
        }
        return Ok(());
    }
    std::fs::create_dir_all(a.out.join("models"))?;
    std::fs::create_dir_all(a.out.join("tex"))?;
    let mut entries = Vec::new();
    let mut written = HashMap::new();
    if let Some(chain) = chain.as_mut() {
        for (n, c) in candidates.iter().enumerate() {
            let parts = match wow_parts(chain, &c.path, &a.out, &mut written) {
                Ok(p) if !p.is_empty() => p,
                Ok(_) => continue,
                Err(e) => {
                    eprintln!("skip {}: {e:#}", c.path);
                    continue;
                }
            };
            let tris: usize = parts.iter().map(|p| p.idx.len() / 3).sum();
            if tris > MAX_TRIS {
                eprintln!("skip {}: {tris} triangles", c.path);
                continue;
            }
            let id = format!("wow-{n}");
            write_json(&a.out.join("models").join(format!("{id}.json")), &parts)?;
            entries.push(Entry {
                tris,
                size: size_of(&parts),
                id,
                source: "wow",
                group: c.group,
                name: stem(&c.path).to_string(),
                path: c.path.clone(),
            });
        }
    }
    if let Some(disc) = &a.ctr {
        for (n, (group, prop, parts)) in ctr_props(disc, &a.out)?.into_iter().enumerate() {
            let id = format!("ctr-{n}");
            write_json(&a.out.join("models").join(format!("{id}.json")), &parts)?;
            entries.push(Entry {
                tris: parts.iter().map(|p| p.idx.len() / 3).sum(),
                size: size_of(&parts),
                id,
                source: "ctr",
                group,
                path: match prop.level {
                    Some(l) => format!(
                        "bigfile {l} level, model {} (id {:#x})",
                        prop.name, prop.model_id
                    ),
                    None => format!("shared.mpk, model {} (id {:#x})", prop.name, prop.model_id),
                },
                name: prop.name,
            });
        }
    }
    let groups: Vec<&str> = (GROUPS.iter().map(|(g, _)| *g))
        .chain(["Hazards", "Pickups", "Misc"])
        .filter(|g| entries.iter().any(|e| e.group == *g))
        .collect();
    write_json(
        &a.out.join("catalog.json"),
        &json!({ "groups": groups, "entries": entries }),
    )?;
    std::fs::write(a.out.join("index.html"), include_str!("../viewer.html"))?;
    let count = |s: &str| entries.iter().filter(|e| e.source == s).count();
    println!(
        "wrote {} ({} WoW, {} CTR models); serve it: python3 -m http.server -d {}",
        a.out.display(),
        count("wow"),
        count("ctr"),
        a.out.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_sort_into_their_groups_first_match_first() {
        assert_eq!(group_of("Wooden_Ramp01"), Some("Ramps"));
        assert_eq!(group_of("DuskwoodDock"), Some("Platforms"));
        assert_eq!(group_of("Bridge_Stone"), Some("Bridges"));
        assert_eq!(group_of("crate_question"), Some("Misc"));
        assert_eq!(
            group_of("PlankRamp"),
            Some("Ramps"),
            "ramps before anything"
        );
        assert_eq!(group_of("Treee01"), None);
        assert_eq!(group_of("DustwallowShrub04"), None, "no wall, and a shrub");
        assert_eq!(group_of("DustwallowCrate01"), Some("Misc"));
        assert_eq!(group_of("FloorCandlesCorner01"), None);
        assert_eq!(group_of("oildrum01"), Some("Misc"));
        assert_eq!(group_of("RedRidgeDocksboardwalk02"), Some("Platforms"));
        assert_eq!(group_of("DeadMineCannonBallStack"), None);
        assert_eq!(
            words("DarkIronWoodPlanks01"),
            ["dark", "iron", "wood", "planks"]
        );
        assert_eq!(group_of("DarkIronWoodPlanks01"), Some("Ramps"));
        assert_eq!(group_of("WallPieceStairs01"), Some("Ramps"));
        assert_eq!(
            group_of("darkironwoodplanks01"),
            Some("Ramps"),
            "one lowercase word"
        );
    }

    #[test]
    fn ctr_models_are_hazards_pickups_or_scenery_never_racers_or_effects() {
        assert_eq!(ctr_group_of("armadillo", Some(41)), Some("Hazards"));
        assert_eq!(
            ctr_group_of("crate_nitro", None),
            Some("Hazards"),
            "it explodes"
        );
        assert_eq!(ctr_group_of("crate_question", Some(113)), Some("Pickups"));
        assert_eq!(ctr_group_of("tombstone", Some(145)), Some("Misc"));
        assert_eq!(ctr_group_of("drum", Some(89)), Some("Misc"));
        assert_eq!(ctr_group_of("crash", None), None, "a racer");
        assert_eq!(ctr_group_of("shockwavered", None), None, "an effect");
        assert_eq!(ctr_group_of("introcrash", Some(216)), None, "the intro's");
    }

    #[test]
    fn world_models_are_doodads_and_wmo_roots() {
        assert!(is_world_model("world\\generic\\crate01.mdx"));
        assert!(is_world_model("world\\wmo\\azeroth\\bridge.wmo"));
        assert!(
            !is_world_model("world\\wmo\\azeroth\\bridge_000.wmo"),
            "a group"
        );
        assert!(
            !is_world_model("creature\\crab\\crab.mdx"),
            "not the world's"
        );
        assert!(!is_world_model("world\\generic\\crate01.blp"));
    }
}
