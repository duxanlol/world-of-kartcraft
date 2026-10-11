//! The Race Marshals on the minimap and the world map. Each marshal spot of `courses.json`
//! (one per course, many courses to a spot) is a landmark, as an `AreaPOI.dbc` row would be:
//! - on the minimap, its flag icon in range, else a rim arrow among the nearest three;
//! - on the world map, a landmark named for the courses starting there.
//!
//! `KartRaceMarshals` shows them; the minimap button's menu toggles it, as a tracking type.

use benilla_formats::AreaPoi;
use bevy::prelude::*;

use super::race::COURSES;

/// The marshals' icon, `POIIcons.blp`'s red flag (the starter's).
const MARSHAL_ICON: u32 = 6;
/// `AreaPOI` `Flags`: a landmark candidate (bit 0) that draws its icon in range (bit 1).
const MARSHAL_FLAGS: u32 = 0x3;
/// A spot's tooltip names this many of its courses, then counts the rest.
const NAMED_COURSES: usize = 3;

/// The spots, built once, and whether they show. Only [`on_cvar`], an observer, writes it, so the
/// map systems reading it need no order against any system.
#[derive(Resource)]
pub(crate) struct RaceMarshals {
    pub(crate) shown: bool,
    spots: Vec<AreaPoi>,
}

impl Default for RaceMarshals {
    fn default() -> Self {
        Self {
            shown: true,
            spots: spots(),
        }
    }
}

impl RaceMarshals {
    /// The marshals on `map`, none while hidden.
    pub(crate) fn on_map(&self, map: u32) -> impl Iterator<Item = &AreaPoi> {
        self.spots
            .iter()
            .filter(move |p| self.shown && p.continent_id == map)
    }
}

/// One landmark per marshal spot, the courses there in file order.
fn spots() -> Vec<AreaPoi> {
    let mut spots: Vec<(u32, [f32; 3], Vec<&str>)> = Vec::new();
    for course in COURSES.iter() {
        let Some(m) = &course.marshal else {
            continue;
        };
        let at = [m.x, m.y, m.z];
        match spots
            .iter_mut()
            .find(|s| s.0 == course.map && (s.1[0] - m.x).hypot(s.1[1] - m.y) < 1.0)
        {
            Some(spot) => spot.2.push(&course.name),
            None => spots.push((course.map, at, vec![&course.name])),
        }
    }
    spots
        .into_iter()
        .map(|(map, pos, courses)| AreaPoi {
            importance: 0,
            icon: MARSHAL_ICON,
            faction_id: 0,
            pos,
            continent_id: map,
            flags: MARSHAL_FLAGS,
            area_id: u32::MAX,
            name: "Race Marshal".into(),
            description: describe(&courses),
            world_state_id: 0,
        })
        .collect()
}

/// "Elwynn Loop, Northshire Sprint and 4 more".
fn describe(courses: &[&str]) -> String {
    let named = courses[..courses.len().min(NAMED_COURSES)].join(", ");
    match courses.len().saturating_sub(NAMED_COURSES) {
        0 => named,
        more => format!("{named} and {more} more"),
    }
}

/// `KartRaceMarshals` as it moves, the saved value at load included.
fn on_cvar(ev: On<crate::cvars::CvarChanged>, mut marshals: ResMut<RaceMarshals>) {
    if ev.is("KartRaceMarshals") {
        marshals.shown = ev.flag();
    }
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<RaceMarshals>().add_observer(on_cvar);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_marshal_spot_is_a_landmark_naming_its_courses() {
        let all = spots();
        let courses = COURSES.iter().filter(|c| c.marshal.is_some()).count();
        assert!(!all.is_empty() && all.len() <= courses);
        assert!(all
            .iter()
            .all(|p| p.flags == MARSHAL_FLAGS && !p.description.is_empty()));
        let course = COURSES
            .iter()
            .find(|c| c.marshal.is_some())
            .expect("a course");
        let m = course.marshal.as_ref().expect("marshal");
        let spot = all
            .iter()
            .find(|p| p.continent_id == course.map && (p.pos[0] - m.x).hypot(p.pos[1] - m.y) < 1.0)
            .expect("Elwynn's marshal");
        assert!(
            spot.description.contains(&course.name),
            "{}",
            spot.description
        );

        let mut marshals = RaceMarshals::default();
        assert!(marshals.on_map(course.map).next().is_some());
        marshals.shown = false;
        assert!(marshals.on_map(course.map).next().is_none(), "hidden");
    }

    #[test]
    fn a_crowded_spot_names_three_and_counts_the_rest() {
        assert_eq!(describe(&["A"]), "A");
        assert_eq!(describe(&["A", "B", "C", "D", "E"]), "A, B, C and 2 more");
    }
}
