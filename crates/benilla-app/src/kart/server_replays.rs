//! Replays kept on the server (`Kartcraft.cpp`, "Replays"): it records every race and time trial
//! from what it already sees (each racer's place and facing, their kart's state line) and keeps
//! the files at `Kartcraft.ReplayUrl`, told at login (`KARTCRAFT\tR\tRUrl\t<url>`, empty none).
//! The Replays window lists ours and the courses' records (the layer's, off the server's lines)
//! and plays one as a `KartReplay` token `s<key>`: fetched over HTTP on a thread, kept under
//! `kart-replays/server/`, and read into a [`Replay`] as the replays recorded here are.

use std::io::Read;
use std::sync::Mutex;

use benilla_assets::coords::wow_to_bevy;
use benilla_protocol::SessionEvent;
use bevy::prelude::*;

use super::ghost::Pose;
use super::replay::{Replay, Track};

/// A server replay's token prefix in `KartReplay`.
pub(crate) const TOKEN: char = 's';
/// Server replays kept on this computer, the newest by fetch.
const KEEP: usize = 30;
/// The longest a fetch may take (s).
const TIMEOUT: u64 = 30;
/// The largest file taken.
const MAX_BYTES: u64 = 8 << 20;

static URL: Mutex<String> = Mutex::new(String::new());

/// Where the server keeps its replays, as it said at login; empty none.
pub(crate) fn url() -> String {
    URL.lock().map(|u| u.clone()).unwrap_or_default()
}

pub(crate) fn on_chat(In(ev): In<SessionEvent>) {
    let SessionEvent::Chat(m) = ev else {
        return;
    };
    if !m.is_addon() {
        return;
    }
    if let Some(url) = m.text.strip_prefix("KARTCRAFT\tR\tRUrl\t") {
        if let Ok(mut u) = URL.lock() {
            *u = url.trim().trim_end_matches('/').to_string();
        }
    }
}

/// A replay key as the server makes them: the race's start (unix time), a dash, its id.
pub(crate) fn key_ok(key: &str) -> bool {
    key.split_once('-').is_some_and(|(a, b)| {
        !a.is_empty()
            && !b.is_empty()
            && key.len() <= 24
            && a.chars().chain(b.chars()).all(|c| c.is_ascii_digit())
    })
}

fn cache_dir() -> Option<std::path::PathBuf> {
    Some(crate::local_state::kart_replays_dir()?.join("server"))
}

fn cached(key: &str) -> Option<std::path::PathBuf> {
    Some(cache_dir()?.join(format!("{key}.krps")))
}

/// A fetch under way.
pub(crate) struct Fetch {
    key: String,
    done: crossbeam_channel::Receiver<Result<Vec<u8>, String>>,
}

/// What a server token gives this frame.
pub(crate) enum Load {
    Ready(Replay),
    Waiting,
    Missing,
}

/// Server replay `key` for `us` (our name): read from this computer, or fetched, `fetch` holding
/// the fetch until it is in.
pub(crate) fn load(key: &str, us: &str, fetch: &mut Option<Fetch>) -> Load {
    if !key_ok(key) {
        return Load::Missing;
    }
    if let Some(bytes) = cached(key).and_then(|p| std::fs::read(p).ok()) {
        return match decode(&bytes, us) {
            Some(r) => Load::Ready(r),
            None => {
                warn!("kart replay: server replay {key} is not a replay file");
                if let Some(p) = cached(key) {
                    let _ = std::fs::remove_file(p);
                }
                Load::Missing
            }
        };
    }
    match fetch.as_ref().filter(|f| f.key == key) {
        None => {
            let base = url();
            if base.is_empty() {
                warn!("kart replay: the server keeps no replays");
                return Load::Missing;
            }
            let (tx, rx) = crossbeam_channel::bounded(1);
            let at = format!("{base}/{key}.krps");
            info!("kart replay: fetching {at}");
            std::thread::spawn(move || {
                let _ = tx.send(get(&at));
            });
            *fetch = Some(Fetch {
                key: key.to_string(),
                done: rx,
            });
            Load::Waiting
        }
        Some(f) => match f.done.try_recv() {
            Err(crossbeam_channel::TryRecvError::Empty) => Load::Waiting,
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                *fetch = None;
                Load::Missing
            }
            Ok(Err(e)) => {
                warn!("kart replay: fetching {key}: {e}");
                *fetch = None;
                Load::Missing
            }
            Ok(Ok(bytes)) => {
                *fetch = None;
                if let (Some(dir), Some(path)) = (cache_dir(), cached(key)) {
                    let _ = std::fs::create_dir_all(&dir);
                    if let Err(e) = crate::local_state::write_atomic_bytes(&path, &bytes) {
                        warn!("kart replay: {}: {e}", path.display());
                    }
                    prune(&dir);
                }
                match decode(&bytes, us) {
                    Some(r) => Load::Ready(r),
                    None => {
                        warn!("kart replay: server replay {key} is not a replay file");
                        Load::Missing
                    }
                }
            }
        },
    }
}

fn get(url: &str) -> Result<Vec<u8>, String> {
    let response = ureq::get(url)
        .timeout(std::time::Duration::from_secs(TIMEOUT))
        .call()
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(MAX_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    Ok(bytes)
}

/// The fetched replays past [`KEEP`], the oldest fetched first, gone.
fn prune(dir: &std::path::Path) {
    let mut files: Vec<(std::time::SystemTime, std::path::PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "krps"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    for (_, old) in files.into_iter().skip(KEEP) {
        let _ = std::fs::remove_file(old);
    }
}

struct Bytes<'a>(&'a [u8]);

impl Bytes<'_> {
    fn u8(&mut self) -> Option<u8> {
        let (&b, rest) = self.0.split_first()?;
        self.0 = rest;
        Some(b)
    }

    fn u16(&mut self) -> Option<u16> {
        Some(u16::from(self.u8()?) | u16::from(self.u8()?) << 8)
    }

    fn u32(&mut self) -> Option<u32> {
        let b = self.0.get(..4)?;
        let v = u32::from_le_bytes(b.try_into().ok()?);
        self.0 = &self.0[4..];
        Some(v)
    }

    fn str(&mut self) -> Option<String> {
        let n = usize::from(self.u8()?);
        let b = self.0.get(..n)?;
        self.0 = &self.0[n..];
        Some(String::from_utf8_lossy(b).into_owned())
    }

    fn var(&mut self) -> Option<u32> {
        let mut v = 0u32;
        for shift in (0..35).step_by(7) {
            let b = self.u8()?;
            v |= u32::from(b & 0x7F) << shift;
            if b & 0x80 == 0 {
                return Some(v);
            }
        }
        None
    }

    fn signed(&mut self) -> Option<i32> {
        let z = self.var()?;
        Some((z >> 1) as i32 ^ -((z & 1) as i32))
    }
}

/// The server's file (`Kartcraft.cpp` `EncodeTake`): `KRPS`, a version, the body's size, the body
/// deflated; read into a [`Replay`] for `us` (our name), its rate the server's step. A racer's
/// kart is turned by their facing and drift on the level, as a remote kart is before its ground.
pub(crate) fn decode(file: &[u8], us: &str) -> Option<Replay> {
    let mut head = Bytes(file);
    if head.0.get(..4)? != b"KRPS" {
        return None;
    }
    head.0 = &head.0[4..];
    if head.u8()? != 1 {
        return None;
    }
    let size = head.u32()? as usize;
    if size > 64 << 20 {
        return None;
    }
    let mut body = Vec::with_capacity(size);
    flate2::read::ZlibDecoder::new(head.0)
        .take(size as u64)
        .read_to_end(&mut body)
        .ok()?;
    let mut r = Bytes(&body);
    let step = r.u16()?;
    let rate = u8::try_from((1000 / u32::from(step.max(1))).clamp(1, 60)).ok()?;
    let at = u64::from(r.u32()?);
    let time_trial = r.u8()? != 0;
    let course = r.str()?;
    let count = usize::from(r.u8()?);
    let mut tracks = Vec::with_capacity(count);
    let (mut ms, mut place) = (0, 0);
    for _ in 0..count {
        let name = r.str()?;
        let racer = r.str()?;
        let (_paint, _trail, _flag) = (r.u8()?, r.u8()?, r.u8()?);
        let finish = r.u8()?;
        let time = r.u32()?;
        let from = r.u32()?;
        let n = r.u32()? as usize;
        if n > 20 * 60 * 60 {
            return None;
        }
        let ours = name == us;
        if ours {
            (ms, place) = (time, finish);
        }
        let (mut x, mut y, mut z, mut o, mut drift) = (0i32, 0i32, 0i32, 0u16, 0i32);
        let mut poses = Vec::with_capacity(n);
        for _ in 0..n {
            x = x.wrapping_add(r.signed()?);
            y = y.wrapping_add(r.signed()?);
            z = z.wrapping_add(r.signed()?);
            o = o.wrapping_add(r.signed()? as u16);
            let steer = r.u8()? as i8;
            drift = drift.wrapping_add(r.signed()?);
            let _flags = r.u8()?;
            let yaw = f32::from(o) / 65536.0 * std::f32::consts::TAU + drift as f32 / 1000.0;
            let fwd = Vec3::new(-yaw.sin(), 0.0, -yaw.cos());
            let rot = Quat::from_mat3(&Mat3::from_cols(Vec3::Y.cross(fwd), Vec3::Y, fwd));
            poses.push(Pose {
                pos: wow_to_bevy([x as f32 / 100.0, y as f32 / 100.0, z as f32 / 100.0]),
                rot,
                steer: f32::from(steer) / 100.0,
            });
        }
        tracks.push(Track {
            name,
            racer: if racer.is_empty() {
                "crash".into()
            } else {
                racer
            },
            us: ours,
            from,
            poses,
        });
    }
    Some(Replay {
        course,
        at,
        ms,
        place,
        field: count as u8,
        time_trial,
        rate,
        tracks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file as `Kartcraft.cpp`'s `EncodeTake` writes it, built here.
    fn take(samples: &[(i32, i32, i32, u16, i8, i32)]) -> Vec<u8> {
        fn var(out: &mut Vec<u8>, mut v: u32) {
            while v >= 0x80 {
                out.push((v & 0x7F) as u8 | 0x80);
                v >>= 7;
            }
            out.push(v as u8);
        }
        fn signed(out: &mut Vec<u8>, v: i32) {
            var(out, ((v << 1) ^ (v >> 31)) as u32);
        }
        fn s(out: &mut Vec<u8>, t: &str) {
            out.push(t.len() as u8);
            out.extend_from_slice(t.as_bytes());
        }
        let mut body = vec![66, 0];
        body.extend_from_slice(&1_792_320_000u32.to_le_bytes());
        body.push(0);
        s(&mut body, "gp-azora");
        body.push(1);
        s(&mut body, "Racer");
        s(&mut body, "wok");
        body.extend_from_slice(&[0, 0, 0, 1]);
        body.extend_from_slice(&61_250u32.to_le_bytes());
        body.extend_from_slice(&3u32.to_le_bytes());
        body.extend_from_slice(&(samples.len() as u32).to_le_bytes());
        let mut last = (0, 0, 0, 0u16, 0);
        for &(x, y, z, o, steer, drift) in samples {
            signed(&mut body, x - last.0);
            signed(&mut body, y - last.1);
            signed(&mut body, z - last.2);
            signed(&mut body, i32::from(o.wrapping_sub(last.3) as i16));
            body.push(steer as u8);
            signed(&mut body, drift - last.4);
            body.push(1);
            last = (x, y, z, o, drift);
        }
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
        std::io::Write::write_all(&mut z, &body).unwrap();
        let mut out = b"KRPS\x01".to_vec();
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&z.finish().unwrap());
        out
    }

    #[test]
    fn a_server_take_reads_as_a_replay_ours_marked() {
        let file = take(&[
            (-565_100, -402_900, -5_180, 0, 0, 0),
            (-565_000, -402_900, -5_180, 16_384, 50, 120),
            (-564_900, -402_850, -5_170, 65_000, -100, -40),
        ]);
        let r = decode(&file, "Racer").expect("a take");
        assert_eq!(
            (r.course.as_str(), r.at, r.time_trial),
            ("gp-azora", 1_792_320_000, false)
        );
        assert_eq!((r.rate, r.ms, r.place, r.field), (15, 61_250, 1, 1));
        let t = &r.tracks[0];
        assert!(t.us && t.racer == "wok" && t.from == 3 && t.poses.len() == 3);
        assert_eq!(t.poses[1].pos, wow_to_bevy([-5650.0, -4029.0, -51.8]));
        assert!((t.poses[1].steer - 0.5).abs() < 1e-6);
        // A quarter turn's facing plus 0.12 rad of drift: the kart's +Z along it.
        let yaw = std::f32::consts::FRAC_PI_2 + 0.12;
        let fwd = t.poses[1].rot * Vec3::Z;
        assert!(
            fwd.distance(Vec3::new(-yaw.sin(), 0.0, -yaw.cos())) < 1e-4,
            "{fwd}"
        );
        // The facing wraps past a turn.
        assert!((t.poses[2].rot * Vec3::Y).distance(Vec3::Y) < 1e-4);
        // Someone else's: not ours.
        assert!(!decode(&file, "Other").unwrap().tracks[0].us);
        assert!(decode(b"KRPL\x01", "Racer").is_none());
    }

    #[test]
    fn only_the_server_s_keys_are_fetched() {
        assert!(key_ok("1792320294-12"));
        assert!(!key_ok("1792320294"));
        assert!(!key_ok("../etc-1"));
        assert!(!key_ok("12-3-4"));
    }
}
