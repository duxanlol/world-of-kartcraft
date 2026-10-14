//! Starting the disc converter and the game: on Windows and macOS the programs themselves, on
//! Linux the Windows ones through the Proton the player picked ([`crate::proton`]), with the
//! paths on Wine's `Z:` drive. Both work in the state folder (`install::state`), where the kart
//! data and the game's own `benilla-config` live.

use std::path::Path;
use std::process::Command;

use crate::config::Config;

/// The `proton` script to run the game with: the one picked, if it is still there, else the
/// newest installed.
#[cfg(target_os = "linux")]
pub fn proton(config: &Config) -> Option<std::path::PathBuf> {
    config
        .proton
        .clone()
        .filter(|p| p.is_file())
        .or_else(|| crate::proton::installed().first().map(|p| p.script.clone()))
}

/// Why the game cannot be started yet on this platform, if it cannot.
pub fn blocked(config: &Config) -> Option<String> {
    #[cfg(target_os = "linux")]
    if proton(config).is_none() {
        return Some(
            "No Proton found: install one through Steam (or GE-Proton), or pick its proton script."
                .into(),
        );
    }
    let _ = config;
    None
}

/// A path as the program will read it.
fn arg(p: &Path) -> std::ffi::OsString {
    #[cfg(target_os = "linux")]
    return crate::proton::wine_path(p).into();
    #[cfg(not(target_os = "linux"))]
    p.as_os_str().to_owned()
}

/// The program `exe`, ready for its arguments, working in `state`.
fn program(state: &Path, config: &Config, exe: &Path) -> Result<Command, String> {
    #[cfg(target_os = "linux")]
    {
        let proton = proton(config).ok_or("No Proton to run the game with.")?;
        Ok(crate::proton::command(&proton, state, exe))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = config;
        let mut cmd = Command::new(exe);
        cmd.current_dir(state);
        Ok(cmd)
    }
}

/// The converter, `disc` into the state folder's `kart-data`, with no console window.
pub fn converter(
    state: &Path,
    config: &Config,
    exe: &Path,
    disc: &Path,
) -> Result<Command, String> {
    let mut cmd = program(state, config, exe)?;
    cmd.arg(arg(disc))
        .arg(arg(&state.join("kart-data")))
        .arg("--all");
    crate::install::quiet(&mut cmd);
    Ok(cmd)
}

/// The game, on the WoW install `wow` and the kart data the disc made. Its own state folder goes
/// with the kart data, not beside the binary: on macOS that would be inside the bundle.
pub fn game(state: &Path, config: &Config, exe: &Path, wow: &Path) -> Result<Command, String> {
    let mut cmd = program(state, config, exe)?;
    cmd.env("WOW_DATA", arg(&wow.join("Data")))
        .env("WOW_KART_DATA", arg(&state.join("kart-data")))
        .env("BENILLA_HOME", arg(&state.join("benilla-config")));
    if let Some(server) = &config.server {
        cmd.env("WOW_HOST", server);
    }
    Ok(cmd)
}
