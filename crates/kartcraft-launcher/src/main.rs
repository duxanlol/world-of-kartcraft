//! World of Kartcraft's launcher: the one file a player downloads. The first run installs it under
//! the user's AppData with a Desktop and a Start menu shortcut; every run updates the game (and
//! the launcher itself) from the update host, finds or asks for the player's WoW 1.12.1 folder and
//! Crash Team Racing disc image, converts the disc, and starts the game. A folder that already
//! holds the game (the zip's, with its `kartcraft.cfg`) is used where it is.
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod config;
#[cfg(windows)]
mod find;
#[cfg(windows)]
mod install;
#[cfg(windows)]
mod update;

#[cfg(windows)]
fn main() {
    if let Some(code) = install::first_steps() {
        std::process::exit(code);
    }
    if let Err(e) = app::run() {
        install::fatal(&format!("The launcher could not open its window: {e}"));
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("The World of Kartcraft launcher is for Windows; on Linux use play-proton.sh.");
}
