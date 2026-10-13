//! `ctr-extract <disc.bin> <out_dir> [--kart <name>] [--audio] [--effects] [--weapons] [--music] [--hud] [--all]`
//!
//! Reads the user's own Crash Team Racing (NTSC-U) disc image and writes `<out_dir>/kart.json`
//! (the chosen racer's kart, default `crash`). `--audio` also writes the kart sound effects to
//! `<out_dir>/audio/*.wav` and the characters' voices to `<out_dir>/audio/voice/`; `--effects`
//! writes `<out_dir>/effects.json` (turbo flames, exhaust smoke, skid marks, particles); `--weapons` writes `<out_dir>/weapons.json` (weapon, crate and
//! Wumpa models). `--all` writes `<out_dir>/kart-<name>.json` for every kart, the audio, the weapons
//! and the effects. The disc image is only read.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ctr_assets::big::BigFile;
use ctr_assets::disc::Disc;
use ctr_assets::kart::{self, KARTS};

fn usage() -> ExitCode {
    eprintln!("usage: ctr-extract <disc.bin|disc.iso> <out_dir> [--kart <name>] [--audio] [--effects] [--weapons] [--music] [--hud] [--all]");
    eprintln!("karts: {}", KARTS.join(", "));
    ExitCode::from(2)
}

fn write(path: &Path, value: &kart::KartFile) -> Result<(), String> {
    let data = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    let partial = path.with_extension("json.partial");
    std::fs::write(&partial, data).map_err(|e| format!("{}: {e}", partial.display()))?;
    std::fs::rename(&partial, path).map_err(|e| format!("{}: {e}", path.display()))
}

fn run(
    disc: &Path,
    out: &Path,
    kart_name: &str,
    all: bool,
    audio: bool,
    effects: bool,
    weapons: bool,
    music: bool,
    hud: bool,
) -> Result<(), String> {
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
        println!(
            "{name}: {} of {} triangles are kart, {} surfaces, {} textures",
            k.source.triangles_kart,
            k.source.triangles_total,
            k.surfaces.len(),
            k.textures.len()
        );
        if all {
            write(&out.join(format!("kart-{name}.json")), &k)?;
        }
        if name == kart_name {
            write(&out.join("kart.json"), &k)?;
        }
    }
    println!("wrote {}", out.join("kart.json").display());
    if hud || all {
        let h = ctr_assets::hud::build(&mut bf).map_err(|e| e.to_string())?;
        let path = out.join("hud.json");
        let data = serde_json::to_vec(&h).map_err(|e| e.to_string())?;
        std::fs::write(&path, data).map_err(|e| format!("{}: {e}", path.display()))?;
        println!("wrote {} ({} sprites)", path.display(), h.sprites.len());
    }
    if effects || all {
        let fx = ctr_assets::effects::build(&mut bf).map_err(|e| e.to_string())?;
        let path = out.join("effects.json");
        let data = serde_json::to_vec(&fx).map_err(|e| e.to_string())?;
        std::fs::write(&path, data).map_err(|e| format!("{}: {e}", path.display()))?;
        println!("wrote {} ({} textures)", path.display(), fx.textures.len());
    }
    if weapons || all {
        let w = ctr_assets::weapons::build(&mut bf).map_err(|e| e.to_string())?;
        let path = out.join("weapons.json");
        let data = serde_json::to_vec(&w).map_err(|e| e.to_string())?;
        std::fs::write(&path, data).map_err(|e| format!("{}: {e}", path.display()))?;
        for (key, m) in &w.models {
            println!("weapon {key:16} {:3} frames", m.frames.len());
        }
        println!(
            "wrote {} ({} models, {} textures)",
            path.display(),
            w.models.len(),
            w.textures.len()
        );
    }
    if audio || all {
        let dir = out.join("audio");
        for r in ctr_assets::audio::extract(&mut disc, &dir).map_err(|e| e.to_string())? {
            println!(
                "audio {:15} {:6} Hz {:5.2} s peak {:5}{}",
                r.file,
                r.rate,
                r.samples as f64 / r.rate as f64,
                r.peak,
                if r.looped { " loop" } else { "" }
            );
        }
        let voices = dir.join("voice");
        let lines = ctr_assets::voice::extract(&mut disc, &voices).map_err(|e| e.to_string())?;
        let seconds: f64 = lines.iter().map(|l| l.seconds).sum();
        println!(
            "voice {} XA lines, {seconds:.0} s, and each racer's two bank lines",
            lines.len()
        );
        println!("wrote {}", dir.display());
    }
    if music || all {
        let dir = out.join("music");
        let tracks = ctr_assets::music::extract(&mut disc, &dir, ctr_assets::music::OUT_RATE)
            .map_err(|e| e.to_string())?;
        for t in tracks {
            println!(
                "music {:16} {:6.1} s {:3} bpm peak {:5}{}",
                t.track,
                t.seconds,
                t.bpm,
                t.peak,
                t.same_as
                    .map(|s| format!(" (same as {s})"))
                    .unwrap_or_default()
            );
        }
        if !dir.join("final_lap.wav").is_file() {
            println!("music final-lap sting: none (no XA audio in a cooked image)");
        }
        println!("wrote {}", dir.display());
    }
    Ok(())
}

fn main() -> ExitCode {
    let mut positional = Vec::new();
    let mut kart_name = "crash".to_string();
    let mut all = false;
    let mut audio = false;
    let mut effects = false;
    let mut weapons = false;
    let mut music = false;
    let mut hud = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--kart" => match args.next() {
                Some(k) => kart_name = k.to_ascii_lowercase(),
                None => return usage(),
            },
            "--all" => all = true,
            "--audio" => audio = true,
            "--effects" => effects = true,
            "--weapons" => weapons = true,
            "--music" => music = true,
            "--hud" => hud = true,
            "-h" | "--help" => return usage(),
            _ => positional.push(PathBuf::from(a)),
        }
    }
    let [disc, out] = positional.as_slice() else {
        return usage();
    };
    if !KARTS.contains(&kart_name.as_str()) {
        eprintln!("unknown kart {kart_name}");
        return usage();
    }
    match run(
        disc, out, &kart_name, all, audio, effects, weapons, music, hud,
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ctr-extract: {e}");
            ExitCode::FAILURE
        }
    }
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
