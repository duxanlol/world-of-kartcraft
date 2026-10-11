//! Kart sounds: the engine as a loop whose pitch follows the speed, a tyre skid looping through
//! powerslides, the turbo's roar while reserves burn, and one-shots for the hop, landings, boosts,
//! an early press's exhaust puff, a full meter, crashes and the start lights' beeps; and the
//! weapons' sounds ([`Sfx`]): the roulette's loop and its stop, crates, fruit, launches,
//! explosions, the TNT, the shield and the mask, with loops while a missile, warp orb or bomb is
//! on its way. Each sound comes from `kart-data/audio/<name>.wav` when present (the player's own
//! CTR audio, converted at setup); the driving sounds are else synthesized here, the weapons' else
//! silent. Played 2D: the driver is the listener, and the weapons' cues carry a gain for their
//! distance.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::Event;
use bevy::prelude::*;
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle, StaticSoundSettings};
use kira::{Decibels, Frame, Tween};

const RATE: u32 = 44_100;
/// The engine loop's fundamental, Hz; the loop is a whole number of its periods.
const ENGINE_HZ: f32 = 80.0;

/// A one-shot the events ask for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cue {
    Hop,
    Land {
        hard: bool,
    },
    Boost(u8),
    Early,
    MeterFull,
    Turbo,
    Crash,
    /// A start light comes on: CTR's countdown beep at each red (OtherFX 0x45), its green sound
    /// at GO (`true`, 0x46) (DotLights.c:91-136).
    Countdown(bool),
    /// A ready check pops (the Race Finder's; no CTR sound).
    Ready,
    /// A weapon sound at a gain (its distance from us).
    Sfx(Sfx, f32),
}

/// The weapons' sounds, CTR's OtherFX ids as `ctr-extract` names them (`docs/ctr-assets.md`,
/// "Audio"; each sound's site in the decompilation is in `sounds.json`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sfx {
    /// 0x5E: the roulette lands; 0x41 with 10 Wumpa, also the fruit counter reaching 10.
    RouletteStop,
    RouletteJuiced,
    /// 0x3C: any crate breaks.
    CrateBreak,
    /// 0x43: a loose fruit taken; 0x42: a fruit reaching the counter.
    Wumpa,
    WumpaCount,
    /// 0x47 / 0x49.
    BombLaunch,
    BombExplode,
    /// 0x4A / 0x4C.
    MissileLaunch,
    MissileExplode,
    /// 0x52: TNT, Nitro or a beaker dropped.
    MineDrop,
    /// 0x50 / 0x51 / 0x3E / 0x3D: a TNT jumps on a head, sits down, honks, goes off.
    TntLand,
    TntJump,
    TntHonk,
    TntExplode,
    /// 0x3F: a beaker or Nitro shatters.
    Glass,
    /// 0x57 / 0x58: the shield goes up; pops, runs out or is fired.
    Shield,
    ShieldPop,
    /// 0x53 / 0x54.
    Aku,
    Uka,
    /// 0x44.
    Clock,
    /// 0x4D / 0x4F: a warp orb launched; dying.
    WarpLaunch,
    WarpHit,
}

impl Sfx {
    const ALL: [Sfx; 22] = [
        Sfx::RouletteStop,
        Sfx::RouletteJuiced,
        Sfx::CrateBreak,
        Sfx::Wumpa,
        Sfx::WumpaCount,
        Sfx::BombLaunch,
        Sfx::BombExplode,
        Sfx::MissileLaunch,
        Sfx::MissileExplode,
        Sfx::MineDrop,
        Sfx::TntLand,
        Sfx::TntJump,
        Sfx::TntHonk,
        Sfx::TntExplode,
        Sfx::Glass,
        Sfx::Shield,
        Sfx::ShieldPop,
        Sfx::Aku,
        Sfx::Uka,
        Sfx::Clock,
        Sfx::WarpLaunch,
        Sfx::WarpHit,
    ];

    fn file(self) -> &'static str {
        match self {
            Sfx::RouletteStop => "roulette_stop",
            Sfx::RouletteJuiced => "roulette_juiced",
            Sfx::CrateBreak => "crate_break",
            Sfx::Wumpa => "wumpa",
            Sfx::WumpaCount => "wumpa_count",
            Sfx::BombLaunch => "bomb_launch",
            Sfx::BombExplode => "bomb_explode",
            Sfx::MissileLaunch => "missile_launch",
            Sfx::MissileExplode => "missile_explode",
            Sfx::MineDrop => "mine_drop",
            Sfx::TntLand => "tnt_land",
            Sfx::TntJump => "tnt_jump",
            Sfx::TntHonk => "tnt_honk",
            Sfx::TntExplode => "tnt_explode",
            Sfx::Glass => "glass",
            Sfx::Shield => "shield",
            Sfx::ShieldPop => "shield_pop",
            Sfx::Aku => "aku",
            Sfx::Uka => "uka",
            Sfx::Clock => "clock",
            Sfx::WarpLaunch => "warp_launch",
            Sfx::WarpHit => "warp_hit",
        }
    }
}

/// The weapons' loops this frame: the roulette spinning, and how loud (0 to 1, by distance) the
/// nearest missile, warp orb and rolling bomb are.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WeaponLoops {
    pub roulette: bool,
    pub missile: f32,
    pub warp: f32,
    pub bomb: f32,
}

/// What the kart update hands the sounds each frame.
#[derive(Resource, Default)]
pub struct KartAudio {
    pub cues: Vec<Cue>,
    /// lu/s.
    pub speed: f32,
    pub drifting: bool,
    pub grounded: bool,
    pub turbo: bool,
    /// Seconds since the turbo's roar last started its sweep: a new boost restarts it.
    pub turbo_age: f32,
    /// The engine's distortion and volume (`ctr_kart::Kart::engine_sound`).
    pub engine: (u8, u8),
    /// The music's volume, 0 to 1 (the host's music slider).
    pub music: f32,
    /// The music's tempo against its own (CTR plays the final lap 20 BPM faster); 0 reads as 1.
    pub music_rate: f64,
    pub weapons: WeaponLoops,
}

/// The turbo roar's pitch `age` seconds into its sweep, as CTR's `VehTurbo_ThTick` plays it
/// (`VehTurbo.c:339-374`): OtherFX 0xE at distortion fireAudioDistort + 0x10, capped at 0x80,
/// the counter zeroed by each boost and counting one a 30 Hz frame. So the roar starts about an
/// octave down and climbs to the sample's own pitch over 3.7 s; played flat at 0x80 it whines.
pub fn turbo_pitch(age: f32) -> f64 {
    let distort = (0x10 + (age.max(0.0) * 30.0) as u32).min(0x80);
    // OtherFX distortion (`distortConst_OtherFX`): an octave per 128.
    2f64.powf((f64::from(distort) - 128.0) / 128.0)
}

/// A landing this fast downward (speed units) or harder plays the heavy thump.
const HARD_LANDING: i32 = 9000;

impl KartAudio {
    /// One frame of `dt` seconds with the turbo burning or not: the roar's sweep runs while it
    /// burns and starts over once it is out.
    pub fn burn(&mut self, burning: bool, dt: f32) {
        self.turbo = burning;
        self.turbo_age = if burning { self.turbo_age + dt } else { 0.0 };
    }

    pub fn observe(&mut self, event: &Event) {
        // Every boost restarts the roar's sweep (`VehFire_Increment` zeroes fireAudioDistort).
        if matches!(
            event,
            Event::Boost { .. } | Event::HangTime { .. } | Event::Turbo
        ) {
            self.turbo_age = 0.0;
        }
        let cue = match *event {
            Event::Hop => Cue::Hop,
            Event::Land { air_ms, fall } if air_ms > 96 => Cue::Land {
                hard: fall > HARD_LANDING,
            },
            Event::Boost { stage, .. } => Cue::Boost(stage),
            Event::HangTime { .. } => Cue::Boost(1),
            Event::BoostEarly => Cue::Early,
            // The same OtherFX 0xF (`VehPhysProc.c` meter full, `VehStuckProc.c:1098` over-rev).
            Event::MeterFull | Event::OverRev => Cue::MeterFull,
            Event::Turbo => Cue::Turbo,
            Event::Slam { .. } | Event::SpinOut => Cue::Crash,
            Event::Hit {
                kind: ctr_kart::HitKind::TntOnHead,
            } => Cue::Sfx(Sfx::TntLand, 1.0),
            Event::TntLanded => Cue::Sfx(Sfx::TntJump, 1.0),
            Event::TntExploded => Cue::Sfx(Sfx::TntExplode, 1.0),
            Event::ShieldPopped | Event::ShieldExpired => Cue::Sfx(Sfx::ShieldPop, 1.0),
            _ => return,
        };
        self.cues.push(cue);
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// `WOW_KART_AUDIO`, else a `kart-data/audio` beside or above the working directory.
fn audio_root() -> Option<PathBuf> {
    std::env::var_os("WOW_KART_AUDIO")
        .map(PathBuf::from)
        .or_else(|| {
            ["kart-data/audio", "../kart-data/audio"]
                .into_iter()
                .map(PathBuf::from)
                .find(|p| p.is_dir())
        })
}

/// A small deterministic noise source.
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

/// `seconds` of mono audio from `f(t, noise)`.
fn synth(seconds: f32, mut f: impl FnMut(f32, &mut Noise) -> f32) -> StaticSoundData {
    let mut noise = Noise(0x2545_f491);
    let n = (seconds * RATE as f32) as usize;
    let frames: Arc<[Frame]> = (0..n)
        .map(|i| {
            let s = f(i as f32 / RATE as f32, &mut noise).clamp(-1.0, 1.0);
            Frame { left: s, right: s }
        })
        .collect();
    StaticSoundData {
        sample_rate: RATE,
        frames,
        settings: StaticSoundSettings::default(),
        slice: None,
    }
}

/// An exponential decay envelope with a few milliseconds of attack.
fn env(t: f32, decay: f32) -> f32 {
    (t / 0.004).min(1.0) * (-t / decay).exp()
}

fn engine() -> StaticSoundData {
    use std::f32::consts::TAU;
    // A buzzy two-stroke: a few saw-ish harmonics, softly clipped, a whole number of periods.
    synth(1.0, |t, n| {
        let p = TAU * ENGINE_HZ * t;
        let tone = p.sin()
            + 0.6 * (2.0 * p).sin()
            + 0.45 * (3.0 * p).sin()
            + 0.3 * (4.0 * p).sin()
            + 0.2 * (6.0 * p).sin();
        let rumble = 0.08 * n.next();
        (0.35 * (tone * 0.6 + rumble)).tanh()
    })
}

fn skid() -> StaticSoundData {
    // Band-limited noise: two one-pole filters, high minus low.
    let (mut lo, mut hi) = (0.0f32, 0.0f32);
    synth(1.0, move |_, n| {
        let x = n.next();
        hi += 0.35 * (x - hi);
        lo += 0.04 * (x - lo);
        0.7 * (hi - lo)
    })
}

fn roar() -> StaticSoundData {
    let mut lo = 0.0f32;
    synth(1.0, move |_, n| {
        lo += 0.08 * (n.next() - lo);
        lo * 2.2
    })
}

fn hop() -> StaticSoundData {
    use std::f32::consts::TAU;
    let mut phase = 0.0f32;
    synth(0.14, move |t, _| {
        phase += TAU * (300.0 + 1800.0 * t) / RATE as f32;
        0.45 * phase.sin() * env(t, 0.05)
    })
}

fn land(hard: bool) -> StaticSoundData {
    use std::f32::consts::TAU;
    let (hz, decay) = if hard { (55.0, 0.14) } else { (85.0, 0.07) };
    let mut lo = 0.0f32;
    synth(0.35, move |t, n| {
        lo += 0.1 * (n.next() - lo);
        (0.8 * (TAU * hz * t).sin() + 1.5 * lo) * env(t, decay)
    })
}

fn boost(stage: u8) -> StaticSoundData {
    use std::f32::consts::TAU;
    let base = 220.0 * (1.0 + 0.26 * f32::from(stage.saturating_sub(1)));
    let mut lo = 0.0f32;
    let mut phase = 0.0f32;
    synth(0.6, move |t, n| {
        lo += (0.05 + 0.4 * t) * (n.next() - lo);
        phase += TAU * base * (1.0 + t) / RATE as f32;
        (0.9 * lo + 0.25 * phase.sin()) * env(t, 0.25)
    })
}

fn puff() -> StaticSoundData {
    let mut lo = 0.0f32;
    synth(0.25, move |t, n| {
        lo += 0.06 * (n.next() - lo);
        2.0 * lo * env(t, 0.06)
    })
}

fn beep() -> StaticSoundData {
    use std::f32::consts::TAU;
    synth(0.12, |t, _| 0.3 * (TAU * 880.0 * t).sin() * env(t, 0.05))
}

/// The beep an octave up, for GO without the player's audio.
fn go_beep() -> StaticSoundData {
    use std::f32::consts::TAU;
    synth(0.2, |t, _| 0.3 * (TAU * 1760.0 * t).sin() * env(t, 0.08))
}

fn crash() -> StaticSoundData {
    let mut lo = 0.0f32;
    synth(0.7, move |t, n| {
        let x = n.next();
        lo += 0.2 * (x - lo);
        (0.6 * x + 1.2 * lo) * env(t, 0.18)
    })
}

/// The sounds, each from the player's audio when present.
struct Bank {
    engine: StaticSoundData,
    skid: StaticSoundData,
    roar: StaticSoundData,
    hop: StaticSoundData,
    land: StaticSoundData,
    land_hard: StaticSoundData,
    boost: [StaticSoundData; 3],
    puff: StaticSoundData,
    beep: StaticSoundData,
    crash: StaticSoundData,
    /// The start lights' red and green sounds.
    countdown: StaticSoundData,
    go: StaticSoundData,
    /// The weapons' one-shots and loops, those the player's audio has.
    sfx: std::collections::HashMap<Sfx, StaticSoundData>,
    roulette: Option<StaticSoundData>,
    missile: Option<StaticSoundData>,
    warp: Option<StaticSoundData>,
    bomb: Option<StaticSoundData>,
}

impl Bank {
    fn load() -> Self {
        let root = audio_root();
        let mut loaded = 0;
        let mut one = |name: &str, fallback: fn() -> StaticSoundData| {
            root.as_ref()
                .and_then(|r| StaticSoundData::from_file(r.join(format!("{name}.wav"))).ok())
                .inspect(|_| loaded += 1)
                .unwrap_or_else(fallback)
        };
        let file = |name: &str| {
            root.as_ref()
                .and_then(|r| StaticSoundData::from_file(r.join(format!("{name}.wav"))).ok())
        };
        let sfx: std::collections::HashMap<Sfx, StaticSoundData> = Sfx::ALL
            .iter()
            .filter_map(|&s| Some((s, file(s.file())?)))
            .collect();
        let (roulette, missile, warp, bomb) = (
            file("roulette"),
            file("missile_fly"),
            file("warp_fly"),
            file("bomb_roll"),
        );
        let weapons = sfx.len()
            + [&roulette, &missile, &warp, &bomb]
                .iter()
                .filter(|l| l.is_some())
                .count();
        let bank = Self {
            engine: one("engine", engine),
            skid: one("skid", skid),
            roar: one("turbo", roar),
            hop: one("hop", hop),
            land: one("land", || land(false)),
            land_hard: one("land_hard", || land(true)),
            boost: [
                one("boost_1", || boost(1)),
                one("boost_2", || boost(2)),
                one("boost_3", || boost(3)),
            ],
            puff: one("boost_early", puff),
            beep: one("meter_full", beep),
            crash: one("crash", crash),
            countdown: one("countdown", beep),
            go: one("go", go_beep),
            sfx,
            roulette,
            missile,
            warp,
            bomb,
        };
        info!(
            "kart sound: {loaded} sounds from the player's audio, the rest synthesized; \
             {weapons} weapon sounds"
        );
        bank
    }
}

/// The loops while driving, each started silent and faded to its level every frame.
#[derive(Default)]
struct Loops {
    engine: Option<StaticSoundHandle>,
    skid: Option<StaticSoundHandle>,
    roar: Option<StaticSoundHandle>,
    roulette: Option<StaticSoundHandle>,
    missile: Option<StaticSoundHandle>,
    warp: Option<StaticSoundHandle>,
    bomb: Option<StaticSoundHandle>,
}

impl Loops {
    fn all(&mut self) -> [&mut Option<StaticSoundHandle>; 7] {
        [
            &mut self.engine,
            &mut self.skid,
            &mut self.roar,
            &mut self.roulette,
            &mut self.missile,
            &mut self.warp,
            &mut self.bomb,
        ]
    }
}

fn tween(ms: u64) -> Tween {
    Tween {
        duration: Duration::from_millis(ms),
        ..Default::default()
    }
}

fn db(amp: f32) -> Decibels {
    if amp <= 0.0001 {
        Decibels::SILENCE
    } else {
        Decibels(20.0 * amp.log10())
    }
}

/// The engine loop (the player's CTR sample, else synthesized), for other drivers' karts.
pub fn engine_loop() -> StaticSoundData {
    static ENGINE: std::sync::OnceLock<StaticSoundData> = std::sync::OnceLock::new();
    ENGINE
        .get_or_init(|| {
            audio_root()
                .and_then(|r| StaticSoundData::from_file(r.join("engine.wav")).ok())
                .unwrap_or_else(engine)
        })
        .clone()
}

/// CTR's engine pitch for a distortion: `2^((d − 128) / 64)` (`distortConst_Engine`).
pub fn engine_rate(distortion: u8) -> f64 {
    f64::from(((f32::from(distortion) - 128.0) / 64.0).exp2())
}

/// The kart's sounds while driving: its loops and the bank they play from. The host plays each
/// sound through `play` (the client's mixer, or the demo's own audio output).
#[derive(Default)]
pub struct Sounds {
    bank: Option<Bank>,
    loops: Loops,
    driving: bool,
    /// The race track playing, and one being read off the disk for it.
    music: Option<StaticSoundHandle>,
    loading: Option<std::sync::mpsc::Receiver<Option<StaticSoundData>>>,
}

/// A CTR race track rendered from the player's disc (`kart-data/music/music.json`).
pub struct Track {
    pub name: String,
    pub file: PathBuf,
}

/// The race tracks there are.
pub fn tracks() -> &'static [Track] {
    static TRACKS: std::sync::OnceLock<Vec<Track>> = std::sync::OnceLock::new();
    TRACKS.get_or_init(|| {
        let Some(root) = crate::model::data_root().map(|r| r.join("music")) else {
            return Vec::new();
        };
        let Ok(data) = std::fs::read(root.join("music.json")) else {
            return Vec::new();
        };
        let Ok(index) = serde_json::from_slice::<serde_json::Value>(&data) else {
            return Vec::new();
        };
        let mut tracks: Vec<Track> = index["tracks"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|t| {
                Some(Track {
                    name: t["track"].as_str()?.to_string(),
                    file: root.join(t["file"].as_str()?),
                })
            })
            .filter(|t| t.file.is_file())
            .collect();
        // Tracks sharing one song are one track.
        let mut seen = std::collections::HashSet::new();
        tracks.retain(|t| seen.insert(t.file.clone()));
        info!("kart music: {} CTR race tracks", tracks.len());
        tracks
    })
}

/// Whether the kart plays CTR's music: there are tracks and `WOW_KART_MUSIC` is not `off`.
pub fn music_on() -> bool {
    std::env::var("WOW_KART_MUSIC").map_or(true, |v| v != "off") && !tracks().is_empty()
}

/// The track for this ride: `WOW_KART_MUSIC=<track>`, else a different one each time.
fn pick_track() -> Option<&'static Track> {
    let tracks = tracks();
    if let Ok(name) = std::env::var("WOW_KART_MUSIC") {
        if let Some(t) = tracks.iter().find(|t| t.name == name) {
            return Some(t);
        }
    }
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as usize);
    tracks.get(seed % tracks.len().max(1))
}

impl Sounds {
    /// One frame: the loops faded to this frame's levels, this frame's cues played. `sfx` is the
    /// effects volume, 0 to 1.
    pub fn update(
        &mut self,
        audio: &mut KartAudio,
        active: bool,
        sfx: f32,
        mut play: impl FnMut(StaticSoundData) -> Option<StaticSoundHandle>,
    ) {
        let cues = std::mem::take(&mut audio.cues);
        if !active {
            if self.driving {
                self.driving = false;
                for handle in self.loops.all() {
                    if let Some(mut h) = handle.take() {
                        h.stop(tween(250));
                    }
                }
                if let Some(mut m) = self.music.take() {
                    m.stop(tween(1200));
                }
                self.loading = None;
            }
            return;
        }
        // The race track: read on its own thread, started when it is in, looped.
        if !self.driving && music_on() {
            if let Some(track) = pick_track() {
                info!("kart music: {}", track.name);
                let (send, receive) = std::sync::mpsc::channel();
                let file = track.file.clone();
                let _ = std::thread::Builder::new()
                    .name("kart-music".into())
                    .spawn(move || {
                        let _ = send.send(StaticSoundData::from_file(file).ok());
                    });
                self.loading = Some(receive);
            }
        }
        if let Some(Ok(loaded)) = self.loading.as_ref().map(|r| r.try_recv()) {
            self.loading = None;
            if let Some(data) = loaded {
                self.music = play(data.loop_region(..).volume(Decibels::SILENCE));
            }
        }
        if let Some(m) = self.music.as_mut() {
            m.set_volume(db(0.8 * audio.music), tween(400));
            let rate = if audio.music_rate > 0.0 {
                audio.music_rate
            } else {
                1.0
            };
            m.set_playback_rate(rate, tween(1500));
        }
        let bank: &Bank = self.bank.get_or_insert_with(Bank::load);
        if !self.driving {
            let mut start = |data: &StaticSoundData| {
                play(data.clone().loop_region(..).volume(Decibels::SILENCE))
            };
            self.loops = Loops {
                engine: start(&bank.engine),
                skid: start(&bank.skid),
                roar: start(&bank.roar),
                roulette: bank.roulette.as_ref().and_then(&mut start),
                missile: bank.missile.as_ref().and_then(&mut start),
                warp: bank.warp.as_ref().and_then(&mut start),
                bomb: bank.bomb.as_ref().and_then(&mut start),
            };
            self.driving = true;
        }

        // The engine as CTR mixes it (`Kart::engine_sound`, from `EngineSound_Player`): the
        // sample at 2^((d − 128) / 64) times its own rate (`distortConst_Engine`), at the game's
        // base volume 153 scaled by the swell. Skid and turbo at their base volumes too.
        let (d, volume) = audio.engine;
        let engine = (
            0.6 * f32::from(volume) / 230.0,
            f64::from(((f32::from(d) - 128.0) / 64.0).exp2()),
        );
        let skid = if audio.drifting && audio.grounded {
            (0.5, 1.0)
        } else {
            (0.0, 1.0)
        };
        let roar = if audio.turbo {
            (0.4, turbo_pitch(audio.turbo_age))
        } else {
            (0.0, 1.0)
        };
        let w = audio.weapons;
        let weapons = [
            (if w.roulette { 0.7 } else { 0.0 }, 1.0),
            (0.7 * w.missile, 1.0),
            (0.7 * w.warp, 1.0),
            (0.6 * w.bomb, 1.0),
        ];
        let levels = [engine, skid, roar].into_iter().chain(weapons);
        for (handle, (level, rate)) in self.loops.all().into_iter().zip(levels) {
            if let Some(h) = handle.as_mut() {
                h.set_volume(db(level * sfx), tween(60));
                h.set_playback_rate(rate, tween(60));
            }
        }

        for cue in cues {
            let (data, gain) = match cue {
                Cue::Hop => (&bank.hop, 0.5),
                Cue::Land { hard: true } => (&bank.land_hard, 1.0),
                Cue::Land { hard: false } => (&bank.land, 0.8),
                // The stage's pitch and volume are in the files (CTR plays one sample three ways).
                Cue::Boost(stage) => (&bank.boost[usize::from(stage.clamp(1, 3) - 1)], 1.0),
                Cue::Early => (&bank.puff, 0.7),
                Cue::MeterFull => (&bank.beep, 0.5),
                Cue::Turbo => (&bank.boost[2], 1.0),
                Cue::Crash => (&bank.crash, 0.6),
                // CTR plays both at full volume (OtherFX_Play's default flags).
                Cue::Countdown(false) => (&bank.countdown, 1.0),
                Cue::Countdown(true) => (&bank.go, 1.0),
                Cue::Ready => (&bank.beep, 0.9),
                Cue::Sfx(sfx, gain) => match bank.sfx.get(&sfx) {
                    Some(data) => (data, gain.clamp(0.0, 1.0)),
                    None => continue,
                },
            };
            let mut data = data.clone().volume(db(gain * sfx));
            if cue == Cue::Ready {
                data = data.playback_rate(2.0);
            }
            let _ = play(data);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_turbo_roar_starts_low_and_climbs_to_its_own_pitch_as_ctr_s_does() {
        // fireAudioDistort 0 plays at 0x10: 2^(-112/128), about 0.55 of the sample's pitch.
        assert!((turbo_pitch(0.0) - 0.5453).abs() < 1e-3);
        // 112 frames (3.73 s) on it reaches 0x80, the sample's own pitch, and stays there.
        assert!(turbo_pitch(2.0) < turbo_pitch(3.0));
        assert!((turbo_pitch(112.0 / 30.0) - 1.0).abs() < 1e-9);
        assert!((turbo_pitch(10.0) - 1.0).abs() < 1e-9);

        // A boost restarts the sweep; the turbo running out does too.
        let mut a = KartAudio::default();
        a.burn(true, 2.0);
        assert!((a.turbo_age - 2.0).abs() < 1e-6);
        a.observe(&Event::Turbo);
        assert_eq!(a.turbo_age, 0.0);
        a.burn(true, 1.0);
        a.burn(false, 0.016);
        assert_eq!(a.turbo_age, 0.0);
        assert!(!a.turbo);
    }

    #[test]
    fn events_become_cues() {
        let mut a = KartAudio::default();
        a.observe(&Event::Hop);
        a.observe(&Event::Land {
            air_ms: 64,
            fall: 100,
        });
        a.observe(&Event::Land {
            air_ms: 900,
            fall: 12000,
        });
        a.observe(&Event::Boost {
            stage: 2,
            reserves: 1000,
        });
        a.observe(&Event::Slam { speed: 9000 });
        assert_eq!(
            a.cues,
            [
                Cue::Hop,
                Cue::Land { hard: true },
                Cue::Boost(2),
                Cue::Crash
            ]
        );
    }

    #[test]
    fn the_synthesized_sounds_are_sane() {
        for s in [
            engine(),
            skid(),
            roar(),
            hop(),
            land(true),
            boost(3),
            crash(),
            beep(),
            go_beep(),
        ] {
            assert!(!s.frames.is_empty());
            assert!(s
                .frames
                .iter()
                .all(|f| f.left.is_finite() && f.left.abs() <= 1.0));
            assert!(s.frames.iter().any(|f| f.left.abs() > 0.05), "silent");
        }
        // The engine loops seamlessly: a whole number of periods.
        let periods = engine().frames.len() as f32 * ENGINE_HZ / RATE as f32;
        assert!(
            (periods - periods.round()).abs() < 1e-3,
            "{periods} periods"
        );
    }
}
