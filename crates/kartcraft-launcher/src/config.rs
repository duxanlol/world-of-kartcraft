//! The launcher's settings: the player's answers in `kartcraft.cfg` (`WOW_DIR`, `CTR_DISC`,
//! `SERVER`, `UPDATE_URL`, `ACCOUNT_SEEN`) in the state folder (`install::state`), the published
//! presets in `defaults.cfg` (`install::defaults_file`), and the presets built into the launcher,
//! last. On Windows both files sit beside the launcher, so a zip install carries on. On Linux the
//! answers are `play-proton.sh`'s `kartcraft-linux.cfg`, with the Proton picked (`PROTON`), so
//! the two Linux launchers share them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The presets the publish builds in (`.setup/publish.cfg`): the game server, the update host
/// and the account page.
pub const BUILT_SERVER: Option<&str> = option_env!("KARTCRAFT_SERVER");
pub const BUILT_UPDATE_URL: Option<&str> = option_env!("KARTCRAFT_UPDATE_URL");
pub const BUILT_REGISTER_URL: Option<&str> = option_env!("KARTCRAFT_REGISTER_URL");
/// The release channel the launcher was built for (`KARTCRAFT_CHANNEL`): `staging` installs apart
/// from the players' game, so a tester's staging launcher never picks up a production install's
/// settings beside it ([`crate::install`]).
pub const BUILT_CHANNEL: Option<&str> = option_env!("KARTCRAFT_CHANNEL");

/// Whether this is the testers' launcher.
pub fn staging() -> bool {
    BUILT_CHANNEL == Some("staging")
}

/// The game's name as the launcher shows it and installs it: the testers' says so.
pub fn title() -> &'static str {
    if staging() {
        "World of Kartcraft Staging"
    } else {
        "World of Kartcraft"
    }
}

/// Mac builds have a manifest and files of their own, in `macos/` beside the Windows ones, so a
/// Mac launcher never fetches a Windows file.
#[cfg(target_os = "macos")]
const PLATFORM_DIR: &str = "/macos";
#[cfg(not(target_os = "macos"))]
const PLATFORM_DIR: &str = "";
/// A Linux launcher runs the Windows game, so the game comes from the Windows manifest; the
/// AppImage itself has a manifest of its own in `linux/`, which the Linux release writes.
#[cfg(target_os = "linux")]
const LAUNCHER_DIR: &str = "/linux";

/// The player's answers: `play-proton.sh`'s on Linux, `kartcraft.cfg` elsewhere.
#[cfg(target_os = "linux")]
pub const CFG_FILE: &str = "kartcraft-linux.cfg";
#[cfg(not(target_os = "linux"))]
pub const CFG_FILE: &str = "kartcraft.cfg";

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
    /// The `proton` script the Windows game runs through (Linux).
    pub proton: Option<PathBuf>,
}

pub(crate) fn read(path: &Path) -> BTreeMap<String, String> {
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
        let own = read(&state.join(CFG_FILE));
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
            proton: own
                .get("PROTON")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
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

    /// Where the launcher's own update comes from, when not with the game's ([`Config::updates`]).
    #[cfg(target_os = "linux")]
    pub fn launcher_updates(&self) -> Option<String> {
        self.updates().map(|u| format!("{u}{LAUNCHER_DIR}"))
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
        if let Some(p) = &self.proton {
            out += &format!("PROTON={}{EOL}", p.display());
        }
        let _ = std::fs::create_dir_all(state);
        let _ = std::fs::write(state.join(CFG_FILE), out);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_answers_round_trip_with_the_proton_picked() {
        let dir = std::env::temp_dir().join(format!("kartcraft-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let config = Config {
            wow_dir: Some(PathBuf::from("/games/WoW 1.12")),
            ctr_disc: Some(PathBuf::from("/games/ctr.bin")),
            account_seen: true,
            proton: Some(PathBuf::from("/steam/GE-Proton10-34/proton")),
            ..Default::default()
        };
        config.save(&dir);
        let text = std::fs::read_to_string(dir.join(CFG_FILE)).unwrap();
        let back = Config::load(&dir, &dir);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(back.wow_dir, config.wow_dir);
        assert_eq!(back.ctr_disc, config.ctr_disc);
        assert_eq!(back.proton, config.proton);
        assert!(back.account_seen);
        // One line per answer, the platform's own line ending.
        assert!(text.ends_with(EOL));
    }
}
