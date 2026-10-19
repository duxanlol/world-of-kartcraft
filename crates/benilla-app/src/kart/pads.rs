//! The turbo pads of the track we are on, laid on the ground ([`kartcraft::pads`]) and fired by
//! our kart as it drives over them: a course's (`courses.json`, `docs/courses.md`, its `"pads"`)
//! and a track's placed ones (the track editor's turbo pad objects, `kart/objects.rs`) in a race
//! or test lap on it, the draft's while the editor is open, and none in the open world (the hub's
//! props keep theirs). Client-side: the server knows nothing of them.

use benilla_world::world_map::CurrentMap;
use bevy::prelude::*;
use kartcraft::pads::{Pad, PadField};

#[derive(serde::Deserialize)]
struct Spot {
    x: f32,
    y: f32,
    /// Absent: the ground is found under the camera's height.
    #[serde(default)]
    z: f32,
    o: f32,
}

#[derive(serde::Deserialize)]
struct CoursePads {
    key: String,
    #[serde(default)]
    pads: Vec<Spot>,
}

/// Every course's pads by its key.
fn all_pads(json: &str) -> Vec<(String, Pad)> {
    #[derive(serde::Deserialize)]
    struct File {
        courses: Vec<CoursePads>,
    }
    let file: File = serde_json::from_str(json).expect("courses/courses.json");
    let mut out: Vec<(String, Pad)> = Vec::new();
    for course in file.courses {
        for s in course.pads {
            out.push((course.key.clone(), Pad::from_wow(s.x, s.y, s.z, s.o)));
        }
    }
    out
}

/// Every course's pads, by its key.
static PADS: std::sync::LazyLock<Vec<(String, Pad)>> =
    std::sync::LazyLock::new(|| all_pads(include_str!("../../../../courses/courses.json")));

/// A track's placed turbo pads (the editor's objects), each draped on the ground under where it
/// was placed, pointing as it was turned.
fn placed_pads(objects: &[super::objects::Placed]) -> impl Iterator<Item = Pad> + '_ {
    objects
        .iter()
        .filter(|o| super::objects::is_pad(&o.model))
        .map(|o| Pad::from_wow(o.x, o.y, o.z, o.yaw))
}

/// The pads of the track we are on: the draft's while editing, else the race's (a course's own
/// and its placed ones), else none.
fn track_pads(edit: &super::track_edit::TrackEdit, race: Option<&super::race::Active>) -> Vec<Pad> {
    if edit.view().state == "edit" {
        let draft: Vec<_> = edit.laid_objects().into_iter().map(|(_, o)| o).collect();
        return placed_pads(&draft).collect();
    }
    let Some(race) = race else {
        return Vec::new();
    };
    let course = PADS
        .iter()
        .filter(|(key, _)| key == race.course_key())
        .map(|(_, p)| *p);
    course.chain(placed_pads(race.objects())).collect()
}

/// The props' pads ([`super::props`]: the stunt park's, Skyhop's), out with the props, at their
/// own heights: the world's ground under Skyhop's is the desert floor far below.
static PROP_PADS: std::sync::LazyLock<Vec<(u32, Pad)>> = std::sync::LazyLock::new(|| {
    super::props::FILE
        .pads
        .iter()
        .map(|p| (p.map, Pad::from_wow(p.x, p.y, p.z, p.o).fixed()))
        .collect()
});

/// Hands the track's pads (and the map's props') to the field, and the ones laid so far to the
/// session.
pub(super) fn feed(
    mut commands: Commands,
    map: Option<Res<CurrentMap>>,
    (edit, races): (Res<super::track_edit::TrackEdit>, Res<super::race::Races>),
    mut field: ResMut<PadField>,
    mut host: ResMut<super::Host>,
) {
    let mut pads = track_pads(&edit, races.race.as_ref());
    if let Some(map) = map {
        let props = PROP_PADS.iter().filter(|_| super::props::enabled());
        pads.extend(props.filter(|(m, _)| *m == map.0).map(|(_, p)| *p));
    }
    field.set(pads, &mut commands);
    if let Some(laid) = field.take_changed() {
        host.session.set_pads(laid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_s_pads_parse_by_course() {
        let json = r#"{"courses": [
            {"key": "a", "map": 1, "pads": [{"x": 10, "y": 20, "z": 5, "o": 0}]},
            {"key": "b", "map": 1, "pads": [{"x": 10.5, "y": 20, "o": 0}, {"x": 90, "y": 20, "o": 1}]},
            {"key": "d", "map": 0}
        ]}"#;
        let pads = all_pads(json);
        assert_eq!(pads.len(), 3);
        assert_eq!(pads.iter().filter(|(k, _)| k == "b").count(), 2);
        // The shipped file parses, pads or none.
        let _ = &*PADS;
    }

    /// A track's pads are its own: none in the open world, a course's and its placed ones in a
    /// race on it, the draft's while the editor is open.
    #[test]
    fn pads_belong_to_the_track_they_are_on() {
        use super::super::objects::{Placed, TURBO_PAD};
        let edit = super::super::track_edit::TrackEdit::default();
        assert!(
            track_pads(&edit, None).is_empty(),
            "the open world has none"
        );
        let course = PADS.first().map(|(k, _)| k.clone());
        let placed = |model: &str| Placed {
            model: model.into(),
            x: 1.0,
            y: 2.0,
            z: 3.0,
            yaw: 0.5,
            pitch: 0.0,
            roll: 0.0,
        };
        let race = super::super::race::Active::practice("Loop", 1, vec![[0.0, 0.0]; 3], None, "Me")
            .with_objects(vec![placed(TURBO_PAD), placed("excavation-plank")]);
        assert_eq!(
            track_pads(&edit, Some(&race)),
            vec![Pad::from_wow(1.0, 2.0, 3.0, 0.5)]
        );
        if let Some(key) = course {
            let on = race.with_course_key(&key);
            let theirs = PADS.iter().filter(|(k, _)| *k == key).count();
            assert_eq!(track_pads(&edit, Some(&on)).len(), theirs + 1);
        }
    }
}
