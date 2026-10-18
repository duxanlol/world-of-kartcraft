//! The turbo pads of `courses.json` (`docs/courses.md`, a course's `"pads"`), every course's on
//! the map we are on, laid on the ground ([`kartcraft::pads`]) and fired by our kart as it drives
//! over them, in a race or not. Client-side: the server knows nothing of them.

use benilla_world::world_map::CurrentMap;
use bevy::prelude::*;
use kartcraft::pads::{Pad, PadField};

/// Pads of different courses closer than this (yards) are one pad: courses share roads.
const SAME_PAD: f32 = 2.0;

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
    #[serde(default)]
    map: u32,
    #[serde(default)]
    pads: Vec<Spot>,
}

/// Every course's pads with their map, each spot once.
fn all_pads(json: &str) -> Vec<(u32, Pad)> {
    #[derive(serde::Deserialize)]
    struct File {
        courses: Vec<CoursePads>,
    }
    let file: File = serde_json::from_str(json).expect("courses/courses.json");
    let mut out: Vec<(u32, Pad)> = Vec::new();
    for course in file.courses {
        for s in course.pads {
            let pad = Pad::from_wow(s.x, s.y, s.z, s.o);
            let seen = out
                .iter()
                .any(|(m, p)| *m == course.map && (p.centre - pad.centre).xz().length() < SAME_PAD);
            if !seen {
                out.push((course.map, pad));
            }
        }
    }
    out
}

/// Every course's pads.
static PADS: std::sync::LazyLock<Vec<(u32, Pad)>> =
    std::sync::LazyLock::new(|| all_pads(include_str!("../../../../courses/courses.json")));

/// The props' pads ([`super::props`]: the stunt park's, Skyhop's), out with the props, at their
/// own heights: the world's ground under Skyhop's is the desert floor far below.
static PROP_PADS: std::sync::LazyLock<Vec<(u32, Pad)>> = std::sync::LazyLock::new(|| {
    super::props::FILE
        .pads
        .iter()
        .map(|p| (p.map, Pad::from_wow(p.x, p.y, p.z, p.o).fixed()))
        .collect()
});

/// Hands the map's pads to the field, and the ones laid so far to the session.
pub(super) fn feed(
    mut commands: Commands,
    map: Option<Res<CurrentMap>>,
    mut field: ResMut<PadField>,
    mut host: ResMut<super::Host>,
) {
    let pads = map.map_or_else(Vec::new, |map| {
        let props = PROP_PADS.iter().filter(|_| super::props::enabled());
        PADS.iter()
            .chain(props)
            .filter(|(m, _)| *m == map.0)
            .map(|(_, p)| *p)
            .collect()
    });
    field.set(pads, &mut commands);
    if let Some(laid) = field.take_changed() {
        host.session.set_pads(laid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_s_pads_parse_and_shared_spots_are_one_pad() {
        let json = r#"{"courses": [
            {"key": "a", "map": 1, "pads": [{"x": 10, "y": 20, "z": 5, "o": 0}]},
            {"key": "b", "map": 1, "pads": [{"x": 10.5, "y": 20, "o": 0}, {"x": 90, "y": 20, "o": 1}]},
            {"key": "c", "map": 0, "pads": [{"x": 10, "y": 20, "o": 0}]},
            {"key": "d", "map": 0}
        ]}"#;
        let pads = all_pads(json);
        assert_eq!(pads.len(), 3);
        assert_eq!(pads.iter().filter(|(m, _)| *m == 1).count(), 2);
        // The shipped file parses, pads or none.
        let _ = &*PADS;
    }
}
