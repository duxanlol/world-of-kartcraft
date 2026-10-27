//! Our racer's voice lines, as CTR's `HOWL_Voiceline.c` picks and paces them: the events that
//! speak ([`line_for`]), the gates and queue of `Voiceline_RequestPlay`, and the queued line
//! started once the last one's cooldown is out (`Voiceline_Update`, `Voiceline_StartPlay`). The
//! lines are the player's own, converted at setup into `kart-data/audio/voice/` (`voices.json`):
//! two short ones per racer from the sound bank, played at once, and the CD-XA lines, queued.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

/// `data.voiceID` (`zGlobal_DATA.c:3369`): the voice set each of the 24 voice line ids says.
pub const VOICE_SET: [u8; 24] = [
    4, 1, 5, 2, 1, 1, 1, 3, 6, 6, 7, 7, 7, 7, 7, 4, 0, 0, 0, 0, 0, 8, 8, 8,
];

/// The voice line ids the kart's events and our weapons ask for (callers in the decompilation).
pub mod line {
    /// `VehPickState.c:220` common damage; `VehPhysCrash.c:212` a crash.
    pub const DAMAGE: u8 = 1;
    /// `VehPickState.c:112`: a hit the mask or the shield shrugged off, the victim laughs.
    pub const LAUGH: u8 = 2;
    /// `VehPhysProc.c:2873` (`VEH_PHYS_PROC_SPIN_VOICELINE_ID`): a spin begins.
    pub const SPIN: u8 = 3;
    /// `VehPickState.c:231` (`VEH_PICK_VOICELINE_SQUISH`).
    pub const SQUISH: u8 = 4;
    /// `VehPhysCrash.c:543` (`VEH_PHYS_CRASH_VOICELINE_HARD_CRASH`): a kart crash louder than
    /// 0xDC.
    pub const HARD_CRASH: u8 = 5;
    /// `COLL.c:2634`: a head-on wall slam.
    pub const WALL_CRASH: u8 = 6;
    /// `UI_Meter.c:110` (`UI_JUMP_METER_LANDING_VOICELINE_ID`): landing a long hang time.
    pub const HANG_TIME: u8 = 7;
    /// `PlayLevel.c:411` (`PLAYLEVEL_PASS_VOICELINE`): a racer passes a player.
    pub const OVERTAKE: u8 = 8;
    /// `VehPickupItem.c:158-162`: weapons fired.
    pub const BOMB: u8 = 10;
    pub const MISSILE: u8 = 11;
    pub const WARP_ORB: u8 = 12;
    pub const CLOCK: u8 = 14;
    pub const MINE: u8 = 15;
    /// `VehFire.c:61` (`VEH_FIRE_VOICELINE_HIGH_BOOST_ID`): a boost at fire level 0x80 or more.
    pub const BIG_BOOST: u8 = 16;
}

/// `UI_JUMP_METER_LANDING_VOICELINE_MIN` (`UI_Meter.c:13`): the hang time (ms) a landing needs
/// to speak.
pub const HANG_TIME_VOICE_MS: i32 = 0x480;

/// The voice line an event of our kart asks for.
pub fn line_for(event: &crate::Event) -> Option<u8> {
    use crate::Event;
    use ctr_kart::HitKind;
    Some(match *event {
        Event::SpinOut => line::SPIN,
        Event::Hit {
            kind: HitKind::Blasted,
        } => line::DAMAGE,
        Event::Hit {
            kind: HitKind::Squished,
        } => line::SQUISH,
        Event::HitBlocked => line::LAUGH,
        Event::Slam { .. } => line::WALL_CRASH,
        // `VehFire_Audio`'s high branch: the third powerslide boost (fire level 0xC0) and a
        // medium or large hang time (0x80, 0x100; `UI_Meter.c:10-12`).
        Event::Boost { stage, .. } if stage >= 3 => line::BIG_BOOST,
        Event::HangTime { level } if level >= 2 => line::BIG_BOOST,
        Event::Land { air_ms, .. } if air_ms > HANG_TIME_VOICE_MS => line::HANG_TIME,
        _ => return None,
    })
}

/// `PLAYLEVEL_PASS_VOICELINE_DELAY` (`PlayLevel.c:22`): no overtake speaks in a race's first
/// 0x4B00 ms.
pub const OVERTAKE_AFTER_MS: f32 = 0x4B00 as f32;

/// `PlayLevel.c:391-415`: a player's place went from `before` to `now` (1 the leader) `race_ms`
/// into the race. When a racer passes a player (the player's place number grew) the racer now
/// one ahead of them speaks; CTR checks every player, so a player we pass hears us from their
/// side, and here we hear it from ours. The places of whoever speaks: `Passed(place)` the racer
/// now at `place` passed us, `Passing` we passed someone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overtake {
    Passed(usize),
    Passing,
}

pub fn overtake(before: usize, now: usize, race_ms: f32) -> Option<Overtake> {
    if race_ms <= OVERTAKE_AFTER_MS || before == 0 || now == 0 || before == now {
        return None;
    }
    Some(if now > before {
        Overtake::Passed(now - 1)
    } else {
        Overtake::Passing
    })
}

/// A racer's lines as `voices.json` lists them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lines {
    /// The bank's voice type 0 and 1 lines (`OtherFX 0x1C + c`, `0x2C + c`).
    pub bank: [Option<PathBuf>; 2],
    /// Each voice set's XA lines and their sectors (`XaBytes`, for the cooldown).
    pub sets: Vec<Vec<(PathBuf, u16)>>,
}

impl Lines {
    /// `racer`'s lines from `<root>/voices.json`, else Crash's; `None` without the file.
    pub fn load(root: &Path, racer: &str) -> Option<Self> {
        let data = std::fs::read(root.join("voices.json")).ok()?;
        let doc: serde_json::Value = serde_json::from_slice(&data).ok()?;
        let r = doc["racers"]
            .get(racer)
            .filter(|r| r.is_object())
            .or_else(|| doc["racers"].get("crash"))?;
        let file = |v: &serde_json::Value| v.as_str().map(|f| root.join(f));
        let bank = [file(&r["bank"][0]), file(&r["bank"][1])];
        let sets = r["sets"]
            .as_array()?
            .iter()
            .map(|set| {
                set.as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|l| {
                        Some((file(&l["file"])?, l["sectors"].as_u64().unwrap_or(0) as u16))
                    })
                    .collect()
            })
            .collect();
        Some(Self { bank, sets })
    }
}

/// `Voiceline_RequestPlay_NextAudioRNG` (`HOWL_Voiceline.c:126-130`).
fn next(rng: &mut u32) -> u32 {
    *rng = (rng.wrapping_shr(3))
        .wrapping_add(rng.wrapping_mul(0x2000_0000))
        .wrapping_mul(5)
        .wrapping_add(1);
    *rng
}

/// The racers' voices as one channel, as CTR's is: the queue of lines asked for, each racer's
/// frames (30 Hz) since their last bank line, and the cooldown of the line playing.
#[derive(Clone, Debug)]
pub struct Voice {
    rng: u32,
    /// CTR's game frames.
    frame: u64,
    /// `timeSet2[character]`: the frame of a racer's last immediate (bank) line.
    last_bank: HashMap<String, u64>,
    /// `timeSet1[character]`: the lines a racer has had queued at least once.
    queued_once: HashMap<String, u32>,
    /// `voicelineCooldown`, frames, and whose line it is (`backupParams`' character).
    cooldown: u32,
    speaking: Option<String>,
    /// `Voiceline2`: (line, racer), newest first, at most 8.
    queue: VecDeque<(u8, String)>,
    /// Where `voices.json` is (`None` unknown yet, `Some(None)` nowhere), and each racer's
    /// lines read from it.
    root: Option<Option<PathBuf>>,
    lines: HashMap<String, Lines>,
}

impl Default for Voice {
    fn default() -> Self {
        Self::seeded(0x1234_5678)
    }
}

/// A line to play: its file.
pub type Say = PathBuf;

impl Voice {
    pub fn seeded(seed: u32) -> Self {
        Self {
            rng: seed,
            frame: 0,
            last_bank: HashMap::new(),
            queued_once: HashMap::new(),
            cooldown: 0,
            speaking: None,
            queue: VecDeque::new(),
            root: None,
            lines: HashMap::new(),
        }
    }

    /// Where the converted voices are (`<kart-data>/audio/voice`), for the lines read on demand.
    pub fn set_root(&mut self, root: Option<PathBuf>) {
        self.root = Some(root);
        self.lines.clear();
    }

    /// Whether the voices' place has been set.
    pub fn has_root(&self) -> bool {
        self.root.is_some()
    }

    /// Gives `racer` these lines (a test's, or lines read elsewhere).
    pub fn insert_lines(&mut self, racer: &str, lines: Lines) {
        self.lines.insert(racer.to_string(), lines);
    }

    /// `racer`'s lines (Crash's for a racer with none, `""` too), read once.
    fn lines(&mut self, racer: &str) -> &Lines {
        let root = self.root.clone().flatten();
        self.lines.entry(racer.to_string()).or_insert_with(|| {
            root.and_then(|r| Lines::load(&r, if racer.is_empty() { "crash" } else { racer }))
                .unwrap_or_default()
        })
    }

    /// `Voiceline_RequestPlay` (`HOWL_Voiceline.c:132-262`) for `racer`: a bank line to play
    /// now, or the line queued, or nothing.
    pub fn request(&mut self, id: u8, racer: &str) -> Option<Say> {
        let set = *VOICE_SET.get(usize::from(id))?;
        // Lines 8 and up speak one time in 4, one in 8 once they have been queued.
        if id >= 8 {
            let once = self.queued_once.get(racer).copied().unwrap_or(0);
            let r = next(&mut self.rng);
            let gate = if once & (1 << id) != 0 { r & 7 } else { r & 3 };
            if gate != 0 {
                return None;
            }
        }
        let frame = self.frame;
        let elapsed = self.last_bank.get(racer).map_or(u64::MAX, |at| frame - at);
        let bank = (set < 2)
            .then(|| self.lines(racer).bank[usize::from(set)].clone())
            .flatten();
        let mut immediate = elapsed >= 0x3D && bank.is_some();
        // A racer does not queue over their own line still cooling down.
        let queue =
            !(self.cooldown != 0 && self.speaking.as_deref() == Some(racer)) && elapsed >= 0x3C;
        if queue && immediate {
            immediate = next(&mut self.rng) & 1 != 0;
        } else if !queue && !immediate {
            return None;
        }
        if immediate {
            self.last_bank.insert(racer.to_string(), frame);
            return bank;
        }
        *self.queued_once.entry(racer.to_string()).or_default() |= 1 << id;
        if !self.queue.iter().any(|(i, r)| *i == id && r == racer) {
            self.queue.push_front((id, racer.to_string()));
            self.queue.truncate(8);
        }
        None
    }

    /// One game frame (`Voiceline_Update`): the cooldown counts down, then the newest queued
    /// line starts (`Voiceline_StartPlay`), one of its set at random, and its length in sectors
    /// / 5 + 30 frames is the next cooldown.
    pub fn frame(&mut self) -> Option<Say> {
        self.frame += 1;
        if self.cooldown != 0 {
            self.cooldown -= 1;
            if self.cooldown != 0 {
                return None;
            }
        }
        let (id, racer) = self.queue.pop_front()?;
        let set = self
            .lines(&racer)
            .sets
            .get(usize::from(VOICE_SET[usize::from(id)]))
            .cloned()?;
        if set.is_empty() {
            self.queue.clear();
            return None;
        }
        let (file, sectors) = set[next(&mut self.rng) as usize % set.len()].clone();
        self.cooldown = u32::from(sectors) / 5 + 0x1E;
        self.speaking = Some(racer);
        Some(file)
    }
}

/// What a racer says for a CTR voice line in their WoW character's own voice (Kartcraft's own:
/// the racers are WoW characters, not CTR's cast). The hits and crashes are the character's
/// wound vocal, the hard ones its critical; the rest are its text-emote voices, by `/command`,
/// among those `EmotesTextSound.dbc` voices for every race and sex (1.12's `/laugh` and `/roar`
/// sound from the body's animation instead, which a seated driver does not play).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WowLine {
    Injury,
    InjuryCrit,
    Emote(&'static str),
}

/// The text emotes [`wow_line`] speaks with.
pub const EMOTES: [&str; 7] = [
    "thank", "bye", "cheer", "openfire", "charge", "incoming", "wait",
];

/// The WoW line for CTR's voice line `id`.
pub fn wow_line(id: u8) -> Option<WowLine> {
    Some(match id {
        line::DAMAGE | line::SPIN => WowLine::Injury,
        line::SQUISH | line::HARD_CRASH | line::WALL_CRASH => WowLine::InjuryCrit,
        // A hit the mask or the shield shrugged off: CTR's victim laughs.
        line::LAUGH => WowLine::Emote("thank"),
        line::OVERTAKE => WowLine::Emote("bye"),
        line::HANG_TIME | line::BIG_BOOST => WowLine::Emote("cheer"),
        line::BOMB | line::MISSILE => WowLine::Emote("openfire"),
        line::WARP_ORB => WowLine::Emote("charge"),
        line::MINE => WowLine::Emote("incoming"),
        line::CLOCK => WowLine::Emote("wait"),
        _ => return None,
    })
}

/// The WoW lines' pacing, CTR's own gates kept: lines 8 and up speak one time in 4
/// (`HOWL_Voiceline.c:144-150`), and a racer speaks again only 0x3D frames after their last line
/// (the bank lines' gap, `:156`). A speaker is `None` for us, else their guid.
#[derive(Clone, Debug)]
pub struct WowPace {
    rng: u32,
    frame: u64,
    last: HashMap<Option<u64>, u64>,
}

impl Default for WowPace {
    fn default() -> Self {
        Self {
            rng: 0x1234_5678,
            frame: 0,
            last: HashMap::new(),
        }
    }
}

impl WowPace {
    /// Whether `who` says line `id` now.
    pub fn request(&mut self, id: u8, who: Option<u64>) -> bool {
        if id >= 8 && next(&mut self.rng) & 3 != 0 {
            return false;
        }
        if self.last.get(&who).is_some_and(|at| self.frame - at < 0x3D) {
            return false;
        }
        self.last.insert(who, self.frame);
        true
    }

    /// CTR's game frames go by.
    pub fn advance(&mut self, frames: u32) {
        self.frame += u64::from(frames);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Event;

    #[test]
    fn ctr_lines_become_the_character_s_wow_voice() {
        assert_eq!(wow_line(line::DAMAGE), Some(WowLine::Injury));
        assert_eq!(wow_line(line::WALL_CRASH), Some(WowLine::InjuryCrit));
        assert_eq!(wow_line(line::BIG_BOOST), Some(WowLine::Emote("cheer")));
        assert_eq!(wow_line(line::OVERTAKE), Some(WowLine::Emote("bye")));
        assert_eq!(wow_line(line::MISSILE), Some(WowLine::Emote("openfire")));
        // Every line an event or a weapon asks for has a WoW voice, its emote listed.
        for id in [1, 2, 3, 4, 5, 6, 7, 8, 10, 11, 12, 14, 15, 16] {
            match wow_line(id) {
                Some(WowLine::Emote(e)) => assert!(EMOTES.contains(&e), "{e}"),
                Some(_) => {}
                None => panic!("line {id}"),
            }
        }
    }

    #[test]
    fn wow_lines_keep_ctr_s_gap_per_speaker_and_its_roll_for_the_common_ones() {
        let mut pace = WowPace::default();
        assert!(pace.request(line::DAMAGE, None));
        // Too soon for us; another racer is free to speak.
        assert!(!pace.request(line::SPIN, None));
        assert!(pace.request(line::SPIN, Some(7)));
        pace.advance(0x3C);
        assert!(!pace.request(line::DAMAGE, None));
        pace.advance(1);
        assert!(pace.request(line::DAMAGE, None));
        // Lines 8 and up: about one time in four, never back to back within the gap.
        let mut spoke = 0;
        for _ in 0..400 {
            pace.advance(0x3D);
            spoke += u32::from(pace.request(line::BIG_BOOST, None));
        }
        assert!((60..140).contains(&spoke), "{spoke} of 400");
    }

    fn lines() -> Lines {
        Lines {
            bank: [Some("fx0.wav".into()), Some("fx1.wav".into())],
            sets: (0..11)
                .map(|k| vec![(PathBuf::from(format!("{k}_0.wav")), 150)])
                .collect(),
        }
    }

    #[test]
    fn events_ask_for_ctr_s_lines() {
        assert_eq!(line_for(&Event::SpinOut), Some(line::SPIN));
        assert_eq!(
            line_for(&Event::Slam { speed: 9000 }),
            Some(line::WALL_CRASH)
        );
        assert_eq!(
            line_for(&Event::Boost {
                stage: 3,
                reserves: 1
            }),
            Some(line::BIG_BOOST)
        );
        assert_eq!(
            line_for(&Event::Boost {
                stage: 2,
                reserves: 1
            }),
            None
        );
        assert_eq!(
            line_for(&Event::Land {
                air_ms: 1200,
                fall: 0,
                water: false
            }),
            Some(line::HANG_TIME)
        );
        assert_eq!(
            line_for(&Event::Land {
                air_ms: 600,
                fall: 0,
                water: false
            }),
            None
        );
        assert_eq!(line_for(&Event::HitBlocked), Some(line::LAUGH));
        // The set table: damage and crashes are bank-able (set 1), the boost yell set 0.
        assert_eq!(VOICE_SET[usize::from(line::DAMAGE)], 1);
        assert_eq!(VOICE_SET[usize::from(line::BIG_BOOST)], 0);
        assert_eq!(VOICE_SET[usize::from(line::SPIN)], 2);
    }

    /// A voice with Crash's and Cortex's test lines.
    fn voice(seed: u32) -> Voice {
        let mut v = Voice::seeded(seed);
        v.set_root(None);
        v.insert_lines("crash", lines());
        v.insert_lines("cortex", lines());
        v
    }

    #[test]
    fn a_line_plays_at_once_or_queues_and_waits_its_cooldown() {
        let mut v = voice(7);
        let mut said = Vec::new();
        // The damage line: a bank line at once, or queued and started next frame.
        if let Some(f) = v.request(line::DAMAGE, "crash") {
            said.push(f);
        }
        for _ in 0..200 {
            if let Some(f) = v.frame() {
                said.push(f);
            }
        }
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(said[0] == Path::new("fx1.wav") || said[0] == Path::new("1_0.wav"));

        // An XA line's cooldown is its sectors / 5 + 30 frames: 60 for 150 sectors. The speaker
        // queues nothing meanwhile; another racer may.
        let mut v = voice(1);
        v.queue.push_front((line::SPIN, "crash".into()));
        assert_eq!(v.frame(), Some(PathBuf::from("2_0.wav")));
        assert_eq!(v.cooldown, 60);
        assert_eq!(v.request(line::SPIN, "crash"), None);
        assert!(v.queue.is_empty());
        v.request(line::SPIN, "cortex");
        assert_eq!(v.queue.len(), 1);
        v.queue.clear();
        for _ in 0..59 {
            assert_eq!(v.frame(), None);
        }
        assert_eq!(v.cooldown, 1);
        v.request(line::SPIN, "crash");
        assert!(v.queue.is_empty(), "still cooling down");
        assert_eq!(v.frame(), None);
        assert_eq!(v.cooldown, 0);
        v.request(line::SPIN, "crash");
        assert_eq!(v.queue.len(), 1);
    }

    #[test]
    fn weapon_lines_speak_one_time_in_four() {
        let mut spoke = 0;
        let mut v = voice(99);
        for _ in 0..4000 {
            v.request(line::MISSILE, "crash");
            if !v.queue.is_empty() {
                spoke += 1;
                v.queue.clear();
                // Once queued, one in eight.
                v.queued_once.clear();
            }
        }
        assert!((800..1200).contains(&spoke), "{spoke} of 4000");
    }

    #[test]
    fn overtakes_speak_after_the_race_s_first_seconds() {
        // Passed from 2nd to 3rd: the racer now 2nd speaks; passing: we do.
        assert_eq!(overtake(2, 3, 30_000.0), Some(Overtake::Passed(2)));
        assert_eq!(overtake(3, 1, 30_000.0), Some(Overtake::Passing));
        assert_eq!(overtake(2, 3, 19_000.0), None);
        assert_eq!(overtake(2, 2, 30_000.0), None);
        assert_eq!(overtake(0, 2, 30_000.0), None);
        // The pass line is set 6, one time in four.
        assert_eq!(VOICE_SET[usize::from(line::OVERTAKE)], 6);
    }
}
