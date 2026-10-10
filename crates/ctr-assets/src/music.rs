//! CTR race music. In-race music is **not** CD-XA: CTR sequences it on the SPU from CSEQ
//! files stored in `SOUNDS\KART.HWL` (the XA files on the disc only hold jingles, voice lines
//! and race start/win/lose stings). This module renders a CSEQ song to PCM with a small SPU
//! model (sample playback at SPU pitch, ADSR envelopes, per-voice volume/pan), following
//! CTR-native's `HOWL_CseqOpcode.c`, `HOWL_Channel.c` (`Channel_ParseSongToChannels`),
//! `HOWL_SongPool.c` and `HOWL_Load.c` (`howl_InstrumentPitch`).
//!
//! CSEQ layout (`include/namespace_Howl.h`):
//! ```text
//! CseqHeader { u32 size; u8 num_long; u8 num_short; u16 num_songs }
//! SampleInstrument[num_long] { u8 1; u8 volume; s16 0; u16 base_pitch; s16 spu_index; u16 ad; u16 sr }
//! SampleDrums[num_short]     { u8 1; u8 volume; s16 pitch; s16 spu_index; s16 0 }
//! s16 song_offset[num_songs]; (align 4) song data
//! CseqSongHeader { u8 unk; u8 num_seqs; s16 bpm; s16 tpqn; u16 seq_offset[num_seqs] } (align 4)
//! SongNoteHeader { u8 flags (1 = drums); u8 unk } then events: varlen delay, opcode, args
//! ```
//! Opcodes: 1 note off (note), 3 end of sequence (loops), 5 note on (note, velocity),
//! 6 sequence volume, 7 pan, 8 reverb, 9 instrument, 0x0A pitch bend; arg bytes per
//! `opcodeOffset = {1,2,2,1,2,3,2,2,2,2,2}` (opcode byte included).

use crate::audio::{decode_adpcm, Howl};
use crate::{Error, Result};

/// The SPU's mixing rate; envelopes and pitch are defined against it.
pub const SPU_RATE: u32 = 44100;
/// Default export rate: half the SPU rate keeps a 90 s stereo track at ~8 MB.
pub const OUT_RATE: u32 = 22050;

/// Race tracks: level id order (`enum LevelID`), with the HOWL sequence each level loads
/// (`data.levBank_Song[level]`, `zGlobal_DATA.c`). Song 0 of that CSEQ is the level music
/// (`CSEQ_SONG_LEVEL`); 1 and 2 are the Aku/Uka themes.
pub const RACE_TRACKS: [(&str, usize); 18] = [
    ("dingo_canyon", 0),
    ("dragon_mines", 1),
    ("blizzard_bluff", 2),
    ("crash_cove", 3),
    ("tiger_temple", 4),
    ("papu_pyramid", 5),
    ("roo_tubes", 6),
    ("hot_air_skyway", 7),
    ("sewer_speedway", 8),
    ("mystery_caves", 9),
    ("cortex_castle", 10),
    ("n_gin_labs", 11),
    ("polar_pass", 12),
    ("oxide_station", 13),
    ("coco_park", 14),
    ("tiny_arena", 15),
    ("slide_coliseum", 16),
    ("turbo_track", 17),
];

const OPCODE_SIZE: [usize; 11] = [1, 2, 2, 1, 2, 3, 2, 2, 2, 2, 2];

/// `data.volumeLR` (`zGlobal_DATA.c`): equal-power pan curve, index 0..255.
const VOLUME_LR: [u8; 256] = [
    0x00, 0x01, 0x03, 0x04, 0x06, 0x07, 0x09, 0x0A, 0x0C, 0x0E, 0x0F, 0x11, 0x12, 0x14, 0x15, 0x17,
    0x19, 0x1A, 0x1C, 0x1D, 0x1F, 0x20, 0x22, 0x24, 0x25, 0x27, 0x28, 0x2A, 0x2B, 0x2D, 0x2E, 0x30,
    0x31, 0x33, 0x35, 0x36, 0x38, 0x39, 0x3B, 0x3C, 0x3E, 0x3F, 0x41, 0x42, 0x44, 0x45, 0x47, 0x48,
    0x4A, 0x4B, 0x4D, 0x4E, 0x50, 0x51, 0x53, 0x54, 0x56, 0x57, 0x59, 0x5A, 0x5C, 0x5D, 0x5F, 0x60,
    0x61, 0x63, 0x64, 0x66, 0x67, 0x69, 0x6A, 0x6C, 0x6D, 0x6E, 0x70, 0x71, 0x73, 0x74, 0x75, 0x77,
    0x78, 0x7A, 0x7B, 0x7C, 0x7E, 0x7F, 0x80, 0x82, 0x83, 0x84, 0x86, 0x87, 0x88, 0x8A, 0x8B, 0x8C,
    0x8E, 0x8F, 0x90, 0x92, 0x93, 0x94, 0x95, 0x97, 0x98, 0x99, 0x9B, 0x9C, 0x9D, 0x9E, 0x9F, 0xA1,
    0xA2, 0xA3, 0xA4, 0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xAB, 0xAD, 0xAE, 0xAF, 0xB0, 0xB1, 0xB2, 0xB3,
    0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xBB, 0xBC, 0xBD, 0xBE, 0xBF, 0xC0, 0xC1, 0xC2, 0xC3, 0xC4,
    0xC5, 0xC6, 0xC7, 0xC8, 0xC9, 0xCA, 0xCB, 0xCC, 0xCD, 0xCE, 0xCF, 0xD0, 0xD1, 0xD2, 0xD3, 0xD3,
    0xD4, 0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xD9, 0xDA, 0xDB, 0xDC, 0xDD, 0xDD, 0xDE, 0xDF, 0xE0, 0xE1,
    0xE1, 0xE2, 0xE3, 0xE3, 0xE4, 0xE5, 0xE6, 0xE6, 0xE7, 0xE8, 0xE8, 0xE9, 0xEA, 0xEA, 0xEB, 0xEB,
    0xEC, 0xED, 0xED, 0xEE, 0xEE, 0xEF, 0xEF, 0xF0, 0xF1, 0xF1, 0xF2, 0xF2, 0xF3, 0xF3, 0xF4, 0xF4,
    0xF4, 0xF5, 0xF5, 0xF6, 0xF6, 0xF7, 0xF7, 0xF7, 0xF8, 0xF8, 0xF9, 0xF9, 0xF9, 0xFA, 0xFA, 0xFA,
    0xFB, 0xFB, 0xFB, 0xFB, 0xFC, 0xFC, 0xFC, 0xFC, 0xFD, 0xFD, 0xFD, 0xFD, 0xFE, 0xFE, 0xFE, 0xFE,
    0xFE, 0xFE, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
];

/// `sdata->vol_Music` default (`howl_InitGlobals`).
const VOL_MUSIC: i64 = 175;
/// Song pool volume for song 0 (`SongPool_Start`: songs 1 and 2 play at 0xFF, others 0xBE).
const VOL_SONG: i64 = 0xBE;

#[derive(Clone, Copy, Debug)]
pub struct LongSample {
    pub volume: u8,
    pub base_pitch: u16,
    pub spu_index: u16,
    pub ad: u16,
    pub sr: u16,
}

#[derive(Clone, Copy, Debug)]
pub struct ShortSample {
    pub volume: u8,
    pub pitch: u16,
    pub spu_index: u16,
}

pub struct SeqTrack {
    pub drums: bool,
    pub events: Vec<u8>,
}

pub struct Song {
    pub bpm: u16,
    pub tpqn: u16,
    pub seqs: Vec<SeqTrack>,
}

pub struct Cseq {
    pub long: Vec<LongSample>,
    pub short: Vec<ShortSample>,
    pub songs: Vec<Song>,
}

fn rd16(d: &[u8], at: usize) -> Result<u16> {
    d.get(at..at + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or_else(|| Error(format!("CSEQ truncated at {at:#x}")))
}

fn align4(x: usize) -> usize {
    (x + 3) & !3
}

impl Cseq {
    pub fn parse(d: &[u8]) -> Result<Self> {
        let n_long = *d.get(4).ok_or_else(|| Error("CSEQ truncated".into()))? as usize;
        let n_short = d[5] as usize;
        let n_songs = rd16(d, 6)? as usize;
        let mut at = 8;
        let mut long = Vec::new();
        for _ in 0..n_long {
            long.push(LongSample {
                volume: d[at + 1],
                base_pitch: rd16(d, at + 4)?,
                spu_index: rd16(d, at + 6)?,
                ad: rd16(d, at + 8)?,
                sr: rd16(d, at + 10)?,
            });
            at += 12;
        }
        let mut short = Vec::new();
        for _ in 0..n_short {
            short.push(ShortSample {
                volume: d[at + 1],
                pitch: rd16(d, at + 2)?,
                spu_index: rd16(d, at + 4)?,
            });
            at += 8;
        }
        let offsets: Vec<usize> = (0..n_songs)
            .map(|i| rd16(d, at + i * 2).map(|v| v as usize))
            .collect::<Result<_>>()?;
        let data = align4(at + n_songs * 2);
        let mut songs = Vec::new();
        for off in offsets {
            let h = data + off;
            let n = *d
                .get(h + 1)
                .ok_or_else(|| Error("CSEQ song truncated".into()))? as usize;
            let bpm = rd16(d, h + 2)?;
            let tpqn = rd16(d, h + 4)?;
            let first = align4(h + 6 + n * 2);
            let mut seqs = Vec::new();
            for i in 0..n {
                let s = first + rd16(d, h + 6 + i * 2)? as usize;
                let flags = *d
                    .get(s)
                    .ok_or_else(|| Error("CSEQ track truncated".into()))?;
                seqs.push(SeqTrack {
                    drums: flags & 1 != 0,
                    events: d[s + 2..].to_vec(),
                });
            }
            songs.push(Song { bpm, tpqn, seqs });
        }
        Ok(Self { long, short, songs })
    }
}

/// Reads a CSEQ variable-length delay (`howl_GetNextNote`).
fn varlen(d: &[u8], pos: &mut usize) -> u32 {
    let mut v = d.get(*pos).copied().unwrap_or(0) as u32;
    *pos += 1;
    if v & 0x80 != 0 {
        v &= 0x7F;
        loop {
            let b = d.get(*pos).copied().unwrap_or(0) as u32;
            *pos += 1;
            v = (v << 7) + (b & 0x7F);
            if b & 0x80 == 0 {
                break;
            }
        }
    }
    v
}

/// SPU pitch of an instrument note (`howl_InstrumentPitch`): `noteFrequency[note]` is
/// `0x1000 * 2^((note - 60) / 12)`; distortion 0x80 is neutral, 64 steps per semitone.
pub fn instrument_pitch(base_pitch: u16, note: u8, distort: u8) -> f64 {
    let semis = note as f64 - 60.0 + (distort as f64 - 128.0) / 64.0;
    base_pitch as f64 * 2f64.powf(semis / 12.0)
}

// ---------------------------------------------------------------------------------------------
// SPU voice model
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
enum Phase {
    Attack,
    Decay,
    Sustain,
    Release,
    Off,
}

/// PSX SPU ADSR (rates as 7-bit `shift << 2 | step`, as in the SPU documentation).
#[derive(Clone, Copy, Debug)]
pub struct Adsr {
    level: i32,
    counter: u32,
    phase: Phase,
    ad: u16,
    sr: u16,
}

impl Adsr {
    pub fn new(ad: u16, sr: u16) -> Self {
        Self {
            level: 0,
            counter: 0,
            phase: Phase::Attack,
            ad,
            sr,
        }
    }

    fn sustain_level(&self) -> i32 {
        (((self.ad & 0xF) as i32) + 1) * 0x800
    }

    /// (rate, exponential, decreasing) of the current phase.
    fn params(&self) -> (u32, bool, bool) {
        match self.phase {
            Phase::Attack => (((self.ad >> 8) & 0x7F) as u32, self.ad & 0x8000 != 0, false),
            Phase::Decay => ((((self.ad >> 4) & 0xF) as u32) << 2, true, true),
            Phase::Sustain => (
                ((self.sr >> 6) & 0x7F) as u32,
                self.sr & 0x8000 != 0,
                self.sr & 0x4000 != 0,
            ),
            Phase::Release | Phase::Off => {
                ((((self.sr & 0x1F) as u32) << 2), self.sr & 0x20 != 0, true)
            }
        }
    }

    pub fn release(&mut self) {
        if self.phase != Phase::Off {
            self.phase = Phase::Release;
            self.counter = 0;
        }
    }

    pub fn is_off(&self) -> bool {
        self.phase == Phase::Off
    }

    /// Advances one 44.1 kHz sample; returns the level (0..0x7FFF).
    pub fn tick(&mut self) -> i32 {
        if self.phase == Phase::Off {
            return 0;
        }
        let (mut rate, exp, dec) = self.params();
        if exp && !dec && self.level >= 0x6000 {
            rate += 8;
        }
        let shift = (rate >> 2) as i32;
        let step_bits = (rate & 3) as i32;
        let mut step = if dec { -8 + step_bits } else { 7 - step_bits };
        let ticks: u32 = if shift < 11 {
            step <<= 11 - shift;
            1
        } else {
            1 << (shift - 11).min(20)
        };
        if exp && dec {
            step = (step * self.level) >> 15;
        }
        self.counter += 1;
        if self.counter >= ticks {
            self.counter = 0;
            self.level = (self.level + step).clamp(0, 0x7FFF);
        }
        match self.phase {
            Phase::Attack if self.level >= 0x7FFF => {
                self.phase = Phase::Decay;
                self.counter = 0;
            }
            Phase::Decay if self.level <= self.sustain_level() => {
                self.phase = Phase::Sustain;
                self.counter = 0;
            }
            Phase::Release if self.level == 0 => self.phase = Phase::Off,
            _ => {}
        }
        self.level
    }
}

struct Sample {
    pcm: Vec<i16>,
    looping: Option<(usize, usize)>,
}

struct Voice {
    seq: usize,
    note: u8,
    vel: u8,
    sample: usize,
    pos: f64,
    step: f64,
    adsr: Adsr,
    vol_l: f32,
    vol_r: f32,
    long: Option<usize>,
    drum: Option<usize>,
}

struct SeqState {
    pos: usize,
    wait: u32,
    elapsed: u32,
    instrument: usize,
    vol: u8,
    pan: u8,
    distort: u8,
    drums: bool,
    passes: u32,
}

fn lr(vol: i64, pan: u8) -> (f32, f32) {
    let v = vol.clamp(0, 0x3FFF);
    let l = (v * VOLUME_LR[255 - pan as usize] as i64) >> 8;
    let r = (v * VOLUME_LR[pan as usize] as i64) >> 8;
    (l as f32 / 16384.0, r as f32 / 16384.0)
}

pub struct Rendered {
    /// Interleaved stereo at `rate`.
    pub pcm: Vec<i16>,
    pub rate: u32,
    pub loop_seconds: f64,
    pub bpm: u16,
    pub voices_max: usize,
}

/// Renders one song of a CSEQ. The song's sequences each loop at their end (`opcode03` with
/// loop-at-end, as `Music_Adjust` starts race music). We play two passes and keep the second,
/// which already contains the first pass's ringing tails: looping the result end-to-start is
/// what the game plays.
pub fn render_song(howl: &Howl, cseq: &Cseq, song_index: usize, rate: u32) -> Result<Rendered> {
    let rate = rate.clamp(8000, SPU_RATE);
    // SPU samples per output sample (envelopes tick at 44.1 kHz).
    let spu_per_out = SPU_RATE as f64 / rate as f64;
    let song = cseq
        .songs
        .get(song_index)
        .ok_or_else(|| Error(format!("CSEQ has no song {song_index}")))?;
    // Decode every referenced sample once.
    let mut samples: Vec<Sample> = Vec::new();
    let mut by_spu = std::collections::HashMap::new();
    let mut get_sample = |spu: u16, samples: &mut Vec<Sample>| -> Option<usize> {
        if let Some(&i) = by_spu.get(&spu) {
            return Some(i);
        }
        let data = howl.samples.get(spu as usize)?.as_deref()?;
        let d = decode_adpcm(data);
        samples.push(Sample {
            pcm: d.pcm,
            looping: d.looping,
        });
        by_spu.insert(spu, samples.len() - 1);
        Some(samples.len() - 1)
    };

    let tpqn = song.tpqn.max(1) as u64;
    // Ticks per 60 Hz frame in 16.16 (SongPool_CalculateTempo).
    let tempo: u64 = ((song.bpm as u64 * tpqn / 60) << 16) / 60;
    let frame_samples = rate as f64 / 60.0;
    let mut env_clock = 0f64;

    let mut seqs: Vec<SeqState> = song
        .seqs
        .iter()
        .map(|s| {
            let mut pos = 0;
            let wait = varlen(&s.events, &mut pos);
            SeqState {
                pos,
                wait,
                elapsed: 0,
                instrument: 0,
                vol: 0xFF,
                pan: 0x80,
                distort: 0x80,
                drums: s.drums,
                passes: 0,
            }
        })
        .collect();
    let mut voices: Vec<Voice> = Vec::new();
    let mut out_l: Vec<f32> = Vec::new();
    let mut out_r: Vec<f32> = Vec::new();
    let mut frac: u64 = 0;
    let mut frame = 0u64;
    let mut sample_clock = 0f64;
    let mut first_pass_end: Option<usize> = None;
    let mut voices_max = 0;
    let max_frames = 60 * 60 * 12; // 12 minutes safety
    loop {
        // --- sequencer (Channel_ParseSongToChannels), once per 60 Hz frame.
        frac += tempo;
        let ticks = (frac >> 16) as u32;
        frac &= 0xFFFF;
        for (si, st) in seqs.iter_mut().enumerate() {
            let ev = &song.seqs[si].events;
            st.elapsed += ticks;
            let mut guard = 0;
            while st.wait <= st.elapsed {
                guard += 1;
                if guard > 10_000 {
                    return Err(Error(format!(
                        "song {song_index} seq {si}: runaway sequence"
                    )));
                }
                st.elapsed -= st.wait;
                let op = ev.get(st.pos).copied().unwrap_or(3) as usize;
                let a1 = ev.get(st.pos + 1).copied().unwrap_or(0);
                let a2 = ev.get(st.pos + 2).copied().unwrap_or(0);
                let mut restart = false;
                match op {
                    1 => {
                        for v in voices.iter_mut().filter(|v| v.seq == si && v.note == a1) {
                            v.adsr.release();
                        }
                    }
                    3 => {
                        restart = true;
                        st.passes += 1;
                    }
                    5 => {
                        let base = (VOL_MUSIC * VOL_SONG * st.vol as i64) >> 10;
                        if st.drums {
                            if let Some(ds) = cseq.short.get(a1 as usize) {
                                if let Some(smp) = get_sample(ds.spu_index, &mut samples) {
                                    let pitch = ds.pitch as f64
                                        * 2f64.powf((st.distort as f64 - 128.0) / 128.0);
                                    let (l, r) =
                                        lr((base * ds.volume as i64 * a2 as i64) >> 15, st.pan);
                                    voices.push(Voice {
                                        seq: si,
                                        note: a1,
                                        vel: a2,
                                        sample: smp,
                                        pos: 0.0,
                                        step: pitch / 4096.0,
                                        adsr: Adsr::new(0x80FF, 0x1FC2),
                                        vol_l: l,
                                        vol_r: r,
                                        long: None,
                                        drum: Some(a1 as usize),
                                    });
                                }
                            }
                        } else if let Some(ls) = cseq.long.get(st.instrument) {
                            if let Some(smp) = get_sample(ls.spu_index, &mut samples) {
                                let pitch = instrument_pitch(ls.base_pitch, a1, st.distort);
                                let (l, r) =
                                    lr((base * ls.volume as i64 * a2 as i64) >> 15, st.pan);
                                voices.push(Voice {
                                    seq: si,
                                    note: a1,
                                    vel: a2,
                                    sample: smp,
                                    pos: 0.0,
                                    step: pitch / 4096.0,
                                    adsr: Adsr::new(ls.ad, ls.sr),
                                    vol_l: l,
                                    vol_r: r,
                                    long: Some(st.instrument),
                                    drum: None,
                                });
                            }
                        }
                    }
                    6 | 7 => {
                        if op == 6 {
                            st.vol = a1;
                        } else {
                            st.pan = a1;
                        }
                        let base = VOL_MUSIC * VOL_SONG * st.vol as i64;
                        for v in voices.iter_mut().filter(|v| v.seq == si) {
                            let (l, r) = lr((base * v.vel as i64) >> 18, st.pan);
                            v.vol_l = l;
                            v.vol_r = r;
                        }
                    }
                    9 => st.instrument = a1 as usize,
                    10 => {
                        st.distort = a1;
                        for v in voices.iter_mut().filter(|v| v.seq == si) {
                            let pitch = if let Some(li) = v.long {
                                instrument_pitch(cseq.long[li].base_pitch, v.note, st.distort)
                            } else if let Some(di) = v.drum {
                                cseq.short[di].pitch as f64
                                    * 2f64.powf((st.distort as f64 - 128.0) / 128.0)
                            } else {
                                continue;
                            };
                            v.step = pitch / 4096.0;
                        }
                    }
                    _ => {}
                }
                if restart {
                    st.pos = 0;
                } else {
                    st.pos += OPCODE_SIZE.get(op).copied().unwrap_or(1);
                }
                st.wait = varlen(ev, &mut st.pos);
            }
        }
        if first_pass_end.is_none() && seqs.iter().all(|s| s.passes >= 1) {
            first_pass_end = Some(out_l.len());
        }
        if let Some(end) = first_pass_end {
            if out_l.len() >= end * 2 {
                let len = end;
                // Second pass: [end, 2 * end).
                let (l, r) = (&out_l[end..2 * end], &out_r[end..2 * end]);
                let peak = l
                    .iter()
                    .chain(r)
                    .fold(0f32, |m, x| m.max(x.abs()))
                    .max(1e-6);
                let gain = 0.89 / peak; // normalize to -1 dBFS
                let mut pcm = Vec::with_capacity(l.len() * 2);
                for (a, b) in l.iter().zip(r) {
                    pcm.push((a * gain * 32767.0).round().clamp(-32768.0, 32767.0) as i16);
                    pcm.push((b * gain * 32767.0).round().clamp(-32768.0, 32767.0) as i16);
                }
                return Ok(Rendered {
                    pcm,
                    rate,
                    loop_seconds: len as f64 / rate as f64,
                    bpm: song.bpm,
                    voices_max,
                });
            }
        }
        if frame > max_frames {
            return Err(Error(format!("song {song_index} never loops")));
        }
        // --- mix one frame of audio.
        sample_clock += frame_samples;
        let n = sample_clock as usize - out_l.len();
        voices_max = voices_max.max(voices.len());
        for _ in 0..n {
            env_clock += spu_per_out;
            let env_ticks = env_clock as u32;
            env_clock -= env_ticks as f64;
            let (mut l, mut r) = (0f32, 0f32);
            for v in voices.iter_mut() {
                let s = &samples[v.sample];
                let i = v.pos as usize;
                if i + 1 >= s.pcm.len() && s.looping.is_none() {
                    v.adsr = Adsr {
                        phase: Phase::Off,
                        ..v.adsr
                    };
                    continue;
                }
                let a = s.pcm[i.min(s.pcm.len() - 1)] as f32;
                let b = s.pcm[(i + 1).min(s.pcm.len() - 1)] as f32;
                let t = (v.pos - i as f64) as f32;
                let x = a + (b - a) * t;
                let mut lvl = 0;
                for _ in 0..env_ticks.max(1) {
                    lvl = v.adsr.tick();
                }
                let env = lvl as f32 / 32768.0;
                l += x * env * v.vol_l / 32768.0;
                r += x * env * v.vol_r / 32768.0;
                v.pos += v.step * spu_per_out;
                if let Some((ls, le)) = s.looping {
                    while v.pos >= le as f64 {
                        v.pos -= (le - ls).max(1) as f64;
                    }
                }
            }
            voices.retain(|v| !v.adsr.is_off());
            out_l.push(l);
            out_r.push(r);
        }
        frame += 1;
    }
}

/// One exported race track.
pub struct TrackFile {
    pub file: String,
    pub track: &'static str,
    pub level_id: usize,
    pub howl_sequence: usize,
    pub seconds: f64,
    pub bpm: u16,
    pub peak: i16,
    pub same_as: Option<String>,
}

/// Renders every race track's music into `dir` as stereo WAV (`<track>.wav`, looping
/// end-to-start, `smpl` loop chunk) plus `music.json`. Tracks sharing one sequence (Slide
/// Coliseum and Turbo Track reuse Tiny Arena's) point at the first file via `same_as`.
pub fn extract(
    disc: &mut crate::disc::Disc,
    dir: &std::path::Path,
    rate: u32,
) -> Result<Vec<TrackFile>> {
    let (lba, size) = disc.find_file(crate::audio::HWL_PATH)?;
    let howl = Howl::parse(&disc.read_bytes(lba, size as usize)?)?;
    std::fs::create_dir_all(dir).map_err(|e| Error(format!("{}: {e}", dir.display())))?;
    let mut out: Vec<TrackFile> = Vec::new();
    for (level, (name, seq)) in RACE_TRACKS.iter().enumerate() {
        let data = howl
            .seqs
            .get(*seq)
            .ok_or_else(|| Error(format!("HOWL has no sequence {seq}")))?;
        if let Some(prev) = out.iter().find(|t| howl.seqs[t.howl_sequence] == *data) {
            let t = TrackFile {
                file: prev.file.clone(),
                track: name,
                level_id: level,
                howl_sequence: *seq,
                seconds: prev.seconds,
                bpm: prev.bpm,
                peak: prev.peak,
                same_as: Some(prev.track.to_string()),
            };
            out.push(t);
            continue;
        }
        let cseq = Cseq::parse(data)?;
        let r = render_song(&howl, &cseq, 0, rate)?;
        let file = format!("{name}.wav");
        let path = dir.join(&file);
        std::fs::write(&path, crate::audio::wav_channels(&r.pcm, 2, r.rate, true))
            .map_err(|e| Error(format!("{}: {e}", path.display())))?;
        let peak = r.pcm.iter().map(|s| s.saturating_abs()).max().unwrap_or(0);
        out.push(TrackFile {
            file,
            track: name,
            level_id: level,
            howl_sequence: *seq,
            seconds: r.loop_seconds,
            bpm: r.bpm,
            peak,
            same_as: None,
        });
    }
    let doc = serde_json::json!({
        "source": "SOUNDS/KART.HWL CSEQ sequences, song 0 (CSEQ_SONG_LEVEL) of data.levBank_Song[level]; rendered with a software SPU model (no reverb)",
        "format": { "channels": 2, "bits": 16, "rate_hz": rate },
        "loop": "every file loops end-to-start seamlessly (it is the second pass of the song, so notes ringing over the loop point are included); the WAV smpl chunk marks the whole file as the loop",
        "final_lap": "HOWL_AudioState.c AUDIO_FINAL_LAP: once the final-lap XA sting has played ~1 s, Audio_SetMaskSong(20) raises the tempo by 20 BPM (Music_Adjust -> CseqMusic_ChangeTempo); approximate with playback speed (bpm + 20) / bpm (pitch rises too, unlike the game)",
        "final_lap_bpm_delta": 20,
        "tracks": out.iter().map(|t| serde_json::json!({
            "file": t.file,
            "track": t.track,
            "level_id": t.level_id,
            "howl_sequence": t.howl_sequence,
            "duration_seconds": t.seconds,
            "loop_start_seconds": 0.0,
            "loop_end_seconds": t.seconds,
            "bpm": t.bpm,
            "peak": t.peak,
            "same_music_as": t.same_as,
        })).collect::<Vec<_>>(),
    });
    let path = dir.join("music.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&doc).unwrap())
        .map_err(|e| Error(format!("{}: {e}", path.display())))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varlen_matches_midi_style_delays() {
        let d = [0x05, 0x81, 0x00, 0x83, 0xFF, 0x7F];
        let mut p = 0;
        assert_eq!(varlen(&d, &mut p), 5);
        assert_eq!(varlen(&d, &mut p), 128);
        assert_eq!(varlen(&d, &mut p), (3 << 14) | (0x7F << 7) | 0x7F);
    }

    #[test]
    fn instrument_pitch_follows_note_table() {
        // noteFrequency[60] = 0x1000, [72] = 0x2000, [48] = 0x800; distortion 0x80 neutral.
        assert!((instrument_pitch(0x1000, 60, 0x80) - 4096.0).abs() < 1e-6);
        assert!((instrument_pitch(0x1000, 72, 0x80) - 8192.0).abs() < 1e-6);
        assert!((instrument_pitch(0x1000, 61, 0x80) - 0x10F3 as f64).abs() < 1.0);
        // +64 distortion = one semitone up.
        assert!((instrument_pitch(0x1000, 60, 0xC0) - 0x10F3 as f64).abs() < 1.0);
    }

    #[test]
    fn adsr_attack_decay_sustain_release() {
        // Fast linear attack (rate 0), decay rate 0 to sustain level 0x8000 * (7+1)/16.
        let mut e = Adsr::new(0x0007, 0x0000);
        let mut n = 0;
        while e.phase == Phase::Attack && n < 100_000 {
            e.tick();
            n += 1;
        }
        assert_eq!(e.phase, Phase::Decay);
        while e.phase == Phase::Decay && n < 200_000 {
            e.tick();
            n += 1;
        }
        assert_eq!(e.phase, Phase::Sustain);
        assert!(e.level <= 0x4000 && e.level > 0x3000, "{:#x}", e.level);
        e.release();
        for _ in 0..200_000 {
            e.tick();
        }
        assert!(e.is_off());
    }

    #[test]
    fn parses_cseq_layout() {
        let mut d = vec![0u8; 8];
        d[4] = 1; // one long sample
        d[5] = 1; // one drum
        d[6] = 1; // one song
        d.extend([1, 200, 0, 0, 0x00, 0x10, 5, 0, 0xFF, 0x80, 0xC2, 0x1F]);
        d.extend([1, 100, 0x00, 0x08, 6, 0, 0, 0]);
        d.extend(0u16.to_le_bytes()); // song offset
        while !d.len().is_multiple_of(4) {
            d.push(0);
        }
        let song = d.len();
        d.extend([0, 1]); // unk, 1 seq
        d.extend(120u16.to_le_bytes());
        d.extend(96u16.to_le_bytes());
        d.extend(0u16.to_le_bytes()); // seq offset
        while !(d.len() - song).is_multiple_of(4) || !d.len().is_multiple_of(4) {
            d.push(0);
        }
        d.extend([1, 0, 0x00, 0x05, 60, 100, 0x10, 0x03]);
        let size = d.len() as u32;
        d[0..4].copy_from_slice(&size.to_le_bytes());
        let c = Cseq::parse(&d).unwrap();
        assert_eq!(c.long[0].base_pitch, 0x1000);
        assert_eq!(c.long[0].spu_index, 5);
        assert_eq!(c.short[0].pitch, 0x800);
        assert_eq!(c.songs[0].bpm, 120);
        assert!(c.songs[0].seqs[0].drums);
        assert_eq!(&c.songs[0].seqs[0].events[..4], &[0x00, 0x05, 60, 100]);
    }
}
