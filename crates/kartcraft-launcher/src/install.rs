//! Where the launcher keeps the game: portable, nothing installed. In a folder that already holds
//! the game (the zip's, with its `kartcraft.cfg` or `WorldOfKartcraft.exe`) it works there; anywhere
//! else (Downloads, the Desktop) in a `WorldOfKartcraft` folder beside itself, so the game's files
//! stay together. The launcher itself stays where the player put it, under whatever name.

use std::path::{Path, PathBuf};

/// `CREATE_NO_WINDOW`: a helper process without a console flashing up.
pub const NO_WINDOW: u32 = 0x0800_0000;

/// The game's folder beside a launcher that is not in one.
const GAME_FOLDER: &str = "WorldOfKartcraft";

/// The launcher's own file.
pub fn me() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

fn is_game_folder(dir: &Path) -> bool {
    dir.join("kartcraft.cfg").is_file() || dir.join("WorldOfKartcraft.exe").is_file()
}

/// The folder the game lives in.
pub fn root() -> PathBuf {
    let here = me()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));
    if is_game_folder(&here) {
        here
    } else {
        here.join(GAME_FOLDER)
    }
}

/// The first things a run does: the game's folder made, and what a self-update left tidied up.
/// `Some(exit code)` when this run cannot go on.
pub fn first_steps() -> Option<i32> {
    if let Some(me) = me() {
        let _ = std::fs::remove_file(format!("{}.old", me.display()));
    }
    let root = root();
    if let Err(e) = std::fs::create_dir_all(&root) {
        fatal(&format!("Could not make {}: {e}", root.display()));
        return Some(1);
    }
    None
}

/// An error before the window can show it.
pub fn fatal(text: &str) {
    rfd::MessageDialog::new()
        .set_title("World of Kartcraft")
        .set_description(text)
        .set_level(rfd::MessageLevel::Error)
        .show();
}
