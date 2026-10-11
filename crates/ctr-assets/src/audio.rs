//! CTR sound effects: the HOWL sound bank (`SOUNDS\KART.HWL`), PS-ADPCM decoding and WAV
//! output for the kart, weapon and pickup sounds the client plays.
//!
//! ```text
//! HowlHeader { char magic[4] = "HOWL"; u32 version (0x80); u32; u32;
//!              u32 num_spu_addrs; u32 num_other_fx; u32 num_engine_fx;
//!              u32 num_banks; u32 num_seqs; u32 header_size }            // 0x28 bytes
//! SpuAddr     { u16 runtime_addr (0 on disc); u16 size_div_8 }           // num_spu_addrs
//! OtherFX     { u8 flags; u8 volume; u16 pitch; u16 spu_index; u16 duration }  // num_other_fx
//! EngineFX    { u8 flags; u8 volume; u16 pitch; u16 unk; u16 spu_index }       // num_engine_fx
//! u16 bank_sector[num_banks]; u16 seq_sector[num_seqs]                    // in 2048-byte units
//! Bank (at bank_sector * 2048) { u16 count; u16 spu_index[count]; pad to 2048;
//!        then the samples' raw PS-ADPCM, size_div_8 * 8 bytes each, in that order }
//! ```
//!
//! `pitch` is the SPU pitch: 0x1000 plays at 44100 Hz, so a sample's natural rate is
//! `pitch * 44100 / 4096`. OtherFX `flags & 2` marks looped sounds (the channel is kept).
//! Struct layouts follow CTR-native `include/namespace_Howl.h`; note EngineFX keeps its
//! `spu_index` at offset 6, unlike OtherFX.

use crate::{Error, Result};

/// One OtherFX / EngineFX table entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Instrument {
    pub flags: u8,
    pub volume: u8,
    pub pitch: u16,
    pub spu_index: u16,
}

impl Instrument {
    /// Natural sample rate (Hz) of this instrument.
    pub fn rate(&self) -> u32 {
        ((self.pitch as u64 * 44100 + 2048) / 4096) as u32
    }
    pub fn looped(&self) -> bool {
        self.flags & 2 != 0
    }
}

pub struct Howl {
    pub version: u32,
    pub other_fx: Vec<Instrument>,
    pub engine_fx: Vec<Instrument>,
    /// Raw ADPCM per SPU index (`None` if no bank holds it).
    pub samples: Vec<Option<Vec<u8>>>,
    /// The CSEQ music files (`u32 size` + data), in HOWL order.
    pub seqs: Vec<Vec<u8>>,
}

fn u16_at(d: &[u8], at: usize) -> Result<u16> {
    d.get(at..at + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or_else(|| Error(format!("HOWL truncated at {at:#x}")))
}

fn u32_at(d: &[u8], at: usize) -> Result<u32> {
    d.get(at..at + 4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .ok_or_else(|| Error(format!("HOWL truncated at {at:#x}")))
}

impl Howl {
    pub fn parse(d: &[u8]) -> Result<Self> {
        if d.get(0..4) != Some(b"HOWL") {
            return Err(Error("not a HOWL file".into()));
        }
        let version = u32_at(d, 4)?;
        let n_spu = u32_at(d, 0x10)? as usize;
        let n_other = u32_at(d, 0x14)? as usize;
        let n_engine = u32_at(d, 0x18)? as usize;
        let n_banks = u32_at(d, 0x1C)? as usize;
        if n_spu > 4096 || n_other > 4096 || n_engine > 4096 || n_banks > 1024 {
            return Err(Error("implausible HOWL header".into()));
        }
        let mut at = 0x28;
        let mut sizes = Vec::with_capacity(n_spu);
        for _ in 0..n_spu {
            sizes.push(u16_at(d, at + 2)? as usize * 8);
            at += 4;
        }
        let read_inst = |engine: bool, at: usize| -> Result<Instrument> {
            Ok(Instrument {
                flags: d[at],
                volume: d[at + 1],
                pitch: u16_at(d, at + 2)?,
                spu_index: u16_at(d, at + if engine { 6 } else { 4 })?,
            })
        };
        let mut other_fx = Vec::with_capacity(n_other);
        for _ in 0..n_other {
            other_fx.push(read_inst(false, at)?);
            at += 8;
        }
        let mut engine_fx = Vec::with_capacity(n_engine);
        for _ in 0..n_engine {
            engine_fx.push(read_inst(true, at)?);
            at += 8;
        }
        let mut samples: Vec<Option<Vec<u8>>> = vec![None; n_spu];
        for b in 0..n_banks {
            let start = u16_at(d, at + b * 2)? as usize * 2048;
            let count = u16_at(d, start)? as usize;
            if count > 1023 {
                return Err(Error(format!(
                    "HOWL bank {b}: implausible sample count {count}"
                )));
            }
            let mut pos = (start + 2 + count * 2).div_ceil(2048) * 2048;
            for i in 0..count {
                let idx = u16_at(d, start + 2 + i * 2)? as usize;
                let size = *sizes
                    .get(idx)
                    .ok_or_else(|| Error(format!("HOWL bank {b}: sample {idx} out of range")))?;
                let data = d
                    .get(pos..pos + size)
                    .ok_or_else(|| Error(format!("HOWL bank {b}: sample {idx} truncated")))?;
                if samples[idx].is_none() {
                    samples[idx] = Some(data.to_vec());
                }
                pos += size;
            }
        }
        let n_seqs = u32_at(d, 0x20)? as usize;
        let mut seqs = Vec::with_capacity(n_seqs);
        for i in 0..n_seqs.min(256) {
            let start = u16_at(d, at + (n_banks + i) * 2)? as usize * 2048;
            let size = u32_at(d, start)? as usize;
            let data = d
                .get(start..start + size)
                .ok_or_else(|| Error(format!("HOWL sequence {i} truncated")))?;
            seqs.push(data.to_vec());
        }
        Ok(Self {
            version,
            other_fx,
            engine_fx,
            samples,
            seqs,
        })
    }

    pub fn sample(&self, inst: &Instrument) -> Result<&[u8]> {
        self.samples
            .get(inst.spu_index as usize)
            .and_then(|s| s.as_deref())
            .ok_or_else(|| Error(format!("SPU sample {} is in no bank", inst.spu_index)))
    }
}

// ---------------------------------------------------------------------------------------------
// PS-ADPCM
// ---------------------------------------------------------------------------------------------

const F0: [i32; 5] = [0, 60, 115, 98, 122];
const F1: [i32; 5] = [0, 0, -52, -55, -60];

/// Decoded ADPCM: PCM and the loop region signalled by the block flags (bit 2 = loop start,
/// bit 0 = end, bit 1 = repeat: an end+repeat block jumps back to the loop start).
#[derive(Clone, Debug, PartialEq)]
pub struct Decoded {
    pub pcm: Vec<i16>,
    /// `(start, end)` sample indices (end exclusive) when the data loops.
    pub looping: Option<(usize, usize)>,
}

/// Decodes one 16-byte block into 28 samples, updating the two-sample history.
pub fn decode_block(block: &[u8], hist: &mut [i32; 2], out: &mut Vec<i16>) {
    let shift = (block[0] & 0x0F) as i32;
    let filter = ((block[0] >> 4) as usize).min(4);
    let shift = if shift > 12 { 9 } else { shift };
    for i in 0..28 {
        let byte = block[2 + i / 2];
        let nib = if i % 2 == 0 { byte & 0x0F } else { byte >> 4 };
        let s = ((nib as i16) << 12) as i32 >> shift;
        let v = s + ((hist[0] * F0[filter] + hist[1] * F1[filter] + 32) >> 6);
        let v = v.clamp(-32768, 32767);
        hist[1] = hist[0];
        hist[0] = v;
        out.push(v as i16);
    }
}

/// Decodes a raw PS-ADPCM stream up to its end flag.
pub fn decode_adpcm(data: &[u8]) -> Decoded {
    let mut pcm = Vec::with_capacity(data.len() / 16 * 28);
    let mut hist = [0i32; 2];
    let mut loop_start = None;
    let mut looping = None;
    for block in data.as_chunks::<16>().0 {
        let flags = block[1];
        if flags & 4 != 0 && loop_start.is_none() {
            loop_start = Some(pcm.len());
        }
        decode_block(block, &mut hist, &mut pcm);
        if flags & 1 != 0 {
            if flags & 2 != 0 {
                looping = Some((loop_start.unwrap_or(0), pcm.len()));
            }
            break;
        }
    }
    Decoded { pcm, looping }
}

/// For a looped sound: the loop body as the SPU plays it on every pass after the first (the
/// ADPCM filter history carries over from the loop end), so repeating it is seamless.
pub fn steady_loop(data: &[u8]) -> Option<Vec<i16>> {
    let first = decode_adpcm(data);
    let (start, end) = first.looping?;
    if end <= start + 28 {
        return None;
    }
    let mut hist = [first.pcm[end - 1] as i32, first.pcm[end - 2] as i32];
    let mut out = Vec::with_capacity(end - start);
    for block in data
        .as_chunks::<16>()
        .0
        .iter()
        .skip(start / 28)
        .take((end - start) / 28)
    {
        decode_block(block, &mut hist, &mut out);
    }
    Some(out)
}

// ---------------------------------------------------------------------------------------------
// WAV
// ---------------------------------------------------------------------------------------------

/// A mono 16-bit WAV; with `looped`, a `smpl` chunk marks the whole file as one forward loop.
pub fn wav(pcm: &[i16], rate: u32, looped: bool) -> Vec<u8> {
    wav_channels(pcm, 1, rate, looped)
}

/// A 16-bit WAV with `channels` interleaved channels (see [`wav`]).
pub fn wav_channels(pcm: &[i16], channels: u16, rate: u32, looped: bool) -> Vec<u8> {
    let data_len = pcm.len() as u32 * 2;
    let frames = pcm.len() / channels.max(1) as usize;
    let smpl_len = if looped { 8 + 36 + 24 } else { 0 };
    let block = 2 * channels as u32;
    let mut w = Vec::with_capacity(44 + data_len as usize + smpl_len as usize);
    w.extend(b"RIFF");
    w.extend((36 + data_len + smpl_len).to_le_bytes());
    w.extend(b"WAVEfmt ");
    w.extend(16u32.to_le_bytes());
    w.extend(1u16.to_le_bytes()); // PCM
    w.extend(channels.to_le_bytes());
    w.extend(rate.to_le_bytes());
    w.extend((rate * block).to_le_bytes());
    w.extend((block as u16).to_le_bytes());
    w.extend(16u16.to_le_bytes());
    w.extend(b"data");
    w.extend(data_len.to_le_bytes());
    for s in pcm {
        w.extend(s.to_le_bytes());
    }
    if looped {
        w.extend(b"smpl");
        w.extend(60u32.to_le_bytes());
        let period = 1_000_000_000 / rate.max(1);
        for v in [0u32, 0, period, 60, 0, 0, 0, 1, 0] {
            w.extend(v.to_le_bytes());
        }
        // One loop: id, type (forward), start, end (inclusive, in frames), fraction, count (inf).
        for v in [0u32, 0, 0, frames.saturating_sub(1) as u32, 0, 0] {
            w.extend(v.to_le_bytes());
        }
    }
    w
}

// ---------------------------------------------------------------------------------------------
// Kart, weapon and pickup sound selection
// ---------------------------------------------------------------------------------------------

/// Which HOWL table an output file comes from.
#[derive(Clone, Copy, Debug)]
pub enum Source {
    /// `OtherFX` sound id (`OtherFX_Play*`).
    Other(usize),
    /// `EngineFX` id: `engine_class * 4 + driver_slot` (`EngineSound_Player`).
    Engine(usize),
}

pub struct SoundSpec {
    pub file: &'static str,
    pub source: Source,
    /// Write only the steady loop (the sound loops in game).
    pub loop_: bool,
    /// HOWL "distortion" applied by the game (0x80 = none). Baked in as a WAV rate change:
    /// OtherFX pitch factor `2^((d - 0x80) / 128)` (`distortConst_OtherFX`).
    pub distort: u8,
    /// HOWL volume the game plays it at (0xFF = full); baked in as gain.
    pub volume: u8,
    pub reference: &'static str,
}

/// Engine class of Crash (BALANCED = 0), player slot 0 (`EngineSound_Player`:
/// `EngineAudio_Recalculate(engine * 4 + driverID, ...)`).
pub const ENGINE_BALANCED_P1: usize = 0;

/// A weapon/pickup OtherFX row: played at full event volume with no distortion.
const fn weapon(file: &'static str, id: usize, loop_: bool, reference: &'static str) -> SoundSpec {
    SoundSpec {
        file,
        source: Source::Other(id),
        loop_,
        distort: 0x80,
        volume: 0xFF,
        reference,
    }
}

pub const SOUNDS: [SoundSpec; 41] = [
    SoundSpec {
        file: "engine.wav",
        source: Source::Engine(ENGINE_BALANCED_P1),
        loop_: true,
        distort: 0x80,
        volume: 0xFF,
        reference: "HOWL_Engine.c EngineSound_Player: EngineFX[engineID*4 + driverID]; Crash engineID = BALANCED (zGlobal_DATA.c MetaDataCharacters)",
    },
    SoundSpec {
        file: "skid.wav",
        source: Source::Other(17),
        loop_: true,
        distort: 0x80,
        volume: 0xFF,
        reference: "VehEmitter.c: OtherFX_RecycleNew(.., terrain->skidSound, ..); asphalt skidSound = 17 (zGlobal_DATA.c MetaDataTerrain)",
    },
    SoundSpec {
        file: "turbo.wav",
        source: Source::Other(0xE),
        loop_: true,
        distort: 0x80,
        volume: 0xFF,
        reference: "VehTurbo.c: OtherFX_RecycleNew(driverAudio, TURBO_AUDIO_SFX_ID = 0xE, ..) every frame while the turbo thread lives",
    },
    SoundSpec {
        file: "hop.wav",
        source: Source::Other(8),
        loop_: false,
        distort: 0x80,
        volume: 0xFF,
        reference: "VehPhysGeneral.c: OtherFX_Play_Echo(VEH_PHYS_JUMP_NORMAL_SFX = 8, ..)",
    },
    SoundSpec {
        file: "land.wav",
        source: Source::Other(7),
        loop_: false,
        distort: 0x80,
        volume: 0x78,
        reference: "COLL.c: OtherFX_Play_LowLevel(7, 1, volume = MapToRange(|landing speed|, 0x100, 0x3C00, 0x78, 0xFA)); soft end",
    },
    SoundSpec {
        file: "land_hard.wav",
        source: Source::Other(7),
        loop_: false,
        distort: 0x80,
        volume: 0xFA,
        reference: "COLL.c: same sound 7 at the hard end of the landing volume range (0xFA)",
    },
    SoundSpec {
        file: "boost_1.wav",
        source: Source::Other(0xD),
        loop_: false,
        distort: 0x94,
        volume: 0x80,
        reference: "VehFire.c VehFire_Audio(VEH_FIRE_AUDIO_SFX = 0xD): fireLevel = numBoostsSuccess << 6; first boost (< 0x40): volume 0x80, distort 0x94",
    },
    SoundSpec {
        file: "boost_2.wav",
        source: Source::Other(0xD),
        loop_: false,
        distort: 0x80,
        volume: 0xC0,
        reference: "VehFire.c VehFire_Audio: second boost (< 0x80): volume 0xC0, no distortion",
    },
    SoundSpec {
        file: "boost_3.wav",
        source: Source::Other(0xD),
        loop_: false,
        distort: 0x6C,
        volume: 0xFF,
        reference: "VehFire.c VehFire_Audio: third boost: volume 0xFF, distort 0x6C (+ voice line 0x10)",
    },
    SoundSpec {
        file: "meter_full.wav",
        source: Source::Other(0xF),
        loop_: false,
        distort: 0x80,
        volume: 0xFF,
        reference: "VehPhysProc.c PowerSlide_Update: meter runs out -> OtherFX_Play_Echo(VEH_PHYS_PROC_DRIFT_METER_FULL_FX = 0xF, ..)",
    },
    SoundSpec {
        file: "crash.wav",
        source: Source::Other(6),
        loop_: false,
        distort: 0x80,
        volume: 0xFF,
        reference: "COLL.c wall scrub impact: OtherFX_Play_LowLevel(6, 1, volume max) (+ voice line 6, rumble)",
    },
    // Crates, Wumpa and the item roulette.
    weapon("roulette.wav", 0x5D, true, "RB_Crate.c:202 OtherFX_Play(0x5d, 0) when a weapon crate starts the roll; stopped by OtherFX_Stop2(0x5d) (UI_RenderFrame.c:875)"),
    weapon("roulette_stop.wav", 0x5E, false, "VehPhysProc.c:21 ITEM_ROLL_NORMAL_FX = 0x5E: the roulette lands on an item"),
    weapon("roulette_juiced.wav", 0x41, false, "VehPhysProc.c:22 ITEM_ROLL_JUICED_FX = 0x41: the roulette lands with >= 10 Wumpa"),
    weapon("crate_break.wav", 0x3C, false, "RB_Crate.c:150 PlaySound3D(0x3c, crateInst): any crate breaks"),
    weapon("wumpa.wav", 0x43, false, "RB_Fruit.c:60 PlaySound3D(0x43, fruitInst): a loose Wumpa fruit is picked up"),
    weapon("wumpa_count.wav", 0x42, false, "UI_RenderFrame.c:235 OtherFX_Play(0x42, 1): a collected Wumpa reaches the HUD counter"),
    // Bowling bomb.
    weapon("bomb_launch.wav", 0x47, false, "VehPickupItem.c:151 SOUND_BOMB_LAUNCH = 0x47"),
    weapon("bomb_roll.wav", 0x48, true, "RB_MovingExplosive.c:50 PlaySound3D_Flags(.., 0x48, inst) while the bomb rolls"),
    weapon("bomb_explode.wav", 0x49, false, "RB_MovingExplosive.c:379 PlaySound3D(0x49, inst)"),
    // Homing missile.
    weapon("missile_launch.wav", 0x4A, false, "VehPickupItem.c:152 SOUND_MISSILE_LAUNCH = 0x4A"),
    weapon("missile_fly.wav", 0x4B, true, "RB_MovingExplosive.c:43 PlaySound3D_Flags(.., 0x4b, inst) while the missile flies"),
    weapon("missile_explode.wav", 0x4C, false, "RB_MovingExplosive.c:392 PlaySound3D(0x4c, inst)"),
    weapon("missile_lock.wav", 0x56, false, "UI_RaceHud.c:31 UI_TRACKER_BEEP_SFX = 0x56: lock-on beep for the targeted driver, every 5/10/30 frames by distance"),
    // TNT, Nitro and the potion.
    weapon("mine_drop.wav", 0x52, false, "VehPickupItem.c:153 SOUND_MINE_DROP = 0x52: TNT, Nitro or a potion is dropped"),
    weapon("tnt_land.wav", 0x50, false, "RB_GenericMine.c:284 PlaySound3D(0x50, inst): TNT lands on a driver's head"),
    weapon("tnt_jump.wav", 0x51, false, "RB_TNT.c:289 PlaySound3D(0x51, inst): each hop that shakes TNT off"),
    weapon("tnt_honk.wav", 0x3E, false, "RB_TNT.c:208 PlaySound3D(0x3e, inst): the TNT countdown tick"),
    weapon("tnt_explode.wav", 0x3D, false, "RB_TNT.c:32 / RB_GenericMine.c:404 PlaySound3D(0x3d, inst): TNT and Nitro explode"),
    weapon("glass.wav", 0x3F, false, "RB_GenericMine.c:397 PlaySound3D(0x3f, inst): a potion or Nitro shatters"),
    // Shield, masks, clock, warp orb.
    weapon("shield.wav", 0x57, false, "VehPickupItem.c:154 SOUND_SHIELD = 0x57: the shield goes up"),
    weapon("shield_pop.wav", 0x58, false, "RB_MaskShieldCloud.c:261 PlaySound3D(0x58, instDark): the shield pops"),
    weapon("shield_roll.wav", 0x59, true, "RB_MovingExplosive.c:56 PlaySound3D_Flags(.., 0x59, inst): a thrown shield rolls"),
    weapon("aku.wav", 0x53, false, "VehPickupItem.c:284 OtherFX_Play_Echo(STATIC_AKUAKU + 0x1A = 0x53, ..): Aku Aku mask"),
    weapon("uka.wav", 0x54, false, "VehPickupItem.c:305 OtherFX_Play_Echo(STATIC_UKAUKA + 0x1A = 0x54, ..): Uka Uka mask"),
    weapon("clock.wav", 0x44, false, "VehPickupItem.c:155 SOUND_CLOCK = 0x44"),
    weapon("warp_launch.wav", 0x4D, false, "VehPickupItem.c:156 SOUND_WARPBALL = 0x4D"),
    weapon("warp_fly.wav", 0x4E, true, "RB_Warpball.c:605 PlaySound3D_Flags(.., 0x4e, inst) while the warp orb flies"),
    weapon("warp_hit.wav", 0x4F, false, "RB_Warpball.c:66 PlaySound3D(0x4f, inst): the warp orb hits"),
    // The start lights: OtherFX_Play(id, 0) plays at the default flags, full volume, no
    // distortion (HOWL_OtherFX.c:19-21, namespace_Howl.h:65).
    weapon("countdown.wav", 0x45, false, "DotLights.c:26 DOT_LIGHT_FX_COUNTDOWN = 0x45: OtherFX_Play(0x45, 0) as each red start light comes on (DotLights.c:108, 121, 134)"),
    weapon("go.wav", 0x46, false, "DotLights.c:27 DOT_LIGHT_FX_GREEN = 0x46: OtherFX_Play(0x46, 0) as the green start light comes on (DotLights.c:95)"),
];

/// OtherFX distortion pitch factor (`distortConst_OtherFX[d] / 0x10000`).
pub fn other_fx_pitch_factor(distort: u8) -> f64 {
    2f64.powf((distort as f64 - 128.0) / 128.0)
}

/// EngineFX distortion pitch factor (`distortConst_Engine[d] / 0x10000`): one octave per 64.
pub fn engine_pitch_factor(distort: u8) -> f64 {
    2f64.powf((distort as f64 - 128.0) / 64.0)
}

pub struct Rendered {
    pub file: &'static str,
    pub rate: u32,
    pub samples: usize,
    pub looped: bool,
    pub peak: i16,
    pub spu_index: u16,
    pub wav: Vec<u8>,
}

pub fn render(howl: &Howl, spec: &SoundSpec) -> Result<Rendered> {
    let inst = match spec.source {
        Source::Other(i) => howl.other_fx.get(i),
        Source::Engine(i) => howl.engine_fx.get(i),
    }
    .copied()
    .ok_or_else(|| Error(format!("{}: sound id out of range", spec.file)))?;
    let data = howl.sample(&inst)?;
    let full = decode_adpcm(data);
    let (mut pcm, looped) = match (spec.loop_, steady_loop(data)) {
        (true, Some(l)) => (l, true),
        _ => (full.pcm, false),
    };
    // Very short loops (the engine's is 952 samples, ~80 ms) are repeated to at least half a
    // second; the steady loop is periodic, so the repetition is exactly what the SPU plays.
    if looped && !pcm.is_empty() {
        let min = inst.rate() as usize / 2;
        let one = pcm.clone();
        while pcm.len() < min {
            pcm.extend_from_slice(&one);
        }
    }
    // Trim trailing silence of one-shots.
    if !looped {
        while pcm.last() == Some(&0) {
            pcm.pop();
        }
    }
    if spec.volume != 0xFF {
        let g = spec.volume as i32;
        for s in &mut pcm {
            *s = ((*s as i32 * g) / 0xFF) as i16;
        }
    }
    let rate = (inst.rate() as f64 * other_fx_pitch_factor(spec.distort)).round() as u32;
    let peak = pcm.iter().map(|s| s.saturating_abs()).max().unwrap_or(0);
    let wav = wav(&pcm, rate, looped);
    Ok(Rendered {
        file: spec.file,
        rate,
        samples: pcm.len(),
        looped,
        peak,
        spu_index: inst.spu_index,
        wav,
    })
}

/// Path of the HOWL bank on the disc.
pub const HWL_PATH: &str = "SOUNDS/KART.HWL";

/// Reads `SOUNDS\KART.HWL` from the disc, writes every [`SOUNDS`] entry as WAV into `dir`
/// plus `sounds.json` (rates, loops, ids, game volumes), and returns what was written.
pub fn extract(disc: &mut crate::disc::Disc, dir: &std::path::Path) -> Result<Vec<Rendered>> {
    let (lba, size) = disc.find_file(HWL_PATH)?;
    let howl = Howl::parse(&disc.read_bytes(lba, size as usize)?)?;
    std::fs::create_dir_all(dir).map_err(|e| Error(format!("{}: {e}", dir.display())))?;
    let mut out = Vec::new();
    let mut manifest = Vec::new();
    for spec in &SOUNDS {
        let r = render(&howl, spec)?;
        let path = dir.join(r.file);
        std::fs::write(&path, &r.wav).map_err(|e| Error(format!("{}: {e}", path.display())))?;
        let inst = match spec.source {
            Source::Other(i) => howl.other_fx[i],
            Source::Engine(i) => howl.engine_fx[i],
        };
        manifest.push(serde_json::json!({
            "file": r.file,
            "source": match spec.source {
                Source::Other(i) => format!("OtherFX {i:#04x}"),
                Source::Engine(i) => format!("EngineFX {i}"),
            },
            "spu_sample": r.spu_index,
            "rate_hz": r.rate,
            "samples": r.samples,
            "seconds": r.samples as f64 / r.rate as f64,
            "looped": r.looped,
            "game_distortion": spec.distort,
            "game_event_volume": spec.volume,
            "instrument_volume": inst.volume,
            "peak": r.peak,
            "reference": spec.reference,
        }));
        out.push(r);
    }
    let engine = howl.engine_fx[ENGINE_BALANCED_P1];
    let doc = serde_json::json!({
        "source": HWL_PATH,
        "engine": {
            "base_rate_hz": engine.rate(),
            "spu_pitch": engine.pitch,
            "playback_rate_factor": "2^((d - 128) / 64), d = engine distortion 0x3C (idle, x0.479) .. 200 (top speed, x2.181)",
        },
        "sounds": manifest,
    });
    let path = dir.join("sounds.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&doc).unwrap())
        .map_err(|e| Error(format!("{}: {e}", path.display())))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(shift_filter: u8, flags: u8, nibbles: [u8; 28]) -> [u8; 16] {
        let mut b = [0u8; 16];
        b[0] = shift_filter;
        b[1] = flags;
        for i in 0..14 {
            b[2 + i] = nibbles[2 * i] | (nibbles[2 * i + 1] << 4);
        }
        b
    }

    #[test]
    fn decodes_raw_nibbles_without_filter() {
        // Filter 0, shift 12: sample = signed nibble.
        let mut n = [0u8; 28];
        n[0] = 1;
        n[1] = 0xF; // -1
        n[2] = 7;
        n[3] = 8; // -8
        let d = decode_adpcm(&block(0x0C, 1, n));
        assert_eq!(&d.pcm[..4], &[1, -1, 7, -8]);
        assert_eq!(d.pcm.len(), 28);
        assert_eq!(d.looping, None);
        // Shift 0: nibble << 12.
        let d = decode_adpcm(&block(0x00, 1, n));
        assert_eq!(&d.pcm[..2], &[4096, -4096]);
    }

    #[test]
    fn applies_prediction_filter() {
        // Filter 1 (60/64 * s1), shift 0: 4096, then 4096*60/64 = 3840, then 3600.
        let mut n = [0u8; 28];
        n[0] = 1;
        let d = decode_adpcm(&block(0x10, 1, n));
        assert_eq!(&d.pcm[..3], &[4096, 3840, 3600]);
        // Clamps to i16.
        let mut n = [7u8; 28];
        n[27] = 7;
        let d = decode_adpcm(&block(0x10, 1, n));
        assert_eq!(*d.pcm.last().unwrap(), 32767);
    }

    #[test]
    fn finds_loop_and_stops_at_end() {
        let z = [0u8; 28];
        let mut data = Vec::new();
        data.extend(block(0x0C, 0, z));
        data.extend(block(0x0C, 4, z)); // loop start
        data.extend(block(0x0C, 0, z));
        data.extend(block(0x0C, 3, z)); // end + repeat
        data.extend(block(0x0C, 0, z)); // never reached
        let d = decode_adpcm(&data);
        assert_eq!(d.pcm.len(), 4 * 28);
        assert_eq!(d.looping, Some((28, 112)));
        assert_eq!(steady_loop(&data).unwrap().len(), 84);
    }

    #[test]
    fn steady_loop_carries_history() {
        // Loop of one filtered block: the second pass starts from the first pass's tail.
        let mut n = [0u8; 28];
        n[0] = 1;
        let mut data = Vec::new();
        data.extend(block(0x00, 4, n));
        data.extend(block(0x10, 3, [0u8; 28]));
        let first = decode_adpcm(&data);
        let steady = steady_loop(&data).unwrap();
        assert_eq!(steady.len(), 56);
        assert_eq!(steady[0], 4096); // filter 0 block ignores history
        assert_eq!(steady[28], first.pcm[28]);
    }

    #[test]
    fn writes_wav_header_and_loop() {
        let w = wav(&[1, -1, 2], 22050, true);
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(
            u32::from_le_bytes(w[4..8].try_into().unwrap()) as usize,
            w.len() - 8
        );
        assert_eq!(u32::from_le_bytes(w[24..28].try_into().unwrap()), 22050);
        assert_eq!(&w[36..40], b"data");
        assert_eq!(u32::from_le_bytes(w[40..44].try_into().unwrap()), 6);
        assert_eq!(&w[50..54], b"smpl");
        // Loop end (inclusive) = last sample.
        assert_eq!(
            u32::from_le_bytes(w[w.len() - 12..w.len() - 8].try_into().unwrap()),
            2
        );
        assert_eq!(wav(&[0; 4], 8000, false).len(), 44 + 8);
    }

    #[test]
    fn parses_howl_tables_and_banks() {
        let mut d = vec![0u8; 0x28];
        d[0..4].copy_from_slice(b"HOWL");
        d[4..8].copy_from_slice(&0x80u32.to_le_bytes());
        for (o, v) in [(0x10, 2u32), (0x14, 1), (0x18, 1), (0x1C, 1)] {
            d[o..o + 4].copy_from_slice(&v.to_le_bytes());
        }
        // SPU sizes: sample 0 = 16 bytes, sample 1 = 32 bytes.
        for s in [2u16, 4] {
            d.extend(0u16.to_le_bytes());
            d.extend(s.to_le_bytes());
        }
        d.extend([2, 0x80, 0x00, 0x10, 1, 0, 0, 0]); // OtherFX: looped, 44100 Hz, sample 1
        d.extend([0, 0xFF, 0x00, 0x08, 9, 9, 0, 0]); // EngineFX: 22050 Hz, sample 0 (offset 6)
        d.extend(1u16.to_le_bytes()); // bank at sector 1
        d.resize(2048, 0);
        d.extend(2u16.to_le_bytes());
        d.extend(1u16.to_le_bytes());
        d.extend(0u16.to_le_bytes());
        d.resize(4096, 0);
        d.extend([0xAA; 32]);
        d.extend([0xBB; 16]);
        let h = Howl::parse(&d).unwrap();
        assert_eq!(h.other_fx[0].rate(), 44100);
        assert!(h.other_fx[0].looped());
        assert_eq!(h.engine_fx[0].rate(), 22050);
        assert_eq!(h.engine_fx[0].spu_index, 0);
        assert_eq!(h.sample(&h.other_fx[0]).unwrap(), &[0xAA; 32]);
        assert_eq!(h.sample(&h.engine_fx[0]).unwrap(), &[0xBB; 16]);
    }

    #[test]
    fn the_start_lights_sounds_are_in_the_catalogue() {
        // DotLights.c:26-28: OtherFX 0x45 at each red, 0x46 at green, default flags.
        for (file, id) in [("countdown.wav", 0x45), ("go.wav", 0x46)] {
            let s = SOUNDS
                .iter()
                .find(|s| s.file == file)
                .unwrap_or_else(|| panic!("{file} missing"));
            assert!(matches!(s.source, Source::Other(i) if i == id), "{file}");
            assert!(!s.loop_ && s.volume == 0xFF && s.distort == 0x80, "{file}");
        }
        let mut files: Vec<&str> = SOUNDS.iter().map(|s| s.file).collect();
        files.sort_unstable();
        files.dedup();
        assert_eq!(files.len(), SOUNDS.len(), "a file is written twice");
    }

    #[test]
    fn distortion_factors_match_game_tables() {
        assert!((other_fx_pitch_factor(0x80) - 1.0).abs() < 1e-9);
        assert!((other_fx_pitch_factor(0) - 0.5).abs() < 1e-9);
        // distortConst_OtherFX[0x94] = 0x11D48, [0x6C] = 0xE5B9
        assert!((other_fx_pitch_factor(0x94) - 0x11D48 as f64 / 65536.0).abs() < 1e-4);
        assert!((other_fx_pitch_factor(0x6C) - 0xE5B9 as f64 / 65536.0).abs() < 1e-4);
        // distortConst_Engine[0x3C] = 0x7A92, [200] = 0x22E57
        assert!((engine_pitch_factor(0x3C) - 0x7A92 as f64 / 65536.0).abs() < 1e-4);
        assert!((engine_pitch_factor(200) - 0x22E57 as f64 / 65536.0).abs() < 1e-4);
    }
}
