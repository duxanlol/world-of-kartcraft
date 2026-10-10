//! CTR dynamic models (`.ctr`): the racer models (kart + driver in one mesh), weapons, etc.
//!
//! File: `u32 data_size`, `data[data_size]`, `u32 map_size`, `u32 pointer_map[map_size / 4]`.
//! Every pointer inside `data` is an offset from the start of `data` (the map lists where the
//! pointers are so the game can relocate them; we just treat them as offsets).
//!
//! ```text
//! Model       { char name[16]; s16 thread_id; s16 num_headers; u32 ptr_headers; }
//! ModelHeader { char name[16]; i32 unk; s16 lod_distance; u16 flags; s16 scale[3]; s16 pad;
//!               u32 ptr_commands; u32 ptr_frame /* null if animated */; u32 ptr_tex_layouts;
//!               u32 ptr_colors; u32 unk3; u32 num_anims; u32 ptr_anims; u32 anim_tex; } // 0x40
//! ModelFrame  { s16 pos[3]; s16 pad; u8 unk[16]; u32 vertex_offset /* 0x1C */; u8 verts[][3] }
//! ModelAnim   { char name[16]; u16 num_frames /* bit15: 30fps-interpolated */; s16 frame_size;
//!               u32 ptr_deltas /* null if uncompressed */; ModelFrame frames[] /* frame_size each */ }
//! ```
//!
//! The command list is `u32 scratch_color_count` followed by `u32` draw commands terminated by
//! `0xFFFFFFFF`; see [`Command`] and [`ModelMesh::triangles`]. Decoding follows CTR-native's
//! `RenderBucket_DrawFunc_Normal` / `RenderBucket_UncompressAnimationFrame`.

use crate::{Error, Result};

/// A little-endian view of a model's relocated data block.
#[derive(Clone, Copy)]
pub struct Bytes<'a>(pub &'a [u8]);

impl<'a> Bytes<'a> {
    fn get(&self, at: usize, len: usize) -> Result<&'a [u8]> {
        self.0
            .get(
                at..at
                    .checked_add(len)
                    .ok_or_else(|| Error("offset overflow".into()))?,
            )
            .ok_or_else(|| {
                Error(format!(
                    "read of {len} bytes at {at:#x} is out of bounds ({:#x})",
                    self.0.len()
                ))
            })
    }
    pub fn u8(&self, at: usize) -> Result<u8> {
        Ok(self.get(at, 1)?[0])
    }
    pub fn u16(&self, at: usize) -> Result<u16> {
        Ok(u16::from_le_bytes(self.get(at, 2)?.try_into().unwrap()))
    }
    pub fn i16(&self, at: usize) -> Result<i16> {
        Ok(self.u16(at)? as i16)
    }
    pub fn u32(&self, at: usize) -> Result<u32> {
        Ok(u32::from_le_bytes(self.get(at, 4)?.try_into().unwrap()))
    }
    pub fn name(&self, at: usize) -> Result<String> {
        let raw = self.get(at, 16)?;
        let end = raw.iter().position(|&b| b == 0).unwrap_or(16);
        Ok(String::from_utf8_lossy(&raw[..end]).into_owned())
    }
}

/// Strips the `{size, data, pointer map}` wrapper and returns `data`.
pub fn unwrap_patched(file: &[u8]) -> Result<&[u8]> {
    let size = Bytes(file).u32(0)? as usize;
    file.get(4..4 + size)
        .ok_or_else(|| Error("patched container truncated".into()))
}

/// PSX texture page/CLUT mapping for one triangle (`TextureLayout`, 12 bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TexLayout {
    pub uv: [[u8; 2]; 4],
    /// Raw CLUT word: x / 16 in bits 0-5, y in bits 6-14.
    pub clut: u16,
    /// Raw tpage word: x / 64 in bits 0-3, y / 256 in bit 4, semi-transparency in bits 5-6,
    /// colour depth (0 = 4bpp, 1 = 8bpp, 2 = 15bpp) in bits 7-8.
    pub tpage: u16,
}

impl TexLayout {
    pub fn parse(b: Bytes, at: usize) -> Result<Self> {
        Ok(Self {
            uv: [
                [b.u8(at)?, b.u8(at + 1)?],
                [b.u8(at + 4)?, b.u8(at + 5)?],
                [b.u8(at + 8)?, b.u8(at + 9)?],
                [b.u8(at + 10)?, b.u8(at + 11)?],
            ],
            clut: b.u16(at + 2)?,
            tpage: b.u16(at + 6)?,
        })
    }
    pub fn page_x(&self) -> u32 {
        (self.tpage as u32 & 0xF) * 64
    }
    pub fn page_y(&self) -> u32 {
        ((self.tpage as u32 >> 4) & 1) * 256
    }
    pub fn blend_mode(&self) -> u8 {
        ((self.tpage >> 5) & 3) as u8
    }
    pub fn bpp(&self) -> u8 {
        ((self.tpage >> 7) & 3) as u8
    }
    pub fn clut_x(&self) -> u32 {
        (self.clut as u32 & 0x3F) * 16
    }
    pub fn clut_y(&self) -> u32 {
        (self.clut as u32 >> 6) & 0x1FF
    }
    /// The game draws a model primitive as semi-transparent (PSX GT3 code 0x36) unless the
    /// tpage's blend mode is 3 (code 0x34). Only texels with the STP bit set blend.
    pub fn semi_transparent(&self) -> bool {
        self.blend_mode() != 3
    }
}

/// One 32-bit draw command: `s l n d k v ? ? | stack index (8) | color (7) | tex (9)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Command(pub u32);

impl Command {
    pub fn flags(self) -> u8 {
        (self.0 >> 24) as u8
    }
    /// `s`: restart the triangle strip.
    pub fn new_strip(self) -> bool {
        self.flags() & 0x80 != 0
    }
    /// `l`: the new triangle reuses the strip's older vertex (fan-like turn).
    pub fn swap_first(self) -> bool {
        self.flags() & 0x40 != 0
    }
    /// `n`: flip the winding used for back-face culling.
    pub fn flip_normal(self) -> bool {
        self.flags() & 0x20 != 0
    }
    /// `d`: back-face cull (single sided); otherwise double sided.
    pub fn cull(self) -> bool {
        self.flags() & 0x10 != 0
    }
    /// `v`: take the vertex from the stack slot instead of the next one in the vertex array.
    pub fn from_stack(self) -> bool {
        self.flags() & 0x04 != 0
    }
    pub fn stack_index(self) -> usize {
        ((self.0 >> 16) & 0xFF) as usize
    }
    pub fn color_index(self) -> usize {
        ((self.0 >> 9) & 0x7F) as usize
    }
    /// 0 = untextured; otherwise `tex_layouts[tex_index - 1]`.
    pub fn tex_index(self) -> usize {
        (self.0 & 0x1FF) as usize
    }
    /// A command whose upper 16 bits are zero only loads colours.
    pub fn color_only(self) -> bool {
        self.0 >> 16 == 0
    }
}

/// One decoded vertex frame: the frame origin and the byte-quantized vertices in raw order
/// (`[x, z, y]` as stored; see [`ModelMesh::position`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub pos: [i16; 3],
    pub verts: Vec<[u8; 3]>,
}

#[derive(Clone, Debug)]
pub struct Anim {
    pub name: String,
    pub frames: Vec<Frame>,
}

/// One LOD mesh of a model.
#[derive(Clone, Debug)]
pub struct ModelMesh {
    pub name: String,
    pub flags: u16,
    pub scale: [i16; 3],
    pub commands: Vec<Command>,
    pub colors: Vec<[u8; 4]>,
    pub tex_layouts: Vec<TexLayout>,
    /// The static frame, or the first frame of the first animation.
    pub frame: Frame,
    pub anims: Vec<Anim>,
}

#[derive(Clone, Debug)]
pub struct Model {
    pub name: String,
    pub meshes: Vec<ModelMesh>,
}

/// One assembled triangle in model space, as the game would draw it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Triangle {
    /// Raw vertex indices into the frame's vertex array (for grouping/diagnostics).
    pub vert_ids: [usize; 3],
    /// Model-space positions in CTR units (see [`ModelMesh::position`]).
    pub positions: [[f32; 3]; 3],
    pub colors: [[u8; 4]; 3],
    /// `Some` for textured triangles: layout index into `tex_layouts`; corner `i` uses `uv[i]`.
    pub tex: Option<usize>,
    pub cull: bool,
    /// Winding flip for culling (`n` flag of the command that drew it).
    pub flip: bool,
}

impl Model {
    /// Parses a model from a relocated data block (see [`unwrap_patched`]).
    pub fn parse(data: &[u8]) -> Result<Self> {
        Self::parse_at(data, 0)
    }

    /// Parses the model header at offset `at` of a relocated block (models inside an `.mpk`).
    pub fn parse_at(data: &[u8], at: usize) -> Result<Self> {
        let b = Bytes(data);
        let name = b.name(at)?;
        let num = b.i16(at + 0x12)?;
        let ptr = b.u32(at + 0x14)? as usize;
        if !(1..=16).contains(&num) {
            return Err(Error(format!(
                "model {name}: implausible header count {num}"
            )));
        }
        let meshes = (0..num as usize)
            .map(|i| ModelMesh::parse(b, ptr + i * 0x40))
            .collect::<Result<_>>()?;
        Ok(Self { name, meshes })
    }

    /// Parses a `.ctr` file as stored in the bigfile.
    pub fn parse_file(file: &[u8]) -> Result<Self> {
        Self::parse(unwrap_patched(file)?)
    }
}

impl ModelMesh {
    pub fn parse(b: Bytes, at: usize) -> Result<Self> {
        let name = b.name(at)?;
        let flags = b.u16(at + 0x16)?;
        let scale = [b.i16(at + 0x18)?, b.i16(at + 0x1A)?, b.i16(at + 0x1C)?];
        let ptr_cmd = b.u32(at + 0x20)? as usize;
        let ptr_frame = b.u32(at + 0x24)? as usize;
        let ptr_tex = b.u32(at + 0x28)? as usize;
        let ptr_colors = b.u32(at + 0x2C)? as usize;
        let num_anims = b.u32(at + 0x34)? as usize;
        let ptr_anims = b.u32(at + 0x38)? as usize;

        let mut commands = Vec::new();
        let mut pos = ptr_cmd + 4;
        loop {
            let c = b.u32(pos)?;
            pos += 4;
            if c == 0xFFFF_FFFF {
                break;
            }
            commands.push(Command(c));
            if commands.len() > 100_000 {
                return Err(Error(format!("mesh {name}: unterminated command list")));
            }
        }
        let mut num_verts = 0;
        let mut max_color = 0;
        let mut max_tex = 0;
        for c in &commands {
            if c.color_only() {
                max_color = max_color
                    .max(c.color_index())
                    .max(((c.0 & 0x1FC) >> 2) as usize);
                continue;
            }
            if !c.from_stack() {
                num_verts += 1;
            }
            max_color = max_color.max(c.color_index());
            max_tex = max_tex.max(c.tex_index());
        }
        let colors = (0..=max_color)
            .map(|i| {
                let w = b.u32(ptr_colors + i * 4)?;
                Ok(w.to_le_bytes())
            })
            .collect::<Result<Vec<_>>>()?;
        let tex_layouts = (0..max_tex)
            .map(|i| TexLayout::parse(b, b.u32(ptr_tex + i * 4)? as usize))
            .collect::<Result<Vec<_>>>()?;

        let mut anims = Vec::new();
        for i in 0..num_anims {
            let a = b.u32(ptr_anims + i * 4)? as usize;
            anims.push(parse_anim(b, a, num_verts)?);
        }
        let frame = if num_anims == 0 {
            parse_frame(b, ptr_frame, num_verts, None)?
        } else {
            anims
                .first()
                .and_then(|a| a.frames.first())
                .cloned()
                .ok_or_else(|| Error(format!("mesh {name}: animation without frames")))?
        };
        Ok(Self {
            name,
            flags,
            scale,
            commands,
            colors,
            tex_layouts,
            frame,
            anims,
        })
    }

    /// Model-space position of a raw vertex in CTR units: `(byte + frame.pos) * scale / 4096`,
    /// with the stored `[x, z, y]` byte order resolved to `[x, y, z]` (CTR-native
    /// `RenderBucket_PackModelVertexXY`: VX = b0 + pos.x, VY = b2 + pos.y, VZ = b1 + pos.z,
    /// then the header scale goes through the GTE light matrix as 4.12 fixed point).
    pub fn position(&self, frame: &Frame, v: [u8; 3]) -> [f32; 3] {
        let s = |i: usize| self.scale[i] as f32 / 4096.0;
        [
            (v[0] as f32 + frame.pos[0] as f32) * s(0),
            (v[2] as f32 + frame.pos[1] as f32) * s(1),
            (v[1] as f32 + frame.pos[2] as f32) * s(2),
        ]
    }

    /// Assembles the triangle strips of `frame` exactly like `RenderBucket_DrawFunc_Normal`.
    pub fn triangles(&self, frame: &Frame) -> Result<Vec<Triangle>> {
        let mut stack_pos = [[0f32; 3]; 256];
        let mut stack_id = [usize::MAX; 256];
        let mut coords = [[0f32; 3]; 4];
        let mut ids = [usize::MAX; 4];
        let mut colors = [[0u8; 4]; 4];
        let mut strip_len = 0usize;
        let mut prim_cmd = Command(0);
        let mut vertex_index = 0usize;
        let mut out = Vec::new();
        let color = |i: usize| self.colors.get(i).copied().unwrap_or([0x80, 0x80, 0x80, 0]);

        for &cmd in &self.commands {
            if cmd.color_only() {
                let a = color(cmd.color_index());
                let bcol = color(((cmd.0 & 0x1FC) >> 2) as usize);
                colors[1] = a;
                colors[2] = a;
                colors[3] = bcol;
                continue;
            }
            let si = cmd.stack_index();
            if !cmd.from_stack() {
                let v = *frame.verts.get(vertex_index).ok_or_else(|| {
                    Error(format!(
                        "mesh {}: vertex {vertex_index} out of range",
                        self.name
                    ))
                })?;
                stack_pos[si] = self.position(frame, v);
                stack_id[si] = vertex_index;
                vertex_index += 1;
            }
            coords.rotate_left(1);
            coords[3] = stack_pos[si];
            ids.rotate_left(1);
            ids[3] = stack_id[si];
            colors.rotate_left(1);
            colors[3] = color(cmd.color_index());

            if cmd.new_strip() || strip_len == 0 {
                prim_cmd = cmd;
            }
            if cmd.new_strip() {
                strip_len = 0;
            }
            if strip_len > 2 && cmd.swap_first() {
                coords[1] = coords[0];
                ids[1] = ids[0];
                colors[1] = colors[0];
            }
            if strip_len < 2 {
                strip_len += 1;
                continue;
            }
            let draw = if strip_len == 2 { prim_cmd } else { cmd };
            strip_len += 1;
            let tex = match draw.tex_index() {
                0 => None,
                t if t - 1 < self.tex_layouts.len() => Some(t - 1),
                t => {
                    return Err(Error(format!(
                        "mesh {}: texture {t} out of range",
                        self.name
                    )))
                }
            };
            out.push(Triangle {
                vert_ids: [ids[1], ids[2], ids[3]],
                positions: [coords[1], coords[2], coords[3]],
                colors: [colors[1], colors[2], colors[3]],
                tex,
                cull: draw.cull(),
                flip: draw.flip_normal(),
            });
        }
        Ok(out)
    }

    pub fn num_verts(&self) -> usize {
        self.frame.verts.len()
    }
}

fn parse_anim(b: Bytes, at: usize, num_verts: usize) -> Result<Anim> {
    let name = b.name(at)?;
    let raw_frames = b.u16(at + 0x10)?;
    let frame_size = b.i16(at + 0x12)? as usize;
    let ptr_deltas = b.u32(at + 0x14)? as usize;
    let mut num_frames = (raw_frames & 0x7FFF) as usize;
    if raw_frames & 0x8000 != 0 {
        num_frames = num_frames / 2 + 1;
    }
    let deltas = if ptr_deltas != 0 {
        Some(
            (0..num_verts)
                .map(|i| b.u32(ptr_deltas + i * 4))
                .collect::<Result<Vec<_>>>()?,
        )
    } else {
        None
    };
    let frames = (0..num_frames)
        .map(|i| parse_frame(b, at + 0x18 + i * frame_size, num_verts, deltas.as_deref()))
        .collect::<Result<_>>()?;
    Ok(Anim { name, frames })
}

fn parse_frame(b: Bytes, at: usize, num_verts: usize, deltas: Option<&[u32]>) -> Result<Frame> {
    let pos = [b.i16(at)?, b.i16(at + 2)?, b.i16(at + 4)?];
    let off = b.u32(at + 0x18)? as usize;
    let data = &b.0[(at + off).min(b.0.len())..];
    let verts = match deltas {
        None => (0..num_verts)
            .map(|i| {
                Ok([
                    b.u8(at + off + i * 3)?,
                    b.u8(at + off + i * 3 + 1)?,
                    b.u8(at + off + i * 3 + 2)?,
                ])
            })
            .collect::<Result<_>>()?,
        Some(d) => decompress_frame(data, d)?,
    };
    Ok(Frame { pos, verts })
}

/// MSB-first reader over little-endian 32-bit words (CTR-native `RenderBucket_GetSignedBits`).
struct BitReader<'a> {
    data: &'a [u8],
    bit: usize,
}

impl BitReader<'_> {
    fn word(&self, i: usize) -> u32 {
        let mut w = [0u8; 4];
        for (k, x) in w.iter_mut().enumerate() {
            *x = self.data.get(i * 4 + k).copied().unwrap_or(0);
        }
        u32::from_le_bytes(w)
    }
    /// Reads `bits` (1..=32) bits as a sign-extended value.
    fn signed(&mut self, bits: u32) -> i32 {
        let mut v: u64 = 0;
        for _ in 0..bits {
            let w = self.word(self.bit / 32);
            let b = (w >> (31 - (self.bit % 32))) & 1;
            v = (v << 1) | b as u64;
            self.bit += 1;
        }
        let shift = 64 - bits;
        ((v << shift) as i64 >> shift) as i32
    }
}

fn sign_extend(v: u32, bits: u32) -> i32 {
    let shift = 32 - bits;
    ((v << shift) as i32) >> shift
}

/// Decodes one delta-compressed frame (Crash 2 style temporal deltas). Per vertex, the shared
/// delta word holds `x_base:7 (<<1) | b_base:8 | c_base:8 | x_bits:3 | b_bits:3 | c_bits:3`
/// (high to low); each component reads `bits + 1` signed bits from the frame's bitstream and
/// accumulates (`bits == 7` resets the accumulator to the read value). Components arrive as
/// x, raw-z, raw-y.
pub fn decompress_frame(stream: &[u8], deltas: &[u32]) -> Result<Vec<[u8; 3]>> {
    let mut r = BitReader {
        data: stream,
        bit: 0,
    };
    let (mut ax, mut ab, mut ac) = (0i32, 0i32, 0i32);
    let mut out = Vec::with_capacity(deltas.len());
    let step = |r: &mut BitReader, bits: u32, base: i32, acc: &mut i32| {
        let v = r.signed(bits + 1);
        *acc = if bits == 7 {
            v as i8 as i32
        } else {
            (*acc + v + base) as i8 as i32
        };
    };
    for &t in deltas {
        let x_bits = (t >> 6) & 7;
        let b_bits = (t >> 3) & 7;
        let c_bits = t & 7;
        let bx = sign_extend(t >> 25, 7) << 1;
        let bb = sign_extend((t >> 17) & 0xFF, 8);
        let bc = sign_extend((t >> 9) & 0xFF, 8);
        step(&mut r, x_bits, bx, &mut ax);
        step(&mut r, b_bits, bb, &mut ab);
        step(&mut r, c_bits, bc, &mut ac);
        // Stack: x = ax, y(raw index 1) = ac, z(raw index 2) = ab.
        out.push([ax as u8, ac as u8, ab as u8]);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_fields() {
        let c = Command(0x9C05_0A07);
        assert!(c.new_strip());
        assert!(c.cull());
        assert!(c.from_stack());
        assert!(!c.swap_first());
        assert_eq!(c.stack_index(), 5);
        assert_eq!(c.color_index(), 5);
        assert_eq!(c.tex_index(), 7);
    }

    #[test]
    fn tex_layout_fields() {
        // u0 v0 clut | u1 v1 tpage | u2 v2 u3 v3
        let raw = [1, 2, 0x45, 0x7E, 3, 4, 0x19, 0x00, 5, 6, 7, 8];
        let t = TexLayout::parse(Bytes(&raw), 0).unwrap();
        assert_eq!(t.uv, [[1, 2], [3, 4], [5, 6], [7, 8]]);
        assert_eq!(t.clut_x(), 5 * 16);
        assert_eq!(t.clut_y(), 0x7E45 >> 6);
        assert_eq!(t.page_x(), 9 * 64);
        assert_eq!(t.page_y(), 256);
        assert_eq!(t.bpp(), 0);
        assert_eq!(t.blend_mode(), 0);
        assert!(t.semi_transparent());
    }

    #[test]
    fn bit_reader_is_msb_first_over_le_words() {
        // Word 0x8000_0001 stored little endian: first bit read is 1, last is 1.
        let data = 0x8000_0001u32.to_le_bytes();
        let mut r = BitReader {
            data: &data,
            bit: 0,
        };
        assert_eq!(r.signed(2), -2); // bits "10"
        assert_eq!(r.signed(29), 0);
        assert_eq!(r.signed(1), -1);
    }

    #[test]
    fn delta_decode_accumulates_and_resets() {
        // Vertex 0: all bits 7 (8-bit reads, reset), stream 0x10 0x20 0x30.
        // Vertex 1: all bits 0 (1-bit reads, value 0), bases x=+4 (2<<1), b=+1, c=-1.
        let t0 = (7 << 6) | (7 << 3) | 7;
        let t1 = (2u32 << 25) | (1 << 17) | (0xFF << 9);
        let stream = u32::from_be_bytes([0x10, 0x20, 0x30, 0x00]).to_le_bytes();
        let v = decompress_frame(&stream, &[t0, t1]).unwrap();
        assert_eq!(v[0], [0x10, 0x30, 0x20]);
        assert_eq!(v[1], [0x14, 0x2F, 0x21]);
    }

    #[test]
    fn strips_assemble_like_the_game() {
        // Four new vertices in one strip -> two triangles; second command keyed by itself.
        let mut mesh = ModelMesh {
            name: "t".into(),
            flags: 0,
            scale: [4096; 3],
            commands: vec![
                Command(0x8000_0000),
                Command(0x0001_0000),
                Command(0x0002_0001),
                Command(0x0003_0000),
            ],
            colors: vec![[1, 2, 3, 0]],
            tex_layouts: vec![TexLayout {
                uv: [[0; 2]; 4],
                clut: 0,
                tpage: 0,
            }],
            frame: Frame {
                pos: [0; 3],
                verts: vec![[0, 0, 0], [1, 0, 0], [0, 1, 0], [1, 1, 0]],
            },
            anims: vec![],
        };
        let f = mesh.frame.clone();
        let tris = mesh.triangles(&f).unwrap();
        assert_eq!(tris.len(), 2);
        assert_eq!(tris[0].vert_ids, [0, 1, 2]);
        assert_eq!(tris[0].tex, None); // first triangle uses the strip's first command
        assert_eq!(tris[1].vert_ids, [1, 2, 3]);
        // Swap flag on a later command reuses the older vertex.
        mesh.commands.push(Command(0x4004_0000));
        mesh.frame.verts.push([2, 2, 0]);
        let f = mesh.frame.clone();
        let tris = mesh.triangles(&f).unwrap();
        assert_eq!(tris[2].vert_ids, [1, 3, 4]);
        // Raw [x, z, y] bytes map to [x, y, z].
        assert_eq!(mesh.position(&f, [1, 2, 3]), [1.0, 3.0, 2.0]);
    }
}
