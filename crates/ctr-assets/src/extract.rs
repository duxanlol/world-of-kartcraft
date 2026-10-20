//! The whole conversion, the disc image into an output folder, as `ctr-extract` runs it and the
//! Android app runs it in-process: the karts (`kart.json`, and with [`Parts::all`] every racer's
//! `kart-<name>.json`), and whichever of the HUD, effects, weapons, props, sound effects, voices
//! and music [`Parts`] asks for. Each step's line goes to `say`, the binary's stdout.

use std::path::Path;

use crate::big::BigFile;
use crate::disc::Disc;
use crate::kart::{self, KARTS};

/// What to convert beside the chosen kart; `all` is every part and every kart.
#[derive(Debug, Clone, Copy, Default)]
pub struct Parts {
    pub all: bool,
    pub audio: bool,
    pub effects: bool,
    pub weapons: bool,
    pub music: bool,
    pub hud: bool,
    pub props: bool,
}

fn write(path: &Path, value: &kart::KartFile) -> Result<(), String> {
    let data = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    let partial = path.with_extension("json.partial");
    std::fs::write(&partial, data).map_err(|e| format!("{}: {e}", partial.display()))?;
    std::fs::rename(&partial, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Converts `disc` into `out`, `kart_name` as `kart.json`; the error is the player's message.
pub fn run(
    disc: &Path,
    out: &Path,
    kart_name: &str,
    parts: Parts,
    say: &mut dyn FnMut(String),
) -> Result<(), String> {
    let Parts {
        all,
        audio,
        effects,
        weapons,
        music,
        hud,
        props,
    } = parts;
    let mut disc = Disc::open(disc).map_err(|e| e.to_string())?;
    // Only the US disc's files sit where the bigfile's indices say (`big.rs`): another version
    // would convert into karts that are not there, so it is refused by name.
    match disc.game_id() {
        Ok(id) if id == "SCUS-94426" => {}
        Ok(id) => return Err(wrong_disc(&id)),
        Err(e) => {
            return Err(format!(
                "This doesn't look like a PlayStation disc ({e}). World of Kartcraft needs the US \
                 Crash Team Racing, SCUS-94426."
            ))
        }
    }
    let mut bf = BigFile::open(&mut disc).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let names: Vec<&str> = if all { KARTS.to_vec() } else { vec![kart_name] };
    for name in names {
        let k = kart::build(&mut bf, name).map_err(|e| e.to_string())?;
        say(format!(
            "{name}: {} of {} triangles are kart, {} surfaces, {} textures",
            k.source.triangles_kart,
            k.source.triangles_total,
            k.surfaces.len(),
            k.textures.len()
        ));
        if all {
            write(&out.join(format!("kart-{name}.json")), &k)?;
        }
        if name == kart_name {
            write(&out.join("kart.json"), &k)?;
        }
    }
    say(format!("wrote {}", out.join("kart.json").display()));
    if hud || all {
        let h = crate::hud::build(&mut bf).map_err(|e| e.to_string())?;
        let path = out.join("hud.json");
        let data = serde_json::to_vec(&h).map_err(|e| e.to_string())?;
        std::fs::write(&path, data).map_err(|e| format!("{}: {e}", path.display()))?;
        say(format!(
            "wrote {} ({} sprites)",
            path.display(),
            h.sprites.len()
        ));
    }
    if effects || all {
        let fx = crate::effects::build(&mut bf).map_err(|e| e.to_string())?;
        let path = out.join("effects.json");
        let data = serde_json::to_vec(&fx).map_err(|e| e.to_string())?;
        std::fs::write(&path, data).map_err(|e| format!("{}: {e}", path.display()))?;
        say(format!(
            "wrote {} ({} textures)",
            path.display(),
            fx.textures.len()
        ));
    }
    if weapons || all {
        let w = crate::weapons::build(&mut bf).map_err(|e| e.to_string())?;
        let path = out.join("weapons.json");
        let data = serde_json::to_vec(&w).map_err(|e| e.to_string())?;
        std::fs::write(&path, data).map_err(|e| format!("{}: {e}", path.display()))?;
        for (key, m) in &w.models {
            say(format!("weapon {key:16} {:3} frames", m.frames.len()));
        }
        say(format!(
            "wrote {} ({} models, {} textures)",
            path.display(),
            w.models.len(),
            w.textures.len()
        ));
    }
    if props || all {
        let file = crate::props::file(crate::props::build(&mut bf).map_err(|e| e.to_string())?);
        let path = out.join("props.json");
        let data = serde_json::to_vec(&file).map_err(|e| e.to_string())?;
        std::fs::write(&path, data).map_err(|e| format!("{}: {e}", path.display()))?;
        say(format!(
            "wrote {} ({} props, {} textures)",
            path.display(),
            file.props.len(),
            file.textures.len()
        ));
    }
    if audio || all {
        let dir = out.join("audio");
        for r in crate::audio::extract(&mut disc, &dir).map_err(|e| e.to_string())? {
            say(format!(
                "audio {:15} {:6} Hz {:5.2} s peak {:5}{}",
                r.file,
                r.rate,
                r.samples as f64 / r.rate as f64,
                r.peak,
                if r.looped { " loop" } else { "" }
            ));
        }
        let voices = dir.join("voice");
        let lines = crate::voice::extract(&mut disc, &voices).map_err(|e| e.to_string())?;
        let seconds: f64 = lines.iter().map(|l| l.seconds).sum();
        say(format!(
            "voice {} XA lines, {seconds:.0} s, and each racer's two bank lines",
            lines.len()
        ));
        say(format!("wrote {}", dir.display()));
    }
    if music || all {
        let dir = out.join("music");
        let tracks = crate::music::extract(&mut disc, &dir, crate::music::OUT_RATE)
            .map_err(|e| e.to_string())?;
        for t in tracks {
            say(format!(
                "music {:16} {:6.1} s {:3} bpm peak {:5}{}",
                t.track,
                t.seconds,
                t.bpm,
                t.peak,
                t.same_as
                    .map(|s| format!(" (same as {s})"))
                    .unwrap_or_default()
            ));
        }
        if !dir.join("final_lap.wav").is_file() {
            say("music final-lap sting: none (no XA audio in a cooked image)".into());
        }
        say(format!("wrote {}", dir.display()));
    }
    Ok(())
}

/// Another disc than the US Crash Team Racing, named by its id's publisher prefix.
fn wrong_disc(id: &str) -> String {
    let what = match id {
        "SCES-02105" => "the European Crash Team Racing",
        "SCPS-10118" | "SCPS-91230" => "the Japanese Crash Team Racing",
        _ if id.starts_with("SCES") || id.starts_with("SLES") => "a European PlayStation game",
        _ if id.starts_with("SCPS") || id.starts_with("SLPS") || id.starts_with("SCPM") => {
            "a Japanese PlayStation game"
        }
        _ => "another PlayStation game",
    };
    format!(
        "This disc image is {what} ({id}). World of Kartcraft needs the US version of Crash Team \
         Racing, SCUS-94426: other versions keep their files elsewhere, and the karts would come \
         out invisible."
    )
}
