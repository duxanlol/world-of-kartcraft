//! The kart garage: which engine class we drive and whose kart we drive it in. The class is earned
//! with the Karting skill, at the profession tiers a 1.12 trade opens its ranks at: Turning from the
//! start (the slowest kart, and the easiest to steer), then Balanced at 75, Acceleration at 150 and
//! Speed at 225. Any racer's kart is free. Both are CVars the layer's garage window writes
//! (`KartClass`, `KartRacer`; [`super::settings`]); the class we drive is the one picked only if
//! our skill has it, else the best one below it ([`effective`]).

use kartcraft::Class;

/// The classes in the order they open, with the Karting skill each needs.
pub(crate) const TIERS: [(Class, u16); 4] = [
    (Class::Turn, 0),
    (Class::Balanced, 75),
    (Class::Accel, 150),
    (Class::Speed, 225),
];

/// Whether `skill` (none before the line is ours) has `class`.
pub(crate) fn unlocked(class: Class, skill: Option<u16>) -> bool {
    TIERS
        .iter()
        .find(|(c, _)| *c == class)
        .is_some_and(|&(_, needs)| skill.unwrap_or(0) >= needs)
}

/// The class we drive: `chosen` if `skill` has it, else the best one below it, never above.
pub(crate) fn effective(chosen: Class, skill: Option<u16>) -> Class {
    let at = TIERS.iter().position(|(c, _)| *c == chosen).unwrap_or(0);
    TIERS[..=at]
        .iter()
        .rev()
        .map(|&(c, _)| c)
        .find(|&c| unlocked(c, skill))
        .unwrap_or(TIERS[0].0)
}

/// A class's place among the four on each of top speed, acceleration and turning, 1 (least) to 4.
fn ranks(class: Class) -> [usize; 3] {
    let stat = |c: Class| {
        let s = c.stats();
        [s.speed, s.accel, s.turn_rate]
    };
    let own = stat(class);
    std::array::from_fn(|i| Class::ALL.iter().filter(|&&c| stat(c)[i] <= own[i]).count())
}

/// The garage's classes, one `key<TAB>name<TAB>skill needed<TAB>description` a line, in tier order.
pub(crate) fn class_list() -> String {
    TIERS
        .iter()
        .map(|&(class, needs)| {
            let [speed, accel, turn] = ranks(class);
            format!(
                "{}\t{}\t{needs}\tSpeed {speed}/4, acceleration {accel}/4, turning {turn}/4",
                kartcraft::class_key(class),
                class.name()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A racer's name by the key of their kart (`ctr-extract`'s names).
fn racer_name(key: &str) -> String {
    match key {
        "crash" => "Crash Bandicoot",
        "cortex" => "Dr. Neo Cortex",
        "tiny" => "Tiny Tiger",
        "coco" => "Coco Bandicoot",
        "polar" => "Polar",
        "pura" => "Pura",
        "pinstripe" => "Pinstripe",
        "roo" => "Ripper Roo",
        "joe" => "Komodo Joe",
        "pen" => "Penta Penguin",
        "fake" => "Fake Crash",
        other => return other.to_string(),
    }
    .to_string()
}

/// The garage's karts, one `key<TAB>name` a line: the racers the kart data has a kart for.
pub(crate) fn racer_list(racers: &[&str]) -> String {
    racers
        .iter()
        .map(|k| format!("{k}\t{}", racer_name(k)))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_skill_opens_turning_then_balanced_acceleration_and_speed() {
        let open = |skill: Option<u16>| -> Vec<Class> {
            Class::ALL
                .into_iter()
                .filter(|&c| unlocked(c, skill))
                .collect()
        };
        assert_eq!(open(None), [Class::Turn]);
        assert_eq!(open(Some(1)), [Class::Turn]);
        assert_eq!(open(Some(74)), [Class::Turn]);
        assert_eq!(open(Some(75)), [Class::Balanced, Class::Turn]);
        assert_eq!(
            open(Some(150)),
            [Class::Balanced, Class::Accel, Class::Turn]
        );
        assert_eq!(open(Some(225)), Class::ALL);
        assert_eq!(open(Some(300)), Class::ALL);
    }

    #[test]
    fn a_locked_class_falls_back_to_the_best_one_below_it() {
        assert_eq!(effective(Class::Speed, Some(300)), Class::Speed);
        assert_eq!(effective(Class::Speed, Some(160)), Class::Accel);
        assert_eq!(effective(Class::Speed, Some(80)), Class::Balanced);
        assert_eq!(effective(Class::Accel, Some(10)), Class::Turn);
        assert_eq!(effective(Class::Accel, None), Class::Turn);
        // Never above what was picked.
        assert_eq!(effective(Class::Turn, Some(300)), Class::Turn);
        assert_eq!(effective(Class::Balanced, Some(300)), Class::Balanced);
    }

    #[test]
    fn the_starting_class_is_the_slowest_and_the_last_the_fastest() {
        let speed = |c: Class| c.stats().speed;
        let first = TIERS[0].0;
        let last = TIERS[3].0;
        assert!(Class::ALL.iter().all(|&c| speed(first) <= speed(c)));
        assert!(Class::ALL.iter().all(|&c| speed(last) >= speed(c)));
        assert!(Class::ALL
            .iter()
            .all(|&c| first.stats().turn_rate >= c.stats().turn_rate));
        assert!(Class::ALL
            .iter()
            .all(|&c| TIERS[2].0.stats().accel >= c.stats().accel));
    }

    #[test]
    fn the_lists_name_every_class_and_racer() {
        let classes = class_list();
        let lines: Vec<&str> = classes.lines().collect();
        assert_eq!(
            lines[0],
            "turn\tTurning\t0\tSpeed 1/4, acceleration 3/4, turning 4/4"
        );
        assert!(lines[3].starts_with("speed\tSpeed\t225\tSpeed 4/4"));
        assert_eq!(
            racer_list(&["crash", "coco"]),
            "crash\tCrash Bandicoot\ncoco\tCoco Bandicoot"
        );
    }
}
