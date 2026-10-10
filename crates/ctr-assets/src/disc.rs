//! Raw PlayStation disc images (`.bin`, 2352-byte Mode 2 sectors) and their ISO 9660 filesystem.
//!
//! A raw sector is 12 sync bytes, a 4-byte header (minute/second/frame/mode) and, for Mode 2
//! Form 1 (what the PS1 uses for data files), an 8-byte XA subheader followed by 2048 bytes of
//! user data, then EDC/ECC. A plain 2048-byte-per-sector `.iso` is accepted too.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::{Error, Result};

pub const SECTOR_DATA: usize = 2048;
const RAW_SECTOR: u64 = 2352;
const SYNC: [u8; 12] = [
    0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0,
];

/// Where a sector's 2048 user-data bytes sit inside the image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SectorLayout {
    /// 2352-byte raw sectors; `data_offset` is 24 for Mode 2 Form 1, 16 for Mode 1.
    Raw { data_offset: u64 },
    /// 2048-byte cooked sectors (`.iso`).
    Cooked,
}

impl SectorLayout {
    /// Detects the layout from the start of sector 16 (the primary volume descriptor), given
    /// the bytes at raw offset `16 * 2352` (at least 24 + 6 bytes).
    pub fn detect(raw_pvd: &[u8], cooked_pvd: &[u8]) -> Option<Self> {
        if raw_pvd.len() >= 30 && raw_pvd[..12] == SYNC {
            let mode = raw_pvd[15];
            let data_offset = if mode == 2 { 24 } else { 16 };
            let d = data_offset as usize;
            if raw_pvd.get(d + 1..d + 6) == Some(b"CD001") {
                return Some(SectorLayout::Raw { data_offset });
            }
        }
        if cooked_pvd.get(1..6) == Some(b"CD001") {
            return Some(SectorLayout::Cooked);
        }
        None
    }
}

/// A read-only disc image.
pub struct Disc {
    file: File,
    layout: SectorLayout,
}

impl Disc {
    pub fn open(path: &Path) -> Result<Self> {
        let mut file = File::open(path).map_err(|e| Error(format!("{}: {e}", path.display())))?;
        let mut raw = vec![0u8; 64];
        let mut cooked = vec![0u8; 64];
        file.seek(SeekFrom::Start(16 * RAW_SECTOR))?;
        let n = read_up_to(&mut file, &mut raw)?;
        raw.truncate(n);
        file.seek(SeekFrom::Start(16 * SECTOR_DATA as u64))?;
        let n = read_up_to(&mut file, &mut cooked)?;
        cooked.truncate(n);
        let layout = SectorLayout::detect(&raw, &cooked)
            .ok_or_else(|| Error(format!("{}: no ISO 9660 volume found", path.display())))?;
        Ok(Self { file, layout })
    }

    pub fn layout(&self) -> SectorLayout {
        self.layout
    }

    /// Reads `count` consecutive sectors' user data starting at `lba`.
    pub fn read_sectors(&mut self, lba: u32, count: u32) -> Result<Vec<u8>> {
        let mut out = vec![0u8; count as usize * SECTOR_DATA];
        match self.layout {
            SectorLayout::Cooked => {
                self.file
                    .seek(SeekFrom::Start(lba as u64 * SECTOR_DATA as u64))?;
                self.file.read_exact(&mut out)?;
            }
            SectorLayout::Raw { data_offset } => {
                // Read whole raw runs in chunks and strip headers/ECC.
                const CHUNK: u32 = 256;
                let mut raw = vec![0u8; (CHUNK as u64 * RAW_SECTOR) as usize];
                let mut done = 0u32;
                while done < count {
                    let n = (count - done).min(CHUNK);
                    let bytes = (n as u64 * RAW_SECTOR) as usize;
                    self.file
                        .seek(SeekFrom::Start((lba + done) as u64 * RAW_SECTOR))?;
                    self.file.read_exact(&mut raw[..bytes])?;
                    for i in 0..n as usize {
                        let src = i * RAW_SECTOR as usize + data_offset as usize;
                        let dst = (done as usize + i) * SECTOR_DATA;
                        out[dst..dst + SECTOR_DATA].copy_from_slice(&raw[src..src + SECTOR_DATA]);
                    }
                    done += n;
                }
            }
        }
        Ok(out)
    }

    /// Reads `len` bytes starting at sector `lba`.
    pub fn read_bytes(&mut self, lba: u32, len: usize) -> Result<Vec<u8>> {
        let sectors = len.div_ceil(SECTOR_DATA) as u32;
        let mut data = self.read_sectors(lba, sectors)?;
        data.truncate(len);
        Ok(data)
    }

    /// Finds a file by its ISO 9660 path (case-insensitive, `;1` version suffix optional), e.g.
    /// `"BIGFILE.BIG"`. Returns `(lba, size)`.
    pub fn find_file(&mut self, path: &str) -> Result<(u32, u32)> {
        let pvd = self.read_sectors(16, 1)?;
        if &pvd[1..6] != b"CD001" || pvd[0] != 1 {
            return Err(Error("primary volume descriptor not found".into()));
        }
        let root =
            DirRecord::parse(&pvd[156..190]).ok_or_else(|| Error("bad root record".into()))?;
        let mut dir = (root.lba, root.size);
        let parts: Vec<&str> = path.split(['/', '\\']).filter(|p| !p.is_empty()).collect();
        for (i, part) in parts.iter().enumerate() {
            let data = self.read_bytes(dir.0, dir.1 as usize)?;
            let entry = list_dir(&data)
                .into_iter()
                .find(|r| names_match(&r.name, part))
                .ok_or_else(|| Error(format!("{path}: {part} not found on disc")))?;
            if i + 1 < parts.len() && !entry.is_dir {
                return Err(Error(format!("{path}: {part} is not a directory")));
            }
            dir = (entry.lba, entry.size);
        }
        Ok(dir)
    }
}

fn read_up_to(file: &mut File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match file.read(&mut buf[n..])? {
            0 => break,
            k => n += k,
        }
    }
    Ok(n)
}

/// One ISO 9660 directory record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirRecord {
    pub lba: u32,
    pub size: u32,
    pub is_dir: bool,
    pub name: String,
}

impl DirRecord {
    pub fn parse(rec: &[u8]) -> Option<Self> {
        let len = *rec.first()? as usize;
        if len < 34 || rec.len() < len {
            return None;
        }
        let lba = u32::from_le_bytes(rec[2..6].try_into().ok()?);
        let size = u32::from_le_bytes(rec[10..14].try_into().ok()?);
        let is_dir = rec[25] & 2 != 0;
        let name_len = rec[32] as usize;
        let name = String::from_utf8_lossy(rec.get(33..33 + name_len)?).into_owned();
        Some(Self {
            lba,
            size,
            is_dir,
            name,
        })
    }
}

/// Lists the records of a directory extent (records never straddle a sector boundary).
pub fn list_dir(data: &[u8]) -> Vec<DirRecord> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        let len = data[pos] as usize;
        if len == 0 {
            // Padding to the next sector.
            pos = (pos / SECTOR_DATA + 1) * SECTOR_DATA;
            continue;
        }
        if let Some(r) = DirRecord::parse(&data[pos..]) {
            if r.name != "\0" && r.name != "\u{1}" {
                out.push(r);
            }
        }
        pos += len;
    }
    out
}

fn names_match(iso_name: &str, wanted: &str) -> bool {
    let strip = |s: &str| {
        s.split(';')
            .next()
            .unwrap_or("")
            .trim_end_matches('.')
            .to_ascii_uppercase()
    };
    strip(iso_name) == strip(wanted)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(name: &str, lba: u32, size: u32, dir: bool) -> Vec<u8> {
        let mut r = vec![0u8; 33 + name.len() + (1 - name.len() % 2)];
        r[0] = r.len() as u8;
        r[2..6].copy_from_slice(&lba.to_le_bytes());
        r[6..10].copy_from_slice(&lba.to_be_bytes());
        r[10..14].copy_from_slice(&size.to_le_bytes());
        r[25] = if dir { 2 } else { 0 };
        r[32] = name.len() as u8;
        r[33..33 + name.len()].copy_from_slice(name.as_bytes());
        r
    }

    #[test]
    fn parses_directory_records() {
        let mut data = Vec::new();
        data.extend(record("\0", 20, 2048, true));
        data.extend(record("\u{1}", 20, 2048, true));
        data.extend(record("BIGFILE.BIG;1", 1234, 99, false));
        data.extend(record("SUB", 40, 2048, true));
        data.resize(2048, 0);
        let recs = list_dir(&data);
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].lba, 1234);
        assert_eq!(recs[0].size, 99);
        assert!(names_match(&recs[0].name, "bigfile.big"));
        assert!(recs[1].is_dir);
    }

    #[test]
    fn detects_mode2_form1() {
        let mut raw = vec![0u8; 64];
        raw[..12].copy_from_slice(&SYNC);
        raw[15] = 2;
        raw[24] = 1;
        raw[25..30].copy_from_slice(b"CD001");
        assert_eq!(
            SectorLayout::detect(&raw, &[]),
            Some(SectorLayout::Raw { data_offset: 24 })
        );
        let mut cooked = vec![0u8; 8];
        cooked[1..6].copy_from_slice(b"CD001");
        assert_eq!(
            SectorLayout::detect(&[0; 64], &cooked),
            Some(SectorLayout::Cooked)
        );
    }
}
