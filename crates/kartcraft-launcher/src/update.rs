//! Updates from the update host, as the zip's `update.ps1` does: its `manifest.json` names every
//! file of the current client with its SHA-256, and only the files that differ here are
//! downloaded (gzip-compressed, by hash) and checked before they replace the old. The launcher
//! itself is one of them: a new one goes in beside the running one, which then gives way to it.

use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// The launcher's name in the manifest; here it is whatever file the player runs.
pub const LAUNCHER: &str = "Kartcraft Launcher.exe";

/// Where a file of the manifest lives: the launcher is the running file, the rest are in `root`.
fn local(root: &Path, path: &str) -> PathBuf {
    if path == LAUNCHER {
        if let Some(me) = crate::install::me() {
            return me;
        }
    }
    root.join(path)
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub path: String,
    pub sha256: String,
    pub gz: String,
    pub gz_size: u64,
}

#[derive(Clone, Debug)]
pub struct Manifest {
    pub version: String,
    /// The disc converter's hash: kart data from another converter is converted again.
    pub converter: Option<String>,
    pub files: Vec<Entry>,
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(8))
        .timeout_read(std::time::Duration::from_secs(60))
        .build()
}

pub fn manifest(base: &str) -> Result<Manifest, String> {
    let text = agent()
        .get(&format!("{base}/manifest.json"))
        .call()
        .map_err(|e| e.to_string())?
        .into_string()
        .map_err(|e| e.to_string())?;
    let doc: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let files = doc["files"]
        .as_array()
        .ok_or("the manifest lists no files")?
        .iter()
        .filter_map(|f| {
            Some(Entry {
                path: f["path"].as_str()?.to_string(),
                sha256: f["sha256"].as_str()?.to_string(),
                gz: f["gz"].as_str()?.to_string(),
                gz_size: f["gz_size"].as_u64().unwrap_or(0),
            })
        })
        .collect();
    Ok(Manifest {
        version: doc["version"].as_str().unwrap_or("").to_string(),
        converter: doc["converter"].as_str().map(str::to_string),
        files,
    })
}

pub fn sha256(path: &Path) -> Option<String> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Some(format!("{:x}", h.finalize()))
}

/// The files that differ from the manifest's here.
pub fn stale(root: &Path, m: &Manifest) -> Vec<Entry> {
    m.files
        .iter()
        .filter(|f| sha256(&local(root, &f.path)).as_deref() != Some(f.sha256.as_str()))
        .cloned()
        .collect()
}

/// Downloads `files`, each checked against its hash before it replaces the old, reporting the
/// compressed bytes done. The launcher is written as `<launcher>.new`, for [`swap_in`].
pub fn download(
    base: &str,
    root: &Path,
    files: &[Entry],
    mut progress: impl FnMut(u64, &str),
) -> Result<(), String> {
    let agent = agent();
    let mut done = 0u64;
    for f in files {
        progress(done, &f.path);
        let target = local(root, &f.path);
        if let Some(dir) = target.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let part = PathBuf::from(format!("{}.download", target.display()));
        let response = agent
            .get(&format!("{base}/{}", f.gz))
            .call()
            .map_err(|e| format!("{}: {e}", f.path))?;
        let mut counted = Counting {
            inner: response.into_reader(),
            done: &mut done,
            report: &mut progress,
            name: &f.path,
        };
        let mut gz = flate2::read::GzDecoder::new(&mut counted);
        let mut out = std::fs::File::create(&part).map_err(|e| e.to_string())?;
        std::io::copy(&mut gz, &mut out).map_err(|e| format!("{}: {e}", f.path))?;
        drop(out);
        if sha256(&part).as_deref() != Some(f.sha256.as_str()) {
            let _ = std::fs::remove_file(&part);
            return Err(format!("the download of {} is damaged; try again", f.path));
        }
        let dest = if f.path == LAUNCHER {
            PathBuf::from(format!("{}.new", target.display()))
        } else {
            target
        };
        std::fs::rename(&part, &dest).map_err(|e| format!("{}: {e}", f.path))?;
    }
    progress(done, "");
    Ok(())
}

/// A reader that reports how many bytes went through it.
struct Counting<'a, R, F: FnMut(u64, &str)> {
    inner: R,
    done: &'a mut u64,
    report: &'a mut F,
    name: &'a str,
}

impl<R: Read, F: FnMut(u64, &str)> Read for Counting<'_, R, F> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        *self.done += n as u64;
        (self.report)(*self.done, self.name);
        Ok(n)
    }
}

/// A new launcher downloaded beside this one: the running one steps aside (Windows lets a running
/// file be renamed, not replaced), the new one takes its name and is started. `true` when it was.
pub fn swap_in() -> bool {
    let Some(me) = crate::install::me() else {
        return false;
    };
    let new = PathBuf::from(format!("{}.new", me.display()));
    if !new.is_file() {
        return false;
    }
    let old = PathBuf::from(format!("{}.old", me.display()));
    let _ = std::fs::remove_file(&old);
    if std::fs::rename(&me, &old).is_err() || std::fs::rename(&new, &me).is_err() {
        let _ = std::fs::rename(&old, &me);
        return false;
    }
    std::process::Command::new(&me).spawn().is_ok()
}

/// The kart data was made by another converter than the manifest's.
pub fn converter_changed(root: &Path, m: &Manifest) -> bool {
    let stamp =
        std::fs::read_to_string(root.join("kart-data").join(".converter")).unwrap_or_default();
    m.converter
        .as_deref()
        .is_some_and(|c| root.join("kart-data").join("kart.json").is_file() && stamp.trim() != c)
}

pub fn write_version(root: &Path, m: &Manifest) {
    let _ = std::fs::write(root.join("version.txt"), &m.version);
}
