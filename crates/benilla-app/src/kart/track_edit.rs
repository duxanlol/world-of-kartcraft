//! The track editor: the minimap's track button (`KartTrackEdit`) flies the free camera, and the
//! editor's panels ([`super::track_edit_ui`], each press a [`TrackCmd`]) place the start, the
//! checkpoints and the starting grid where the camera aims, drives the draft as a test lap
//! ([`Active::practice`]) and saves it as a `courses/courses.json` course in
//! `benilla-config/kart-tracks/` (docs/courses.md). The saved tracks go to the track browser
//! (`BENILLA_KART_MYTRACKS`), which runs a test lap of one.

use benilla_assets::coords::{bevy_to_wow, wow_to_bevy};
use benilla_world::collision::WorldCollision;
use bevy::prelude::*;

use super::freecam::{FreeCam, FreeMode};
use super::objects::{kinds, Placed};
use super::race::{Active, Course, Races, COURSES, GATE_HALF_WIDTH};

/// How far the camera's aim reaches for the ground (yards).
const AIM_REACH: f32 = 600.0;
/// How far a pixel of the mouse turns the gate about to be placed, middle button held (radians),
/// and how fast Q and E held turn it (radians a second).
const TURN_PER_PIXEL: f32 = 0.01;
const TURN_PER_SEC: f32 = 1.5;
/// A gate's half-width as the mouse wheel sets it (yards a notch), between a kart's squeeze and
/// the server's reach: it takes a checkpoint within 45 yards (`Kartcraft.cpp` `CHECKPOINT_RADIUS`),
/// so a wider gate would pass on the client and not the server.
const WIDTH_PER_NOTCH: f32 = 2.0;
const WIDTH_MIN: f32 = 2.0;
const WIDTH_MAX: f32 = 45.0;
/// Edit's trip to a track's start: how near the kart lands (yards), and how long it may take
/// (the kart's grace through a test lap's teleport, `race.rs`).
const TRIP_NEAR: f32 = 30.0;
const TRIP_SECS: f32 = 10.0;
/// A saved track's note, for whoever adds it to courses.json.
const NOTE: &str = "Made with the in-game track editor.";
/// The aim's colour, the grid's, and what the move cursor is over or holds.
const AIM_COLOR: Color = Color::srgb(1.0, 0.85, 0.2);
const GRID_COLOR: Color = Color::srgb(0.3, 1.0, 0.4);
/// Where a snapped object meets another.
const SNAP_COLOR: Color = Color::srgb(0.3, 0.8, 1.0);
const HOVER_COLOR: Color = Color::srgb(0.4, 0.8, 1.0);
const ERASE_COLOR: Color = Color::srgb(1.0, 0.3, 0.25);
/// A pinned object's axes (yards each way) and the ring the mouse turns it about (yards), in the
/// usual colours: yaw green, pitch red, roll blue.
const AXIS_LENGTH: f32 = 4.0;
/// The keys an object turns on, held from the game's own bindings while it does: yaw, pitch,
/// roll, each pair.
/// The keys that size a build piece, held or about to be placed: `[` `]` its length, `;` `'` its
/// width, `-` `=` its height, each pair shorter then longer.
pub(crate) const SIZE_KEYS: [KeyCode; 6] = [
    KeyCode::BracketLeft,
    KeyCode::BracketRight,
    KeyCode::Semicolon,
    KeyCode::Quote,
    KeyCode::Minus,
    KeyCode::Equal,
];
/// How fast a size key held grows or shrinks a piece (yards a second).
const SIZE_PER_SEC: f32 = 6.0;

pub(crate) const TURN_KEYS: [KeyCode; 6] = [
    KeyCode::KeyQ,
    KeyCode::KeyE,
    KeyCode::KeyR,
    KeyCode::KeyF,
    KeyCode::KeyZ,
    KeyCode::KeyC,
];
/// How far a pixel of the mouse, Ctrl held, raises an object (yards).
const LIFT_PER_PIXEL: f32 = 0.04;
const AXIS_RING: f32 = 2.5;
const YAW_COLOR: Color = Color::srgb(0.3, 1.0, 0.35);
const PITCH_COLOR: Color = Color::srgb(1.0, 0.3, 0.3);
const ROLL_COLOR: Color = Color::srgb(0.35, 0.55, 1.0);
/// How near the grid's spot the move cursor takes it (yards): its circle.
const GRID_PICK: f32 = 6.0;
/// Each checkpoint's marker while editing, as big as the cursor's own circle (yards), and how
/// near it the move cursor takes the checkpoint; the marker's pole, seen from afar.
const MARKER: f32 = 2.0;
const MARKER_PICK: f32 = 3.0;
const MARKER_POLE: f32 = 6.0;
/// The start's marker, and the checkpoints': see-through, so the ground shows under them.
const START_COLOR: Color = Color::srgba(0.3, 1.0, 0.4, 0.6);
const MARKER_COLOR: Color = Color::srgba(1.0, 0.9, 0.3, 0.6);

/// A starting grid: its first slot (WoW x, y, z) and the way it faces (radians, WoW's).
#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize)]
struct Grid {
    x: f32,
    y: f32,
    z: f32,
    o: f32,
}

/// How far apart a race's grid slots stand (yards): `Kartcraft.cpp`'s `GRID_SPACING`.
const GRID_SPACING: f32 = 4.0;

impl Grid {
    /// Where a test lap starts (WoW x, y, facing).
    fn spot(self) -> [f32; 3] {
        [self.x, self.y, self.o]
    }
}

/// A test lap's grid: `grid` where one is placed, else the slot a race puts its first racer in,
/// behind the start facing down the first leg (`Kartcraft.cpp` `TeleportToGrid`, slot 0).
fn grid_spot(grid: Option<Grid>, points: &[[f32; 2]]) -> Option<[f32; 3]> {
    if let Some(g) = grid {
        return Some(g.spot());
    }
    let (s, n) = (points.first()?, points.get(1)?);
    let (dx, dy) = (n[0] - s[0], n[1] - s[1]);
    let len = dx.hypot(dy).max(1.0);
    let (dx, dy) = (dx / len, dy / len);
    let (back, side) = (GRID_SPACING, -GRID_SPACING * 0.5);
    Some([
        s[0] - dx * back - dy * side,
        s[1] - dy * back + dx * side,
        dy.atan2(dx),
    ])
}

/// The kart at `kart` (Bevy world) stands within TRIP_NEAR of `to` (WoW x, y).
fn near(kart: Vec3, to: [f32; 3]) -> bool {
    let w = bevy_to_wow(kart);
    (w[0] - to[0]).hypot(w[1] - to[1]) < TRIP_NEAR
}

/// A track's floor (WoW z): its grid's height, else its start's, where known. In a dungeon the
/// server lands a trip onto the grid on the floor under it (`Kartcraft.cpp` `TeleportOnto`), not on
/// a roof over it.
fn floor(grid: Option<Grid>, heights: &[Option<f32>]) -> Option<f32> {
    grid.map(|g| g.z)
        .or_else(|| heights.first().copied().flatten())
}

/// A test lap refused off the track's own map (a continent or a dungeon).
const ELSEWHERE: &str = "That track is on another map: go there to test it.";

/// A saved track, read back from `kart-tracks/`.
struct Saved {
    course: Course,
    grid: Option<Grid>,
    objects: Vec<Placed>,
}

impl Saved {
    fn floor(&self) -> Option<f32> {
        floor(self.grid, &self.course.heights())
    }
}

/// What a left click in the world places, as the bar's buttons pick it: `Move` nothing, it
/// takes what is under the cursor to drag; `Number` picks a checkpoint to renumber, `Erase` one
/// or an object to delete; `Object` places the kind [`kinds`]`()[k]`. The first checkpoint placed
/// is the start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    Move,
    Number,
    Erase,
    Checkpoint,
    /// A ring checkpoint, hung in the air.
    Ring,
    Grid,
    Object(usize),
}

/// What the keys and the middle mouse turn: an object, or a ring checkpoint, held (its index) or
/// about to be placed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Turning {
    Object(Option<usize>),
    Ring(Option<usize>),
}

/// What the move cursor holds: a gate (0 the start), the grid or a placed object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Held {
    Gate(usize),
    Grid,
    Object(usize),
}

/// What undo took, for redo: a gate (its spot, facing, half-width and height) or an object.
#[derive(Clone, Debug, PartialEq)]
enum Undone {
    Gate([f32; 2], Option<f32>, f32, Option<f32>, Option<[f32; 2]>),
    Object(Placed),
}

/// The editor, its draft and the saved tracks. A draft goes once saved or closed; one the kart's
/// loss closed waits for the editor to open again on its map.
#[derive(Resource)]
pub(crate) struct TrackEdit {
    on: bool,
    /// A test lap is under way (the practice race), as of this frame.
    testing: bool,
    /// The start first, then the checkpoints, WoW x, y, and each one's facing where turned.
    points: Vec<[f32; 2]>,
    facings: Vec<Option<f32>>,
    /// Each gate's height (WoW z) where placed: a saved track's, and a dungeon trip's floor.
    heights: Vec<Option<f32>>,
    /// Which checkpoints are rings, each one's pitch and roll (its yaw its facing, its radius its
    /// half-width, its centre its height); and the tilt the next ring takes.
    rings: Vec<Option<[f32; 2]>>,
    tilt: [f32; 2],
    /// Each gate's half-width (yards), and the one the next gate gets (the mouse wheel's).
    widths: Vec<f32>,
    width: f32,
    /// The facing the gate about to be placed is turned to (middle mouse); `None` square to the
    /// way from the last gate.
    turn: Option<f32>,
    grid: Option<Grid>,
    /// 0 a sprint, else the laps.
    laps: u8,
    /// The karts' and racers' size on the track, 1 their own (`docs/courses.md`, `scale`).
    scale: f32,
    /// The map the draft is on, from where the editor opened.
    map: u32,
    /// The tool picked; [`TrackEdit::tool`] is the one a click uses.
    tool: Place,
    /// What the move cursor is dragging, until let go, and its offset from the cursor (WoW x,
    /// y) as taken, so a press without a drag moves nothing.
    held: Option<Held>,
    grab: [f32; 2],
    /// What undo took, the last on top, for redo; a new gate or object forgets them.
    undone: Vec<Undone>,
    /// The placed objects, and what was placed in turn (true an object, false a gate), which undo
    /// takes back last first.
    objects: Vec<Placed>,
    placed: Vec<bool>,
    /// The turn the next object takes (yaw, pitch, roll, radians), and the ghost: the object
    /// held, or the one a click would place, as of this frame.
    object_turn: [f32; 3],
    /// The size the next build piece of each shape is placed at (length, width, height), as the
    /// size keys set it ([`SIZE_KEYS`]); a held one's are its own.
    piece_sizes: [[f32; 3]; 2],
    ghost: Option<Placed>,
    /// Snapping (the bar's Snap, off at first): an object placed or dragged near another takes
    /// its turn when near it and meets it face to face ([`super::objects::snap`]); and the ghost
    /// as snapped, with where it meets, which a click places.
    snap: bool,
    snapped: Option<(Placed, Vec3)>,
    /// Where the middle mouse or Ctrl held pins the object turned (WoW, its ground), so the mouse
    /// turns or raises it in place; and how high over its ground it is raised (yards).
    pin: Option<[f32; 3]>,
    lift: f32,
    /// The checkpoint the number tool picked, to renumber.
    picked: Option<usize>,
    /// The saved tracks once read, and how often the list changed: each change goes to the
    /// track browser.
    saved: Option<Vec<Saved>>,
    saves: u32,
    /// The last word for the player, and how many words so far: each new one goes to chat.
    note: String,
    notes: u32,
    /// The last thing that could not be done and why, and how many so far: each a popup.
    error: String,
    errors: u32,
    /// The name a saved track opened under (My tracks' Edit), for the bar's name box to take.
    named: Option<String>,
    /// Each checkpoint shown this frame (0 the start) and the top of its marker's pole (Bevy
    /// world), where its number hangs.
    marks: Vec<(usize, Vec3)>,
    /// Edit's trip to the track's start (WoW x, y, o), and when it was asked of the server: the
    /// camera flies from there once the kart lands.
    trip: Option<([f32; 3], f32)>,
}

impl Default for TrackEdit {
    fn default() -> Self {
        Self {
            on: false,
            testing: false,
            points: Vec::new(),
            facings: Vec::new(),
            heights: Vec::new(),
            rings: Vec::new(),
            tilt: [0.0; 2],
            widths: Vec::new(),
            width: GATE_HALF_WIDTH,
            turn: None,
            grid: None,
            laps: 3,
            scale: 1.0,
            map: 0,
            tool: Place::Move,
            held: None,
            grab: [0.0; 2],
            undone: Vec::new(),
            objects: Vec::new(),
            placed: Vec::new(),
            object_turn: [0.0; 3],
            piece_sizes: super::pieces::Shape::ALL.map(super::pieces::Shape::default_size),
            ghost: None,
            snap: false,
            snapped: None,
            pin: None,
            lift: 0.0,
            picked: None,
            saved: None,
            saves: 0,
            note: String::new(),
            notes: 0,
            error: String::new(),
            errors: 0,
            named: None,
            marks: Vec::new(),
            trip: None,
        }
    }
}

/// The bar's view of the editor (`BENILLA_KART_TRACKEDIT`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EditView {
    /// `edit`, `test` (a test lap of the draft), `run` (of a saved track) or empty, closed.
    pub(crate) state: &'static str,
    /// The draft's gates, the start among them, and whether it has its grid.
    pub(crate) gates: usize,
    pub(crate) grid: bool,
    pub(crate) laps: u8,
    /// The karts' size on the track.
    pub(crate) scale: f32,
    pub(crate) notes: u32,
    /// What a click does: `move`, `number` (picks a checkpoint), or places a `checkpoint` (the
    /// start when there is none) or the `grid`.
    pub(crate) tool: &'static str,
    /// The checkpoint the number tool picked, 1 the start; 0 none.
    pub(crate) picked: usize,
    /// The object kind a click places, with the object tool.
    pub(crate) object: Option<&'static str>,
    /// An object turns on the keys ([`TURN_KEYS`]): one is held, or about to be placed.
    pub(crate) turns_object: bool,
    /// Objects snap to each other.
    pub(crate) snap: bool,
    /// The objects placed.
    pub(crate) objects: usize,
}

impl TrackEdit {
    /// `p` snapped onto the draft's other objects (all but `skip`), with where it meets one,
    /// while snapping is on.
    fn snap_onto(
        &self,
        p: &Placed,
        skip: Option<usize>,
        models: &super::objects::Models,
    ) -> Option<(Placed, Vec3)> {
        if !self.snap {
            return None;
        }
        let bounds = models.bounds(p)?;
        let others = (self.objects.iter().enumerate())
            .filter(|(i, _)| Some(*i) != skip)
            .filter_map(|(_, o)| Some((o.clone(), models.bounds(o)?)));
        super::objects::snap(p, bounds, others)
    }

    fn say(&mut self, note: impl Into<String>) {
        self.note = note.into();
        self.notes += 1;
    }

    /// What the player asked for cannot be done, and why: a popup.
    fn fail(&mut self, why: impl Into<String>) {
        self.error = why.into();
        self.errors += 1;
    }

    /// The draft for its gates and the race map: while editing, not while testing.
    pub(crate) fn preview(&self, me: &str) -> Option<Active> {
        (self.on && !self.testing && !self.points.is_empty()).then(|| {
            Active::draft(self.laps, self.points.clone(), me)
                .with_facings(self.facings.clone())
                .with_heights(self.heights.clone())
                .with_rings(self.rings.clone())
                .with_gates(self.widths.clone())
                // So its default-wide gates are drawn as the race on it sizes them.
                .with_scale(self.scale)
        })
    }

    pub(crate) fn view(&self) -> EditView {
        let state = match (self.on, self.testing) {
            (true, false) => "edit",
            (true, true) => "test",
            (false, true) => "run",
            (false, false) => "",
        };
        EditView {
            state,
            gates: self.points.len(),
            grid: self.grid.is_some(),
            laps: self.laps,
            scale: self.scale,
            notes: self.notes,
            tool: match self.tool {
                Place::Move => "move",
                Place::Number => "number",
                Place::Erase => "erase",
                Place::Object(_) => "object",
                Place::Checkpoint => "checkpoint",
                Place::Ring => "ring",
                Place::Grid => "grid",
            },
            picked: self.picked.map_or(0, |i| i + 1),
            object: match self.tool {
                Place::Object(k) => kinds().get(k).map(|k| k.key),
                _ => None,
            },
            turns_object: self.on && !self.testing && self.turning().is_some(),
            snap: self.snap,
            objects: self.objects.len(),
        }
    }

    /// The bar's tool: what a left click does from now, nothing picked.
    fn use_tool(&mut self, tool: Place) {
        self.tool = tool;
        self.lift = 0.0;
        self.picked = None;
    }

    /// Takes the last gate away, for redo.
    fn undo(&mut self) {
        self.drop_held();
        // What was placed last; a track opened from a file has no order, its gates first.
        let object = self.placed.pop().unwrap_or(self.points.is_empty());
        if object {
            if let Some(o) = self.objects.pop() {
                self.undone.push(Undone::Object(o));
            }
        } else if let Some(p) = self.points.pop() {
            let o = self.facings.pop().flatten();
            let w = self.widths.pop().unwrap_or(GATE_HALF_WIDTH);
            let z = self.heights.pop().flatten();
            self.rings.resize(self.points.len() + 1, None);
            let r = self.rings.pop().flatten();
            self.undone.push(Undone::Gate(p, o, w, z, r));
        }
        self.picked = self.picked.filter(|&i| i < self.points.len());
    }

    /// Puts back what undo took last.
    fn redo(&mut self) {
        match self.undone.pop() {
            Some(Undone::Gate(p, o, w, z, r)) => {
                self.rings.resize(self.points.len(), None);
                self.points.push(p);
                self.facings.push(o);
                self.widths.push(w);
                self.heights.push(z);
                self.rings.push(r);
                self.placed.push(false);
            }
            Some(Undone::Object(o)) => {
                self.objects.push(o);
                self.placed.push(true);
            }
            None => self.fail("Nothing to redo."),
        }
    }

    /// Deletes placed object `i`.
    fn remove_object(&mut self, i: usize) {
        if i >= self.objects.len() {
            return;
        }
        self.drop_held();
        let o = self.objects.remove(i);
        forget_one(&mut self.placed, true);
        self.undone.clear();
        let name = super::objects::kind(&o.model).map_or("Object", |(_, k)| k.name);
        self.say(format!("{name} deleted."));
    }

    /// The objects to lay in the world while the editor is open (with their index), all but the
    /// one held, which the ghost shows.
    pub(crate) fn laid_objects(&self) -> Vec<(usize, Placed)> {
        let held = match self.held {
            Some(Held::Object(i)) => Some(i),
            _ => None,
        };
        (self.objects.iter().cloned().enumerate())
            .filter(|(i, _)| self.on && Some(*i) != held)
            .collect()
    }

    /// The placed turbo pad lying under WoW `at`, if any (its index among the objects): a pad has
    /// no collider for the aim to hit.
    fn pad_under(&self, at: [f32; 3]) -> Option<usize> {
        let half = kartcraft::pads::PAD_LENGTH * 0.5;
        (self.objects.iter().enumerate())
            .filter(|(_, o)| super::objects::is_pad(&o.model))
            .filter(|(_, o)| (o.z - at[2]).abs() <= kartcraft::pads::PAD_REACH + half)
            .map(|(i, o)| (i, (o.x - at[0]).hypot(o.y - at[1])))
            .filter(|&(_, d)| d <= half)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    /// The ghost as of this frame.
    /// The size a new object of kind `key` is placed at: a build piece's as the size keys set it
    /// for its shape, none for a model.
    fn piece_size(&self, key: &str) -> Option<[f32; 3]> {
        let (shape, _) = super::objects::piece_of(key)?;
        let i = super::pieces::Shape::ALL.iter().position(|&s| s == shape)?;
        Some(self.piece_sizes[i])
    }

    /// The size keys held this frame grow or shrink the build piece held (`held`, its index among
    /// the objects), or else the next of kind `next` to be placed, by `step` yards each way.
    fn resize_piece(&mut self, held: Option<usize>, next: Option<&str>, step: [f32; 3]) {
        if step == [0.0; 3] {
            return;
        }
        let grow = |shape, size: [f32; 3]| {
            super::pieces::clamp_size(shape, std::array::from_fn(|i| size[i] + step[i]))
        };
        if let Some(o) = held.and_then(|i| self.objects.get_mut(i)) {
            if let Some((shape, _)) = super::objects::piece_of(&o.model) {
                o.size = Some(grow(shape, o.size.unwrap_or(shape.default_size())));
            }
            return;
        }
        let Some((shape, _)) = next.and_then(super::objects::piece_of) else {
            return;
        };
        if let Some(i) = super::pieces::Shape::ALL.iter().position(|&s| s == shape) {
            self.piece_sizes[i] = grow(shape, self.piece_sizes[i]);
        }
    }

    pub(crate) fn ghost(&self) -> Option<Placed> {
        self.ghost.clone().filter(|_| self.on && !self.testing)
    }

    /// The picked checkpoint as number `to` (1 the start), the others moving up or down a place.
    fn renumber(&mut self, to: usize) {
        let n = self.points.len();
        let Some(i) = self.picked.filter(|&i| i < n) else {
            return self.fail("Click a checkpoint with Edit checkpoint first.");
        };
        if !(1..=n).contains(&to) {
            return self.fail(format!("A checkpoint's number is 1 (the start) to {n}."));
        }
        let j = to - 1;
        self.heights.resize(n, None);
        let p = self.points.remove(i);
        self.points.insert(j, p);
        let o = self.facings.remove(i);
        self.facings.insert(j, o);
        let w = self.widths.remove(i);
        self.widths.insert(j, w);
        let z = self.heights.remove(i);
        self.heights.insert(j, z);
        self.rings.resize(n, None);
        let r = self.rings.remove(i);
        self.rings.insert(j, r);
        self.picked = Some(j);
        let now = if j == 0 {
            "the start".to_string()
        } else {
            to.to_string()
        };
        self.say(format!("Checkpoint {} is now {now}.", i + 1));
    }

    /// Deletes checkpoint `i` (0 the start, the next then the start), the others closing up.
    fn erase(&mut self, i: usize) {
        let n = self.points.len();
        if i >= n {
            return;
        }
        self.drop_held();
        self.heights.resize(n, None);
        self.rings.resize(n, None);
        self.points.remove(i);
        self.facings.remove(i);
        self.widths.remove(i);
        self.heights.remove(i);
        self.rings.remove(i);
        self.picked = None;
        // Undo and redo take the last gate, which is another one now.
        self.undone.clear();
        forget_one(&mut self.placed, false);
        self.say(match i {
            0 if n > 1 => "Start deleted: the next checkpoint is the start now.".to_string(),
            0 => "Start deleted.".to_string(),
            _ => format!("Checkpoint {} deleted.", i + 1),
        });
    }

    /// The last note.
    pub(crate) fn note(&self) -> &str {
        &self.note
    }

    /// The last thing that could not be done, and how many so far.
    pub(crate) fn error(&self) -> (&str, u32) {
        (&self.error, self.errors)
    }

    /// The last note and how many so far.
    pub(crate) fn notes(&self) -> (&str, u32) {
        (&self.note, self.notes)
    }

    /// Whether a save as `name` would replace one of ours.
    pub(crate) fn is_saved(&self, name: &str) -> bool {
        let key = key_of(&clean(name));
        self.saved.iter().flatten().any(|s| s.course.key == key)
    }

    /// Each checkpoint's number's place this frame, while editing.
    pub(crate) fn marks(&self) -> &[(usize, Vec3)] {
        &self.marks
    }

    /// The name a saved track opened under, once.
    pub(crate) fn take_named(&mut self) -> Option<String> {
        self.named.take()
    }

    /// What the bar says a left click does now.
    pub(crate) fn hint(&self) -> String {
        match (self.tool, self.picked) {
            (Place::Number, Some(i)) => format!(
                "Checkpoint {} of {} (1 is the start). New number:",
                i + 1,
                self.points.len()
            ),
            (Place::Number, None) => "Click a checkpoint to give it a new number.".into(),
            (Place::Erase, _) => "Click a checkpoint to delete it.".into(),
            (Place::Move, _) => "Drag the start, a checkpoint or the grid to move it.".into(),
            (Place::Checkpoint, _) if self.points.is_empty() => {
                "Left-click the ground to place the start, the first checkpoint.".into()
            }
            (Place::Checkpoint, _) => "Left-click the ground to place the next checkpoint.".into(),
            (Place::Grid, _) => "Left-click the ground to place the starting grid.".into(),
            (Place::Ring, _) => {
                let [pitch, roll] = self.tilt.map(|a| a.to_degrees().round() as i32);
                format!(
                    "Left-click to hang a ring checkpoint, {:.0} yd across (the wheel sizes it). \
                     Q/E yaw, R/F pitch, Z/C roll, the middle mouse turns it, Ctrl raises it. \
                     Pitch {pitch}°, roll {roll}°, {:.1} yd up.",
                    self.width * 2.0,
                    self.lift
                )
            }
            (Place::Object(k), _) => {
                let [yaw, pitch, roll] = self.object_turn.map(|a| a.to_degrees().round() as i32);
                format!(
                    "Left-click to place the {}. Q/E yaw, R/F pitch, Z/C roll; the middle mouse \
                     turns it as you see it (Shift: rolls it), Ctrl raises it. Yaw {yaw}°, pitch \
                     {pitch}°, roll {roll}°, {lift:.1} yd up.{}",
                    kinds().get(k).map_or("object", |k| k.name),
                    if self.snap { " Snap: on." } else { "" },
                    lift = self.lift,
                )
            }
        }
    }

    /// The saved tracks as the track browser lists courses ([`crate::ui_kart`]), each with its
    /// grid (`x,y,o` or empty), its gates as a publish sends them (`x,y[,o[,w]];...`), its
    /// floor (`z` or empty, [`floor`]) and its objects ([`super::objects::wire`]), and how often
    /// the list has changed.
    #[allow(clippy::type_complexity)]
    pub(crate) fn saved(
        &self,
    ) -> (
        impl Iterator<Item = (&Course, String, String, String, String)>,
        u32,
    ) {
        let list = self.saved.iter().flatten().map(|s| {
            let grid = s
                .grid
                .map_or_else(String::new, |g| format!("{:.1},{:.1},{:.3}", g.x, g.y, g.o));
            (
                &s.course,
                grid,
                spots(
                    &s.course.route(),
                    &s.course.facings(),
                    &s.course.gates(),
                    &s.course.heights(),
                    &s.course.rings(),
                ),
                s.floor().map_or_else(String::new, |z| format!("{z:.1}")),
                super::objects::wire(&s.objects),
            )
        });
        (list, self.saves)
    }

    /// A new draft: no gates, no grid.
    fn clear(&mut self) {
        self.scale = 1.0;
        self.points.clear();
        self.facings.clear();
        self.heights.clear();
        self.rings.clear();
        self.widths.clear();
        self.width = GATE_HALF_WIDTH;
        self.turn = None;
        self.grid = None;
        self.tool = Place::Move;
        self.held = None;
        self.undone.clear();
        self.objects.clear();
        self.placed.clear();
        self.ghost = None;
        self.pin = None;
        self.lift = 0.0;
        self.picked = None;
        self.trip = None;
    }

    /// What the move cursor at `at` (WoW x, y) would take: the nearest checkpoint's marker, or
    /// the grid within its circle.
    fn under(&self, at: [f32; 2]) -> Option<Held> {
        let off = |p: [f32; 2]| (p[0] - at[0]).hypot(p[1] - at[1]);
        let gates =
            (self.points.iter().enumerate()).map(|(i, &p)| (Held::Gate(i), off(p), MARKER_PICK));
        let grid = self.grid.map(|g| (Held::Grid, off([g.x, g.y]), GRID_PICK));
        gates
            .chain(grid)
            .filter(|&(_, d, reach)| d <= reach)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(held, ..)| held)
    }

    /// Takes `held` up with the cursor at `at` (WoW x, y): the turn and the wheel's width are
    /// its own while dragged.
    fn take(&mut self, held: Held, at: [f32; 2]) {
        self.held = Some(held);
        let p = match held {
            Held::Gate(i) => self.points.get(i).copied(),
            Held::Grid => self.grid.map(|g| [g.x, g.y]),
            Held::Object(i) => self.objects.get(i).map(|o| [o.x, o.y]),
        };
        self.grab = p.map_or([0.0; 2], |p| [p[0] - at[0], p[1] - at[1]]);
        match held {
            Held::Gate(i) => {
                self.turn = self.facings.get(i).copied().flatten();
                self.width = self.widths.get(i).copied().unwrap_or(GATE_HALF_WIDTH);
                if let Some(tilt) = self.rings.get(i).copied().flatten() {
                    self.tilt = tilt;
                }
            }
            Held::Grid => self.turn = self.grid.map(|g| g.o),
            Held::Object(_) => {}
        }
    }

    /// What is held to the cursor at `at` (Bevy world), facing as turned (`o` for the grid).
    fn drag(&mut self, at: Vec3, o: f32) {
        let mut w = bevy_to_wow(at);
        w[0] += self.grab[0];
        w[1] += self.grab[1];
        match self.held {
            Some(Held::Gate(i)) if self.is_ring(i) => {
                self.set_ring(i, [w[0], w[1], w[2] + self.lift])
            }
            Some(Held::Gate(i)) if i < self.points.len() => {
                self.points[i] = [w[0], w[1]];
                self.facings[i] = self.turn;
                self.widths[i] = self.width;
                self.heights.resize(self.points.len(), None);
                self.heights[i] = Some(w[2]);
            }
            Some(Held::Grid) => {
                self.grid = Some(Grid {
                    x: w[0],
                    y: w[1],
                    z: w[2],
                    o,
                });
            }
            Some(Held::Object(i)) if i < self.objects.len() => {
                let obj = &mut self.objects[i];
                (obj.x, obj.y, obj.z) = (w[0], w[1], w[2] + self.lift);
            }
            _ => self.held = None,
        }
    }

    /// Whether checkpoint `i` is a ring.
    fn is_ring(&self, i: usize) -> bool {
        self.rings.get(i).is_some_and(Option::is_some)
    }

    /// Ring `i`'s centre (WoW).
    fn ring_at(&self, i: usize) -> Option<[f32; 3]> {
        let p = self.points.get(i)?;
        Some([p[0], p[1], self.heights.get(i).copied().flatten()?])
    }

    /// Ring `i` with its centre at `at` (WoW), turned and sized as the editor has it now.
    fn set_ring(&mut self, i: usize, at: [f32; 3]) {
        let n = self.points.len();
        if i >= n {
            return;
        }
        self.heights.resize(n, None);
        self.rings.resize(n, None);
        self.points[i] = [at[0], at[1]];
        self.heights[i] = Some(at[2]);
        self.facings[i] = self.turn;
        self.widths[i] = self.width;
        self.rings[i] = Some(self.tilt);
    }

    /// What the keys and the middle mouse turn now, if anything.
    fn turning(&self) -> Option<Turning> {
        match (self.held, self.tool) {
            (Some(Held::Object(i)), _) => Some(Turning::Object(Some(i))),
            (Some(Held::Gate(i)), _) if self.is_ring(i) => Some(Turning::Ring(Some(i))),
            (None, Place::Object(_)) => Some(Turning::Object(None)),
            (None, Place::Ring) => Some(Turning::Ring(None)),
            _ => None,
        }
    }

    /// Editing, not in a test lap: the left button places and drags, so the camera's left-drag
    /// orbit (which locks the cursor) stays off.
    pub(crate) fn owns_left(&self) -> bool {
        self.on && !self.testing && self.trip.is_none()
    }

    /// Lets go of what is held.
    fn drop_held(&mut self) {
        if let Some(Held::Object(_)) = self.held {
            self.lift = 0.0;
        }
        if self.held.take().is_some() {
            self.turn = None;
            self.width = GATE_HALF_WIDTH;
        }
    }

    /// The saved track `key` as the draft, to edit on and save over (by its name).
    fn edit_saved(&mut self, key: &str) -> Result<String, &'static str> {
        let s = self
            .saved
            .iter()
            .flatten()
            .find(|s| s.course.key == key)
            .ok_or("That track is not saved any more.")?;
        let c = &s.course;
        if c.map != self.map {
            return Err(ELSEWHERE);
        }
        let (points, facings, widths, grid, laps) =
            (c.route(), c.facings(), c.gates(), s.grid, c.laps);
        let objects = s.objects.clone();
        let heights = c.heights();
        let scale = c.scale();
        let rings = c.rings();
        let name = c.name.clone();
        self.clear();
        (self.points, self.facings, self.widths) = (points, facings, widths);
        self.heights = heights;
        self.rings = rings;
        (self.grid, self.laps) = (grid, laps);
        self.scale = scale;
        self.objects = objects;
        self.named = Some(name.clone());
        Ok(name)
    }

    /// The draft's start for the kart at `kart` (Bevy world), as a test lap goes to its grid:
    /// the server's line for the trip, none when the kart is there already.
    fn go_to_start(&mut self, now: f32, kart: Vec3) -> Option<String> {
        let to = grid_spot(self.grid, &self.points)?;
        if near(kart, to) {
            return None;
        }
        self.trip = Some((to, now));
        // Onto the track's floor, under a roof where it runs under one (as a test lap's trip).
        let z = floor(self.grid, &self.heights).map_or_else(String::new, |z| format!("\t{z:.1}"));
        Some(format!(
            "KARTCRAFT\tR\tTG\t{:.1}\t{:.1}\t{:.3}{z}",
            to[0], to[1], to[2]
        ))
    }

    /// Edit's trip a frame on, the kart where it is (Bevy world; `None` out of it): over when
    /// the kart stands at the start, or after TRIP_SECS (the server refused, or the kart is
    /// gone, which closes the editor).
    fn trip_step(&mut self, now: f32, kart: Option<Vec3>) {
        if let Some((to, at)) = self.trip {
            if kart.is_some_and(|k| near(k, to)) || now > at + TRIP_SECS {
                self.trip = None;
            }
        }
    }

    /// Why the draft cannot be raced yet, if it cannot.
    fn unready(&self) -> Option<&'static str> {
        if self.points.is_empty() {
            Some("Place the start first.")
        } else if self.laps == 0 && self.points.len() < 2 {
            Some("A sprint needs a finish too: place a checkpoint after the start.")
        } else if self.laps > 0 && self.points.len() < 3 {
            Some("A circuit needs the start and at least 2 checkpoints.")
        } else {
            None
        }
    }

    /// The draft as a courses.json course named `name`, in `zone`: its key and the JSON.
    fn course(&self, name: &str, zone: &str) -> Result<(String, String), String> {
        if let Some(why) = self.unready() {
            return Err(why.into());
        }
        let name = clean(name);
        let key = key_of(&name);
        if key.is_empty() || key == "any" {
            return Err("Name the track first (letters or digits).".into());
        }
        if COURSES.iter().any(|c| c.key == key) {
            return Err(format!(
                "A course is already called \"{key}\": pick another name."
            ));
        }
        let tenth = |v: f32| (f64::from(v) * 10.0).round() / 10.0;
        let checkpoints: Vec<_> = self
            .points
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let mut c = serde_json::json!({ "x": tenth(p[0]), "y": tenth(p[1]) });
                if let Some(o) = self.facings.get(i).copied().flatten() {
                    c["o"] = serde_json::json!((f64::from(o) * 1000.0).round() / 1000.0);
                }
                match self.widths.get(i) {
                    Some(&w) if w != GATE_HALF_WIDTH => c["gate"] = serde_json::json!(tenth(w)),
                    _ => {}
                }
                if let Some(z) = self.heights.get(i).copied().flatten() {
                    c["z"] = serde_json::json!(tenth(z));
                }
                if let Some([pitch, roll]) = self.rings.get(i).copied().flatten() {
                    let milli = |a: f32| (f64::from(a) * 1000.0).round() / 1000.0;
                    c["ring"] = serde_json::json!([milli(pitch), milli(roll)]);
                }
                c
            })
            .collect();
        let mut course = serde_json::json!({
            "key": key,
            "name": name,
            "zone": clean(zone),
            "map": self.map,
            "laps": self.laps,
            "note": NOTE,
            "checkpoints": checkpoints,
        });
        if let Some(g) = self.grid {
            let o = (f64::from(g.o) * 1000.0).round() / 1000.0;
            course["grid"] =
                serde_json::json!({ "x": tenth(g.x), "y": tenth(g.y), "z": tenth(g.z), "o": o });
        }
        if !self.objects.is_empty() {
            course["objects"] = serde_json::to_value(&self.objects).map_err(|e| e.to_string())?;
        }
        if self.scale != 1.0 {
            course["scale"] = serde_json::json!((f64::from(self.scale) * 100.0).round() / 100.0);
        }
        let json = serde_json::to_string_pretty(&course).map_err(|e| e.to_string())?;
        Ok((key, json + "\n"))
    }

    /// Saves the draft as `name` in `benilla-config/kart-tracks/<key>.json`, the editor going on
    /// with a new one; over a track saved under that name only with `overwrite`, which the save
    /// popup asks for once it has warned.
    fn save(&mut self, name: &str, zone: &str, overwrite: bool) {
        let saved = self.course(name, zone).and_then(|(key, json)| {
            let dir = crate::local_state::kart_tracks_dir()
                .ok_or_else(|| "This build keeps no files.".to_string())?;
            let path = dir.join(format!("{key}.json"));
            if !overwrite && path.exists() {
                return Ok(None);
            }
            crate::local_state::write_atomic(&path, &json).map_err(|e| e.to_string())?;
            Ok(Some(path))
        });
        match saved {
            Ok(None) => self.fail(format!("A track named {} is saved already.", clean(name))),
            Ok(Some(path)) => {
                info!("kart track editor: saved {}", path.display());
                self.say(format!(
                    "Track saved to {} (in the track browser). A new track begins.",
                    path.display()
                ));
                self.clear();
                self.saved = None;
            }
            Err(why) => self.fail(why),
        }
    }

    /// Deletes the saved track `key` (its file in `kart-tracks/`, a key and nothing else, so no
    /// other file); the note says how it went, and the list is read again.
    fn delete(&mut self, key: &str) {
        let path = crate::local_state::kart_tracks_dir()
            .filter(|_| !key.is_empty() && key_of(key) == key)
            .map(|dir| dir.join(format!("{key}.json")));
        match path.map(std::fs::remove_file) {
            Some(Ok(())) => {
                info!("kart track editor: deleted {key}");
                self.say("Track deleted.");
            }
            _ => self.fail("That track could not be deleted."),
        }
        self.saved = None;
    }

    /// The saved tracks, read once and after each save.
    fn load(&mut self) {
        if self.saved.is_none() {
            self.saved = Some(read_saved());
            self.saves += 1;
        }
    }
}

/// Drops the last `object` (or gate) from the order things were placed in.
fn forget_one(placed: &mut Vec<bool>, object: bool) {
    if let Some(i) = placed.iter().rposition(|&o| o == object) {
        placed.remove(i);
    }
}

/// Every saved track in `kart-tracks/`, by file name; a file that is no course is passed over.
fn read_saved() -> Vec<Saved> {
    let Some(dir) = crate::local_state::kart_tracks_dir() else {
        return Vec::new();
    };
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    files
        .iter()
        .filter_map(|path| {
            let text = std::fs::read_to_string(path).ok()?;
            let parsed = serde_json::from_str::<Course>(&text).and_then(|course| {
                let value: serde_json::Value = serde_json::from_str(&text)?;
                let grid = value
                    .get("grid")
                    .and_then(|g| serde_json::from_value(g.clone()).ok());
                let objects = (value.get("objects"))
                    .and_then(|o| serde_json::from_value(o.clone()).ok())
                    .unwrap_or_default();
                Ok(Saved {
                    course,
                    grid,
                    objects,
                })
            });
            parsed
                .inspect_err(|e| warn!("kart track editor: {}: {e}", path.display()))
                .ok()
        })
        .collect()
}

/// Gates as a publish and a player track's list line carry them: `x,y[,o[,w[,z,p,r]]];...`, the
/// start first, `o` a turned gate's facing (empty where it is not and `w` follows), `w` a
/// half-width other than `GATE_HALF_WIDTH`; a ring has them all, `z` its centre's height and `p`,
/// `r` its pitch and roll.
fn spots(
    points: &[[f32; 2]],
    facings: &[Option<f32>],
    widths: &[f32],
    heights: &[Option<f32>],
    rings: &[Option<[f32; 2]>],
) -> String {
    let spot = |(i, p): (usize, &[f32; 2])| {
        let mut text = format!("{:.1},{:.1}", p[0], p[1]);
        let o = facings.get(i).copied().flatten();
        let ring = rings.get(i).copied().flatten();
        let z = heights.get(i).copied().flatten();
        if let (Some([pitch, roll]), Some(z)) = (ring, z) {
            let w = widths.get(i).copied().unwrap_or(GATE_HALF_WIDTH);
            let o = o.unwrap_or(0.0);
            return text + &format!(",{o:.3},{w:.1},{z:.1},{pitch:.3},{roll:.3}");
        }
        let w = widths.get(i).copied().filter(|&w| w != GATE_HALF_WIDTH);
        match (o, w) {
            (Some(o), _) => text += &format!(",{o:.3}"),
            (None, Some(_)) => text += ",",
            (None, None) => {}
        }
        if let Some(w) = w {
            text += &format!(",{w:.1}");
        }
        text
    };
    points
        .iter()
        .enumerate()
        .map(spot)
        .collect::<Vec<_>>()
        .join(";")
}

/// Gates as [`parse_spots`] reads them back: each one's spot (WoW x, y), facing where turned,
/// half-width, and for a ring its centre's height and its pitch and roll.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Gates {
    pub(crate) points: Vec<[f32; 2]>,
    pub(crate) facings: Vec<Option<f32>>,
    pub(crate) widths: Vec<f32>,
    pub(crate) heights: Vec<Option<f32>>,
    pub(crate) rings: Vec<Option<[f32; 2]>>,
}

/// [`spots`] back into gates; `None` if any spot is not numbers.
pub(crate) fn parse_spots(text: &str) -> Option<Gates> {
    let mut g = Gates::default();
    for spot in text.split(';').filter(|s| !s.is_empty()) {
        let v: Vec<&str> = spot.split(',').collect();
        let num = |i: usize| v.get(i).and_then(|t| t.trim().parse::<f32>().ok());
        let opt = |i: usize| match v.get(i) {
            Some(t) if !t.is_empty() => num(i).map(Some),
            _ => Some(None),
        };
        g.points.push([num(0)?, num(1)?]);
        g.facings.push(opt(2)?);
        g.widths.push(opt(3)?.unwrap_or(GATE_HALF_WIDTH));
        let ring = v.len() >= 7;
        g.heights.push(if ring { Some(num(4)?) } else { None });
        g.rings
            .push(if ring { Some([num(5)?, num(6)?]) } else { None });
    }
    Some(g)
}

/// A name as courses.json takes one: ASCII, no quotes, backslashes or control characters, and no
/// `|` (the UI's escapes), as the server's `CleanTrackText` keeps it: a published track's name is
/// the name saved here, and the overwrite question compares them.
fn clean(name: &str) -> String {
    let kept: String = name
        .chars()
        .filter(|c| c.is_ascii() && !c.is_ascii_control() && *c != '"' && *c != '\\' && *c != '|')
        .take(40)
        .collect();
    kept.trim().to_string()
}

/// A course key from a name: `a-z 0-9`, runs of anything else one `-`, at most 32.
fn key_of(name: &str) -> String {
    let mut key = String::new();
    for c in name.chars().map(|c| c.to_ascii_lowercase()) {
        if c.is_ascii_alphanumeric() {
            key.push(c);
        } else if !key.is_empty() && !key.ends_with('-') {
            key.push('-');
        }
    }
    key.truncate(32);
    key.trim_end_matches('-').to_string()
}

/// One of the editor's commands, its verb and arguments: from the editor's panels
/// ([`super::track_edit_ui`]) or from Lua's `KartTrackCmd` (the track browser and the popups).
#[derive(Event, Clone, Debug)]
pub(crate) struct TrackCmd(pub(crate) Vec<String>);

impl TrackCmd {
    pub(crate) fn new(words: &[&str]) -> Self {
        Self(words.iter().map(|w| w.to_string()).collect())
    }
}

/// `KartTrackEdit` opens and closes the editor; `KartTrackCmd` is a [`TrackCmd`] from Lua.
// ponytail: a CVar per press, `<n>\t<verb>\t<args>` with n counting so a repeat still changes
// it; a Lua-to-Rust call would do better if Lua's half grows again.
pub(crate) fn on_cvar(
    ev: On<crate::cvars::CvarChanged>,
    mut commands: Commands,
    mut edit: ResMut<TrackEdit>,
    mut free: ResMut<FreeCam>,
    mut races: ResMut<Races>,
    current_map: Option<Res<benilla_world::world_map::CurrentMap>>,
) {
    let testing = races.race.as_ref().is_some_and(|r| r.practice);
    let map = current_map.map_or(0, |m| m.0);
    match ev.key().as_str() {
        "karttrackedit" if ev.flag() != edit.on => {
            if ev.flag() && races.in_race() && !testing {
                edit.fail("Leave the race first to edit a track.");
                return;
            }
            if !ev.flag() {
                return close(&mut edit, &mut free, &mut races, testing);
            }
            edit.on = true;
            if testing {
                races.race = None;
            }
            // A draft the kart's loss closed goes on, on its own map.
            if map != edit.map {
                edit.clear();
            }
            edit.map = map;
            free.set(FreeMode::Fly);
        }
        "karttrackcmd" => {
            let words: Vec<&str> = ev.new.split('\t').skip(1).collect();
            commands.trigger(TrackCmd::new(&words));
        }
        _ => {}
    }
}

/// The editor's commands, whoever sent them.
pub(crate) fn on_cmd(
    ev: On<TrackCmd>,
    mut edit: ResMut<TrackEdit>,
    mut free: ResMut<FreeCam>,
    mut races: ResMut<Races>,
    drive: Res<super::KartDrive>,
    current_map: Option<Res<benilla_world::world_map::CurrentMap>>,
    (time, profession): (Res<Time>, super::profession::Profession),
) {
    let testing = races.race.as_ref().is_some_and(|r| r.practice);
    let map = current_map.map_or(0, |m| m.0);
    let words: Vec<&str> = ev.0.iter().map(String::as_str).collect();
    let editing = edit.on && !testing;
    match words.as_slice() {
        // The bar picks what a left click in the world places; it places nothing itself.
        ["place", ..] if editing => edit.use_tool(Place::Checkpoint),
        ["ring", ..] if editing => edit.use_tool(Place::Ring),
        ["grid", ..] if editing => edit.use_tool(Place::Grid),
        ["move", ..] if editing => edit.use_tool(Place::Move),
        ["number", ..] if editing => edit.use_tool(Place::Number),
        ["erase", ..] if editing => edit.use_tool(Place::Erase),
        ["object", key, ..] if editing => {
            if let Some((k, _)) = super::objects::kind(key) {
                edit.use_tool(Place::Object(k));
            }
        }
        ["renumber", to, ..] if editing => edit.renumber(to.parse().unwrap_or(0)),
        ["snap", ..] if editing => {
            edit.snap = !edit.snap;
            let on = edit.snap;
            edit.say(match on {
                true => "Snapping on: an object near another meets it face to face, turned as it.",
                false => "Snapping off.",
            });
        }
        ["undo", ..] if editing => edit.undo(),
        ["redo", ..] if editing => edit.redo(),
        ["clear", ..] if editing => edit.clear(),
        // The bar's Close: by a press, as `KartTrackEdit` may already read "0" (the
        // editor opened from My tracks' Edit).
        ["close", ..] => close(&mut edit, &mut free, &mut races, testing),
        // My tracks' Edit: the saved track as the draft, in the editor.
        ["edit", key, ..] => {
            let why = if !drive.active {
                Some("Summon your kart first (J).")
            } else if races.in_race() && !testing {
                Some("Leave the race first to edit a track.")
            } else {
                None
            };
            if let Some(why) = why {
                return edit.fail(why);
            }
            edit.map = map;
            match edit.edit_saved(key) {
                Ok(name) => {
                    edit.on = true;
                    if testing {
                        races.race = None;
                    }
                    let trip = edit.go_to_start(time.elapsed_secs(), drive.pos);
                    if let Some(line) = trip {
                        profession.send(line);
                    }
                    free.set(match edit.trip {
                        Some(_) => FreeMode::Off,
                        None => FreeMode::Fly,
                    });
                    edit.say(format!(
                        "Editing {name}: drag what to move, and save under its name to \
                         replace it."
                    ));
                }
                Err(why) => edit.fail(why),
            }
        }
        ["laps", n, ..] if editing => edit.laps = n.parse().unwrap_or(0).min(9),
        // The karts' size on the track (the bar's Size), within the race scale's bounds.
        ["scale", n, ..] if editing => {
            let n: f32 = n.parse().unwrap_or(1.0);
            edit.scale = n.clamp(kartcraft::MIN_RACE_SCALE, kartcraft::MAX_RACE_SCALE);
        }
        ["practice", name, ..] if editing => match edit.unready() {
            Some(why) => edit.fail(why),
            None => {
                let name = Some(clean(name)).filter(|n| !n.is_empty());
                let name = name.unwrap_or_else(|| "Test lap".into());
                let grid = grid_spot(edit.grid, &edit.points);
                let race = Active::practice(&name, edit.laps, edit.points.clone(), grid, &races.me);
                let race = race.with_facings(edit.facings.clone());
                let race = race
                    .with_floor(floor(edit.grid, &edit.heights))
                    .with_heights(edit.heights.clone())
                    .with_rings(edit.rings.clone())
                    .with_objects(edit.objects.clone())
                    .with_scale(edit.scale);
                races.race = Some(race.with_gates(edit.widths.clone()));
                free.set(FreeMode::Off);
            }
        },
        ["save", name, rest @ ..] if editing => {
            let zone = rest.first().copied().unwrap_or("");
            edit.save(name, zone, rest.get(1) == Some(&"overwrite"));
        }
        // A saved track's test lap, from the track browser.
        ["run", key, ..] => {
            let run = edit.saved.iter().flatten().find(|s| s.course.key == *key);
            let why = match run {
                None => Some("That track is not saved any more."),
                Some(_) if !drive.active => Some("Summon your kart first (J)."),
                Some(_) if races.in_race() && !testing => {
                    Some("Leave the race first to test a track.")
                }
                Some(s) if s.course.map != map => Some(ELSEWHERE),
                Some(_) => None,
            };
            let race = run.map(|s| {
                let c = &s.course;
                let route = c.route();
                let grid = grid_spot(s.grid, &route);
                Active::practice(&c.name, c.laps, route, grid, &races.me)
                    .with_facings(c.facings())
                    .with_gates(c.gates())
                    .with_floor(s.floor())
                    .with_heights(c.heights())
                    .with_rings(c.rings())
                    .with_objects(s.objects.clone())
                    .with_scale(c.scale())
            });
            if let Some(why) = why {
                edit.fail(why);
                return;
            }
            // Opened from the editor, the lap's end goes back to it, the draft as it was.
            races.race = race;
            free.set(FreeMode::Off);
        }
        // A saved track gone from the browser: its file deleted, the list read again.
        ["delete", key, ..] => edit.delete(key),
        // A player track's test lap, from the track browser's player tracks: its line's
        // name, laps, map, grid (`x,y,o` or empty), gates and floor (`z`, or empty or
        // absent where the server sent none).
        ["runspots", name, laps, track_map, grid, gates, rest @ ..] => {
            let floor = rest.first().and_then(|z| z.trim().parse::<f32>().ok());
            let objects = rest
                .get(1)
                .map_or_else(Vec::new, |o| super::objects::parse_wire(o));
            let scale = rest.get(2).and_then(|v| v.trim().parse::<f32>().ok());
            let route = parse_spots(gates).filter(|g| g.points.len() >= 2);
            let grid = parse_spots(grid)
                .and_then(|g| Some([g.points.first()?[0], g.points[0][1], g.facings[0]?]));
            let why = match &route {
                None => Some("That track's checkpoints did not arrive whole."),
                Some(_) if !drive.active => Some("Summon your kart first (J)."),
                Some(_) if races.in_race() && !testing => {
                    Some("Leave the race first to test a track.")
                }
                Some(_) if track_map.parse::<u32>().ok() != Some(map) => Some(ELSEWHERE),
                Some(_) => None,
            };
            if let Some(why) = why {
                edit.fail(why);
                return;
            }
            let Some(g) = route else {
                return;
            };
            let laps = laps.parse().unwrap_or(0);
            let grid = grid.or_else(|| grid_spot(None, &g.points));
            let race = Active::practice(&clean(name), laps, g.points, grid, &races.me);
            let race = race.with_facings(g.facings).with_floor(floor);
            let race = race.with_heights(g.heights).with_rings(g.rings);
            let race = race.with_gates(g.widths).with_objects(objects);
            races.race = Some(race.with_scale(scale.unwrap_or(1.0)));
            free.set(FreeMode::Off);
        }
        ["restart", ..] => {
            if let Some(race) = races.race.as_mut().filter(|r| r.practice) {
                race.restart_practice();
            }
        }
        ["back", ..] if testing => races.race = None,
        // The server would not make Edit's trip (too far, too soon): edit from here.
        ["stay", why @ ..] => {
            edit.trip = None;
            if let Some(why) = why.first() {
                edit.fail(*why);
            }
        }
        _ => {}
    }
}

/// The editor closed, its draft gone, and a test lap of it ended.
fn close(edit: &mut TrackEdit, free: &mut FreeCam, races: &mut Races, testing: bool) {
    edit.on = false;
    if testing {
        races.race = None;
    }
    edit.clear();
    free.set(FreeMode::Off);
}

/// Each frame of the editor: out of the kart it closes; while editing the camera flies, the aim
/// follows the cursor to the ground (the screen's centre while looking), the middle mouse held
/// turns the gate about to go there, and a left click, Fire or the bar places it: the start when
/// there is none, else a checkpoint. The grid shows where it stands, while editing and in its
/// test lap.
#[allow(clippy::type_complexity)]
pub(crate) fn update(
    drive: Res<super::KartDrive>,
    races: Res<Races>,
    collide: WorldCollision,
    mut edit: ResMut<TrackEdit>,
    mut free: ResMut<FreeCam>,
    mut gizmos: Gizmos,
    placed_objects: Query<&super::objects::PlacedObject>,
    (buttons, mouse, over_ui, window, camera): (
        Res<ButtonInput<MouseButton>>,
        Res<bevy::input::mouse::AccumulatedMouseMotion>,
        Res<crate::ui_script::PointerOverUi>,
        Query<&Window, With<bevy::window::PrimaryWindow>>,
        Query<(&Camera, &GlobalTransform), With<benilla_world::view::WorldCamera>>,
    ),
    (time, keys, typing, overlay, scroll, models): (
        Res<Time>,
        Res<ButtonInput<KeyCode>>,
        Res<crate::ui_script::UiKeyboardCapture>,
        Res<crate::bindings::BindingOverlay>,
        Res<bevy::input::mouse::AccumulatedMouseScroll>,
        Res<super::objects::Models>,
    ),
) {
    edit.load();
    // Edit's trip to the start: over when the kart stands there or after TRIP_SECS (the
    // server refused, or the kart is gone, which closes the editor below).
    edit.trip_step(time.elapsed_secs(), drive.active.then_some(drive.pos));
    // The controller's Fire places at the screen's centre; the keyboard's is E, which turns the
    // gate here, so the keyboard places with a click.
    let fired = overlay.pad_just_pressed(super::bindings::FIRE);
    let practice = races.race.as_ref().filter(|r| r.practice);
    edit.testing = practice.is_some();
    // `KartTrackEdit` may stay "1": the button sets "0" before its "1". A test lap's trip to its
    // grid takes the kart away for the teleport, which closes nothing: the lap ends back here.
    if edit.on && !edit.testing && edit.trip.is_none() && !drive.active {
        edit.on = false;
        free.set(FreeMode::Off);
        edit.fail("The track editor needs your kart: summon it (J) first.");
    }
    let editing = edit.owns_left();
    // Back from a test lap, or the camera switched off (/kartfree): the editor flies it.
    if editing && !free.flying() {
        free.set(FreeMode::Fly);
    }
    let eye = drive.camera.filter(|_| editing);
    // The cursor over the world, not the interface (none while it is held to look).
    let cursor = window
        .single()
        .ok()
        .and_then(Window::cursor_position)
        .filter(|_| !over_ui.0);
    let aim = eye.and_then(|eye| {
        let through = cursor.and_then(|c| {
            let (cam, at) = camera.single().ok()?;
            cam.viewport_to_world(at, c).ok()
        });
        let (from, way) = through.map_or((eye.translation, eye.forward()), |r| {
            (r.origin, r.direction)
        });
        let ahead = collide
            .ray_body(from, way, AIM_REACH)
            .map(|hit| (from + way * hit.distance, Some(hit.entity)));
        ahead.or_else(|| {
            collide
                .ray_body(eye.translation, Dir3::NEG_Y, 4000.0)
                .map(|hit| (eye.translation - Vec3::Y * hit.distance, None))
        })
    });
    // The placed object the aim is on, if any: a solid one it hit, else a turbo pad (no collider,
    // the pad field's to draw) lying under it.
    let aim_object = aim
        .and_then(|(_, e)| placed_objects.get(e?).ok())
        .map(|o| o.0)
        .or_else(|| {
            let (at, _) = aim?;
            edit.pad_under(bevy_to_wow(at))
        });
    let aim = aim.map(|(at, _)| at);
    let tool = edit.tool;
    // An object, held or about to be placed, turns in all three axes: Q/E and the middle mouse
    // across yaw it, the middle mouse up and down pitches it (Shift: rolls it).
    let turning_object = edit.turning();
    // The axes an object turned about this frame (Bevy world), with their colours, for its rings.
    let mut turned: Vec<(Vec3, Color)> = Vec::new();
    let ctrl = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    let middle = buttons.pressed(MouseButton::Middle);
    // A ring about to be hung faces square to the way from the last checkpoint, as a gate does,
    // until turned.
    let ring_auto = aim.zip(eye).map_or(0.0, |(at, eye)| {
        let w = bevy_to_wow(at);
        let from = match edit.points.last() {
            Some(p) => *p,
            None => {
                let back = bevy_to_wow(at - eye.forward() * 10.0);
                [back[0], back[1]]
            }
        };
        (w[1] - from[1]).atan2(w[0] - from[0])
    });
    if let Some(which) = turning_object.filter(|_| editing) {
        let key = |k: KeyCode| f32::from(u8::from(keys.pressed(k) && !typing.typing));
        let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
        let step = TURN_PER_SEC * time.delta_secs();
        // Its turn as yaw, pitch and roll: an object's own, a ring's its facing and tilt.
        let [yaw, pitch, roll] = match which {
            Turning::Object(i) => match i.and_then(|i| edit.objects.get(i)) {
                Some(o) => [o.yaw, o.pitch, o.roll],
                None => edit.object_turn,
            },
            Turning::Ring(_) => {
                let [pitch, roll] = edit.tilt;
                [edit.turn.unwrap_or(ring_auto), pitch, roll]
            }
        };
        let mut q = super::objects::turn(yaw, pitch, roll);
        // About an axis (WoW, unit), shown with its ring while turned or the mouse holds it.
        let mut about = |q: &mut Quat, axis: Vec3, angle: f32, color: Color, shown: bool| {
            if angle != 0.0 {
                *q = Quat::from_axis_angle(axis, angle) * *q;
            }
            if shown || angle != 0.0 {
                turned.push((wow_to_bevy(axis.to_array()), color));
            }
        };
        // The keys turn it about its own axes: Q/E yaw, R/F pitch (R its front up), Z/C roll.
        let yaw = (key(KeyCode::KeyQ) - key(KeyCode::KeyE)) * step;
        about(&mut q, Vec3::Z, yaw, YAW_COLOR, false);
        let side = q * Vec3::Y;
        let pitch = -(key(KeyCode::KeyR) - key(KeyCode::KeyF)) * step;
        about(&mut q, side, pitch, PITCH_COLOR, false);
        let length = q * Vec3::X;
        let roll = (key(KeyCode::KeyZ) - key(KeyCode::KeyC)) * step;
        about(&mut q, length, roll, ROLL_COLOR, false);
        // The middle mouse turns it as the camera sees it: across about the vertical, up and down
        // about the view's side (Shift: about the line of sight), so a drag turns it the way the
        // mouse goes whichever way it faces.
        if let (true, false, Some(eye)) = (middle, ctrl, eye) {
            let wow = |v: Vec3| Vec3::from(bevy_to_wow(v)).normalize();
            let d = mouse.delta * TURN_PER_PIXEL;
            about(&mut q, Vec3::Z, -d.x, YAW_COLOR, true);
            match shift {
                false => about(&mut q, wow(*eye.right()), d.y, PITCH_COLOR, true),
                true => about(&mut q, wow(*eye.forward()), d.y, ROLL_COLOR, true),
            }
        }
        let mut turn = Placed {
            model: String::new(),
            x: 0.0,
            y: 0.0,
            z: 0.0,
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            size: None,
        };
        turn.set_turn(q);
        match which {
            Turning::Object(i) => match i.and_then(|i| edit.objects.get_mut(i)) {
                Some(o) => (o.yaw, o.pitch, o.roll) = (turn.yaw, turn.pitch, turn.roll),
                None => edit.object_turn = [turn.yaw, turn.pitch, turn.roll],
            },
            Turning::Ring(_) => {
                edit.turn = Some(turn.yaw);
                edit.tilt = [turn.pitch, turn.roll];
            }
        }
        // Ctrl and the mouse up and down raise it off its ground, or lower it.
        if ctrl {
            edit.lift -= mouse.delta.y * LIFT_PER_PIXEL;
        }
        // A build piece's size keys: `[` `]` its length, `;` `'` its width, `-` `=` its height.
        let grow = SIZE_PER_SEC * time.delta_secs();
        let pair = |a: usize| (key(SIZE_KEYS[a + 1]) - key(SIZE_KEYS[a])) * grow;
        let next = match tool {
            Place::Object(k) => kinds().get(k).map(|k| k.key),
            _ => None,
        };
        // Only an object's: a ring is sized with the wheel.
        if let Turning::Object(held) = which {
            edit.resize_piece(held, next, [pair(0), pair(2), pair(4)]);
        }
    }
    // The middle mouse or Ctrl held pins what turns where it is (its ground: where it stands,
    // less its lift), so the mouse turns or raises it in place; let go, it follows the cursor.
    let pinning = editing && turning_object.is_some() && (middle || ctrl);
    if !pinning {
        edit.pin = None;
    } else if edit.pin.is_none() {
        let lift = edit.lift;
        edit.pin = match turning_object {
            Some(Turning::Object(Some(i))) => edit.objects.get(i).map(|o| [o.x, o.y, o.z - lift]),
            Some(Turning::Ring(Some(i))) => edit.ring_at(i).map(|g| [g[0], g[1], g[2] - lift]),
            _ => aim.map(bevy_to_wow),
        };
    }
    // What is held stays where it is pinned, at its lift.
    if let Some(pin) = edit.pin {
        let lift = edit.lift;
        match turning_object {
            Some(Turning::Object(Some(i))) => {
                if let Some(o) = edit.objects.get_mut(i) {
                    (o.x, o.y, o.z) = (pin[0], pin[1], pin[2] + lift);
                }
            }
            Some(Turning::Ring(Some(i))) => edit.set_ring(i, [pin[0], pin[1], pin[2] + lift]),
            _ => {}
        }
    }
    // What is about to be placed faces as the middle mouse turns it, else a gate square to the
    // way from the last one, the start and the grid the way the camera looks.
    let facing = aim.zip(eye).map(|(at, eye)| {
        let w = bevy_to_wow(at);
        let last = match edit.held {
            Some(_) => None,
            None => edit.points.last().filter(|_| tool == Place::Checkpoint),
        };
        let from = match last {
            Some(p) => [p[0], p[1]],
            None => {
                let back = bevy_to_wow(at - eye.forward() * 10.0);
                [back[0], back[1]]
            }
        };
        let auto = (w[1] - from[1]).atan2(w[0] - from[0]);
        let gate_turns = editing && turning_object.is_none();
        if gate_turns && buttons.pressed(MouseButton::Middle) {
            edit.turn = Some(edit.turn.unwrap_or(auto) - mouse.delta.x * TURN_PER_PIXEL);
        }
        // Q turns it left (WoW's facing grows anticlockwise), E right; not while typing a name.
        let held = |k: KeyCode| f32::from(u8::from(keys.pressed(k) && !typing.typing));
        let q_e = held(KeyCode::KeyQ) - held(KeyCode::KeyE);
        if gate_turns && q_e != 0.0 {
            edit.turn = Some(edit.turn.unwrap_or(auto) + q_e * TURN_PER_SEC * time.delta_secs());
        }
        edit.turn.unwrap_or(auto)
    });
    // The wheel over the world (a list under the cursor scrolls instead) sizes the next gate.
    let wheel = crate::bindings::wheel_lines(scroll.unit, scroll.delta.y);
    if editing && cursor.is_some() && wheel != 0.0 {
        edit.widen(wheel);
    }
    // A left click on the world (never one on the interface), or Fire at the screen's centre.
    let clicked = buttons.just_pressed(MouseButton::Left) && cursor.is_some();
    // The move cursor: a press on something takes it, held it follows the aim, let go (Fire
    // again on a controller) it stays.
    let at = aim.map(bevy_to_wow).map(|w| [w[0], w[1]]);
    // A placed object under the aim, for the move cursor and delete, before a gate or the grid.
    let object_tools = matches!(tool, Place::Move | Place::Erase);
    let hover = aim_object
        .filter(|_| object_tools)
        .map(Held::Object)
        .or_else(|| at.and_then(|w| edit.under(w)));
    let let_go = edit.held.is_some() && (fired || buttons.just_released(MouseButton::Left));
    if !editing || let_go {
        edit.drop_held();
    } else if let Some(held) = edit.held {
        if let (Some(at), Some(o), None) = (aim, facing, edit.pin) {
            edit.drag(at, o);
            // A dragged object snaps onto the others as it goes.
            if let Held::Object(i) = held {
                let snapped =
                    (edit.objects.get(i)).and_then(|p| edit.snap_onto(p, Some(i), &models));
                if let Some((p, meets)) = snapped {
                    edit.objects[i] = p;
                    gizmos.sphere(
                        Isometry3d::from_translation(wow_to_bevy(meets.into())),
                        0.4,
                        SNAP_COLOR,
                    );
                }
            }
        }
        if let Some(at) = aim.filter(|_| held == Held::Grid) {
            gizmos.circle(flat(at + Vec3::Y * 0.3), GRID_PICK, HOVER_COLOR);
        }
    } else if (fired || clicked) && tool == Place::Move {
        if let Some((held, at)) = hover.zip(at) {
            edit.take(held, at);
            // An object or a ring keeps its height over the ground it is dragged across.
            let z = match held {
                Held::Object(i) => edit.objects.get(i).map(|o| o.z),
                Held::Gate(i) if edit.is_ring(i) => edit.ring_at(i).map(|r| r[2]),
                _ => None,
            };
            if let (Some(z), Some(aim)) = (z, aim) {
                edit.lift = z - bevy_to_wow(aim)[2];
            }
        }
    } else if (fired || clicked) && tool == Place::Number {
        edit.picked = match hover {
            Some(Held::Gate(i)) => Some(i),
            _ => None,
        };
    } else if (fired || clicked) && tool == Place::Erase {
        match hover {
            Some(Held::Gate(i)) => edit.erase(i),
            Some(Held::Object(i)) => edit.remove_object(i),
            _ => {}
        }
    } else if fired || clicked {
        match aim.zip(facing) {
            // Pinned, it goes where it is pinned.
            Some((at, o)) => {
                let at = edit.pin.map_or(at, wow_to_bevy);
                place(&mut edit, tool, at, o);
            }
            None => edit.fail("Aim at the ground to place it."),
        }
    }
    if let (Some(at), Place::Move | Place::Number | Place::Erase) = (aim.filter(|_| editing), tool)
    {
        // The cursor, and a ring round what a press would take (the number tool's a checkpoint
        // alone) and round the checkpoint picked.
        gizmos.circle(flat(at + Vec3::Y * 0.3), MARKER, HOVER_COLOR);
        let near = edit.held.or(hover);
        let near = near.filter(|h| tool == Place::Move || *h != Held::Grid);
        let ring_color = if tool == Place::Erase {
            ERASE_COLOR
        } else {
            HOVER_COLOR
        };
        let ring = match near {
            Some(Held::Gate(i)) => edit.points.get(i).map(|p| (*p, MARKER_PICK)),
            Some(Held::Grid) => edit.grid.map(|g| ([g.x, g.y], GRID_PICK)),
            // An object shows itself: held, as the ghost.
            Some(Held::Object(_)) | None => None,
        };
        let picked = edit.picked.and_then(|i| edit.points.get(i));
        let picked = picked.map(|p| (*p, MARKER_PICK + 1.0));
        for (p, r) in ring.into_iter().chain(picked) {
            let centre = wow_to_bevy([p[0], p[1], 0.0]).with_y(at.y + 0.5);
            gizmos.circle(flat(centre), r, ring_color);
        }
    } else if let (Some(at), Place::Object(_)) = (aim.filter(|_| editing), tool) {
        // The ghost shows what goes there; a small ring marks its origin.
        gizmos.circle(flat(at + Vec3::Y * 0.3), 0.5, AIM_COLOR);
    } else if let (Some(at), Some(o), Place::Grid) = (aim, facing, tool) {
        gizmos.circle(flat(at + Vec3::Y * 0.3), 6.0, GRID_COLOR);
        let w = bevy_to_wow(at);
        let ahead = wow_to_bevy([w[0] + 8.0 * o.cos(), w[1] + 8.0 * o.sin(), 0.0]).with_y(at.y);
        gizmos.arrow(at + Vec3::Y * 0.3, ahead + Vec3::Y * 0.3, GRID_COLOR);
    } else if let (Some(at), Some(o)) = (aim, facing) {
        gizmos.circle(flat(at), 2.0, AIM_COLOR);
        // Its posts either side, square to the way it faces, and an arrow along it.
        let w = bevy_to_wow(at);
        let side = [o.sin() * edit.width, -o.cos() * edit.width];
        let post = |k: f32| wow_to_bevy([w[0] + k * side[0], w[1] + k * side[1], 0.0]).with_y(at.y);
        let (left, right) = (post(1.0), post(-1.0));
        for p in [left, right] {
            gizmos.line(p, p + Vec3::Y * 8.0, AIM_COLOR);
        }
        gizmos.line(left + Vec3::Y * 8.0, right + Vec3::Y * 8.0, AIM_COLOR);
        let ahead = wow_to_bevy([w[0] + 8.0 * o.cos(), w[1] + 8.0 * o.sin(), 0.0]).with_y(at.y);
        gizmos.arrow(at + Vec3::Y * 0.5, ahead + Vec3::Y * 0.5, AIM_COLOR);
    }
    // The ghost: the object held, or the one a click would place at the aim.
    edit.ghost = match (edit.held, tool) {
        (Some(Held::Object(i)), _) => edit.objects.get(i).cloned(),
        (None, Place::Object(k)) if editing => aim.zip(kinds().get(k)).map(|(at, kind)| {
            let w = edit.pin.unwrap_or_else(|| bevy_to_wow(at));
            let [yaw, pitch, roll] = edit.object_turn;
            Placed {
                model: kind.key.into(),
                x: w[0],
                y: w[1],
                z: w[2] + edit.lift,
                yaw,
                pitch,
                roll,
                size: edit.piece_size(kind.key),
            }
        }),
        _ => None,
    };
    // The one a click would place, snapped onto the others: the click places it as it shows.
    let placing = edit.held.is_none().then(|| edit.ghost.clone()).flatten();
    edit.snapped = placing.and_then(|g| edit.snap_onto(&g, None, &models));
    if let Some((g, meets)) = &edit.snapped {
        gizmos.sphere(
            Isometry3d::from_translation(wow_to_bevy((*meets).into())),
            0.4,
            SNAP_COLOR,
        );
        edit.ghost = Some(g.clone());
    }
    // Pinned, its axes show, each with its ring, the ones the mouse turns bright: across yaws,
    // up and down pitches (with Shift, rolls).
    // What turns, where its axes show: the ghost of an object, else a ring held or to be hung.
    let ring_turned = |yaw: Option<f32>, at: [f32; 3]| {
        let [pitch, roll] = edit.tilt;
        Placed {
            model: String::new(),
            x: at[0],
            y: at[1],
            z: at[2],
            yaw: yaw.unwrap_or(ring_auto),
            pitch,
            roll,
            size: None,
        }
    };
    let ring_shown = match turning_object.filter(|_| editing) {
        Some(Turning::Ring(Some(i))) => edit.ring_at(i).map(|at| ring_turned(edit.turn, at)),
        Some(Turning::Ring(None)) => {
            let ground = edit.pin.or(aim.map(bevy_to_wow));
            let lift = edit.width + edit.lift;
            ground.map(|g| ring_turned(edit.turn, [g[0], g[1], g[2] + lift]))
        }
        _ => None,
    };
    // A ring about to be hung: its hoop where a click hangs it, and a line down to its ground.
    if let (Some(r), Some(Turning::Ring(None))) = (&ring_shown, turning_object) {
        let t = r.transform();
        let through = t.rotation * wow_to_bevy([1.0, 0.0, 0.0]);
        let hoop = Isometry3d::new(t.translation, Quat::from_rotation_arc(Vec3::Z, through));
        gizmos.circle(hoop, edit.width, AIM_COLOR);
        let ground = t.translation - Vec3::Y * (edit.width + edit.lift);
        gizmos.line(ground, t.translation - Vec3::Y * edit.width, AIM_COLOR);
    }
    let axes_at = edit.ghost.clone().or(ring_shown);
    if let (true, Some(g)) = (edit.pin.is_some() || !turned.is_empty(), &axes_at) {
        let centre = g.transform().translation;
        let colors = [YAW_COLOR, PITCH_COLOR, ROLL_COLOR];
        for (axis, color) in g.axes().into_iter().zip(colors) {
            let color = color.with_alpha(0.4);
            gizmos.line(
                centre - axis * AXIS_LENGTH,
                centre + axis * AXIS_LENGTH,
                color,
            );
        }
        for (axis, color) in &turned {
            let axis = axis.normalize_or_zero();
            gizmos.line(
                centre - axis * AXIS_LENGTH,
                centre + axis * AXIS_LENGTH,
                *color,
            );
            let ring = Isometry3d::new(centre, Quat::from_rotation_arc(Vec3::Z, axis));
            gizmos.circle(ring, AXIS_RING, *color);
        }
        if ctrl && edit.ghost.is_some() {
            // Raising: a line down to its ground.
            let ground = centre - Vec3::Y * edit.lift;
            gizmos.line(ground, centre, AIM_COLOR);
        }
    }
    // Each checkpoint's small marker and pole on the floor it was placed on (under a roof, as
    // the Caverns of Time's way down, not on the roof), the start's green, its number over it.
    let mut marks = Vec::new();
    if eye.is_some() {
        for (i, p) in edit.points.iter().enumerate() {
            let column = wow_to_bevy([p[0], p[1], 0.0]);
            let hint = edit
                .heights
                .get(i)
                .copied()
                .flatten()
                .map(super::race::bevy_y);
            let Some(y) = super::race::floor_under(&collide, column, hint).or(hint) else {
                continue;
            };
            let foot = column.with_y(y + 0.3);
            let color = if i == 0 { START_COLOR } else { MARKER_COLOR };
            gizmos.circle(flat(foot), MARKER, color);
            gizmos.line(foot, foot + Vec3::Y * MARKER_POLE, color);
            // A ring's number over its hoop, a gate's over its marker.
            let over = match edit.ring_at(i) {
                Some(c) => {
                    let r = edit.widths.get(i).copied().unwrap_or(GATE_HALF_WIDTH);
                    wow_to_bevy(c) + Vec3::Y * (r + 1.0)
                }
                None => foot + Vec3::Y * (MARKER_POLE + 1.0),
            };
            marks.push((i, over));
        }
    }
    edit.marks = marks;
    // The grid: the draft's while editing, a test lap's until its start.
    let grid = match practice {
        Some(race) => race.grid().map(|g| (g, None)),
        None => edit.grid.filter(|_| editing).map(|g| ([g.x, g.y], Some(g))),
    };
    if let Some((at, full)) = grid {
        let z = full.map_or(drive.pos.y, |g| wow_to_bevy([g.x, g.y, g.z]).y);
        let centre = wow_to_bevy([at[0], at[1], 0.0]).with_y(z + 0.3);
        gizmos.circle(flat(centre), 6.0, GRID_COLOR);
        if let Some(g) = full {
            let ahead = wow_to_bevy([g.x + 8.0 * g.o.cos(), g.y + 8.0 * g.o.sin(), 0.0]);
            gizmos.arrow(centre, ahead.with_y(centre.y), GRID_COLOR);
        }
    }
}

/// A circle at `at` lying on the ground.
fn flat(at: Vec3) -> Isometry3d {
    Isometry3d::new(at, Quat::from_rotation_x(std::f32::consts::FRAC_PI_2))
}

impl TrackEdit {
    /// The next gate `notches` of the wheel wider (narrower, negative), within its bounds.
    fn widen(&mut self, notches: f32) {
        self.width = (self.width + notches * WIDTH_PER_NOTCH).clamp(WIDTH_MIN, WIDTH_MAX);
    }
}

/// Places `what` at `at` (Bevy world): a gate keeps its facing `o` only where the middle mouse
/// turned it, the grid always, and the wheel's width. Checkpoints come next.
fn place(edit: &mut TrackEdit, what: Place, at: Vec3, o: f32) {
    let w = bevy_to_wow(at);
    match what {
        Place::Checkpoint => {
            if edit.points.is_empty() {
                edit.say("Start placed: now click the checkpoints, in driving order.");
            }
            let n = edit.points.len();
            edit.rings.resize(n, None);
            edit.points.push([w[0], w[1]]);
            edit.facings.push(edit.turn.take());
            edit.widths.push(edit.width);
            edit.heights.push(Some(w[2]));
            edit.rings.push(None);
            edit.undone.clear();
            edit.placed.push(false);
        }
        Place::Ring => {
            if edit.points.is_empty() {
                edit.say("Start placed: now place the checkpoints, in driving order.");
            }
            let n = edit.points.len();
            edit.heights.resize(n, None);
            edit.rings.resize(n, None);
            edit.points.push([w[0], w[1]]);
            edit.facings.push(Some(o));
            edit.widths.push(edit.width);
            // It hangs with its bottom on the ground, raised by Ctrl from there.
            edit.heights.push(Some(w[2] + edit.width + edit.lift));
            edit.rings.push(Some(edit.tilt));
            edit.undone.clear();
            edit.placed.push(false);
            // The tool stays: the next click hangs another.
            return;
        }
        Place::Grid => {
            edit.turn = None;
            edit.grid = Some(Grid {
                x: w[0],
                y: w[1],
                z: w[2],
                o,
            });
            edit.say("Starting grid placed.");
        }
        Place::Object(k) => {
            let Some(kind) = kinds().get(k) else { return };
            let [yaw, pitch, roll] = edit.object_turn;
            let size = edit.piece_size(kind.key);
            let snapped = edit.snapped.take().map(|(p, _)| p);
            let placed = snapped.filter(|p| p.model == kind.key).unwrap_or(Placed {
                model: kind.key.into(),
                x: w[0],
                y: w[1],
                z: w[2] + edit.lift,
                yaw,
                pitch,
                roll,
                size,
            });
            edit.objects.push(placed);
            edit.placed.push(true);
            edit.undone.clear();
            // The tool stays: the next click places another.
            return;
        }
        Place::Move | Place::Number | Place::Erase => return,
    }
    edit.tool = Place::Checkpoint;
    info!(
        "kart track editor: {what:?} at {w:?}, {} gates",
        edit.points.len()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kart::objects::KINDS;

    /// A left click at `at`, the gate or grid facing `o`.
    fn click(edit: &mut TrackEdit, at: Vec3, o: f32) {
        let tool = edit.tool;
        place(edit, tool, at, o);
    }

    fn draft(laps: u8, points: &[[f32; 2]]) -> TrackEdit {
        TrackEdit {
            on: true,
            laps,
            points: points.to_vec(),
            ..default()
        }
    }

    #[test]
    fn gates_go_out_and_come_back_with_their_facings() {
        let points = [[1.0, 2.0], [3.04, -4.0], [5.0, 6.0], [7.0, 8.0]];
        let facings = [None, Some(1.5), None, Some(0.5)];
        let widths = [GATE_HALF_WIDTH, GATE_HALF_WIDTH, 12.0, 30.0];
        let heights = [None, None, None, Some(40.0)];
        let rings = [None, None, None, Some([0.25, -0.5])];
        let text = spots(&points, &facings, &widths, &[None; 4], &[None; 4]);
        assert_eq!(
            text,
            "1.0,2.0;3.0,-4.0,1.500;5.0,6.0,,12.0;7.0,8.0,0.500,30.0"
        );
        let g = parse_spots(&text).unwrap();
        assert_eq!(g.points[1], [3.0, -4.0]);
        assert_eq!(g.facings, facings);
        assert_eq!(g.widths, widths);
        assert_eq!(g.rings, [None; 4]);
        // A ring carries all: its facing, radius, centre height, pitch and roll.
        let text = spots(&points, &facings, &widths, &heights, &rings);
        assert!(
            text.ends_with(";7.0,8.0,0.500,30.0,40.0,0.250,-0.500"),
            "{text}"
        );
        let g = parse_spots(&text).unwrap();
        assert_eq!((g.heights, g.rings), (heights.to_vec(), rings.to_vec()));
        assert!(parse_spots("1,2;x,4").is_none());
        assert_eq!(parse_spots("").unwrap().points.len(), 0);
    }

    #[test]
    fn a_name_makes_a_course_key() {
        assert_eq!(key_of("My  Track #2!"), "my-track-2");
        assert_eq!(key_of("  --  "), "");
        assert_eq!(key_of(&"a".repeat(40)).len(), 32);
        assert_eq!(clean(" Bob's \"Loop\"\\ "), "Bob's Loop");
        assert_eq!(
            clean("A|B"),
            "AB",
            "the server's name, the overwrite question's"
        );
    }

    #[test]
    fn a_saved_track_reads_back_as_a_course_with_its_grid() {
        let mut edit = draft(3, &[[-9436.04, 59.0], [-9500.0, 120.0], [-9560.0, 60.0]]);
        edit.grid = Some(Grid {
            x: -9420.0,
            y: 50.0,
            z: 56.2,
            o: 2.5,
        });
        let (key, json) = edit.course("Goldshire Ring", "Elwynn Forest").unwrap();
        assert_eq!(key, "goldshire-ring");
        let course: Course = serde_json::from_str(&json).unwrap();
        assert_eq!(course.key, "goldshire-ring");
        assert_eq!(course.name, "Goldshire Ring");
        assert_eq!(course.zone, "Elwynn Forest");
        assert_eq!(course.laps, 3);
        assert_eq!(course.route()[0], [-9436.0, 59.0]);
        assert_eq!(course.route().len(), 3);
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let grid: Grid = serde_json::from_value(value["grid"].clone()).unwrap();
        assert_eq!(grid.o, 2.5);
    }

    #[test]
    fn a_track_too_short_or_misnamed_is_not_saved() {
        let two = [[0.0, 0.0], [100.0, 0.0]];
        assert!(draft(0, &[]).course("Dash", "").is_err(), "no start");
        assert!(
            draft(3, &two).course("Loop", "").is_err(),
            "a circuit needs 3"
        );
        assert!(
            draft(0, &two).course("Dash", "").is_ok(),
            "a sprint needs 2"
        );
        assert!(
            draft(0, &two).course("!!", "").is_err(),
            "no key in the name"
        );
        let err = draft(0, &two).course(&COURSES[0].key, "").unwrap_err();
        assert!(err.contains("already"), "{err}");
    }

    /// Each gate keeps the height it was placed at, which the saved track writes; its floor is the
    /// grid's height, else the start's, the floor a dungeon's trip lands on.
    #[test]
    fn a_track_keeps_its_heights_and_its_floor_is_the_grids_else_the_starts() {
        let mut edit = draft(0, &[]);
        edit.tool = Place::Checkpoint;
        click(&mut edit, wow_to_bevy([20.0, 0.0, 133.0]), 0.0);
        click(&mut edit, wow_to_bevy([50.0, 0.0, 140.0]), 0.0);
        assert_eq!(edit.heights, vec![Some(133.0), Some(140.0)]);
        assert_eq!(floor(edit.grid, &edit.heights), Some(133.0));
        let (_, json) = edit.course("Crypt Dash", "Stratholme").unwrap();
        let saved: Course = serde_json::from_str(&json).unwrap();
        assert_eq!(saved.heights(), vec![Some(133.0), Some(140.0)]);
        // Undo and redo carry the height with the gate.
        edit.undo();
        edit.redo();
        assert_eq!(edit.heights, vec![Some(133.0), Some(140.0)]);
        edit.tool = Place::Grid;
        click(&mut edit, wow_to_bevy([10.0, 0.0, 131.0]), 0.0);
        assert_eq!(floor(edit.grid, &edit.heights), Some(131.0));
    }

    #[test]
    fn the_first_checkpoint_is_the_start_and_the_grid_faces_as_aimed() {
        let mut edit = draft(0, &[]);
        let at = |x: f32| wow_to_bevy([x, 0.0, 0.0]);
        // The first checkpoint placed is the start; there is no tool of its own.
        edit.tool = Place::Checkpoint;
        edit.turn = Some(1.0);
        click(&mut edit, at(20.0), 1.0);
        assert!(edit.note().starts_with("Start placed"), "{}", edit.note());
        click(&mut edit, at(50.0), 0.0);
        assert_eq!(edit.points, vec![[20.0, 0.0], [50.0, 0.0]]);
        // A turned gate keeps its facing; the next one goes square again.
        assert_eq!(edit.facings, vec![Some(1.0), None]);
        assert!(edit.turn.is_none());
        let (_, json) = edit.course("Facing", "").unwrap();
        assert!(json.contains("\"o\": 1.0"), "{json}");
        // The grid's tool places the grid alone, facing as aimed, and checkpoints come next.
        edit.tool = Place::Grid;
        click(&mut edit, at(0.0), 2.5);
        assert_eq!(edit.grid.map(|g| g.o), Some(2.5));
        assert_eq!(edit.points.len(), 2);
        assert_eq!(edit.view().tool, "checkpoint");
    }

    #[test]
    fn redo_puts_back_what_undo_took_until_a_new_gate() {
        let mut edit = draft(0, &[]);
        edit.tool = Place::Checkpoint;
        let at = |x: f32| wow_to_bevy([x, 0.0, 0.0]);
        for x in [0.0, 100.0, 200.0] {
            click(&mut edit, at(x), 0.0);
        }
        edit.widths[2] = 8.0;
        edit.undo();
        edit.undo();
        assert_eq!(edit.points, vec![[0.0, 0.0]]);
        edit.redo();
        edit.redo();
        assert_eq!(edit.points.len(), 3);
        assert_eq!(edit.widths[2], 8.0, "with its width");
        edit.redo();
        assert_eq!(edit.error().0, "Nothing to redo.");
        edit.undo();
        click(&mut edit, at(300.0), 0.0);
        edit.redo();
        assert_eq!(
            edit.points[2],
            [300.0, 0.0],
            "a new gate forgets the undone"
        );
        assert_eq!(edit.points.len(), 3);
    }

    #[test]
    fn edit_checkpoint_gives_a_picked_one_a_new_number() {
        let mut edit = draft(0, &[[0.0, 0.0], [100.0, 0.0], [200.0, 0.0]]);
        edit.facings = vec![None, None, Some(0.5)];
        edit.widths = vec![20.0, 20.0, 8.0];
        edit.renumber(1);
        assert!(
            edit.error().0.starts_with("Click a checkpoint"),
            "nothing picked"
        );
        edit.use_tool(Place::Number);
        edit.picked = Some(2);
        assert_eq!(edit.view().picked, 3);
        edit.renumber(4);
        assert_eq!(
            edit.error().0,
            "A checkpoint's number is 1 (the start) to 3."
        );
        // 3 to 1: the start, and the others one on, each with its facing and width.
        edit.renumber(1);
        assert_eq!(edit.points, vec![[200.0, 0.0], [0.0, 0.0], [100.0, 0.0]]);
        assert_eq!(edit.facings, vec![Some(0.5), None, None]);
        assert_eq!(edit.widths, vec![8.0, 20.0, 20.0]);
        assert_eq!(edit.note(), "Checkpoint 3 is now the start.");
        assert_eq!(edit.picked, Some(0), "still picked where it went");
        edit.renumber(2);
        assert_eq!(edit.points[1], [200.0, 0.0]);
        edit.use_tool(Place::Move);
        assert_eq!(edit.view().picked, 0, "another tool drops the pick");
    }

    #[test]
    fn objects_place_turned_and_undo_in_the_order_things_were_placed() {
        let mut edit = draft(0, &[]);
        edit.tool = Place::Checkpoint;
        click(&mut edit, wow_to_bevy([0.0, 0.0, 0.0]), 0.0);
        edit.use_tool(Place::Object(1));
        edit.object_turn = [0.5, 0.25, 0.1];
        click(&mut edit, wow_to_bevy([5.0, 6.0, 7.0]), 0.0);
        assert_eq!(edit.objects.len(), 1);
        let o = &edit.objects[0];
        assert_eq!(
            (o.model.as_str(), o.yaw, o.pitch, o.roll),
            (KINDS[1].key, 0.5, 0.25, 0.1)
        );
        assert!((o.x - 5.0).abs() < 1e-4 && (o.y - 6.0).abs() < 1e-4 && (o.z - 7.0).abs() < 1e-4);
        assert_eq!(edit.tool, Place::Object(1), "the tool stays, for another");
        assert_eq!(edit.view().object, Some(KINDS[1].key));
        assert_eq!(edit.laid_objects().len(), 1);
        edit.held = Some(Held::Object(0));
        assert!(edit.laid_objects().is_empty(), "held, the ghost shows it");
        edit.held = None;
        // Undo takes the object, placed last, then the gate; redo puts them back in turn.
        edit.undo();
        assert!(edit.objects.is_empty() && edit.points.len() == 1);
        edit.undo();
        assert!(edit.points.is_empty());
        edit.redo();
        edit.redo();
        assert_eq!((edit.points.len(), edit.objects.len()), (1, 1));
        // Deleted, undo goes on to the gate; a new draft has none.
        edit.remove_object(0);
        assert!(edit.objects.is_empty());
        edit.undo();
        assert!(edit.points.is_empty());
        edit.objects.push(Placed {
            model: KINDS[0].key.into(),
            x: 0.0,
            y: 0.0,
            z: 0.0,
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            size: None,
        });
        edit.clear();
        assert!(edit.objects.is_empty());
    }

    #[test]
    fn a_ring_hangs_by_its_radius_turned_and_keeps_its_place_in_the_course() {
        let mut edit = draft(0, &[]);
        edit.tool = Place::Checkpoint;
        click(&mut edit, wow_to_bevy([0.0, 0.0, 5.0]), 0.0);
        edit.use_tool(Place::Ring);
        assert_eq!(edit.view().tool, "ring");
        assert!(edit.view().turns_object, "its keys turn it");
        (edit.width, edit.lift, edit.tilt) = (6.0, 3.0, [0.4, 0.1]);
        click(&mut edit, wow_to_bevy([100.0, 0.0, 5.0]), 0.25);
        assert_eq!(edit.tool, Place::Ring, "the tool stays");
        assert_eq!(edit.rings, vec![None, Some([0.4, 0.1])]);
        assert_eq!(edit.facings[1], Some(0.25));
        let z = edit.heights[1].unwrap();
        assert!(
            (z - 14.0).abs() < 1e-3,
            "its bottom on the ground, raised by Ctrl: {z}"
        );
        // Undone and redone, renumbered: its tilt goes with it.
        edit.undo();
        assert_eq!(edit.rings, vec![None]);
        edit.redo();
        assert_eq!(edit.rings, vec![None, Some([0.4, 0.1])]);
        edit.picked = Some(1);
        edit.renumber(1);
        assert_eq!(edit.rings, vec![Some([0.4, 0.1]), None], "now the start");
        // Saved and opened again, it is a ring still.
        let (_, json) = edit.course("Ring Run", "Durotar").unwrap();
        let course: Course = serde_json::from_str(&json).unwrap();
        assert_eq!(course.rings(), vec![Some([0.4, 0.1]), None]);
        assert!((course.heights()[0].unwrap() - 14.0).abs() < 0.1);
        assert_eq!(course.gates()[0], 6.0);
    }

    #[test]
    fn erase_deletes_a_checkpoint_and_the_rest_close_up() {
        let mut edit = draft(0, &[[0.0, 0.0], [100.0, 0.0], [200.0, 0.0]]);
        edit.facings = vec![None, Some(0.5), None];
        edit.widths = vec![20.0, 8.0, 20.0];
        edit.undo();
        edit.erase(1);
        assert_eq!(edit.points, vec![[0.0, 0.0]]);
        assert_eq!(
            (edit.facings.clone(), edit.widths.clone()),
            (vec![None], vec![20.0])
        );
        assert_eq!(edit.note(), "Checkpoint 2 deleted.");
        edit.redo();
        assert_eq!(edit.points.len(), 1, "nothing to redo after a delete");
        edit.points.push([300.0, 0.0]);
        edit.facings.push(None);
        edit.widths.push(20.0);
        edit.erase(0);
        assert_eq!(edit.points, vec![[300.0, 0.0]], "the next is the start");
        assert!(edit.note().starts_with("Start deleted"));
        edit.erase(5);
        assert_eq!(edit.points.len(), 1, "no such checkpoint");
    }

    #[test]
    fn the_wheel_sizes_the_next_gate_and_the_save_keeps_it() {
        let mut edit = draft(0, &[]);
        edit.tool = Place::Checkpoint;
        let at = |x: f32| wow_to_bevy([x, 0.0, 0.0]);
        click(&mut edit, at(0.0), 0.0);
        edit.widen(-3.0);
        click(&mut edit, at(100.0), 0.0);
        edit.widen(100.0);
        assert_eq!(edit.width, WIDTH_MAX, "no wider than the server reaches");
        edit.widen(-100.0);
        assert_eq!(edit.width, WIDTH_MIN);
        assert_eq!(edit.widths, vec![GATE_HALF_WIDTH, GATE_HALF_WIDTH - 6.0]);
        let (_, json) = edit.course("Narrow", "").unwrap();
        let course: Course = serde_json::from_str(&json).unwrap();
        assert_eq!(course.gates(), vec![GATE_HALF_WIDTH, 14.0]);
        assert!(
            !json.contains("\"gate\": 20"),
            "the default stays implied: {json}"
        );
        edit.clear();
        assert_eq!(edit.width, GATE_HALF_WIDTH);
    }

    #[test]
    fn the_move_cursor_places_nothing_and_drags_a_gate_or_the_grid() {
        let mut edit = draft(0, &[]);
        assert_eq!(edit.tool, Place::Move, "a cursor first");
        let at = |x: f32, y: f32| wow_to_bevy([x, y, 0.0]);
        place(&mut edit, Place::Move, at(0.0, 0.0), 0.0);
        assert!(edit.points.is_empty());
        edit.tool = Place::Checkpoint;
        click(&mut edit, at(0.0, 0.0), 0.0);
        click(&mut edit, at(100.0, 0.0), 0.0);
        edit.widen(-2.0);
        click(&mut edit, at(200.0, 0.0), 0.0);
        edit.width = GATE_HALF_WIDTH;
        edit.grid = Some(Grid {
            x: -10.0,
            y: 0.0,
            z: 0.0,
            o: 1.0,
        });
        // Under the cursor: the nearest checkpoint's small marker, else nothing.
        assert_eq!(edit.under([102.0, 1.0]), Some(Held::Gate(1)));
        assert_eq!(
            edit.under([105.0, 10.0]),
            None,
            "a gate's width is not its marker"
        );
        assert_eq!(edit.under([-9.0, 0.0]), Some(Held::Grid));
        // Taken off its centre: a press with no drag moves nothing.
        edit.take(Held::Gate(2), [201.0, 2.0]);
        edit.drag(at(201.0, 2.0), 0.0);
        let p = edit.points[2];
        assert!((p[0] - 200.0).abs() < 1e-3 && p[1].abs() < 1e-3, "{p:?}");
        // Dragged: it follows the cursor with its width, turned as the turn is.
        assert_eq!(edit.width, 16.0);
        edit.turn = Some(0.5);
        edit.drag(at(251.0, 32.0), 0.0);
        edit.drop_held();
        let p = edit.points[2];
        assert!(
            (p[0] - 250.0).abs() < 1e-3 && (p[1] - 30.0).abs() < 1e-3,
            "{p:?}"
        );
        assert_eq!((edit.facings[2], edit.widths[2]), (Some(0.5), 16.0));
        assert_eq!(
            edit.width, GATE_HALF_WIDTH,
            "the next gate is the default again"
        );
        edit.take(Held::Grid, [-10.0, 0.0]);
        edit.drag(at(-20.0, 5.0), 1.0);
        let g = edit.grid.unwrap();
        assert!((g.x + 20.0).abs() < 1e-3 && (g.y - 5.0).abs() < 1e-3 && g.o == 1.0);
        // Undo lets go of what it takes away.
        edit.take(Held::Gate(2), [250.0, 30.0]);
        edit.drop_held();
        assert!(edit.held.is_none());
    }

    #[test]
    fn a_test_lap_without_a_grid_starts_where_a_race_s_first_slot_is() {
        // Down +x from the start: 4 yards behind it, 2 to the left of the line, facing +x.
        let spot = grid_spot(None, &[[100.0, 50.0], [200.0, 50.0]]).unwrap();
        assert_eq!(spot, [96.0, 48.0, 0.0]);
        let placed = Grid {
            x: 1.0,
            y: 2.0,
            z: 3.0,
            o: 0.5,
        };
        assert_eq!(grid_spot(Some(placed), &[]), Some([1.0, 2.0, 0.5]));
        assert_eq!(grid_spot(None, &[[0.0, 0.0]]), None);
    }

    #[test]
    fn a_save_lands_in_kart_tracks_starts_afresh_and_lists_the_track() {
        use crate::local_state::test_env::{EnvGuard, ENV_LOCK};
        let _l = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let home = std::env::temp_dir().join(format!("benilla-tracks-{}", std::process::id()));
        std::fs::remove_dir_all(&home).ok();
        let _c = EnvGuard::unset("WOW_CAPTURE");
        let _h = EnvGuard::set("BENILLA_HOME", home.to_str().unwrap());
        let mut edit = draft(0, &[[0.0, 0.0], [100.0, 0.0]]);
        let plank = Placed {
            model: KINDS[2].key.into(),
            x: 50.0,
            y: 2.0,
            z: 3.0,
            yaw: 0.5,
            pitch: 0.25,
            roll: 0.0,
            size: None,
        };
        edit.objects.push(plank.clone());
        edit.load();
        let saves = edit.saved().1;
        assert_eq!(edit.saved().0.count(), 0);
        edit.save("Short Dash", "Durotar", false);
        let file = home.join("kart-tracks").join("short-dash.json");
        assert!(file.exists(), "{}", edit.note);
        assert!(edit.note.contains("short-dash.json"));
        assert!(edit.points.is_empty(), "a new draft");
        edit.load();
        let (list, now) = edit.saved();
        assert_eq!(
            list.map(|c| c.0.name.clone()).collect::<Vec<_>>(),
            ["Short Dash"]
        );
        assert!(now > saves, "the browser hears of it");
        // Edited: back as the draft, on its own map alone.
        assert_eq!(edit.edit_saved("short-dash"), Ok("Short Dash".to_string()));
        assert_eq!(
            edit.take_named().as_deref(),
            Some("Short Dash"),
            "for the name box"
        );
        assert_eq!(edit.take_named(), None, "once");
        assert_eq!(edit.points.len(), 2);
        assert_eq!(edit.objects, [plank], "its objects as placed and turned");
        assert_eq!(edit.tool, Place::Move);
        // Taken to its start (a race's first slot, with no grid) from afar, not from near it;
        // the editor's left button waits for the kart to land there, or for TRIP_SECS.
        assert!(edit.go_to_start(1.0, Vec3::ZERO).is_none(), "there already");
        let far = wow_to_bevy([900.0, 0.0, 0.0]);
        let line = edit.go_to_start(1.0, far).unwrap();
        assert_eq!(line, "KARTCRAFT\tR\tTG\t-4.0\t-2.0\t0.000");
        assert!(!edit.owns_left(), "not editing before it lands");
        edit.trip_step(3.0, None);
        edit.trip_step(3.0, Some(far));
        assert!(
            edit.trip.is_some(),
            "the kart away for the teleport, or not moved yet"
        );
        edit.trip_step(4.0, Some(wow_to_bevy([-4.0, -2.0, 0.0])));
        assert!(edit.trip.is_none() && edit.owns_left(), "landed");
        edit.go_to_start(10.0, far);
        edit.trip_step(10.0 + TRIP_SECS + 0.1, Some(far));
        assert!(edit.trip.is_none(), "given up on");
        edit.map = 1;
        assert!(edit.edit_saved("short-dash").is_err());
        assert_eq!(edit.points.len(), 2, "a refusal keeps the draft");
        // Saved again under its name (any spelling of its key): the popup warns, and only an
        // overwrite writes it.
        assert!(edit.is_saved("short dash!") && !edit.is_saved("Long Dash"));
        let errors = edit.error().1;
        edit.save("short dash!", "Durotar", false);
        assert_eq!(
            edit.error(),
            ("A track named short dash! is saved already.", errors + 1)
        );
        assert_eq!(edit.points.len(), 2, "the draft kept");
        edit.save("short dash!", "Durotar", true);
        assert!(edit.points.is_empty(), "{}", edit.note);
        // Deleted: gone from the folder and the list; a path is no key.
        edit.delete("../short-dash");
        assert!(file.exists());
        edit.delete("short-dash");
        assert!(!file.exists(), "{}", edit.note);
        edit.load();
        assert_eq!(edit.saved().0.count(), 0);
        std::fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn the_bar_reads_editing_testing_and_a_saved_track_s_run() {
        let mut edit = draft(3, &[[0.0, 0.0]]);
        assert_eq!(edit.view().state, "edit");
        assert!(edit.preview("Me").is_some());
        edit.testing = true;
        assert_eq!(edit.view().state, "test");
        assert!(
            edit.preview("Me").is_none(),
            "the test lap's own gates show"
        );
        edit.on = false;
        assert_eq!(edit.view().state, "run");
        edit.testing = false;
        assert_eq!(edit.view().state, "");
    }

    /// Snapping starts off; on, a block placed beside another meets it, side to side, and
    /// the bar says so.
    #[test]
    fn snapping_is_off_until_the_bar_turns_it_on_then_a_block_meets_its_neighbour() {
        let mut edit = draft(0, &[]);
        assert!(!edit.view().snap);
        let models = super::super::objects::Models::default();
        let block = |x: f32| Placed {
            model: "block-grate".into(),
            x,
            y: 0.0,
            z: 0.0,
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            size: None,
        };
        edit.objects.push(block(0.0));
        let [lo, hi] = models.bounds(&block(0.0)).expect("a piece's box");
        let side = hi.x - lo.x;
        let ghost = block(side + 1.0);
        assert!(edit.snap_onto(&ghost, None, &models).is_none(), "off");
        edit.snap = true;
        assert!(edit.view().snap);
        let (p, _) = edit.snap_onto(&ghost, None, &models).expect("on, in reach");
        assert!((p.x - side).abs() < 1e-3, "{} vs {side}", p.x);
        assert!(
            edit.snap_onto(&block(0.5), Some(0), &models).is_none(),
            "not onto itself"
        );
    }
}
