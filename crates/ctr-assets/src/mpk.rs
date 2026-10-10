//! Model packs (`.mpk`): `{size, data, pointer map}` like a `.ctr`; `data` starts with
//! `u32 ptr_icon_pack` followed by model pointers. The icon pack is
//! `u32 num_icons, u32 ptr_icons, u32 num_groups, u32 ptr_groups`, then `num_icons` icons
//! (`char name[16]; i32 index; TextureLayout layout` = 0x20 bytes) and, at `ptr_groups`,
//! `num_groups` pointers to `IconGroup { char name[16]; s16 group_id; s16 num_icons;
//! u32 icon_ptrs[num_icons] }`. The game indexes `gGT->iconGroup[group_id]`; group 0 is the
//! tire sprite animation used by `DrawTires`.

use crate::ctr::{unwrap_patched, Bytes, TexLayout};
use crate::Result;

#[derive(Clone, Debug)]
pub struct Icon {
    pub name: String,
    pub layout: TexLayout,
}

#[derive(Clone, Debug)]
pub struct IconGroup {
    pub name: String,
    pub id: i16,
    pub icons: Vec<Icon>,
}

pub fn icon_groups(file: &[u8]) -> Result<Vec<IconGroup>> {
    let data = unwrap_patched(file)?;
    let b = Bytes(data);
    let pack = b.u32(0)? as usize;
    let num_groups = b.u32(pack + 8)? as usize;
    let ptr_groups = b.u32(pack + 12)? as usize;
    let icon = |at: usize| -> Result<Icon> {
        Ok(Icon {
            name: b.name(at)?,
            layout: TexLayout::parse(b, at + 0x14)?,
        })
    };
    (0..num_groups)
        .map(|g| {
            let at = b.u32(ptr_groups + g * 4)? as usize;
            let n = b.i16(at + 0x12)?.max(0) as usize;
            let icons = (0..n)
                .map(|i| icon(b.u32(at + 0x14 + i * 4)? as usize))
                .collect::<Result<Vec<_>>>()?;
            Ok(IconGroup {
                name: b.name(at)?,
                id: b.i16(at + 0x10)?,
                icons,
            })
        })
        .collect()
}

/// One icon of the pack's flat icon list; `index` is the game's global icon number
/// (`gGT->ptrIcons[index]`).
#[derive(Clone, Debug)]
pub struct IndexedIcon {
    pub index: i32,
    pub icon: Icon,
}

/// All icons of the pack (the list at `ptr_icons`, `num_icons` entries of 0x20 bytes).
pub fn icons(file: &[u8]) -> Result<Vec<IndexedIcon>> {
    let data = unwrap_patched(file)?;
    let b = Bytes(data);
    let pack = b.u32(0)? as usize;
    let num = b.u32(pack)? as usize;
    let ptr = b.u32(pack + 4)? as usize;
    (0..num)
        .map(|i| {
            let at = ptr + i * 0x20;
            Ok(IndexedIcon {
                index: b.u32(at + 0x10)? as i32,
                icon: Icon {
                    name: b.name(at)?,
                    layout: TexLayout::parse(b, at + 0x14)?,
                },
            })
        })
        .collect()
}

/// Offsets (into the unwrapped pack data) of the pack's models, with their names and
/// `gGT->modelPtr` ids. The pointer list follows `ptr_icon_pack` and ends where the first
/// model's data begins.
pub fn models(file: &[u8]) -> Result<Vec<(usize, String, i16)>> {
    let data = unwrap_patched(file)?;
    let b = Bytes(data);
    let first = b.u32(4)? as usize;
    if first < 8 {
        return Ok(Vec::new());
    }
    let n = (first - 4) / 4 - 1;
    let mut out = Vec::new();
    for i in 0..n {
        let p = b.u32(4 + i * 4)? as usize;
        if p != 0 {
            out.push((p, b.name(p)?, b.i16(p + 0x10)?));
        }
    }
    Ok(out)
}
