//! `course-gen`: kart course tools over the player's own install (`$WOW_DATA`).
//!
//! `course-gen roads --zone "Elwynn Forest" --out <dir>` writes `roads.json`, `obstacles.json`
//! and `roads.png`, and prints the ground textures with their coverage.
//!
//! `course-gen courses --zone "Elwynn Forest" --out <dir>` generates kart courses over the zone's
//! drivability field, test-drives the best and writes `<dir>/<zone>/candidates.json` with a map
//! and an elevation-profile PNG per finalist.
//!
//! `course-gen export --candidates <dir>/candidates.json --all` adds those finalists to
//! `courses/courses.json`, each replacing the course of its key.
//!
//! `course-gen marshals` writes each course's Race Marshal spot (`"marshal"`) into
//! `courses/courses.json`; run it after adding courses, then `setup/vmangos/pack.py`.
//!
//! `course-gen pads` lays turbo pads on every course's straights (`"pads"`); the client alone
//! reads them.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use course_gen::render::{render, Landmark};
use course_gen::select::{run, SelectParams};
use course_gen::{extract_roads, load_zone, open_install, preview, RoadParams, TerrainParams};

#[derive(Parser)]
#[command(about = "Kart course generation from WoW 1.12.1 terrain")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Extract a zone's road network.
    Roads {
        /// The zone's AreaTable name ("Elwynn Forest"); its subzones come with it.
        #[arg(long)]
        zone: String,
        /// Output directory (created).
        #[arg(long)]
        out: PathBuf,
        /// Raster resolution in yards per pixel.
        #[arg(long, default_value_t = course_gen::terrain::DEFAULT_YARDS_PER_PX)]
        res: f32,
        /// Road texture keywords, replacing the built-in list (repeatable).
        #[arg(long = "road-keyword")]
        road_keywords: Vec<String>,
        /// Road weight at or above which a pixel is road.
        #[arg(long)]
        threshold: Option<f32>,
        /// Dead ends shorter than this many yards are pruned.
        #[arg(long)]
        spur: Option<f32>,
        /// Also write `probe-<n>.png`, the visible weight of textures whose path contains this
        /// (repeatable, any case).
        #[arg(long = "probe")]
        probes: Vec<String>,
        /// Also write `crop.png`: the render around world `x,y` within `r` yards, scaled ×2.
        #[arg(long, allow_hyphen_values = true)]
        crop: Option<String>,
    },
    /// Generate, test-drive and rank kart courses for a zone.
    Courses {
        #[arg(long)]
        zone: String,
        /// Output root; the zone's files go in `<out>/<zone-slug>/`.
        #[arg(long)]
        out: PathBuf,
        /// Generation attempts.
        #[arg(long, default_value_t = 1500)]
        attempts: usize,
        /// Candidates test-driven.
        #[arg(long, default_value_t = 40)]
        drive: usize,
        /// Finalists kept.
        #[arg(long, default_value_t = 8)]
        keep: usize,
        #[arg(long, default_value_t = 1)]
        seed: u64,
        #[arg(long, default_value_t = 8)]
        threads: usize,
        /// For an instance whose ground is its streets (Stratholme): the circuit's waypoints,
        /// WoW `x,y` in driving order, separated by `;`.
        #[arg(long, allow_hyphen_values = true)]
        via: Option<String>,
    },
    /// Run `courses` over many zones, a few at a time, exporting each zone's finalists as it
    /// finishes and logging a line a zone to `<out>/batch-log.txt`.
    Batch {
        /// Zone names, comma-separated, or `alliance` (Alliance and contested zones), `horde`
        /// or `all`.
        #[arg(long, value_delimiter = ',')]
        zones: Vec<String>,
        #[arg(long)]
        out: PathBuf,
        /// Zones run at once (the largest zones take about 5 GB each).
        #[arg(long, default_value_t = 2)]
        jobs: usize,
        /// Threads per zone.
        #[arg(long, default_value_t = 12)]
        threads: usize,
        #[arg(long, default_value_t = 1500)]
        attempts: usize,
        #[arg(long, default_value_t = 40)]
        drive: usize,
        #[arg(long, default_value_t = 8)]
        keep: usize,
        #[arg(long, default_value_t = 1)]
        seed: u64,
        /// The course file each zone's finalists are added to; a key already there is never
        /// replaced (the new course takes `-2`).
        #[arg(long, default_value = "courses/courses.json")]
        courses: PathBuf,
        /// Finalists scoring under this are not exported.
        #[arg(long, default_value_t = 20.0)]
        min_score: f32,
        /// Do not export.
        #[arg(long)]
        no_export: bool,
        /// Also run zones that already have courses in the course file.
        #[arg(long)]
        rerun: bool,
    },
    /// Add a run's finalists to `courses/courses.json`, replacing any course of the same key.
    Export {
        /// A `candidates.json` written by `courses`.
        #[arg(long)]
        candidates: PathBuf,
        /// Finalist ranks to export, comma-separated.
        #[arg(long, value_delimiter = ',', conflicts_with = "all")]
        ranks: Vec<usize>,
        /// Export every finalist.
        #[arg(long)]
        all: bool,
        /// The course file.
        #[arg(long, default_value = "courses/courses.json")]
        courses: PathBuf,
        /// Only finalists scoring at least this.
        #[arg(long, default_value_t = f32::NEG_INFINITY, allow_hyphen_values = true)]
        min_score: f32,
        /// Keys to remove from the file (repeatable).
        #[arg(long)]
        remove: Vec<String>,
    },
    /// Place a Race Marshal beside every distinct course start, at the ground's height, and
    /// write each course's `"marshal"` into the course file.
    Marshals {
        /// The course file.
        #[arg(long, default_value = "courses/courses.json")]
        courses: PathBuf,
    },
    /// Lay turbo pads on every course's straights, at the ground's height, and write each
    /// course's `"pads"` into the course file (`course_gen::pads`).
    Pads {
        /// The course file.
        #[arg(long, default_value = "courses/courses.json")]
        courses: PathBuf,
    },
}

/// Alliance zones and the contested ones, Eastern Kingdoms then Kalimdor.
const ALLIANCE: &[&str] = &[
    "Elwynn Forest",
    "Westfall",
    "Dun Morogh",
    "Loch Modan",
    "Redridge Mountains",
    "Duskwood",
    "Wetlands",
    "Arathi Highlands",
    "Hillsbrad Foothills",
    "Alterac Mountains",
    "Stranglethorn Vale",
    "Swamp of Sorrows",
    "Blasted Lands",
    "Deadwind Pass",
    "Badlands",
    "Searing Gorge",
    "Burning Steppes",
    "Western Plaguelands",
    "Eastern Plaguelands",
    "The Hinterlands",
    "Teldrassil",
    "Darkshore",
    "Ashenvale",
    "Stonetalon Mountains",
    "Desolace",
    "Feralas",
    "Thousand Needles",
    "Tanaris",
    "Un'Goro Crater",
    "Silithus",
    "Winterspring",
    "Felwood",
    "Azshara",
    "Dustwallow Marsh",
    "Moonglade",
];
/// Horde zones.
const HORDE: &[&str] = &[
    "Tirisfal Glades",
    "Silverpine Forest",
    "Durotar",
    "Mulgore",
    "The Barrens",
];

struct BatchOpts {
    out: PathBuf,
    jobs: usize,
    threads: usize,
    attempts: usize,
    drive: usize,
    keep: usize,
    seed: u64,
    courses: Option<PathBuf>,
    min_score: f32,
    rerun: bool,
}

fn batch(zones: &[String], o: &BatchOpts) {
    let mut list: Vec<String> = Vec::new();
    for z in zones {
        match z.to_ascii_lowercase().as_str() {
            "alliance" => list.extend(ALLIANCE.iter().map(|s| s.to_string())),
            "horde" => list.extend(HORDE.iter().map(|s| s.to_string())),
            "all" => list.extend(ALLIANCE.iter().chain(HORDE).map(|s| s.to_string())),
            _ => list.push(z.clone()),
        }
    }
    let mut seen = std::collections::HashSet::new();
    list.retain(|z| seen.insert(z.to_ascii_lowercase()));
    let log_path = o.out.join("batch-log.txt");
    let has_courses = |zone: &str| -> bool {
        let Some(path) = &o.courses else {
            return false;
        };
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|v| v["courses"].as_array().cloned())
            .is_some_and(|cs| {
                cs.iter().any(|c| {
                    c["zone"]
                        .as_str()
                        .is_some_and(|z| z.eq_ignore_ascii_case(zone))
                })
            })
    };
    let lock = std::sync::Mutex::new(());
    let next = std::sync::atomic::AtomicUsize::new(0);
    let log_lock = std::sync::Mutex::new(());
    let append = |line: &str| {
        use std::io::Write as _;
        let _guard = log_lock.lock();
        println!("{line}");
        let _ = std::fs::create_dir_all(&o.out);
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            let _ = writeln!(f, "{line}");
        }
    };
    std::thread::scope(|s| {
        for _ in 0..o.jobs.max(1) {
            s.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let Some(zone) = list.get(i) else {
                    break;
                };
                if !o.rerun && has_courses(zone) {
                    append(&format!("{zone}: skipped, already has courses"));
                    continue;
                }
                let started = std::time::Instant::now();
                let run = courses(
                    zone,
                    &o.out,
                    o.attempts,
                    o.drive,
                    o.keep,
                    o.seed,
                    o.threads,
                    &[],
                );
                let secs = started.elapsed().as_secs_f32();
                let line = match run {
                    Err(e) => format!("{zone}: failed: {e:#}"),
                    Ok(run) => {
                        let mut line = format!(
                            "{}: generated {} / driven {} / kept {}",
                            run.zone, run.generated, run.driven, run.kept
                        );
                        match &run.top {
                            Some((name, lap)) => {
                                line.push_str(&format!("; top {name:?} {lap:.1}s"))
                            }
                            None => line.push_str("; no course kept"),
                        }
                        if let (Some(path), true) = (&o.courses, run.kept > 0) {
                            let _guard = lock.lock();
                            match course_gen::export::export(
                                &run.candidates,
                                None,
                                o.min_score,
                                path,
                                &[],
                                false,
                            ) {
                                Ok(w) => line.push_str(&format!("; exported {}", w.len())),
                                Err(e) => line.push_str(&format!("; export failed: {e:#}")),
                            }
                        }
                        line
                    }
                };
                append(&format!("{line} ({secs:.0}s)"));
            });
        }
    });
}

/// Public landmark positions for the orientation check: (name, WoW x, WoW y).
const LANDMARKS: &[(&str, f32, f32)] = &[
    ("Goldshire", -9465.0, 64.0),
    ("Stormwind gate", -9000.0, 500.0),
    ("Northshire Abbey", -8913.0, -137.0),
    ("Sentinel Hill", -10628.0, 1037.0),
    ("Moonbrook", -11018.0, 1513.0),
];

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Batch {
            zones,
            out,
            jobs,
            threads,
            attempts,
            drive,
            keep,
            seed,
            courses,
            min_score,
            no_export,
            rerun,
        } => {
            let opts = BatchOpts {
                out,
                jobs,
                threads,
                attempts,
                drive,
                keep,
                seed,
                courses: (!no_export).then_some(courses),
                min_score,
                rerun,
            };
            batch(&zones, &opts);
        }
        Cmd::Export {
            candidates,
            ranks,
            min_score,
            all,
            courses,
            remove,
        } => {
            anyhow::ensure!(all || !ranks.is_empty(), "give --ranks or --all");
            let ranks = (!all).then_some(ranks.as_slice());
            let done =
                course_gen::export::export(&candidates, ranks, min_score, &courses, &remove, true)?;
            for e in &done {
                println!(
                    "{}  {}  ({} checkpoints)",
                    e["key"].as_str().unwrap_or(""),
                    e["name"].as_str().unwrap_or(""),
                    e["checkpoints"].as_array().map_or(0, Vec::len)
                );
            }
            println!("exported {} into {}", done.len(), courses.display());
        }
        Cmd::Marshals { courses } => {
            let n = course_gen::marshal::run(&courses, open_install()?)?;
            println!("{n} marshals written into {}", courses.display());
        }
        Cmd::Pads { courses } => {
            let n = course_gen::pads::run(&courses, open_install()?)?;
            println!("{n} turbo pads written into {}", courses.display());
        }
        Cmd::Courses {
            zone,
            out,
            attempts,
            drive,
            keep,
            seed,
            threads,
            via,
        } => {
            let via = parse_via(via.as_deref().unwrap_or(""))?;
            let run = courses(&zone, &out, attempts, drive, keep, seed, threads, &via)?;
            print!("{}", run.log);
        }
        Cmd::Roads {
            zone,
            out,
            res,
            road_keywords,
            threshold,
            spur,
            probes,
            crop,
        } => {
            let mut tp = TerrainParams {
                yards_per_px: res,
                probe_textures: probes.iter().map(|p| p.to_ascii_lowercase()).collect(),
                ..Default::default()
            };
            if !road_keywords.is_empty() {
                tp.road_keywords = road_keywords
                    .iter()
                    .map(|k| k.to_ascii_lowercase())
                    .collect();
            }
            let mut rp = RoadParams::default();
            if let Some(t) = threshold {
                rp.threshold = t;
            }
            if let Some(s) = spur {
                rp.spur_len = s;
            }
            std::fs::create_dir_all(&out).with_context(|| format!("creating {}", out.display()))?;

            let started = std::time::Instant::now();
            let mut chain = open_install()?;
            let terrain = load_zone(&mut chain, &zone, &tp)?;
            let g = terrain.grid;
            println!(
                "{} (area {}, map {} {}): {}x{} px at {} yd/px, x {:.0}..{:.0}, y {:.0}..{:.0}, loaded in {:.1}s",
                terrain.zone,
                terrain.area_id,
                terrain.map_id,
                terrain.map,
                g.width,
                g.height,
                g.yards_per_px,
                g.north_x - g.height as f32 * g.yards_per_px,
                g.north_x,
                g.west_y - g.width as f32 * g.yards_per_px,
                g.west_y,
                started.elapsed().as_secs_f32()
            );
            let zone_px = terrain.in_zone.iter().filter(|&&b| b).count() as f64;
            println!("ground textures on the zone's chunks (coverage of {zone_px:.0} px):");
            for t in &terrain.textures {
                println!(
                    "  {} {:6.2}%  {:9.0} px  {}",
                    if t.is_road { "ROAD" } else { "    " },
                    100.0 * t.pixels / zone_px,
                    t.pixels,
                    t.path
                );
            }

            let (graph, rasters) = extract_roads(&terrain, &rp);
            let junctions = graph.nodes.iter().filter(|n| n.degree >= 3).count();
            let ends = graph.nodes.iter().filter(|n| n.degree == 1).count();
            let mean_width = {
                let (s, n) = graph
                    .edges
                    .iter()
                    .flat_map(|e| &e.points)
                    .fold((0.0, 0usize), |(s, n), p| (s + p.width, n + 1));
                s / n.max(1) as f32
            };
            println!(
                "roads: {:.2} km, {} nodes ({} junctions, {} ends), {} edges, mean width {:.1} yd, mask {} px",
                graph.total_length() / 1093.61,
                graph.nodes.len(),
                junctions,
                ends,
                graph.edges.len(),
                mean_width,
                rasters.mask.count()
            );
            let solid = terrain.obstacles.iter().filter(|o| o.solid).count();
            println!("obstacles: {} ({} solid)", terrain.obstacles.len(), solid);

            let mut landmarks = Vec::new();
            for &(name, x, y) in LANDMARKS {
                let Some(h) = terrain.height_at(x, y) else {
                    continue;
                };
                let (c, r) = g.pixel_of(x, y);
                // The nearest graph point, to check the road passes the landmark.
                let near = graph
                    .edges
                    .iter()
                    .flat_map(|e| &e.points)
                    .map(|p| ((p.x - x).powi(2) + (p.y - y).powi(2)).sqrt())
                    .fold(f32::INFINITY, f32::min);
                println!(
                    "landmark {name}: px ({c:.0}, {r:.0}), height {h:.1}, road weight {:.2}, in zone {:?}, nearest road {near:.0} yd",
                    terrain.road_at(x, y).unwrap_or(0.0),
                    terrain.in_zone_at(x, y).unwrap_or(false)
                );
                landmarks.push(Landmark { name, x, y });
            }

            graph.save(&out.join("roads.json"))?;
            std::fs::write(
                out.join("obstacles.json"),
                serde_json::to_string(&terrain.obstacles)?,
            )?;
            let png = out.join("roads.png");
            let full = render(&terrain, &rasters, &graph, &landmarks);
            full.save(&png)
                .with_context(|| format!("writing {}", png.display()))?;
            if let Some(spec) = crop {
                let v: Vec<f32> = spec
                    .split(',')
                    .map(|s| s.trim().parse::<f32>())
                    .collect::<Result<_, _>>()
                    .context("--crop takes x,y,r")?;
                anyhow::ensure!(v.len() == 3, "--crop takes x,y,r");
                let (c, r) = g.pixel_of(v[0], v[1]);
                let half = v[2] / g.yards_per_px;
                let c0 = (c - half).max(0.0) as u32;
                let r0 = (r - half).max(0.0) as u32;
                let side = (2.0 * half) as u32;
                let cw = side.min(full.width().saturating_sub(c0));
                let ch = side.min(full.height().saturating_sub(r0));
                let sub = image::imageops::crop_imm(&full, c0, r0, cw, ch).to_image();
                let big = image::imageops::resize(
                    &sub,
                    cw * 2,
                    ch * 2,
                    image::imageops::FilterType::Nearest,
                );
                big.save(out.join("crop.png"))?;
            }
            for (k, probe) in terrain.probes.iter().enumerate() {
                let img = image::GrayImage::from_fn(g.width as u32, g.height as u32, |c, r| {
                    let i = r as usize * g.width + c as usize;
                    let base = if terrain.in_zone[i] { 40.0 } else { 0.0 };
                    image::Luma([(base + 215.0 * probe[i]).min(255.0) as u8])
                });
                let path = out.join(format!("probe-{k}.png"));
                img.save(&path)
                    .with_context(|| format!("writing {}", path.display()))?;
            }
            println!("wrote {}", out.display());
        }
    }
    Ok(())
}

macro_rules! say {
    ($log:expr, $($t:tt)*) => {{
        $log.push_str(&format!($($t)*));
        $log.push('\n');
    }};
}

/// What a zone's run made.
struct ZoneRun {
    zone: String,
    candidates: PathBuf,
    generated: usize,
    driven: usize,
    kept: usize,
    /// The best finalist's name and lap (or sprint) time.
    top: Option<(String, f32)>,
    log: String,
}

/// `x,y;x,y;...` as world points; empty for none.
fn parse_via(text: &str) -> Result<Vec<glam::Vec2>> {
    text.split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|pair| {
            let v: Vec<f32> = pair
                .split(',')
                .map(|t| t.trim().parse::<f32>())
                .collect::<Result<_, _>>()
                .with_context(|| format!("--via point {pair:?}"))?;
            anyhow::ensure!(v.len() == 2, "--via point {pair:?} is not x,y");
            Ok(glam::Vec2::new(v[0], v[1]))
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn courses(
    zone: &str,
    out: &std::path::Path,
    attempts: usize,
    drive: usize,
    keep: usize,
    seed: u64,
    threads: usize,
    via: &[glam::Vec2],
) -> Result<ZoneRun> {
    let mut log = String::new();
    let started = std::time::Instant::now();
    let mut chain = open_install()?;
    let terrain = load_zone(&mut chain, zone, &TerrainParams::default())?;
    let (graph, _) = extract_roads(&terrain, &RoadParams::default());
    let slug: String = terrain
        .zone
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let dir = out.join(&slug);
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let mut p = SelectParams {
        drive,
        keep,
        via: via.to_vec(),
        ..Default::default()
    };
    p.gen.attempts = attempts;
    p.gen.seed = seed;
    p.gen.threads = threads;
    let result = run(&terrain, &graph, &p);
    let s = &result.stats;
    say!(log,
        "{}: {} hubs, {} attempts -> {} valid, {} distinct, {} driven, {} drove clean, {} kept ({:.1}s)",
        terrain.zone,
        s.hubs,
        s.attempts,
        s.generated,
        s.distinct,
        s.driven,
        s.drive_ok,
        s.kept,
        started.elapsed().as_secs_f32()
    );
    say!(log, "  generation failures: {:?}", s.failures);
    {
        let f = &result.field;
        let inside = f
            .block
            .iter()
            .filter(|b| **b != course_gen::cost::Block::Outside)
            .count()
            .max(1) as f32;
        let share = |k: &dyn Fn(usize) -> bool| {
            (0..f.block.len()).filter(|&i| k(i)).count() as f32 / inside * 100.0
        };
        say!(
            log,
            "  ground: clear {:.0}%, water {:.0}%, steep {:.0}%, obstacle {:.0}%, clearance >= 8 yd {:.1}%, largest open piece {:.2} km² (open up to {:.0}°)",
            share(&|i| f.block[i] == course_gen::cost::Block::Clear),
            share(&|i| f.block[i] == course_gen::cost::Block::Water),
            share(&|i| f.block[i] == course_gen::cost::Block::Steep),
            share(&|i| f.block[i] == course_gen::cost::Block::Obstacle),
            share(&|i| f.clearance[i] >= 8.0),
            s.largest_open_km2,
            s.max_slope
        );
    }
    say!(
        log,
        "  wall contacts over all driven: {} ({} in hairpins)",
        s.wall_contacts,
        s.hairpin_contacts
    );
    preview::field(&result.field, &result.hubs).save(dir.join("field.png"))?;
    if std::env::var_os("COURSE_DEBUG").is_some() {
        for (k, c) in result.rejected.iter().enumerate() {
            preview::map(&terrain, &result.field, c).save(dir.join(format!("rejected-{k}.png")))?;
            if let Some(d) = &c.drive {
                say!(log,
                    "  rejected-{k} {}: {:?} gates? laps {:?} walls {} slams {} stuck {} off {:.1}s stalls {:?}",
                    c.name, d.reject, d.lap_times, d.wall_contacts, d.slams, d.stuck_recoveries, d.off_track_time, d.stalls
                );
            }
        }
    }
    for r in &s.rejects {
        say!(log, "  rejected {r}");
    }
    for c in &result.finalists {
        let m = &c.metrics;
        let d = c.drive.as_ref();
        say!(log,
            "#{} {:.1} {} [{}]: {:.2} km, lap {:.1}s (est {:.0}), road {:.0}%, bends {:?} slide-worthy {} hairpins {}, elev {:.0}..{:.0} climb {:.0}, jumps {} (air {:.2}s), drive: slides {} boosts {} longest air {:.2}s walls {} (hairpin {}) stuck {} off {:.1}s, min clr {:.1}, water {:.0}%, {:?}",
            c.rank,
            c.score,
            c.name,
            if c.closed { "circuit" } else { "sprint" },
            m.length_m / 1000.0,
            d.map_or(0.0, |d| d.best_lap),
            m.est_time,
            m.road_frac * 100.0,
            m.bend_hist,
            m.slide_bends,
            m.hairpins,
            m.elev_min,
            m.elev_max,
            m.climb,
            m.jumps.len(),
            m.jumps.iter().map(|j| j.airtime).fold(0.0, f32::max),
            d.map_or(0, |d| d.powerslides),
            d.map_or(0, |d| d.boosts),
            d.map_or(0.0, |d| d.longest_air),
            d.map_or(0, |d| d.wall_contacts),
            d.map_or(0, |d| d.hairpin_contacts),
            d.map_or(0, |d| d.stuck_recoveries),
            d.map_or(0.0, |d| d.off_track_time),
            m.min_clearance,
            m.water_frac * 100.0,
            m.subzones,
        );
        let map = preview::map(&terrain, &result.field, c);
        let path = dir.join(format!("course-{}.png", c.rank));
        map.save(&path)
            .with_context(|| format!("writing {}", path.display()))?;
        let prof = preview::profile(c);
        let path = dir.join(format!("course-{}-profile.png", c.rank));
        prof.save(&path)
            .with_context(|| format!("writing {}", path.display()))?;
    }
    let json = serde_json::json!({
        "zone": terrain.zone,
        "map": terrain.map,
        "map_id": terrain.map_id,
        "units": "WoW world yards; route points (x north, y west, z up) every 3 yd from the start",
        "stats": s,
        "hubs": result.hubs,
        "courses": result.finalists,
    });
    let path = dir.join("candidates.json");
    std::fs::write(&path, serde_json::to_string_pretty(&json)?)
        .with_context(|| format!("writing {}", path.display()))?;
    say!(log, "wrote {}", dir.display());
    std::fs::write(dir.join("run.log"), &log)?;
    Ok(ZoneRun {
        zone: terrain.zone.clone(),
        candidates: path,
        generated: s.generated,
        driven: s.driven,
        kept: s.kept,
        top: result
            .finalists
            .first()
            .map(|c| (c.name.clone(), c.drive.as_ref().map_or(0.0, |d| d.best_lap))),
        log,
    })
}
