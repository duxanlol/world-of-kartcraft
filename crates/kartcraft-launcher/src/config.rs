//! The launcher's settings: the player's answers in `kartcraft.cfg` (`WOW_DIR`, `CTR_DISC`,
//! `SERVER`, `UPDATE_URL`, `ACCOUNT_SEEN`) in the state folder (`install::state`), the published
//! presets in `defaults.cfg` (`install::defaults_file`), and the presets built into the launcher,
//! last. On Windows both files sit beside the launcher, so a zip install carries on.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The presets the publish builds in (`.setup/publish.cfg`): the game server, the update host
/// and the account page.
pub const BUILT_SERVER: Option<&str> = option_env!("KARTCRAFT_SERVER");
pub const BUILT_UPDATE_URL: Option<&str> = option_env!("KARTCRAFT_UPDATE_URL");
pub const BUILT_REGISTER_URL: Option<&str> = option_env!("KARTCRAFT_REGISTER_URL");

/// Mac builds have a manifest and files of their own, in `macos/` beside the Windows ones, so a
/// Mac launcher never fetches a Windows file.
#[cfg(target_os = "macos")]
const PLATFORM_DIR: &str = "/macos";
#[cfg(windows)]
const PLATFORM_DIR: &str = "";

/// The settings file's line ending: Notepad's on Windows.
#[cfg(windows)]
const EOL: &str = "\r\n";
#[cfg(not(windows))]
const EOL: &str = "\n";

#[derive(Clone, Debug, Default)]
pub struct Config {
    pub wow_dir: Option<PathBuf>,
    pub ctr_disc: Option<PathBuf>,
    pub server: Option<String>,
    pub update_url: Option<String>,
    pub register_url: Option<String>,
    /// The player's own `UPDATE_URL` and `SERVER`, kept when written back.
    own_server: Option<String>,
    own_update_url: Option<String>,
    pub account_seen: bool,
}

fn read(path: &Path) -> BTreeMap<String, String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let (k, v) = l.split_once('=')?;
            Some((k.trim().to_string(), v.trim().trim_matches('"').to_string()))
        })
        .collect()
}

impl Config {
    /// `root` holds the published presets, `state` the player's own file.
    pub fn load(root: &Path, state: &Path) -> Config {
        let defaults = read(&crate::install::defaults_file(root));
        let own = read(&state.join("kartcraft.cfg"));
        let get = |key: &str, built: Option<&str>| {
            own.get(key)
                .or_else(|| defaults.get(key))
                .cloned()
                .or_else(|| built.map(str::to_string))
                .filter(|v| !v.is_empty())
        };
        Config {
            wow_dir: own
                .get("WOW_DIR")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
            ctr_disc: own
                .get("CTR_DISC")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
            server: get("SERVER", BUILT_SERVER),
            update_url: get("UPDATE_URL", BUILT_UPDATE_URL),
            register_url: get("REGISTER_URL", BUILT_REGISTER_URL),
            own_server: own.get("SERVER").cloned(),
            own_update_url: own.get("UPDATE_URL").cloned(),
            account_seen: own.contains_key("ACCOUNT_SEEN"),
        }
    }

    /// Where this platform's updates come from: the setting, else the zip launcher's default,
    /// the server's port 8080; plus the platform's folder.
    pub fn updates(&self) -> Option<String> {
        self.update_url
            .clone()
            .or_else(|| self.server.as_ref().map(|s| format!("http://{s}:8080")))
            .map(|u| format!("{}{PLATFORM_DIR}", u.trim_end_matches('/')))
    }

    pub fn save(&self, state: &Path) {
        let mut out = String::new();
        if let Some(w) = &self.wow_dir {
            out += &format!("WOW_DIR={}{EOL}", w.display());
        }
        if let Some(s) = &self.own_server {
            out += &format!("SERVER={s}{EOL}");
        }
        if let Some(c) = &self.ctr_disc {
            out += &format!("CTR_DISC={}{EOL}", c.display());
        }
        if let Some(u) = &self.own_update_url {
            out += &format!("UPDATE_URL={u}{EOL}");
        }
        if self.account_seen {
            out += &format!("ACCOUNT_SEEN=1{EOL}");
        }
        let _ = std::fs::create_dir_all(state);
        let _ = std::fs::write(state.join("kartcraft.cfg"), out);
    }
}

/// A WoW 1.12.1 folder: the one with `Data/dbc.MPQ` (a `Data` folder picked by mistake counts as
/// its parent).
pub fn wow_folder(picked: &Path) -> Option<PathBuf> {
    let dir = if picked
        .file_name()
        .is_some_and(|n| n.eq_ignore_ascii_case("Data"))
    {
        picked.parent()?.to_path_buf()
    } else {
        picked.to_path_buf()
    };
    dir.join("Data").join("dbc.MPQ").is_file().then_some(dir)
}

/// The disc's track the converter reads: a `.cue`'s `.bin` beside it (named in its `FILE` line,
/// else by its own name), a `.bin` or `.iso` as it is.
pub fn disc_image(picked: &Path) -> Option<PathBuf> {
    let ext = picked.extension()?.to_string_lossy().to_ascii_lowercase();
    match ext.as_str() {
        "bin" | "iso" | "img" => picked.is_file().then(|| picked.to_path_buf()),
        "cue" => {
            let text = std::fs::read_to_string(picked).ok()?;
            let named = text.lines().find_map(|l| {
                let l = l.trim();
                let rest = l.strip_prefix("FILE ")?;
                let name = rest.split('"').nth(1)?;
                Some(picked.with_file_name(name))
            });
            named
                .filter(|p| p.is_file())
                .or_else(|| Some(picked.with_extension("bin")).filter(|p| p.is_file()))
        }
        _ => None,
    }
}
