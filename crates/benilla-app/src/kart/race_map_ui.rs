//! The race map, bevy_ui over the world while a race is on: the course fitted north up (WoW x
//! up the screen, y right to left) and drawn once per race as a road (an outline, the sand, a
//! centre line, a checkered start and, a sprint's, a red finish), the racers on it as round
//! dots with their places, us on top in gold. A racer too far off to be relayed is at their last
//! checkpoint, dimmed. A left drag moves the card; where it was let go is kept (`KartMapX`,
//! `KartMapY`, its centre as fractions of the window). It steps aside with the HUD.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use super::race::RaceMap;
use super::ui::{KartOverlay, RacerDot};
use crate::glue::art::GOLD;
use crate::ui_script::EnginePanel;

/// Under the editor's panels and the menu, over the world's FrameXML.
const Z: i32 = 20;
/// The card's width, its padding and title row, and the map's side (UI px at 768 tall).
const CARD_W: f32 = 212.0;
const PAD: f32 = 10.0;
const TITLE_H: f32 = 18.0;
const SIDE: f32 = CARD_W - 2.0 * PAD;
const CARD_H: f32 = SIDE + 2.0 * PAD + TITLE_H;
/// The track's image, drawn this many pixels to a UI pixel for its edges.
const SUPERSAMPLE: f32 = 2.0;
/// How much of the map the course fills.
const FILL: f32 = 0.84;
/// The road's widths (UI px): its glow, outline, sand and centre line.
const GLOW: f32 = 7.0;
const OUTLINE: f32 = 4.5;
const ROAD: f32 = 3.0;
const LINE: f32 = 0.6;
const SAND: [f32; 3] = [0.89, 0.78, 0.56];
const EDGE: [f32; 3] = [0.17, 0.11, 0.06];
const CENTRE: [f32; 3] = [1.0, 0.95, 0.82];
/// Our dot and the others', their sides (UI px).
const ME: f32 = 12.0;
const RACER: f32 = 9.0;
const RACER_COLOR: Color = Color::srgb(0.95, 0.3, 0.25);
/// A racer only at their last checkpoint.
const FAR_ALPHA: f32 = 0.45;

/// The course fitted in the map: its centre (WoW x, y) and UI px a yard.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Fit {
    cx: f32,
    cy: f32,
    scale: f32,
}

impl Fit {
    fn of(points: &[[f32; 2]]) -> Fit {
        let (lo, hi) = points.iter().fold(
            (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)),
            |(lo, hi), p| (lo.min(Vec2::from(*p)), hi.max(Vec2::from(*p))),
        );
        let size = (hi - lo).max_element().max(1.0);
        Fit {
            cx: (lo.x + hi.x) / 2.0,
            cy: (lo.y + hi.y) / 2.0,
            scale: SIDE * FILL / size,
        }
    }

    /// Where a WoW x, y stands on the map, UI px from its top left: north up.
    fn at(&self, p: [f32; 2]) -> Vec2 {
        let half = SIDE / 2.0;
        Vec2::new(
            half + (self.cy - p[1]) * self.scale,
            half + (self.cx - p[0]) * self.scale,
        )
    }
}

/// The road's legs on the map (UI px): round to the start on a circuit, to the finish on a
/// sprint.
fn legs(track: &RaceMap, fit: &Fit) -> Vec<(Vec2, Vec2)> {
    let pts: Vec<Vec2> = track.points.iter().map(|p| fit.at(*p)).collect();
    let n = pts.len();
    let count = if track.laps > 0 { n } else { n - 1 };
    (0..count).map(|i| (pts[i], pts[(i + 1) % n])).collect()
}

fn distance(p: Vec2, (a, b): (Vec2, Vec2)) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

/// The track's image, RGBA, `SIDE · SUPERSAMPLE` square: each layer laid over the last with
/// its edge smoothed over a pixel.
fn draw_track(track: &RaceMap, fit: &Fit) -> (u32, Vec<u8>) {
    let n = (SIDE * SUPERSAMPLE) as u32;
    let legs = legs(track, fit);
    let start = fit.at(track.points[0]);
    let finish = (track.laps == 0).then(|| fit.at(*track.points.last().unwrap()));
    let mut rgba = vec![0u8; (n * n * 4) as usize];
    for py in 0..n {
        for px in 0..n {
            let p = Vec2::new(px as f32 + 0.5, py as f32 + 0.5) / SUPERSAMPLE;
            let d = legs
                .iter()
                .map(|l| distance(p, *l))
                .fold(f32::MAX, f32::min);
            // Coverage of a band of half-width `r`, smoothed over one image pixel.
            let cover = |r: f32| ((r - d) * SUPERSAMPLE + 0.5).clamp(0.0, 1.0);
            let mut c = [0.0f32; 3];
            let mut a = 0.0f32;
            let mut over = |rgb: [f32; 3], alpha: f32| {
                for k in 0..3 {
                    c[k] = c[k] * (1.0 - alpha) + rgb[k] * alpha;
                }
                a = a + alpha * (1.0 - a);
            };
            let glow = (1.0 - d / GLOW).clamp(0.0, 1.0);
            over([0.0; 3], glow * glow * 0.45);
            over(EDGE, cover(OUTLINE));
            over(SAND, cover(ROAD));
            over(CENTRE, cover(LINE) * 0.8);
            // The start a checkered square, a sprint's finish a red disc.
            let s = p - start;
            if s.x.abs() < 4.0 && s.y.abs() < 4.0 {
                let white = ((s.x + 4.0) / 2.0) as i32 % 2 == ((s.y + 4.0) / 2.0) as i32 % 2;
                over(if white { [1.0; 3] } else { [0.05; 3] }, 1.0);
            }
            if let Some(f) = finish {
                over(
                    [0.9, 0.2, 0.2],
                    ((3.5 - p.distance(f)) * SUPERSAMPLE).clamp(0.0, 1.0),
                );
            }
            let at = ((py * n + px) * 4) as usize;
            // Straight alpha: the colour as if alone, its coverage apart.
            let un = |v: f32| (v / a.max(1e-6) * 255.0).clamp(0.0, 255.0) as u8;
            rgba[at..at + 4].copy_from_slice(&[un(c[0]), un(c[1]), un(c[2]), (a * 255.0) as u8]);
        }
    }
    (n, rgba)
}

/// The card, the racers' dots and their places.
#[derive(Component)]
pub(crate) struct MapCard;
#[derive(Component)]
pub(crate) struct MapDot(usize);
#[derive(Component)]
pub(crate) struct MapPlace(usize);

/// What the card was built for: the track, each racer's place and whether they are near, and
/// the screen's scale.
#[derive(Clone, PartialEq)]
struct Built {
    track: RaceMap,
    places: Vec<(u8, bool)>,
    s: f32,
}

#[derive(Resource)]
pub(crate) struct RaceMapUi {
    built: Option<Built>,
    root: Option<Entity>,
    image: Option<Handle<Image>>,
    /// The cursor as the drag last moved the card.
    drag_at: Option<Vec2>,
    /// The card's centre, a fraction of the window, as kept (`KartMapX`, `KartMapY`).
    kept: Vec2,
}

impl Default for RaceMapUi {
    fn default() -> Self {
        RaceMapUi {
            built: None,
            root: None,
            image: None,
            drag_at: None,
            kept: Vec2::new(0.87, 0.42),
        }
    }
}

/// `KartMapX` and `KartMapY`: where the card stands.
pub(crate) fn on_cvar(ev: On<crate::cvars::CvarChanged>, mut ui: ResMut<RaceMapUi>) {
    match ev.key().as_str() {
        "kartmapx" => ui.kept.x = ev.num().clamp(0.0, 1.0),
        "kartmapy" => ui.kept.y = ev.num().clamp(0.0, 1.0),
        _ => {}
    }
}

/// The race map as the race stands: built again when its track, its racers' count or the
/// screen's scale changes, the dots moved every frame, the card dragged and its place kept.
#[allow(clippy::type_complexity)]
pub(crate) fn draw(
    mut commands: Commands,
    mut ui: ResMut<RaceMapUi>,
    (overlay, hidden): (Res<KartOverlay>, Res<kartcraft::hud::HudHidden>),
    (assets, mut images): (Res<AssetServer>, ResMut<Assets<Image>>),
    window: Query<&Window, With<PrimaryWindow>>,
    mut card: Query<(&Interaction, &mut Node), With<MapCard>>,
    mut dots: Query<
        (
            &MapDot,
            &mut Node,
            &mut BackgroundColor,
            &mut BorderColor,
            &mut ZIndex,
        ),
        Without<MapCard>,
    >,
    mut places: Query<(&MapPlace, &mut Node), (Without<MapCard>, Without<MapDot>)>,
) {
    let win = window.single().ok();
    let s = crate::glue::screen_scale(win);
    let track = overlay.track.as_ref().filter(|t| t.points.len() >= 2);
    let want = track.filter(|_| !hidden.0).map(|t| Built {
        track: t.clone(),
        places: overlay.racers.iter().map(|r| (r.place, r.near)).collect(),
        s,
    });
    if ui.built != want {
        if let Some(root) = ui.root.take() {
            commands.entity(root).despawn();
        }
        let redraw = ui.built.as_ref().map(|b| &b.track) != want.as_ref().map(|b| &b.track);
        if redraw {
            ui.image = want.as_ref().map(|b| {
                let (n, rgba) = draw_track(&b.track, &Fit::of(&b.track.points));
                images.add(benilla_assets::sprite_image(n, n, rgba))
            });
        }
        if let (Some(b), Some(image)) = (&want, &ui.image) {
            let size = win.map_or(Vec2::new(1024.0, 768.0), |w| w.size());
            let centre = ui.kept * size;
            let font = crate::char_select::wow_font(&assets);
            ui.root = Some(spawn(&mut commands, b, image.clone(), &font, centre));
        }
        ui.built = want;
    }
    let Some(built) = &ui.built else {
        ui.drag_at = None;
        return;
    };
    let fit = Fit::of(&built.track.points);
    for (dot, mut node, mut fill, mut edge, mut z) in &mut dots {
        let Some(r) = overlay.racers.get(dot.0) else {
            continue;
        };
        // Ours the bigger, drawn over the others.
        let side = if r.me { ME } else { RACER };
        let at = (fit.at(r.at) - side / 2.0) * s;
        (node.left, node.top) = (Val::Px(at.x), Val::Px(at.y));
        (node.width, node.height) = (Val::Px(side * s), Val::Px(side * s));
        z.set_if_neq(ZIndex(i32::from(r.me)));
        let (c, e) = looks(r);
        fill.set_if_neq(BackgroundColor(c));
        if edge.top != e {
            *edge = BorderColor::all(e);
        }
    }
    for (place, mut node) in &mut places {
        let Some(r) = overlay.racers.get(place.0) else {
            continue;
        };
        let at = (fit.at(r.at) + Vec2::new(7.0, -7.0)) * s;
        (node.left, node.top) = (Val::Px(at.x), Val::Px(at.y));
    }
    // A press held on the card drags it; let go, its centre is kept.
    let cursor = win.and_then(Window::cursor_position);
    let Ok((interaction, mut node)) = card.single_mut() else {
        return;
    };
    if *interaction == Interaction::Pressed {
        if let (Some(at), Some(was), Val::Px(x), Val::Px(y)) =
            (cursor, ui.drag_at, node.left, node.top)
        {
            let d = at - was;
            (node.left, node.top) = (Val::Px(x + d.x), Val::Px(y + d.y));
        }
        ui.drag_at = cursor;
    } else if ui.drag_at.take().is_some() {
        if let (Some(w), Val::Px(x), Val::Px(y)) = (win, node.left, node.top) {
            let centre = (Vec2::new(x, y) + Vec2::new(CARD_W, CARD_H) * s / 2.0) / w.size();
            let centre = centre.clamp(Vec2::ZERO, Vec2::ONE);
            ui.kept = centre;
            // Kept as the settings are, at the next apply: no system here writes the CVars.
            commands.queue(move |world: &mut World| {
                let mut cvars = world.resource_mut::<crate::cvars::Cvars>();
                cvars.set("KartMapX", &format!("{:.3}", centre.x));
                cvars.set("KartMapY", &format!("{:.3}", centre.y));
            });
        }
    }
}

/// A racer's dot: us gold, ringed dark; the others red, ringed white; dimmed when far.
fn looks(r: &RacerDot) -> (Color, Color) {
    let alpha = if r.near || r.me { 1.0 } else { FAR_ALPHA };
    match r.me {
        true => (GOLD, Color::srgb(0.15, 0.1, 0.0)),
        false => (
            RACER_COLOR.with_alpha(alpha),
            Color::WHITE.with_alpha(alpha),
        ),
    }
}

/// The card at `centre` (window px), kept on the screen: the course's name and laps over the
/// map, a dot and a place for each racer.
fn spawn(
    commands: &mut Commands,
    built: &Built,
    image: Handle<Image>,
    font: &Handle<Font>,
    centre: Vec2,
) -> Entity {
    let s = built.s;
    let px = |v: f32| Val::Px(v * s);
    let top_left = centre - Vec2::new(CARD_W, CARD_H) * s / 2.0;
    let card = Node {
        position_type: PositionType::Absolute,
        left: Val::Px(top_left.x.max(0.0)),
        top: Val::Px(top_left.y.max(0.0)),
        width: px(CARD_W),
        height: px(CARD_H),
        padding: UiRect::all(px(PAD)),
        flex_direction: FlexDirection::Column,
        border: UiRect::all(px(1.0)),
        border_radius: BorderRadius::all(px(10.0)),
        ..default()
    };
    let looks = (
        BackgroundColor(Color::srgba(0.04, 0.05, 0.08, 0.8)),
        BorderColor::all(GOLD.with_alpha(0.45)),
    );
    let root = commands
        .spawn((
            MapCard,
            EnginePanel,
            Interaction::default(),
            card,
            looks,
            GlobalZIndex(Z),
        ))
        .id();
    let face = |size: f32| TextFont {
        font: font.clone(),
        font_size: size * s,
        ..default()
    };
    let laps = match built.track.laps {
        0 => "sprint".to_string(),
        1 => "1 lap".to_string(),
        n => format!("{n} laps"),
    };
    commands.entity(root).with_children(|c| {
        c.spawn((
            Text::new(format!("{}  ", built.track.name)),
            face(11.0),
            TextColor(GOLD),
            TextShadow::default(),
            TextLayout::new_with_no_wrap(),
            Node {
                height: px(TITLE_H),
                overflow: Overflow::clip(),
                ..default()
            },
        ))
        .with_child((
            TextSpan::new(laps),
            face(9.0),
            TextColor(Color::srgb(0.7, 0.7, 0.7)),
        ));
        let canvas = Node {
            width: px(SIDE),
            height: px(SIDE),
            ..default()
        };
        c.spawn((ImageNode::new(image), canvas)).with_children(|m| {
            for (i, &(place, near)) in built.places.iter().enumerate() {
                let text = if place > 0 {
                    place.to_string()
                } else {
                    String::new()
                };
                let alpha = if near { 1.0 } else { FAR_ALPHA };
                m.spawn((
                    MapPlace(i),
                    Text::new(text),
                    face(9.0),
                    TextShadow::default(),
                    TextColor(Color::WHITE.with_alpha(alpha)),
                    Node {
                        position_type: PositionType::Absolute,
                        ..default()
                    },
                ));
            }
            for i in 0..built.places.len() {
                m.spawn((
                    MapDot(i),
                    ZIndex(0),
                    Node {
                        position_type: PositionType::Absolute,
                        width: px(RACER),
                        height: px(RACER),
                        border: UiRect::all(px(1.5)),
                        border_radius: BorderRadius::MAX,
                        ..default()
                    },
                    BackgroundColor(RACER_COLOR),
                    BorderColor::all(Color::WHITE),
                ));
            }
        });
    });
    root
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(laps: u8) -> RaceMap {
        RaceMap {
            name: "Elwynn Loop".into(),
            laps,
            points: vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]],
            order: Vec::new(),
            checkpoints: Vec::new(),
        }
    }

    /// North up: WoW +x up the map, +y to its left; the course centred and filling it.
    #[test]
    fn the_course_fits_the_map_north_up() {
        let fit = Fit::of(&square(3).points);
        let half = SIDE / 2.0;
        assert_eq!(
            fit.at([50.0, 50.0]),
            Vec2::splat(half),
            "its middle in the map's"
        );
        let north = fit.at([100.0, 50.0]);
        assert!(north.y < half && (north.x - half).abs() < 1e-3, "{north}");
        let west = fit.at([50.0, 100.0]);
        assert!(west.x < half, "+y to the left: {west}");
        let span = fit.at([0.0, 0.0]).y - fit.at([100.0, 0.0]).y;
        assert!((span - SIDE * FILL).abs() < 1e-3);
    }

    /// A circuit's road closes round to its start, a sprint's ends at its finish.
    #[test]
    fn a_circuit_closes_and_a_sprint_does_not() {
        let fit = Fit::of(&square(3).points);
        assert_eq!(legs(&square(3), &fit).len(), 4);
        assert_eq!(legs(&square(0), &fit).len(), 3);
    }

    /// The road is drawn on the course, solid and sand-coloured on it, and clear off it; the start
    /// is checkered.
    #[test]
    fn the_road_is_drawn_along_the_course_and_nowhere_else() {
        let track = square(3);
        let fit = Fit::of(&track.points);
        let (n, rgba) = draw_track(&track, &fit);
        let px = |p: Vec2| {
            let q = (p * SUPERSAMPLE).as_uvec2();
            let at = ((q.y * n + q.x) * 4) as usize;
            [rgba[at], rgba[at + 1], rgba[at + 2], rgba[at + 3]]
        };
        // Halfway along a leg: the sand, opaque.
        let mid = (fit.at([100.0, 0.0]) + fit.at([100.0, 100.0])) / 2.0 + Vec2::new(0.0, 1.5);
        let road = px(mid);
        assert_eq!(road[3], 255, "{road:?}");
        assert!(road[0] > 180 && road[2] < 170, "sand: {road:?}");
        // The middle of the loop, clear.
        assert_eq!(px(Vec2::splat(SIDE / 2.0))[3], 0);
        // The start: both a white and a black square.
        let start = fit.at([0.0, 0.0]);
        let a = px(start + Vec2::new(-3.0, -3.0));
        let b = px(start + Vec2::new(-1.0, -3.0));
        assert_ne!(a[0] > 128, b[0] > 128, "checkered: {a:?} {b:?}");
    }

    /// Us gold and bigger, a rival red ringed white, one only at their last checkpoint dimmed.
    #[test]
    fn a_racer_s_dot_says_who_and_how_sure() {
        let racer = |me, near| RacerDot {
            name: "Goblin".into(),
            at: [0.0, 0.0],
            place: 2,
            me,
            near,
        };
        assert_eq!(looks(&racer(true, true)).0, GOLD);
        let (fill, ring) = looks(&racer(false, true));
        assert_eq!((fill.alpha(), ring), (1.0, Color::WHITE));
        assert_eq!(looks(&racer(false, false)).0.alpha(), FAR_ALPHA);
        const { assert!(ME > RACER) };
    }
}
