//! CTR's race HUD over the game while we drive, drawn by `ctr_assets::hud_draw` from the player's
//! `kart-data/hud.json`: the speedometer with its needle, the powerslide's turbo meter (green,
//! red in the boost window), the hang-time meter while a hop is in the air, the Wumpa count, and
//! in a race the lap, the place, the held item (its roulette while it spins), the start lights
//! and the race clock with its lap times. The 512×216 canvas (CTR's framebuffer, 4:3 pixels) is drawn into one
//! texture each frame and shown as three crops: the left elements pinned to the left edge, the
//! middle to the centre and the right to the right edge, at the window's height. CTR's
//! centre-screen texts ("FINAL LAP" flying by, a flashing "WRONG WAY") centre on the whole view,
//! so they go on a second canvas as wide as the window in CTR's pixels, drawn only while one
//! shows.

use std::sync::OnceLock;

pub use ctr_assets::hud_draw::{
    lights_timer, start_light_sound, StartSound, LIGHTS_TIMER_END, LIGHTS_TIMER_START,
};

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::window::PrimaryWindow;
use ctr_assets::hud_draw::{
    Banners, HudRenderer, HudState, CROP_LEFT, CROP_MIDDLE, CROP_RIGHT, HEIGHT, WIDTH,
};

use crate::session::KartView;

/// The race clock the host hands the HUD.
pub use ctr_assets::hud_draw::RaceClock;

/// How long the hang-time meter stays up after landing (s), as the game keeps it.
const JUMP_HOLD: f32 = 1.5;
/// CTR's pixels are this wide for each unit of height on its 4:3 screen.
const PIXEL_ASPECT: f32 = 0.5625;

fn renderer() -> Option<&'static HudRenderer> {
    static HUD: OnceLock<Option<HudRenderer>> = OnceLock::new();
    HUD.get_or_init(|| {
        let path = crate::model::data_root()?.join("hud.json");
        HudRenderer::load(&path)
            .map_err(|e| warn!("kart HUD: {}: {e}", path.display()))
            .ok()
    })
    .as_ref()
}

/// Whether the HUD draws CTR's start lights: the player's `hud.json` has them (one converted
/// before them has not, and the race shows its countdown as text instead).
pub fn has_start_lights() -> bool {
    renderer().is_some_and(HudRenderer::has_start_lights)
}

/// What the HUD shows beyond the kart itself: race standing, when there is a race.
#[derive(Resource, Default, Clone, Debug)]
pub struct HudExtras {
    pub lap: Option<(u8, u8)>,
    pub place: Option<u8>,
    pub ranks: Vec<String>,
    /// Wumpa fruit held (10 is juiced).
    pub wumpa: u8,
    /// The item slot: `(heldItemID, count)` in CTR's ids, 0x10 the roulette; a count above 0
    /// draws the digit (`UI/UI_Weapon.c`).
    pub item: Option<(u8, u8)>,
    /// CTR's traffic-light timer ([`lights_timer`]) while a race counts down; `None` hides the
    /// start lights.
    pub start_timer: Option<i32>,
    /// The race clock and its lap times (`UI_DrawRaceClock`).
    pub clock: Option<RaceClock>,
    /// Frames into "FINAL LAP"'s fly-by (30 fps), while it runs.
    pub final_lap_frame: Option<u32>,
    /// "WRONG WAY" is lit this frame (the race flashes it).
    pub wrong_way: bool,
}

/// The centre-screen texts' canvas: as wide as the window in CTR's pixels, over the whole view.
#[derive(Component)]
pub struct HudBanner;

#[derive(Component)]
pub struct HudCrop {
    span: [i32; 2],
    anchor: Anchor,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Anchor {
    Left,
    Middle,
    Right,
}

#[derive(Resource, Default)]
pub struct KartHud {
    image: Option<Handle<Image>>,
    pixels: Vec<u8>,
    /// The last hang-time reading and when the hop landed.
    jump: Option<(i32, Option<f32>)>,
    /// The banners' canvas, its width in CTR's pixels, and its pixels.
    banner: Option<(Handle<Image>, usize)>,
    banner_pixels: Vec<u8>,
}

fn canvas_image(width: usize) -> Image {
    let mut image = Image::new(
        Extent3d {
            width: width as u32,
            height: HEIGHT as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![0; width * HEIGHT * 4],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.sampler = ImageSampler::nearest();
    image
}

/// How many of CTR's pixels wide a window is, its height being the canvas's 216: CTR's pixels
/// are 0.5625 as wide as tall, so a 4:3 window is 512 and a 16:9 one 683.
fn screen_width(window_width: f32, height: f32) -> usize {
    let scale = height.max(1.0) / HEIGHT as f32;
    ((window_width / (PIXEL_ASPECT * scale)).round() as usize).clamp(64, 4096)
}

/// Spawns the HUD's image and its three crops, hidden until we drive.
pub fn spawn(mut commands: Commands, mut images: ResMut<Assets<Image>>, mut hud: ResMut<KartHud>) {
    if renderer().is_none() {
        return;
    }
    let handle = images.add(canvas_image(WIDTH));
    for (span, anchor) in [
        (CROP_LEFT, Anchor::Left),
        (CROP_MIDDLE, Anchor::Middle),
        (CROP_RIGHT, Anchor::Right),
    ] {
        commands.spawn((
            HudCrop { span, anchor },
            ImageNode {
                image: handle.clone(),
                rect: Some(Rect::new(
                    span[0] as f32,
                    0.0,
                    span[1] as f32,
                    HEIGHT as f32,
                )),
                ..default()
            },
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(0.0),
                ..default()
            },
            Pickable::IGNORE,
            GlobalZIndex(9),
            Visibility::Hidden,
        ));
    }
    let banner = images.add(canvas_image(WIDTH));
    commands.spawn((
        HudBanner,
        ImageNode::new(banner.clone()),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(0.0),
            left: Val::Px(0.0),
            ..default()
        },
        Pickable::IGNORE,
        GlobalZIndex(9),
        Visibility::Hidden,
    ));
    hud.pixels = vec![0; WIDTH * HEIGHT * 4];
    hud.image = Some(handle);
    hud.banner = Some((banner, WIDTH));
    hud.banner_pixels = vec![0; WIDTH * HEIGHT * 4];
}

/// Draws this frame's HUD and lays the crops out for the window.
pub fn draw(
    time: Res<Time>,
    view: Res<KartView>,
    extras: Option<Res<HudExtras>>,
    mut hud: ResMut<KartHud>,
    mut images: ResMut<Assets<Image>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut crops: Query<(&HudCrop, &mut Node, &mut Visibility), Without<HudBanner>>,
    mut banner: Query<(&mut Node, &mut Visibility), With<HudBanner>>,
) {
    let (Some(renderer), Some(handle)) = (renderer(), hud.image.clone()) else {
        return;
    };
    let shown = view.active;
    for (_, _, mut vis) in &mut crops {
        *vis = if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
    if !shown {
        hud.jump = None;
        for (_, mut vis) in &mut banner {
            *vis = Visibility::Hidden;
        }
        return;
    }
    let now = time.elapsed_secs();
    // The hang-time meter: while a hop counts, then held a moment after landing.
    hud.jump = match (view.jump, hud.jump) {
        (Some(ms), _) => Some((ms, None)),
        (None, Some((ms, None))) => Some((ms, Some(now))),
        (None, Some((ms, Some(at)))) if now - at < JUMP_HOLD => Some((ms, Some(at))),
        _ => None,
    };
    let extras = extras.map(|e| e.clone()).unwrap_or_default();
    let state = HudState {
        speedometer: true,
        speed: view.ctr_speed as f32,
        accel_speed: view.class_speed as f32,
        sacred_fire_speed: 4096.0,
        drift_meter: view
            .meter
            .map(|(filled, _)| (((1.0 - filled) * 960.0).round() as u32, 960, 480)),
        jump: hud.jump.map(|(ms, _)| ms.max(0) as u32),
        wumpa: Some(extras.wumpa),
        juiced: extras.wumpa >= 10,
        lap: extras.lap,
        place: extras.place,
        item: extras.item,
        ranks: extras.ranks,
        time_seconds: now,
        start_timer: extras.start_timer,
        clock: extras.clock,
        ..Default::default()
    };
    let banners = Banners {
        final_lap_frame: extras.final_lap_frame,
        wrong_way: extras.wrong_way,
    };
    let hud = &mut *hud;
    renderer.render(&state, &mut hud.pixels);
    if let Some(image) = images.get_mut(&handle) {
        if let Some(data) = image.data.as_mut() {
            data.copy_from_slice(&hud.pixels);
        }
    }

    // The canvas fills the window's height; each crop keeps CTR's pixel aspect.
    // Above whatever the host keeps at the bottom of the window (WoW's action bars).
    let height =
        windows.single().map_or(720.0, |w| w.height()) * (1.0 - crate::framing().hud_bottom_inset);
    let scale = height / HEIGHT as f32;
    let width_of = |span: [i32; 2]| (span[1] - span[0]) as f32 * PIXEL_ASPECT * scale;
    let window_width = windows.single().map_or(1280.0, |w| w.width());
    // The banners: drawn only while one shows, on a canvas the window's width.
    let showing = banners.final_lap_frame.is_some_and(|f| f < 90) || banners.wrong_way;
    for (mut node, mut vis) in &mut banner {
        let Some((handle, w)) = hud.banner.as_mut().filter(|_| showing) else {
            *vis = Visibility::Hidden;
            continue;
        };
        let Some(image) = images.get_mut(&*handle) else {
            continue;
        };
        // The window's width changed: the same image, remade that wide.
        let width = screen_width(window_width, height);
        if *w != width {
            *image = canvas_image(width);
            *w = width;
            hud.banner_pixels = vec![0; width * HEIGHT * 4];
        }
        renderer.render_banners(&banners, width, &mut hud.banner_pixels);
        if let Some(data) = image.data.as_mut() {
            data.copy_from_slice(&hud.banner_pixels);
        }
        node.width = Val::Px(window_width);
        node.height = Val::Px(height);
        *vis = Visibility::Inherited;
    }
    for (crop, mut node, _) in &mut crops {
        let w = width_of(crop.span);
        node.width = Val::Px(w);
        node.height = Val::Px(height);
        node.left = Val::Px(match crop.anchor {
            Anchor::Left => 6.0 * PIXEL_ASPECT * scale,
            // A few of CTR's pixels in from the edge, as on a TV's safe area.
            Anchor::Right => window_width - w - 6.0 * PIXEL_ASPECT * scale,
            // The middle crop sits where it would on CTR's 4:3 screen centred in ours.
            Anchor::Middle => {
                let canvas = WIDTH as f32 * PIXEL_ASPECT * scale;
                (window_width - canvas) / 2.0 + crop.span[0] as f32 * PIXEL_ASPECT * scale
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_banner_canvas_is_the_window_s_width_in_ctr_s_pixels() {
        // A 4:3 window is CTR's own 512; 16:9 is wider by the same pixel aspect.
        assert_eq!(screen_width(960.0, 720.0), 512);
        assert_eq!(screen_width(1280.0, 720.0), 683);
        assert_eq!(screen_width(2560.0, 1080.0), 910);
    }
}
