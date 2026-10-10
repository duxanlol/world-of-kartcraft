//! `effects.json`: CTR's kart visual effects (turbo flames, exhaust smoke, skid marks, dirt and
//! landing particles), with textures from the player's disc and the game's spawn/animation
//! parameters converted to seconds and meters.
//!
//! Textures and the flame models come from the disc (`shared.mpk` / `shared.vrm`). The
//! numeric particle parameters are code constants from the game executable, transcribed from
//! the CTR-native decompilation (`zGlobal_DATA.c` emitter sets, `Vehicle/*.c`); each block
//! cites its source.
//!
//! Unit conventions (all derived from the kart export, `kart.rs`):
//! - kart.json meters = CTR world units / 51.2 (= model units / 64; racer instances are drawn
//!   at 0.8 scale, so 1 world unit = 1/0.8 model units).
//! - particle positions/velocities are FP8 world units (`>> 8`), per game frame; the game
//!   ticks at 30 frames per second.

use serde::Serialize;
use serde_json::{json, Value};

use crate::big::{self, BigFile};
use crate::ctr::{Model, ModelMesh, TexLayout};
use crate::kart::Texture;
use crate::mpk;
use crate::vram::Vram;
use crate::{Error, Result};

/// CTR world units per kart.json meter.
pub const WORLD_PER_METER: f32 = 51.2;
/// Game frames per second.
pub const FPS: f32 = 30.0;

fn m(world: f32) -> f32 {
    world / WORLD_PER_METER
}

/// FP8 world units (particle axes) to meters.
fn fp8_m(v: f32) -> f32 {
    v / 256.0 / WORLD_PER_METER
}

/// Exhaust smoke spawn points, kart-local (`VehEmitter.c`: `scale * 9 >> 3`, `scale * 7 >> 1`,
/// `scale * -0x38 >> 4` and `scale * -0x12 >> 4` in FP8 with the racer scale 0xCCC).
pub fn exhaust_anchors() -> [[f32; 3]; 2] {
    let s = 0xCCC as f32;
    let y = fp8_m(s * 7.0 / 2.0);
    let z = fp8_m(s * -56.0 / 16.0);
    [
        [fp8_m(s * 9.0 / 8.0), y, z],
        [fp8_m(s * -18.0 / 16.0), y, z],
    ]
}

/// Turbo flame instance origins, kart-local (`VehTurbo.c`: `scale * 9 >> 11`,
/// `scale * 3 >> 8`, `scale * -0x34 >> 12`; right: `scale * -0x12 >> 12`, world units).
pub fn flame_origins() -> [[f32; 3]; 2] {
    let s = 0xCCC as f32;
    let y = m(s * 3.0 / 256.0);
    let z = m(s * -52.0 / 4096.0);
    [[m(s * 9.0 / 2048.0), y, z], [m(s * -18.0 / 4096.0), y, z]]
}

#[derive(Serialize)]
pub struct Vertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
    pub color: [f32; 4],
}

#[derive(Serialize)]
pub struct Surface {
    pub texture: Option<usize>,
    /// `"opaque"`, `"mix"` (0.5 dst + 0.5 src), `"additive"` (dst + src),
    /// `"subtractive"` (dst − src) or `"add_quarter"` (dst + 0.25 src): PSX blend modes.
    pub blend: &'static str,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

#[derive(Serialize)]
pub struct MeshFrame {
    pub name: String,
    pub surfaces: Vec<Surface>,
}

pub fn blend_name(abr: u8) -> &'static str {
    ["mix", "additive", "subtractive", "add_quarter"][abr as usize & 3]
}

/// Texture atlas shared by all effects; deduplicates page/CLUT/rect crops.
#[derive(Default)]
struct Textures {
    list: Vec<Texture>,
    keys: Vec<(u16, u16, u32, u32, u32, u32)>,
}

impl Textures {
    fn add(&mut self, vram: &Vram, name: &str, l: &TexLayout, rect: [u32; 4]) -> usize {
        let key = (l.tpage & !0x60, l.clut, rect[0], rect[1], rect[2], rect[3]);
        if let Some(i) = self.keys.iter().position(|k| *k == key) {
            return i;
        }
        let [u0, v0, w, h] = rect;
        self.keys.push(key);
        // Decode as semi-transparent: STP texels get alpha 128 (they blend), others 255.
        let semi = TexLayout {
            tpage: l.tpage & !0x60,
            ..*l
        };
        self.list.push(Texture {
            name: name.to_string(),
            width: w,
            height: h,
            rgba: vram.decode_rgba(&semi, u0, v0, w, h),
        });
        self.list.len() - 1
    }

    fn icon(&mut self, vram: &Vram, icon: &mpk::Icon) -> usize {
        let l = icon.layout;
        let (u0, v0) = (l.uv[0][0] as u32, l.uv[0][1] as u32);
        let (u1, v1) = (l.uv[3][0] as u32, l.uv[3][1] as u32);
        self.add(
            vram,
            &icon.name,
            &l,
            [u0, v0, u1.saturating_sub(u0) + 1, v1.saturating_sub(v0) + 1],
        )
    }
}

/// Converts a static effect model to surfaces in meters (model units are world units here).
fn mesh_frame(name: &str, mesh: &ModelMesh, vram: &Vram, tex: &mut Textures) -> Result<MeshFrame> {
    let tris = mesh.triangles(&mesh.frame)?;
    // UV bounds per page+CLUT.
    let mut rects: Vec<((u16, u16), [u32; 4])> = Vec::new();
    for t in &tris {
        let Some(li) = t.tex else { continue };
        let l = mesh.tex_layouts[li];
        let key = (l.tpage & !0x60, l.clut);
        let i = match rects.iter().position(|r| r.0 == key) {
            Some(i) => i,
            None => {
                rects.push((key, [u32::MAX, u32::MAX, 0, 0]));
                rects.len() - 1
            }
        };
        let r = &mut rects[i].1;
        for uv in &l.uv[..3] {
            r[0] = r[0].min(uv[0] as u32);
            r[1] = r[1].min(uv[1] as u32);
            r[2] = r[2].max(uv[0] as u32);
            r[3] = r[3].max(uv[1] as u32);
        }
    }
    let mut surfaces: Vec<Surface> = Vec::new();
    for t in &tris {
        let (texture, blend, uvs) = match t.tex {
            Some(li) => {
                let l = mesh.tex_layouts[li];
                let key = (l.tpage & !0x60, l.clut);
                let r = rects.iter().find(|r| r.0 == key).unwrap().1;
                let (w, h) = (r[2] - r[0] + 1, r[3] - r[1] + 1);
                let ti = tex.add(vram, &format!("{name}_tex"), &l, [r[0], r[1], w, h]);
                let blend = if l.semi_transparent() {
                    blend_name(l.blend_mode())
                } else {
                    "opaque"
                };
                let uvs = [0, 1, 2].map(|i| {
                    [
                        (l.uv[i][0] as f32 - r[0] as f32 + 0.5) / w as f32,
                        (l.uv[i][1] as f32 - r[1] as f32 + 0.5) / h as f32,
                    ]
                });
                (Some(ti), blend, uvs)
            }
            None => (None, "opaque", [[0.0; 2]; 3]),
        };
        let si = match surfaces
            .iter()
            .position(|s| s.texture == texture && s.blend == blend)
        {
            Some(i) => i,
            None => {
                surfaces.push(Surface {
                    texture,
                    blend,
                    vertices: vec![],
                    indices: vec![],
                });
                surfaces.len() - 1
            }
        };
        let s = &mut surfaces[si];
        let base = s.vertices.len() as u32;
        let div = if texture.is_some() { 128.0 } else { 255.0 };
        for ((p, c), uv) in t.positions.iter().zip(&t.colors).zip(uvs) {
            s.vertices.push(Vertex {
                position: [m(p[0]), m(p[1]), m(p[2])],
                uv,
                color: [c[0] as f32 / div, c[1] as f32 / div, c[2] as f32 / div, 1.0],
            });
        }
        s.indices.extend([base, base + 1, base + 2]);
    }
    Ok(MeshFrame {
        name: name.to_string(),
        surfaces,
    })
}

#[derive(Serialize)]
pub struct EffectsFile {
    pub format: &'static str,
    pub version: u32,
    pub units: Value,
    pub textures: Vec<Texture>,
    pub turbo_flame: Value,
    pub exhaust_smoke: Value,
    pub failed_boost_puff: Value,
    pub skid_marks: Value,
    pub dirt_dust: Value,
    pub landing_sparks: Value,
}

/// Per-frame FP8 particle value → per-second meters.
fn v_ms(v: f32) -> f32 {
    fp8_m(v) * FPS
}

/// Per-frame² FP8 → m/s².
fn a_ms2(a: f32) -> f32 {
    fp8_m(a) * FPS * FPS
}

pub fn build(bf: &mut BigFile) -> Result<EffectsFile> {
    let mut vram = Vram::new();
    vram.load_vrm(&bf.read(big::index::SHARED_VRM)?)?;
    let mpk_file = bf.read(big::index::SHARED_MPK)?;
    let data = crate::ctr::unwrap_patched(&mpk_file)?;
    let mut tex = Textures::default();

    // --- Turbo flame: models turbo..turbo7 (ids 0x2C..0x33, STATIC_TURBO_EFFECT..7).
    let models = mpk::models(&mpk_file)?;
    let mut frames = Vec::new();
    for id in 0x2C..=0x33i16 {
        let (at, name, _) = models
            .iter()
            .find(|m| m.2 == id)
            .ok_or_else(|| Error(format!("shared.mpk has no model id {id:#x}")))?;
        let model = Model::parse_at(data, *at)?;
        let mesh = model
            .meshes
            .first()
            .ok_or_else(|| Error(format!("{name}: no mesh")))?;
        frames.push(mesh_frame(name, mesh, &vram, &mut tex)?);
    }
    let [fl, fr] = flame_origins();
    let turbo_flame = json!({
        "source": "shared.mpk models turbo..turbo7 (modelPtr ids 0x2C..0x33, STATIC_TURBO_EFFECT); VehTurbo.c VehTurbo_ThTick, VehFire.c VehFire_Increment",
        "frames": frames,
        "frame_rate_fps": FPS,
        "animation": "frame index i advances by 1 every game frame and wraps at 8; the left flame shows frame i, the right flame frame (i + 3) & 7",
        "instances": [
            { "name": "left", "origin": fl, "mirror_x": false },
            { "name": "right", "origin": fr, "mirror_x": true }
        ],
        "placement": "mesh positions are in meters at fire size 8, in the kart's axes (+Y up, +Z forward, +X left); place each frame at its instance origin, rotate with the kart, scale uniformly by fire_size / 8, and negate X for the right flame",
        "fire_size": {
            "formula": "fire_size = clamp(5 + (fire_level >> 6), 4, 8), recomputed when a boost raises the speed cap",
            "power_slide": "fire_level = boosts_so_far << 6: 1st boost 5, 2nd 6, 3rd 7",
            "start_boost_and_items": "rev start / turbo item / turbo pad fire levels >= 0xC0 give 8; sacred fire (fire_level 0x100) is also 8, there is no separate blue flame model"
        },
        "colors": "vertex color multiplies the texture; textured triangles are semi-transparent with their tpage blend mode (the flame core is additive, an outer shell subtractive, darkening what is behind it)",
        "fade": {
            "alpha_scale": "0 = fully visible .. 4096 = gone; drawn with color *= 1 - alpha_scale / 4096 (DEPTH_FADE)",
            "visible": "hidden for 96 ms (fireVisibilityCooldown 0x60 ms) after a non-pad boost, then shown",
            "fade_out_slow_per_second": 0x40 as f32 * FPS / 4096.0,
            "fade_out_fast_per_second": 0x100 as f32 * FPS / 4096.0,
            "rules": "while reserves >= 16 and the disappear countdown has not expired the flame stays at alpha 0; power-slide flames get a 2-frame disappear countdown, then fade fast (0x100/frame); other flames fade slowly (0x40/frame) once reserves < 16; at 4096 the turbo thread dies"
        },
        "spawn": "one turbo thread per driver, created or refreshed by VehFire_Increment (every boost: power slide, start boost, turbo pad, turbo item, hang time); lives while reserves > 0 (VehTurbo.c)"
    });

    // --- Exhaust smoke: icon group 1 (dustpuff), emSet_Exhaust_High.
    let groups = mpk::icon_groups(&mpk_file)?;
    let dust = groups
        .iter()
        .find(|g| g.id == 1)
        .ok_or_else(|| Error("shared.mpk has no dustpuff icon group (id 1)".into()))?;
    let dust_tex: Vec<usize> = dust.icons.iter().map(|i| tex.icon(&vram, i)).collect();
    let icon_w =
        (dust.icons[0].layout.uv[1][0] as f32 - dust.icons[0].layout.uv[0][0] as f32) + 1.0;
    // Billboard half-size in world units = icon width * scale / 4096 (Particle_RenderList).
    let half = |scale: f32| m(icon_w * scale / 4096.0);
    let [el, er] = exhaust_anchors();
    let vel_up = fp8_m(0x400 as f32);
    let exhaust_smoke = json!({
        "source": "VehEmitter.c VehEmitter_Exhaust + exhaust block in VehEmitter_DriverMain; zGlobal_DATA.c emSet_Exhaust_High; Particle.c",
        "texture": dust_tex[0],
        "texture_frames_available": dust_tex,
        "note_frames": "the exhaust uses only dustpuff00 (no icon-frame axis); the other dustpuff frames are exported for convenience",
        "billboard": true,
        "blend": "additive",
        "blend_boost_ready": "subtractive",
        "boost_ready_rule": "while power-sliding with the drift meter in its boost window (129 <= turbo_MeterRoomLeft <= (const_turboLowRoomWarning + 2) * 32) or while revving the start boost at the right moment (revEngineState == 1), the puffs draw subtractive: the exhaust turns dark, CTR's 'boost now' cue",
        "color": { "start_gray": 64.0 / 255.0, "per_second": -10.0 / 255.0 * FPS,
                   "note": "gray (R=G=B) 0x40 falling 0xA/frame; additive brightens by it, subtractive darkens by it" },
        "lifetime_seconds": 6.0 / FPS,
        "lifetime_frames": 6,
        "size": { "half_extent_m_start": half(1536.0), "half_extent_m_per_second": half(400.0) * FPS },
        "rotation": { "start_turns_random": [-0.125, 0.0], "turns_per_second_random": [-256.0 / 4096.0 * FPS, 256.0 / 4096.0 * FPS],
                      "note": "screen-plane rotation (axis 4), 4096 units per turn" },
        "spawn_points": [el, er],
        "velocity": { "kart_local_m_per_s": [0.0, v_ms(0x400 as f32), v_ms(-0x400 as f32)],
                      "gravity_m_per_s2": [0.0, a_ms2(-256.0), 0.0],
                      "frame": "velocity is the kart-local (0, +4, -4) world units/frame rotated into world space at spawn (up and backwards out of the pipes); positions then follow the kart's translation (DRIVER_LOCAL particles add the driver's position each frame) but not its rotation" },
        "per_frame_velocity_m": vel_up,
        "spawn": {
            "rate": "one puff per pipe per frame (60 per second in total) in 1-player; 2P every other frame, 3P/4P every 4th",
            "when": "always while the kart is drawn and visible, except: no puffs while the turbo flame is shown (a turbo thread exists) unless the meter is in its boost window or failedBoostExhaustTimer > 0; none while revEngineState == 2"
        }
    });

    let failed_boost_puff = json!({
        "source": "VehPhysProc.c VehPhysProc_PowerSlide_Update (failedBoostExhaustTimer = 8 when L1/R1 is pressed before the meter's window), VehEmitter.c exhaust block",
        "effect": "not a separate particle: for 8 frames (0.27 s) the normal exhaust puffs (exhaust_smoke) are emitted even if a turbo flame is burning; the meter is reset to 0, so they draw additive (light gray)",
        "frames": 8,
        "seconds": 8.0 / FPS,
        "uses": "exhaust_smoke"
    });

    // --- Skid marks: global icon 0x2F ("tread"), VehGroundSkids.c.
    let all_icons = mpk::icons(&mpk_file)?;
    let tread = all_icons
        .iter()
        .find(|i| i.index == 0x2F)
        .ok_or_else(|| Error("shared.mpk has no icon 0x2F (tread)".into()))?;
    let tread_tex = tex.icon(&vram, &tread.icon);
    let skid_marks = json!({
        "source": "VehEmitter.c VehEmitter_Skidmarks (laying), VehGroundSkids.c (drawing), icon ptrIcons[0x2F] 'tread'",
        "texture": tread_tex,
        "blend": "subtractive",
        "color": { "near": 127.0 / 128.0, "note": "texel * color is subtracted from the ground; full (0x7F) within ~0x180 depth units of the camera, halving with each doubling of distance beyond, dropped below 0x10" },
        "width_m": m(20.0),
        "tires": {
            "rear_left": [m(30.0), 0.0, m(-20.0)], "rear_right": [m(-30.0), 0.0, m(-20.0)],
            "front_left": [m(30.0), 0.0, m(40.0)], "front_right": [m(-30.0), 0.0, m(40.0)],
            "lateral_offset_note": "each mark is shifted by half of 15 world units along the kart's slide direction (axisRotationX)"
        },
        "laying": "every game frame each enabled tire writes one cross-section (two ground points `width_m` apart, perpendicular to the kart's heading) into an 8-entry ring buffer; consecutive cross-sections of the same tire are joined into textured quads (u across the mark, v along). Marks are in world space and stay put, but only the last 8 frames (0.27 s) exist: CTR skid marks are short trails that follow the kart",
        "ring_frames": 8,
        "spawn": {
            "rear_tires": "ACTION_BACK_SKID: set every frame while power-sliding (VehPhysProc_PowerSlide_PhysLinear), on hard cornering/braking (VehPhysForce.c), on terrains with TERRAIN_FLAG_FORCE_SKIDMARKS, and at wheelie start",
            "front_tires": "ACTION_FRONT_SKID: power-sliding and forced-skid terrain",
            "requires": "wheels on the ground (ACTION_TOUCH_GROUND)"
        }
    });

    // --- Dirt/grass spray during driving/drifting (line particles), emSet_DirtLR.
    let dirt_dust = json!({
        "source": "VehEmitter.c VehEmitter_Terrain_Ground; zGlobal_DATA.c emSet_DirtLR / emSet_GrassL/R (terrain em_OddFrame/em_EvenFrame)",
        "kind": "line",
        "note": "CTR draws these as short additive G2 lines (tail black, head colored) from the previous to the current position, not sprites; asphalt/track/stone/ice have no terrain emitter, so on roads there is no drift dust",
        "blend": "additive",
        "color_rgb_start": [1.0, 0.5, 0.0],
        "color_rgb_per_second": [-2048.0 / 65280.0 * FPS, -1024.0 / 65280.0 * FPS, 0.0],
        "lifetime_seconds": 10.0 / FPS,
        "spawn_offset_random_m": [fp8_m(-2560.0), fp8_m(2560.0)],
        "velocity": { "horizontal_random_m_per_s": [v_ms(-500.0), v_ms(500.0)],
                      "up_m_per_s": [v_ms(1500.0), v_ms(1900.0)],
                      "gravity_m_per_s2": a_ms2(-300.0),
                      "frame": "x/z are kart-local, rotated into world space at spawn" },
        "spawn_points": {
            "rear": [[m(-30.0), m(10.0), m(-20.0)], [m(30.0), m(10.0), m(-20.0)]],
            "front": [[m(-30.0), m(10.0), m(40.0)], [m(30.0), m(10.0), m(40.0)]]
        },
        "spawn": "1-player only; every frame on a terrain with an emitter (dirt, grass, ...), wheels on the ground, not holding brake-accel-prevention, |speed| >= 0x300 (about 5-7 % of top speed): one particle per rear tire, or per all four tires while drifting (kartState == KS_DRIFTING)"
    });

    let landing_sparks = json!({
        "source": "VehEmitter.c VehEmitter_Sparks_Ground (count 10); zGlobal_DATA.c emSet_GroundSparks",
        "kind": "line",
        "blend": "additive",
        "count": 10,
        "color_rgb_start": [1.0, 1.0, 0.5],
        "color_rgb_per_second": [-7680.0 / 65280.0 * FPS, -7680.0 / 65280.0 * FPS, -3840.0 / 65280.0 * FPS],
        "lifetime_seconds": 8.0 / FPS,
        "line_length_factor": 2.0,
        "velocity": { "horizontal_random_m_per_s": [v_ms(-500.0), v_ms(500.0)],
                      "up_m_per_s": [v_ms(1000.0), v_ms(1800.0)],
                      "gravity_m_per_s2_random": [a_ms2(-300.0), a_ms2(-200.0)] },
        "spawn": "1-player only; on the frame the kart lands (ACTION_STARTED_TOUCH_GROUND) on a terrain with TERRAIN_FLAG_LANDING_SPARKS (asphalt, stone, icy road), when |speed| > 6.0 (FP8) and the jump height exceeded 22.0 (FP8): 10 sparks around the kart"
    });

    Ok(EffectsFile {
        format: "ctr-effects",
        version: 1,
        units: json!({
            "length": "meters, kart.json space (+Y up, +Z forward, +X left); 1 CTR world unit = 1/51.2 m",
            "time": "seconds; the game updates effects once per frame at 30 fps",
            "colors": "0..1 multipliers; for textured meshes 1.0 = PSX 0x80 (neutral)",
            "textures": "RGBA8 rows top-first; alpha 0 = transparent (PSX black), 128 = texel blends with the surface's blend mode (STP bit), 255 = texel drawn opaque even on a blended surface"
        }),
        textures: tex.list,
        turbo_flame,
        exhaust_smoke,
        failed_boost_puff,
        skid_marks,
        dirt_dust,
        landing_sparks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchors_match_the_kart_scale() {
        // Smoke spawns at 14.4 world units left/right, 44.8 up, 44.8 back.
        let [l, r] = exhaust_anchors();
        assert!((l[0] - 0.28125).abs() < 1e-3 && (r[0] + 0.28125).abs() < 1e-3);
        assert!((l[1] - 0.875).abs() < 1e-3 && (l[2] + 0.875).abs() < 1e-3);
        // Flame origins: 14.4 / 38.4 / -41.6 world units.
        let [fl, fr] = flame_origins();
        assert!((fl[0] - 0.28125).abs() < 2e-3 && (fr[0] + 0.28125).abs() < 2e-3);
        assert!((fl[1] - 0.75).abs() < 2e-3 && (fl[2] + 0.8125).abs() < 2e-3);
    }

    #[test]
    fn blend_names_follow_psx_abr() {
        assert_eq!(blend_name(0), "mix");
        assert_eq!(blend_name(1), "additive");
        assert_eq!(blend_name(2), "subtractive");
        assert_eq!(blend_name(3), "add_quarter");
    }

    #[test]
    fn unit_conversion() {
        // 0x400 FP8 per frame = 4 world units/frame = 120 world/s = 2.34 m/s.
        assert!((v_ms(1024.0) - 120.0 / 51.2).abs() < 1e-4);
        assert!((a_ms2(-256.0) + 900.0 / 51.2).abs() < 1e-3);
    }
}
