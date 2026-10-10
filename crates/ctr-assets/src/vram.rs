//! PSX VRAM (1024x512 16-bit words) as populated from CTR `.vrm` files, and texture decoding.
//!
//! A `.vrm` is either a single TIM or, when the first word is `0x20`, a stream of
//! `{u32 size, TIM}` entries ended by a zero size. A TIM is `u32 0x10`, `u32 flags`
//! (bit 3: has CLUT, bits 0-1: depth), then an optional CLUT block and an image block, each
//! `{u32 len, u16 x, u16 y, u16 w, u16 h, u16 data[w*h]}` in VRAM halfword units. CTR packs
//! its atlases as raw 16-bit blocks, so loading is a plain blit.
//!
//! Texel colours are 15-bit BGR (`0bSBBBBBGGGGGRRRRR`). PSX rules applied when decoding:
//! a texel of exactly `0x0000` is fully transparent; a texel with the STP bit (15) set is
//! blended when the primitive is semi-transparent (we export alpha 128 for those), otherwise
//! opaque.

use crate::ctr::TexLayout;
use crate::{Error, Result};

pub const VRAM_W: usize = 1024;
pub const VRAM_H: usize = 512;

pub struct Vram {
    pub words: Vec<u16>,
    /// Which halfwords were written by a loaded file (for diagnostics).
    pub written: Vec<bool>,
}

impl Default for Vram {
    fn default() -> Self {
        Self::new()
    }
}

impl Vram {
    pub fn new() -> Self {
        Self {
            words: vec![0; VRAM_W * VRAM_H],
            written: vec![false; VRAM_W * VRAM_H],
        }
    }

    pub fn get(&self, x: u32, y: u32) -> u16 {
        self.words[(y as usize % VRAM_H) * VRAM_W + (x as usize % VRAM_W)]
    }

    pub fn is_written(&self, x: u32, y: u32) -> bool {
        self.written[(y as usize % VRAM_H) * VRAM_W + (x as usize % VRAM_W)]
    }

    fn blit(&mut self, block: &Block) {
        for row in 0..block.h as usize {
            for col in 0..block.w as usize {
                let x = (block.x as usize + col) % VRAM_W;
                let y = (block.y as usize + row) % VRAM_H;
                if let Some(&v) = block.data.get(row * block.w as usize + col) {
                    self.words[y * VRAM_W + x] = v;
                    self.written[y * VRAM_W + x] = true;
                }
            }
        }
    }

    /// Loads every TIM of a `.vrm` file into VRAM.
    pub fn load_vrm(&mut self, data: &[u8]) -> Result<usize> {
        let tims = parse_vrm(data)?;
        let n = tims.len();
        for tim in tims {
            if let Some(c) = &tim.clut {
                self.blit(c);
            }
            self.blit(&tim.image);
        }
        Ok(n)
    }

    /// Raw texel index (or 15-bit colour for 16bpp) at texture-page coordinate `(u, v)`.
    fn texel(&self, page_x: u32, page_y: u32, bpp: u8, u: u32, v: u32) -> u32 {
        let y = page_y + v;
        match bpp {
            0 => {
                let w = self.get(page_x + u / 4, y);
                ((w >> ((u % 4) * 4)) & 0xF) as u32
            }
            1 => {
                let w = self.get(page_x + u / 2, y);
                ((w >> ((u % 2) * 8)) & 0xFF) as u32
            }
            _ => self.get(page_x + u, y) as u32,
        }
    }

    /// The 15-bit colour of texel `(u, v)` as seen through `layout`'s page and CLUT.
    pub fn color(&self, layout: &TexLayout, u: u32, v: u32) -> u16 {
        let t = self.texel(layout.page_x(), layout.page_y(), layout.bpp(), u, v);
        match layout.bpp() {
            0 | 1 => self.get(layout.clut_x() + t, layout.clut_y()),
            _ => t as u16,
        }
    }

    /// Decodes the texel rectangle `[u0, u0+w) x [v0, v0+h)` of `layout`'s page to RGBA8.
    pub fn decode_rgba(&self, layout: &TexLayout, u0: u32, v0: u32, w: u32, h: u32) -> Vec<u8> {
        let semi = layout.semi_transparent();
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for v in v0..v0 + h {
            for u in u0..u0 + w {
                out.extend(psx_to_rgba(self.color(layout, u, v), semi));
            }
        }
        out
    }
}

/// Converts a 15-bit PSX colour to RGBA8 (see module docs for the transparency rules).
pub fn psx_to_rgba(c: u16, semi_transparent: bool) -> [u8; 4] {
    if c == 0 {
        return [0, 0, 0, 0];
    }
    let ch = |s: u16| {
        let v = ((c >> s) & 0x1F) as u8;
        (v << 3) | (v >> 2)
    };
    let a = if c & 0x8000 != 0 && semi_transparent {
        128
    } else {
        255
    };
    [ch(0), ch(5), ch(10), a]
}

pub struct Block {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
    pub data: Vec<u16>,
}

pub struct Tim {
    pub flags: u32,
    pub clut: Option<Block>,
    pub image: Block,
}

fn word(d: &[u8], at: usize) -> Result<u32> {
    d.get(at..at + 4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .ok_or_else(|| Error("TIM truncated".into()))
}

fn parse_block(d: &[u8], at: usize) -> Result<(Block, usize)> {
    let len = word(d, at)? as usize;
    let h16 = |o: usize| -> Result<u16> {
        d.get(at + o..at + o + 2)
            .map(|b| u16::from_le_bytes(b.try_into().unwrap()))
            .ok_or_else(|| Error("TIM block truncated".into()))
    };
    let (x, y, w, h) = (h16(4)?, h16(6)?, h16(8)?, h16(10)?);
    let n = w as usize * h as usize;
    let bytes = d
        .get(at + 12..at + 12 + n * 2)
        .ok_or_else(|| Error(format!("TIM block {w}x{h} at ({x},{y}) truncated")))?;
    let data = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .collect();
    Ok((Block { x, y, w, h, data }, at + len.max(12)))
}

pub fn parse_tim(d: &[u8]) -> Result<Tim> {
    if word(d, 0)? != 0x10 {
        return Err(Error("not a TIM".into()));
    }
    let flags = word(d, 4)?;
    let mut at = 8;
    let clut = if flags & 8 != 0 {
        let (b, next) = parse_block(d, at)?;
        at = next;
        Some(b)
    } else {
        None
    };
    let (image, _) = parse_block(d, at)?;
    Ok(Tim { flags, clut, image })
}

pub fn parse_vrm(d: &[u8]) -> Result<Vec<Tim>> {
    if word(d, 0)? != 0x20 {
        return Ok(vec![parse_tim(d)?]);
    }
    let mut out = Vec::new();
    let mut at = 4;
    while at + 4 <= d.len() {
        let size = word(d, at)? as usize;
        if size == 0 {
            break;
        }
        let tim = d
            .get(at + 4..at + 4 + size)
            .ok_or_else(|| Error("VRM entry truncated".into()))?;
        out.push(parse_tim(tim)?);
        at += 4 + size;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tim(x: u16, y: u16, w: u16, h: u16, data: &[u16]) -> Vec<u8> {
        let mut d = Vec::new();
        d.extend(0x10u32.to_le_bytes());
        d.extend(2u32.to_le_bytes());
        d.extend((12 + data.len() as u32 * 2).to_le_bytes());
        for v in [x, y, w, h] {
            d.extend(v.to_le_bytes());
        }
        for v in data {
            d.extend(v.to_le_bytes());
        }
        d
    }

    #[test]
    fn loads_tim_stream_and_decodes_4bpp() {
        // Page at (64, 0): one word holding texels 1,2,0,3 (low nibble first).
        let page = tim(64, 0, 1, 1, &[0x3021]);
        // CLUT at (0, 1): index 0 = black (transparent), 1 = red, 2 = green+STP, 3 = blue.
        let clut = tim(0, 1, 4, 1, &[0x0000, 0x001F, 0x83E0, 0x7C00]);
        let mut vrm = 0x20u32.to_le_bytes().to_vec();
        for t in [&page, &clut] {
            vrm.extend((t.len() as u32).to_le_bytes());
            vrm.extend(t);
        }
        vrm.extend(0u32.to_le_bytes());
        let mut v = Vram::new();
        assert_eq!(v.load_vrm(&vrm).unwrap(), 2);
        let layout = TexLayout {
            uv: [[0; 2]; 4],
            clut: 1 << 6,
            tpage: 1,
        };
        let rgba = v.decode_rgba(&layout, 0, 0, 4, 1);
        assert_eq!(&rgba[0..4], &[255, 0, 0, 255]);
        assert_eq!(&rgba[4..8], &[0, 255, 0, 128]);
        assert_eq!(&rgba[8..12], &[0, 0, 0, 0]);
        assert_eq!(&rgba[12..16], &[0, 0, 255, 255]);
        // Opaque blend mode (3): STP texels stay opaque.
        let opaque = TexLayout {
            tpage: 1 | (3 << 5),
            ..layout
        };
        assert_eq!(&v.decode_rgba(&opaque, 1, 0, 1, 1)[..], &[0, 255, 0, 255]);
    }

    #[test]
    fn decodes_8bpp() {
        let mut v = Vram::new();
        v.load_vrm(&tim(128, 0, 1, 1, &[0x0201])).unwrap();
        v.load_vrm(&tim(0, 2, 3, 1, &[0, 0x001F, 0x7C00])).unwrap();
        let layout = TexLayout {
            uv: [[0; 2]; 4],
            clut: 2 << 6,
            tpage: 2 | (1 << 7) | (3 << 5),
        };
        assert_eq!(
            v.decode_rgba(&layout, 0, 0, 2, 1),
            vec![255, 0, 0, 255, 0, 0, 255, 255]
        );
    }
}
