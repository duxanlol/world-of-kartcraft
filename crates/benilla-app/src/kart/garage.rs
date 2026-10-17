//! The kart garage: which engine class we drive and whose kart we drive it in. Every class is open
//! from the start, Turning (the slowest kart, and the easiest to steer) picked first; the tiers
//! keep a Karting skill each needs, all none now, so a class could be earned again. Any racer's
//! kart is free. Both are CVars the layer's garage window writes
//! (`KartClass`, `KartRacer`; [`super::settings`]); the class we drive is the one picked only if
//! our skill has it, else the best one below it ([`effective`]).

use kartcraft::Class;

/// The classes in the garage's order, with the Karting skill each needs: none, every class is
/// free to pick.
pub(crate) const TIERS: [(Class, u16); 4] = [
    (Class::Turn, 0),
    (Class::Balanced, 0),
    (Class::Accel, 0),
    (Class::Speed, 0),
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

/// A racer's name by the key of their kart (`ctr-extract`'s names), or a built-in kart's own
/// (`kartcraft::goods`).
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
        other => {
            return kartcraft::goods::good(other)
                .map_or_else(|| other.to_string(), |g| g.name.into())
        }
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
    fn every_class_is_open_from_the_start() {
        let open = |skill: Option<u16>| -> Vec<Class> {
            Class::ALL
                .into_iter()
                .filter(|&c| unlocked(c, skill))
                .collect()
        };
        assert_eq!(open(None), Class::ALL);
        assert_eq!(open(Some(1)), Class::ALL);
        assert_eq!(open(Some(300)), Class::ALL);
    }

    #[test]
    fn the_class_picked_is_the_class_driven() {
        assert_eq!(effective(Class::Speed, Some(300)), Class::Speed);
        assert_eq!(effective(Class::Speed, Some(1)), Class::Speed);
        assert_eq!(effective(Class::Accel, None), Class::Accel);
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
        assert!(lines[3].starts_with("speed\tSpeed\t0\tSpeed 4/4"));
        assert_eq!(
            racer_list(&["crash", "coco"]),
            "crash\tCrash Bandicoot\ncoco\tCoco Bandicoot"
        );
    }
}
