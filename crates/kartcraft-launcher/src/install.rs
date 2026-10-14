//! Where the launcher keeps the game, per platform.
//!
//! Windows: portable, nothing installed. In a folder that already holds the game (the zip's, with
//! its `kartcraft.cfg` or `WorldOfKartcraft.exe`) it works there; anywhere else (Downloads, the
//! Desktop) in a `WorldOfKartcraft` folder beside itself, so the game's files stay together. The
//! launcher itself stays where the player put it, under whatever name. The settings, the kart
//! data and `version.txt` live in that same folder.
//!
//! macOS: the launcher is the executable of `World of Kartcraft.app`, and the game, the demo and
//! the converter sit beside it in `Contents/MacOS`. The bundle is the update's root (the manifest's
//! paths are relative to it). A bundle opened outside an Applications folder (unzipped in
//! Downloads) copies itself to `~/Applications/World of Kartcraft.app` and opens from there. The
//! settings, the kart data, `version.txt` and the game's own `benilla-config` live under
//! `~/Library/Application Support/World of Kartcraft`, never inside the bundle.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The game, the converter and the launcher, as the update host ships them, relative to
/// [`root`].
#[cfg(windows)]
pub const GAME: &str = "WorldOfKartcraft.exe";
#[cfg(windows)]
pub const CONVERTER: &str = "ctr-extract.exe";
#[cfg(windows)]
pub const LAUNCHER: &str = "Kartcraft Launcher.exe";
#[cfg(target_os = "macos")]
pub const GAME: &str = "Contents/MacOS/WorldOfKartcraft";
#[cfg(target_os = "macos")]
pub const CONVERTER: &str = "Contents/MacOS/ctr-extract";
#[cfg(target_os = "macos")]
pub const LAUNCHER: &str = "Contents/MacOS/Kartcraft Launcher";

/// `CREATE_NO_WINDOW`: a helper process without a console flashing up.
#[cfg(windows)]
const NO_WINDOW: u32 = 0x0800_0000;

/// The game's folder beside a launcher that is not in one.
#[cfg(windows)]
const GAME_FOLDER: &str = "WorldOfKartcraft";

/// The bundle's name, where it is installed.
#[cfg(target_os = "macos")]
const BUNDLE: &str = "World of Kartcraft.app";

/// The launcher's own file.
pub fn me() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

/// A helper process (the converter) with no console window of its own; nothing to do off Windows.
pub fn quiet(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(NO_WINDOW)
    }
    #[cfg(not(windows))]
    cmd
}

#[cfg(windows)]
fn is_game_folder(dir: &Path) -> bool {
    dir.join("kartcraft.cfg").is_file() || dir.join(GAME).is_file()
}

/// The folder the game lives in: on Windows the launcher's own or `WorldOfKartcraft` beside it,
/// on macOS the `.app` bundle (or, run bare from a build folder, that folder).
pub fn root() -> PathBuf {
    let here = me()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));
    #[cfg(windows)]
    {
        if is_game_folder(&here) {
            here
        } else {
            here.join(GAME_FOLDER)
        }
    }
    #[cfg(target_os = "macos")]
    {
        bundle().unwrap_or(here)
    }
}

/// The `.app` this launcher runs from: `<bundle>/Contents/MacOS/<launcher>`.
#[cfg(target_os = "macos")]
fn bundle() -> Option<PathBuf> {
    let app = me()?.ancestors().nth(3)?.to_path_buf();
    (app.extension().is_some_and(|e| e == "app")).then_some(app)
}

/// Where the player's settings, the kart data and `version.txt` live: the game's folder on
/// Windows, Application Support on macOS.
pub fn state() -> PathBuf {
    #[cfg(windows)]
    {
        root()
    }
    #[cfg(target_os = "macos")]
    {
        match std::env::var_os("HOME") {
            Some(home) => PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("World of Kartcraft"),
            None => root(),
        }
    }
}

/// The published presets, `defaults.cfg`: beside the launcher on Windows, in the bundle's
/// Resources on macOS.
pub fn defaults_file(root: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        root.join("defaults.cfg")
    }
    #[cfg(target_os = "macos")]
    {
        root.join("Contents").join("Resources").join("defaults.cfg")
    }
}

/// The first things a run does: the game's folder made, what a self-update left tidied up, and on
/// macOS the bundle moved into `~/Applications`. `Some(exit code)` when this run cannot go on.
pub fn first_steps() -> Option<i32> {
    if let Some(me) = me() {
        let _ = std::fs::remove_file(format!("{}.old", me.display()));
    }
    #[cfg(target_os = "macos")]
    if let Some(code) = install_bundle() {
        return Some(code);
    }
    for dir in [root(), state()] {
        if let Err(e) = std::fs::create_dir_all(&dir) {
            fatal(&format!("Could not make {}: {e}", dir.display()));
            return Some(1);
        }
    }
    None
}

/// The bundle into `~/Applications` the first time it is opened from anywhere else (the Downloads
/// folder, the Desktop), then opened from there: `Some(0)` when the copy took over. A bundle
/// already in an Applications folder, or one that could not be copied, is used where it is. The
/// quarantine flag Safari puts on a download comes off the copy (and the bundle used in place),
/// since Gatekeeper has had its say by the time this runs and would otherwise ask again after
/// every update changes the bundle.
#[cfg(target_os = "macos")]
fn install_bundle() -> Option<i32> {
    let app = bundle()?;
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let applications = [PathBuf::from("/Applications"), home.join("Applications")];
    let installed = app
        .parent()
        .is_some_and(|p| applications.iter().any(|a| a == p));
    if installed {
        unquarantine(&app);
        return None;
    }
    let target = home.join("Applications").join(BUNDLE);
    if !target.is_dir() {
        if std::fs::create_dir_all(home.join("Applications")).is_err() {
            return None;
        }
        // ditto keeps the bundle whole (signatures, permissions, links); a half copy goes.
        let ok = Command::new("/usr/bin/ditto")
            .arg(&app)
            .arg(&target)
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            let _ = std::fs::remove_dir_all(&target);
            return None;
        }
    }
    unquarantine(&target);
    let opened = Command::new("/usr/bin/open")
        .arg("-n")
        .arg(&target)
        .status()
        .is_ok_and(|s| s.success());
    opened.then_some(0)
}

/// Takes `com.apple.quarantine` off a bundle and everything in it; nothing to report if absent.
#[cfg(target_os = "macos")]
pub fn unquarantine(path: &Path) {
    let _ = Command::new("/usr/bin/xattr")
        .args(["-dr", "com.apple.quarantine"])
        .arg(path)
        .output();
}

/// Starts the launcher again, after a self-update: the bundle through LaunchServices on macOS,
/// the file itself on Windows.
pub fn restart() -> bool {
    let Some(me) = me() else {
        return false;
    };
    #[cfg(target_os = "macos")]
    if let Some(app) = bundle() {
        return Command::new("/usr/bin/open")
            .arg("-n")
            .arg(&app)
            .status()
            .is_ok_and(|s| s.success());
    }
    Command::new(&me).spawn().is_ok()
}

/// An error before the window can show it.
pub fn fatal(text: &str) {
    rfd::MessageDialog::new()
        .set_title("World of Kartcraft")
        .set_description(text)
        .set_level(rfd::MessageLevel::Error)
        .show();
}
