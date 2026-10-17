//! The track editor: the minimap's track button (`KartTrackEdit`) flies the free camera, and the
//! editor's bar (`KartcraftFrames.xml`, through `KartTrackCmd`) places the start, the
//! checkpoints and the starting grid where the camera aims, drives the draft as a test lap
//! ([`Active::practice`]) and saves it as a `courses/courses.json` course in
//! `benilla-config/kart-tracks/` (docs/courses.md). The saved tracks go to the track browser
//! (`BENILLA_KART_MYTRACKS`), which runs a test lap of one.

use benilla_assets::coords::{bevy_to_wow, wow_to_bevy};
use benilla_world::collision::WorldCollision;
use bevy::prelude::*;

use super::freecam::{FreeCam, FreeMode};
use super::race::{Active, Course, Races, COURSES, GATE_HALF_WIDTH};

/// How far the camera's aim reaches for the ground (yards).
const AIM_REACH: f32 = 600.0;
/// How far a pixel of the mouse turns the gate about to be placed, middle button held (radians),
/// and how fast Q and E held turn it (radians a second).
const TURN_PER_PIXEL: f32 = 0.01;
const TURN_PER_SEC: f32 = 1.5;
/// A saved track's note, for whoever adds it to courses.json.
const NOTE: &str = "Made with the in-game track editor.";
/// The aim's colour, and the grid's.
const AIM_COLOR: Color = Color::srgb(1.0, 0.85, 0.2);
const GRID_COLOR: Color = Color::srgb(0.3, 1.0, 0.4);

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

/// A saved track, read back from `kart-tracks/`.
struct Saved {
    course: Course,
    grid: Option<Grid>,
}

/// What a left click in the world places, as the bar's buttons pick it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    Start,
    Checkpoint,
    Grid,
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
    /// The facing the gate about to be placed is turned to (middle mouse); `None` square to the
    /// way from the last gate.
    turn: Option<f32>,
    grid: Option<Grid>,
    /// 0 a sprint, else the laps.
    laps: u8,
    /// The map the draft is on, from where the editor opened.
    map: u32,
    /// The tool picked; [`TrackEdit::tool`] is the one a click uses.
    tool: Place,
    /// The saved tracks once read, and how often the list changed: each change goes to the
    /// track browser.
    saved: Option<Vec<Saved>>,
    saves: u32,
    /// The last word for the player, and how many words so far: each new one goes to chat.
    note: String,
    notes: u32,
}

impl Default for TrackEdit {
    fn default() -> Self {
        Self {
            on: false,
            testing: false,
            points: Vec::new(),
            facings: Vec::new(),
            turn: None,
            grid: None,
            laps: 3,
            map: 0,
            tool: Place::Start,
            saved: None,
            saves: 0,
            note: String::new(),
            notes: 0,
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
    pub(crate) notes: u32,
    /// What a click places: `start`, `checkpoint` or `grid`.
    pub(crate) tool: &'static str,
}

impl TrackEdit {
    fn say(&mut self, note: impl Into<String>) {
        self.note = note.into();
        self.notes += 1;
    }

    /// The draft for its gates and the race map: while editing, not while testing.
    pub(crate) fn preview(&self, me: &str) -> Option<Active> {
        (self.on && !self.testing && !self.points.is_empty()).then(|| {
            Active::draft(self.laps, self.points.clone(), me).with_facings(self.facings.clone())
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
            notes: self.notes,
            tool: match self.tool() {
                Place::Start => "start",
                Place::Checkpoint => "checkpoint",
                Place::Grid => "grid",
            },
        }
    }

    /// What a click places: the tool picked, but the start while there is none.
    fn tool(&self) -> Place {
        match self.tool {
            Place::Checkpoint if self.points.is_empty() => Place::Start,
            tool => tool,
        }
    }

    /// The last note.
    pub(crate) fn note(&self) -> &str {
        &self.note
    }

    /// The saved tracks as the track browser lists courses ([`crate::ui_kart`]), each with its
    /// grid (`x,y,o` or empty) and its gates as a publish sends them (`x,y[,o];...`), and how
    /// often the list has changed.
    pub(crate) fn saved(&self) -> (impl Iterator<Item = (&Course, String, String)>, u32) {
        let list = self.saved.iter().flatten().map(|s| {
            let grid = s
                .grid
                .map_or_else(String::new, |g| format!("{:.1},{:.1},{:.3}", g.x, g.y, g.o));
            (
                &s.course,
                grid,
                spots(&s.course.route(), &s.course.facings()),
            )
        });
        (list, self.saves)
    }

    /// A new draft: no gates, no grid.
    fn clear(&mut self) {
        self.points.clear();
        self.facings.clear();
        self.turn = None;
        self.grid = None;
        self.tool = Place::Start;
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
        let json = serde_json::to_string_pretty(&course).map_err(|e| e.to_string())?;
        Ok((key, json + "\n"))
    }

    /// Saves the draft as `name` in `benilla-config/kart-tracks/<key>.json`, the editor going on
    /// with a new one; the note says how it went.
    fn save(&mut self, name: &str, zone: &str) {
        let saved = self.course(name, zone).and_then(|(key, json)| {
            let dir = crate::local_state::kart_tracks_dir()
                .ok_or_else(|| "This build keeps no files.".to_string())?;
            let path = dir.join(format!("{key}.json"));
            crate::local_state::write_atomic(&path, &json).map_err(|e| e.to_string())?;
            Ok(path)
        });
        match saved {
            Ok(path) => {
                info!("kart track editor: saved {}", path.display());
                self.say(format!(
                    "Track saved to {} (in the track browser). A new track begins.",
                    path.display()
                ));
                self.clear();
                self.saved = None;
            }
            Err(why) => self.say(why),
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
            _ => self.say("That track could not be deleted."),
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
                Ok(Saved { course, grid })
            });
            parsed
                .inspect_err(|e| warn!("kart track editor: {}: {e}", path.display()))
                .ok()
        })
        .collect()
}

/// Gates as a publish and a player track's list line carry them: `x,y[,o];...`, the start first.
fn spots(points: &[[f32; 2]], facings: &[Option<f32>]) -> String {
    let spot = |(i, p): (usize, &[f32; 2])| match facings.get(i).copied().flatten() {
        Some(o) => format!("{:.1},{:.1},{o:.3}", p[0], p[1]),
        None => format!("{:.1},{:.1}", p[0], p[1]),
    };
    points
        .iter()
        .enumerate()
        .map(spot)
        .collect::<Vec<_>>()
        .join(";")
}

/// Gates (WoW x, y) and each one's facing where turned.
type Gates = (Vec<[f32; 2]>, Vec<Option<f32>>);

/// `x,y[,o];...` back into gates and their facings; `None` if any spot is not numbers.
fn parse_spots(text: &str) -> Option<Gates> {
    let mut points = Vec::new();
    let mut facings = Vec::new();
    for spot in text.split(';').filter(|s| !s.is_empty()) {
        let v: Vec<&str> = spot.split(',').collect();
        let num = |i: usize| v.get(i).and_then(|t| t.trim().parse::<f32>().ok());
        points.push([num(0)?, num(1)?]);
        facings.push(match v.get(2) {
            Some(t) if !t.is_empty() => Some(num(2)?),
            _ => None,
        });
    }
    Some((points, facings))
}

/// A name as courses.json takes one: ASCII, no quotes, backslashes or control characters.
fn clean(name: &str) -> String {
    let kept: String = name
        .chars()
        .filter(|c| c.is_ascii() && !c.is_ascii_control() && *c != '"' && *c != '\\')
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

/// `KartTrackEdit` opens and closes the editor; `KartTrackCmd` is the bar's buttons and the track
/// browser's test lap.
// ponytail: a CVar per press, `<n>\t<verb>\t<args>` with n counting so a repeat still changes
// it; a Lua-to-Rust call would do better if the editor grows.
pub(crate) fn on_cvar(
    ev: On<crate::cvars::CvarChanged>,
    mut edit: ResMut<TrackEdit>,
    mut free: ResMut<FreeCam>,
    mut races: ResMut<Races>,
    drive: Res<super::KartDrive>,
    current_map: Option<Res<benilla_world::world_map::CurrentMap>>,
) {
    let testing = races.race.as_ref().is_some_and(|r| r.practice);
    let map = current_map.map_or(0, |m| m.0);
    match ev.key().as_str() {
        "karttrackedit" if ev.flag() != edit.on => {
            if ev.flag() && races.in_race() && !testing {
                edit.say("Leave the race first to edit a track.");
                return;
            }
            edit.on = ev.flag();
            if testing {
                races.race = None;
            }
            if edit.on {
                // A draft the kart's loss closed goes on, on its own map.
                if map != edit.map {
                    edit.clear();
                }
                edit.map = map;
                free.set(FreeMode::Fly);
            } else {
                edit.clear();
                free.set(FreeMode::Off);
            }
        }
        "karttrackcmd" => {
            let words: Vec<&str> = ev.new.split('\t').skip(1).collect();
            let editing = edit.on && !testing;
            match words.as_slice() {
                // The bar picks what a left click in the world places; it places nothing itself.
                ["start", ..] if editing => edit.tool = Place::Start,
                ["place", ..] if editing && edit.points.is_empty() => {
                    edit.say("Place the start first: it is the first checkpoint, the line.");
                }
                ["place", ..] if editing => edit.tool = Place::Checkpoint,
                ["grid", ..] if editing => edit.tool = Place::Grid,
                ["undo", ..] if editing => {
                    edit.points.pop();
                    edit.facings.pop();
                }
                ["clear", ..] if editing => edit.clear(),
                ["laps", n, ..] if editing => edit.laps = n.parse().unwrap_or(0).min(9),
                ["practice", name, ..] if editing => match edit.unready() {
                    Some(why) => edit.say(why),
                    None => {
                        let name = Some(clean(name)).filter(|n| !n.is_empty());
                        let name = name.unwrap_or_else(|| "Test lap".into());
                        let grid = grid_spot(edit.grid, &edit.points);
                        let race = Active::practice(
                            &name,
                            edit.laps,
                            edit.points.clone(),
                            grid,
                            &races.me,
                        );
                        races.race = Some(race.with_facings(edit.facings.clone()));
                        free.set(FreeMode::Off);
                    }
                },
                ["save", name, rest @ ..] if editing => {
                    let zone = rest.first().copied().unwrap_or("");
                    edit.save(name, zone);
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
                        Some(s) if s.course.map != map => {
                            Some("That track is on another continent.")
                        }
                        Some(_) => None,
                    };
                    let race = run.map(|s| {
                        let c = &s.course;
                        let route = c.route();
                        let grid = grid_spot(s.grid, &route);
                        Active::practice(&c.name, c.laps, route, grid, &races.me)
                            .with_facings(c.facings())
                    });
                    if let Some(why) = why {
                        edit.say(why);
                        return;
                    }
                    races.race = race;
                    edit.on = false;
                    edit.clear();
                    free.set(FreeMode::Off);
                }
                // A saved track gone from the browser: its file deleted, the list read again.
                ["delete", key, ..] => edit.delete(key),
                // A player track's test lap, from the track browser's player tracks: its line's
                // name, laps, map, grid (`x,y,o` or empty) and gates.
                ["runspots", name, laps, track_map, grid, gates, ..] => {
                    let route = parse_spots(gates).filter(|(p, _)| p.len() >= 2);
                    let grid =
                        parse_spots(grid).and_then(|(p, o)| Some([p.first()?[0], p[0][1], o[0]?]));
                    let why = match &route {
                        None => Some("That track's checkpoints did not arrive whole."),
                        Some(_) if !drive.active => Some("Summon your kart first (J)."),
                        Some(_) if races.in_race() && !testing => {
                            Some("Leave the race first to test a track.")
                        }
                        Some(_) if track_map.parse::<u32>().ok() != Some(map) => {
                            Some("That track is on another continent.")
                        }
                        Some(_) => None,
                    };
                    if let Some(why) = why {
                        edit.say(why);
                        return;
                    }
                    let Some((points, facings)) = route else {
                        return;
                    };
                    let laps = laps.parse().unwrap_or(0);
                    let grid = grid.or_else(|| grid_spot(None, &points));
                    let race = Active::practice(&clean(name), laps, points, grid, &races.me);
                    races.race = Some(race.with_facings(facings));
                    edit.on = false;
                    edit.clear();
                    free.set(FreeMode::Off);
                }
                ["restart", ..] => {
                    if let Some(race) = races.race.as_mut().filter(|r| r.practice) {
                        race.restart_practice();
                    }
                }
                ["back", ..] if testing => races.race = None,
                _ => {}
            }
        }
        _ => {}
    }
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
    (buttons, mouse, over_ui, window, camera): (
        Res<ButtonInput<MouseButton>>,
        Res<bevy::input::mouse::AccumulatedMouseMotion>,
        Res<crate::ui_script::PointerOverUi>,
        Query<&Window, With<bevy::window::PrimaryWindow>>,
        Query<(&Camera, &GlobalTransform), With<benilla_world::view::WorldCamera>>,
    ),
    (time, keys, typing, overlay): (
        Res<Time>,
        Res<ButtonInput<KeyCode>>,
        Res<crate::ui_script::UiKeyboardCapture>,
        Res<crate::bindings::BindingOverlay>,
    ),
) {
    edit.load();
    // The controller's Fire places at the screen's centre; the keyboard's is E, which turns the
    // gate here, so the keyboard places with a click.
    let fired = overlay.pad_just_pressed(super::bindings::FIRE);
    let practice = races.race.as_ref().filter(|r| r.practice);
    edit.testing = practice.is_some();
    // `KartTrackEdit` may stay "1": the button sets "0" before its "1".
    if edit.on && !drive.active {
        edit.on = false;
        free.set(FreeMode::Off);
        edit.say("The track editor needs your kart: summon it (J) first.");
    }
    let editing = edit.on && !edit.testing;
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
            .map(|hit| from + way * hit.distance);
        ahead.or_else(|| {
            collide
                .ray_body(eye.translation, Dir3::NEG_Y, 4000.0)
                .map(|hit| eye.translation - Vec3::Y * hit.distance)
        })
    });
    // What is about to be placed faces as the middle mouse turns it, else a gate square to the
    // way from the last one, the start and the grid the way the camera looks.
    let tool = edit.tool();
    let facing = aim.zip(eye).map(|(at, eye)| {
        let w = bevy_to_wow(at);
        let from = match edit.points.last().filter(|_| tool == Place::Checkpoint) {
            Some(p) => [p[0], p[1]],
            None => {
                let back = bevy_to_wow(at - eye.forward() * 10.0);
                [back[0], back[1]]
            }
        };
        let auto = (w[1] - from[1]).atan2(w[0] - from[0]);
        if editing && buttons.pressed(MouseButton::Middle) {
            edit.turn = Some(edit.turn.unwrap_or(auto) - mouse.delta.x * TURN_PER_PIXEL);
        }
        // Q turns it left (WoW's facing grows anticlockwise), E right; not while typing a name.
        let held = |k: KeyCode| f32::from(u8::from(keys.pressed(k) && !typing.typing));
        let q_e = held(KeyCode::KeyQ) - held(KeyCode::KeyE);
        if editing && q_e != 0.0 {
            edit.turn = Some(edit.turn.unwrap_or(auto) + q_e * TURN_PER_SEC * time.delta_secs());
        }
        edit.turn.unwrap_or(auto)
    });
    // A left click on the world (never one on the interface), or Fire at the screen's centre.
    let clicked = buttons.just_pressed(MouseButton::Left) && cursor.is_some();
    if editing && (fired || clicked) {
        match aim.zip(facing) {
            Some((at, o)) => place(&mut edit, tool, at, o),
            None => edit.say("Aim at the ground to place it."),
        }
    }
    if let (Some(at), Some(o), Place::Grid) = (aim, facing, tool) {
        gizmos.circle(flat(at + Vec3::Y * 0.3), 6.0, GRID_COLOR);
        let w = bevy_to_wow(at);
        let ahead = wow_to_bevy([w[0] + 8.0 * o.cos(), w[1] + 8.0 * o.sin(), 0.0]).with_y(at.y);
        gizmos.arrow(at + Vec3::Y * 0.3, ahead + Vec3::Y * 0.3, GRID_COLOR);
    } else if let (Some(at), Some(o)) = (aim, facing) {
        gizmos.circle(flat(at), 2.0, AIM_COLOR);
        // Its posts either side, square to the way it faces, and an arrow along it.
        let w = bevy_to_wow(at);
        let side = [o.sin() * GATE_HALF_WIDTH, -o.cos() * GATE_HALF_WIDTH];
        let post = |k: f32| wow_to_bevy([w[0] + k * side[0], w[1] + k * side[1], 0.0]).with_y(at.y);
        let (left, right) = (post(1.0), post(-1.0));
        for p in [left, right] {
            gizmos.line(p, p + Vec3::Y * 8.0, AIM_COLOR);
        }
        gizmos.line(left + Vec3::Y * 8.0, right + Vec3::Y * 8.0, AIM_COLOR);
        let ahead = wow_to_bevy([w[0] + 8.0 * o.cos(), w[1] + 8.0 * o.sin(), 0.0]).with_y(at.y);
        gizmos.arrow(at + Vec3::Y * 0.5, ahead + Vec3::Y * 0.5, AIM_COLOR);
    }
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

/// Places `what` at `at` (Bevy world): a gate keeps its facing `o` only where the middle mouse
/// turned it, the grid always. Checkpoints come next.
fn place(edit: &mut TrackEdit, what: Place, at: Vec3, o: f32) {
    let w = bevy_to_wow(at);
    match what {
        Place::Start if edit.points.is_empty() => {
            edit.points.push([w[0], w[1]]);
            edit.facings.push(edit.turn.take());
            edit.say("Start placed: now click the checkpoints, in driving order.");
        }
        Place::Start => {
            edit.points[0] = [w[0], w[1]];
            edit.facings[0] = edit.turn.take();
            edit.say("Start moved.");
        }
        Place::Checkpoint => {
            edit.points.push([w[0], w[1]]);
            edit.facings.push(edit.turn.take());
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

    /// A left click at `at`, the gate or grid facing `o`.
    fn click(edit: &mut TrackEdit, at: Vec3, o: f32) {
        let tool = edit.tool();
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
        let text = spots(&[[1.0, 2.0], [3.04, -4.0]], &[None, Some(1.5)]);
        assert_eq!(text, "1.0,2.0;3.0,-4.0,1.500");
        let (points, facings) = parse_spots(&text).unwrap();
        assert_eq!(points, vec![[1.0, 2.0], [3.0, -4.0]]);
        assert_eq!(facings, vec![None, Some(1.5)]);
        assert!(parse_spots("1,2;x,4").is_none());
        assert_eq!(parse_spots("").unwrap().0.len(), 0);
    }

    #[test]
    fn a_name_makes_a_course_key() {
        assert_eq!(key_of("My  Track #2!"), "my-track-2");
        assert_eq!(key_of("  --  "), "");
        assert_eq!(key_of(&"a".repeat(40)).len(), 32);
        assert_eq!(clean(" Bob's \"Loop\"\\ "), "Bob's Loop");
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

    #[test]
    fn the_start_comes_first_and_moves_and_the_grid_faces_as_aimed() {
        let mut edit = draft(0, &[]);
        let at = |x: f32| wow_to_bevy([x, 0.0, 0.0]);
        // A click with nothing placed is the start, whatever the tool; checkpoints follow.
        edit.tool = Place::Checkpoint;
        assert_eq!(edit.tool(), Place::Start);
        click(&mut edit, at(10.0), 0.0);
        assert_eq!(edit.tool(), Place::Checkpoint);
        click(&mut edit, at(50.0), 0.0);
        edit.tool = Place::Start;
        edit.turn = Some(1.0);
        click(&mut edit, at(20.0), 1.0);
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
        edit.load();
        let saves = edit.saved().1;
        assert_eq!(edit.saved().0.count(), 0);
        edit.save("Short Dash", "Durotar");
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
}
