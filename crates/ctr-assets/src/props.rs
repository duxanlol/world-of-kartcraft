//! Every model the disc's level files and `shared.mpk` carry, each at its static frame and with
//! its textures, for the track editor's prop catalog (`prop-catalog`). The bigfile names none of
//! its entries, so a level is any entry that parses as one (`level_models`, names all printable),
//! its textures in the entry before it (a track's `data.vrm` precedes its `data.lev`, as Crash
//! Cove's 112 and 113 do) over `shared.vrm`. A model met again in a later level is skipped.
//!
//! Units follow `effects.json`: meters, +Y up, at instance scale 0x1000.

use std::collections::HashSet;

use crate::big::{self, BigFile};
use crate::ctr::{unwrap_patched, Model};
use crate::effects::{mesh_frame_at, MeshFrame, Textures};
use crate::kart::Texture;
use crate::mpk;
use crate::vram::Vram;
use crate::weapons::level_models;
use crate::Result;

/// A model as found: the bigfile entry of the level it came from (`None` for `shared.mpk`), its
/// name and id, and its first mesh's static frame.
pub struct Prop {
    pub level: Option<usize>,
    pub name: String,
    pub model_id: i16,
    pub frame: MeshFrame,
}

/// The props, and the textures their surfaces index.
pub struct Props {
    pub textures: Vec<Texture>,
    pub props: Vec<Prop>,
}

/// Whether `models` are a level's: some, each named in printable ASCII.
fn plausible(models: &[(usize, String, i16)]) -> bool {
    !models.is_empty()
        && models
            .iter()
            .all(|(_, n, _)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_graphic()))
}

pub fn build(bf: &mut BigFile) -> Result<Props> {
    let shared_vrm = bf.read(big::index::SHARED_VRM)?;
    let mut tex = Textures::default();
    let mut props = Vec::new();
    let mut seen = HashSet::new();
    let mut take =
        |level: Option<usize>, data: &[u8], models: &[(usize, String, i16)], vram: &Vram| {
            for (at, name, id) in models {
                if !seen.insert(name.clone()) {
                    continue;
                }
                let Ok(model) = Model::parse_at(data, *at) else {
                    continue;
                };
                let Some(mesh) = model.meshes.first() else {
                    continue;
                };
                let Ok(frame) = mesh_frame_at(name, mesh, &mesh.frame, vram, &mut tex) else {
                    continue;
                };
                if frame.surfaces.iter().any(|s| !s.indices.is_empty()) {
                    props.push(Prop {
                        level,
                        name: name.clone(),
                        model_id: *id,
                        frame,
                    });
                }
            }
        };
    let mut vram = Vram::new();
    vram.load_vrm(&shared_vrm)?;
    let mpk_file = bf.read(big::index::SHARED_MPK)?;
    take(
        None,
        unwrap_patched(&mpk_file)?,
        &mpk::models(&mpk_file)?,
        &vram,
    );
    for i in 1..bf.entries.len() {
        let Ok(file) = bf.read(i) else { continue };
        let Some(data) = unwrap_patched(&file).ok() else {
            continue;
        };
        let Some(models) = level_models(data).ok().filter(|m| plausible(m)) else {
            continue;
        };
        let mut vram = Vram::new();
        vram.load_vrm(&shared_vrm)?;
        if let Ok(vrm) = bf.read(i - 1) {
            // Not every level's textures precede it; those models keep the shared art.
            let _ = vram.load_vrm(&vrm);
        }
        take(Some(i), data, &models, &vram);
    }
    Ok(Props {
        textures: tex.list,
        props,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_level_has_printable_named_models() {
        let named = |n: &str| vec![(0, n.to_string(), 0)];
        assert!(plausible(&named("crate_fruit")));
        assert!(!plausible(&named("")));
        assert!(!plausible(&named("cr\u{1}te")));
        assert!(!plausible(&[]));
    }
}
