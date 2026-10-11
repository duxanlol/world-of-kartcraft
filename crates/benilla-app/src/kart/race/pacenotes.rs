//! Rally pace notes for a race: each checkpoint's turn graded the way a co-driver calls it, which
//! note the sign shows for where the kart is and where it faces, and the sign's glyphs.
//!
//! A turn is the heading change at a checkpoint, from the leg in to the leg out (WoW x north, y
//! west: a positive cross product of in × out turns left). Grades, by that change:
//! straight under 15°, easy to 45°, medium to 80°, hard to 135°, hairpin beyond. 15° is about what
//! a gate's own width absorbs: a 15–25 yd half-width over a 100–160 yd leg is 6–14° of slack, so a
//! smaller change is driven flat through the gate. Under 45° is held at a racing kart's speed, a
//! medium bend wants a lift, and from a little short of square (80°, so a right angle is never
//! read either side of the line) it wants the brake; past 135° the kart comes back on itself.
//!
//! Consecutive easy changes the same way (each 15–45°, up to [`MAX_LONG_GATES`] gates) are one
//! long bend, graded by their sum; a straight gate ends it. A bend that ends close before the next turn says so: "into"
//! within [`INTO_YARDS`], "then" within [`THEN_YARDS`] (about 1.2 s and 2.7 s at a racing kart's
//! 40 yd/s).

use bevy::math::Vec2;

/// Below this change (degrees) a checkpoint is straight; then easy, medium, hard; beyond hairpin.
pub(crate) const STRAIGHT_BELOW: f32 = 15.0;
pub(crate) const EASY_BELOW: f32 = 45.0;
pub(crate) const MEDIUM_BELOW: f32 = 80.0;
pub(crate) const HARD_BELOW: f32 = 135.0;
/// At most this many gates make one long bend.
pub(crate) const MAX_LONG_GATES: usize = 4;
/// A turn this near after a bend's last gate (yards) is called with it: "into", then "then".
pub(crate) const INTO_YARDS: f32 = 50.0;
pub(crate) const THEN_YARDS: f32 = 110.0;
/// The next checkpoint's note is called this far out at least (yards), or this long before the
/// kart reaches it at its speed (s); before that the sign shows straight.
pub(crate) const CALL_YARDS: f32 = 70.0;
pub(crate) const CALL_SECS: f32 = 4.0;
/// Facing more than this many degrees from both the next checkpoint and the leg to it is the
/// wrong way, except this near (yards) the last gate passed, where the kart is still turning.
pub(crate) const WRONG_WAY_DEG: f32 = 100.0;
pub(crate) const WRONG_WAY_GRACE_YARDS: f32 = 30.0;
/// Further than this from the leg being driven (yards) is off course.
pub(crate) const OFF_COURSE_YARDS: f32 = 60.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Side {
    Left,
    Right,
}

impl Side {
    fn word(self) -> &'static str {
        match self {
            Side::Left => "LEFT",
            Side::Right => "RIGHT",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum Grade {
    Straight,
    Easy,
    Medium,
    Hard,
    Hairpin,
}

impl Grade {
    /// The grade of a heading change of `deg` degrees (either way).
    pub(crate) fn of(deg: f32) -> Grade {
        let deg = deg.abs();
        if deg < STRAIGHT_BELOW {
            Grade::Straight
        } else if deg < EASY_BELOW {
            Grade::Easy
        } else if deg < MEDIUM_BELOW {
            Grade::Medium
        } else if deg < HARD_BELOW {
            Grade::Hard
        } else {
            Grade::Hairpin
        }
    }

    fn word(self) -> &'static str {
        match self {
            Grade::Straight => "STRAIGHT",
            Grade::Easy => "EASY",
            Grade::Medium => "MEDIUM",
            Grade::Hard => "HARD",
            Grade::Hairpin => "HAIRPIN",
        }
    }

    /// The sign's colour for this grade: white, green, yellow, orange, red.
    pub(crate) fn colour(self) -> [u8; 3] {
        match self {
            Grade::Straight => [255, 255, 255],
            Grade::Easy => [120, 255, 110],
            Grade::Medium => [255, 230, 60],
            Grade::Hard => [255, 140, 30],
            Grade::Hairpin => [255, 64, 50],
        }
    }
}

/// One bend: its grade, which way (none when straight), whether it runs over several gates, and
/// its whole heading change (degrees).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Bend {
    pub(crate) grade: Grade,
    pub(crate) side: Option<Side>,
    pub(crate) long: bool,
    pub(crate) degrees: f32,
    /// How many gates it spans (1 for a single turn).
    pub(crate) gates: usize,
}

impl Bend {
    const STRAIGHT: Bend = Bend {
        grade: Grade::Straight,
        side: None,
        long: false,
        degrees: 0.0,
        gates: 1,
    };

    fn new(signed: f32, gates: usize) -> Bend {
        let grade = Grade::of(signed);
        Bend {
            grade,
            side: (grade != Grade::Straight).then_some(if signed > 0.0 {
                Side::Left
            } else {
                Side::Right
            }),
            long: gates > 1,
            degrees: signed.abs(),
            gates,
        }
    }

    pub(crate) fn is_turn(&self) -> bool {
        self.side.is_some()
    }

    /// What a co-driver calls it: "HARD RIGHT", "HAIRPIN LEFT", "LONG RIGHT", "STRAIGHT".
    pub(crate) fn call(&self) -> String {
        let Some(side) = self.side else {
            return Grade::Straight.word().into();
        };
        match (self.long, self.grade) {
            (true, Grade::Easy | Grade::Medium) => format!("LONG {}", side.word()),
            (true, g) => format!("LONG {} {}", g.word(), side.word()),
            (false, g) => format!("{} {}", g.word(), side.word()),
        }
    }
}

/// How a following turn joins a bend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Link {
    Into,
    Then,
}

/// The note for one checkpoint: its bend and the turn that follows close behind, if any.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Note {
    pub(crate) bend: Bend,
    pub(crate) next: Option<(Link, Bend)>,
}

impl Note {
    /// The smaller line: "then HARD RIGHT", "into EASY LEFT".
    pub(crate) fn follow(&self) -> Option<String> {
        let (link, bend) = self.next?;
        let word = match link {
            Link::Into => "into",
            Link::Then => "then",
        };
        Some(format!("{word} {}", bend.call()))
    }
}

fn unit(a: [f32; 2], b: [f32; 2]) -> Option<Vec2> {
    Vec2::new(b[0] - a[0], b[1] - a[1]).try_normalize()
}

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    (b[0] - a[0]).hypot(b[1] - a[1])
}

/// The checkpoint after `i`: wrapping on a circuit, none past a sprint's finish.
fn after(n: usize, circuit: bool, i: usize) -> Option<usize> {
    if circuit {
        Some((i + 1) % n)
    } else {
        (i + 1 < n).then_some(i + 1)
    }
}

/// The heading change at checkpoint `i` (degrees, positive to the left), from the leg in to the
/// leg out; none at a sprint's start or finish.
pub(crate) fn turn_angle(points: &[[f32; 2]], circuit: bool, i: usize) -> Option<f32> {
    let n = points.len();
    if n < 3 || i >= n {
        return None;
    }
    let before = if circuit || i > 0 {
        (i + n - 1) % n
    } else {
        return None;
    };
    let out = after(n, circuit, i)?;
    let a = unit(points[before], points[i])?;
    let b = unit(points[i], points[out])?;
    Some(a.perp_dot(b).atan2(a.dot(b)).to_degrees())
}

/// The bend that starts at checkpoint `i`: its own turn, or a long bend of it and the easy
/// changes the same way after it.
fn bend_at(angles: &[Option<f32>], circuit: bool, i: usize) -> Bend {
    let n = angles.len();
    let Some(first) = angles[i] else {
        return Bend::STRAIGHT;
    };
    let easy = |a: f32| Grade::of(a) == Grade::Easy;
    if !easy(first) {
        return Bend::new(first, 1);
    }
    let (mut total, mut gates, mut j) = (first, 1, i);
    while gates < MAX_LONG_GATES {
        let Some(k) = after(n, circuit, j).filter(|&k| k != i) else {
            break;
        };
        match angles[k] {
            Some(a) if easy(a) && a.signum() == first.signum() => {
                total += a;
                gates += 1;
                j = k;
            }
            _ => break,
        }
    }
    Bend::new(total, gates)
}

/// Each checkpoint's note, in course order: on a circuit every checkpoint turns, wrapping round
/// through the start; on a sprint the start and the finish are straight.
pub(crate) fn notes(points: &[[f32; 2]], circuit: bool) -> Vec<Note> {
    let n = points.len();
    let angles: Vec<Option<f32>> = (0..n).map(|i| turn_angle(points, circuit, i)).collect();
    let bends: Vec<Bend> = (0..n).map(|i| bend_at(&angles, circuit, i)).collect();
    (0..n)
        .map(|i| {
            let bend = bends[i];
            // The bend's last gate, then the turn after it.
            let mut last = i;
            for _ in 1..bend.gates {
                last = after(n, circuit, last).unwrap_or(last);
            }
            let next = after(n, circuit, last)
                .filter(|&k| k != i && bend.is_turn() && bends[k].is_turn())
                .and_then(|k| {
                    let d = dist(points[last], points[k]);
                    if d <= INTO_YARDS {
                        Some((Link::Into, bends[k]))
                    } else if d <= THEN_YARDS {
                        Some((Link::Then, bends[k]))
                    } else {
                        None
                    }
                });
            Note { bend, next }
        })
        .collect()
}

/// What the pace-note sign shows.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Sign {
    /// The next checkpoint's note.
    Note(Note),
    /// Nothing to call yet: straight on, with the coming turn as the smaller line.
    Straight(Option<Bend>),
    /// The next checkpoint is the finish.
    Finish,
    /// Facing away from the course: turn round.
    WrongWay,
    /// Far off the leg: the next checkpoint lies this way (none: ahead).
    OffCourse(Option<Side>),
}

/// Where the kart is in the race, for [`choose`].
pub(crate) struct Place<'a> {
    pub(crate) points: &'a [[f32; 2]],
    pub(crate) notes: &'a [Note],
    /// The last checkpoint passed (the start before any) and the next to reach.
    pub(crate) last: usize,
    pub(crate) next: usize,
    /// The next checkpoint is the finish.
    pub(crate) finish: bool,
    /// The kart, WoW x, y; its facing (radians, WoW: 0 north, towards +y west positive); its
    /// speed (yd/s).
    pub(crate) at: [f32; 2],
    pub(crate) facing: f32,
    pub(crate) speed: f32,
}

/// The angle (degrees, positive left) from the facing `facing` to the direction `d`.
fn off_facing(facing: f32, d: Vec2) -> f32 {
    let f = Vec2::from_angle(facing);
    f.perp_dot(d).atan2(f.dot(d)).to_degrees()
}

/// The sign for the kart where it is: wrong way first, then off course, then the next
/// checkpoint's note once it is near enough to call, else straight.
pub(crate) fn choose(p: &Place) -> Sign {
    let target = p.points[p.next];
    let from = p.points[p.last];
    let to_target = dist(p.at, target);
    let bearing = unit(p.at, target).map(|d| off_facing(p.facing, d));
    let leg = unit(from, target);
    let along_leg = leg.map(|d| off_facing(p.facing, d));
    let wrong = |a: Option<f32>| a.is_none_or(|a| a.abs() > WRONG_WAY_DEG);
    if bearing.is_some()
        && wrong(bearing)
        && wrong(along_leg)
        && dist(p.at, from) > WRONG_WAY_GRACE_YARDS
    {
        return Sign::WrongWay;
    }
    // Off the leg: its distance from the segment between the last gate and the next.
    if let Some(d) = leg {
        let rel = Vec2::new(p.at[0] - from[0], p.at[1] - from[1]);
        let t = rel.dot(d).clamp(0.0, dist(from, target));
        if (rel - d * t).length() > OFF_COURSE_YARDS {
            let side = bearing.and_then(|b| {
                (b.abs() >= STRAIGHT_BELOW).then_some(if b > 0.0 {
                    Side::Left
                } else {
                    Side::Right
                })
            });
            return Sign::OffCourse(side);
        }
    }
    if p.finish {
        return Sign::Finish;
    }
    let note = p.notes.get(p.next).copied();
    let call = CALL_YARDS.max(p.speed * CALL_SECS);
    match note {
        Some(n) if n.bend.is_turn() && to_target <= call => Sign::Note(n),
        Some(n) => Sign::Straight(n.bend.is_turn().then_some(n.bend)),
        None => Sign::Straight(None),
    }
}

impl Sign {
    /// What the sign shows while it is being placed: a full note, so its whole size shows.
    pub(crate) fn sample() -> Sign {
        Sign::Note(Note {
            bend: Bend::new(-100.0, 1),
            next: Some((Link::Then, Bend::new(160.0, 1))),
        })
    }

    /// The big line and the smaller one under it.
    pub(crate) fn lines(&self) -> (String, Option<String>) {
        match self {
            Sign::Note(n) => (n.bend.call(), n.follow()),
            Sign::Straight(coming) => (
                "STRAIGHT".into(),
                coming.map(|b| format!("then {}", b.call())),
            ),
            Sign::Finish => ("FINISH".into(), None),
            Sign::WrongWay => ("WRONG WAY".into(), Some("turn around".into())),
            Sign::OffCourse(side) => (
                "OFF COURSE".into(),
                Some(match side {
                    Some(s) => format!("checkpoint to the {}", s.word().to_lowercase()),
                    None => "checkpoint ahead".into(),
                }),
            ),
        }
    }

    /// The glyph it shows.
    pub(crate) fn glyph(&self) -> Glyph {
        match self {
            Sign::Note(n) => Glyph::of(&n.bend),
            Sign::Straight(_) => Glyph::STRAIGHT,
            Sign::Finish => Glyph {
                colour: [120, 255, 110],
                ..Glyph::STRAIGHT
            },
            Sign::WrongWay => Glyph {
                shape: Shape::UTurn,
                side: Side::Left,
                colour: Grade::Hairpin.colour(),
            },
            Sign::OffCourse(side) => match side {
                Some(s) => Glyph {
                    shape: Shape::Bend(Grade::Medium, false),
                    side: *s,
                    colour: [110, 200, 255],
                },
                None => Glyph {
                    colour: [110, 200, 255],
                    ..Glyph::STRAIGHT
                },
            },
        }
    }
}

/// The arrow's shape: straight up, bent by a grade (long bends sweep wider), or a U-turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Shape {
    Straight,
    Bend(Grade, bool),
    UTurn,
}

/// A glyph: shape, which way it bends, its colour. Cached by the sign by this key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Glyph {
    pub(crate) shape: Shape,
    pub(crate) side: Side,
    pub(crate) colour: [u8; 3],
}

impl Glyph {
    pub(crate) const STRAIGHT: Glyph = Glyph {
        shape: Shape::Straight,
        side: Side::Right,
        colour: [255, 255, 255],
    };

    fn of(bend: &Bend) -> Glyph {
        let Some(side) = bend.side else {
            return Glyph::STRAIGHT;
        };
        let shape = if bend.grade == Grade::Hairpin {
            Shape::UTurn
        } else {
            Shape::Bend(bend.grade, bend.long)
        };
        Glyph {
            shape,
            side,
            colour: bend.grade.colour(),
        }
    }

    /// The arrow's centre line (path units, y up, starting at the tail) and the shaft's end
    /// direction, where the head goes on.
    fn path(&self) -> (Vec<Vec2>, Vec2) {
        // How far it turns (degrees, to the right) and the bend's radius.
        let (turn, radius, exit): (f32, f32, f32) = match self.shape {
            Shape::Straight => (0.0, 0.0, 0.0),
            Shape::Bend(g, long) => {
                let turn = match g {
                    Grade::Straight => 0.0,
                    Grade::Easy => 30.0,
                    Grade::Medium => 65.0,
                    Grade::Hard => 105.0,
                    Grade::Hairpin => 180.0,
                };
                (turn, if long { 1.4 } else { 0.5 }, 0.55)
            }
            Shape::UTurn => (180.0, 0.55, 0.9),
        };
        let shaft = if turn == 0.0 { 2.2 } else { 1.0 };
        let mut pts = vec![Vec2::ZERO, Vec2::new(0.0, shaft)];
        let mut dir = Vec2::Y;
        if turn > 0.0 {
            // A clockwise arc about the centre to the right of the shaft's top.
            let centre = Vec2::new(radius, shaft);
            let steps = 24;
            for s in 1..=steps {
                let a = (turn * s as f32 / steps as f32).to_radians();
                pts.push(centre + Vec2::new(-a.cos(), a.sin()) * radius);
            }
            let a = turn.to_radians();
            dir = Vec2::new(a.sin(), a.cos());
            let end = *pts.last().unwrap_or(&Vec2::ZERO);
            pts.push(end + dir * exit);
        }
        if self.side == Side::Left {
            for p in &mut pts {
                p.x = -p.x;
            }
            dir.x = -dir.x;
        }
        (pts, dir)
    }

    /// The glyph as `size`² RGBA pixels (row 0 at the top): the arrow in its colour, a black
    /// outline round it, transparent elsewhere.
    pub(crate) fn rasterize(&self, size: usize) -> Vec<u8> {
        const HALF_WIDTH: f32 = 0.17;
        const HEAD_HALF: f32 = 0.45;
        const HEAD_LENGTH: f32 = 0.55;
        const OUTLINE: f32 = 0.09;
        let (pts, dir) = self.path();
        let end = *pts.last().unwrap_or(&Vec2::ZERO);
        let side = dir.perp();
        let head = [
            end + dir * HEAD_LENGTH,
            end + side * HEAD_HALF,
            end - side * HEAD_HALF,
        ];
        // Fit everything, strokes included, square in the image with a margin.
        let pad = HALF_WIDTH + OUTLINE;
        let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
        for p in pts.iter().chain(head.iter()) {
            lo = lo.min(*p - Vec2::splat(pad));
            hi = hi.max(*p + Vec2::splat(pad));
        }
        let span = (hi - lo).max_element() * 1.06;
        let mid = (lo + hi) / 2.0;
        let px = span / size as f32;
        let seg = |p: Vec2, a: Vec2, b: Vec2| {
            let ab = b - a;
            let t = ((p - a).dot(ab) / ab.length_squared().max(1e-9)).clamp(0.0, 1.0);
            (p - a - ab * t).length()
        };
        let inside = |p: Vec2| {
            let s = |a: Vec2, b: Vec2| (b - a).perp_dot(p - a);
            let (d0, d1, d2) = (
                s(head[0], head[1]),
                s(head[1], head[2]),
                s(head[2], head[0]),
            );
            (d0 >= 0.0 && d1 >= 0.0 && d2 >= 0.0) || (d0 <= 0.0 && d1 <= 0.0 && d2 <= 0.0)
        };
        let mut out = vec![0u8; size * size * 4];
        for row in 0..size {
            for col in 0..size {
                let p = mid
                    + Vec2::new(
                        (col as f32 + 0.5) / size as f32 - 0.5,
                        0.5 - (row as f32 + 0.5) / size as f32,
                    ) * span;
                let shaft = pts
                    .windows(2)
                    .map(|w| seg(p, w[0], w[1]))
                    .fold(f32::MAX, f32::min)
                    - HALF_WIDTH;
                let edge = seg(p, head[0], head[1])
                    .min(seg(p, head[1], head[2]))
                    .min(seg(p, head[2], head[0]));
                let tip = if inside(p) { -edge } else { edge };
                // Signed distance to the arrow, in pixels.
                let d = shaft.min(tip) / px;
                let outline = OUTLINE / px;
                let alpha = (outline + 0.5 - d).clamp(0.0, 1.0);
                if alpha <= 0.0 {
                    continue;
                }
                let fill = (0.5 - d).clamp(0.0, 1.0);
                let i = (row * size + col) * 4;
                for c in 0..3 {
                    out[i + c] = (f32::from(self.colour[c]) * fill).round() as u8;
                }
                out[i + 3] = (alpha * 255.0).round() as u8;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A square loop run anticlockwise seen from above with north up (x north, y west): north,
    /// then west, south, east. Each corner is a left turn.
    fn square() -> Vec<[f32; 2]> {
        vec![[0.0, 0.0], [200.0, 0.0], [200.0, 200.0], [0.0, 200.0]]
    }

    #[test]
    fn a_square_loop_turns_left_at_every_corner_through_the_start() {
        let notes = notes(&square(), true);
        assert_eq!(notes.len(), 4);
        for (i, n) in notes.iter().enumerate() {
            assert_eq!(n.bend.side, Some(Side::Left), "corner {i}");
            assert_eq!(n.bend.grade, Grade::Hard, "a right angle is hard: {i}");
            assert!((n.bend.degrees - 90.0).abs() < 1e-3);
        }
        // Run the other way round, every corner is a right.
        let mut back = square();
        back.reverse();
        assert!(notes_sides(&back, true)
            .iter()
            .all(|s| *s == Some(Side::Right)));
        assert_eq!(notes[0].bend.call(), "HARD LEFT");
    }

    fn notes_sides(points: &[[f32; 2]], circuit: bool) -> Vec<Option<Side>> {
        notes(points, circuit).iter().map(|n| n.bend.side).collect()
    }

    #[test]
    fn north_then_east_is_a_right_turn_in_wow_coordinates() {
        // North (+x), then east (−y): a right turn.
        let a = turn_angle(&[[0.0, 0.0], [100.0, 0.0], [100.0, -100.0]], false, 1).unwrap();
        assert!((a + 90.0).abs() < 1e-3, "{a}");
    }

    #[test]
    fn grades_follow_the_thresholds() {
        assert_eq!(Grade::of(10.0), Grade::Straight);
        assert_eq!(Grade::of(-20.0), Grade::Easy);
        assert_eq!(Grade::of(60.0), Grade::Medium);
        assert_eq!(Grade::of(-90.0), Grade::Hard);
        assert_eq!(Grade::of(-100.0), Grade::Hard);
        assert_eq!(Grade::of(160.0), Grade::Hairpin);
    }

    /// A sprint north whose legs bend by the given changes (degrees, positive left), 150 yd each.
    fn sprint(turns: &[f32]) -> Vec<[f32; 2]> {
        let mut at = Vec2::ZERO;
        let mut heading = 0.0f32;
        let mut pts = vec![[0.0, 0.0]];
        for t in std::iter::once(&0.0).chain(turns) {
            heading += t.to_radians();
            at += Vec2::from_angle(heading) * 150.0;
            pts.push([at.x, at.y]);
        }
        pts
    }

    #[test]
    fn a_sprint_grades_each_turn_and_its_ends_are_straight() {
        let pts = sprint(&[-60.0, 110.0, -170.0, 5.0]);
        let notes = notes(&pts, false);
        let calls: Vec<String> = notes.iter().map(|n| n.bend.call()).collect();
        assert_eq!(
            calls,
            [
                "STRAIGHT",
                "MEDIUM RIGHT",
                "HARD LEFT",
                "HAIRPIN RIGHT",
                "STRAIGHT",
                "STRAIGHT"
            ]
        );
        assert_eq!(turn_angle(&pts, false, 0), None, "no turn at the start");
        assert_eq!(
            turn_angle(&pts, false, pts.len() - 1),
            None,
            "none past the finish"
        );
    }

    #[test]
    fn small_changes_the_same_way_make_one_long_bend() {
        let pts = sprint(&[-25.0, -30.0, -20.0, 40.0]);
        let notes = notes(&pts, false);
        let b = notes[1].bend;
        assert!(
            b.long && b.gates == 3 && b.side == Some(Side::Right),
            "{b:?}"
        );
        assert_eq!(b.grade, Grade::Medium, "75° in all");
        assert_eq!(b.call(), "LONG RIGHT");
        // Its later gates call what is left of it; the left after it stands alone.
        assert_eq!(notes[2].bend.gates, 2);
        assert_eq!(notes[3].bend.call(), "EASY RIGHT");
        assert_eq!(notes[4].bend.call(), "EASY LEFT");
        // A long bend that goes on past a right angle says how hard.
        let pts = sprint(&[-40.0, -40.0, -40.0]);
        assert_eq!(notes_calls(&pts)[1], "LONG HARD RIGHT");
    }

    fn notes_calls(pts: &[[f32; 2]]) -> Vec<String> {
        notes(pts, false).iter().map(|n| n.bend.call()).collect()
    }

    #[test]
    fn a_turn_close_behind_is_called_with_the_bend() {
        // A hard right, 40 yd on a hairpin left: "into"; 100 yd on: "then"; 150 yd: nothing.
        for (gap, link) in [
            (40.0, Some(Link::Into)),
            (100.0, Some(Link::Then)),
            (150.0, None),
        ] {
            let pts = vec![
                [0.0, 0.0],
                [150.0, 0.0],
                [150.0, -gap],
                [300.0, -gap - 20.0],
            ];
            let n = notes(&pts, false)[1];
            assert_eq!(n.bend.call(), "HARD RIGHT");
            assert_eq!(n.next.map(|x| x.0), link, "gap {gap}");
        }
        let pts = vec![
            [0.0, 0.0],
            [150.0, 0.0],
            [150.0, -40.0],
            [0.0, -40.0],
            [-150.0, -40.0],
        ];
        let n = notes(&pts, false)[1];
        assert_eq!(n.follow().as_deref(), Some("into HARD RIGHT"));
    }

    #[test]
    fn a_circuit_wraps_its_long_bends_and_follows_through_the_start() {
        // A regular dodecagon: 30° left at every gate, round and round. Every note is a long left
        // capped at four gates.
        let pts: Vec<[f32; 2]> = (0..12)
            .map(|i| {
                let a = (i as f32 * 30.0).to_radians();
                [a.sin() * 300.0, -a.cos() * 300.0 + 300.0]
            })
            .collect();
        for n in notes(&pts, true) {
            assert!(n.bend.long && n.bend.gates == MAX_LONG_GATES, "{n:?}");
            assert_eq!(n.bend.side, Some(Side::Left));
        }
        // The square's last corner leads on into its first.
        let sq = notes(&square(), true);
        assert_eq!(sq[3].next, None, "200 yd on is not close");
        let small = [[0.0, 0.0], [60.0, 0.0], [60.0, 60.0], [0.0, 60.0]];
        assert_eq!(notes(&small, true)[3].next.map(|x| x.0), Some(Link::Then));
    }

    fn place<'a>(pts: &'a [[f32; 2]], notes: &'a [Note], at: [f32; 2], facing: f32) -> Place<'a> {
        Place {
            points: pts,
            notes,
            last: 0,
            next: 1,
            finish: false,
            at,
            facing,
            speed: 0.0,
        }
    }

    #[test]
    fn the_sign_calls_the_next_turn_once_near_it() {
        let pts = square();
        let notes = notes(&pts, true);
        // Heading north towards the first corner (facing 0 is north).
        let far = place(&pts, &notes, [20.0, 0.0], 0.0);
        assert_eq!(choose(&far), Sign::Straight(Some(notes[1].bend)));
        assert_eq!(choose(&far).lines().1.as_deref(), Some("then HARD LEFT"));
        let near = place(&pts, &notes, [150.0, 0.0], 0.0);
        assert_eq!(choose(&near), Sign::Note(notes[1]));
        // Fast, it is called sooner: 40 yd/s × 4 s.
        let fast = Place {
            speed: 40.0,
            ..place(&pts, &notes, [50.0, 0.0], 0.0)
        };
        assert_eq!(choose(&fast), Sign::Note(notes[1]));
        let finish = Place {
            finish: true,
            ..place(&pts, &notes, [150.0, 0.0], 0.0)
        };
        assert_eq!(choose(&finish), Sign::Finish);
    }

    #[test]
    fn facing_back_down_the_course_is_the_wrong_way() {
        let pts = square();
        let notes = notes(&pts, true);
        let back = std::f32::consts::PI;
        assert_eq!(
            choose(&place(&pts, &notes, [100.0, 0.0], back)),
            Sign::WrongWay
        );
        // Sideways is not.
        assert_ne!(
            choose(&place(
                &pts,
                &notes,
                [100.0, 0.0],
                std::f32::consts::FRAC_PI_2
            )),
            Sign::WrongWay
        );
        // Just through a gate, still swinging round, is not either.
        assert_ne!(
            choose(&place(&pts, &notes, [10.0, 0.0], back)),
            Sign::WrongWay
        );
    }

    #[test]
    fn the_placement_sample_is_a_full_note() {
        let (call, follow) = Sign::sample().lines();
        assert_eq!(call, "HARD RIGHT");
        assert_eq!(follow.as_deref(), Some("then HAIRPIN LEFT"));
    }

    #[test]
    fn far_off_the_leg_points_back_to_the_checkpoint() {
        let pts = square();
        let notes = notes(&pts, true);
        // 100 yd west of the first leg, facing north: the checkpoint is ahead and to the right.
        let s = choose(&place(&pts, &notes, [100.0, 100.0], 0.0));
        assert_eq!(s, Sign::OffCourse(Some(Side::Right)));
        assert_eq!(s.lines().1.as_deref(), Some("checkpoint to the right"));
    }

    #[test]
    fn glyphs_bend_the_way_they_turn_and_are_outlined() {
        let size = 64;
        let right = Glyph::of(&Bend::new(-100.0, 1)).rasterize(size);
        let left = Glyph::of(&Bend::new(100.0, 1)).rasterize(size);
        // The ink's middle lies the way it turns from the tail (the lowest inked row's middle).
        let lean = |img: &[u8]| {
            let inked = |i: usize| img[i * 4 + 3] > 128;
            let mean = |is: Vec<usize>| {
                is.iter().map(|i| (i % size) as f32).sum::<f32>() / is.len().max(1) as f32
            };
            let tail_row = (0..size * size).filter(|&i| inked(i)).max().unwrap() / size;
            let tail = mean(
                (tail_row * size..(tail_row + 1) * size)
                    .filter(|&i| inked(i))
                    .collect(),
            );
            mean((0..size * size).filter(|&i| inked(i)).collect()) - tail
        };
        assert!(lean(&right) > 4.0, "{}", lean(&right));
        assert!(lean(&left) < -4.0, "{}", lean(&left));
        let ink = |img: &[u8], right_half: bool| {
            (0..size * size)
                .filter(|i| (i % size >= size / 2) == right_half && img[i * 4 + 3] > 128)
                .count()
        };
        // Some of it is the colour, some the black outline, and the corners are clear.
        let px =
            |img: &[u8], i: usize| [img[i * 4], img[i * 4 + 1], img[i * 4 + 2], img[i * 4 + 3]];
        assert!((0..size * size).any(|i| px(&right, i) == [255, 140, 30, 255]));
        assert!((0..size * size).any(|i| px(&right, i) == [0, 0, 0, 255]));
        assert_eq!(px(&right, 0)[3], 0);
        // A straight one is symmetric.
        let s = Glyph::STRAIGHT.rasterize(size);
        assert!(ink(&s, true).abs_diff(ink(&s, false)) <= size);
    }

    #[test]
    fn the_northshire_sprint_bends_the_way_its_road_does() {
        // Into checkpoint 3 the road runs south by a little west, out of it south-east: facing
        // south, east is on the left, by 60°. The three easy rights after it make a long, hard
        // right 60 yd on.
        let c = crate::kart::COURSES
            .iter()
            .find(|c| c.key == "sprint")
            .unwrap();
        let notes = notes(&c.route(), false);
        assert_eq!(notes[3].bend.call(), "MEDIUM LEFT");
        assert_eq!(notes[3].follow().as_deref(), Some("then LONG HARD RIGHT"));
        assert_eq!(notes[4].bend.gates, 3);
        assert_eq!(notes[7].bend.call(), "STRAIGHT", "the finish");
    }
}
