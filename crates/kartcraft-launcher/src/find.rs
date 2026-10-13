//! Looking for the player's games where people keep them, so most never have to browse: the
//! launcher's own folder and its neighbours, the usual game folders, Desktop, Downloads and
//! Documents, and two levels down every drive's root.

use std::path::{Path, PathBuf};

/// A CTR image is a whole disc: the US one is about 500 MB, so a smaller `.bin` is something else.
const DISC_MIN_BYTES: u64 = 300_000_000;

fn user_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from);
    let mut dirs = Vec::new();
    if let Some(h) = &home {
        for d in [
            "Desktop",
            "Downloads",
            "Documents",
            "Games",
            "OneDrive\\Desktop",
            "OneDrive\\Documents",
        ] {
            dirs.push(h.join(d));
        }
    }
    dirs
}

fn drives() -> Vec<PathBuf> {
    (b'C'..=b'Z')
        .map(|d| PathBuf::from(format!("{}:\\", d as char)))
        .filter(|p| p.is_dir())
        .collect()
}

fn children(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|r| {
            r.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default()
}

/// The places worth a look, nearest first: `root` and its neighbours, the user's folders, each
/// drive's root and the folders one and two levels under it.
fn places(root: &Path) -> Vec<PathBuf> {
    let mut out = vec![root.to_path_buf()];
    if let Some(p) = root.parent() {
        out.push(p.to_path_buf());
    }
    out.extend(children(root));
    for d in user_dirs() {
        out.extend(children(&d));
        out.push(d);
    }
    for drive in drives() {
        for program in ["Program Files", "Program Files (x86)"] {
            out.extend(children(&drive.join(program)));
        }
        for child in children(&drive) {
            let name = child
                .file_name()
                .map(|n| n.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            if name == "windows" || name.starts_with('$') || name == "programdata" {
                continue;
            }
            out.extend(children(&child));
            out.push(child);
        }
    }
    out
}

/// A WoW 1.12.1 folder somewhere usual.
pub fn wow(root: &Path) -> Option<PathBuf> {
    places(root)
        .into_iter()
        .find_map(|p| crate::config::wow_folder(&p))
}

/// A Crash Team Racing disc image somewhere usual: a disc-sized `.bin`, `.iso` or `.img`, the one
/// whose name says Crash Team Racing first.
pub fn ctr(root: &Path) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    for dir in places(root) {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            let ext = p
                .extension()
                .map(|x| x.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            if matches!(ext.as_str(), "bin" | "iso" | "img")
                && e.metadata().is_ok_and(|m| m.len() > DISC_MIN_BYTES)
                && !found.contains(&p)
            {
                found.push(p);
            }
        }
    }
    let named = |p: &PathBuf| {
        let n = p
            .file_name()
            .map(|n| n.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        n.contains("crash team racing") || n.contains("ctr") || n.contains("scus-94426")
    };
    found
        .iter()
        .find(|p| named(p))
        .or_else(|| found.first())
        .cloned()
}
