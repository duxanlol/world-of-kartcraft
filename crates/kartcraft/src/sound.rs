//! Kart sounds: the engine as a loop whose pitch follows the speed, a tyre skid looping through
//! powerslides, the turbo's roar while reserves burn, and one-shots for the hop, landings, boosts,
//! an early press's exhaust puff, a full meter, crashes and the start lights' beeps; and the
//! weapons' sounds ([`Sfx`]): the roulette's loop and its stop, crates, fruit, launches,
//! explosions, the TNT, the shield and the mask, with loops while a missile, warp orb or bomb is
//! on its way. Each sound comes from `kart-data/audio/<name>.wav` when present (the player's own
//! CTR audio, converted at setup); the driving sounds are else synthesized here, the weapons' else
//! silent. Played 2D: the driver is the listener, and the weapons' cues carry a gain for their
//! distance.
//!
//! The skid loop plays while the tyres skid ([`ctr_kart::Kart::skids`]), not only in a
//! powerslide, and it and a rolling loop are the ground's own, as CTR's terrain table names them.

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
    /// A landing at CTR's volume for it, 0x78 to 0xFA ([`landing_volume`]).
    Land {
        volume: u8,
    },
    /// A crash into another kart at its volume, 0x3F to 0xFF ([`bump_volume`]).
    Bump(u8),
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
    /// The tyres skid ([`ctr_kart::Kart::skids`], either pair).
    pub skidding: bool,
    /// The ground under the kart, as it is (not as it drives): its skid and rolling sounds.
    pub ground: ctr_kart::Ground,
    /// Seconds since the turbo's roar last started its sweep: a new boost restarts it.
    pub turbo_age: f32,
    /// The engine's distortion and volume (`ctr_kart::Kart::engine_sound`).
    pub engine: (u8, u8),
    /// The music's volume, 0 to 1 (the host's music slider).
    pub music: f32,
    /// Beats a minute on the playing song's own tempo: CTR's final lap adds 20
    /// (`HOWL_AudioState.c:338`, `Audio_SetMaskSong(20)`), 0 plays it as written.
    pub music_bpm_delta: f64,
    pub weapons: WeaponLoops,
    /// The engine class whose sample the engine plays (the garage's).
    pub engine_class: crate::Class,
    /// Our mask's theme while we wear it: Aku Aku's (`true`) or Uka Uka's.
    pub mask_theme: Option<bool>,
    /// The wall scrape loop's volume, CTR's `engineVol` 0 to 0xFF ([`KartAudio::tick`]).
    pub wall_volume: u8,
    /// Voice lines to play this frame (files).
    pub voices: Vec<PathBuf>,
    /// The game frames left before another kart crash sounds (`VehPhysCrash.c:520-524`).
    bump_cooldown: u8,
    voice: crate::voice::Voice,
}

/// The playback rate that plays a song written at `bpm` at `bpm + delta`: CTR changes the
/// sequence's tempo (`CseqMusic_ChangeTempo`), our rendered song can only be played faster
/// (its pitch rises with it). A song of unknown tempo plays as it is.
pub fn tempo_rate(bpm: f64, delta: f64) -> f64 {
    if bpm > 0.0 {
        ((bpm + delta) / bpm).max(0.1)
    } else {
        1.0
    }
}

/// `VehCalc_MapToRange`: `x` clamped to `[in_lo, in_hi]` and mapped linearly onto
/// `[out_lo, out_hi]`.
pub fn map_to_range(x: i32, in_lo: i32, in_hi: i32, out_lo: i32, out_hi: i32) -> i32 {
    let x = x.clamp(in_lo, in_hi);
    out_lo + (x - in_lo) * (out_hi - out_lo) / (in_hi - in_lo).max(1)
}

/// A landing's volume (`COLL.c:1508`): `MapToRange(|landing speed|, 0x100, 0x3C00, 0x78, 0xFA)`,
/// played at every touchdown.
pub fn landing_volume(fall: i32) -> u8 {
    map_to_range(fall.abs(), 0x100, 0x3C00, 0x78, 0xFA) as u8
}

/// A kart crash's volume (`VehPhysCrash.c:533`): `MapToRange(impact, 0, 0x1900, 0x3F, 0xFF)`;
/// `None` at 0x200 or softer, which make no sound.
pub fn bump_volume(strength: i32) -> Option<u8> {
    (strength > 0x200).then(|| map_to_range(strength, 0, 0x1900, 0x3F, 0xFF) as u8)
}

/// `OtherFX_DriverCrashing` (`HOWL_OtherFXRecycle.c:44-63`): the crash sample a volume plays,
/// 0 soft (0xB), 1 middling (0xC, over 0xA0), 2 hard (0xA, 0xDD and over).
pub fn bump_sample(volume: u8) -> usize {
    match volume {
        0xDD.. => 2,
        0xA1.. => 1,
        _ => 0,
    }
}

/// `VEH_EMITTER_WALL_RUB_VOLUME_STEP` (`VehEmitter.c:104`): the scrape loop's volume climbs and
/// falls this much a frame.
const WALL_RUB_STEP: u8 = 0x14;

/// The turbo roar's pitch `age` seconds into its sweep, as CTR's `VehTurbo_ThTick` plays it
/// (`VehTurbo.c:339-374`): OtherFX 0xE at distortion fireAudioDistort + 0x10, capped at 0x80,
/// the counter zeroed by each boost and counting one a 30 Hz frame. So the roar starts about an
/// octave down and climbs to the sample's own pitch over 3.7 s; played flat at 0x80 it whines.
pub fn turbo_pitch(age: f32) -> f64 {
    let distort = (0x10 + (age.max(0.0) * 30.0) as u32).min(0x80);
    // OtherFX distortion (`distortConst_OtherFX`): an octave per 128.
    2f64.powf((f64::from(distort) - 128.0) / 128.0)
}

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
        if let Some(line) = crate::voice::line_for(event) {
            self.say(line);
        }
        let cue = match *event {
            Event::Hop => Cue::Hop,
            // Every touchdown thumps, softly for a small hop (`COLL.c:1504-1514`).
            Event::Land { fall, .. } => Cue::Land {
                volume: landing_volume(fall),
            },
            // One kart crash in three frames (`VEH_PHYS_CRASH_FEEDBACK_COOLDOWN_FRAMES`).
            Event::Bump { strength } => match bump_volume(strength) {
                Some(volume) if self.bump_cooldown == 0 => {
                    self.bump_cooldown = 3;
                    if volume > 0xDC {
                        self.say(crate::voice::line::HARD_CRASH);
                    }
                    Cue::Bump(volume)
                }
                _ => return,
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

    /// The voices, reading the converted lines from `kart-data/audio/voice`.
    fn voice(&mut self) -> &mut crate::voice::Voice {
        if !self.voice.has_root() {
            self.voice.set_root(audio_root().map(|r| r.join("voice")));
        }
        &mut self.voice
    }

    /// Asks for our racer's ([`crate::model::our_kart`]) voice line `id`
    /// (`Voiceline_RequestPlay`).
    pub fn say(&mut self, id: u8) {
        self.say_as(id, crate::model::our_kart());
    }

    /// Asks for voice line `id` in `racer`'s voice (another driver's kart look; `""` is Crash).
    pub fn say_as(&mut self, id: u8, racer: &str) {
        if let Some(file) = self.voice().request(id, racer) {
            self.voices.push(file);
        }
    }

    /// The rest of a frame, after its events: the step's game frames (`fb`) pace the voice
    /// queue, the crash cooldown and the wall scrape's volume (`VehEmitter.c:1356-1383`: up
    /// 0x14 a frame scraping, down 0x14 a frame off the wall); our weapons fired this frame
    /// speak (`VehPickupItem.c:789-1372`), the clock unless it was another's that caught us.
    pub fn tick(&mut self, fb: &crate::session::Feedback, clocked: bool) {
        use crate::voice::line;
        let fired: Vec<u8> = self
            .cues
            .iter()
            .filter_map(|c| match *c {
                // Our own shots sound at full gain (no distance).
                Cue::Sfx(s, gain) if gain >= 0.99 => match s {
                    Sfx::BombLaunch => Some(line::BOMB),
                    Sfx::MissileLaunch => Some(line::MISSILE),
                    Sfx::WarpLaunch => Some(line::WARP_ORB),
                    Sfx::MineDrop => Some(line::MINE),
                    Sfx::Clock if !clocked => Some(line::CLOCK),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        for id in fired {
            self.say(id);
        }
        let off = fb.frames.saturating_sub(fb.wall_frames);
        let v = self
            .wall_volume
            .saturating_add(WALL_RUB_STEP.saturating_mul(fb.wall_frames.min(13)));
        self.wall_volume = v.saturating_sub(WALL_RUB_STEP.saturating_mul(off.min(13)));
        self.bump_cooldown = self.bump_cooldown.saturating_sub(fb.frames);
        let voice = self.voice();
        let said: Vec<PathBuf> = (0..fb.frames).filter_map(|_| voice.frame()).collect();
        self.voices.extend(said);
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

/// A ground's skid loop file by its OtherFX id (`MetaDataTerrain.skidSound`): 17 asphalt, 18
/// dirt and grass, 19 snow and icy road (`ctr_assets::audio::SOUNDS`).
fn skid_file(id: u16) -> Option<&'static str> {
    match id {
        17 => Some("skid"),
        18 => Some("skid_dirt"),
        19 => Some("skid_snow"),
        _ => None,
    }
}

/// A ground's rolling loop file by its OtherFX id (`MetaDataTerrain.sound`).
fn rolling_file(id: u16) -> Option<&'static str> {
    match id {
        22 => Some("ground_ice"),
        23 => Some("ground_snow"),
        24 => Some("ground_dirt"),
        25 => Some("ground_water"),
        26 => Some("ground_metal"),
        27 => Some("ground_mud"),
        _ => None,
    }
}

/// The skid loop's level, 0 to 1, at `speed` (CTR's units): `VehEmitter`'s volume (0x14 to
/// 0xAA over 2000 to 12000, none below 0x201), at our 0.5 for a skid at full speed.
fn skid_level(skidding: bool, grounded: bool, speed: i32) -> f32 {
    if !(skidding && grounded) {
        return 0.0;
    }
    0.5 * ctr_kart::terrain::skid_volume(speed) as f32 / 170.0
}

/// The rolling loop's level and pitch at `speed`: `VehEmitter`'s volume (0 to 200 over 0 to
/// 5000) and OtherFX distortion (0x6C to 0xD2 over 0 to 12000), on the ground only.
fn rolling_mix(grounded: bool, speed: i32) -> (f32, f64) {
    let (volume, distort) = ctr_kart::terrain::rolling_mix(speed);
    let level = if grounded {
        0.6 * volume as f32 / 255.0
    } else {
        0.0
    };
    (level, 2f64.powf((f64::from(distort) - 128.0) / 128.0))
}

/// The sounds, each from the player's audio when present.
struct Bank {
    /// Each engine class's sample, in [`crate::Class::ALL`] order (`HOWL_Engine.c:323`).
    engines: [StaticSoundData; 4],
    skid: StaticSoundData,
    /// The other grounds' skid loops and every ground's rolling loop, those the audio has, by
    /// OtherFX id.
    skids: std::collections::HashMap<u16, StaticSoundData>,
    rolling: std::collections::HashMap<u16, StaticSoundData>,
    roar: StaticSoundData,
    hop: StaticSoundData,
    /// Sound 7 at its loudest landing volume, 0xFA; softer landings scale it down.
    land: StaticSoundData,
    /// `OtherFX_DriverCrashing`'s three samples, soft to hard.
    bumps: [StaticSoundData; 3],
    /// The wall scrape loop (0x14).
    wall: StaticSoundData,
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
        let skids = [18u16, 19]
            .into_iter()
            .filter_map(|id| Some((id, file(skid_file(id)?)?)))
            .collect();
        let rolling = (22u16..=27)
            .filter_map(|id| Some((id, file(rolling_file(id)?)?)))
            .collect();
        // Kart data converted before the other classes' engines were has Balanced's alone.
        let balanced = one("engine", engine);
        let class = |name: &str| file(name).unwrap_or_else(|| balanced.clone());
        let engines = [
            balanced.clone(),
            class("engine_accel"),
            class("engine_speed"),
            class("engine_turn"),
        ];
        let bank = Self {
            engines,
            skid: one("skid", skid),
            skids,
            rolling,
            roar: one("turbo", roar),
            hop: one("hop", hop),
            land: one("land_hard", || land(true)),
            bumps: [
                one("bump_soft", crash),
                one("bump_mid", crash),
                one("bump_hard", crash),
            ],
            wall: one("wall_rub", skid),
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
    /// The ground's rolling loop.
    ground: Option<StaticSoundHandle>,
    /// The OtherFX ids the skid and the rolling loops play, swapped as the ground changes.
    skid_id: Option<u16>,
    ground_id: Option<u16>,
}

impl Loops {
    fn all(&mut self) -> [&mut Option<StaticSoundHandle>; 8] {
        [
            &mut self.engine,
            &mut self.skid,
            &mut self.roar,
            &mut self.roulette,
            &mut self.missile,
            &mut self.warp,
            &mut self.bomb,
            &mut self.ground,
        ]
    }
}

impl Bank {
    /// A ground's skid loop: its own when the audio has it, else asphalt's.
    fn skid_for(&self, id: u16) -> &StaticSoundData {
        self.skids.get(&id).unwrap_or(&self.skid)
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
    /// The playing (or loading) song's own tempo, from `music.json`; 0 unknown.
    music_bpm: f64,
    /// The scrape, the engine's class, the mask theme and the voices: [`Extras`].
    extras: Extras,
}

/// The wall scrape loop, the engine's class, the mask theme and the voice lines.
#[derive(Default)]
struct Extras {
    scrape: Option<StaticSoundHandle>,
    /// The class whose engine sample the engine loop plays.
    engine_class: Option<crate::Class>,
    /// This ride's race track, read in once (to start again when the mask is off), and the
    /// mask theme playing instead of it: Aku Aku's (`true`) or Uka Uka's.
    track: Option<&'static Track>,
    track_data: Option<StaticSoundData>,
    /// The mask worn, and whether its theme took the music over.
    mask: Option<(bool, bool)>,
    /// Aku Aku's and Uka Uka's themes, read once (`None` inside: unreadable).
    themes: [Option<Option<StaticSoundData>>; 2],
    /// The voice lines read so far (`None`: unreadable).
    voices: std::collections::HashMap<PathBuf, Option<StaticSoundData>>,
}

impl Extras {
    fn stop(&mut self) {
        if let Some(mut h) = self.scrape.take() {
            h.stop(tween(250));
        }
        self.engine_class = None;
        self.track = None;
        self.track_data = None;
        self.mask = None;
    }

    /// `Audio_SetMaskSong` (`HOWL_AudioState.c:142-190`): while we wear the mask its theme
    /// (song 1 or 2 of the track's CSEQ) plays instead of the race track, and the race track
    /// starts over once the mask is off (`Music_Adjust` restarts song 0).
    fn mask_music(
        &mut self,
        theme: Option<bool>,
        music: &mut Option<StaticSoundHandle>,
        play: &mut impl FnMut(StaticSoundData) -> Option<StaticSoundHandle>,
    ) {
        let Some(track) = self.track else {
            return;
        };
        if self.mask.map(|(aku, _)| aku) == theme {
            return;
        }
        let swapped = self.mask.take().is_some_and(|(_, swapped)| swapped);
        let mut switch = |data: &StaticSoundData| {
            if let Some(mut m) = music.take() {
                m.stop(tween(300));
            }
            *music = play(data.clone().loop_region(..).volume(Decibels::SILENCE));
        };
        match theme {
            Some(aku) => {
                let file = if aku { &track.aku } else { &track.uka };
                let data = self.themes[usize::from(!aku)].get_or_insert_with(|| {
                    file.as_ref()
                        .and_then(|f| StaticSoundData::from_file(f).ok())
                });
                if let Some(data) = data {
                    switch(data);
                }
                self.mask = Some((aku, data.is_some()));
            }
            None if swapped => {
                if let Some(data) = &self.track_data {
                    switch(data);
                }
            }
            None => {}
        }
    }
}

/// A CTR race track rendered from the player's disc (`kart-data/music/music.json`), and the
/// mask themes played over it (`aku_file`, `uka_file`; kart data converted before they were
/// has none).
pub struct Track {
    pub name: String,
    pub file: PathBuf,
    /// Its tempo as the sequence is written (`CseqSongHeader.bpm`); 0 unknown.
    pub bpm: f64,
    pub aku: Option<PathBuf>,
    pub uka: Option<PathBuf>,
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
                let theme = |key: &str| {
                    t[key]
                        .as_str()
                        .map(|f| root.join(f))
                        .filter(|f| f.is_file())
                };
                Some(Track {
                    name: t["track"].as_str()?.to_string(),
                    file: root.join(t["file"].as_str()?),
                    bpm: t["bpm"].as_f64().unwrap_or(0.0),
                    aku: theme("aku_file"),
                    uka: theme("uka_file"),
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
                self.extras.stop();
            }
            audio.voices.clear();
            return;
        }
        // The race track: read on its own thread, started when it is in, looped.
        if !self.driving && music_on() {
            if let Some(track) = pick_track() {
                info!("kart music: {} ({} BPM)", track.name, track.bpm);
                self.music_bpm = track.bpm;
                self.extras.track = Some(track);
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
                self.extras.track_data = Some(data.clone());
                if !self.extras.mask.is_some_and(|(_, swapped)| swapped) {
                    self.music = play(data.loop_region(..).volume(Decibels::SILENCE));
                }
            }
        }
        self.extras
            .mask_music(audio.mask_theme, &mut self.music, &mut play);
        if let Some(m) = self.music.as_mut() {
            m.set_volume(db(0.8 * audio.music), tween(400));
            // CTR sets the new tempo at once; a short tween only keeps the change from clicking.
            m.set_playback_rate(tempo_rate(self.music_bpm, audio.music_bpm_delta), tween(60));
        }
        let bank: &Bank = self.bank.get_or_insert_with(Bank::load);
        let class = audio.engine_class;
        let engine_sample = &bank.engines[crate::Class::ALL
            .iter()
            .position(|c| *c == class)
            .unwrap_or(0)];
        if self.driving && self.extras.engine_class != Some(class) {
            // The garage changed the class: its engine from now on.
            if let Some(mut h) = self.loops.engine.take() {
                h.stop(tween(60));
            }
            self.loops.engine = play(
                engine_sample
                    .clone()
                    .loop_region(..)
                    .volume(Decibels::SILENCE),
            );
        }
        self.extras.engine_class = Some(class);
        if !self.driving {
            let mut start = |data: &StaticSoundData| {
                play(data.clone().loop_region(..).volume(Decibels::SILENCE))
            };
            self.extras.scrape = start(&bank.wall);
            self.loops = Loops {
                engine: start(engine_sample),
                skid: start(&bank.skid),
                roar: start(&bank.roar),
                roulette: bank.roulette.as_ref().and_then(&mut start),
                missile: bank.missile.as_ref().and_then(&mut start),
                warp: bank.warp.as_ref().and_then(&mut start),
                bomb: bank.bomb.as_ref().and_then(&mut start),
                ground: None,
                skid_id: Some(17),
                ground_id: None,
            };
            self.driving = true;
        }
        // The ground's own skid and rolling loops (`terrain->skidSound`, `terrain->sound`),
        // swapped in as it changes; a ground with neither is silent (`VehEmitter.c:1187-1267`).
        let ground = audio.ground;
        let want_skid = ground.skid_sound();
        if want_skid != self.loops.skid_id {
            if let Some(mut h) = self.loops.skid.take() {
                h.stop(tween(60));
            }
            self.loops.skid = want_skid.and_then(|id| {
                play(
                    bank.skid_for(id)
                        .clone()
                        .loop_region(..)
                        .volume(Decibels::SILENCE),
                )
            });
            self.loops.skid_id = want_skid;
        }
        let want_ground = ground
            .rolling_sound()
            .filter(|id| bank.rolling.contains_key(id));
        if want_ground != self.loops.ground_id {
            if let Some(mut h) = self.loops.ground.take() {
                h.stop(tween(120));
            }
            self.loops.ground = want_ground.and_then(|id| {
                play(
                    bank.rolling[&id]
                        .clone()
                        .loop_region(..)
                        .volume(Decibels::SILENCE),
                )
            });
            self.loops.ground_id = want_ground;
        }

        // The engine as CTR mixes it (`Kart::engine_sound`, from `EngineSound_Player`): the
        // sample at 2^((d − 128) / 64) times its own rate (`distortConst_Engine`), at the game's
        // base volume 153 scaled by the swell. Skid and turbo at their base volumes too.
        let (d, volume) = audio.engine;
        let engine = (
            0.6 * f32::from(volume) / 230.0,
            f64::from(((f32::from(d) - 128.0) / 64.0).exp2()),
        );
        // CTR's speed units from lu/s.
        let ctr_speed = (audio.speed * 256.0 / 30.0) as i32;
        let skid = (skid_level(audio.skidding, audio.grounded, ctr_speed), 1.0);
        let rolling = rolling_mix(audio.grounded, ctr_speed);
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
        let levels = [engine, skid, roar]
            .into_iter()
            .chain(weapons)
            .chain([rolling]);
        for (handle, (level, rate)) in self.loops.all().into_iter().zip(levels) {
            if let Some(h) = handle.as_mut() {
                h.set_volume(db(level * sfx), tween(60));
                h.set_playback_rate(rate, tween(60));
            }
        }
        // The scrape loop at CTR's `engineVol` for it (`VehEmitter.c:1393-1404`).
        if let Some(h) = self.extras.scrape.as_mut() {
            h.set_volume(
                db(0.6 * f32::from(audio.wall_volume) / 255.0 * sfx),
                tween(60),
            );
        }
        for file in audio.voices.drain(..) {
            let data = self
                .extras
                .voices
                .entry(file)
                .or_insert_with_key(|f| StaticSoundData::from_file(f).ok());
            if let Some(data) = data {
                let _ = play(data.clone().volume(db(sfx)));
            }
        }

        for cue in cues {
            let (data, gain) = match cue {
                Cue::Hop => (&bank.hop, 0.5),
                // The file is the loudest landing (0xFA); softer ones scale it.
                Cue::Land { volume } => (&bank.land, f32::from(volume) / 250.0),
                Cue::Bump(volume) => (&bank.bumps[bump_sample(volume)], f32::from(volume) / 255.0),
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
    fn the_final_lap_adds_twenty_beats_to_each_song_s_own_tempo() {
        // Crash Cove at 190 BPM, Dragon Mines at 229: each 20 BPM up from its own.
        assert!((tempo_rate(190.0, 20.0) - 210.0 / 190.0).abs() < 1e-12);
        assert!((tempo_rate(229.0, 20.0) - 249.0 / 229.0).abs() < 1e-12);
        assert!(
            tempo_rate(148.0, 20.0) > tempo_rate(229.0, 20.0),
            "slower songs speed up more"
        );
        // As written off the final lap, and a song of unknown tempo never changes.
        assert_eq!(tempo_rate(190.0, 0.0), 1.0);
        assert_eq!(tempo_rate(0.0, 20.0), 1.0);
    }

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
    fn the_skid_and_the_ground_play_as_vehemitter_mixes_them() {
        // A skid at speed on the ground, none in the air, none crawling, none without a skid.
        assert_eq!(skid_level(true, true, 12000), 0.5);
        assert_eq!(skid_level(true, false, 12000), 0.0);
        assert_eq!(skid_level(true, true, 0x200), 0.0);
        assert_eq!(skid_level(false, true, 12000), 0.0);
        assert!(skid_level(true, true, 3000) < skid_level(true, true, 9000));
        // The rolling loop: silent standing, full by 5000, rising in pitch to 0xD2.
        assert_eq!(rolling_mix(true, 0).0, 0.0);
        assert!((rolling_mix(true, 5000).0 - 0.6 * 200.0 / 255.0).abs() < 1e-6);
        assert_eq!(rolling_mix(false, 5000).0, 0.0);
        assert!(rolling_mix(true, 0).1 < 1.0 && rolling_mix(true, 12000).1 > 1.0);
        // Each ground's loops have files: grass skids on dirt's, mud has no skid.
        assert_eq!(
            ctr_kart::Ground::Grass.skid_sound().and_then(skid_file),
            Some("skid_dirt")
        );
        assert_eq!(
            ctr_kart::Ground::Grass
                .rolling_sound()
                .and_then(rolling_file),
            Some("ground_dirt")
        );
        assert_eq!(ctr_kart::Ground::Mud.skid_sound(), None);
        for g in ctr_kart::Ground::ALL {
            if let Some(id) = g.skid_sound() {
                assert!(skid_file(id).is_some(), "{g:?}");
            }
            // Wood and the track's edge roll on CTR's one-shot path, which is not looped.
            if let Some(id) = g.rolling_sound() {
                assert!(rolling_file(id).is_some(), "{g:?}");
            }
        }
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
                Cue::Land { volume: 0x78 },
                Cue::Land { volume: 0xDD },
                Cue::Boost(2),
                Cue::Crash
            ]
        );
    }

    #[test]
    fn landings_and_kart_crashes_sound_at_ctr_s_volumes() {
        // COLL.c:1508: 0x78 at 0x100 or softer, 0xFA at 0x3C00 or harder, linear between.
        assert_eq!(landing_volume(0), 0x78);
        assert_eq!(landing_volume(-0x100), 0x78);
        assert_eq!(landing_volume(0x3C00), 0xFA);
        assert_eq!(landing_volume(99_999), 0xFA);
        assert_eq!(landing_volume(0x1E80), 0x78 + (0xFA - 0x78) / 2);
        // VehPhysCrash.c:529-533: silent up to 0x200, then 0x3F..0xFF over 0..0x1900.
        assert_eq!(bump_volume(0x200), None);
        assert_eq!(bump_volume(0x1900), Some(0xFF));
        assert_eq!(
            bump_volume(0x201),
            Some(0x3F + (0x201 * 0xC0 / 0x1900) as u8)
        );
        // OtherFX_DriverCrashing's three samples.
        assert_eq!(bump_sample(0xA0), 0);
        assert_eq!(bump_sample(0xA1), 1);
        assert_eq!(bump_sample(0xDC), 1);
        assert_eq!(bump_sample(0xDD), 2);

        // One crash sound in three frames.
        let mut a = KartAudio::default();
        a.observe(&Event::Bump { strength: 0x800 });
        a.observe(&Event::Bump { strength: 0x800 });
        assert_eq!(a.cues.len(), 1);
        let frame = crate::session::Feedback {
            frames: 3,
            ..Default::default()
        };
        a.tick(&frame, false);
        a.observe(&Event::Bump { strength: 0x100 });
        a.observe(&Event::Bump { strength: 0x1900 });
        assert_eq!(a.cues[1], Cue::Bump(0xFF));
    }

    #[test]
    fn the_wall_scrape_swells_while_scraping_and_fades_after() {
        let mut a = KartAudio::default();
        let scrape = crate::session::Feedback {
            frames: 1,
            wall_frames: 1,
            landing: None,
        };
        let off = crate::session::Feedback {
            frames: 1,
            ..Default::default()
        };
        for _ in 0..3 {
            a.tick(&scrape, false);
        }
        assert_eq!(a.wall_volume, 3 * 0x14);
        for _ in 0..20 {
            a.tick(&scrape, false);
        }
        assert_eq!(a.wall_volume, 0xFF);
        a.tick(&off, false);
        assert_eq!(a.wall_volume, 0xFF - 0x14);
        for _ in 0..20 {
            a.tick(&off, false);
        }
        assert_eq!(a.wall_volume, 0);
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
