//! The Kartcraft menu (Options on the controller, `KARTCRAFT_MENU`), bevy_ui over the world: a
//! grid of icon tiles, its subcategories ([`PAGES`]), each opening into a grid of its entries.
//! The controller drives it alone while it is up ([`EnginePad`]): the D-pad or the left stick
//! moves, Cross chooses, Circle goes back, L1 and R1 turn the subcategory, Options closes; the
//! mouse and the arrows, Enter and Escape do as well. The entries are Lua's
//! (`BENILLA_KART_MENU`): which are shown now and their labels come in as [`KartMenu::rows`], and
//! one chosen goes back by its id (`crate::ui_kart`), the menu closed first so a window it opens
//! takes the controller. An entry only for a mode (Leave race in a race) is in the rows only
//! then. The dungeon teleporter is a grid of its own, the server's dungeons (`dt:<map>` rows),
//! each shown by its loading screen's art.

use std::collections::HashMap;

use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use benilla_assets::{BlpLoaderSettings, BlpVariant, MapCatalogRes};

use super::track_edit_ui::{backdrop, label, FONT, FRAME, FRAME_HOVER, SMALL, TITLE};
use crate::glue::art::{GlueArt, GOLD};
use crate::ui_script::{EnginePad, EnginePanel};

/// Over the editor's panels and popups.
const Z: i32 = 130;
/// The tiles a row holds, a tile's size and its icon's.
const COLS: usize = 4;
const TILE: Vec2 = Vec2::new(120.0, 92.0);
const TILE_ICON: f32 = 44.0;
/// A dungeon's art, its loading screen's 4:3.
const TILE_ART: Vec2 = Vec2::new(104.0, 78.0);
/// The entry that opens the dungeons' grid, and their rows' prefix.
const DUNGEONS: &str = "dungeons";
const DUNGEON_ROW: &str = "dt:";
/// A dungeon without its own loading screen.
const DUNGEON_ICON: &str = "Interface\\Icons\\Spell_Arcane_TeleportStormWind";
/// The stick past this leans a step, and held, steps again this often (s).
const STICK_LEAN: f32 = 0.5;
const STICK_REPEAT: f32 = 0.22;
const LEGEND: &str =
    "D-pad / stick: move    Cross / A: choose    Circle / B: back    L1 / R1: subcategory";

/// A subcategory: its name and icon, and its entries' ids (Lua's) with their icons.
pub(crate) struct Page {
    name: &'static str,
    icon: &'static str,
    entries: &'static [(&'static str, &'static str)],
}

pub(crate) const PAGES: [Page; 6] = [
    Page {
        name: "Race",
        icon: "Interface\\Icons\\Ability_Rogue_Sprint",
        entries: &[
            ("find", "Interface\\Icons\\Ability_Hunter_Pathfinding"),
            ("leave", "Interface\\Icons\\Ability_Rogue_Feint"),
            ("restart", "Interface\\Icons\\Spell_Holy_Resurrection"),
            ("daily", "Interface\\Icons\\INV_Misc_PocketWatch_01"),
            ("weekly", "Interface\\Icons\\INV_Misc_PocketWatch_02"),
            ("finder", "Interface\\Icons\\INV_Misc_Spyglass_02"),
            ("tracks", "Interface\\Icons\\INV_Misc_Map_01"),
            ("replays", "Interface\\Icons\\Spell_Holy_MindVision"),
            ("racemode", "Interface\\Icons\\Spell_Holy_PowerWordShield"),
            ("unstuck", "Interface\\Icons\\Spell_Nature_Earthbind"),
        ],
    },
    Page {
        name: "Track editor",
        icon: "Interface\\Icons\\INV_Misc_Wrench_01",
        entries: &[
            ("editor", "Interface\\Icons\\INV_Misc_Wrench_01"),
            ("mytracks", "Interface\\Icons\\INV_Scroll_03"),
            ("pubtracks", "Interface\\Icons\\INV_Scroll_05"),
        ],
    },
    Page {
        name: "World",
        icon: "Interface\\Icons\\Ability_Tracking",
        entries: &[
            (DUNGEONS, DUNGEON_ICON),
            ("leavedungeon", "Interface\\Icons\\Spell_Arcane_Blink"),
            ("jobs", "Interface\\Icons\\INV_Crate_01"),
        ],
    },
    Page {
        name: "Kart",
        icon: "Interface\\Icons\\Ability_Mount_MechaStrider",
        entries: &[
            ("garage", "Interface\\Icons\\Trade_Engineering"),
            ("shop", "Interface\\Icons\\INV_Misc_Coin_01"),
        ],
    },
    Page {
        name: "Settings",
        icon: "Interface\\Icons\\INV_Misc_Gear_01",
        entries: &[
            ("settings", "Interface\\Icons\\INV_Misc_Gear_01"),
            ("pad", "Interface\\Icons\\INV_Gizmo_01"),
        ],
    },
    Page {
        name: "Help",
        icon: "Interface\\Icons\\INV_Misc_QuestionMark",
        entries: &[("guide", "Interface\\Icons\\INV_Misc_Book_09")],
    },
];

/// What a press asks of the menu.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Nav {
    Move(IVec2),
    Choose,
    Back,
    Turn(i32),
    Close,
}

/// A tile: its id (an entry's, a dungeon's row or a subcategory's name), label and picture.
#[derive(Clone, PartialEq, Debug)]
struct Tile {
    id: String,
    label: String,
    art: Art,
}

/// A tile's picture: an icon, or a dungeon's map, by its loading screen.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Art {
    Icon(&'static str),
    Map(u32),
}

/// What a tree was built for: the grid's title, its tiles and what it says with none, at the
/// screen's scale.
#[derive(Clone, PartialEq)]
struct Built {
    title: String,
    tiles: Vec<Tile>,
    empty: &'static str,
    s: f32,
}

/// A tile in the world, its place in the grid.
#[derive(Component)]
pub(crate) struct MenuTile(usize);

#[derive(Resource, Default)]
pub(crate) struct KartMenu {
    pub(crate) open: bool,
    /// The subcategory open, or the subcategories; and in it, the dungeons' grid.
    page: Option<usize>,
    dungeons: bool,
    /// The tile with the focus.
    at: usize,
    /// The entries Lua shows now, id and label (`BenillaKartMenu_Rows`).
    rows: Vec<(String, String)>,
    /// What the feed told Lua last: the menu shown, and when it read the rows (s).
    pub(crate) shown: bool,
    pub(crate) read_at: Option<f32>,
    /// The entry chosen, for Lua to run.
    run: Option<String>,
    /// The stick's lean as last stepped, and when it steps again.
    stick: (IVec2, f32),
    built: Option<Built>,
    root: Option<Entity>,
    images: HashMap<String, Handle<Image>>,
}

impl KartMenu {
    /// Lua's rows, one `id<TAB>label` a line.
    pub(crate) fn set_rows(&mut self, text: &str) {
        self.rows = text
            .lines()
            .filter_map(|l| l.split_once('\t'))
            .map(|(id, label)| (id.to_string(), label.to_string()))
            .collect();
        self.at = self.at.min(self.tiles().len().saturating_sub(1));
    }

    /// The entry chosen, once.
    pub(crate) fn take_run(&mut self) -> Option<String> {
        self.run.take()
    }

    /// The dungeons' grid is up and has none yet: the server's answer is on its way.
    pub(crate) fn waiting(&self) -> bool {
        self.open && self.dungeons && self.tiles().is_empty()
    }

    /// Lua's toggle, or its opening at an entry's own grid (`dungeons`).
    fn toggle(&mut self, at: Option<&str>) {
        if at == Some(DUNGEONS) {
            self.open = true;
            let world = PAGES
                .iter()
                .position(|p| p.entries.iter().any(|e| e.0 == DUNGEONS));
            self.page = world;
            self.enter_dungeons();
            return;
        }
        self.open = !self.open;
        (self.page, self.dungeons, self.at) = (None, false, 0);
    }

    /// Into the dungeons' grid, Lua asked for the list.
    fn enter_dungeons(&mut self) {
        (self.dungeons, self.at) = (true, 0);
        self.run = Some(DUNGEONS.to_string());
    }

    fn title(&self) -> String {
        match (self.page, self.dungeons) {
            (_, true) => "World of Kartcraft: Dungeon teleporter".to_string(),
            (Some(i), false) => format!("World of Kartcraft: {}", PAGES[i].name),
            (None, false) => "World of Kartcraft".to_string(),
        }
    }

    /// The grid as it stands: the subcategories, or the open one's entries Lua shows now.
    fn tiles(&self) -> Vec<Tile> {
        if self.dungeons {
            return (self.rows.iter())
                .filter_map(|(id, label)| {
                    let map = id.strip_prefix(DUNGEON_ROW)?.parse().ok()?;
                    Some(Tile {
                        id: id.clone(),
                        label: label.clone(),
                        art: Art::Map(map),
                    })
                })
                .collect();
        }
        let Some(p) = self.page else {
            return PAGES
                .iter()
                .map(|p| Tile {
                    id: p.name.to_string(),
                    label: p.name.to_string(),
                    art: Art::Icon(p.icon),
                })
                .collect();
        };
        PAGES[p]
            .entries
            .iter()
            .filter_map(|&(id, icon)| {
                let (_, label) = self.rows.iter().find(|(r, _)| r == id)?;
                Some(Tile {
                    id: id.to_string(),
                    label: label.clone(),
                    art: Art::Icon(icon),
                })
            })
            .collect()
    }

    fn nav(&mut self, nav: Nav) {
        let n = self.tiles().len();
        match nav {
            Nav::Move(d) if n > 0 => {
                let (at, cols) = (self.at as i32, COLS as i32);
                self.at = if d.x != 0 {
                    (at + d.x).rem_euclid(n as i32) as usize
                } else {
                    // Up or down a row; off the bottom row's end, onto the last tile.
                    let to = at + d.y * cols;
                    let rows = (n as i32 + cols - 1) / cols;
                    if to < 0 || to / cols >= rows {
                        self.at
                    } else {
                        (to as usize).min(n - 1)
                    }
                };
            }
            Nav::Move(_) => {}
            Nav::Choose => match self.page {
                None => (self.page, self.at) = (Some(self.at.min(PAGES.len() - 1)), 0),
                Some(_) => match self.tiles().get(self.at) {
                    Some(t) if t.id == DUNGEONS && !self.dungeons => self.enter_dungeons(),
                    Some(t) => {
                        self.run = Some(t.id.clone());
                        self.open = false;
                    }
                    None => {}
                },
            },
            Nav::Back => match (self.page, self.dungeons) {
                (_, true) => {
                    self.dungeons = false;
                    let tiles = self.tiles();
                    self.at = tiles.iter().position(|t| t.id == DUNGEONS).unwrap_or(0);
                }
                (Some(p), false) => (self.page, self.at) = (None, p),
                (None, false) => self.open = false,
            },
            Nav::Turn(_) if self.dungeons => {}
            Nav::Turn(d) => match self.page {
                Some(p) => {
                    let p = (p as i32 + d).rem_euclid(PAGES.len() as i32) as usize;
                    (self.page, self.at) = (Some(p), 0);
                }
                None => self.nav(Nav::Move(IVec2::new(d, 0))),
            },
            Nav::Close => self.open = false,
        }
    }
}

/// `KartMenu`, counting Lua's toggles (`BenillaKartMenu_Toggle`), opens and closes the menu;
/// `<n>\t<id>` opens it at an entry's own grid (`BenillaKartMenu_Open`).
pub(crate) fn on_cvar(ev: On<crate::cvars::CvarChanged>, mut menu: ResMut<KartMenu>) {
    if ev.key() == "kartmenu" {
        menu.toggle(ev.new.split('\t').nth(1));
    }
}

/// The controller's presses as the menu reads them.
fn pad_nav(name: &str) -> Option<Nav> {
    Some(match name {
        "PADDUP" => Nav::Move(IVec2::NEG_Y),
        "PADDDOWN" => Nav::Move(IVec2::Y),
        "PADDLEFT" => Nav::Move(IVec2::NEG_X),
        "PADDRIGHT" => Nav::Move(IVec2::X),
        "PAD1" => Nav::Choose,
        "PAD2" => Nav::Back,
        "PADLSHOULDER" => Nav::Turn(-1),
        "PADRSHOULDER" => Nav::Turn(1),
        "PADFORWARD" => Nav::Close,
        _ => return None,
    })
}

/// The left stick's lean as a grid step (down positive), the stronger axis only.
fn stick_step(at: Vec2) -> IVec2 {
    match (at.x.abs() >= STICK_LEAN, at.y.abs() >= STICK_LEAN) {
        _ if at.x.abs() < STICK_LEAN && at.y.abs() < STICK_LEAN => IVec2::ZERO,
        _ if at.x.abs() >= at.y.abs() => IVec2::new(at.x.signum() as i32, 0),
        _ => IVec2::new(0, -at.y.signum() as i32),
    }
}

/// The menu's presses: the controller's (handed over by `feed_pad`), its stick, the keys and the
/// tiles' clicks; and whether it holds the controller, for the next frame's feed.
#[allow(clippy::type_complexity)]
pub(crate) fn input(
    mut menu: ResMut<KartMenu>,
    mut pad: ResMut<EnginePad>,
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    (clicks, tiles, pads): (
        Res<crate::glue::GlueClicks>,
        Query<(Entity, &MenuTile, &Interaction)>,
        Query<&Gamepad, Without<kartcraft::input::PadIgnored>>,
    ),
) {
    if menu.open {
        let mut navs: Vec<Nav> = pad.presses.iter().filter_map(|n| pad_nav(n)).collect();
        let key_navs = [
            (KeyCode::ArrowUp, Nav::Move(IVec2::NEG_Y)),
            (KeyCode::ArrowDown, Nav::Move(IVec2::Y)),
            (KeyCode::ArrowLeft, Nav::Move(IVec2::NEG_X)),
            (KeyCode::ArrowRight, Nav::Move(IVec2::X)),
            (KeyCode::Enter, Nav::Choose),
            (KeyCode::NumpadEnter, Nav::Choose),
            (KeyCode::Escape, Nav::Back),
        ];
        navs.extend(
            key_navs
                .iter()
                .filter(|(k, _)| keys.just_pressed(*k))
                .map(|(_, n)| *n),
        );
        // The stick: a step on leaning, then one every repeat while it leans the same way.
        let lean = pads
            .iter()
            .map(|p| Vec2::new(p.left_stick().x, p.left_stick().y))
            .max_by(|a, b| a.length().total_cmp(&b.length()))
            .map_or(IVec2::ZERO, stick_step);
        let now = time.elapsed_secs();
        if lean == IVec2::ZERO {
            menu.stick = (IVec2::ZERO, 0.0);
        } else if lean != menu.stick.0 || now >= menu.stick.1 {
            menu.stick = (lean, now + STICK_REPEAT);
            navs.push(Nav::Move(lean));
        }
        for (e, tile, interaction) in &tiles {
            // The pointer over a tile gives it the focus; a click chooses it.
            if *interaction == Interaction::Hovered && menu.at != tile.0 {
                menu.at = tile.0;
            }
            if clicks.hit(e) {
                menu.at = tile.0;
                navs.push(Nav::Choose);
            }
        }
        for nav in navs {
            if menu.open {
                menu.nav(nav);
            }
        }
    }
    if pad.open != menu.open {
        pad.open = menu.open;
    }
}

/// The menu as it stands: its tree built again when its subcategory or tiles change, the focus
/// framed gold.
pub(crate) fn draw(
    mut commands: Commands,
    mut menu: ResMut<KartMenu>,
    (art, assets): (Res<GlueArt>, Res<AssetServer>),
    (maps, screens): (
        Option<Res<MapCatalogRes>>,
        Option<Res<crate::loading_screen::LoadingScreenCatalogRes>>,
    ),
    window: Query<&Window, With<PrimaryWindow>>,
    mut frames: Query<(&MenuTile, &Interaction, &mut BorderColor)>,
) {
    let s = crate::glue::screen_scale(window.single().ok());
    let want = menu.open.then(|| Built {
        title: menu.title(),
        tiles: menu.tiles(),
        empty: if menu.dungeons {
            "Asking the server for its dungeons..."
        } else {
            "Nothing here now."
        },
        s,
    });
    if menu.built != want {
        if let Some(root) = menu.root.take() {
            commands.entity(root).despawn();
        }
        menu.built = want.clone();
        if let Some(built) = want {
            let font = crate::char_select::wow_font(&assets);
            let images = &mut menu.images;
            // An icon's file, or a dungeon's loading screen (`Map.dbc` to `LoadingScreens.dbc`).
            let mut image = |a: Art| -> Handle<Image> {
                let path = match a {
                    Art::Icon(p) => format!("{p}.blp"),
                    Art::Map(m) => (maps.as_ref().and_then(|c| c.0.loading_screen_id(m)))
                        .and_then(|id| Some(screens.as_ref()?.0.path(id)?.to_string()))
                        .unwrap_or_else(|| format!("{DUNGEON_ICON}.blp")),
                };
                let load = || {
                    let url = format!("mpq://{}", path.replace('\\', "/"));
                    assets.load_with_settings(url, |s: &mut BlpLoaderSettings| {
                        s.variant = BlpVariant::Sprite;
                    })
                };
                images.entry(path.clone()).or_insert_with(load).clone()
            };
            let root = spawn(&mut commands, &art, &mut image, &font, &built);
            menu.root = Some(root);
        }
    }
    for (tile, interaction, mut frame) in &mut frames {
        let color = match (tile.0 == menu.at, interaction) {
            (true, _) => GOLD,
            (false, Interaction::None) => FRAME,
            _ => FRAME_HOVER,
        };
        if frame.top != color {
            *frame = BorderColor::all(color);
        }
    }
}

/// The window: its title, the grid and the controller's legend, in the middle of the screen.
fn spawn(
    commands: &mut Commands,
    art: &GlueArt,
    image: &mut dyn FnMut(Art) -> Handle<Image>,
    font: &Handle<Font>,
    built: &Built,
) -> Entity {
    let s = built.s;
    let px = |v: f32| Val::Px(v * s);
    let full = Node {
        position_type: PositionType::Absolute,
        width: Val::Percent(100.0),
        height: Val::Percent(100.0),
        justify_content: JustifyContent::Center,
        align_items: AlignItems::Center,
        ..default()
    };
    let mut root = commands.spawn((full, GlobalZIndex(Z), Pickable::IGNORE));
    root.with_children(|r| {
        let node = Node {
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            padding: UiRect::all(px(16.0)),
            row_gap: px(10.0),
            ..default()
        };
        let mut panel = r.spawn((EnginePanel, Interaction::default(), node));
        backdrop(&mut panel, art, s);
        panel.with_children(|p| {
            label(p, font, &built.title, TITLE, GOLD, Node::default(), s);
            let grid = Node {
                width: px(COLS as f32 * (TILE.x + 8.0)),
                flex_wrap: FlexWrap::Wrap,
                column_gap: px(8.0),
                row_gap: px(8.0),
                ..default()
            };
            p.spawn(grid).with_children(|g| {
                for (i, t) in built.tiles.iter().enumerate() {
                    tile(g, image, font, s, i, t);
                }
            });
            if built.tiles.is_empty() {
                let white = Color::WHITE;
                label(p, font, built.empty, FONT, white, Node::default(), s);
            }
            let grey = Color::srgb(0.6, 0.6, 0.6);
            label(p, font, LEGEND, SMALL, grey, Node::default(), s);
        });
    });
    root.id()
}

/// A tile: its picture over its label, framed; a dungeon's picture its loading screen, wider.
fn tile(
    parent: &mut ChildSpawnerCommands,
    image: &mut dyn FnMut(Art) -> Handle<Image>,
    font: &Handle<Font>,
    s: f32,
    i: usize,
    t: &Tile,
) {
    let px = |v: f32| Val::Px(v * s);
    let node = Node {
        width: px(TILE.x),
        min_height: px(TILE.y),
        flex_direction: FlexDirection::Column,
        align_items: AlignItems::Center,
        justify_content: JustifyContent::Center,
        row_gap: px(6.0),
        border: UiRect::all(px(2.0)),
        padding: UiRect::all(px(4.0)),
        ..default()
    };
    let looks = (
        BorderColor::all(FRAME),
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
    );
    parent
        .spawn((MenuTile(i), Button, node, looks))
        .with_children(|b| {
            let size = match t.art {
                Art::Icon(_) => Vec2::splat(TILE_ICON),
                Art::Map(_) => TILE_ART,
            };
            let picture = Node {
                width: px(size.x),
                height: px(size.y),
                ..default()
            };
            b.spawn((ImageNode::new(image(t.art)), picture));
            let text = Node {
                max_width: px(TILE.x - 8.0),
                ..default()
            };
            label(b, font, &t.label, FONT, Color::WHITE, text, s);
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn menu(rows: &str) -> KartMenu {
        let mut m = KartMenu::default();
        m.toggle(None);
        m.set_rows(rows);
        m
    }

    #[test]
    fn options_opens_the_subcategories_and_cross_opens_one() {
        let mut m = menu("find\tFind a race\nunstuck\tUnstuck\neditor\tOpen the editor");
        assert!(m.open);
        let names: Vec<String> = m.tiles().into_iter().map(|t| t.id).collect();
        assert_eq!(names[..2], ["Race", "Track editor"]);
        m.nav(Nav::Move(IVec2::X));
        m.nav(Nav::Choose);
        assert_eq!(m.page, Some(1));
        let ids: Vec<String> = m.tiles().into_iter().map(|t| t.id).collect();
        assert_eq!(
            ids,
            ["editor"],
            "only what Lua shows: My tracks is not in these rows"
        );
        m.nav(Nav::Choose);
        assert!(!m.open, "closed first");
        assert_eq!(m.take_run().as_deref(), Some("editor"));
        assert_eq!(m.take_run(), None);
    }

    #[test]
    fn an_entry_for_a_mode_has_a_tile_only_while_lua_shows_it() {
        let mut m = menu("find\tFind a race\nunstuck\tUnstuck");
        m.nav(Nav::Choose);
        let ids = |m: &KartMenu| m.tiles().into_iter().map(|t| t.id).collect::<Vec<_>>();
        assert_eq!(ids(&m), ["find", "unstuck"]);
        m.nav(Nav::Move(IVec2::X));
        m.set_rows("find\tLeave the queue\nleave\tLeave race\nunstuck\tUnstuck");
        assert_eq!(ids(&m), ["find", "leave", "unstuck"]);
        assert_eq!(m.tiles()[0].label, "Leave the queue");
        // Out of the race again: the focus stays on a tile.
        m.nav(Nav::Move(IVec2::X));
        m.set_rows("find\tFind a race");
        assert_eq!(m.at, 0);
    }

    #[test]
    fn the_d_pad_walks_the_grid_and_circle_goes_back_then_closes() {
        let mut m = menu("");
        // Six subcategories, four a row.
        m.nav(Nav::Move(IVec2::Y));
        assert_eq!(m.at, 4);
        m.nav(Nav::Move(IVec2::Y));
        assert_eq!(m.at, 4, "no row under");
        m.nav(Nav::Move(IVec2::NEG_X));
        m.nav(Nav::Move(IVec2::NEG_Y));
        assert_eq!(m.at, 3);
        m.nav(Nav::Move(IVec2::Y));
        assert_eq!(m.at, 5, "onto the short row's last");
        m.nav(Nav::Move(IVec2::X));
        assert_eq!(m.at, 0, "round");
        m.nav(Nav::Move(IVec2::X));
        m.nav(Nav::Move(IVec2::X));
        m.nav(Nav::Choose);
        m.nav(Nav::Turn(1));
        assert_eq!(m.page, Some(3), "R1: the next subcategory");
        m.nav(Nav::Back);
        assert_eq!((m.page, m.at, m.open), (None, 3, true));
        m.nav(Nav::Back);
        assert!(!m.open);
        assert_eq!(m.take_run(), None);
    }

    /// The dungeon teleporter opens a grid of the server's dungeons, each by its map; Circle
    /// goes back to its entry, and the minimap's menu opens the grid straight away.
    #[test]
    fn the_dungeon_teleporter_is_a_grid_of_the_server_s_dungeons() {
        let rows = "dungeons\tDungeon teleporter\njobs\tCourier jobs";
        let mut m = menu(rows);
        m.nav(Nav::Move(IVec2::X));
        m.nav(Nav::Move(IVec2::X));
        m.nav(Nav::Choose);
        assert_eq!(m.title(), "World of Kartcraft: World");
        m.nav(Nav::Choose);
        assert!(m.open, "the dungeons' grid, not a run that closes");
        assert_eq!(
            m.take_run().as_deref(),
            Some("dungeons"),
            "Lua asks for them"
        );
        assert!(m.waiting());
        m.set_rows(&format!("{rows}\ndt:389\tRagefire Chasm\ndt:36\tDeadmines"));
        assert!(!m.waiting());
        let tiles = m.tiles();
        assert_eq!(tiles[1].art, Art::Map(36));
        assert_eq!(tiles[1].label, "Deadmines");
        m.nav(Nav::Turn(1));
        m.nav(Nav::Move(IVec2::X));
        m.nav(Nav::Choose);
        assert!(!m.open);
        assert_eq!(m.take_run().as_deref(), Some("dt:36"));
        // Back out of it onto its entry.
        let mut m = menu(rows);
        m.toggle(Some("dungeons"));
        assert!(
            m.open && m.dungeons,
            "the minimap's: straight to the dungeons"
        );
        m.take_run();
        m.nav(Nav::Back);
        assert_eq!((m.dungeons, m.page, m.at), (false, Some(2), 0));
    }

    #[test]
    fn the_stick_leans_one_way_at_a_time() {
        assert_eq!(stick_step(Vec2::new(0.2, -0.3)), IVec2::ZERO);
        assert_eq!(stick_step(Vec2::new(0.9, 0.6)), IVec2::X);
        assert_eq!(stick_step(Vec2::new(0.1, -0.8)), IVec2::Y, "down the grid");
        assert_eq!(pad_nav("PAD2"), Some(Nav::Back));
        assert_eq!(pad_nav("PAD3"), None);
    }

    /// Every entry's id is one of Lua's (`BENILLA_KART_MENU`), each once.
    #[test]
    fn every_entry_is_once_in_a_subcategory() {
        let ids: Vec<&str> = PAGES
            .iter()
            .flat_map(|p| p.entries.iter().map(|e| e.0))
            .collect();
        let mut seen = ids.clone();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), ids.len());
        let lua = include_str!("../../assets/ui/KartcraftFrames.xml");
        for id in ids {
            assert!(
                lua.contains(&format!("{{ id = \"{id}\"")),
                "{id} is not Lua's"
            );
        }
    }

    #[test]
    fn every_icon_is_in_the_install() {
        let data = benilla_formats::wow_data_or_skip!();
        let chain = benilla_formats::open_chain(&data).expect("the install's archives");
        let all = PAGES
            .iter()
            .flat_map(|p| std::iter::once(p.icon).chain(p.entries.iter().map(|e| e.1)))
            .chain([DUNGEON_ICON]);
        let missing: Vec<&str> = all
            .filter(|p| !chain.contains(&format!("{p}.blp")))
            .collect();
        assert!(missing.is_empty(), "not in the install: {missing:?}");
    }

    /// Every party dungeon the teleporter can offer has its own loading screen to show.
    #[test]
    fn every_dungeon_s_loading_screen_is_in_the_install() {
        let data = benilla_formats::wow_data_or_skip!();
        let mut chain = benilla_formats::open_chain(&data).expect("the install's archives");
        let maps = benilla_formats::load_map_catalog(&mut chain).expect("Map.dbc");
        let screens = benilla_formats::load_loading_screens(&mut chain).expect("LoadingScreens");
        let dungeons = [
            33, 34, 36, 43, 47, 48, 70, 90, 109, 129, 189, 209, 229, 230, 289, 329, 349, 389, 429,
        ];
        for map in dungeons {
            let path = maps.loading_screen_id(map).and_then(|id| screens.path(id));
            let path = path.unwrap_or_else(|| panic!("map {map}: no loading screen"));
            assert!(chain.contains(path), "map {map}: {path}");
        }
    }
}
