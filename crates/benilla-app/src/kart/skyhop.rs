//! Skyhop and the challenges of `props/props.json`: courses of floating platforms ([`super::props`])
//! run against the clock, in the kart and out of any race. The run starts at its first gate, on
//! the ground before the launch ramp; each later gate is a platform, passed in order within its
//! radius and at its height (the platforms float over open ground, which never passes for them);
//! a fall below the course's floor ends the run, and the start is driven back to. The finish is
//! told to the server (`R\tHop\t<key>\t<ms>`), which keeps each racer's best and the record, and
//! says them (`R\tHopBest`, the layer's line). The clock and the platform count are the race
//! banner's while a run is on.

use benilla_assets::coords::bevy_to_wow;
use benilla_world::world_map::CurrentMap;
use bevy::prelude::*;
use kartcraft::KartView;

use crate::net::{ClientCommand, NetCommands};

/// One gate: a centre (WoW), its radius (yards) and, past the first, its height to be passed at.
#[derive(serde::Deserialize, Clone, Copy, Debug)]
pub(crate) struct Gate {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) z: f32,
    pub(crate) r: f32,
}

/// A challenge: its key (the server's) and name, its map, its gates (the start first, the goal
/// last) and the height (WoW z) below which a run has fallen.
#[derive(serde::Deserialize, Clone, Debug)]
pub(crate) struct Challenge {
    pub(crate) key: String,
    pub(crate) name: String,
    pub(crate) map: u32,
    pub(crate) gates: Vec<Gate>,
    pub(crate) floor: f32,
}

/// How far (yards) above or below a platform's top a kart passes its gate, and the start's.
const GATE_REACH: f32 = 3.0;
const START_REACH: f32 = 4.0;
/// The start is offered from this near (yards).
const OFFER: f32 = 40.0;
/// A run is given up after this long (s).
const GIVE_UP: f32 = 240.0;
/// How long (s) a fall's or a finish's word stays up.
const SAY_FOR: f32 = 4.0;

/// Where our run is.
#[derive(Resource, Default)]
pub(crate) struct Run {
    state: State,
    /// What the banner says after a fall or a finish, and until when.
    said: Option<(String, f32)>,
}

impl Run {
    /// A challenge under way.
    pub(crate) fn running(&self) -> bool {
        matches!(self.state, State::Running { .. })
    }
}

#[derive(Default, Clone, Copy, PartialEq, Debug)]
enum State {
    #[default]
    Idle,
    /// The challenge (its index), when it started, and the next gate.
    Running { at: usize, since: f32, next: usize },
}

/// Whether `p` (WoW) passes gate `g`, the start's reach on the ground or a platform's at its top.
fn passes(g: &Gate, p: [f32; 3], start: bool) -> bool {
    let d = Vec2::new(p[0] - g.x, p[1] - g.y).length();
    let reach = if start { START_REACH } else { GATE_REACH };
    d <= g.r && (p[2] - g.z).abs() <= reach
}

/// One step of a run: the kart at `p` (WoW) at `now` on challenge `c`, the state before. A
/// finish's time (ms), if this step finished it.
fn step(state: &mut State, at: usize, c: &Challenge, p: [f32; 3], now: f32) -> Step {
    match *state {
        State::Idle => {
            if passes(&c.gates[0], p, true) {
                *state = State::Running {
                    at,
                    since: now,
                    next: 1,
                };
                return Step::Started;
            }
            Step::None
        }
        State::Running { at: a, since, next } if a == at => {
            if now - since > GIVE_UP {
                *state = State::Idle;
                return Step::None;
            }
            if passes(&c.gates[next], p, false) {
                if next + 1 == c.gates.len() {
                    *state = State::Idle;
                    return Step::Finished(((now - since) * 1000.0).round() as u32);
                }
                *state = State::Running {
                    at,
                    since,
                    next: next + 1,
                };
                return Step::Gate;
            }
            // Past the launch (onto the platforms), a drop under the course is a fall.
            if next >= 2 && p[2] < c.floor {
                *state = State::Idle;
                return Step::Fell(next - 1);
            }
            Step::None
        }
        State::Running { .. } => Step::None,
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Step {
    None,
    Started,
    Gate,
    Fell(usize),
    Finished(u32),
}

/// Runs our challenge, out of any race: the gates, the banner, the finish told to the server.
pub(crate) fn update(
    time: Res<Time>,
    view: Res<KartView>,
    map: Option<Res<CurrentMap>>,
    races: Res<super::race::Races>,
    mut run: ResMut<Run>,
    mut text: ResMut<super::race::RaceText>,
    net: Option<Res<NetCommands>>,
) {
    let now = time.elapsed_secs();
    if !view.active || races.race.is_some() || !super::props::enabled() {
        run.state = State::Idle;
        return;
    }
    let Some(map) = map.map(|m| m.0) else {
        return;
    };
    let p = bevy_to_wow(view.transform.translation);
    let challenges = &super::props::FILE.challenges;
    for (i, c) in challenges.iter().enumerate() {
        if c.map != map || c.gates.len() < 2 {
            continue;
        }
        let mut state = run.state;
        match step(&mut state, i, c, p, now) {
            Step::Finished(ms) => {
                run.said = Some((format!("{}: {}!", c.name, clock(ms)), now + SAY_FOR));
                if let Some(net) = &net {
                    let _ = net.0.send(ClientCommand::AddonMessage {
                        distribution: benilla_ui::script::AddonDistribution::Party,
                        text: format!("KARTCRAFT\tR\tHop\t{}\t{ms}", c.key),
                    });
                }
                info!("kart skyhop: {} in {ms} ms", c.key);
            }
            Step::Fell(platform) => {
                run.said = Some((
                    format!(
                        "{}: fell after platform {platform} of {}, back to the start",
                        c.name,
                        c.gates.len() - 1
                    ),
                    now + SAY_FOR,
                ));
            }
            Step::Started => run.said = None,
            _ => {}
        }
        run.state = state;
        // The banner: the run's clock, or the word after it, or the start's offer.
        if let State::Running { at, since, next } = run.state {
            if at == i {
                text.banner = format!(
                    "{}  {}   platform {} of {}",
                    c.name,
                    clock(((now - since) * 1000.0) as u32),
                    next - 1,
                    c.gates.len() - 1
                );
            }
        } else if let Some((said, until)) = run.said.as_ref().filter(|(_, u)| now < *u) {
            let _ = until;
            text.banner = said.clone();
        } else if Vec2::new(p[0] - c.gates[0].x, p[1] - c.gates[0].y).length() < OFFER
            && text.banner.is_empty()
        {
            text.banner = format!(
                "{} starts here: up the ramp, and hop the gaps at full speed",
                c.name
            );
        }
    }
}

/// `ms` as a race clock, "0:12.34", as every race time is shown.
fn clock(ms: u32) -> String {
    format!(
        "{}:{:02}.{:02}",
        ms / 60_000,
        ms / 1000 % 60,
        ms % 1000 / 10
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn course() -> Challenge {
        Challenge {
            key: "t".into(),
            name: "Test".into(),
            map: 1,
            gates: vec![
                Gate {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                    r: 8.0,
                },
                Gate {
                    x: 30.0,
                    y: 0.0,
                    z: 7.0,
                    r: 10.0,
                },
                Gate {
                    x: 60.0,
                    y: 0.0,
                    z: 7.5,
                    r: 8.0,
                },
            ],
            floor: 4.0,
        }
    }

    #[test]
    fn a_run_starts_passes_its_platforms_in_order_and_finishes() {
        let c = course();
        let mut s = State::Idle;
        assert_eq!(step(&mut s, 0, &c, [0.0, 0.0, 0.0], 1.0), Step::Started);
        // Under the platform, on the ground, is no pass.
        assert_eq!(step(&mut s, 0, &c, [30.0, 0.0, 0.0], 2.0), Step::None);
        assert_eq!(step(&mut s, 0, &c, [30.0, 0.0, 7.2], 3.0), Step::Gate);
        assert_eq!(
            step(&mut s, 0, &c, [60.0, 0.0, 7.6], 13.5),
            Step::Finished(12_500)
        );
        assert_eq!(s, State::Idle);
    }

    #[test]
    fn a_fall_off_the_platforms_ends_the_run() {
        let c = course();
        let mut s = State::Idle;
        step(&mut s, 0, &c, [0.0, 0.0, 0.0], 0.0);
        // On the ramp, under the floor, is not a fall yet.
        assert_eq!(step(&mut s, 0, &c, [10.0, 0.0, 2.0], 1.0), Step::None);
        step(&mut s, 0, &c, [30.0, 0.0, 7.0], 2.0);
        assert_eq!(step(&mut s, 0, &c, [45.0, 0.0, 1.0], 3.0), Step::Fell(1));
        assert_eq!(s, State::Idle);
        assert_eq!(clock(12_340), "0:12.34");
        assert_eq!(clock(62_340), "1:02.34");
    }

    /// Skyhop as shipped is a course a kart finishes: a bot on CTR's physics drives the props'
    /// real faces over the flats, full gas at each next platform, hopping at each edge, and the
    /// run's judge sees it through every gate to the goal. One that never jumps falls.
    #[test]
    fn skyhop_is_driven_to_its_goal_hopping_and_not_without() {
        let (time, fell) = bot(true);
        assert!(
            fell.is_none(),
            "the hopping bot fell after platform {fell:?}"
        );
        let ms = time.expect("the hopping bot finishes");
        println!("Skyhop by the bot: {}", clock(ms));
        assert!(ms > 8_000, "a course, not a dash: {ms} ms");
        let (time, fell) = bot(false);
        assert!(
            time.is_none() && fell.is_some(),
            "without a hop the gaps are not crossed"
        );
    }

    /// The server takes a finish at the goal it knows (`Kartcraft.cpp` `HOP_COURSES`): the file's.
    #[test]
    fn the_server_knows_skyhop_s_goal() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let cpp = std::fs::read_to_string(root.join("setup/vmangos/src/Kartcraft.cpp")).unwrap();
        let line = cpp
            .lines()
            .find(|l| l.trim_start().starts_with("{ \"skyhop\", "))
            .expect("HOP_COURSES' skyhop");
        let goal: Vec<f32> = line[line.find("{ -").unwrap() + 2..line.find(" },").unwrap()]
            .split(", ")
            .map(|v| v.trim_end_matches('f').parse().unwrap())
            .collect();
        let c = super::super::props::FILE
            .challenges
            .iter()
            .find(|c| c.key == "skyhop")
            .unwrap();
        let last = c.gates.last().unwrap();
        assert_eq!(goal, [last.x, last.y, last.z]);
    }

    /// A bot's Skyhop: its time, or the platform it fell after.
    fn bot(hops: bool) -> (Option<u32>, Option<usize>) {
        use benilla_assets::coords::wow_to_bevy;
        use kartcraft::{from_lu, to_lu};
        let file = &*super::super::props::FILE;
        let c = file.challenges.iter().find(|c| c.key == "skyhop").unwrap();
        let start = c.gates[0];
        let origin = wow_to_bevy([start.x, start.y, start.z]);
        let mut tris: Vec<[[f32; 3]; 3]> = file
            .props
            .iter()
            .filter(|p| p.map == c.map)
            .flat_map(super::super::props::triangles)
            .map(|t| t.map(|v| to_lu(v, origin).to_array()))
            .collect();
        // The flats under the course, in 50-yard tiles as the world's faces come.
        let g = |dx: f32, dz: f32| to_lu(origin + Vec3::new(dx, 0.0, dz), origin).to_array();
        for i in -14..14 {
            for j in -14..14 {
                let (x0, z0) = (i as f32 * 50.0, j as f32 * 50.0);
                let (x1, z1) = (x0 + 50.0, z0 + 50.0);
                tris.push([g(x0, z0), g(x0, z1), g(x1, z1)]);
                tris.push([g(x0, z0), g(x1, z1), g(x1, z0)]);
            }
        }
        let world = ctr_kart::Collision::new(tris, 160.0);
        let lu = |w: [f32; 3]| to_lu(wow_to_bevy(w), origin);
        let first = lu([c.gates[1].x, c.gates[1].y, c.gates[1].z]);
        let back = -first.normalize() * 30.0;
        let heading = (back.x.atan2(back.z).to_degrees() * 4096.0 / 360.0) as i32;
        let mut kart = ctr_kart::Kart::new(
            ctr_kart::Class::Balanced,
            Vec3::new(back.x, 20.0, back.z),
            (heading + 2048) & 0xFFF,
        );
        let mods = ctr_kart::Modifiers::default();
        let mut state = State::Idle;
        let platforms: Vec<&super::super::props::Prop> = file
            .props
            .iter()
            .filter(|p| p.kind == super::super::props::Kind::Box && p.z > start.z + 1.0)
            .collect();
        let mut hopped_from = None;
        let pads: Vec<kartcraft::pads::Pad> = file
            .pads
            .iter()
            .filter(|p| p.map == c.map)
            .map(|p| kartcraft::pads::Pad::from_wow(p.x, p.y, p.z, p.o))
            .collect();
        for tick in 0..60 * 30 {
            let pose = kart.pose();
            let at = from_lu(pose.position, origin);
            let w = bevy_to_wow(at);
            let next = match state {
                State::Running { next, .. } => next,
                State::Idle => 1,
            };
            let goal = c.gates[next.min(c.gates.len() - 1)];
            let to = lu([goal.x, goal.y, goal.z]) - pose.position;
            let h = pose.heading as f32 * std::f32::consts::TAU / 4096.0;
            let right = Vec3::new(-h.cos(), 0.0, h.sin());
            let steer = (to.normalize_or_zero().dot(right) * 3.0).clamp(-1.0, 1.0);
            // Hop before a platform's far edge, 2.5 to 5 yards short of it: right at the edge the hop's
            // lift is spent on the edge's slope (CTR's PROCESS_JUMP reads the ground under the nose).
            let mut hop = false;
            if hops && kart.grounded() {
                for (i, p) in platforms.iter().enumerate() {
                    let f = Vec2::new(p.o.cos(), p.o.sin());
                    let l = Vec2::new(-p.o.sin(), p.o.cos());
                    let d = Vec2::new(w[0] - p.x, w[1] - p.y);
                    let (along, across) = (d.dot(f), d.dot(l));
                    let on = across.abs() <= p.width / 2.0 && along.abs() <= p.length / 2.0;
                    if on
                        && (w[2] - (p.z + p.height)).abs() < 1.0
                        && (2.5..5.0).contains(&(p.length / 2.0 - along))
                        && hopped_from != Some(i)
                    {
                        hop = true;
                        hopped_from = Some(i);
                    }
                }
            }
            let input = ctr_kart::Input {
                steer,
                gas: true,
                brake: false,
                down: false,
                r1: hop,
                l1: false,
            };
            // The props' turbo pads, as the session fires them.
            if pads.iter().any(|p| p.contains(at)) {
                kart.turbo_pad();
            }
            kart.tick(&world, &input, &mods);
            let w = bevy_to_wow(from_lu(kart.pose().position, origin));
            match step(&mut state, 0, c, w, tick as f32 / 30.0) {
                Step::Finished(ms) => return (Some(ms), None),
                Step::Fell(n) => return (None, Some(n)),
                _ => {}
            }
        }
        (None, None)
    }
}
