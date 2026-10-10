//! `hud.json`: CTR's 1-player race HUD. Sprites (fonts, item icons, racer portraits) come from
//! the disc (`shared.mpk` icons + `shared.vrm`); the big rank digits and the Wumpa fruit are
//! small Gouraud meshes from `shared.mpk` (`big1`, `fruitdisp`). Layout and drawing rules are
//! code constants of the executable, transcribed from the CTR-native decompilation
//! (`game/UI/*.c`, `DecalFont.c`, `DecalHUD.c`, `zGlobal_DATA.c` `hud_1P_P1`); every element
//! cites its source.
//!
//! Coordinates are CTR framebuffer pixels: the 1P race view is **512×216**, origin top-left,
//! +y down (`UI_ConvertX_2/Y_2` centre the HUD on (256, 108)). The PS1 shows that framebuffer
//! on a 4:3 TV, so pixels are 0.5625 as wide as tall: for square pixels use x·0.5625
//! (a 288×216 canvas), or keep 512×216 and stretch to 4:3.

use serde::Serialize;
use serde_json::{json, Value};

use crate::big::{self, BigFile};
use crate::ctr::Model;
use crate::kart::Texture;
use crate::mpk;
use crate::vram::{psx_to_rgba, Vram};
use crate::{Error, Result};

/// Framebuffer size of the 1P race view.
pub const SCREEN: [u32; 2] = [512, 216];

#[derive(Serialize)]
pub struct Sprite {
    pub name: String,
    pub texture: usize,
    /// Drawn size at scale 1.0 (`(u1 - u0) × (v2 - v0)`, as DecalHUD/DecalFont do).
    pub size: [u32; 2],
}

#[derive(Serialize)]
pub struct Glyph {
    pub ch: String,
    pub sprite: usize,
}

#[derive(Serialize)]
pub struct Font {
    pub name: &'static str,
    /// Pen advance for letters/digits, for `.` and `:`, and the line height.
    pub advance: u32,
    pub advance_punct: u32,
    pub line_height: u32,
    pub glyphs: Vec<Glyph>,
}

#[derive(Serialize)]
pub struct HudMesh {
    pub name: String,
    /// Triangles in HUD pixels relative to the element anchor (+y down), RGB 0..1.
    pub positions: Vec<[f32; 2]>,
    pub colors: Vec<[f32; 3]>,
    /// The same vertices before projection, in HUD pixels at the element's scale and depth
    /// (+y up, +z away from the viewer); used to spin the Wumpa fruit about +Y. Project with
    /// `(x, -y · 0.5625)`.
    pub positions3d: Vec<[f32; 3]>,
}

#[derive(Serialize)]
pub struct HudFile {
    pub format: &'static str,
    pub version: u32,
    pub screen: Value,
    pub textures: Vec<Texture>,
    pub sprites: Vec<Sprite>,
    pub fonts: Vec<Font>,
    pub colors: Value,
    pub meshes: Vec<HudMesh>,
    pub elements: Value,
}

struct Builder<'a> {
    vram: &'a Vram,
    textures: Vec<Texture>,
    sprites: Vec<Sprite>,
}

impl Builder<'_> {
    /// Item icons are stored on their side; `DecalHUD_DrawWeapon(.., rot = 1)` turns them a
    /// quarter turn (texture top-right corner to the screen's top-left). Bake that in.
    fn sprite_rotated(&mut self, name: &str, icon: &mpk::Icon) -> usize {
        let i = self.sprite(name, icon);
        let ti = self.sprites[i].texture;
        let t = &self.textures[ti];
        let (w, h) = (t.width as usize, t.height as usize);
        let mut rgba = vec![0u8; w * h * 4];
        // Rotated image is h wide, w tall: screen (sx, sy) <- texture (w - 1 - sy, sx).
        for sy in 0..w {
            for sx in 0..h {
                let (u, v) = (w - 1 - sy, sx);
                let src = (v * w + u) * 4;
                let dst = (sy * h + sx) * 4;
                rgba[dst..dst + 4].copy_from_slice(&t.rgba[src..src + 4]);
            }
        }
        let t = &mut self.textures[ti];
        t.rgba = rgba;
        t.width = h as u32;
        t.height = w as u32;
        let s = &mut self.sprites[i];
        s.size = [s.size[1], s.size[0]];
        i
    }

    fn sprite(&mut self, name: &str, icon: &mpk::Icon) -> usize {
        let l = icon.layout;
        let (u0, v0) = (l.uv[0][0] as u32, l.uv[0][1] as u32);
        let (u1, v1) = (l.uv[3][0] as u32, l.uv[3][1] as u32);
        let (w, h) = (u1.saturating_sub(u0) + 1, v1.saturating_sub(v0) + 1);
        // HUD sprites are drawn opaque (black texels transparent).
        let opaque = crate::ctr::TexLayout {
            tpage: l.tpage | 0x60,
            ..l
        };
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for v in v0..v0 + h {
            for u in u0..u0 + w {
                rgba.extend(psx_to_rgba(self.vram.color(&opaque, u, v), false));
            }
        }
        self.textures.push(Texture {
            name: name.to_string(),
            width: w,
            height: h,
            rgba,
        });
        self.sprites.push(Sprite {
            name: name.to_string(),
            texture: self.textures.len() - 1,
            size: [
                (l.uv[1][0] as u32).saturating_sub(u0),
                (l.uv[2][1] as u32).saturating_sub(v0),
            ],
        });
        self.sprites.len() - 1
    }
}

/// Font glyph names in `shared.mpk` mapped to characters (`font_characterIconID`).
fn glyph_char(name: &str) -> Option<&'static str> {
    const PUNCT: [(&str, &str); 14] = [
        ("apostrophe", "'"),
        ("bang", "!"),
        ("colon", ":"),
        ("comma", ","),
        ("equal", "="),
        ("hyphen", "-"),
        ("percent", "%"),
        ("period", "."),
        ("plus", "+"),
        ("question", "?"),
        ("underscore", "_"),
        ("slash", "/"),
        ("delete", "<"),
        ("star", "*star"),
    ];
    const ALNUM: &str = "0123456789abcdefghijklmnopqrstuvwxyz";
    if name.len() == 1 {
        let i = ALNUM.find(name)?;
        return Some(&ALNUM[i..i + 1]);
    }
    PUNCT.iter().find(|p| p.0 == name).map(|p| p.1)
}

fn font(
    b: &mut Builder,
    group: &mpk::IconGroup,
    prefix: &str,
    name: &'static str,
    adv: [u32; 3],
) -> Font {
    let mut glyphs = Vec::new();
    for icon in &group.icons {
        let base = icon.name.strip_prefix(prefix).unwrap_or(&icon.name);
        if let Some(ch) = glyph_char(base) {
            if glyphs.iter().any(|g: &Glyph| g.ch == ch) {
                continue;
            }
            let s = b.sprite(&format!("{name}_{base}"), icon);
            glyphs.push(Glyph {
                ch: ch.to_string(),
                sprite: s,
            });
        }
    }
    Font {
        name,
        advance: adv[0],
        advance_punct: adv[1],
        line_height: adv[2],
        glyphs,
    }
}

/// Flattens a flat-shaded HUD model to pixels: `screen = model · scale/4096 · 256/z`
/// (`UI_ConvertX_2` puts the HUD camera's focal length at 256), y squashed by 0x360/0x600
/// (`PushBuffer_SetMatrixVP` aspect) and flipped to +y down.
fn flatten(model: &crate::ctr::ModelMesh, factor: f32) -> Result<HudMesh> {
    let tris = model.triangles(&model.frame)?;
    let mut positions = Vec::new();
    let mut positions3d = Vec::new();
    let mut colors = Vec::new();
    for t in &tris {
        for i in 0..3 {
            let p = t.positions[i];
            positions.push([p[0] * factor, -p[1] * factor * 0.5625]);
            positions3d.push([p[0] * factor, p[1] * factor, p[2] * factor]);
            let c = t.colors[i];
            colors.push([
                c[0] as f32 / 255.0,
                c[1] as f32 / 255.0,
                c[2] as f32 / 255.0,
            ]);
        }
    }
    Ok(HudMesh {
        name: model.name.clone(),
        positions,
        colors,
        positions3d,
    })
}

/// Gradient colour tables (`data.colors`, 0xBBGGRR; corners TL, TR, BL, BR).
fn gradient(words: [u32; 4]) -> Value {
    json!(words
        .iter()
        .map(|w| [
            (w & 0xFF) as f32 / 255.0,
            ((w >> 8) & 0xFF) as f32 / 255.0,
            ((w >> 16) & 0xFF) as f32 / 255.0
        ])
        .collect::<Vec<_>>())
}

pub fn build(bf: &mut BigFile) -> Result<HudFile> {
    let mut vram = Vram::new();
    vram.load_vrm(&bf.read(big::index::SHARED_VRM)?)?;
    let mpk_file = bf.read(big::index::SHARED_MPK)?;
    let data = crate::ctr::unwrap_patched(&mpk_file)?;
    let groups = mpk::icon_groups(&mpk_file)?;
    let icons = mpk::icons(&mpk_file)?;
    let mut b = Builder {
        vram: &vram,
        textures: Vec::new(),
        sprites: Vec::new(),
    };

    let group = |id: i16| {
        groups
            .iter()
            .find(|g| g.id == id)
            .ok_or_else(|| Error(format!("no icon group {id}")))
    };
    let big_font = font(&mut b, group(4)?, "", "big", [17, 11, 17]);
    let small_font = font(&mut b, group(5)?, "small", "small", [13, 7, 8]);
    let icon = |index: i32| {
        icons
            .iter()
            .find(|i| i.index == index)
            .map(|i| i.icon.clone())
    };

    // Item icons: ptrIcons[heldItemID + 5] (UI_Weapon_DrawSelf); juiced TNT/potion/shield at
    // +0x11; Uka mask 0x32.
    let items = [
        "turbo",
        "bomb",
        "missile",
        "tnt",
        "potion",
        "spring",
        "shield",
        "aku_mask",
        "clock",
        "warpball",
        "bomb_x3",
        "missile_x3",
        "invisibility",
        "super_engine",
    ];
    let mut item_sprites = serde_json::Map::new();
    for (i, name) in items.iter().enumerate() {
        if let Some(ic) = icon(i as i32 + 5) {
            let s = b.sprite_rotated(&format!("item_{name}"), &ic);
            item_sprites.insert(name.to_string(), json!(s));
        }
    }
    for (idx, name) in [
        (0x11 + 3, "tnt_juiced"),
        (0x11 + 4, "potion_juiced"),
        (0x11 + 6, "shield_juiced"),
        (0x32, "uka_mask"),
        (0x31, "juiced_shine"),
    ] {
        if let Some(ic) = icon(idx) {
            let s = if name == "juiced_shine" {
                b.sprite(&format!("item_{name}"), &ic)
            } else {
                b.sprite_rotated(&format!("item_{name}"), &ic)
            };
            item_sprites.insert(name.to_string(), json!(s));
        }
    }
    // Racer portraits (MetaDataCharacters iconID), big rank icons 0x19.. (3P/4P only).
    let mut portraits = serde_json::Map::new();
    for i in &icons {
        if (32..=55).contains(&i.index) {
            let s = b.sprite(&format!("portrait_{}", i.icon.name), &i.icon);
            portraits.insert(i.icon.name.clone(), json!(s));
        }
    }
    let mut rank_icons = Vec::new();
    for idx in 0x19..=0x1C {
        if let Some(ic) = icon(idx) {
            rank_icons.push(b.sprite(&format!("rank_{}", ic.name), &ic));
        }
    }
    // Missile/warp orb lock-on square: ptrIcons[UI_TRACKER_LOCK_ICON = 0x2D] "target".
    let lock_on = icon(0x2D)
        .map(|ic| b.sprite("lock_on_target", &ic))
        .ok_or_else(|| Error("shared.mpk has no icon 0x2D (target)".into()))?;

    // Meshes: big rank digits (mesh i = rank i+1, chosen by the instance depth 0x100 + rank)
    // and the Wumpa fruit.
    let models = mpk::models(&mpk_file)?;
    let find = |name: &str| {
        models
            .iter()
            .find(|m| m.1 == name)
            .map(|m| m.0)
            .ok_or_else(|| Error(format!("shared.mpk has no {name}")))
    };
    let big1 = Model::parse_at(data, find("big1")?)?;
    let fruit = Model::parse_at(data, find("fruitdisp")?)?;
    let mut meshes = Vec::new();
    // hud_1P_P1[BIG1] = (50, 191, z 256, scale 5120): factor 5120/4096 · 256/256.
    for m in &big1.meshes {
        meshes.push(flatten(m, 5120.0 / 4096.0)?);
    }
    // hud_1P_P1[FRUIT_MODEL] = (316, 24, z 512, scale 4096): factor 0.5.
    meshes.push(flatten(&fruit.meshes[0], 256.0 / 512.0)?);

    let colors = json!({
        "note": "font/sprite vertex colours multiply the texture with 0x80 = 1.0 (values up to ~2 brighten); gradients list the top-left, top-right, bottom-left, bottom-right corner colours (DecalFont passes ptrColor[style][0..3] to the 4 quad corners)",
        "orange": gradient([0xC8FF, 0xC8FF, 0xFF, 0xFF]),
        "periwinkle": gradient([0xFFC8C8, 0xFFC8C8, 0xE1A0A0, 0xE1A0A0]),
        "red": gradient([0xFF, 0xFF, 0x80, 0x80]),
        "white": gradient([0xFFFFFF; 4]),
        "gray_neutral": gradient([0x808080; 4]),
    });

    let speedo_inner = [
        (-65, 16),
        (-70, 0),
        (-65, -16),
        (-50, -29),
        (-25, -39),
        (0, -42),
        (25, -39),
    ];
    let speedo_outer = [
        (-54, 13),
        (-58, 0),
        (-54, -13),
        (-42, -24),
        (-21, -32),
        (0, -35),
        (21, -32),
    ];
    let speedo_ticks = [
        (-48, 11),
        (-51, 0),
        (-48, -11),
        (-37, -21),
        (-18, -29),
        (0, -31),
        (18, -29),
    ];

    let elements = json!({
        "mode_note": "CTR's 1P race HUD has two modes toggled with Triangle (HudAndDebugFlags & 8): speedometer (with jump meter) or track map. This file describes speedometer mode.",
        "rank_list": {
            "source": "UI_Rank.c UI_DrawRankedDrivers (1P)",
            "rows": 4,
            "row_height": 27,
            "number": { "font": "small", "x": 52, "y0": 56, "color": "white", "finished_color": "red", "text": "1..4" },
            "portrait": { "x": 20, "y0": 57, "scale": 1.0, "rule": "portrait of the racer currently in that place (top 4 only); slides between rows over 5 frames when places change" },
        },
        "weapon": {
            "source": "UI_Weapon.c UI_Weapon_DrawSelf; hud_1P_P1[WEAPON] = (232, 5), scale 1.0",
            "x": 232, "y": 5, "scale": 1.0, "anchor": "top-left",
            "note": "item sprites are exported upright (the game stores them rotated and draws them with a quarter turn); they are drawn with 50% semi-transparency for STP texels (TRANS_50_DECAL)",
            "sprites": item_sprites,
            "rules": "icon = ptrIcons[heldItemID + 5]; with >= 10 wumpa, TNT/potion/shield use the juiced icons; villains get the Uka mask; while the roulette spins a random racing item is shown each frame; a quantity digit (small font, white) is drawn at (232, 5) when holding 3 missiles/bombs; the icon flickers (every other frame) while noItemTimer runs; with >= 10 wumpa a red 'juiced_shine' sprite pulses behind it at (286, 9), scale 0.6"
        },
        "lock_on": {
            "source": "UI_RaceHud.c:48 UI_TRACKER_LOCK_ICON = 0x2D; UI_TrackerSelf (UI_RaceHud.c:217), UI_TrackerBG (UI_Icon.c:184)",
            "sprite": lock_on,
            "rule": "drawn on the targeted driver's own view, centred on their kart's screen position, while a missile or warp orb tracks them; colour red (0xFF), white on the frames the lock-on beep (OtherFX 0x56) plays: every 5 / 10 / 30 frames for a tracker distance (sqrt(distanceToTarget) / 0x32) up to 100 / 200 / beyond; the square zooms in over 8 frames on a new lock (trackerAnim, UI_TRACKER_TIMER_NEW_TARGET_FRAMES); the sprite is one quarter of the frame: UI_TrackerBG draws it 4 times, mirrored in x and/or y about the centre, as one tinted (vertex colour x texel) semi-transparent 50/50 square (transparency 1 = PSX blend mode 0); side arrows (orange for a missile, blue for a warp orb) close in with the distance"
        },
        "lap": {
            "source": "UI_RaceHud.c UI_DrawLapCount; hud_1P_P1[LAP_COUNT] = (484, 8)",
            "label": { "text": "LAP", "font": "small", "x": 484, "y": 8, "justify": "right", "color": "periwinkle" },
            "value": { "text": "{lap}/{laps}", "font": "big", "x": 484, "y": 16, "justify": "right", "color": "periwinkle" }
        },
        "wumpa": {
            "source": "UI_DrawNum.c UI_DrawNumWumpa; UI_RenderFrame.c; hud_1P_P1[FRUIT_MODEL] = (316, 24, z 512), [WUMPA_COUNT] = (336, 16)",
            "fruit": { "mesh": "fruitdisp", "x": 316, "y": 24, "rule": "3D fruit model drawn at the anchor (centre), spinning; flies in from the pickup point when collecting" },
            "x_sign": { "text": "x", "font": "small", "x": 336, "y": 20, "color": "orange" },
            "count": { "text": "{wumpa}", "font": "big", "x": 349, "y": 16, "color": "orange" }
        },
        "rank": {
            "source": "UI_RaceHud.c UI_DrawPosSuffix; UI_Instance.c (big number instance); hud_1P_P1[BIG1] = (50, 191, z 256, scale 5120), [RANK] = (70, 171)",
            "number": { "meshes": "big1..big8 (mesh index = rank - 1)", "x": 50, "y": 191, "anchor": "centre" },
            "suffix": { "text": "ST|ND|RD|TH (lngStrings 0x19..0x1C)", "font": "big", "x": 70, "y": 171, "color": "orange" }
        },
        "speedometer": {
            "source": "UI_Speedometer.c UI_DrawSpeedBG, UI_DrawSpeedNeedle; hud_1P_P1[SPEEDOMETER] = (414, 145)",
            "origin": [480, 190],
            "arc_inner": speedo_inner, "arc_outer": speedo_outer, "tick_inner": speedo_ticks,
            "segments": "6 Gouraud quads between consecutive spokes i, i+1 (inner_i, outer_i, inner_i+1, outer_i+1); colour at spoke i = spoke_colors[i]",
            "spoke_colors": [[0.0, 0.71, 0.0], [0.0, 0.71, 0.0], [0.0, 0.71, 0.0], [1.0, 0.82, 0.0], [0.86, 0.0, 0.0], [0.86, 0.0, 0.0], [0.86, 0.0, 0.0]],
            "outline": "1-px white lines (with a black copy offset by (+1, +1)): each spoke from arc_inner[i] to tick_inner[i], and the inner and outer arcs through consecutive points",
            "backdrop": "black 50% semi-transparent triangles (arc_outer[i], arc_outer[i+1], (arc_outer[6].x, arc_outer[0].y)) darken the dial's inside",
            "needle": {
                "pivot": [414 + 65, 145 + 41],
                "length": 60, "half_width": 6, "y_scale": 0.625,
                "triangles": [
                    { "points": ["tip", "back", "side_plus"], "colors": [[91, 91, 0], [50, 43, 1], [255, 187, 0]] },
                    { "points": ["tip", "back", "side_minus"], "colors": [[255, 255, 255], [156, 105, 0], [255, 255, 0]] }
                ],
                "points": "with a = needle angle, b = a + 90°, s = sin, c = cos: tip = pivot + (60·s(a), 0.625·60·c(a)); back = pivot − (6·s(a), 0.625·6·c(a)); side± = pivot ± (6·s(b), 0.625·8·c(b))",
                "angle": "CTR angle convention (sin for x, cos for y, y down). speed in [0, accel_speed] maps linearly to a = 305.2° → 213.75°; speed in [accel_speed, accel_speed + sacred_fire_speed] maps to 213.75° → 157.5°. So the needle sweeps from lower-left (standstill) up to vertical-left at normal top speed and on to upper-right at full boost. speedometerNeedleValue is the driver's smoothed speed."
            }
        },
        "slide_meter": {
            "source": "UI_Meter.c UI_DrawSlideMeter; hud_1P_P1[SLIDE_METER] = (490, 206)",
            "box": { "x": 441, "y": 199, "w": 49, "h": 7, "outline": "black wire box" },
            "empty_color": [0.5, 0.5, 0.5],
            "fill": "while power-sliding, turbo_MeterRoomLeft counts down from const_turboMaxRoom; fill width = 49 − 49 · room_left / max_room, growing leftwards from the right edge (x 490)",
            "fill_color": "green (0, 255, 0) while room_left > const_turboLowRoomWarning; red (255, 0, 0) inside the boost window (room_left <= low_room_warning)",
            "when": "drawn every frame while racing (empty gray bar when not sliding)"
        },
        "jump_meter": {
            "source": "UI_Meter.c UI_JumpMeter_Update / UI_JumpMeter_Draw; hud_1P_P1[JUMP_METER] = (491, 206)",
            "bar": { "x": 491, "y": 168, "w": 12, "h": 38, "outline": "black wire box", "empty_color": [0.5, 0.5, 0.5] },
            "fill": "height = 38 · jump_meter / 2400 from the bottom (y 206); jump_meter is airtime in 1/960 s (game 'ms'), clamped to 2400 (2.5 s)",
            "fill_color": "red < 640 (0.67 s), green < 960 (1.0 s), cyan < 1440 (1.5 s), blue >= 1440 — the three hang-time boost tiers",
            "readout": "above the bar, a white box (x 471..505, y 161..171) with the airtime as S.T H digits in the debug font",
            "when": "speedometer mode only; set while airborne, held for 1.5 s (jumpMeterTimer 0x5A0) after landing, then cleared"
        }
    });

    Ok(HudFile {
        format: "ctr-hud",
        version: 1,
        screen: json!({
            "width": SCREEN[0], "height": SCREEN[1],
            "pixel_aspect": 0.5625,
            "note": "CTR framebuffer pixels (+y down); the image is shown 4:3, so a square-pixel canvas is 288x216 (x * 0.5625)"
        }),
        textures: b.textures,
        sprites: b.sprites,
        fonts: vec![big_font, small_font],
        colors,
        meshes,
        elements: json!({
            "portraits": portraits,
            "rank_icons_3p4p": rank_icons,
            "layout": elements,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyph_names_map_to_characters() {
        assert_eq!(glyph_char("7"), Some("7"));
        assert_eq!(glyph_char("q"), Some("q"));
        assert_eq!(glyph_char("slash"), Some("/"));
        assert_eq!(glyph_char("circlebutton"), None);
    }

    #[test]
    fn gradient_unpacks_bgr() {
        let g = gradient([0xC8FF, 0xC8FF, 0xFF, 0xFF]);
        assert_eq!(g[0][0].as_f64().unwrap(), 1.0);
        assert!((g[0][1].as_f64().unwrap() - 200.0 / 255.0).abs() < 1e-6);
        assert_eq!(g[2][1].as_f64().unwrap(), 0.0);
    }
}
