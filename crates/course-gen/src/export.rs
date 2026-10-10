//! Export finalists into `courses/courses.json` (`docs/courses.md`): a stable key and a short
//! banner name per course, its checkpoints with their gate half-widths, added or replaced by key.
//! [`validate`] mirrors the checks `setup/vmangos/pack.py` makes on the file.

use std::path::Path;

use anyhow::{bail, Context, Result};
use serde_json::{Map, Value};

/// The longest name the in-game banner shows whole.
pub const MAX_NAME: usize = 40;
/// The longest key the wire and the server table take.
pub const MAX_KEY: usize = 32;

/// A short, stable slug for a zone, the prefix of its courses' keys.
pub fn zone_slug(zone: &str) -> String {
    const SHORT: &[(&str, &str)] = &[
        ("Elwynn Forest", "elwynn"),
        ("Dun Morogh", "dunmorogh"),
        ("Loch Modan", "lochmodan"),
        ("Redridge Mountains", "redridge"),
        ("Arathi Highlands", "arathi"),
        ("Hillsbrad Foothills", "hillsbrad"),
        ("Alterac Mountains", "alterac"),
        ("Stranglethorn Vale", "stv"),
        ("Swamp of Sorrows", "sorrows"),
        ("Blasted Lands", "blasted"),
        ("Searing Gorge", "searing"),
        ("Burning Steppes", "steppes"),
        ("Western Plaguelands", "wpl"),
        ("Eastern Plaguelands", "epl"),
        ("Tirisfal Glades", "tirisfal"),
        ("Silverpine Forest", "silverpine"),
        ("The Hinterlands", "hinterlands"),
        ("Deadwind Pass", "deadwind"),
        ("Stonetalon Mountains", "stonetalon"),
        ("Thousand Needles", "needles"),
        ("Un'Goro Crater", "ungoro"),
        ("Dustwallow Marsh", "dustwallow"),
        ("The Barrens", "barrens"),
    ];
    if let Some((_, s)) = SHORT.iter().find(|(z, _)| z.eq_ignore_ascii_case(zone)) {
        return (*s).to_string();
    }
    let words = slug_words(zone);
    words
        .iter()
        .find(|w| *w != "the")
        .cloned()
        .unwrap_or_else(|| "zone".into())
}

/// Lowercase a-z0-9 words of `s`, with possessive `'s` dropped.
fn slug_words(s: &str) -> Vec<String> {
    s.to_ascii_lowercase()
        .replace("'s", "")
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// Words a place name shares with many others: a key takes the first word that is not one.
const GENERIC: &[&str] = &[
    "the",
    "of",
    "a",
    "an",
    "and",
    "mine",
    "farm",
    "farmstead",
    "lake",
    "hill",
    "hills",
    "pond",
    "place",
    "tower",
    "camp",
    "village",
    "ruins",
    "river",
    "falls",
    "pass",
    "road",
    "field",
    "fields",
    "patch",
    "plains",
    "keep",
    "post",
    "outpost",
    "point",
    "grove",
    "glade",
    "vale",
    "valley",
    "isle",
    "island",
    "crossing",
    "landing",
    "bridge",
    "hold",
    "ridge",
    "peak",
    "cave",
    "den",
    "lair",
    "shore",
    "coast",
    "beach",
    "fort",
    "citadel",
    "tomb",
    "pumpkin",
    "vineyards",
    "gorge",
    "canyon",
    "woods",
    "forest",
    "marsh",
    "lodge",
    "inn",
    "station",
    "estate",
    "lands",
];

/// The circuit or sprint name split into its places and its kind.
struct Parsed {
    places: Vec<String>,
    sprint: bool,
    /// A repeat's numeral ("II"), if any.
    numeral: Option<String>,
}

fn parse_name(name: &str) -> Parsed {
    let mut rest = name.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut numeral = None;
    if let Some((head, tail)) = rest.rsplit_once(' ') {
        if !tail.is_empty()
            && (tail.chars().all(|c| matches!(c, 'I' | 'V' | 'X'))
                || tail.chars().all(|c| c.is_ascii_digit()))
        {
            numeral = Some(tail.to_string());
            rest = head.to_string();
        }
    }
    let mut sprint = false;
    if let Some(r) = rest.strip_suffix(" Circuit") {
        rest = r.to_string();
    } else if let Some(r) = rest.strip_suffix(" Sprint") {
        rest = r.to_string();
        sprint = true;
    }
    let places = if sprint {
        rest.split(" to ").map(str::to_string).collect()
    } else {
        rest.split(" & ").map(str::to_string).collect()
    };
    Parsed {
        places,
        sprint,
        numeral,
    }
}

fn numeral_value(n: &str) -> usize {
    if let Ok(v) = n.parse() {
        return v;
    }
    match n {
        "II" => 2,
        "III" => 3,
        "IV" => 4,
        "V" => 5,
        "VI" => 6,
        "VII" => 7,
        "VIII" => 8,
        _ => 9,
    }
}

/// The banner name: the candidate's name without "Circuit", at most [`MAX_NAME`] characters,
/// shortened by dropping a leading "The", then "Sprint", then each place's generic words.
pub fn short_name(name: &str) -> String {
    let p = parse_name(name);
    let suffix = p
        .numeral
        .as_ref()
        .map_or(String::new(), |n| format!(" {n}"));
    let join = |places: &[String], sprint_word: bool| {
        let mut s = if p.sprint {
            places.join(" to ")
        } else {
            places.join(" & ")
        };
        if p.sprint && sprint_word {
            s.push_str(" Sprint");
        }
        s.push_str(&suffix);
        s
    };
    let mut places = p.places.clone();
    let first = join(&places, true);
    if first.len() <= MAX_NAME {
        return first;
    }
    for pl in &mut places {
        if let Some(r) = pl.strip_prefix("The ") {
            *pl = r.to_string();
        }
    }
    for sprint_word in [true, false] {
        let s = join(&places, sprint_word);
        if s.len() <= MAX_NAME {
            return s;
        }
    }
    // Each place down to its distinctive words.
    let trimmed: Vec<String> = places
        .iter()
        .map(|pl| {
            let keep: Vec<&str> = pl
                .split(' ')
                .filter(|w| !GENERIC.contains(&w.to_ascii_lowercase().replace("'s", "").as_str()))
                .collect();
            if keep.is_empty() {
                pl.clone()
            } else {
                keep.join(" ")
            }
        })
        .collect();
    let s = join(&trimmed, false);
    if s.len() <= MAX_NAME {
        return s;
    }
    let mut cut: String = s.chars().take(MAX_NAME).collect();
    if let Some(i) = cut.rfind(' ') {
        cut.truncate(i);
    }
    cut.trim_end_matches([' ', '&']).to_string()
}

/// The key: the zone's slug, then each place's first distinctive word, `-sprint` for a sprint
/// and the repeat's number; at most [`MAX_KEY`] characters, cut at a word.
pub fn course_key(zone: &str, name: &str) -> String {
    let p = parse_name(name);
    let zs = zone_slug(zone);
    let zone_words = slug_words(zone);
    let mut parts = vec![zs.clone()];
    for pl in &p.places {
        let words = slug_words(pl);
        // A place that is the zone itself ("Elwynn Forest Fields") reads as "fields".
        if words
            .iter()
            .all(|w| zone_words.contains(w) || w == "fields")
        {
            parts.push(if pl.contains("Fields") {
                "fields".into()
            } else {
                "wilds".into()
            });
            continue;
        }
        let w = words
            .iter()
            .find(|w| !GENERIC.contains(&w.as_str()) && !zone_words.contains(w))
            .or_else(|| words.iter().find(|w| *w != "the"))
            .or(words.first());
        if let Some(w) = w {
            parts.push(w.clone());
        }
    }
    if p.sprint {
        parts.push("sprint".into());
    }
    if let Some(n) = &p.numeral {
        parts.push(numeral_value(n).to_string());
    }
    // Too long: the longest place word loses a letter at a time, so every part survives.
    let places = 1..1 + p.places.len();
    while parts.join("-").len() > MAX_KEY {
        let Some(i) = places
            .clone()
            .filter(|&i| parts[i].len() > 3)
            .max_by_key(|&i| parts[i].len())
        else {
            break;
        };
        parts[i].pop();
    }
    parts.join("-").chars().take(MAX_KEY).collect()
}

fn round1(v: f64) -> Value {
    let r = (v * 10.0).round() / 10.0;
    if r == r.trunc() && r.abs() < 1e15 {
        Value::from(r as i64)
    } else {
        Value::from(r)
    }
}

/// One course entry from a candidate of `candidates.json`.
pub fn course_entry(zone: &str, map_id: u64, cand: &Value, key: &str) -> Result<Value> {
    let name = cand["name"].as_str().context("candidate without a name")?;
    let laps = cand["laps"].as_u64().unwrap_or(0);
    let mut cps = Vec::new();
    for p in cand["checkpoints"]
        .as_array()
        .context("candidate without checkpoints")?
    {
        let mut o = Map::new();
        o.insert("x".into(), round1(p["x"].as_f64().context("checkpoint x")?));
        o.insert("y".into(), round1(p["y"].as_f64().context("checkpoint y")?));
        if let Some(z) = p["z"].as_f64() {
            o.insert("z".into(), round1(z));
        }
        if let Some(g) = p["half_width"].as_f64() {
            o.insert("gate".into(), round1(g));
        }
        cps.push(Value::Object(o));
    }
    let km = cand["metrics"]["length_m"].as_f64().unwrap_or(0.0) / 1000.0;
    let lap = cand["drive"]["best_lap"].as_f64().unwrap_or(0.0);
    let note = format!(
        "Generated by course-gen ({}, seed {}): {:.2} km, test lap {:.1}s.",
        name,
        cand["seed"].as_u64().unwrap_or(0),
        km,
        lap
    );
    let mut o = Map::new();
    o.insert("key".into(), key.into());
    o.insert("name".into(), short_name(name).into());
    o.insert("zone".into(), zone.into());
    o.insert("map".into(), map_id.into());
    o.insert("laps".into(), laps.into());
    o.insert("note".into(), note.into());
    o.insert("checkpoints".into(), Value::Array(cps));
    Ok(Value::Object(o))
}

/// The selected candidates of a `candidates.json` as course entries, keys unique among
/// themselves (a repeat gets `-2`, `-3`).
pub fn entries(candidates: &Value, ranks: Option<&[usize]>, min_score: f32) -> Result<Vec<Value>> {
    let zone = candidates["zone"]
        .as_str()
        .context("candidates.json: zone")?;
    let map_id = candidates["map_id"]
        .as_u64()
        .context("candidates.json: map_id")?;
    let mut out: Vec<Value> = Vec::new();
    for c in candidates["courses"]
        .as_array()
        .context("candidates.json: courses")?
    {
        let rank = c["rank"].as_u64().unwrap_or(0) as usize;
        if ranks.is_some_and(|r| !r.contains(&rank))
            || c["score"].as_f64().unwrap_or(0.0) < f64::from(min_score)
        {
            continue;
        }
        let name = c["name"].as_str().unwrap_or("");
        let base = course_key(zone, name);
        let mut key = base.clone();
        let mut n = 2;
        while out.iter().any(|e| e["key"] == key.as_str()) {
            let tail = format!("-{n}");
            key = format!("{}{tail}", &base[..base.len().min(MAX_KEY - tail.len())]);
            n += 1;
        }
        out.push(course_entry(zone, map_id, c, &key)?);
    }
    if let Some(r) = ranks {
        for want in r {
            if !candidates["courses"]
                .as_array()
                .is_some_and(|cs| cs.iter().any(|c| c["rank"].as_u64() == Some(*want as u64)))
            {
                bail!("no candidate of rank {want}");
            }
        }
    }
    Ok(out)
}

/// Adds `new` to the file's courses, each replacing the course of its key in place (or, unless
/// `replace`, taking the key's first free `-2`, `-3` instead) or appended at the end; `remove`
/// keys are dropped first. Returns the entries as written.
pub fn merge(
    file: &mut Value,
    new: Vec<Value>,
    remove: &[String],
    replace: bool,
) -> Result<Vec<Value>> {
    let courses = file
        .get_mut("courses")
        .and_then(Value::as_array_mut)
        .context("courses.json: no \"courses\" list")?;
    courses.retain(|c| {
        !c["key"]
            .as_str()
            .is_some_and(|k| remove.iter().any(|r| r == k))
    });
    let mut written = Vec::new();
    for mut e in new {
        if !replace {
            let base = e["key"].as_str().unwrap_or_default().to_string();
            let mut n = 2;
            while courses.iter().any(|c| c["key"] == e["key"]) {
                let tail = format!("-{n}");
                e["key"] =
                    format!("{}{tail}", &base[..base.len().min(MAX_KEY - tail.len())]).into();
                n += 1;
            }
        }
        written.push(e.clone());
        match courses.iter_mut().find(|c| c["key"] == e["key"]) {
            Some(slot) => *slot = e,
            None => courses.push(e),
        }
    }
    Ok(written)
}

const ORDER: &[&str] = &[
    "key",
    "name",
    "zone",
    "map",
    "laps",
    "note",
    "checkpoints",
    "crates",
];

fn inline(v: &Value) -> String {
    match v {
        Value::Object(o) => {
            let fields: Vec<String> = o
                .iter()
                .map(|(k, v)| format!("{}: {}", Value::from(k.as_str()), v))
                .collect();
            format!("{{ {} }}", fields.join(", "))
        }
        _ => v.to_string(),
    }
}

/// The file as written: a course's fields in [`ORDER`], one checkpoint or crate per line.
pub fn format(file: &Value) -> String {
    let mut s = String::from("{\n  \"courses\": [\n");
    let courses = file["courses"].as_array().cloned().unwrap_or_default();
    for (i, c) in courses.iter().enumerate() {
        s.push_str("    {\n");
        let Some(o) = c.as_object() else {
            continue;
        };
        let mut keys: Vec<&String> = o.keys().collect();
        keys.sort_by_key(|k| {
            (
                ORDER.iter().position(|o| o == k).unwrap_or(ORDER.len()),
                (*k).clone(),
            )
        });
        for (j, k) in keys.iter().enumerate() {
            let v = &o[k.as_str()];
            let name = Value::from(k.as_str());
            match v {
                Value::Array(a) if a.iter().all(Value::is_object) && !a.is_empty() => {
                    s.push_str(&format!("      {name}: [\n"));
                    for (n, item) in a.iter().enumerate() {
                        let comma = if n + 1 < a.len() { "," } else { "" };
                        s.push_str(&format!("        {}{comma}\n", inline(item)));
                    }
                    s.push_str("      ]");
                }
                _ => s.push_str(&format!("      {name}: {v}")),
            }
            s.push_str(if j + 1 < keys.len() { ",\n" } else { "\n" });
        }
        s.push_str(if i + 1 < courses.len() {
            "    },\n"
        } else {
            "    }\n"
        });
    }
    s.push_str("  ]\n}\n");
    s
}

/// The checks `setup/vmangos/pack.py` makes, but the course count (the table is not capped
/// here): an error names the first offence.
pub fn validate(file: &Value) -> Result<()> {
    let courses = file["courses"]
        .as_array()
        .filter(|c| !c.is_empty())
        .context("\"courses\" must be a non-empty list")?;
    let mut keys = std::collections::HashSet::new();
    let num = |v: &Value, what: &str| -> Result<f64> {
        let Some(x) = v.as_f64() else {
            bail!("{what} is not a number")
        };
        if !x.is_finite() || x.abs() > 1e6 {
            bail!("{what} is out of range");
        }
        Ok(x)
    };
    for (n, c) in courses.iter().enumerate() {
        let key = c["key"].as_str().unwrap_or("");
        if key.is_empty()
            || key.len() > MAX_KEY
            || key == "any"
            || !key
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        {
            bail!("course {n}: key must be 1-32 of a-z 0-9 _ - and not \"any\"");
        }
        if !keys.insert(key) {
            bail!("course {key}: key used twice");
        }
        let name = c["name"].as_str().unwrap_or("");
        if name.is_empty() || !name.is_ascii() || name.contains(['\t', '\n', '\r', '"', '\\']) {
            bail!("course {key}: name must be a non-empty ASCII string without tabs, quotes or backslashes");
        }
        if !c["map"].as_u64().is_some_and(|m| m < 1 << 32) {
            bail!("course {key}: map must be a map id");
        }
        if !c["laps"].as_u64().is_some_and(|l| l <= 255) {
            bail!("course {key}: laps must be 0 (a sprint) to 255");
        }
        let cps = c["checkpoints"]
            .as_array()
            .filter(|p| p.len() >= 2)
            .with_context(|| format!("course {key}: at least two checkpoints"))?;
        for (i, p) in cps.iter().enumerate() {
            if !p.is_object() {
                bail!("course {key}: checkpoint {i} is not an object");
            }
            num(&p["x"], &format!("course {key}: checkpoint {i} x"))?;
            num(&p["y"], &format!("course {key}: checkpoint {i} y"))?;
            if p.get("z").is_some() {
                num(&p["z"], &format!("course {key}: checkpoint {i} z"))?;
            }
            if p.get("gate").is_some()
                && num(&p["gate"], &format!("course {key}: checkpoint {i} gate"))? < 0.0
            {
                bail!("course {key}: checkpoint {i} gate is negative");
            }
        }
        for (i, s) in c["crates"].as_array().into_iter().flatten().enumerate() {
            if !matches!(s["kind"].as_str(), Some("F" | "Q" | "W")) {
                bail!("course {key}: crate {i} needs a kind of F, Q or W");
            }
            num(&s["x"], &format!("course {key}: crate {i} x"))?;
            num(&s["y"], &format!("course {key}: crate {i} y"))?;
            if s.get("z").is_some() {
                num(&s["z"], &format!("course {key}: crate {i} z"))?;
            }
        }
    }
    Ok(())
}

/// Reads `candidates`, exports the chosen ranks (all when `None`) scoring at least `min_score`
/// into `courses`, drops
/// `remove`, validates and writes the file; see [`merge`] for `replace`. Returns the exported
/// entries.
pub fn export(
    candidates: &Path,
    ranks: Option<&[usize]>,
    min_score: f32,
    courses: &Path,
    remove: &[String],
    replace: bool,
) -> Result<Vec<Value>> {
    let cand: Value = serde_json::from_str(
        &std::fs::read_to_string(candidates)
            .with_context(|| format!("reading {}", candidates.display()))?,
    )
    .with_context(|| format!("parsing {}", candidates.display()))?;
    let new = entries(&cand, ranks, min_score)?;
    let mut file: Value = serde_json::from_str(
        &std::fs::read_to_string(courses)
            .with_context(|| format!("reading {}", courses.display()))?,
    )
    .with_context(|| format!("parsing {}", courses.display()))?;
    let written = merge(&mut file, new, remove, replace)?;
    validate(&file).with_context(|| format!("{} would be invalid", courses.display()))?;
    let text = format(&file);
    // The written text must read back as the same file.
    let back: Value = serde_json::from_str(&text).context("re-reading the formatted file")?;
    anyhow::ensure!(back == file, "formatting changed the file");
    std::fs::write(courses, text).with_context(|| format!("writing {}", courses.display()))?;
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_names() {
        assert_eq!(
            course_key("Westfall", "Jangolode Mine & Saldean's Farm Circuit"),
            "westfall-jangolode-saldean"
        );
        assert_eq!(
            course_key("Elwynn Forest", "Tower of Azora & Jasperlode Mine Circuit"),
            "elwynn-azora-jasperlode"
        );
        assert_eq!(
            course_key(
                "Elwynn Forest",
                "Brackwell Pumpkin Patch to Elwynn Forest Sprint"
            ),
            "elwynn-brackwell-wilds-sprint"
        );
        assert_eq!(
            course_key("Westfall", "Longshore & Saldean's Farm Circuit II"),
            "westfall-longshore-saldean-2"
        );
        assert_eq!(
            short_name("Jangolode Mine & Saldean's Farm Circuit"),
            "Jangolode Mine & Saldean's Farm"
        );
        assert_eq!(
            short_name("Brackwell Pumpkin Patch to Elwynn Forest Sprint"),
            "Brackwell Pumpkin Patch to Elwynn Forest"
        );
        let long = short_name("The Stonefield Farm & The Maclure Vineyards Circuit III");
        assert!(long.len() <= MAX_NAME, "{long}");
        for z in [
            "The Barrens",
            "Un'Goro Crater",
            "Stranglethorn Vale",
            "Durotar",
        ] {
            let k = course_key(z, "The Great Lift & Crossroads Circuit");
            assert!(k.len() <= MAX_KEY && !k.contains("--"), "{k}");
        }
    }

    #[test]
    fn merge_replaces_by_key_and_format_round_trips() {
        let mut file = serde_json::json!({"courses": [
            {"key": "sprint", "name": "A", "map": 0, "laps": 0,
             "checkpoints": [{"x": 1, "y": 2}, {"x": 3, "y": 4}]},
            {"key": "old", "name": "B", "map": 0, "laps": 3,
             "checkpoints": [{"x": 1, "y": 2}, {"x": 3, "y": 4}]}
        ]});
        let e = serde_json::json!({"key": "sprint", "name": "C", "map": 1, "laps": 0,
            "checkpoints": [{"x": 1.5, "y": 2, "gate": 15}, {"x": 3, "y": 4}]});
        merge(&mut file, vec![e.clone()], &["old".to_string()], true).unwrap();
        assert_eq!(file["courses"].as_array().unwrap().len(), 1);
        assert_eq!(file["courses"][0]["name"], "C");
        validate(&file).unwrap();
        let back: Value = serde_json::from_str(&format(&file)).unwrap();
        assert_eq!(back, file);
        merge(&mut file, vec![e], &[], false).unwrap();
        assert_eq!(file["courses"][1]["key"], "sprint-2");
        file["courses"][0]["name"] = "a \"quoted\" name".into();
        assert!(validate(&file).is_err());
    }
}
