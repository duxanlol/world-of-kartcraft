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
//!
//! `course-gen gps` finds the cheapest drivable way between every pair of delivery towns over
//! each continent's drivability grid and lays them as the deliveries' GPS roads
//! (`roads/roads.json`, `kart_gps`).
//!
//! `course-gen showcase [site...] --out <dir> --write showcase/showcase.json` authors the
//! showcase runs (`/kartshow`) over the real world and keeps their input tapes.

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
        /// One designed circuit instead of generated ones, or the circuit of an instance whose
        /// ground is its streets (Stratholme): its waypoints,
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
    /// Make a gauntlet of a course (`course_gen::gauntlet`): its copy, one lap, hazards along
    /// its route, written into the course file (replacing one of the same key).
    Gauntlet {
        /// The course it is made from.
        #[arg(long)]
        from: String,
        /// The gauntlet's key and name.
        #[arg(long)]
        key: String,
        #[arg(long)]
        name: String,
        /// The course file.
        #[arg(long, default_value = "courses/courses.json")]
        courses: PathBuf,
    },
    /// Scout flight courses (`course_gen::scout`): tops chained down a glide at a time round a
    /// spot, flown by the flight bot; the ones it flies, as course entries.
    Scout {
        /// The map: 0 the Eastern Kingdoms, 1 Kalimdor.
        #[arg(long)]
        map: u32,
        /// The area's centre, WoW `x,y`.
        #[arg(long, allow_hyphen_values = true)]
        at: String,
        /// Its half-width, yards.
        #[arg(long, default_value_t = 300.0)]
        half: f32,
        /// About the ground's height there (WoW z).
        #[arg(long, allow_hyphen_values = true, default_value_t = 0.0)]
        z: f32,
        /// The courses kept.
        #[arg(long, default_value_t = 6)]
        keep: usize,
        /// How far over what is round it a top stands, yards.
        #[arg(long, default_value_t = 10.0)]
        prominence: f32,
        /// Yards across per yard down a hop asks of a glide.
        #[arg(long, default_value_t = 5.0)]
        glide: f32,
        /// The longest hop, yards.
        #[arg(long, default_value_t = 220.0)]
        hop: f32,
        /// The sampling step, yards.
        #[arg(long, default_value_t = 3.0)]
        step: f32,
        /// The fewest samples a top holds.
        #[arg(long, default_value_t = 6)]
        cells: usize,
        /// The most a walkable spot rises to its neighbour, yards.
        #[arg(long, default_value_t = 1.6)]
        flat: f32,
        /// Glide runs (`course_gen::scout::scout_runs`): long paths down off the highest tops, low
        /// and between higher ground, rings along them, in place of the hops between tops.
        #[arg(long)]
        runs: bool,
        /// Level hops: roof to roof as high or higher, on the speed off each top.
        #[arg(long)]
        level: bool,
        /// The rings' radius, yards.
        #[arg(long, default_value_t = 10.0)]
        ring: f32,
        /// A turbo pad on each top a hop leaves, pointing at the next.
        #[arg(long)]
        pads: bool,
        /// The shortest hop, yards.
        #[arg(long, default_value_t = 25.0)]
        hop_min: f32,
        /// With --runs: yards across per yard down the path keeps.
        #[arg(long, default_value_t = 5.5)]
        run_glide: f32,
        /// Write the courses found here, as course entries keyed `<prefix>-<n>`.
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long, default_value = "scout")]
        prefix: String,
    },
    /// Fly the flight bot (`course_gen::flight`) over the real world: a flight course's rings
    /// (`--course`), a flight distance as far as it goes (`--leap`), or a course's rings laid at
    /// heights it reaches (`--course` with `--lay`).
    Fly {
        /// A flight course's key.
        #[arg(long)]
        course: Option<String>,
        /// A flight distance's key.
        #[arg(long)]
        leap: Option<String>,
        /// Lay the course's rings this many yards under the bot's best line.
        #[arg(long)]
        lay: Option<f32>,
        /// With --lay: the least clearance, yards, between a ring's bottom and the ground.
        #[arg(long, default_value_t = 6.0)]
        clear: f32,
        /// With --lay: write the heights into the course file (the rings it could not lay
        /// dropped from the course's end).
        #[arg(long)]
        write: bool,
        /// Fly as the client's race autopilot does (full gas, steering for the next ring), to
        /// lay against a live run of it.
        #[arg(long)]
        simple: bool,
        /// Draw the course's map with the bot's flight into this PNG.
        #[arg(long)]
        picture: Option<PathBuf>,
        /// Move a start on a slope (steeper than a character stands on) to the nearest flat
        /// spot, written into the course file.
        #[arg(long)]
        flat_start: bool,
        /// The course file.
        #[arg(long, default_value = "courses/courses.json")]
        courses: PathBuf,
    },
    /// Place a Race Marshal beside every distinct course start, at the ground's height, and
    /// write each course's `"marshal"` into the course file.
    Marshals {
        /// The course file.
        #[arg(long, default_value = "courses/courses.json")]
        courses: PathBuf,
    },
    /// Build the deliveries' GPS roads (`course_gen::gps`) and route the delivery towns over them.
    Gps {
        /// The road file written.
        #[arg(long, default_value = "roads/roads.json")]
        out: PathBuf,
        /// The delivery towns routed between.
        #[arg(long, default_value = "deliveries/deliveries.json")]
        deliveries: PathBuf,
        /// Also draw each continent's grid into `<dir>/gps-map<id>.png`.
        #[arg(long)]
        render: Option<PathBuf>,
        /// Zones loaded, and towns searched from, at once (a large zone takes about 2 GB).
        #[arg(long, default_value_t = 3)]
        jobs: usize,
        /// Test-drive rounds at most (0: lay the roads undriven).
        #[arg(long, default_value_t = course_gen::gps::REPAIR_ROUNDS)]
        rounds: usize,
        /// One map only (0 the Eastern Kingdoms, 1 Kalimdor); the other keeps its roads.
        #[arg(long)]
        map: Option<u32>,
    },
    /// Measure, drive and draw every course (`course_gen::audit`): `<out>/audit.json` and each
    /// course's map and profile.
    Audit {
        #[arg(long, default_value = "courses/courses.json")]
        courses: PathBuf,
        #[arg(long)]
        out: PathBuf,
        /// Zones at once.
        #[arg(long, default_value_t = 3)]
        jobs: usize,
        /// Only the courses whose key holds this.
        #[arg(long)]
        only: Option<String>,
    },
    /// Drive the GPS roads with the CTR-physics bot (`course_gen::gps_drive`): every road cut
    /// into stretches, each driven over its zone's terrain; the ones it cannot drive listed.
    GpsDrive {
        /// The road file driven.
        #[arg(long, default_value = "roads/roads.json")]
        roads: PathBuf,
        /// Every stretch and how it drove, as JSON.
        #[arg(long)]
        out: PathBuf,
        /// One map only (0 the Eastern Kingdoms, 1 Kalimdor).
        #[arg(long)]
        map: Option<u32>,
        /// Zones loaded at once.
        #[arg(long, default_value_t = 3)]
        jobs: usize,
    },
    /// Lay turbo pads on every course's straights, at the ground's height, and write each
    /// course's `"pads"` into the course file (`course_gen::pads`).
    Pads {
        /// The course file.
        #[arg(long, default_value = "courses/courses.json")]
        courses: PathBuf,
    },
    /// Author the showcase runs (`course_gen::showcase`): fit each site's choreographies into its
    /// open ground, drive them, keep the best clean one, draw each site into `<out>/<name>.png`.
    Showcase {
        /// The sites to author (all with none): mirage, flats, aq, gurubashi, stormwind,
        /// thunderbluff, blackrock, bootybay.
        sites: Vec<String>,
        /// Where the pictures go.
        #[arg(long)]
        out: PathBuf,
        /// Write the runs into this showcase file, each replacing the run of its name.
        #[arg(long)]
        write: Option<PathBuf>,
        /// Layouts driven per site, at most.
        #[arg(long, default_value_t = 240)]
        budget: usize,
        /// Try the site at another spot, `x,y,z` (WoW).
        #[arg(long, allow_hyphen_values = true)]
        spot: Option<String>,
        /// How far above the spot the ground is looked for, yards.
        #[arg(long)]
        probe: Option<f32>,
        /// How far the ground may climb or fall from the spot's, yards.
        #[arg(long)]
        band: Option<f32>,
        /// The ground looked at, yards each way.
        #[arg(long)]
        span: Option<f32>,
        /// How far from the spot a run may start, yards.
        #[arg(long)]
        reach: Option<f32>,
        /// The clear ground a planned line keeps, yards.
        #[arg(long)]
        margin: Option<f32>,
        /// The engine class: balanced, accel, speed or turn.
        #[arg(long)]
        class: Option<String>,
        /// Boosts a slide takes before the next hop, 1 to 3.
        #[arg(long)]
        boosts: Option<u8>,
        /// The drift radii tried, yards, `a,b,c`.
        #[arg(long)]
        radii: Option<String>,
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
        Cmd::Gauntlet {
            from,
            key,
            name,
            courses,
        } => {
            let text = std::fs::read_to_string(&courses)
                .with_context(|| format!("reading {}", courses.display()))?;
            let mut file: serde_json::Value = serde_json::from_str(&text)?;
            let entry =
                course_gen::gauntlet::make(&mut open_install()?, &file, &from, &key, &name)?;
            let hazards = entry["hazards"].as_array().map_or(0, Vec::len);
            let list = file["courses"].as_array_mut().context("courses")?;
            list.retain(|c| c["key"] != key.as_str());
            list.push(entry);
            std::fs::write(&courses, course_gen::export::format(&file))?;
            println!("{key}: {name}, {hazards} hazards, from {from}");
        }
        Cmd::Scout {
            map,
            at,
            half,
            z,
            keep,
            prominence,
            glide,
            hop,
            step,
            cells,
            flat,
            runs,
            level,
            ring,
            pads,
            hop_min,
            run_glide,
            out,
            prefix,
        } => {
            let v: Vec<f32> = at
                .split(',')
                .map(|p| p.trim().parse())
                .collect::<Result<_, _>>()
                .context("--at is x,y")?;
            anyhow::ensure!(v.len() == 2, "--at is x,y");
            let opts = course_gen::scout::ScoutOpts {
                prominence,
                glide,
                hop: (hop_min, hop),
                level,
                ring,
                pads,
                step,
                min_cells: cells,
                flat,
                ..Default::default()
            };
            let found = if runs {
                let r = course_gen::scout::RunOpts {
                    glide: run_glide,
                    ..Default::default()
                };
                course_gen::scout::scout_runs(
                    &mut open_install()?,
                    map,
                    [v[0], v[1]],
                    half,
                    z,
                    &opts,
                    &r,
                    keep,
                )?
            } else {
                course_gen::scout::scout(
                    &mut open_install()?,
                    map,
                    [v[0], v[1]],
                    half,
                    z,
                    &opts,
                    keep,
                )?
            };
            let mut entries = Vec::new();
            for (n, f) in found.iter().enumerate() {
                let p = f.points[0];
                println!(
                    "{prefix}-{}: {} rings, {:.0} yd, {:.1} s, from {:.0},{:.0},{:.0}, heights {}",
                    n + 1,
                    f.passed,
                    f.yards,
                    f.seconds,
                    p[0],
                    p[1],
                    p[2],
                    f.points[1..]
                        .iter()
                        .map(|q| format!("{:.0}", q[2]))
                        .collect::<Vec<_>>()
                        .join(" ")
                );
                let key = format!("{prefix}-{}", n + 1);
                if let (Some(out), Some(pic)) = (&out, &f.picture) {
                    pic.save(out.with_file_name(format!("{key}.png")))?;
                }
                entries.push(course_gen::scout::course_entry(f, &key, &key, "", map));
            }
            if let Some(out) = out {
                std::fs::write(&out, serde_json::to_string_pretty(&entries)?)?;
            }
        }
        Cmd::Fly {
            course,
            leap,
            lay,
            clear,
            write,
            simple,
            picture,
            flat_start,
            courses,
        } => {
            let text = std::fs::read_to_string(&courses)
                .with_context(|| format!("reading {}", courses.display()))?;
            let mut file: serde_json::Value = serde_json::from_str(&text)?;
            let mut chain = open_install()?;
            if let Some(key) = leap {
                let (yards, r) = course_gen::flight::fly_leap(&mut chain, &file, &key)?;
                println!("{key}: {yards:.1} yd in {:.1} s", r.ticks as f32 / 30.0);
            }
            if let Some(key) = course {
                if let Some(margin) = lay {
                    let laid = course_gen::flight::lay(&mut chain, &file, &key, margin, clear)?;
                    for (n, z) in laid.iter().enumerate() {
                        println!("ring {}: z {z:.1}", n + 1);
                    }
                    if write {
                        course_gen::flight::write_heights(&mut file, &key, &laid)?;
                        std::fs::write(&courses, course_gen::export::format(&file))?;
                        println!("{} rings written into {}", laid.len(), courses.display());
                    }
                } else {
                    let mut c = course_gen::flight::FlightCourse::read(&file, &key)?;
                    let sky = course_gen::flight::Sky::gather(&mut chain, c.map, &c.points)?;
                    let s = c.points[0];
                    let steep = course_gen::flight::slope(&sky, s[0], s[1], 3.0);
                    let grid = course_gen::flight::grid_ok(&sky, s, c.points[1]);
                    let slot = course_gen::flight::grid_slot(s, c.points[1]);
                    println!(
                        "start slope {steep:.2}{}, grid slot floor {}{}",
                        if steep > course_gen::flight::FLAT {
                            " (too steep)"
                        } else {
                            ""
                        },
                        sky.floor(slot[0], slot[1])
                            .map_or("?".into(), |z| format!("{:+.1} yd", z - s[2])),
                        if grid { "" } else { " (no grid)" }
                    );
                    let bare = course_gen::flight::grid_on_terrain(&sky, s, c.points[1]);
                    println!("grid on bare terrain: {bare}");
                    if flat_start && (steep > course_gen::flight::FLAT || !grid || !bare) {
                        // The nearest flat spot the bot flies every ring from.
                        let all = c.rings.len();
                        let mut moved = None;
                        for p in course_gen::flight::flat_starts(&sky, &c)
                            .into_iter()
                            .take(40)
                        {
                            let mut t = course_gen::flight::FlightCourse::read(&file, &key)?;
                            t.points[0] = p;
                            let (l, _) = course_gen::flight::fly_in_as(&sky, &t, false)?;
                            if l.len() == all && l.iter().all(|l| l.passed) {
                                moved = Some(p);
                                break;
                            }
                        }
                        let p = moved.ok_or_else(|| {
                            anyhow::anyhow!("{key}: no flat start within 100 yd the bot flies from")
                        })?;
                        println!("start moved to {:.1},{:.1},{:.1}", p[0], p[1], p[2]);
                        c.points[0] = p;
                        course_gen::flight::write_start(&mut file, &key, p)?;
                        std::fs::write(&courses, course_gen::export::format(&file))?;
                    }
                    let (lines, r) = course_gen::flight::fly_in_as(&sky, &c, simple)?;
                    if let Some(png) = &picture {
                        let flown: Vec<[f32; 3]> = r.path.iter().map(|&p| sky.wow(p)).collect();
                        course_gen::scout::course_picture(&sky, &c.points, &flown).save(png)?;
                    }
                    let c = course_gen::flight::FlightCourse::read(&file, &key)?;
                    let zone = |chain: &mut benilla_formats::Chain, p: [f32; 3]| {
                        course_gen::flight::zone_at(chain, c.map, p[0], p[1]).unwrap_or_default()
                    };
                    let (from, to) = (c.points[0], *c.points.last().unwrap_or(&c.points[0]));
                    println!(
                        "zones: {} to {}",
                        zone(&mut chain, from),
                        zone(&mut chain, to)
                    );
                    for l in &lines {
                        let under = l.under.map_or("?".into(), |g| format!("{g:.0}"));
                        println!(
                            "ring {}: {} at {:.2} s, z {:.1} (ring {:.1}), {:.1} yd off centre, ground {under}{}",
                            l.ring,
                            if l.passed { "passed" } else { "MISSED" },
                            l.secs,
                            l.z,
                            l.ring_z,
                            l.off,
                            if l.on_prop { " (on a prop)" } else { "" }
                        );
                    }
                    println!(
                        "{} of the rings in {:.1} s",
                        r.passes.len(),
                        r.ticks as f32 / 30.0
                    );
                }
            }
        }
        Cmd::Marshals { courses } => {
            let n = course_gen::marshal::run(&courses, open_install()?)?;
            println!("{n} marshals written into {}", courses.display());
        }
        Cmd::Showcase {
            sites,
            out,
            write,
            budget,
            spot,
            probe,
            band,
            span,
            reach,
            margin,
            class,
            boosts,
            radii,
        } => {
            let spot = match spot {
                Some(s) => {
                    let v: Vec<f32> = s
                        .split(',')
                        .map(|p| p.trim().parse())
                        .collect::<Result<_, _>>()
                        .context("--spot is x,y,z")?;
                    anyhow::ensure!(v.len() == 3, "--spot is x,y,z");
                    Some([v[0], v[1], v[2]])
                }
                None => None,
            };
            let class = match class.as_deref() {
                None => None,
                Some("balanced") => Some(kart_driver::Class::Balanced),
                Some("accel") => Some(kart_driver::Class::Accel),
                Some("speed") => Some(kart_driver::Class::Speed),
                Some("turn") => Some(kart_driver::Class::Turn),
                Some(other) => anyhow::bail!("no class {other}"),
            };
            let radii: Option<&'static [f32]> = match radii {
                Some(r) => {
                    let v: Vec<f32> = r
                        .split(',')
                        .map(|p| p.trim().parse())
                        .collect::<Result<_, _>>()
                        .context("--radii is a,b,c")?;
                    Some(Box::leak(v.into_boxed_slice()))
                }
                None => None,
            };
            let overrides = course_gen::showcase::Overrides {
                spot,
                probe,
                band,
                span,
                reach,
                margin,
                class,
                boosts,
                radii,
            };
            let log = course_gen::showcase::author_sites(
                &mut open_install()?,
                &sites,
                &out,
                write.as_deref(),
                budget,
                &overrides,
            )?;
            print!("{log}");
        }
        Cmd::Gps {
            out,
            deliveries,
            render,
            jobs,
            rounds,
            map,
        } => {
            // The roads written before, audited against the new grid before they are replaced.
            let previous = std::fs::read_to_string(&out)
                .ok()
                .and_then(|s| kart_gps::RoadFile::parse(&s).ok());
            let (mut roads, report) = course_gen::gps::run(
                &deliveries,
                jobs,
                render.as_deref(),
                previous.as_ref(),
                rounds,
                map,
            )?;
            // One map rebuilt: the other's roads as they were.
            if let (Some(only), Some(old)) = (map, previous.as_ref()) {
                for m in &old.maps {
                    if m.map != only {
                        roads.maps.push(m.clone());
                    }
                }
                roads.maps.sort_by_key(|m| m.map);
            }
            if let Some(dir) = out.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&out, serde_json::to_string(&roads)?)
                .with_context(|| format!("writing {}", out.display()))?;
            print!("{report}");
            println!("wrote {}", out.display());
        }
        Cmd::Audit {
            courses,
            out,
            jobs,
            only,
        } => {
            let all = course_gen::audit::run(&courses, &out, jobs, only.as_deref())?;
            let finished = all
                .iter()
                .filter(|a| a.drive.as_ref().is_some_and(|d| d.finished))
                .count();
            println!(
                "{} courses audited, {finished} driven to the finish; wrote {}",
                all.len(),
                out.join("audit.json").display()
            );
        }
        Cmd::GpsDrive {
            roads,
            out,
            map,
            jobs,
        } => {
            let text = std::fs::read_to_string(&roads)
                .with_context(|| format!("reading {}", roads.display()))?;
            let file = kart_gps::RoadFile::parse(&text)?;
            let driven = course_gen::gps_drive::run(&file, map, jobs)?;
            std::fs::write(&out, serde_json::to_string(&driven)?)
                .with_context(|| format!("writing {}", out.display()))?;
            let mut why: std::collections::BTreeMap<String, (usize, f32)> = Default::default();
            for s in driven.iter().filter(|s| !s.ok) {
                let reason = s
                    .reject
                    .as_deref()
                    .unwrap_or("")
                    .split(" (")
                    .next()
                    .unwrap_or("");
                let e = why.entry(reason.to_string()).or_default();
                e.0 += 1;
                e.1 += s.length;
            }
            let total: f32 = driven.iter().map(|s| s.length).sum();
            let bad: f32 = driven.iter().filter(|s| !s.ok).map(|s| s.length).sum();
            println!(
                "{} stretches, {:.0} yd; not drivable: {} stretches, {:.0} yd ({:.1}%)",
                driven.len(),
                total,
                driven.iter().filter(|s| !s.ok).count(),
                bad,
                100.0 * bad / total.max(1.0)
            );
            for (reason, (n, yards)) in &why {
                println!("  {reason}: {n} stretches, {yards:.0} yd");
            }
            println!("wrote {}", out.display());
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
            // The render's frame, to turn a pixel on it into a `--via` waypoint.
            println!(
                "grid: north x {:.1}, west y {:.1}, {} yd/px, {}x{} px",
                g.north_x, g.west_y, g.yards_per_px, g.width, g.height
            );

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
