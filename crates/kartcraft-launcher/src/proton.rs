//! Linux: the Windows game runs through the player's own Proton, as `play-proton.sh` runs it.
//! The Protons are the ones Steam has (`steamapps/common/Proton*`) and the custom builds
//! (`compatibilitytools.d`, GE-Proton among them), newest first by their version's numbers, not
//! by name: by name GE-Proton9-20 sorts after GE-Proton10-34. The game's Wine prefix is
//! `.proton-prefix` in its folder, and its `version` file names the Proton that last ran it: an
//! older Proton downgrades the prefix, which can hang its first start.

use std::path::{Path, PathBuf};

/// One Proton: its name (`GE-Proton10-34`, `Proton 9.0`) and its `proton` script.
#[derive(Clone, Debug, PartialEq)]
pub struct Proton {
    pub name: String,
    pub script: PathBuf,
}

/// Steam's folders, where they exist: the native install, its `~/.steam` links, and Flatpak's.
pub fn steam_dirs() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = Vec::new();
    for d in [
        ".local/share/Steam",
        ".steam/steam",
        ".steam/root",
        ".var/app/com.valvesoftware.Steam/data/Steam",
    ] {
        let dir = home.join(d);
        let real = std::fs::canonicalize(&dir).unwrap_or(dir);
        if real.is_dir() && !out.contains(&real) {
            out.push(real);
        }
    }
    out
}

/// The Proton named by a folder holding a `proton` script: the `version` file's name when it has
/// one (`1702000000 GE-Proton10-34`), else the folder's.
pub fn at(dir: &Path) -> Option<Proton> {
    let script = dir.join("proton");
    if !script.is_file() {
        return None;
    }
    let from_file = std::fs::read_to_string(dir.join("version"))
        .ok()
        .and_then(|v| v.split_whitespace().nth(1).map(str::to_string));
    let name = from_file.or_else(|| Some(dir.file_name()?.to_string_lossy().into_owned()))?;
    Some(Proton { name, script })
}

/// Every Proton installed, newest first.
pub fn installed() -> Vec<Proton> {
    let mut found: Vec<Proton> = Vec::new();
    for steam in steam_dirs() {
        let mut dirs = Vec::new();
        for sub in ["steamapps/common", "compatibilitytools.d"] {
            if let Ok(entries) = std::fs::read_dir(steam.join(sub)) {
                dirs.extend(entries.flatten().map(|e| e.path()));
            }
        }
        for dir in dirs {
            let is_proton = dir
                .file_name()
                .is_some_and(|n| n.to_string_lossy().to_ascii_lowercase().contains("proton"));
            if let Some(p) = at(&dir).filter(|_| is_proton) {
                if !found.iter().any(|f| f.script == p.script) {
                    found.push(p);
                }
            }
        }
    }
    found.sort_by_key(|p| std::cmp::Reverse(version_key(&p.name)));
    found
}

/// A Proton's version for ordering: its family (the letters before the first digit, lowercased,
/// so GE builds sort with GE builds) and then its numbers in order.
pub fn version_key(name: &str) -> (String, Vec<u32>) {
    let family: String = name
        .chars()
        .take_while(|c| !c.is_ascii_digit())
        .collect::<String>()
        .trim()
        .to_ascii_lowercase();
    let numbers = name
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok())
        .collect();
    (family, numbers)
}

/// The game's Wine prefix.
pub fn prefix(root: &Path) -> PathBuf {
    root.join(".proton-prefix")
}

/// The Proton that last ran the prefix, by the name its `version` file keeps.
pub fn prefix_made_by(root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(prefix(root).join("version")).ok()?;
    let name = text.split_whitespace().last()?.to_string();
    (!name.is_empty()).then_some(name)
}

/// The warning for running `chosen` on a prefix a newer Proton of its family made.
pub fn downgrade(made_by: &str, chosen: &str) -> Option<String> {
    let (made_family, made) = version_key(made_by);
    let (chosen_family, picked) = version_key(chosen);
    (made_family == chosen_family && !made.is_empty() && picked < made).then(|| {
        format!(
            "The game was last run with {made_by}; {chosen} is older and can hang while it \
             converts the game's Wine prefix back. Pick {made_by} or newer."
        )
    })
}

/// A Linux path as the Windows game reads it: Wine's `Z:` drive is `/`.
pub fn wine_path(p: &Path) -> String {
    let abs = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    format!("Z:{}", abs.display().to_string().replace('/', "\\"))
}

/// Wine's WGI hands a DS4 (054c:09cc) read raw its axes as RY RX LY LX R2 L2, not the order SDL's
/// Windows mapping expects (measured under GE-Proton 10); `play-proton.sh` sets the same line.
/// Only for a player who keeps Proton's raw HID path ([`pad_env`]).
pub const PAD_MAPPINGS: &str = "030000004c050000cc09000000000000,PS4 Controller,a:b1,b:b2,back:b8,\
dpdown:h0.4,dpleft:h0.8,dpright:h0.2,dpup:h0.1,guide:b12,leftshoulder:b4,leftstick:b10,\
lefttrigger:a5,leftx:a3,lefty:a2,rightshoulder:b5,rightstick:b11,righttrigger:a4,rightx:a1,\
righty:a0,start:b9,touchpad:b13,x:b0,y:b3,platform:Windows,";

/// The pads' environment for the game, from the player's `PROTON_PREFER_SDL` and
/// `SDL_GAMECONTROLLERCONFIG`. By default Proton reads every pad through SDL
/// (`PROTON_PREFER_SDL=1`, which turns off its raw HID path): a DS4, a DualSense, an Xbox or a
/// generic pad reaches the game as a standard gamepad, with nothing to map (measured with a DS4 under
/// GE-Proton 10: sticks, triggers and buttons all where they belong). A player who sets
/// `PROTON_PREFER_SDL=0` keeps the raw path, and the DS4 line ([`PAD_MAPPINGS`]) after their own.
pub fn pad_env(prefer_sdl: Option<&str>, own: Option<&str>) -> Vec<(&'static str, String)> {
    if prefer_sdl.is_none_or(|v| v.trim() != "0") {
        return vec![("PROTON_PREFER_SDL", "1".into())];
    }
    let pads = match own {
        Some(own) if !own.trim().is_empty() => format!("{own}\n{PAD_MAPPINGS}"),
        _ => PAD_MAPPINGS.to_string(),
    };
    vec![("SDL_GAMECONTROLLERCONFIG", pads)]
}

/// `proton run <exe> <args>` with the game's prefix, Steam's folder, the pads' environment and
/// the graphics backend.
pub fn command(proton: &Path, root: &Path, exe: &Path) -> std::process::Command {
    let prefix = prefix(root);
    let _ = std::fs::create_dir_all(&prefix);
    let steam = steam_dirs()
        .into_iter()
        .next()
        .unwrap_or_else(|| root.to_path_buf());
    let mut cmd = std::process::Command::new(proton);
    cmd.arg("run")
        .arg(exe)
        .current_dir(root)
        .env("STEAM_COMPAT_DATA_PATH", &prefix)
        .env("STEAM_COMPAT_CLIENT_INSTALL_PATH", steam);
    let prefer = std::env::var("PROTON_PREFER_SDL").ok();
    let own = std::env::var("SDL_GAMECONTROLLERCONFIG").ok();
    cmd.envs(pad_env(prefer.as_deref(), own.as_deref()));
    // Vulkan, which Proton passes straight through, unless the player picks one: left to itself
    // wgpu may take DX12 (a Steam Deck's RADV does), whose vkd3d-proton shader compiler fails on
    // the game's compute shaders and crashes it seconds in.
    if std::env::var_os("WGPU_BACKEND").is_none() {
        cmd.env("WGPU_BACKEND", "vulkan");
    }
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protons_order_by_their_numbers_not_their_names() {
        let mut names = vec![
            "GE-Proton9-20",
            "GE-Proton10-34",
            "GE-Proton10-4",
            "GE-Proton8-32",
        ];
        names.sort_by_key(|n| std::cmp::Reverse(version_key(n)));
        assert_eq!(
            names,
            [
                "GE-Proton10-34",
                "GE-Proton10-4",
                "GE-Proton9-20",
                "GE-Proton8-32"
            ]
        );
        assert_eq!(version_key("Proton 9.0").1, [9, 0]);
    }

    #[test]
    fn an_older_proton_of_the_prefix_s_family_is_a_downgrade() {
        assert!(downgrade("GE-Proton10-34", "GE-Proton9-20").is_some());
        assert!(downgrade("GE-Proton10-34", "GE-Proton10-34").is_none());
        assert!(downgrade("GE-Proton9-20", "GE-Proton10-34").is_none());
        // Another family is no ordering: Valve's and GE's numbers do not compare.
        assert!(downgrade("GE-Proton10-34", "Proton 9.0").is_none());
    }

    #[test]
    fn a_folder_with_a_proton_script_is_named_by_its_version_file() {
        let dir = std::env::temp_dir().join(format!("kartcraft-proton-{}", std::process::id()));
        let tool = dir.join("GE-Proton10-34");
        std::fs::create_dir_all(&tool).unwrap();
        assert_eq!(at(&tool), None, "no script, no Proton");
        std::fs::write(tool.join("proton"), "#!/bin/sh\n").unwrap();
        assert_eq!(at(&tool).unwrap().name, "GE-Proton10-34");
        std::fs::write(tool.join("version"), "1760000000 GE-Proton10-35\n").unwrap();
        assert_eq!(at(&tool).unwrap().name, "GE-Proton10-35");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pads_go_through_sdl_unless_the_player_keeps_the_raw_path() {
        let sdl = vec![("PROTON_PREFER_SDL", "1".to_string())];
        assert_eq!(pad_env(None, None), sdl);
        assert_eq!(
            pad_env(Some("1"), Some("own")),
            sdl,
            "their own mapping rides along by inheritance"
        );
        // The raw path: the DS4 line, after the player's own.
        assert_eq!(
            pad_env(Some("0"), None),
            vec![("SDL_GAMECONTROLLERCONFIG", PAD_MAPPINGS.to_string())]
        );
        assert_eq!(
            pad_env(Some("0"), Some("mine")),
            vec![("SDL_GAMECONTROLLERCONFIG", format!("mine\n{PAD_MAPPINGS}"))]
        );
    }

    #[test]
    fn a_path_is_on_wine_s_z_drive() {
        assert_eq!(
            wine_path(Path::new("/nonexistent/WoW 1.12/Data")),
            "Z:\\nonexistent\\WoW 1.12\\Data"
        );
    }
}
