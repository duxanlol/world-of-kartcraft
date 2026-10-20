//! The track editor's own interface over the world, bevy_ui, while the game's is hidden: Exit at
//! the top left, Test lap and Save at the top right, what a click does under them, and named bars
//! of icon buttons (each naming itself on hover) along the bottom: the editor's controls, the
//! checkpoints and the race type. Then the keys card, a test lap's panel and the popups: the
//! save's (the track's name, warned when it replaces one) and one for each thing that cannot be
//! done. A press is a [`TrackCmd`]; each tree is built again when what it shows changes, and the
//! fields' text lives in [`EditorUi`].

use std::collections::HashMap;

use benilla_assets::{BlpLoaderSettings, BlpVariant};
use benilla_ui::widget::EditBoxState;
use bevy::input::keyboard::KeyboardInput;
use bevy::input::ButtonState;
use bevy::prelude::*;
use bevy::ui::FocusPolicy;
use bevy::window::{PrimaryWindow, RawHandleWrapper};

use super::objects::{kinds, Models};
use super::track_edit::{EditView, TrackCmd, TrackEdit, TURN_KEYS};
use crate::glue::art::{GlueArt, GOLD};
use crate::glue::backdrop::{backdrop_border, tiled_bg_node};
use crate::glue::widgets::{glue_edit_box, markup_spans, overlay, paint_glue_field, GlueFieldPart};
use crate::textinput::{self, HostClipboard};
use crate::ui_script::{EngineKeys, EnginePanel, EngineTyping};

/// Over the world's FrameXML, under the cinematic (900) and the loading screen (1000); the
/// hover names over their bars, the popups over everything of ours.
const Z: i32 = 100;
const TIP_Z: i32 = Z + 10;
const POPUP_Z: i32 = Z + 20;
/// The panels' tooltip backdrop and the boxes', as the glue dialog tints them.
const FILL: Color = Color::srgba(0.09, 0.09, 0.09, 0.92);
const EDGE: Color = Color::srgb(0.8, 0.8, 0.8);
/// An icon's frame: plain, under the pointer, and lit for what is picked.
const FRAME: Color = Color::srgb(0.25, 0.25, 0.25);
const FRAME_HOVER: Color = Color::WHITE;
/// The start's number, green as its marker.
const START_GREEN: Color = Color::srgb(0.3, 1.0, 0.4);
/// What cannot be done, and a save that replaces a track.
const WARN: Color = Color::srgb(1.0, 0.35, 0.3);
/// `GameFontNormal`, `GameFontHighlightSmall`, `GameFontNormalLarge`.
const FONT: f32 = 12.0;
const SMALL: f32 = 10.0;
const TITLE: f32 = 16.0;
/// An icon button's side, and a labelled button's icon.
const ICON: f32 = 36.0;
const SMALL_ICON: f32 = 24.0;
/// How long a note shows under the hint (s).
const NOTE_SECS: f32 = 6.0;
/// The name a track takes when its box is empty.
const DEFAULT_NAME: &str = "My Track";

/// The buttons' art, from the player's own install.
mod icon {
    pub(super) const EXIT: &str = "Interface\\Buttons\\UI-Panel-MinimizeButton-Up";
    pub(super) const SAVE: &str = "Interface\\Icons\\INV_Scroll_03";
    pub(super) const TEST: &str = "Interface\\Icons\\INV_Misc_PocketWatch_01";
    pub(super) const MOVE: &str = "Interface\\Icons\\INV_Gauntlets_04";
    pub(super) const UNDO: &str = "Interface\\Buttons\\UI-SpellbookIcon-PrevPage-Up";
    pub(super) const REDO: &str = "Interface\\Buttons\\UI-SpellbookIcon-NextPage-Up";
    pub(super) const PLACE: &str = "Interface\\Icons\\INV_Banner_02";
    pub(super) const NUMBER: &str = "Interface\\Icons\\INV_Misc_Note_02";
    pub(super) const GRID: &str = "Interface\\Icons\\Ability_Warrior_Charge";
    pub(super) const ERASE: &str = "Interface\\Buttons\\UI-GroupLoot-Pass-Up";
    pub(super) const SPRINT: &str = "Interface\\Icons\\Ability_Rogue_Sprint";
    pub(super) const LAPS: &str = "Interface\\Icons\\Spell_Nature_Cyclone";
    pub(super) const BACK: &str = "Interface\\Icons\\INV_Misc_Wrench_01";
    pub(super) const RESTART: &str = "Interface\\Icons\\Spell_Holy_Resurrection";
    pub(super) const OK: &str = "Interface\\Buttons\\UI-CheckBox-Check";
    pub(super) const OBJECTS: &str = "Interface\\Icons\\INV_Box_01";

    pub(super) const SIZE: &str = "Interface\\Icons\\INV_Misc_Spyglass_02";

    #[cfg(test)]
    pub(super) const ALL: [&str; 17] = [
        EXIT, SAVE, TEST, MOVE, UNDO, REDO, PLACE, NUMBER, GRID, ERASE, SPRINT, LAPS, BACK,
        RESTART, OK, OBJECTS, SIZE,
    ];
}

/// The Race type bar's sizes: the karts' scale on the track, its own then smaller, each with
/// its name (the hover's) and its command.
const SIZES: [(f32, &str, &[&str]); 4] = [
    (1.0, "Size x1", &["scale", "1"]),
    (0.75, "Size x0.75", &["scale", "0.75"]),
    (0.5, "Size x0.5", &["scale", "0.5"]),
    (0.35, "Size x0.35", &["scale", "0.35"]),
];

/// The keys card's text, from the bindings as they are when the editor opens
/// ([`crate::ui_kart`], which holds the VM).
#[derive(Resource, Default)]
pub(crate) struct EditorKeysText(pub(crate) String);

/// The keys card's lines: the kart's own bindings the free camera flies on, then the mouse's.
const KEYS: [(&str, Option<&str>, &str); 4] = [
    (
        "KARTCRAFT_ACCELERATE",
        Some("KARTCRAFT_BRAKE"),
        "fly forward, back",
    ),
    (
        "KARTCRAFT_STEERLEFT",
        Some("KARTCRAFT_STEERRIGHT"),
        "fly left, right",
    ),
    ("KARTCRAFT_HOP", Some("KARTCRAFT_LOOKBACK"), "fly up, down"),
    ("KARTCRAFT_BOOST", None, "held: fly fast"),
];

/// The keys card's text from the bindings, `(command, keys)` as the VM lists them.
pub(crate) fn keys_text(bindings: &[(String, Vec<String>)]) -> String {
    let key = |command: &str| {
        let keys = bindings.iter().find(|(c, _)| c == command);
        keys.and_then(|(_, k)| k.first())
            .map_or("unbound".to_string(), Clone::clone)
    };
    let mut lines: Vec<String> = KEYS
        .iter()
        .map(|(a, b, what)| {
            let keys = match b {
                Some(b) => format!("{} / {}", key(a), key(b)),
                None => key(a),
            };
            format!("|cffffd100{keys}|r  {what}")
        })
        .collect();
    lines.extend(
        [
            "|cffffd100Left click|r  place with the tool picked",
            "|cffffd100Left drag|r  with Move: move a gate or the grid",
            "|cffffd100Q / E|r or |cffffd100middle mouse held|r  turn the gate",
            "|cffffd100Mouse wheel|r  widen or narrow the gate",
            "|cffffd100Q/E, R/F, Z/C|r  turn an object: yaw, pitch, roll",
            "|cffffd100Middle mouse held|r  turn it as you see it (Shift: roll)",
            "|cffffd100Ctrl, mouse up and down|r  raise or lower it",
            "|cffffd100Controller Fire|r  place at the screen's centre",
            "|cffffd100Right mouse held|r  look around",
            "|cff999999The first checkpoint is the start; drag this card to move it.|r",
        ]
        .map(String::from),
    );
    lines.join("\n")
}

/// What a button does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Act {
    /// One of the editor's commands.
    Cmd(&'static [&'static str]),
    /// A test lap of the draft, under its name.
    Practice,
    /// The save popup: open it, save, or not.
    Save,
    SaveConfirm,
    Cancel,
    /// The error popup's OK.
    Dismiss,
    /// The objects panel: open or close it, open one of its tabs, pick a kind to place.
    Objects,
    Tab(&'static str),
    Object(&'static str),
}

/// A button and what it does.
#[derive(Component, Clone, Copy)]
pub(crate) struct EditorBtn(Act);

/// A button's frame shows it picked.
#[derive(Component, Clone, Copy)]
pub(crate) struct Lit(bool);

/// A checkpoint's number, hung over its marker.
#[derive(Component)]
pub(crate) struct GateNumber;

/// A button's name, shown over it while the pointer is on it.
#[derive(Component)]
pub(crate) struct Tip;

/// The two text fields: the save popup's name, and Edit checkpoint's new number.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Field {
    Name,
    Number,
}

/// A field's text row, painted from its [`EditBoxState`].
#[derive(Component, Clone, Copy)]
pub(crate) struct FieldText(Field);

/// The panels; the keys card and the lap panel drag anywhere.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Panel {
    Bar,
    Keys,
    Lap,
}

/// The popup up, over everything of ours.
#[derive(Clone, PartialEq, Eq, Debug)]
enum Popup {
    /// What could not be done, and why.
    Error(String),
    /// The save: the name, and whether a track of ours has it already.
    Save { replaces: bool },
}

/// The interface's state: the fields, the one with the keys, the popup, and what each tree was
/// built from.
#[derive(Resource)]
pub(crate) struct EditorUi {
    name: EditBoxState,
    number: EditBoxState,
    focus: Option<Field>,
    popup: Option<Popup>,
    /// The checkpoint Edit checkpoint picked, the errors and the notes, when last seen, and when
    /// that note came.
    picked: usize,
    errors: u32,
    notes: u32,
    note_at: f32,
    root: Option<Entity>,
    built: Option<Built>,
    popup_root: Option<Entity>,
    popup_built: Option<(Popup, f32)>,
    icons: HashMap<&'static str, Handle<Image>>,
    /// The checkpoints' numbers: their root, which checkpoints at which scale, and each label.
    numbers_root: Option<Entity>,
    numbers_of: (Vec<usize>, f32),
    numbers: Vec<Entity>,
    /// Whether the objects panel is open, and its tab.
    objects_open: bool,
    tab: &'static str,
    /// How far the keys card and the lap panel were dragged (px), and the cursor a drag is at.
    moved: [Vec2; 2],
    drag_at: Option<Vec2>,
}

/// What the panels show: the editor's view, its hint and the last note, the keys card, the scale
/// and window, whether the art had loaded, and the objects panel when open.
#[derive(Clone, PartialEq)]
struct Built {
    view: EditView,
    hint: String,
    note: Option<String>,
    keys: String,
    s: f32,
    window: Vec2,
    art: bool,
    objects: Option<ObjectsPanel>,
}

/// The objects panel: the tab open, and its kinds (key, name and icon once drawn).
#[derive(Clone, PartialEq)]
struct ObjectsPanel {
    tab: &'static str,
    tiles: Vec<(&'static str, &'static str, Option<Handle<Image>>)>,
}

/// The objects panel's tabs: each group with a kind in it, in [`kinds`]' order.
fn object_groups() -> Vec<&'static str> {
    let mut groups: Vec<&'static str> = Vec::new();
    for k in kinds() {
        if !groups.contains(&k.group) {
            groups.push(k.group);
        }
    }
    groups
}

impl Default for EditorUi {
    fn default() -> Self {
        let mut name = textinput::field(40, false);
        name.set_text(DEFAULT_NAME);
        let mut number = textinput::field(3, false);
        number.numeric = true;
        Self {
            name,
            number,
            focus: None,
            popup: None,
            picked: 0,
            errors: 0,
            notes: 0,
            note_at: f32::MIN,
            root: None,
            built: None,
            popup_root: None,
            popup_built: None,
            icons: HashMap::new(),
            numbers_root: None,
            numbers_of: (Vec::new(), 0.0),
            numbers: Vec::new(),
            objects_open: false,
            tab: kinds()[0].group,
            moved: [Vec2::ZERO; 2],
            drag_at: None,
        }
    }
}

impl EditorUi {
    fn field(&mut self, f: Field) -> &mut EditBoxState {
        match f {
            Field::Name => &mut self.name,
            Field::Number => &mut self.number,
        }
    }

    /// The editor as it now stands: closed, the keys go and so does the save popup; a new pick
    /// takes them, for its number, and none lets the number's go; a new error pops up.
    fn follow(&mut self, view: &EditView, (why, errors): (&str, u32)) {
        if view.state != "edit" {
            self.focus = None;
            self.popup = self.popup.take().filter(|p| matches!(p, Popup::Error(_)));
        }
        if view.picked != self.picked {
            self.picked = view.picked;
            self.focus = match view.picked {
                0 => self.focus.filter(|f| *f != Field::Number),
                _ => {
                    self.number.set_text("");
                    Some(Field::Number)
                }
            };
        }
        if errors != self.errors {
            self.errors = errors;
            self.popup = Some(Popup::Error(why.to_string()));
            self.focus = None;
        }
    }

    /// Enter (else Escape) in the field with the keys, which lets them go: Enter on the number
    /// renumbers the pick, if it is a number.
    fn end_field(&mut self, enter: bool) -> Option<TrackCmd> {
        if self.focus.take()? != Field::Number {
            return None;
        }
        let to = self.number.text.clone();
        self.number.set_text("");
        (enter && to.parse::<usize>().is_ok()).then(|| TrackCmd::new(&["renumber", &to]))
    }

    /// The save popup up, its name all selected to type over.
    fn open_save(&mut self, replaces: bool) {
        self.popup = Some(Popup::Save { replaces });
        self.focus = Some(Field::Name);
        self.name.highlight_text(0, -1);
        self.name.reset_blink();
    }

    /// The popup's answer: a save in `zone` over whatever has the name (the popup warned), or
    /// none; the popup goes either way.
    fn answer(&mut self, save: bool, zone: &str) -> Option<TrackCmd> {
        self.focus = None;
        let popup = self.popup.take();
        let saving = save && matches!(popup, Some(Popup::Save { .. }));
        let name = self.track_name();
        saving.then(|| TrackCmd::new(&["save", &name, zone, "overwrite"]))
    }

    /// Escape with no popup or field: it closes the objects panel, else drops the object picked
    /// (back to the move cursor); whether it did either, so the game's menu does not open too.
    fn escape(&mut self, object_picked: bool) -> (bool, Option<TrackCmd>) {
        if std::mem::take(&mut self.objects_open) {
            (true, None)
        } else if object_picked {
            (true, Some(TrackCmd::new(&["move"])))
        } else {
            (false, None)
        }
    }

    /// The name to save or test under.
    fn track_name(&self) -> String {
        let name = self.name.text.trim();
        if name.is_empty() { DEFAULT_NAME } else { name }.to_string()
    }
}

/// The interface's input, before the VM's: a button's press as what it does, a press on a field
/// takes the keys (anywhere else lets them go), the keys a field or a popup has, and the
/// popups' Enter and Escape.
#[allow(clippy::type_complexity)]
pub(crate) fn input(
    mut commands: Commands,
    mut ui: ResMut<EditorUi>,
    mut edit: ResMut<TrackEdit>,
    (mut typing, mut engine_keys): (ResMut<EngineTyping>, ResMut<EngineKeys>),
    (clicks, buttons, fields, mouse): (
        Res<crate::glue::GlueClicks>,
        Query<(Entity, &EditorBtn)>,
        Query<(&Field, &Interaction)>,
        Res<ButtonInput<MouseButton>>,
    ),
    (mut keyboard, keys, layout, mut clipboard, raw_handle): (
        MessageReader<KeyboardInput>,
        Res<ButtonInput<KeyCode>>,
        Res<crate::bindings::LayoutNames>,
        NonSendMut<HostClipboard>,
        Query<&RawHandleWrapper, With<PrimaryWindow>>,
    ),
    (world, areas): (
        benilla_world::world_point::WorldPoint,
        Option<Res<crate::area::AreaTableRes>>,
    ),
) {
    let view = edit.view();
    if let Some(name) = edit.take_named() {
        ui.name.set_text(&name);
    }
    // The keys a field or popup had this frame stay off the VM, the Enter or Escape that let
    // them go too.
    let had_keys = ui.focus.is_some() || ui.popup.is_some();
    ui.follow(&view, edit.error());
    let replaces = edit.is_saved(&ui.track_name());
    if let Some(Popup::Save { replaces: r }) = &mut ui.popup {
        *r = replaces;
    }
    if mouse.just_pressed(MouseButton::Left) {
        let pressed = fields.iter().find(|(_, i)| **i == Interaction::Pressed);
        ui.focus = pressed.map(|(f, _)| *f);
        if let Some(f) = ui.focus {
            ui.field(f).reset_blink();
        }
    }
    let zone = || {
        let zone = world.area().zip(areas.as_deref());
        zone.map_or_else(String::new, |(leaf, areas)| areas.zone_name(leaf))
    };
    let mods = textinput::mods_now(&keys);
    let wl = textinput::wayland_display(raw_handle.iter().next());
    let mut ate_escape = false;
    for ev in keyboard.read() {
        if ev.state != ButtonState::Pressed {
            continue;
        }
        let enter = matches!(ev.key_code, KeyCode::Enter | KeyCode::NumpadEnter);
        let ends = enter || ev.key_code == KeyCode::Escape;
        match (&ui.popup, ui.focus) {
            (Some(_), _) if ends => {
                if let Some(cmd) = ui.answer(enter, &zone()) {
                    commands.trigger(cmd);
                }
            }
            (_, Some(_)) if ends => {
                if let Some(cmd) = ui.end_field(enter) {
                    commands.trigger(cmd);
                }
            }
            (_, Some(f)) => {
                let filter = textinput::CharFilter::Any;
                let field = ui.field(f);
                textinput::feed_key(field, ev, mods, &layout, &mut clipboard, wl, filter);
            }
            (None, None) if ev.key_code == KeyCode::Escape && view.state == "edit" => {
                let (ate, cmd) = ui.escape(view.object.is_some());
                ate_escape |= ate;
                if let Some(cmd) = cmd {
                    commands.trigger(cmd);
                }
            }
            _ => {}
        }
    }
    for (button, EditorBtn(act)) in &buttons {
        if !clicks.hit(button) {
            continue;
        }
        ui.focus = None;
        let cmd = match *act {
            Act::Cmd(words) => Some(TrackCmd::new(words)),
            Act::Practice => Some(TrackCmd::new(&["practice", &ui.track_name()])),
            Act::Save => {
                ui.open_save(replaces);
                None
            }
            Act::SaveConfirm => ui.answer(true, &zone()),
            Act::Cancel | Act::Dismiss => ui.answer(false, ""),
            Act::Objects => {
                ui.objects_open = !ui.objects_open;
                None
            }
            Act::Tab(group) => {
                ui.tab = group;
                None
            }
            Act::Object(key) => {
                ui.objects_open = false;
                Some(TrackCmd::new(&["object", key]))
            }
        };
        if let Some(cmd) = cmd {
            commands.trigger(cmd);
        }
    }
    typing.0 = had_keys || ate_escape || ui.focus.is_some() || ui.popup.is_some();
    // An object turning holds its keys from the game's bindings (R would open the hidden chat).
    let turning = view.turns_object && !typing.0;
    let held = if turning {
        TURN_KEYS.to_vec()
    } else {
        Vec::new()
    };
    if engine_keys.0 != held {
        engine_keys.0 = held;
    }
}

/// The interface as the editor stands: each tree built again when what it shows changes, the
/// buttons' frames and names under the pointer, the fields painted and the caret blinking, and
/// a press on the keys card or the lap panel dragging it.
#[allow(clippy::type_complexity)]
pub(crate) fn draw(
    mut commands: Commands,
    mut ui: ResMut<EditorUi>,
    (edit, keys, art, assets, time, models): (
        Res<TrackEdit>,
        Res<EditorKeysText>,
        Res<GlueArt>,
        Res<AssetServer>,
        Res<Time>,
        Res<Models>,
    ),
    window: Query<&Window, With<PrimaryWindow>>,
    mut panels: Query<(&Panel, &Interaction, &mut Node)>,
    (mut frames, hovered, mut tips): (
        Query<(&Interaction, &Lit, &mut BorderColor)>,
        Query<&Interaction>,
        Query<(&ChildOf, &mut Visibility), (With<Tip>, Without<FieldText>)>,
    ),
    mut boxes: Query<(
        &GlueFieldPart,
        Option<&mut Text>,
        &mut Visibility,
        &FieldText,
    )>,
    (camera, mut numbers): (
        Query<(&Camera, &GlobalTransform), With<benilla_world::view::WorldCamera>>,
        Query<&mut Node, (With<GateNumber>, Without<Panel>)>,
    ),
) {
    let win = window.single().ok();
    let s = crate::glue::screen_scale(win);
    let now = time.elapsed_secs();
    let (note, notes) = edit.notes();
    if notes != ui.notes {
        ui.notes = notes;
        ui.note_at = now;
    }
    let view = edit.view();
    let want = (!view.state.is_empty()).then(|| Built {
        hint: edit.hint(),
        note: (now - ui.note_at < NOTE_SECS && !note.is_empty()).then(|| note.to_string()),
        keys: keys.0.clone(),
        s,
        window: win.map_or(Vec2::new(1024.0, 768.0), |w| w.size()),
        art: art.tooltip_bg.is_some(),
        objects: ui.objects_open.then(|| ObjectsPanel {
            tab: ui.tab,
            tiles: (kinds().iter().filter(|k| k.group == ui.tab))
                .map(|k| (k.key, k.name, models.icon(k.key)))
                .collect(),
        }),
        view,
    });
    let fonts = || {
        let edit_font: Handle<Font> = assets.load("mpq://Fonts/ARIALN.ttf");
        (crate::char_select::wow_font(&assets), edit_font)
    };
    if ui.built != want {
        if let Some(root) = ui.root.take() {
            commands.entity(root).despawn();
        }
        ui.built = want.clone();
        if let Some(built) = want {
            let (font, edit_font) = fonts();
            let moved = ui.moved;
            let mut icons = Icons(&assets, &mut ui.icons);
            let fonts = (&font, &edit_font);
            let root = spawn(&mut commands, &art, &mut icons, fonts, &built, moved);
            ui.root = Some(root);
        }
    }
    let want_popup = ui.popup.clone().map(|p| (p, s));
    if ui.popup_built != want_popup {
        if let Some(root) = ui.popup_root.take() {
            commands.entity(root).despawn();
        }
        ui.popup_built = want_popup.clone();
        if let Some((popup, s)) = want_popup {
            let (font, edit_font) = fonts();
            let mut icons = Icons(&assets, &mut ui.icons);
            let fonts = (&font, &edit_font);
            ui.popup_root = Some(spawn_popup(
                &mut commands,
                &art,
                &mut icons,
                fonts,
                &popup,
                s,
            ));
        }
    }
    // Each checkpoint's number over its marker, where the world camera sees it.
    let marks = edit.marks();
    let numbers_of = (marks.iter().map(|(i, _)| *i).collect::<Vec<_>>(), s);
    if ui.numbers_of != numbers_of {
        if let Some(root) = ui.numbers_root.take() {
            commands.entity(root).despawn();
        }
        ui.numbers.clear();
        if !marks.is_empty() {
            let font = crate::char_select::wow_font(&assets);
            let (root, labels) = spawn_numbers(&mut commands, &font, &numbers_of.0, s);
            (ui.numbers_root, ui.numbers) = (Some(root), labels);
        }
        ui.numbers_of = numbers_of;
    }
    let cam = camera.single().ok();
    for (&label, (_, at)) in ui.numbers.iter().zip(marks) {
        let Ok(mut node) = numbers.get_mut(label) else {
            continue;
        };
        let seen = cam.and_then(|(c, t)| c.world_to_viewport(t, *at).ok());
        let (display, left, top) = match seen {
            Some(p) => (
                Display::Flex,
                Val::Px(p.x - 20.0 * s),
                Val::Px(p.y - 24.0 * s),
            ),
            None => (Display::None, node.left, node.top),
        };
        if (node.display, node.left, node.top) != (display, left, top) {
            (node.display, node.left, node.top) = (display, left, top);
        }
    }
    for (interaction, lit, mut frame) in &mut frames {
        let color = match (interaction, lit.0) {
            (Interaction::None, true) => GOLD,
            (Interaction::None, false) => FRAME,
            _ => FRAME_HOVER,
        };
        if frame.top != color {
            *frame = BorderColor::all(color);
        }
    }
    for (button, mut vis) in &mut tips {
        let on = hovered
            .get(button.parent())
            .is_ok_and(|i| *i != Interaction::None);
        let want = if on {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *vis != want {
            *vis = want;
        }
    }
    // A press held on a card's back (not on its buttons, which take it) drags the card.
    let cursor = win.and_then(Window::cursor_position);
    let mut dragging = false;
    for (panel, interaction, mut node) in &mut panels {
        let slot = match panel {
            Panel::Keys => 0,
            Panel::Lap => 1,
            Panel::Bar => continue,
        };
        if *interaction != Interaction::Pressed {
            continue;
        }
        dragging = true;
        if let (Some(at), Some(was)) = (cursor, ui.drag_at) {
            let d = at - was;
            ui.moved[slot] += d;
            if let (Val::Px(x), Val::Px(y)) = (node.left, node.top) {
                (node.left, node.top) = (Val::Px(x + d.x), Val::Px(y + d.y));
            }
        }
    }
    ui.drag_at = cursor.filter(|_| dragging);
    let dt = time.delta_secs();
    for f in [Field::Name, Field::Number] {
        let focused = ui.focus == Some(f);
        let field = ui.field(f);
        textinput::tick_caret(field, focused, dt);
        let parts = boxes.iter_mut().filter(|(.., t)| t.0 == f);
        paint_glue_field(field, focused, parts.map(|(p, t, v, _)| (p, t, v)));
    }
}

/// The icons as loaded once, from the install by the asset server.
struct Icons<'a>(
    &'a AssetServer,
    &'a mut HashMap<&'static str, Handle<Image>>,
);

impl Icons<'_> {
    fn get(&mut self, path: &'static str) -> Handle<Image> {
        let server = self.0;
        let load = || {
            let url = format!("mpq://{}.blp", path.replace('\\', "/"));
            server.load_with_settings(url, |s: &mut BlpLoaderSettings| {
                s.variant = BlpVariant::Sprite;
            })
        };
        self.1.entry(path).or_insert_with(load).clone()
    }
}

/// The fonts: `FRIZQT`, and the fields' `ARIALN`.
type Fonts<'a> = (&'a Handle<Font>, &'a Handle<Font>);

/// The panels for `built`: while editing, Exit, Test lap and Save, the hint, the bars and the
/// keys card; in a test lap, the lap's panel.
fn spawn(
    commands: &mut Commands,
    art: &GlueArt,
    icons: &mut Icons,
    (font, edit_font): Fonts,
    built: &Built,
    moved: [Vec2; 2],
) -> Entity {
    let s = built.s;
    let px = |v: f32| Val::Px(v * s);
    let view = &built.view;
    // A dragged card stays on the screen, its offset from the screen's left and middle.
    let card = |slot: usize, (w, h): (f32, f32), below_middle: f32| {
        let at = Vec2::new(20.0 * s, built.window.y * 0.5 - below_middle * s) + moved[slot];
        let max = (built.window - Vec2::new(w, h) * s).max(Vec2::ZERO);
        let at = at.clamp(Vec2::ZERO, max);
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(at.x),
            top: Val::Px(at.y),
            width: px(w),
            height: px(h),
            ..default()
        }
    };
    let root = (
        Name::new("track editor"),
        GlobalZIndex(Z),
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
    );
    commands
        .spawn(root)
        .with_children(|root| {
            if view.state != "edit" {
                // A test lap: back to the editor (out of a saved track's lap), and the lap again.
                let (back, back_icon) = match view.state {
                    "run" => ("End test lap", icon::EXIT),
                    _ => ("Back to editor", icon::BACK),
                };
                panel(root, art, Panel::Lap, card(1, (120.0, 80.0), 120.0), s).with_children(|p| {
                    label(p, font, "Test lap", FONT, GOLD, at(s, 12.0, 10.0), s);
                    row(p, s, at(s, 12.0, 30.0), |r| {
                        let back_act = Act::Cmd(&["back"]);
                        icon_button(r, icons, font, s, back_icon, back, back_act, false);
                        let again = Act::Cmd(&["restart"]);
                        icon_button(r, icons, font, s, icon::RESTART, "Restart", again, false);
                    });
                });
                return;
            }
            // The corners: Exit, and Test lap and Save.
            let corner = |left: bool| Node {
                position_type: PositionType::Absolute,
                top: px(16.0),
                left: if left { px(16.0) } else { Val::Auto },
                right: if left { Val::Auto } else { px(16.0) },
                column_gap: px(8.0),
                ..default()
            };
            root.spawn((EnginePanel, corner(true))).with_children(|c| {
                labelled_button(c, icons, font, s, icon::EXIT, "Exit", Act::Cmd(&["close"]));
            });
            root.spawn((EnginePanel, corner(false))).with_children(|c| {
                labelled_button(c, icons, font, s, icon::TEST, "Test lap", Act::Practice);
                labelled_button(c, icons, font, s, icon::SAVE, "Save", Act::Save);
            });
            // What a click does now, and the last word for the player.
            let status = Node {
                position_type: PositionType::Absolute,
                top: px(20.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: px(4.0),
                ..default()
            };
            root.spawn(status).with_children(|st| {
                label(st, font, &built.hint, FONT, GOLD, Node::default(), s);
                if let Some(note) = &built.note {
                    label(st, font, note, FONT, Color::WHITE, Node::default(), s);
                }
            });
            // The bars along the bottom, each named.
            let bars = Node {
                position_type: PositionType::Absolute,
                bottom: px(24.0),
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::FlexEnd,
                column_gap: px(12.0),
                ..default()
            };
            root.spawn(bars).with_children(|bars| {
                let tool = |t: &str| view.tool == t;
                bar(bars, art, font, s, "Editor controls", |b| {
                    row(b, s, Node::default(), |r| {
                        let move_ = Act::Cmd(&["move"]);
                        icon_button(r, icons, font, s, icon::MOVE, "Move", move_, tool("move"));
                        let undo = Act::Cmd(&["undo"]);
                        icon_button(r, icons, font, s, icon::UNDO, "Undo", undo, false);
                        let redo = Act::Cmd(&["redo"]);
                        icon_button(r, icons, font, s, icon::REDO, "Redo", redo, false);
                        let open = built.objects.is_some();
                        let objects = Act::Objects;
                        icon_button(r, icons, font, s, icon::OBJECTS, "Objects", objects, open);
                    });
                });
                bar(bars, art, font, s, "Checkpoints", |b| {
                    let count = match view.gates {
                        0 => "|cffff8040No start yet|r".to_string(),
                        n => format!("Start + {} checkpoints", n - 1),
                    };
                    label(b, font, &count, SMALL, Color::WHITE, Node::default(), s);
                    row(b, s, Node::default(), |r| {
                        let (place, number) = (Act::Cmd(&["place"]), Act::Cmd(&["number"]));
                        let lit = tool("checkpoint");
                        icon_button(
                            r,
                            icons,
                            font,
                            s,
                            icon::PLACE,
                            "Place checkpoint",
                            place,
                            lit,
                        );
                        let lit = tool("number");
                        let name = "Edit checkpoint";
                        icon_button(r, icons, font, s, icon::NUMBER, name, number, lit);
                        let grid = Act::Cmd(&["grid"]);
                        icon_button(
                            r,
                            icons,
                            font,
                            s,
                            icon::GRID,
                            "Place grid",
                            grid,
                            tool("grid"),
                        );
                        let erase = Act::Cmd(&["erase"]);
                        let lit = tool("erase");
                        let name = "Delete checkpoint";
                        icon_button(r, icons, font, s, icon::ERASE, name, erase, lit);
                    });
                    let grid = match view.grid {
                        true => "Grid placed",
                        false => "|cff999999No grid: a flying start|r",
                    };
                    label(b, font, grid, SMALL, Color::WHITE, Node::default(), s);
                    if view.tool == "number" && view.picked > 0 {
                        row(b, s, Node::default(), |r| {
                            let at = Node {
                                align_self: AlignSelf::Center,
                                ..default()
                            };
                            label(r, font, "New number", SMALL, GOLD, at, s);
                            field(r, art, edit_font, Field::Number, (50.0, 28.0), s);
                        });
                    }
                });
                bar(bars, art, font, s, "Race type", |b| {
                    row(b, s, Node::default(), |r| {
                        let laps = |n: u8| view.laps == n;
                        let sprint = Act::Cmd(&["laps", "0"]);
                        icon_button(r, icons, font, s, icon::SPRINT, "Sprint", sprint, laps(0));
                        let circuits: [(u8, &str, &'static [&'static str]); 2] =
                            [(1, "1 lap", &["laps", "1"]), (3, "3 laps", &["laps", "3"])];
                        for (n, name, words) in circuits {
                            let act = Act::Cmd(words);
                            let b = icon_button(r, icons, font, s, icon::LAPS, name, act, laps(n));
                            // The laps over the icon, as a stack count.
                            r.commands().entity(b).with_children(|b| {
                                let corner = Node {
                                    position_type: PositionType::Absolute,
                                    right: px(3.0),
                                    bottom: px(1.0),
                                    ..default()
                                };
                                label(b, font, &n.to_string(), TITLE, Color::WHITE, corner, s);
                            });
                        }
                    });
                    // The karts' size on the track: a small one makes the world drive bigger.
                    row(b, s, Node::default(), |r| {
                        for (scale, name, words) in SIZES {
                            let lit = (view.scale - scale).abs() < 0.005;
                            let b = icon_button(
                                r,
                                icons,
                                font,
                                s,
                                icon::SIZE,
                                name,
                                Act::Cmd(words),
                                lit,
                            );
                            r.commands().entity(b).with_children(|b| {
                                let corner = Node {
                                    position_type: PositionType::Absolute,
                                    right: px(2.0),
                                    bottom: px(1.0),
                                    ..default()
                                };
                                let short = name.trim_start_matches("Size ");
                                label(b, font, short, SMALL, Color::WHITE, corner, s);
                            });
                        }
                    });
                });
            });
            if let Some(objects) = &built.objects {
                objects_panel(root, art, font, s, objects, view.object);
            }
            panel(root, art, Panel::Keys, card(0, (260.0, 204.0), 182.0), s).with_children(|k| {
                label(
                    k,
                    font,
                    "Track Editor keys",
                    FONT,
                    GOLD,
                    at(s, 12.0, 10.0),
                    s,
                );
                let mut text = at(s, 12.0, 30.0);
                text.width = px(236.0);
                label(k, font, &built.keys, SMALL, Color::WHITE, text, s);
            });
        })
        .id()
}

/// The objects panel over the bars: its tabs, and the open tab's kinds as tiles of icon and name,
/// the one a click places lit.
fn objects_panel(
    root: &mut ChildSpawnerCommands,
    art: &GlueArt,
    font: &Handle<Font>,
    s: f32,
    objects: &ObjectsPanel,
    picked: Option<&'static str>,
) {
    let px = |v: f32| Val::Px(v * s);
    let over_bars = Node {
        position_type: PositionType::Absolute,
        bottom: px(150.0),
        left: Val::Px(0.0),
        right: Val::Px(0.0),
        justify_content: JustifyContent::Center,
        ..default()
    };
    root.spawn(over_bars).with_children(|w| {
        let node = Node {
            width: px(540.0),
            flex_direction: FlexDirection::Column,
            padding: UiRect::all(px(12.0)),
            row_gap: px(8.0),
            ..default()
        };
        panel(w, art, Panel::Bar, node, s).with_children(|p| {
            label(p, font, "Objects", FONT, GOLD, Node::default(), s);
            row(p, s, Node::default(), |r| {
                for group in object_groups() {
                    let lit = group == objects.tab;
                    text_button(r, font, s, group, Act::Tab(group), lit);
                }
            });
            let grid = Node {
                flex_wrap: FlexWrap::Wrap,
                column_gap: px(8.0),
                row_gap: px(8.0),
                ..default()
            };
            p.spawn(grid).with_children(|g| {
                for (key, name, icon) in &objects.tiles {
                    tile(g, font, s, key, name, icon.clone(), picked == Some(*key));
                }
            });
        });
    });
}

/// A tab's button: its name in a frame, lit while open.
fn text_button(
    parent: &mut ChildSpawnerCommands,
    font: &Handle<Font>,
    s: f32,
    caption: &str,
    act: Act,
    lit: bool,
) {
    let px = |v: f32| Val::Px(v * s);
    let node = Node {
        padding: UiRect::axes(px(10.0), px(4.0)),
        border: UiRect::all(px(2.0)),
        ..default()
    };
    let looks = (
        BorderColor::all(if lit { GOLD } else { FRAME }),
        BackgroundColor(FILL),
    );
    parent
        .spawn((EditorBtn(act), Button, Lit(lit), node, looks))
        .with_children(|b| label(b, font, caption, FONT, GOLD, Node::default(), s));
}

/// A kind's tile: its icon (a dark square until drawn) and its name under it.
fn tile(
    parent: &mut ChildSpawnerCommands,
    font: &Handle<Font>,
    s: f32,
    key: &'static str,
    name: &str,
    icon: Option<Handle<Image>>,
    lit: bool,
) {
    let px = |v: f32| Val::Px(v * s);
    let node = Node {
        width: px(92.0),
        flex_direction: FlexDirection::Column,
        align_items: AlignItems::Center,
        padding: UiRect::all(px(4.0)),
        row_gap: px(4.0),
        border: UiRect::all(px(2.0)),
        ..default()
    };
    let looks = (
        BorderColor::all(if lit { GOLD } else { FRAME }),
        BackgroundColor(Color::BLACK.with_alpha(0.45)),
    );
    let act = Act::Object(key);
    let mut b = parent.spawn((EditorBtn(act), Button, Lit(lit), node, looks));
    b.with_children(|b| {
        let face = Node {
            width: px(72.0),
            height: px(72.0),
            ..default()
        };
        match icon {
            Some(icon) => b.spawn((ImageNode::new(icon), face)),
            None => b.spawn((BackgroundColor(FRAME), face)),
        };
        let caption = Node {
            max_width: px(84.0),
            ..default()
        };
        label(b, font, name, SMALL, Color::WHITE, caption, s);
    });
}

/// The numbers of the checkpoints `of` (0 the start, numbered 1), each in its own box for
/// [`draw`] to move over its marker; the boxes' root and the boxes.
fn spawn_numbers(
    commands: &mut Commands,
    font: &Handle<Font>,
    of: &[usize],
    s: f32,
) -> (Entity, Vec<Entity>) {
    let mut labels = Vec::new();
    let root = (
        Name::new("track editor numbers"),
        GlobalZIndex(Z - 1),
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
    );
    let root = commands
        .spawn(root)
        .with_children(|r| {
            for &i in of {
                let boxed = Node {
                    position_type: PositionType::Absolute,
                    width: Val::Px(40.0 * s),
                    height: Val::Px(24.0 * s),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    display: Display::None,
                    ..default()
                };
                let color = if i == 0 { START_GREEN } else { Color::WHITE };
                let face = TextFont {
                    font: font.clone(),
                    font_size: 18.0 * s,
                    ..default()
                };
                let shadow = TextShadow {
                    offset: Vec2::splat(s),
                    color: Color::BLACK,
                };
                let number = (
                    Text::new((i + 1).to_string()),
                    face,
                    TextColor(color),
                    shadow,
                );
                labels.push(r.spawn((GateNumber, boxed)).with_child(number).id());
            }
        })
        .id();
    (root, labels)
}

/// The popup over a dimmed screen, which takes every click from the world and the panels.
fn spawn_popup(
    commands: &mut Commands,
    art: &GlueArt,
    icons: &mut Icons,
    (font, edit_font): Fonts,
    popup: &Popup,
    s: f32,
) -> Entity {
    let px = |v: f32| Val::Px(v * s);
    let screen = (
        Name::new("track editor popup"),
        GlobalZIndex(POPUP_Z),
        EnginePanel,
        Interaction::default(),
        FocusPolicy::Block,
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.35)),
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
    );
    let boxed = Node {
        width: px(380.0),
        flex_direction: FlexDirection::Column,
        align_items: AlignItems::Center,
        padding: UiRect::all(px(18.0)),
        row_gap: px(12.0),
        ..default()
    };
    let wrapped = || Node {
        max_width: px(340.0),
        ..default()
    };
    commands
        .spawn(screen)
        .with_children(|screen| {
            let mut b = screen.spawn(boxed);
            backdrop(&mut b, art, s);
            b.with_children(|b| match popup {
                Popup::Error(why) => {
                    label(
                        b,
                        font,
                        "That cannot be done",
                        TITLE,
                        WARN,
                        Node::default(),
                        s,
                    );
                    label(b, font, why, FONT, Color::WHITE, wrapped(), s);
                    labelled_button(b, icons, font, s, icon::OK, "OK", Act::Dismiss);
                }
                Popup::Save { replaces } => {
                    label(b, font, "Save track", TITLE, GOLD, Node::default(), s);
                    label(b, font, "Track name", FONT, GOLD, Node::default(), s);
                    field(b, art, edit_font, Field::Name, (300.0, 30.0), s);
                    if *replaces {
                        let warn = "A track of yours has this name already: saving replaces it.";
                        label(b, font, warn, FONT, WARN, wrapped(), s);
                    }
                    row(b, s, Node::default(), |r| {
                        let caption = if *replaces { "Overwrite" } else { "Save" };
                        labelled_button(r, icons, font, s, icon::SAVE, caption, Act::SaveConfirm);
                        labelled_button(r, icons, font, s, icon::EXIT, "Cancel", Act::Cancel);
                    });
                }
            });
        })
        .id()
}

/// An absolute node at `(left, top)` in the panel's units.
fn at(s: f32, left: f32, top: f32) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: Val::Px(left * s),
        top: Val::Px(top * s),
        ..default()
    }
}

/// The tooltip backdrop behind a box.
fn backdrop(b: &mut EntityCommands, art: &GlueArt, s: f32) {
    b.with_children(|b| match (&art.tooltip_bg, &art.tooltip_border) {
        (Some(bg), Some(border)) => {
            let inset = Val::Px(4.0 * s);
            b.spawn((
                tiled_bg_node(bg.clone(), 16.0, s, FILL),
                Node {
                    position_type: PositionType::Absolute,
                    left: inset,
                    right: inset,
                    top: inset,
                    bottom: inset,
                    ..default()
                },
            ));
            backdrop_border(b, border, 16.0, EDGE);
        }
        _ => {
            b.spawn((BackgroundColor(FILL), overlay()));
        }
    });
}

/// A panel at `node` on the tooltip backdrop, which takes the pointer from the world.
fn panel<'a>(
    parent: &'a mut ChildSpawnerCommands,
    art: &GlueArt,
    which: Panel,
    node: Node,
    s: f32,
) -> EntityCommands<'a> {
    let mut p = parent.spawn((which, EnginePanel, Interaction::default(), node));
    backdrop(&mut p, art, s);
    p
}

/// A named bar of buttons.
fn bar(
    parent: &mut ChildSpawnerCommands,
    art: &GlueArt,
    font: &Handle<Font>,
    s: f32,
    name: &str,
    children: impl FnOnce(&mut ChildSpawnerCommands),
) {
    let node = Node {
        flex_direction: FlexDirection::Column,
        padding: UiRect::axes(Val::Px(12.0 * s), Val::Px(10.0 * s)),
        row_gap: Val::Px(6.0 * s),
        ..default()
    };
    panel(parent, art, Panel::Bar, node, s).with_children(|b| {
        label(b, font, name, FONT, GOLD, Node::default(), s);
        children(b);
    });
}

/// A row of buttons at `node`.
fn row(
    parent: &mut ChildSpawnerCommands,
    s: f32,
    mut node: Node,
    children: impl FnOnce(&mut ChildSpawnerCommands),
) {
    node.column_gap = Val::Px(6.0 * s);
    parent.spawn(node).with_children(children);
}

/// An icon button doing `act`, lit while what it picks is picked, its `name` over it on hover.
#[allow(clippy::too_many_arguments)]
fn icon_button(
    parent: &mut ChildSpawnerCommands,
    icons: &mut Icons,
    font: &Handle<Font>,
    s: f32,
    path: &'static str,
    name: &str,
    act: Act,
    lit: bool,
) -> Entity {
    let px = |v: f32| Val::Px(v * s);
    let node = Node {
        width: px(ICON),
        height: px(ICON),
        border: UiRect::all(px(2.0)),
        ..default()
    };
    let frame = BorderColor::all(if lit { GOLD } else { FRAME });
    let mut b = parent.spawn((EditorBtn(act), Button, Lit(lit), node, frame));
    b.insert(BackgroundColor(Color::BLACK));
    b.with_children(|b| {
        let face = Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        };
        b.spawn((ImageNode::new(icons.get(path)), face));
        let tip = Node {
            position_type: PositionType::Absolute,
            bottom: Val::Percent(100.0),
            left: px(-2.0),
            margin: UiRect::bottom(px(6.0)),
            padding: UiRect::axes(px(6.0), px(3.0)),
            border: UiRect::all(px(1.0)),
            ..default()
        };
        let looks = (BackgroundColor(FILL), BorderColor::all(EDGE));
        let mut tip = b.spawn((Tip, Visibility::Hidden, GlobalZIndex(TIP_Z), tip, looks));
        tip.with_children(|t| {
            let face = TextFont {
                font: font.clone(),
                font_size: FONT * s,
                ..default()
            };
            let one_line = TextLayout::new_with_no_wrap();
            t.spawn((Text::new(name), face, TextColor(Color::WHITE), one_line));
        });
    });
    b.id()
}

/// A button with its icon and its name beside it, doing `act`.
fn labelled_button(
    parent: &mut ChildSpawnerCommands,
    icons: &mut Icons,
    font: &Handle<Font>,
    s: f32,
    path: &'static str,
    name: &str,
    act: Act,
) {
    let px = |v: f32| Val::Px(v * s);
    let node = Node {
        align_items: AlignItems::Center,
        column_gap: px(6.0),
        padding: UiRect::new(px(4.0), px(10.0), px(4.0), px(4.0)),
        border: UiRect::all(px(2.0)),
        ..default()
    };
    let looks = (BorderColor::all(FRAME), BackgroundColor(FILL));
    let mut b = parent.spawn((EditorBtn(act), Button, Lit(false), node, looks));
    b.with_children(|b| {
        let face = Node {
            width: px(SMALL_ICON),
            height: px(SMALL_ICON),
            ..default()
        };
        b.spawn((ImageNode::new(icons.get(path)), face));
        label(b, font, name, FONT, GOLD, Node::default(), s);
    });
}

/// A string at `node`, its `|c…|r` colours decoded, with a drop shadow as the game fonts have.
fn label(
    parent: &mut ChildSpawnerCommands,
    font: &Handle<Font>,
    text: &str,
    size: f32,
    color: Color,
    node: Node,
    s: f32,
) {
    let face = TextFont {
        font: font.clone(),
        font_size: size * s,
        ..default()
    };
    let shadow = TextShadow {
        offset: Vec2::splat(s),
        color: Color::BLACK,
    };
    parent
        .spawn((Text::default(), face.clone(), shadow, node))
        .with_children(|t| {
            for (span, color) in markup_spans(text, color, true) {
                t.spawn((TextSpan::new(span), face.clone(), TextColor(color)));
            }
        });
}

/// A text field of `size`, its row marked for [`draw`] to paint.
fn field(
    parent: &mut ChildSpawnerCommands,
    art: &GlueArt,
    font: &Handle<Font>,
    f: Field,
    size: (f32, f32),
    s: f32,
) {
    let extras = (f, Interaction::default());
    let colors = (EDGE, FILL);
    glue_edit_box(
        parent,
        art,
        font,
        extras,
        FieldText(f),
        size,
        colors,
        (8.0, 4.0, 0.0, 4.0),
        s,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kart::objects::KINDS;

    fn view(state: &'static str, tool: &'static str, picked: usize) -> EditView {
        EditView {
            state,
            gates: 3,
            grid: false,
            laps: 3,
            scale: 1.0,
            notes: 0,
            tool,
            picked,
            object: None,
            turns_object: false,
        }
    }

    /// What a built tree holds: its panels, its fields, each button's act and whether it came
    /// lit, and the names the icons show on hover.
    struct Tree {
        panels: Vec<Panel>,
        fields: Vec<Field>,
        buttons: Vec<(Act, bool)>,
        tips: Vec<String>,
    }

    fn tree(spawn_it: impl FnOnce(&mut Commands, &mut Icons) -> Entity) -> Tree {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()));
        app.init_asset::<Image>();
        let server = app.world().resource::<AssetServer>().clone();
        let world = app.world_mut();
        let mut cache = HashMap::new();
        spawn_it(&mut world.commands(), &mut Icons(&server, &mut cache));
        world.flush();
        let mut buttons = world.query::<(&EditorBtn, &Lit)>();
        let buttons = buttons.iter(world).map(|(b, l)| (b.0, l.0)).collect();
        let mut tips = world.query_filtered::<&Children, With<Tip>>();
        let tips: Vec<Entity> = tips.iter(world).map(|c| c[0]).collect();
        let tips = tips
            .iter()
            .map(|t| world.get::<Text>(*t).unwrap().0.clone())
            .collect();
        Tree {
            panels: world.query::<&Panel>().iter(world).copied().collect(),
            fields: world.query::<&Field>().iter(world).copied().collect(),
            buttons,
            tips,
        }
    }

    fn build(view: EditView) -> Tree {
        let built = Built {
            view,
            hint: String::new(),
            note: None,
            keys: String::new(),
            s: 1.0,
            window: Vec2::new(1024.0, 768.0),
            art: false,
            objects: None,
        };
        let font = Handle::default();
        let art = GlueArt::default();
        tree(|c, icons| spawn(c, &art, icons, (&font, &font), &built, [Vec2::ZERO; 2]))
    }

    fn lit(tree: &Tree) -> Vec<Act> {
        tree.buttons
            .iter()
            .filter(|(_, l)| *l)
            .map(|(a, _)| *a)
            .collect()
    }

    fn acts(tree: &Tree) -> Vec<Act> {
        tree.buttons.iter().map(|(a, _)| *a).collect()
    }

    #[test]
    fn editing_shows_exit_save_and_the_named_bars_the_tool_and_laps_lit() {
        let t = build(view("edit", "checkpoint", 0));
        let bars = t.panels.iter().filter(|p| **p == Panel::Bar).count();
        assert_eq!(bars, 3, "editor controls, checkpoints, race type");
        assert!(t.panels.contains(&Panel::Keys) && !t.panels.contains(&Panel::Lap));
        assert!(
            t.fields.is_empty(),
            "no number box without a pick, the name in the popup"
        );
        assert_eq!(
            lit(&t),
            [
                Act::Cmd(&["place"]),
                Act::Cmd(&["laps", "3"]),
                Act::Cmd(&["scale", "1"])
            ]
        );
        for act in [
            Act::Cmd(&["close"]),
            Act::Practice,
            Act::Save,
            Act::Cmd(&["grid"]),
        ] {
            assert!(acts(&t).contains(&act), "{act:?}");
        }
        // Every icon names itself on hover; Exit, Test lap and Save say it beside theirs.
        let named = [
            "Move",
            "Undo",
            "Redo",
            "Objects",
            "Place checkpoint",
            "Edit checkpoint",
            "Place grid",
            "Delete checkpoint",
            "Sprint",
            "1 lap",
            "3 laps",
            "Size x1",
            "Size x0.75",
            "Size x0.5",
            "Size x0.35",
        ];
        let mut tips = t.tips.clone();
        tips.sort();
        let mut want = named.map(String::from).to_vec();
        want.sort();
        assert_eq!(tips, want);
        assert_eq!(t.buttons.len(), named.len() + 3);
    }

    #[test]
    fn each_checkpoint_has_its_number_the_start_green() {
        let mut world = World::new();
        let font = Handle::default();
        let (_, labels) = spawn_numbers(&mut world.commands(), &font, &[0, 1, 4], 1.0);
        world.flush();
        let numbers: Vec<(String, Color)> = labels
            .iter()
            .map(|l| {
                let text = world.get::<Children>(*l).unwrap()[0];
                let color = world.get::<TextColor>(text).unwrap().0;
                (world.get::<Text>(text).unwrap().0.clone(), color)
            })
            .collect();
        let white = Color::WHITE;
        assert_eq!(
            numbers,
            [
                ("1".into(), START_GREEN),
                ("2".into(), white),
                ("5".into(), white)
            ]
        );
    }

    #[test]
    fn the_objects_panel_has_its_tabs_and_each_kind_as_a_tile_the_picked_one_lit() {
        let mut v = view("edit", "object", 0);
        v.object = Some(KINDS[2].key);
        let built = Built {
            view: v,
            hint: String::new(),
            note: None,
            keys: String::new(),
            s: 1.0,
            window: Vec2::new(1024.0, 768.0),
            art: false,
            objects: Some(ObjectsPanel {
                tab: "Ramps",
                tiles: KINDS.iter().map(|k| (k.key, k.name, None)).collect(),
            }),
        };
        let font = Handle::default();
        let art = GlueArt::default();
        let t = tree(|c, icons| spawn(c, &art, icons, (&font, &font), &built, [Vec2::ZERO; 2]));
        assert!(
            t.buttons.contains(&(Act::Objects, true)),
            "its button, lit while open"
        );
        assert!(t.buttons.contains(&(Act::Tab("Ramps"), true)));
        let tiles: Vec<(Act, bool)> = (t.buttons.iter())
            .filter(|(a, _)| matches!(a, Act::Object(_)))
            .copied()
            .collect();
        assert_eq!(tiles.len(), KINDS.len());
        assert_eq!(lit_tiles(&tiles), [Act::Object(KINDS[2].key)]);
    }

    #[test]
    fn escape_closes_the_objects_panel_then_drops_the_object_picked() {
        let mut ui = EditorUi {
            objects_open: true,
            ..default()
        };
        assert!(matches!(ui.escape(true), (true, None)), "the panel first");
        assert!(!ui.objects_open);
        let (ate, cmd) = ui.escape(true);
        assert!(ate);
        assert_eq!(cmd.expect("back to the move cursor").0, ["move"]);
        assert!(
            matches!(ui.escape(false), (false, None)),
            "nothing: the game's menu"
        );
    }

    fn lit_tiles(tiles: &[(Act, bool)]) -> Vec<Act> {
        tiles.iter().filter(|(_, l)| *l).map(|(a, _)| *a).collect()
    }

    #[test]
    fn a_pick_shows_the_number_box() {
        let t = build(view("edit", "number", 2));
        assert_eq!(t.fields, [Field::Number]);
        assert_eq!(
            lit(&t),
            [
                Act::Cmd(&["number"]),
                Act::Cmd(&["laps", "3"]),
                Act::Cmd(&["scale", "1"])
            ]
        );
    }

    #[test]
    fn a_test_lap_has_its_own_panel_alone() {
        for (state, back) in [("test", "Back to editor"), ("run", "End test lap")] {
            let t = build(view(state, "move", 0));
            assert_eq!(t.panels, [Panel::Lap], "{state}");
            assert!(t.fields.is_empty());
            assert_eq!(acts(&t), [Act::Cmd(&["back"]), Act::Cmd(&["restart"])]);
            assert_eq!(t.tips, [back, "Restart"]);
        }
    }

    #[test]
    fn the_popups_say_why_and_warn_of_a_save_that_replaces() {
        let font = Handle::default();
        let art = GlueArt::default();
        let popup =
            |p: Popup| tree(|c, icons| spawn_popup(c, &art, icons, (&font, &font), &p, 1.0));
        let t = popup(Popup::Error("Place the start first.".into()));
        assert_eq!(acts(&t), [Act::Dismiss]);
        let t = popup(Popup::Save { replaces: true });
        assert_eq!(t.fields, [Field::Name]);
        assert_eq!(acts(&t), [Act::SaveConfirm, Act::Cancel]);
    }

    #[test]
    fn a_pick_takes_the_keys_and_enter_renumbers_it() {
        let mut ui = EditorUi::default();
        ui.follow(&view("edit", "number", 2), ("", 0));
        assert_eq!(ui.focus, Some(Field::Number));
        ui.number.insert("3");
        let cmd = ui.end_field(true).expect("a renumber");
        assert_eq!(cmd.0, ["renumber", "3"]);
        assert_eq!(ui.focus, None);
        // Escape, or a number that is none, renumbers nothing; the pick gone lets the keys go.
        ui.focus = Some(Field::Number);
        ui.number.insert("3");
        assert!(ui.end_field(false).is_none());
        ui.focus = Some(Field::Number);
        assert!(ui.end_field(true).is_none(), "an empty box");
        ui.focus = Some(Field::Number);
        ui.follow(&view("edit", "number", 0), ("", 0));
        assert_eq!(ui.focus, None);
    }

    #[test]
    fn a_new_error_pops_up_once_and_outlives_the_editor() {
        let mut ui = EditorUi::default();
        let editing = view("edit", "move", 0);
        ui.follow(&editing, ("Place the start first.", 1));
        assert_eq!(
            ui.popup,
            Some(Popup::Error("Place the start first.".into()))
        );
        assert!(ui.answer(false, "").is_none());
        ui.follow(&editing, ("Place the start first.", 1));
        assert_eq!(ui.popup, None, "the same error once");
        ui.follow(&editing, ("That track is not saved any more.", 2));
        ui.follow(
            &view("", "move", 0),
            ("That track is not saved any more.", 2),
        );
        assert!(ui.popup.is_some(), "an error outlives the editor");
    }

    #[test]
    fn the_save_popup_takes_the_name_and_saves_over_on_a_yes() {
        let mut ui = EditorUi::default();
        assert_eq!(ui.track_name(), DEFAULT_NAME);
        ui.open_save(false);
        assert_eq!(ui.focus, Some(Field::Name));
        ui.name.insert("Loop");
        let cmd = ui.answer(true, "Durotar").expect("a save");
        assert_eq!(cmd.0, ["save", "Loop", "Durotar", "overwrite"]);
        assert_eq!((ui.popup.clone(), ui.focus), (None, None));
        // Cancelled, nothing is saved; the name stays for the next save and the test lap.
        ui.open_save(true);
        assert!(ui.answer(false, "Durotar").is_none());
        assert_eq!(ui.track_name(), "Loop");
        ui.name.set_text("  ");
        assert_eq!(ui.track_name(), DEFAULT_NAME, "an empty box");
        // Closing the editor takes the save popup away.
        ui.open_save(false);
        ui.follow(&view("", "move", 0), ("", 0));
        assert_eq!((ui.popup.clone(), ui.focus), (None, None));
    }

    #[test]
    fn the_keys_card_names_the_keys_bound_now() {
        let bound = |c: &str, k: &str| (c.to_string(), vec![k.to_string(), "PAD1".to_string()]);
        let text = keys_text(&[
            bound("KARTCRAFT_ACCELERATE", "W"),
            bound("KARTCRAFT_BRAKE", "S"),
            bound("KARTCRAFT_BOOST", "SHIFT"),
        ]);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "|cffffd100W / S|r  fly forward, back");
        assert_eq!(lines[1], "|cffffd100unbound / unbound|r  fly left, right");
        assert_eq!(lines[3], "|cffffd100SHIFT|r  held: fly fast");
        assert_eq!(lines.len(), 14);
    }

    #[test]
    fn every_icon_is_in_the_install() {
        let data = benilla_formats::wow_data_or_skip!();
        let chain = benilla_formats::open_chain(&data).expect("the install's archives");
        let missing: Vec<&str> = icon::ALL
            .into_iter()
            .filter(|p| !chain.contains(&format!("{p}.blp")))
            .collect();
        assert!(missing.is_empty(), "not in the install: {missing:?}");
    }
}
