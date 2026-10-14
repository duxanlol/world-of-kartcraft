//! World of Kartcraft's launcher: the one file a player downloads. Every run updates the game
//! (and the launcher itself) from the update host, finds or asks for the player's WoW 1.12.1
//! folder and Crash Team Racing disc image, converts the disc, and starts the game. On Windows it
//! keeps the game beside itself; on macOS it is the `World of Kartcraft.app` bundle's own
//! program, which moves the bundle into `~/Applications` on its first run and keeps the player's
//! settings and kart data under `~/Library/Application Support`. On Linux it is an AppImage that
//! keeps the game beside itself, as on Windows, and runs the Windows game through the player's
//! own Proton ([`proton`]). The platform seams are in `install` (where things live, the program
//! names), `find` (where to look for the games) and `launch` (how the programs start); the
//! window, the settings, the update and the search are shared.
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
mod app;
#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
mod config;
#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
mod find;
#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
mod install;
#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
mod launch;
#[cfg(target_os = "linux")]
mod proton;
#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
mod update;

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
fn main() {
    if let Some(code) = install::first_steps() {
        std::process::exit(code);
    }
    if let Err(e) = app::run() {
        install::fatal(&format!("The launcher could not open its window: {e}"));
    }
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn main() {
    eprintln!("The World of Kartcraft launcher is for Windows, macOS and Linux.");
}
