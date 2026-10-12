//! CTR's character voices: the CD-XA voice lines (`XA\ENG\GAME\Sxx.XA`, indexed by
//! `XA\ENG.XNF`) and the two short lines per character in the HOWL bank, written as WAV under
//! `audio/voice/<racer>/` with `voice/voices.json`.
//!
//! ```text
//! XNF  { char magic[4] = "XINF"; u32 version (102); u32 num_types (3: music, extra, game);
//!        u32 num_xas_total; u32 num_tracks_total; u32 num_xa[3]; u32 first_xa_index[3];
//!        u32 num_songs[3]; u32 first_song_index[3] }                       // 0x44 bytes
//!      i32 xa_cd_pos[num_xas_total]                                       // 0 on disc
//!      XaSize { u8 channel; u8 file; s16 sectors }[num_tracks_total]
//! ```
//!
//! A game track `xaID` is `XaSize[first_song_index[2] + xaID]`: the interleaved file
//! `Sfile.XA`, read from its start for `sectors` sectors, keeping the audio sectors of that
//! channel (`CDSYS.c:658-760` `CDSYS_XAPlay`, CTR-native `platform/native_audio.c`
//! `NativeAudio_LookupXATrackInfo`). Which tracks a character says for a voice line is
//! `data.voiceData[character].voiceSet[set]` (`zGlobal_DATA.c:3100-3368`).

use crate::audio::{render, Howl, SoundSpec, Source};
use crate::disc::Disc;
use crate::{Error, Result};

/// The XA index of the voices.
pub const XNF_PATH: &str = "XA/ENG.XNF";
/// `enum XA_TYPE` (`namespace_Cdsys.h`): the voice lines are `CDSYS_XA_TYPE_GAME`.
const XA_TYPE_GAME: usize = 2;
const XNF_HEADER: usize = 0x44;

/// Every character in `enum Characters` order (`namespace_Vehicle.h:37-55`), by the names
/// `ctr-extract` gives their karts.
pub const RACERS: [&str; 16] = [
    "crash",
    "cortex",
    "tiny",
    "coco",
    "ngin",
    "dingo",
    "polar",
    "pura",
    "pinstripe",
    "papu",
    "roo",
    "joe",
    "tropy",
    "pen",
    "fake",
    "oxide",
];

/// A character's voice sets (`data.voiceData[c].voiceSet[0..11]`, `zGlobal_DATA.c:3100-3368`):
/// set k < 8 holds tracks `19c + 2k` and `19c + 2k + 1`, sets 8 to 10 one track each,
/// `19c + 16 .. 18`. Ripper Roo's set 0 is tracks 200 and 201 (his set 5's), not his own.
pub fn voice_sets(character: usize) -> [Vec<u16>; 11] {
    let base = 19 * character as u16;
    std::array::from_fn(|k| {
        let k = k as u16;
        if k < 8 {
            if character == 10 && k == 0 {
                vec![0xC8, 0xC9]
            } else {
                vec![base + 2 * k, base + 2 * k + 1]
            }
        } else {
            vec![base + 8 + k]
        }
    })
}

/// One game XA track's place on the disc.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct XaTrack {
    pub channel: u8,
    pub file: u8,
    pub sectors: u16,
}

fn u32_at(d: &[u8], at: usize) -> Result<usize> {
    d.get(at..at + 4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize)
        .ok_or_else(|| Error(format!("XNF truncated at {at:#x}")))
}

/// The game tracks of an `ENG.XNF`, by `xaID`.
pub fn parse_xnf(d: &[u8]) -> Result<Vec<XaTrack>> {
    if d.get(0..4) != Some(b"XINF") || u32_at(d, 4)? != 102 || u32_at(d, 8)? != 3 {
        return Err(Error("not a CTR XNF (XINF, version 102, 3 types)".into()));
    }
    let xas = u32_at(d, 0xC)?;
    let tracks = u32_at(d, 0x10)?;
    let songs = u32_at(d, 0x2C + XA_TYPE_GAME * 4)?;
    let first = u32_at(d, 0x38 + XA_TYPE_GAME * 4)?;
    if xas > 4096 || tracks > 4096 || first + songs > tracks {
        return Err(Error("implausible XNF header".into()));
    }
    let sizes = XNF_HEADER + xas * 4;
    (first..first + songs)
        .map(|i| {
            let e = d
                .get(sizes + i * 4..sizes + i * 4 + 4)
                .ok_or_else(|| Error(format!("XNF track {i} truncated")))?;
            Ok(XaTrack {
                channel: e[0],
                file: e[1],
                sectors: u16::from_le_bytes([e[2], e[3]]),
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// CD-XA ADPCM (PSX-SPX "CDROM XA Audio ADPCM Compression"; native_audio.c:1774-1904)
// ---------------------------------------------------------------------------------------------

const RAW_SECTOR: usize = 2352;
const SUBHEADER: usize = 16;
const GROUPS: usize = 18;
const GROUP: usize = 128;
const POS: [i32; 5] = [0, 60, 115, 98, 122];
const NEG: [i32; 5] = [0, 0, -52, -55, -60];

/// Decoded XA audio: interleaved PCM, its channel count and rate.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct XaAudio {
    pub pcm: Vec<i16>,
    pub channels: u16,
    pub rate: u32,
}

/// Whether a raw sector is 4-bit XA audio of file 1, `channel` (`NativeAudio_IsXAAudioSector`).
fn is_audio(sector: &[u8], channel: u8) -> bool {
    let h = &sector[SUBHEADER..SUBHEADER + 4];
    h[2] & 0x04 != 0 && h[0] == 1 && h[1] == channel && (h[3] >> 4) & 3 == 0
}

/// 28 samples of one sound unit: `param` its shift/filter byte, `byte(i)` the i-th data byte,
/// the nibble in its high (`high`) or low half.
fn unit(
    param: u8,
    byte: impl Fn(usize) -> u8,
    high: bool,
    hist: &mut [i32; 2],
    out: &mut impl FnMut(i16),
) {
    let shift = i32::from(param & 0xF);
    let filter = usize::from(param >> 4).min(4);
    for i in 0..28 {
        let b = byte(i);
        let nib = if high { b & 0xF0 } else { (b & 0xF) << 4 };
        let mut s = (i32::from(nib as i8) * 0x100) >> shift;
        s += (hist[0] * POS[filter]) >> 6;
        s += (hist[1] * NEG[filter]) >> 6;
        let s = s.clamp(-32768, 32767);
        hist[1] = hist[0];
        hist[0] = s;
        out(s as i16);
    }
}

/// Decodes one raw audio sector into `out`, carrying each channel's filter history.
pub fn decode_sector(sector: &[u8], hist: &mut [[i32; 2]; 2], out: &mut Vec<i16>) {
    let stereo = sector[SUBHEADER + 3] & 3 != 0;
    for g in 0..GROUPS {
        let at = SUBHEADER + 8 + g * GROUP;
        let group = &sector[at..at + GROUP];
        if stereo {
            for block in 0..4 {
                let mut left = Vec::with_capacity(28);
                let mut right = Vec::with_capacity(28);
                let byte = |i: usize| group[16 + i * 4 + block];
                unit(group[4 + block * 2], byte, false, &mut hist[0], &mut |s| {
                    left.push(s)
                });
                unit(
                    group[4 + block * 2 + 1],
                    byte,
                    true,
                    &mut hist[1],
                    &mut |s| right.push(s),
                );
                for (l, r) in left.into_iter().zip(right) {
                    out.push(l);
                    out.push(r);
                }
            }
        } else {
            for su in 0..8 {
                let param = group[4 + ((su & 3) | ((su & 4) << 1))];
                let byte = |i: usize| group[16 + i * 4 + (su >> 1)];
                unit(param, byte, su & 1 == 1, &mut hist[0], &mut |s| out.push(s));
            }
        }
    }
}

/// One channel of an interleaved XA file's raw sectors.
pub fn decode_channel(raw: &[u8], channel: u8) -> XaAudio {
    let mut audio = XaAudio::default();
    let mut hist = [[0i32; 2]; 2];
    for sector in raw.as_chunks::<RAW_SECTOR>().0 {
        if !is_audio(sector, channel) {
            continue;
        }
        let coding = sector[SUBHEADER + 3];
        audio.channels = if coding & 3 != 0 { 2 } else { 1 };
        audio.rate = if (coding >> 2) & 3 == 0 { 37800 } else { 18900 };
        decode_sector(sector, &mut hist, &mut audio.pcm);
    }
    audio
}

/// One voice line written.
pub struct VoiceFile {
    pub racer: &'static str,
    pub file: String,
    pub seconds: f64,
}

fn write_wav(path: &std::path::Path, wav: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| Error(format!("{}: {e}", dir.display())))?;
    }
    std::fs::write(path, wav).map_err(|e| Error(format!("{}: {e}", path.display())))
}

/// The HOWL bank's two lines of `character`, `Voiceline_RequestPlay`'s immediate plays
/// (`HOWL_Voiceline.c:203-210`): voice type 0 is OtherFX `0x1C + c`, type 1 `0x2C + c`.
pub fn bank_line(howl: &Howl, character: usize, voice_type: usize) -> Result<Vec<u8>> {
    let base = if voice_type == 0 { 0x1C } else { 0x2C };
    let spec = SoundSpec {
        file: "",
        source: Source::Other(base + character),
        loop_: false,
        distort: 0x80,
        volume: 0xFF,
        reference: "",
    };
    Ok(render(howl, &spec)?.wav)
}

/// Writes every character's voices into `dir` (`<racer>/fx0.wav`, `<racer>/fx1.wav` from the
/// HOWL bank; `<racer>/<set>_<n>.wav` from XA when the image is raw) and `voices.json`. A
/// cooked `.iso` has no XA audio: only the bank's lines are written, and the manifest says so.
pub fn extract(disc: &mut Disc, dir: &std::path::Path) -> Result<Vec<VoiceFile>> {
    let (lba, size) = disc.find_file(crate::audio::HWL_PATH)?;
    let howl = Howl::parse(&disc.read_bytes(lba, size as usize)?)?;
    std::fs::create_dir_all(dir).map_err(|e| Error(format!("{}: {e}", dir.display())))?;
    let mut out = Vec::new();

    // The XA tracks, every file read once and split into its channels.
    let (xlba, xsize) = disc.find_file(XNF_PATH)?;
    let tracks = parse_xnf(&disc.read_bytes(xlba, xsize as usize)?)?;
    let mut decoded: std::collections::HashMap<u16, XaAudio> = Default::default();
    let mut files: Vec<u8> = tracks.iter().map(|t| t.file).collect();
    files.sort_unstable();
    files.dedup();
    let mut xa_available = true;
    for file in files {
        let path = format!("XA/ENG/GAME/S{file:02}.XA");
        let (flba, _) = disc.find_file(&path)?;
        let span = tracks
            .iter()
            .filter(|t| t.file == file)
            .map(|t| u32::from(t.sectors))
            .max()
            .unwrap_or(0);
        let Some(raw) = disc.read_raw_sectors(flba, span)? else {
            xa_available = false;
            break;
        };
        for (id, t) in tracks.iter().enumerate() {
            if t.file == file {
                // Only as far as this track's own span (`XA_EndPos`).
                let end = (usize::from(t.sectors) * RAW_SECTOR).min(raw.len());
                decoded.insert(id as u16, decode_channel(&raw[..end], t.channel));
            }
        }
    }

    let mut racers = serde_json::Map::new();
    for (c, racer) in RACERS.iter().enumerate() {
        let mut bank = Vec::new();
        for t in 0..2 {
            // A line no bank holds is left out (its manifest entry null).
            let Ok(wav) = bank_line(&howl, c, t) else {
                bank.push(serde_json::Value::Null);
                continue;
            };
            let file = format!("{racer}/fx{t}.wav");
            write_wav(&dir.join(&file), &wav)?;
            bank.push(serde_json::json!(file));
        }
        let mut sets = Vec::new();
        for (k, set) in voice_sets(c).iter().enumerate() {
            let mut lines = Vec::new();
            for (n, &id) in set.iter().enumerate() {
                let Some(a) = decoded.get(&id).filter(|a| !a.pcm.is_empty()) else {
                    continue;
                };
                let mut pcm = a.pcm.clone();
                while pcm.len() > 1 && pcm.last() == Some(&0) {
                    pcm.pop();
                }
                let file = format!("{racer}/{k}_{n}.wav");
                write_wav(
                    &dir.join(&file),
                    &crate::audio::wav_channels(&pcm, a.channels, a.rate, false),
                )?;
                let seconds = pcm.len() as f64 / f64::from(a.channels) / f64::from(a.rate);
                lines.push(serde_json::json!({
                    "file": file,
                    "xa": id,
                    "sectors": tracks[usize::from(id)].sectors,
                    "seconds": seconds,
                }));
                out.push(VoiceFile {
                    racer,
                    file,
                    seconds,
                });
            }
            sets.push(serde_json::Value::Array(lines));
        }
        racers.insert(
            racer.to_string(),
            serde_json::json!({ "character": c, "bank": bank, "sets": sets }),
        );
    }
    let doc = serde_json::json!({
        "source": "XA/ENG.XNF + XA/ENG/GAME/Sxx.XA (CDSYS_XA_TYPE_GAME) and SOUNDS/KART.HWL OtherFX 0x1C + c, 0x2C + c",
        "xa": xa_available,
        "bank": "fx0 = voice type 0 (OtherFX 0x1C + character), fx1 = voice type 1 (0x2C + character): Voiceline_RequestPlay's immediate plays (HOWL_Voiceline.c:203-210)",
        "sets": "sets[k] = data.voiceData[character].voiceSet[k] (zGlobal_DATA.c:3100-3368); a voice line picks set data.voiceID[voiceID] (zGlobal_DATA.c:3369) and one of its tracks at random (Voiceline_StartPlay); 'sectors' is the track's XaBytes, its cooldown sectors / 5 + 30 frames",
        "racers": racers,
    });
    let path = dir.join("voices.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&doc).unwrap())
        .map_err(|e| Error(format!("{}: {e}", path.display())))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_sets_follow_the_game_s_table() {
        // Crash: index {0..16, 0, 17, 0, 18, 0}.
        let crash = voice_sets(0);
        assert_eq!(crash[0], [0, 1]);
        assert_eq!(crash[7], [14, 15]);
        assert_eq!(crash[8], [16]);
        assert_eq!(crash[10], [18]);
        // Oxide: 285.. (zGlobal_DATA.c [15]).
        assert_eq!(voice_sets(15)[0], [285, 286]);
        assert_eq!(voice_sets(15)[10], [303]);
        // Ripper Roo's set 0 is 0xC8, 0xC9, the rest his own from 190.
        let roo = voice_sets(10);
        assert_eq!(roo[0], [0xC8, 0xC9]);
        assert_eq!(roo[1], [0xC0, 0xC1]);
        assert_eq!(roo[10], [0xD0]);
        assert_eq!(RACERS[10], "roo");
    }

    #[test]
    fn parses_the_xnf_game_tracks() {
        let mut d = vec![0u8; XNF_HEADER];
        d[0..4].copy_from_slice(b"XINF");
        let mut put = |at: usize, v: u32| d[at..at + 4].copy_from_slice(&v.to_le_bytes());
        put(4, 102);
        put(8, 3);
        put(0xC, 2); // two XA files
        put(0x10, 3); // three tracks
        put(0x2C + 8, 2); // two game tracks
        put(0x38 + 8, 1); // from entry 1
        d.extend([0u8; 8]); // xa_cd_pos
        d.extend([0, 0, 0, 0]);
        d.extend([3, 1, 40, 0]);
        d.extend([5, 2, 0x10, 1]);
        let t = parse_xnf(&d).unwrap();
        assert_eq!(
            t,
            [
                XaTrack {
                    channel: 3,
                    file: 1,
                    sectors: 40
                },
                XaTrack {
                    channel: 5,
                    file: 2,
                    sectors: 0x110
                }
            ]
        );
        assert!(parse_xnf(b"HOWL").is_err());
    }

    fn sector(channel: u8, coding: u8) -> Vec<u8> {
        let mut s = vec![0u8; RAW_SECTOR];
        s[SUBHEADER..SUBHEADER + 4].copy_from_slice(&[1, channel, 0x64, coding]);
        s
    }

    #[test]
    fn decodes_mono_xa_by_channel() {
        // Group 0: unit 0 shift 12 filter 0 (params byte 4), first data byte nibble 1 -> 1.
        let mut a = sector(2, 0x00);
        let g = SUBHEADER + 8;
        a[g + 4] = 0x0C;
        a[g + 16] = 0x01;
        // Unit 1 (high nibbles of byte column 0) shift 0: nibble 0xF -> -4096.
        a[g + 5] = 0x00;
        a[g + 16 + 4] = 0xF0;
        let other = sector(3, 0x00);
        let mut raw = a.clone();
        raw.extend(other);
        let x = decode_channel(&raw, 2);
        assert_eq!((x.channels, x.rate), (1, 37800));
        assert_eq!(x.pcm.len(), GROUPS * 8 * 28);
        assert_eq!(x.pcm[0], 1);
        // Unit 1 starts at sample 28; its second sample reads data byte 4's high nibble.
        assert_eq!(x.pcm[29], -4096);
        // 18.9 kHz stereo.
        let s = decode_channel(&sector(0, 0x05), 0);
        assert_eq!((s.channels, s.rate), (2, 18900));
        assert_eq!(s.pcm.len(), GROUPS * 4 * 28 * 2);
        assert!(decode_channel(&a, 7).pcm.is_empty());
    }
}
