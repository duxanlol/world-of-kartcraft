//! `BIGFILE.BIG`: CTR's packed archive. Header: `u32 0`, `u32 count`, then `count` entries of
//! `{u32 offset_in_sectors, u32 size_in_bytes}` (little endian). Entry `i` lives at
//! `offset * 2048` from the start of the bigfile. Entries have no names; the NTSC-U retail
//! indices are fixed (see [`index`]).

use crate::disc::Disc;
use crate::{Error, Result};

/// NTSC-U (SCUS-94426) retail bigfile indices (CTR-tools `big_usa_release.txt`, CTR-native
/// `namespace_Load.h`).
pub mod index {
    /// `models\racers\hi\<name>.ctr`, + character id.
    pub const RACER_HI: usize = 242;
    /// `packs\shared.vrm` (wheel sprites, shared HUD textures, racer textures).
    pub const SHARED_VRM: usize = 258;
    /// `packs\shared.mpk` (icon groups incl. the tire sprites, shared models).
    pub const SHARED_MPK: usize = 259;
    /// `models\racers\med\<name>.ctr`, + character id.
    pub const RACER_MED: usize = 308;
    /// `models\racers\low\<name>.ctr`, + character id.
    pub const RACER_LOW: usize = 332;
}

/// Character ids in bigfile order (`GAME_CHARACTER_IDS`).
pub const CHARACTERS: [&str; 16] = [
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
    "ntropy",
    "pen",
    "fake",
    "oxide",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BigEntry {
    pub offset_sectors: u32,
    pub size: u32,
}

/// Parses the bigfile table of contents from its first bytes.
pub fn parse_toc(data: &[u8]) -> Result<Vec<BigEntry>> {
    let word = |i: usize| -> Result<u32> {
        data.get(i..i + 4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
            .ok_or_else(|| Error("bigfile header truncated".into()))
    };
    if word(0)? != 0 {
        return Err(Error("bigfile magic is not 0".into()));
    }
    let count = word(4)? as usize;
    if count == 0 || count > 4096 {
        return Err(Error(format!("implausible bigfile entry count {count}")));
    }
    (0..count)
        .map(|i| {
            Ok(BigEntry {
                offset_sectors: word(8 + i * 8)?,
                size: word(12 + i * 8)?,
            })
        })
        .collect()
}

/// An opened bigfile on a disc.
pub struct BigFile<'a> {
    disc: &'a mut Disc,
    lba: u32,
    pub entries: Vec<BigEntry>,
}

impl<'a> BigFile<'a> {
    pub fn open(disc: &'a mut Disc) -> Result<Self> {
        let (lba, _size) = disc.find_file("BIGFILE.BIG")?;
        let head = disc.read_sectors(lba, 1)?;
        let count = u32::from_le_bytes(head[4..8].try_into().unwrap()) as usize;
        let toc_bytes = 8 + count * 8;
        let toc = disc.read_bytes(lba, toc_bytes)?;
        let entries = parse_toc(&toc)?;
        Ok(Self { disc, lba, entries })
    }

    pub fn read(&mut self, index: usize) -> Result<Vec<u8>> {
        let e = *self
            .entries
            .get(index)
            .ok_or_else(|| Error(format!("bigfile has no entry {index}")))?;
        self.disc
            .read_bytes(self.lba + e.offset_sectors, e.size as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_toc() {
        let mut d = vec![0u8; 8];
        d[4..8].copy_from_slice(&2u32.to_le_bytes());
        for (o, s) in [(1u32, 100u32), (2, 3000)] {
            d.extend(o.to_le_bytes());
            d.extend(s.to_le_bytes());
        }
        let toc = parse_toc(&d).unwrap();
        assert_eq!(
            toc,
            vec![
                BigEntry {
                    offset_sectors: 1,
                    size: 100
                },
                BigEntry {
                    offset_sectors: 2,
                    size: 3000
                }
            ]
        );
        d[0] = 1;
        assert!(parse_toc(&d).is_err());
    }
}
