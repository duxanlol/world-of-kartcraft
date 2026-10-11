//! Draws CTR's 1-player race HUD from `hud.json` into a 512×216 RGBA8 overlay, every frame.
//!
//! The canvas is CTR's framebuffer (see [`crate::hud`]): 512×216 pixels, +y down, shown on a
//! 4:3 screen (pixel aspect 0.5625). The background is fully transparent; opaque HUD pixels get
//! alpha 255 and the speedometer's 50% black backdrop alpha 128. All layout comes from
//! `hud.json` (`elements.layout`, transcribed from the decompilation); the per-frame drawing
//! follows `game/UI/*.c` (see `docs/ctr-assets.md`, "HUD").
//!
//! ```no_run
//! use ctr_assets::hud_draw::{HudRenderer, HudState, WIDTH, HEIGHT};
//! let hud = HudRenderer::load("kart-data/hud.json".as_ref()).unwrap();
//! let mut pixels = vec![0u8; WIDTH * HEIGHT * 4];
//! let state = HudState { speedometer: true, speed: 9000.0, accel_speed: 10000.0, ..Default::default() };
//! hud.render(&state, &mut pixels);
//! ```

use std::path::Path;

use serde_json::Value;

use crate::{Error, Result};

pub const WIDTH: usize = 512;
pub const HEIGHT: usize = 216;

/// What to draw this frame. `Default` draws nothing.
#[derive(Clone, Debug, Default)]
pub struct HudState {
    /// Speedometer mode (dial + needle). CTR toggles it against the track map with Triangle.
    pub speedometer: bool,
    /// Driver speed, `const_AccelSpeed` and `const_SacredFireSpeed` in CTR speed units (any
    /// consistent unit works: only ratios are used). The needle sweeps 305.2° → 213.75° over
    /// `0..accel_speed` and 213.75° → 157.5° over `accel_speed..accel_speed+sacred_fire_speed`.
    pub speed: f32,
    pub accel_speed: f32,
    pub sacred_fire_speed: f32,
    /// Drift meter `(room_left, max_room, low_room_warning)`, any consistent time unit (CTR
    /// uses ms). Fill = 49 − 49·room_left/max_room, red once `room_left <= low_room_warning`.
    /// `None` draws the empty gray meter, as the game does while racing outside a slide.
    pub drift_meter: Option<(u32, u32, u32)>,
    /// Hides the drift meter entirely (the game hides it in battle and after finishing).
    pub hide_drift_meter: bool,
    /// Jump meter airtime in milliseconds (real time); `None` hides the meter. The caller
    /// times visibility: CTR shows it while airborne and for 1.5 s after landing (it also only
    /// shows it in speedometer mode). Colours: red < 667 ms, green < 1000, cyan < 1500, blue.
    pub jump: Option<u32>,
    /// Wumpa count (`None` hides the counter and fruit).
    pub wumpa: Option<u8>,
    /// 10+ wumpa: juiced item icons and the pulsing shine behind the item.
    pub juiced: bool,
    /// `(current, total)` laps.
    pub lap: Option<(u8, u8)>,
    /// Race position 1..=8.
    pub place: Option<u8>,
    /// `(heldItemID, count)`: CTR item ids (0 turbo, 1 bomb, 2 missile, 3 TNT, 4 potion,
    /// 5 spring, 6 shield, 7 mask, 8 clock, 9 warpball, 10 bomb ×3, 11 missile ×3,
    /// 12 invisibility, 13 super engine, 0x10 roulette). `count > 0` draws the digit.
    pub item: Option<(u8, u8)>,
    /// Top-4 portraits, by portrait name (`crash`, `cortex`, `tiny`, `coco`, `ngin`, `dingo`,
    /// `polar`, `pura`, `roo`, `papu`, `komodojoe`, `pinstripe`, `ntropy`, `penguin`,
    /// `fakecrash`, `oxide`). Empty hides the list.
    pub ranks: Vec<String>,
    /// Seconds since start, for the fruit spin (0.9375 turns/s), juiced shine pulse and the
    /// item roulette.
    pub time_seconds: f32,
    /// The start lights' timer (see [`start_lights`]); `None` draws none.
    pub start_timer: Option<i32>,
}

/// Where an element should be pinned when the 512×216 canvas is shown on a wider screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    TopLeft,
    Top,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Clone, Debug)]
pub struct Region {
    pub name: &'static str,
    /// `[x, y, w, h]` in canvas pixels.
    pub rect: [i32; 4],
    pub anchor: Anchor,
}

/// Horizontal canvas spans for drawing the HUD in three crops on a wide screen: pin `left` to
/// the screen's left edge, `middle` to its centre and `right` to its right edge. Every element
/// lies entirely inside one span.
pub const CROP_LEFT: [i32; 2] = [0, 120];
pub const CROP_MIDDLE: [i32; 2] = [120, 400];

/// CTR's traffic-light timer (`gGT->trafficLightsTimer`) counts its "ms": 32 for a 30 Hz frame
/// (MainFrame.c:188-203, `elapsedTimeMS`), 960 a second.
pub const LIGHTS_UNITS_PER_SECOND: f32 = 960.0;
/// The timer as the race starts (MainGameStart.c:17), and while the lights slide in
/// (DotLights.c:21 `DOT_LIGHT_TIMER_HIDE`).
pub const LIGHTS_TIMER_START: i32 = 0xF00;
/// The lights show down to here, sliding out (DotLights.c:16 `DOT_LIGHT_TIMER_SHOW_MIN`).
pub const LIGHTS_TIMER_END: i32 = -0x3BF;
/// The first, second and third red light come on, and the green (DotLights.c:17-20).
pub const LIGHTS_TIMER_RED: [i32; 3] = [0xB40, 0x780, 0x3C0];
pub const LIGHTS_TIMER_GREEN: i32 = 0;
/// The lights' slide reaches the top third of the screen (DotLights.c:63 `rect.h / 3`).
const LIGHTS_DROP: i32 = HEIGHT as i32 / 3;

/// The traffic-light timer `seconds` before GO: GO is 0, and the green light comes on with it.
pub fn lights_timer(seconds: f32) -> i32 {
    (seconds * LIGHTS_UNITS_PER_SECOND).ceil() as i32
}

/// The start lights at a timer: how many are lit (0, the three reds one by one, then 4 with the
/// green) and how far they have slid down, 4096 = in place (DotLights.c:85-152). `None` when
/// they are off screen: before CTR's countdown begins and once they have slid away.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartLights {
    pub lit: u8,
    pub pos: i32,
}

/// `DotLights_TweenPos` (DotLights.c:74-78): `value / 960`, truncated toward zero, by its
/// reciprocal multiply.
fn lights_tween(value: i32) -> i32 {
    // DotLights.c:23 DOT_LIGHT_TWEEN_RECIPROCAL_960, :24 DOT_LIGHT_TWEEN_SHIFT.
    let quotient = ((i64::from(value) * -0x7777_7777i64) >> 32) as i32;
    ((quotient + value) >> 9) - (value >> 31)
}

pub fn start_lights(timer: i32) -> Option<StartLights> {
    if !(LIGHTS_TIMER_END..=LIGHTS_TIMER_START).contains(&timer) {
        return None;
    }
    let [red1, red2, red3] = LIGHTS_TIMER_RED;
    Some(if timer <= LIGHTS_TIMER_GREEN {
        StartLights {
            lit: 4,
            pos: lights_tween((timer + red3) * 4096),
        }
    } else if timer <= red1 {
        let lit = if timer <= red3 {
            3
        } else if timer <= red2 {
            2
        } else {
            1
        };
        StartLights { lit, pos: 4096 }
    } else {
        StartLights {
            lit: 0,
            pos: lights_tween((LIGHTS_TIMER_START - timer) * 4096),
        }
    })
}

/// A start light's sound: CTR's countdown beep (OtherFX 0x45) as each red comes on, its green
/// sound (0x46) with the green.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartSound {
    Red,
    Green,
}

/// The sound a frame plays with the timer at `timer`, last frame's at `prev`: a light's when the
/// timer has crossed into its stretch (DotLights.c:91-136, `trafficLightsTimer_prevFrame`).
pub fn start_light_sound(prev: i32, timer: i32) -> Option<StartSound> {
    if timer < LIGHTS_TIMER_END {
        return None;
    }
    if timer <= LIGHTS_TIMER_GREEN {
        return (prev > LIGHTS_TIMER_GREEN).then_some(StartSound::Green);
    }
    LIGHTS_TIMER_RED
        .iter()
        .rev()
        .find(|&&red| timer <= red)
        .and_then(|&red| (prev > red).then_some(StartSound::Red))
}
pub const CROP_RIGHT: [i32; 2] = [400, 512];

struct Image {
    w: usize,
    h: usize,
    rgba: Vec<u8>,
    /// Drawn size at scale 1 (CTR draws `u1 - u0` × `v2 - v0`).
    draw: [f32; 2],
}

struct Font {
    advance: i32,
    advance_punct: i32,
    glyphs: [Option<u16>; 128],
}

struct Mesh {
    pos2: Vec<[f32; 2]>,
    pos3: Vec<[f32; 3]>,
    col: Vec<[f32; 3]>,
}

type Grad = [[f32; 3]; 4];

struct Layout {
    rank_portrait: [f32; 2],
    rank_number: [f32; 2],
    rank_row: f32,
    weapon: [f32; 2],
    lap_label: [f32; 2],
    lap_value: [f32; 2],
    fruit: [f32; 2],
    x_sign: [f32; 2],
    count: [f32; 2],
    place: [f32; 2],
    suffix: [f32; 2],
    speedo_origin: [f32; 2],
    arc_inner: [[f32; 2]; 7],
    arc_outer: [[f32; 2]; 7],
    tick_inner: [[f32; 2]; 7],
    spoke_colors: [[f32; 3]; 7],
    pivot: [f32; 2],
    slide_box: [f32; 4],
    jump_bar: [f32; 4],
}

pub struct HudRenderer {
    images: Vec<Image>,
    big: Font,
    small: Font,
    orange: Grad,
    periwinkle: Grad,
    white: Grad,
    big_digits: Vec<Mesh>,
    fruit: Mesh,
    items: Vec<(String, usize)>,
    portraits: Vec<(String, usize)>,
    /// `trafficLightIcon[0..4]`: red off, red on, green off, green on; absent in a `hud.json`
    /// converted before they were.
    start_lights: Option<[usize; 4]>,
    l: Layout,
}

fn f(v: &Value) -> Result<f32> {
    v.as_f64()
        .map(|x| x as f32)
        .ok_or_else(|| Error(format!("hud.json: expected a number, got {v}")))
}

fn p2(v: &Value) -> Result<[f32; 2]> {
    Ok([f(&v[0])?, f(&v[1])?])
}

fn xy(v: &Value) -> Result<[f32; 2]> {
    Ok([f(&v["x"])?, f(&v["y"])?])
}

fn grad(v: &Value) -> Result<Grad> {
    let mut g = [[0.0; 3]; 4];
    for (i, c) in g.iter_mut().enumerate() {
        for (k, x) in c.iter_mut().enumerate() {
            *x = f(&v[i][k])?;
        }
    }
    Ok(g)
}

fn arr7(v: &Value) -> Result<[[f32; 2]; 7]> {
    let mut out = [[0.0; 2]; 7];
    for (i, o) in out.iter_mut().enumerate() {
        *o = p2(&v[i])?;
    }
    Ok(out)
}

impl HudRenderer {
    pub fn load(path: &Path) -> Result<Self> {
        let data = std::fs::read(path).map_err(|e| Error(format!("{}: {e}", path.display())))?;
        let j: Value =
            serde_json::from_slice(&data).map_err(|e| Error(format!("{}: {e}", path.display())))?;
        Self::from_json(&j)
    }

    pub fn from_json(j: &Value) -> Result<Self> {
        let textures = j["textures"]
            .as_array()
            .ok_or_else(|| Error("hud.json: no textures".into()))?;
        let sprites = j["sprites"]
            .as_array()
            .ok_or_else(|| Error("hud.json: no sprites".into()))?;
        let mut images = Vec::with_capacity(sprites.len());
        for s in sprites {
            let t = &textures[s["texture"].as_u64().unwrap_or(0) as usize];
            images.push(Image {
                w: t["width"].as_u64().unwrap_or(1) as usize,
                h: t["height"].as_u64().unwrap_or(1) as usize,
                rgba: t["rgba"]
                    .as_array()
                    .map(|a| a.iter().map(|x| x.as_u64().unwrap_or(0) as u8).collect())
                    .unwrap_or_default(),
                draw: p2(&s["size"])?,
            });
        }
        let font = |name: &str| -> Result<Font> {
            let fj = j["fonts"]
                .as_array()
                .and_then(|a| a.iter().find(|f| f["name"] == name))
                .ok_or_else(|| Error(format!("hud.json: no font {name}")))?;
            let mut glyphs = [None; 128];
            for g in fj["glyphs"].as_array().into_iter().flatten() {
                if let (Some(ch), Some(s)) = (g["ch"].as_str(), g["sprite"].as_u64()) {
                    if ch.len() == 1 {
                        let c = ch.as_bytes()[0] as usize;
                        glyphs[c] = Some(s as u16);
                        glyphs[(c as u8).to_ascii_uppercase() as usize] = Some(s as u16);
                    }
                }
            }
            Ok(Font {
                advance: f(&fj["advance"])? as i32,
                advance_punct: f(&fj["advance_punct"])? as i32,
                glyphs,
            })
        };
        let mesh = |name: &str| -> Result<Mesh> {
            let m = j["meshes"]
                .as_array()
                .and_then(|a| a.iter().find(|m| m["name"] == name))
                .ok_or_else(|| Error(format!("hud.json: no mesh {name}")))?;
            let pos2 = m["positions"]
                .as_array()
                .into_iter()
                .flatten()
                .map(p2)
                .collect::<Result<_>>()?;
            let pos3 = m["positions3d"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|v| Ok([f(&v[0])?, f(&v[1])?, f(&v[2])?]))
                .collect::<Result<_>>()?;
            let col = m["colors"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|v| Ok([f(&v[0])?, f(&v[1])?, f(&v[2])?]))
                .collect::<Result<_>>()?;
            Ok(Mesh { pos2, pos3, col })
        };
        let named = |v: &Value| -> Vec<(String, usize)> {
            v.as_object()
                .map(|o| {
                    o.iter()
                        .filter_map(|(k, v)| v.as_u64().map(|s| (k.clone(), s as usize)))
                        .collect()
                })
                .unwrap_or_default()
        };
        let lay = &j["elements"]["layout"];
        let sp = &lay["speedometer"];
        let o = p2(&sp["origin"])?;
        let offset = |a: [[f32; 2]; 7]| a.map(|p| [p[0] + o[0], p[1] + o[1]]);
        let mut spoke_colors = [[0.0; 3]; 7];
        for (i, c) in spoke_colors.iter_mut().enumerate() {
            *c = [
                f(&sp["spoke_colors"][i][0])?,
                f(&sp["spoke_colors"][i][1])?,
                f(&sp["spoke_colors"][i][2])?,
            ];
        }
        let b = |v: &Value| -> Result<[f32; 4]> {
            Ok([f(&v["x"])?, f(&v["y"])?, f(&v["w"])?, f(&v["h"])?])
        };
        let l = Layout {
            rank_portrait: [
                f(&lay["rank_list"]["portrait"]["x"])?,
                f(&lay["rank_list"]["portrait"]["y0"])?,
            ],
            rank_number: [
                f(&lay["rank_list"]["number"]["x"])?,
                f(&lay["rank_list"]["number"]["y0"])?,
            ],
            rank_row: f(&lay["rank_list"]["row_height"])?,
            weapon: xy(&lay["weapon"])?,
            lap_label: xy(&lay["lap"]["label"])?,
            lap_value: xy(&lay["lap"]["value"])?,
            fruit: xy(&lay["wumpa"]["fruit"])?,
            x_sign: xy(&lay["wumpa"]["x_sign"])?,
            count: xy(&lay["wumpa"]["count"])?,
            place: xy(&lay["rank"]["number"])?,
            suffix: xy(&lay["rank"]["suffix"])?,
            speedo_origin: o,
            arc_inner: offset(arr7(&sp["arc_inner"])?),
            arc_outer: offset(arr7(&sp["arc_outer"])?),
            tick_inner: offset(arr7(&sp["tick_inner"])?),
            spoke_colors,
            pivot: p2(&sp["needle"]["pivot"])?,
            slide_box: b(&lay["slide_meter"]["box"])?,
            jump_bar: b(&lay["jump_meter"]["bar"])?,
        };
        let big_digits = (1..=8)
            .map(|i| mesh(&format!("big{i}")))
            .collect::<Result<_>>()?;
        Ok(Self {
            big: font("big")?,
            small: font("small")?,
            orange: grad(&j["colors"]["orange"])?,
            periwinkle: grad(&j["colors"]["periwinkle"])?,
            white: grad(&j["colors"]["white"])?,
            big_digits,
            fruit: mesh("fruitdisp")?,
            items: named(&lay["weapon"]["sprites"]),
            portraits: named(&j["elements"]["portraits"]),
            start_lights: lay["start_lights"]["sprites"].as_array().and_then(|a| {
                let ids: Vec<usize> = a
                    .iter()
                    .filter_map(|v| v.as_u64().map(|s| s as usize))
                    .filter(|&s| s < images.len())
                    .collect();
                ids.try_into().ok()
            }),
            images,
            l,
        })
    }

    /// Whether this HUD has CTR's start lights (a `hud.json` converted with them).
    pub fn has_start_lights(&self) -> bool {
        self.start_lights.is_some()
    }

    /// The start lights' top-left corner and the size of one light (canvas pixels) at a slide
    /// `pos` (4096 = in place), 1P at scale 1.0 (DotLights.c:58-68).
    fn start_lights_rect(&self, pos: i32) -> Option<[i32; 4]> {
        let img = self.images.get(self.start_lights?[0])?;
        let (w, h) = (img.draw[0] as i32, img.draw[1] as i32);
        let x = (WIDTH as i32 - w * 4) / 2;
        let y = ((LIGHTS_DROP * pos) >> 12) - h;
        Some([x, y, w, h])
    }

    /// Screen regions of every element (canvas pixels) and how to anchor them.
    pub fn regions(&self) -> Vec<Region> {
        let l = &self.l;
        let r = |name, x: f32, y: f32, w: f32, h: f32, anchor| Region {
            name,
            rect: [x as i32, y as i32, w as i32, h as i32],
            anchor,
        };
        let lights = self.start_lights_rect(4096).map(|[x, y, w, h]| Region {
            name: "start_lights",
            rect: [x, y, 4 * w, h],
            anchor: Anchor::Top,
        });
        let mut regions = vec![
            r(
                "rank_list",
                l.rank_portrait[0],
                l.rank_portrait[1],
                46.0,
                4.0 * l.rank_row,
                Anchor::TopLeft,
            ),
            r(
                "place",
                l.place[0] - 36.0,
                l.place[1] - 26.0,
                104.0 - l.place[0] + 36.0 + l.suffix[0] - 70.0,
                52.0,
                Anchor::BottomLeft,
            ),
            r("item", l.weapon[0], l.weapon[1], 56.0, 40.0, Anchor::Top),
            r(
                "wumpa",
                l.fruit[0] - 16.0,
                l.count[1] - 2.0,
                l.count[0] + 3.0 * 17.0 - (l.fruit[0] - 16.0),
                26.0,
                Anchor::Top,
            ),
            r(
                "lap",
                432.0,
                l.lap_label[1],
                l.lap_label[0] - 432.0,
                25.0,
                Anchor::TopRight,
            ),
            r(
                "speedometer",
                l.speedo_origin[0] - 72.0,
                l.speedo_origin[1] - 44.0,
                72.0 + 27.0,
                62.0,
                Anchor::BottomRight,
            ),
            r(
                "drift_meter",
                l.slide_box[0],
                l.slide_box[1],
                l.slide_box[2] + 1.0,
                l.slide_box[3] + 1.0,
                Anchor::BottomRight,
            ),
            r(
                "jump_meter",
                471.0,
                161.0,
                34.0,
                l.jump_bar[1] + l.jump_bar[3] - 161.0 + 1.0,
                Anchor::BottomRight,
            ),
        ];
        regions.extend(lights);
        regions
    }

    /// Clears `out` (512×216 RGBA8, rows top-first) to transparent and draws `state`.
    pub fn render(&self, s: &HudState, out: &mut [u8]) {
        assert!(
            out.len() >= WIDTH * HEIGHT * 4,
            "HUD buffer must be 512*216*4 bytes"
        );
        out[..WIDTH * HEIGHT * 4].fill(0);
        let c = &mut Canvas { px: out };
        let l = &self.l;

        // Rank list (UI_DrawRankedDrivers).
        for (i, who) in s.ranks.iter().take(4).enumerate() {
            let y = i as f32 * l.rank_row;
            if let Some(&(_, sp)) = self.portraits.iter().find(|p| p.0 == *who) {
                self.sprite(c, sp, l.rank_portrait[0], l.rank_portrait[1] + y, None, 1.0);
            }
            let d = [b'1' + i as u8];
            self.text(
                c,
                &self.small,
                &d,
                l.rank_number[0],
                l.rank_number[1] + y,
                &self.white,
                false,
            );
        }

        // Item (UI_Weapon_DrawSelf / DrawBG).
        if let Some((id, count)) = s.item {
            if s.juiced {
                if let Some(sh) = self.item_sprite("juiced_shine") {
                    // wumpaShineTheta += 0x100 per frame (4096 = one turn), 30 fps.
                    let pulse = (s.time_seconds * 30.0 * 256.0 / 4096.0 * std::f32::consts::TAU)
                        .sin()
                        .abs();
                    let g = [[0.5 + 0.5 * pulse, 0.0, 0.0]; 4];
                    self.sprite(c, sh, 286.0, 9.0, Some(&g), 0.6);
                }
            }
            let id = if id == 0x10 {
                // Roulette: a random racing item each frame (spring shown as turbo).
                let r = ((s.time_seconds * 30.0) as u32).wrapping_mul(2654435761) >> 16;
                let x = (r % 12) as u8;
                if x == 5 {
                    0
                } else {
                    x
                }
            } else {
                id
            };
            const NAMES: [&str; 14] = [
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
            let mut name = NAMES.get(id as usize).copied().unwrap_or("turbo");
            if s.juiced {
                name = match id {
                    3 => "tnt_juiced",
                    4 => "potion_juiced",
                    6 => "shield_juiced",
                    _ => name,
                };
            }
            if let Some(sp) = self.item_sprite(name) {
                self.sprite(c, sp, l.weapon[0], l.weapon[1], None, 1.0);
            }
            if count > 0 && id != 0x10 {
                let d = [b'0' + count.min(9)];
                self.text(
                    c,
                    &self.small,
                    &d,
                    l.weapon[0],
                    l.weapon[1],
                    &self.white,
                    false,
                );
            }
        }

        // Wumpa (UI_DrawNumWumpa + fruit instance spinning 0x80/frame at 30 fps).
        if let Some(n) = s.wumpa {
            let angle = s.time_seconds * 30.0 * (0x80 as f32 / 4096.0) * std::f32::consts::TAU;
            self.mesh_spin(c, &self.fruit, l.fruit, angle);
            self.text(
                c,
                &self.small,
                b"x",
                l.x_sign[0],
                l.x_sign[1],
                &self.orange,
                false,
            );
            let mut buf = [0u8; 3];
            let t = num(n as u32, &mut buf);
            self.text(c, &self.big, t, l.count[0], l.count[1], &self.orange, false);
        }

        // Lap (UI_DrawLapCount).
        if let Some((lap, laps)) = s.lap {
            self.text(
                c,
                &self.small,
                b"LAP",
                l.lap_label[0],
                l.lap_label[1],
                &self.periwinkle,
                true,
            );
            let mut a = [0u8; 3];
            let mut b2 = [0u8; 3];
            let la = num(lap.min(laps.max(1)) as u32, &mut a).to_vec();
            let lb = num(laps as u32, &mut b2);
            let mut line = [0u8; 8];
            let n = la.len();
            line[..n].copy_from_slice(&la);
            line[n] = b'/';
            line[n + 1..n + 1 + lb.len()].copy_from_slice(lb);
            self.text(
                c,
                &self.big,
                &line[..n + 1 + lb.len()],
                l.lap_value[0],
                l.lap_value[1],
                &self.periwinkle,
                true,
            );
        }

        // Place (big number instance + UI_DrawPosSuffix).
        if let Some(p) = s.place {
            if (1..=8).contains(&p) {
                let m = &self.big_digits[p as usize - 1];
                c.mesh(&m.pos2, &m.col, l.place);
                let suf: &[u8] = match p {
                    1 => b"ST",
                    2 => b"ND",
                    3 => b"RD",
                    _ => b"TH",
                };
                self.text(
                    c,
                    &self.big,
                    suf,
                    l.suffix[0],
                    l.suffix[1],
                    &self.orange,
                    false,
                );
            }
        }

        // Speedometer (UI_DrawSpeedBG / UI_DrawSpeedNeedle).
        if s.speedometer {
            self.speedometer(c, s);
        }

        // Drift meter (UI_DrawSlideMeter).
        if !s.hide_drift_meter {
            let [x, y, w, h] = l.slide_box;
            c.rect(x, y, w, h, [0.5; 3], 1.0);
            if let Some((room, max, warn)) = s.drift_meter {
                if room > 0 && max > 0 {
                    let len = (w - w * room.min(max) as f32 / max as f32).round();
                    let col = if room <= warn {
                        [1.0, 0.0, 0.0]
                    } else {
                        [0.0, 1.0, 0.0]
                    };
                    c.rect(x + w - len, y, len, h, col, 1.0);
                }
            }
            c.wire(x, y, w, h, [0.0; 3]);
        }

        // Jump meter (UI_JumpMeter_Draw): airtime in 1/960 s, full at 2400.
        if let Some(ms) = s.jump {
            let t = (ms as f32 * 0.96).min(2400.0);
            let [x, y, w, h] = l.jump_bar;
            c.rect(x, y, w, h, [0.5; 3], 1.0);
            let fill = (h * t / 2400.0).floor();
            let col = if t < 640.0 {
                [1.0, 0.0, 0.0]
            } else if t < 960.0 {
                [0.0, 1.0, 0.0]
            } else if t < 1440.0 {
                [0.0, 1.0, 1.0]
            } else {
                [0.0, 0.0, 1.0]
            };
            c.rect(x, y + h - fill, w, fill, col, 1.0);
            c.wire(x, y, w, h, [0.0; 3]);
            // Readout box (the game prints S.T H in its debug font; drawn here with the small
            // font, black on white).
            c.rect(x - 20.0, y + h - 45.0, 34.0, 10.0, [1.0; 3], 1.0);
            c.wire(x - 20.0, y + h - 45.0, 34.0, 10.0, [0.0; 3]);
            let secs = (t / 960.0) as u32;
            let tenths = ((t - secs as f32 * 960.0) / 96.0) as u32;
            let txt = [b'0' + secs.min(9) as u8, b'.', b'0' + tenths.min(9) as u8];
            self.text(
                c,
                &self.small,
                &txt,
                x - 18.0,
                y + h - 44.0,
                &[[0.0; 3]; 4],
                false,
            );
        }

        // Start lights (DotLights_AudioAndVideo).
        if let Some(timer) = s.start_timer {
            self.draw_start_lights(c, timer);
        }
    }

    fn item_sprite(&self, name: &str) -> Option<usize> {
        self.items.iter().find(|i| i.0 == name).map(|i| i.1)
    }

    fn speedometer(&self, c: &mut Canvas, s: &HudState) {
        let l = &self.l;
        let (inner, outer, ticks) = (&l.arc_inner, &l.arc_outer, &l.tick_inner);
        let corner = [outer[6][0], outer[0][1]];
        for i in 0..6 {
            c.tri([outer[i], outer[i + 1], corner], [[0.0; 3]; 3], 0.5);
        }
        let sc = &l.spoke_colors;
        for i in 0..6 {
            c.tri(
                [inner[i], outer[i], inner[i + 1]],
                [sc[i], sc[i], sc[i + 1]],
                1.0,
            );
            c.tri(
                [outer[i], outer[i + 1], inner[i + 1]],
                [sc[i], sc[i + 1], sc[i + 1]],
                1.0,
            );
        }
        for (d, col) in [(1.0, [0.0; 3]), (0.0, [1.0; 3])] {
            let sh = |p: [f32; 2]| [p[0] + d, p[1] + d];
            for i in 0..7 {
                c.line(sh(inner[i]), sh(ticks[i]), col);
            }
            for i in 0..6 {
                c.line(sh(inner[i]), sh(inner[i + 1]), col);
                c.line(sh(outer[i]), sh(outer[i + 1]), col);
            }
        }
        // Needle angle.
        let accel = s.accel_speed.max(1e-3);
        let deg = if s.speed > accel {
            let t = ((s.speed - accel) / s.sacred_fire_speed.max(1e-3)).clamp(0.0, 1.0);
            213.75 + (157.5 - 213.75) * t
        } else {
            let t = (s.speed / accel).clamp(0.0, 1.0);
            305.2 + (213.75 - 305.2) * t
        };
        let a = deg.to_radians();
        let b = a + std::f32::consts::FRAC_PI_2;
        let pv = l.pivot;
        let tip = [pv[0] + 60.0 * a.sin(), pv[1] + 0.625 * 60.0 * a.cos()];
        let back = [pv[0] - 6.0 * a.sin(), pv[1] - 0.625 * 6.0 * a.cos()];
        let s1 = [pv[0] + 6.0 * b.sin(), pv[1] + 0.625 * 8.0 * b.cos()];
        let s2 = [pv[0] - 6.0 * b.sin(), pv[1] - 0.625 * 8.0 * b.cos()];
        let rgb = |r: f32, g: f32, bl: f32| [r / 255.0, g / 255.0, bl / 255.0];
        c.tri(
            [tip, back, s1],
            [
                rgb(91.0, 91.0, 0.0),
                rgb(50.0, 43.0, 1.0),
                rgb(255.0, 187.0, 0.0),
            ],
            1.0,
        );
        c.tri(
            [tip, back, s2],
            [
                rgb(255.0, 255.0, 255.0),
                rgb(156.0, 105.0, 0.0),
                rgb(255.0, 255.0, 0.0),
            ],
            1.0,
        );
    }

    fn mesh_spin(&self, c: &mut Canvas, m: &Mesh, at: [f32; 2], angle: f32) {
        if m.pos3.len() != m.pos2.len() {
            c.mesh(&m.pos2, &m.col, at);
            return;
        }
        let (sn, cs) = angle.sin_cos();
        // Painter's order, far to near (fixed-size scratch: no allocation).
        let n = (m.pos3.len() / 3).min(256);
        let mut order = [(0f32, 0u16); 256];
        for (t, o) in order.iter_mut().enumerate().take(n) {
            let z: f32 = (0..3)
                .map(|k| {
                    let p = m.pos3[t * 3 + k];
                    -p[0] * sn + p[2] * cs
                })
                .sum();
            *o = (z, t as u16);
        }
        order[..n].sort_unstable_by(|a, b| b.0.total_cmp(&a.0));
        for &(_, t) in &order[..n] {
            let t = t as usize;
            let pts = [0, 1, 2].map(|k| {
                let p = m.pos3[t * 3 + k];
                let x = p[0] * cs + p[2] * sn;
                [at[0] + x, at[1] - p[1] * 0.5625]
            });
            c.tri(pts, [m.col[t * 3], m.col[t * 3 + 1], m.col[t * 3 + 2]], 1.0);
        }
    }

    fn sprite(&self, c: &mut Canvas, index: usize, x: f32, y: f32, g: Option<&Grad>, scale: f32) {
        let Some(img) = self.images.get(index) else {
            return;
        };
        let w = (img.draw[0] * scale).round().max(1.0) as i32;
        let h = (img.draw[1] * scale).round().max(1.0) as i32;
        let (x0, y0) = (x as i32, y as i32);
        for yy in 0..h {
            let py = y0 + yy;
            if py < 0 || py >= HEIGHT as i32 {
                continue;
            }
            let v = (((yy as f32 + 0.5) / h as f32 * img.draw[1]) as usize).min(img.h - 1);
            let fy = yy as f32 / h as f32;
            for xx in 0..w {
                let px = x0 + xx;
                if px < 0 || px >= WIDTH as i32 {
                    continue;
                }
                let u = (((xx as f32 + 0.5) / w as f32 * img.draw[0]) as usize).min(img.w - 1);
                let o = (v * img.w + u) * 4;
                if img.rgba[o + 3] == 0 {
                    continue;
                }
                let t = [
                    img.rgba[o] as f32 / 255.0,
                    img.rgba[o + 1] as f32 / 255.0,
                    img.rgba[o + 2] as f32 / 255.0,
                ];
                let col = match g {
                    Some(g) => {
                        let fx = xx as f32 / w as f32;
                        std::array::from_fn(|k| {
                            let top = g[0][k] * (1.0 - fx) + g[1][k] * fx;
                            let bot = g[2][k] * (1.0 - fx) + g[3][k] * fx;
                            // PSX texture modulation: texel * colour / 128.
                            t[k] * (top * (1.0 - fy) + bot * fy) * (255.0 / 128.0)
                        })
                    }
                    None => t,
                };
                c.put(px, py, col, 1.0);
            }
        }
    }

    /// The start lights (DotLights_Video): four in a row centred, red off/on for the first
    /// three and green off/on for the fourth, `trafficLightIcon[state + 2 * (i == 3)]`.
    fn draw_start_lights(&self, c: &mut Canvas, timer: i32) {
        let (Some(ids), Some(l)) = (self.start_lights, start_lights(timer)) else {
            return;
        };
        let Some([x, y, w, _]) = self.start_lights_rect(l.pos) else {
            return;
        };
        for i in 0..4u8 {
            let on = usize::from(l.lit > i);
            let icon = ids[on + if i == 3 { 2 } else { 0 }];
            self.sprite(c, icon, (x + w * i32::from(i)) as f32, y as f32, None, 1.0);
        }
    }

    fn text(&self, c: &mut Canvas, font: &Font, s: &[u8], x: f32, y: f32, g: &Grad, right: bool) {
        let adv = |ch: u8| {
            if ch == b'.' || ch == b':' {
                font.advance_punct
            } else {
                font.advance
            }
        };
        let width: i32 = s.iter().map(|&ch| adv(ch)).sum();
        let mut pen = if right { x as i32 - width } else { x as i32 };
        for &ch in s {
            if let Some(Some(sp)) = font.glyphs.get(ch as usize) {
                self.sprite(c, *sp as usize, pen as f32, y, Some(g), 1.0);
            }
            pen += adv(ch);
        }
    }
}

fn num(mut n: u32, buf: &mut [u8; 3]) -> &[u8] {
    n = n.min(999);
    let mut i = 3;
    loop {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 || i == 0 {
            break;
        }
    }
    &buf[i..]
}

struct Canvas<'a> {
    px: &'a mut [u8],
}

impl Canvas<'_> {
    /// Straight-alpha "over" compositing onto the RGBA8 canvas.
    fn put(&mut self, x: i32, y: i32, c: [f32; 3], a: f32) {
        if x < 0 || y < 0 || x >= WIDTH as i32 || y >= HEIGHT as i32 {
            return;
        }
        let o = (y as usize * WIDTH + x as usize) * 4;
        let d = &mut self.px[o..o + 4];
        if a >= 1.0 {
            for k in 0..3 {
                d[k] = (c[k].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
            d[3] = 255;
            return;
        }
        let da = d[3] as f32 / 255.0;
        let oa = a + da * (1.0 - a);
        if oa <= 0.0 {
            return;
        }
        for k in 0..3 {
            let dc = d[k] as f32 / 255.0;
            let v = (c[k].clamp(0.0, 1.0) * a + dc * da * (1.0 - a)) / oa;
            d[k] = (v * 255.0 + 0.5) as u8;
        }
        d[3] = (oa * 255.0 + 0.5) as u8;
    }

    fn tri(&mut self, p: [[f32; 2]; 3], c: [[f32; 3]; 3], a: f32) {
        let area =
            (p[1][0] - p[0][0]) * (p[2][1] - p[0][1]) - (p[1][1] - p[0][1]) * (p[2][0] - p[0][0]);
        if area.abs() < 1e-6 {
            return;
        }
        let x0 = p
            .iter()
            .map(|q| q[0])
            .fold(f32::MAX, f32::min)
            .floor()
            .max(0.0) as i32;
        let x1 = p
            .iter()
            .map(|q| q[0])
            .fold(f32::MIN, f32::max)
            .ceil()
            .min(WIDTH as f32 - 1.0) as i32;
        let y0 = p
            .iter()
            .map(|q| q[1])
            .fold(f32::MAX, f32::min)
            .floor()
            .max(0.0) as i32;
        let y1 = p
            .iter()
            .map(|q| q[1])
            .fold(f32::MIN, f32::max)
            .ceil()
            .min(HEIGHT as f32 - 1.0) as i32;
        for y in y0..=y1 {
            let py = y as f32 + 0.5;
            for x in x0..=x1 {
                let px = x as f32 + 0.5;
                let w0 = ((p[1][0] - px) * (p[2][1] - py) - (p[1][1] - py) * (p[2][0] - px)) / area;
                let w1 = ((p[2][0] - px) * (p[0][1] - py) - (p[2][1] - py) * (p[0][0] - px)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < -1e-4 || w1 < -1e-4 || w2 < -1e-4 {
                    continue;
                }
                let col = std::array::from_fn(|k| c[0][k] * w0 + c[1][k] * w1 + c[2][k] * w2);
                self.put(x, y, col, a);
            }
        }
    }

    fn mesh(&mut self, pos: &[[f32; 2]], col: &[[f32; 3]], at: [f32; 2]) {
        for i in (0..pos.len().min(col.len())).step_by(3) {
            if i + 2 >= pos.len() {
                break;
            }
            let pts = [0, 1, 2].map(|k| [at[0] + pos[i + k][0], at[1] + pos[i + k][1]]);
            self.tri(pts, [col[i], col[i + 1], col[i + 2]], 1.0);
        }
    }

    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, c: [f32; 3], a: f32) {
        for yy in y as i32..(y + h) as i32 {
            for xx in x as i32..(x + w) as i32 {
                self.put(xx, yy, c, a);
            }
        }
    }

    fn line(&mut self, a: [f32; 2], b: [f32; 2], c: [f32; 3]) {
        let n = ((b[0] - a[0]).abs().max((b[1] - a[1]).abs()) as i32).max(1);
        for i in 0..=n {
            let t = i as f32 / n as f32;
            self.put(
                (a[0] + (b[0] - a[0]) * t) as i32,
                (a[1] + (b[1] - a[1]) * t) as i32,
                c,
                1.0,
            );
        }
    }

    fn wire(&mut self, x: f32, y: f32, w: f32, h: f32, c: [f32; 3]) {
        let (x1, y1) = (x + w, y + h);
        self.line([x, y], [x1, y], c);
        self.line([x1, y], [x1, y1], c);
        self.line([x1, y1], [x, y1], c);
        self.line([x, y1], [x, y], c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A minimal hud.json with one 2×2 white glyph for every character, two meshes and the
    /// real layout numbers.
    fn tiny_hud() -> Value {
        let glyphs: Vec<Value> = "0123456789abcdefghijklmnopqrstuvwxyz/x"
            .chars()
            .map(|c| json!({"ch": c.to_string(), "sprite": 0}))
            .collect();
        let tri = json!({"name": "", "positions": [[-10.0, -10.0], [10.0, -10.0], [0.0, 10.0]], "colors": [[1.0, 0.5, 0.0], [1.0, 0.5, 0.0], [1.0, 0.5, 0.0]], "positions3d": [[-10.0, 10.0, 0.0], [10.0, 10.0, 0.0], [0.0, -10.0, 0.0]]});
        let mut meshes = Vec::new();
        for i in 1..=8 {
            let mut m = tri.clone();
            m["name"] = json!(format!("big{i}"));
            meshes.push(m);
        }
        let mut fr = tri.clone();
        fr["name"] = json!("fruitdisp");
        meshes.push(fr);
        let g = json!([
            [1.0, 1.0, 1.0],
            [1.0, 1.0, 1.0],
            [1.0, 1.0, 1.0],
            [1.0, 1.0, 1.0]
        ]);
        json!({
            "textures": [{"width": 2, "height": 2, "rgba": [255,255,255,255, 255,255,255,255, 255,255,255,255, 255,255,255,255]}],
            "sprites": [{"texture": 0, "size": [2, 2]}],
            "fonts": [{"name": "big", "advance": 17, "advance_punct": 11, "glyphs": glyphs.clone()},
                      {"name": "small", "advance": 13, "advance_punct": 7, "glyphs": glyphs}],
            "colors": {"orange": g, "periwinkle": g, "red": g, "white": g},
            "meshes": meshes,
            "elements": {
                "portraits": {"crash": 0},
                "layout": {
                    "rank_list": {"row_height": 27, "number": {"x": 52, "y0": 56}, "portrait": {"x": 20, "y0": 57}},
                    "weapon": {"x": 232, "y": 5, "sprites": {"bomb": 0}},
                    "lap": {"label": {"x": 484, "y": 8}, "value": {"x": 484, "y": 16}},
                    "wumpa": {"fruit": {"x": 316, "y": 24}, "x_sign": {"x": 336, "y": 20}, "count": {"x": 349, "y": 16}},
                    "rank": {"number": {"x": 50, "y": 191}, "suffix": {"x": 70, "y": 171}},
                    "speedometer": {
                        "origin": [480, 190],
                        "arc_inner": [[-65,16],[-70,0],[-65,-16],[-50,-29],[-25,-39],[0,-42],[25,-39]],
                        "arc_outer": [[-54,13],[-58,0],[-54,-13],[-42,-24],[-21,-32],[0,-35],[21,-32]],
                        "tick_inner": [[-48,11],[-51,0],[-48,-11],[-37,-21],[-18,-29],[0,-31],[18,-29]],
                        "spoke_colors": [[0,0.7,0],[0,0.7,0],[0,0.7,0],[1,0.8,0],[0.9,0,0],[0.9,0,0],[0.9,0,0]],
                        "needle": {"pivot": [479, 186]}
                    },
                    "slide_meter": {"box": {"x": 441, "y": 199, "w": 49, "h": 7}},
                    "jump_meter": {"bar": {"x": 491, "y": 168, "w": 12, "h": 38}}
                }
            }
        })
    }

    fn full_state() -> HudState {
        HudState {
            speedometer: true,
            speed: 9000.0,
            accel_speed: 10000.0,
            sacred_fire_speed: 2000.0,
            drift_meter: Some((300, 1000, 400)),
            jump: Some(1200),
            wumpa: Some(7),
            juiced: false,
            lap: Some((2, 3)),
            place: Some(1),
            item: Some((1, 0)),
            ranks: vec!["crash".into()],
            time_seconds: 1.0,
            ..Default::default()
        }
    }

    fn alpha(px: &[u8], x: usize, y: usize) -> u8 {
        px[(y * WIDTH + x) * 4 + 3]
    }

    #[test]
    fn draws_speedometer_and_leaves_background_transparent() {
        let hud = HudRenderer::from_json(&tiny_hud()).unwrap();
        let mut px = vec![7u8; WIDTH * HEIGHT * 4];
        hud.render(&full_state(), &mut px);
        // Dial segment (between spokes) is opaque; the empty sky is transparent.
        assert_eq!(alpha(&px, 480 - 60, 190 - 1), 255);
        assert_eq!(alpha(&px, 150, 100), 0);
        assert_eq!(alpha(&px, 256, 120), 0);
        // Drift meter: red fill at the right end, gray at the left.
        let o = (202 * WIDTH + 488) * 4;
        assert_eq!(&px[o..o + 4], &[255, 0, 0, 255]);
        let o = (202 * WIDTH + 445) * 4;
        assert_eq!(&px[o..o + 4], &[128, 128, 128, 255]);
        // Nothing drawn for an empty state.
        hud.render(&HudState::default(), &mut px);
        assert!(px
            .chunks(4)
            .all(|p| p[3] == 0 || (p[0] == 128 || p[0] == 0)));
        let hidden = HudState {
            hide_drift_meter: true,
            ..Default::default()
        };
        hud.render(&hidden, &mut px);
        assert!(px.chunks(4).all(|p| p[3] == 0));
    }

    #[test]
    fn backdrop_keeps_half_alpha() {
        let hud = HudRenderer::from_json(&tiny_hud()).unwrap();
        let mut px = vec![0u8; WIDTH * HEIGHT * 4];
        let s = HudState {
            speedometer: true,
            accel_speed: 1.0,
            hide_drift_meter: true,
            ..Default::default()
        };
        hud.render(&s, &mut px);
        // Inside the dial, away from the needle: 50% black backdrop.
        let a = alpha(&px, 470, 178);
        assert!((120..=135).contains(&a), "alpha {a}");
    }

    #[test]
    fn regions_fit_the_three_crops() {
        let hud = HudRenderer::from_json(&lights_hud()).unwrap();
        let regions = hud.regions();
        assert!(regions.iter().any(|r| r.name == "start_lights"));
        for r in regions {
            let (x0, x1) = (r.rect[0], r.rect[0] + r.rect[2]);
            let ok = [CROP_LEFT, CROP_MIDDLE, CROP_RIGHT]
                .iter()
                .any(|c| x0 >= c[0] && x1 <= c[1]);
            assert!(ok, "{} {:?} straddles crops", r.name, r.rect);
        }
    }

    /// [`tiny_hud`] with the four start lights, 47×31 as on the disc, each one flat colour:
    /// red off dark red, red on red, green off dark green, green on green.
    fn lights_hud() -> Value {
        let mut j = tiny_hud();
        let colours = [[64, 0, 0], [255, 0, 0], [0, 64, 0], [0, 255, 0]];
        for (i, c) in colours.iter().enumerate() {
            let px: Vec<u8> = (0..4).flat_map(|_| [c[0], c[1], c[2], 255]).collect();
            j["textures"]
                .as_array_mut()
                .unwrap()
                .push(json!({"width": 2, "height": 2, "rgba": px}));
            j["sprites"]
                .as_array_mut()
                .unwrap()
                .push(json!({"texture": i + 1, "size": [47, 31]}));
        }
        j["elements"]["layout"]["start_lights"] = json!({"sprites": [1, 2, 3, 4]});
        j
    }

    #[test]
    fn the_start_lights_light_in_ctr_s_order_and_time_to_go() {
        let at = |s: f32| start_lights(lights_timer(s)).map(|l| l.lit);
        // A long countdown shows nothing until CTR's own 4 s of lights.
        assert_eq!(at(25.0), None);
        assert_eq!(at(4.01), None);
        assert_eq!(at(4.0), Some(0));
        // Red one by one at 3, 2 and 1 s before GO; green exactly at GO.
        assert_eq!(at(3.001), Some(0));
        assert_eq!(at(3.0), Some(1));
        assert_eq!(at(2.001), Some(1));
        assert_eq!(at(2.0), Some(2));
        assert_eq!(at(1.001), Some(2));
        assert_eq!(at(1.0), Some(3));
        assert_eq!(at(0.0005), Some(3));
        assert_eq!(at(0.0), Some(4));
        // Green until they have slid away, 0x3BF units after GO.
        assert_eq!(at(-0.99), Some(4));
        assert_eq!(start_lights(LIGHTS_TIMER_END).map(|l| l.lit), Some(4));
        assert_eq!(start_lights(LIGHTS_TIMER_END - 1), None);
        assert_eq!(lights_timer(4.0), LIGHTS_TIMER_START);
        assert_eq!(lights_timer(0.0), LIGHTS_TIMER_GREEN);
    }

    #[test]
    fn the_start_lights_slide_in_and_out_with_ctr_s_tween() {
        // DotLights_TweenPos is value / 960 truncated toward zero.
        for v in (-3_932_160..=3_932_160).step_by(4093) {
            assert_eq!(lights_tween(v), v / 960, "{v}");
        }
        let pos = |t: i32| start_lights(t).unwrap().pos;
        // In over the first second, at rest through the reds, out over the second after GO.
        assert_eq!(pos(0xF00), 0);
        assert_eq!(pos(0xF00 - 480), 2048);
        assert_eq!(pos(0xB41), 4091);
        assert_eq!(pos(0xB40), 4096);
        assert_eq!(pos(1), 4096);
        assert_eq!(pos(0), 4096);
        assert_eq!(pos(-480), 2048);
        assert_eq!(pos(-0x3BF), 4);

        // 47×31 lights: centred, x = (512 - 188) / 2; the bottom edge at a third of the height
        // in place, and wholly above the screen at the start.
        let hud = HudRenderer::from_json(&lights_hud()).unwrap();
        assert!(hud.has_start_lights());
        assert_eq!(hud.start_lights_rect(4096), Some([162, 41, 47, 31]));
        assert_eq!(hud.start_lights_rect(2048), Some([162, 5, 47, 31]));
        assert_eq!(hud.start_lights_rect(0), Some([162, -31, 47, 31]));
        assert!(!HudRenderer::from_json(&tiny_hud())
            .unwrap()
            .has_start_lights());
    }

    #[test]
    fn each_start_light_sound_plays_once_as_its_light_comes_on() {
        // Every frame at 60 Hz from a 30 s countdown to 2 s after GO.
        let mut prev = lights_timer(30.0);
        let mut heard = Vec::new();
        let mut t = 30.0f32;
        while t > -2.0 {
            t -= 1.0 / 60.0;
            let now = lights_timer(t);
            if let Some(s) = start_light_sound(prev, now) {
                heard.push((s, now));
            }
            prev = now;
        }
        let sounds: Vec<StartSound> = heard.iter().map(|h| h.0).collect();
        assert_eq!(
            sounds,
            [
                StartSound::Red,
                StartSound::Red,
                StartSound::Red,
                StartSound::Green
            ]
        );
        // Each in the frame its light comes on.
        for ((_, timer), threshold) in heard.iter().zip([0xB40, 0x780, 0x3C0, 0]) {
            assert!(
                *timer <= threshold && *timer > threshold - 16,
                "{timer} {threshold}"
            );
        }
        // A frame that skips a light plays the one it lands in, once.
        assert_eq!(start_light_sound(3000, 1000), Some(StartSound::Red));
        assert_eq!(start_light_sound(1000, 1000), None);
        assert_eq!(start_light_sound(5000, 3000), None);
        assert_eq!(start_light_sound(10, -100), Some(StartSound::Green));
        assert_eq!(start_light_sound(10, -2000), None);
    }

    #[test]
    fn the_start_lights_draw_lit_as_the_timer_says() {
        let hud = HudRenderer::from_json(&lights_hud()).unwrap();
        let mut px = vec![0u8; WIDTH * HEIGHT * 4];
        let rgb = |px: &[u8], x: usize, y: usize| {
            let o = (y * WIDTH + x) * 4;
            [px[o], px[o + 1], px[o + 2], px[o + 3]]
        };
        let centres = [0, 1, 2, 3].map(|i| 162 + 47 * i + 23);
        let draw = |px: &mut Vec<u8>, timer| {
            let s = HudState {
                hide_drift_meter: true,
                start_timer: Some(timer),
                ..Default::default()
            };
            hud.render(&s, px);
        };
        // Two reds on, in place.
        draw(&mut px, 0x780);
        let row = centres.map(|x| rgb(&px, x, 56));
        assert_eq!(
            row,
            [
                [255, 0, 0, 255],
                [255, 0, 0, 255],
                [64, 0, 0, 255],
                [0, 64, 0, 255]
            ]
        );
        assert_eq!(rgb(&px, 161, 56)[3], 0);
        assert_eq!(rgb(&px, 162 + 188, 56)[3], 0);
        assert_eq!(rgb(&px, 200, 41)[3], 255);
        assert_eq!(rgb(&px, 200, 40)[3], 0);
        assert_eq!(rgb(&px, 200, 72)[3], 0);
        // All on at GO.
        draw(&mut px, 0);
        assert_eq!(rgb(&px, centres[3], 56), [0, 255, 0, 255]);
        // Before CTR's countdown and after the slide out: nothing.
        for t in [0xF01, LIGHTS_TIMER_END - 1] {
            draw(&mut px, t);
            assert!(px.chunks(4).all(|p| p[3] == 0), "{t}");
        }
        // A hud.json without the lights draws none.
        let old = HudRenderer::from_json(&tiny_hud()).unwrap();
        old.render(
            &HudState {
                hide_drift_meter: true,
                start_timer: Some(0),
                ..Default::default()
            },
            &mut px,
        );
        assert!(px.chunks(4).all(|p| p[3] == 0));
    }

    #[test]
    fn number_formatting() {
        let mut b = [0u8; 3];
        assert_eq!(num(7, &mut b), b"7");
        assert_eq!(num(42, &mut b), b"42");
        assert_eq!(num(0, &mut b), b"0");
    }
}
