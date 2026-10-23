//! [`World`] for JavaScript in a Web Worker: built from the player's picked `.MPQ` `File`s, each
//! read by byte range through `FileReaderSync` (synchronous, so worker-only), and answering with
//! plain objects of typed arrays.

use std::io;
use std::sync::Arc;

use benilla_formats::Chain;
use benilla_mpq::Source;
use js_sys::{Array, Float32Array, Object, Reflect, Uint32Array, Uint8Array};
use wasm_bindgen::prelude::*;
use web_sys::{File, FileReaderSync};

use crate::World;

/// A picked file, read by range. The browser's JS objects are single-threaded and this module
/// runs in one worker thread, so the `Send + Sync` the chain asks of a source holds trivially.
struct BlobSource {
    file: File,
    reader: FileReaderSync,
    len: u64,
}

// SAFETY: wasm32-unknown-unknown here is built without threads; nothing is shared across threads.
unsafe impl Send for BlobSource {}
// SAFETY: as above.
unsafe impl Sync for BlobSource {}

impl Source for BlobSource {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, pos: u64, buf: &mut [u8]) -> io::Result<()> {
        let end = pos + buf.len() as u64;
        if end > self.len {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        let blob = self
            .file
            .slice_with_f64_and_f64(pos as f64, end as f64)
            .map_err(|e| io::Error::other(format!("{e:?}")))?;
        let bytes = self
            .reader
            .read_as_array_buffer(&blob)
            .map_err(|e| io::Error::other(format!("{e:?}")))?;
        let view = Uint8Array::new(&bytes);
        if view.length() as usize != buf.len() {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        view.copy_to(buf);
        Ok(())
    }
}

fn err(e: anyhow::Error) -> JsError {
    JsError::new(&format!("{e:#}"))
}

fn set(obj: &Object, key: &str, value: &JsValue) {
    let _ = Reflect::set(obj, &JsValue::from_str(key), value);
}

/// The world over the player's MPQs.
#[wasm_bindgen]
pub struct WebWorld(World);

#[wasm_bindgen]
impl WebWorld {
    /// From the picked `File`s: those named as `Data` MPQs mount in the game's order, the rest
    /// are ignored.
    #[wasm_bindgen(constructor)]
    pub fn new(files: Array) -> Result<WebWorld, JsError> {
        let mut sources: Vec<(String, Arc<dyn Source>)> = Vec::new();
        for f in files.iter() {
            let Ok(file) = f.dyn_into::<File>() else {
                continue;
            };
            let name = file.name();
            if !name.to_ascii_lowercase().ends_with(".mpq") {
                continue;
            }
            let reader = FileReaderSync::new().map_err(|e| JsError::new(&format!("{e:?}")))?;
            let len = file.size() as u64;
            sources.push((name, Arc::new(BlobSource { file, reader, len })));
        }
        let chain = Chain::from_sources(sources).map_err(err)?;
        Ok(WebWorld(World::new(chain)))
    }

    /// A map's folder by `Map.dbc` id, or `undefined`.
    #[wasm_bindgen(js_name = mapDirectory)]
    pub fn map_directory(&mut self, map_id: u32) -> Result<Option<String>, JsError> {
        self.0.map_directory(map_id).map_err(err)
    }

    /// The map's tiles under a world rectangle, as `[x0, y0, x1, y1, ...]`.
    #[wasm_bindgen(js_name = tilesIn)]
    pub fn tiles_in(
        &mut self,
        map: &str,
        min_x: f32,
        min_y: f32,
        max_x: f32,
        max_y: f32,
    ) -> Result<Vec<u32>, JsError> {
        let tiles = self
            .0
            .tiles_in(map, [min_x, min_y], [max_x, max_y])
            .map_err(err)?;
        Ok(tiles.into_iter().flat_map(|(x, y)| [x, y]).collect())
    }

    /// Tile `(x, y)`: `{ chunks: [{ positions, normals, uvs, indices, layers, alpha, shadow }],
    /// water: [{ positions, indices, ocean, magma }] }`, typed arrays throughout.
    pub fn tile(&mut self, map: &str, x: u32, y: u32) -> Result<JsValue, JsError> {
        let tile = self.0.tile(map, x, y).map_err(err)?;
        let chunks = Array::new();
        for c in &tile.chunks {
            let o = Object::new();
            set(&o, "positions", &Float32Array::from(c.positions.as_slice()));
            set(&o, "normals", &Float32Array::from(c.normals.as_slice()));
            set(&o, "uvs", &Float32Array::from(c.uvs.as_slice()));
            set(&o, "indices", &Uint32Array::from(c.indices.as_slice()));
            let layers = Array::new();
            for l in &c.layers {
                layers.push(&JsValue::from_str(l));
            }
            set(&o, "layers", &layers);
            set(
                &o,
                "alpha",
                &c.alpha
                    .as_deref()
                    .map_or(JsValue::NULL, |a| Uint8Array::from(a).into()),
            );
            set(
                &o,
                "shadow",
                &c.shadow
                    .as_deref()
                    .map_or(JsValue::NULL, |a| Uint8Array::from(a).into()),
            );
            chunks.push(&o);
        }
        let water = Array::new();
        for w in &tile.water {
            let o = Object::new();
            set(&o, "positions", &Float32Array::from(w.positions.as_slice()));
            set(&o, "indices", &Uint32Array::from(w.indices.as_slice()));
            set(&o, "ocean", &JsValue::from_bool(w.ocean));
            set(&o, "magma", &JsValue::from_bool(w.magma));
            water.push(&o);
        }
        let placed = |list: &[crate::Placement]| {
            let a = Array::new();
            for p in list {
                let o = Object::new();
                set(&o, "model", &JsValue::from_str(&p.model));
                set(&o, "id", &JsValue::from(p.unique_id));
                set(&o, "matrix", &Float32Array::from(p.matrix.as_slice()));
                a.push(&o);
            }
            a
        };
        let out = Object::new();
        set(&out, "chunks", &chunks);
        set(&out, "water", &water);
        set(&out, "doodads", &placed(&tile.doodads));
        set(&out, "wmos", &placed(&tile.wmos));
        Ok(out.into())
    }

    /// A model's submeshes: `[{ positions, normals, uvs, indices, texture, blend, twoSided }]`,
    /// an M2 or (`wmo`) a WMO with its groups.
    pub fn model(&mut self, path: &str, wmo: bool) -> Result<JsValue, JsError> {
        let subs = self.0.model(path, wmo).map_err(err)?;
        let out = Array::new();
        for s in &subs {
            let o = Object::new();
            set(&o, "positions", &Float32Array::from(s.positions.as_slice()));
            set(&o, "normals", &Float32Array::from(s.normals.as_slice()));
            set(&o, "uvs", &Float32Array::from(s.uvs.as_slice()));
            set(&o, "indices", &Uint32Array::from(s.indices.as_slice()));
            set(
                &o,
                "texture",
                &s.texture
                    .as_deref()
                    .map_or(JsValue::NULL, JsValue::from_str),
            );
            set(&o, "blend", &JsValue::from(s.blend));
            set(&o, "twoSided", &JsValue::from_bool(s.two_sided));
            out.push(&o);
        }
        Ok(out.into())
    }

    /// A texture: `{ width, height, rgba }` at its largest level no wider than `max_width`.
    pub fn texture(&mut self, path: &str, max_width: u32) -> Result<JsValue, JsError> {
        let t = self.0.texture(path, max_width).map_err(err)?;
        let out = Object::new();
        set(&out, "width", &JsValue::from(t.width));
        set(&out, "height", &JsValue::from(t.height));
        set(&out, "rgba", &Uint8Array::from(t.rgba.as_slice()));
        Ok(out.into())
    }
}
