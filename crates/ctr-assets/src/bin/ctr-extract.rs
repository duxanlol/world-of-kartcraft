//! `ctr-extract <disc.bin> <out_dir> [--kart <name>] [--audio] [--effects] [--weapons] [--music] [--hud] [--props] [--all]`
//!
//! Reads the user's own Crash Team Racing (NTSC-U) disc image and writes `<out_dir>/kart.json`
//! (the chosen racer's kart, default `crash`). `--audio` also writes the kart sound effects to
//! `<out_dir>/audio/*.wav` and the characters' voices to `<out_dir>/audio/voice/`; `--effects`
//! writes `<out_dir>/effects.json` (turbo flames, exhaust smoke, skid marks, particles); `--weapons` writes `<out_dir>/weapons.json` (weapon, crate and
//! Wumpa models); `--props` writes `<out_dir>/props.json` (the track editor's CTR objects,
//! `ctr_assets::props`). `--all` writes `<out_dir>/kart-<name>.json` for every kart, the audio, the
//! weapons, the effects and the props. The disc image is only read.
//!
//! The conversion itself is [`ctr_assets::extract`], which the Android app runs in-process.

use std::path::PathBuf;
use std::process::ExitCode;

use ctr_assets::extract::Parts;
use ctr_assets::kart::KARTS;

fn usage() -> ExitCode {
    eprintln!("usage: ctr-extract <disc.bin|disc.iso> <out_dir> [--kart <name>] [--audio] [--effects] [--weapons] [--music] [--hud] [--props] [--all]");
    eprintln!("karts: {}", KARTS.join(", "));
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let mut positional = Vec::new();
    let mut kart_name = "crash".to_string();
    let mut parts = Parts::default();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--kart" => match args.next() {
                Some(k) => kart_name = k.to_ascii_lowercase(),
                None => return usage(),
            },
            "--all" => parts.all = true,
            "--audio" => parts.audio = true,
            "--effects" => parts.effects = true,
            "--weapons" => parts.weapons = true,
            "--music" => parts.music = true,
            "--hud" => parts.hud = true,
            "--props" => parts.props = true,
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
    match ctr_assets::extract::run(disc, out, &kart_name, parts, &mut |line| println!("{line}")) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ctr-extract: {e}");
            ExitCode::FAILURE
        }
    }
}
