//! Converts the user's own Crash Team Racing (PS1, NTSC-U, SCUS-94426) disc image into
//! `kart.json` (a racer's kart without its driver, its wheels and textures, ready to draw) and
//! the kart sound effects as WAV, its visual effects (`effects.json`), the 1P race HUD
//! (`hud.json`) and the race music (rendered from the CSEQ sequences).
//! No game data is part of this crate; everything is read from the disc at setup time.
//!
//! Format knowledge (the file layouts, re-implemented here in Rust) comes from:
//! - CTR-tools by DCxDemo and contributors (<https://github.com/CTR-tools/CTR-tools>, GPL-2.0, no version
//!   limit, so usable under GPL-3.0):
//!   BIGFILE, `.ctr` model, `.mpk`, `.vrm`/TIM, TextureLayout and HOWL bank parsing.
//! - CTR-native (the CTR decompilation, GPL-3.0): model command-list assembly
//!   (`RenderBucket_DrawFunc_Normal`), vertex frame decoding and delta decompression
//!   (`RenderBucket_UncompressAnimationFrame`), tire sprites and wheel placement
//!   (`DrawTires.c`, `VehBirth.c`), bigfile indices (`namespace_Load.h`), and the HOWL sound
//!   bank layout and the sound ids/pitches the kart plays (`HOWL/*.c`, `Vehicle/*.c`, `COLL.c`).
//!
//! Because of that, this crate is licensed GPL-3.0-or-later. See `docs/ctr-assets.md`.

pub mod audio;
pub mod big;
pub mod ctr;
pub mod disc;
pub mod effects;
pub mod hud;
pub mod hud_draw;
pub mod kart;
pub mod mpk;
pub mod music;
pub mod vram;

use std::fmt;

#[derive(Debug)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
