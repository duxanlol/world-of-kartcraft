//! The Race Marshal test hook, `WOW_KART_MARSHAL=<role>:<course>[:leave=<s>]`: races through the
//! marshal's gossip as a player does. It hops beside the course's marshal (`courses.json`'s
//! `marshal`) with `.go xyz` (a GM account), finds the nearest unit named "Race Marshal", opens its
//! menu (`CMSG_GOSSIP_HELLO`) and clicks an option by its text (`CMSG_GOSSIP_SELECT_OPTION`).
//!
//! - `host:<course>` clicks "Host a race: <name>", then "Start the race" once another racer is in.
//! - `join:<course>` clicks "Join ...'s race on <name>", asking again until a race is offered.
//! - `:leave=<s>` sends `/leaverace`'s line (`R\tX`) that long after GO.
//! - `:warp=<s>` hops to the next checkpoint when the autopilot has not reached it in that long
//!   (it steers straight at it, and courses bend round buildings).
//!
//! `WOW_KART_MARSHAL_NOGO=1` skips the hop. Every step logs `kart autotest:`.

use benilla_assets::coords::bevy_to_wow;
use bevy::prelude::*;

use super::race::{Races, COURSES};
use crate::names::NameCache;
use crate::net::{ChatKind, ClientCommand, Guid, NetCommands, NetEntity, ObjectStore, SelfPlayer};
use crate::player::Player;
use crate::ui_gossip::GossipState;

/// The marshal's name (`creature_template` 190000).
const MARSHAL: &str = "Race Marshal";
/// How far from the hop a marshal is looked for (yards).
const SEARCH: f32 = 40.0;
/// How near the marshal a hello is sent (yards; the server's reach is 5 plus her size).
const REACH: f32 = 4.5;
/// Seconds before asking the marshal again when the option is not (yet) on the menu.
const RETRY: f64 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Role {
    Host,
    Join,
}

/// What the hook does next.
#[derive(Debug, Default, PartialEq)]
enum Step {
    #[default]
    Boot,
    Hopped(f64),
    /// Asking the marshal for the option containing the text; `sent` is when the hello went.
    Asking {
        want: String,
        sent: Option<f64>,
        tries: u32,
    },
    /// Hosted or joined: waiting for the roster (host: for a second racer).
    Waiting,
    Racing,
    Done,
}

#[derive(Resource, Default)]
pub(crate) struct Autotest {
    step: Step,
    marshal: Option<u64>,
    /// The race's last logged state, so each change logs once.
    seen: Option<(u32, Vec<String>, bool, bool)>,
    go_at: Option<f64>,
    left: bool,
    hops: u32,
    /// Host: "Start the race" has been asked for.
    start_asked: bool,
    /// The checkpoint being driven for (WoW x, y) and since when.
    target: Option<([f32; 2], f64)>,
}

struct Plan {
    role: Role,
    course: &'static super::race::Course,
    leave_after: Option<f64>,
    warp_after: Option<f64>,
}

fn plan() -> Option<&'static Plan> {
    static PLAN: std::sync::OnceLock<Option<Plan>> = std::sync::OnceLock::new();
    PLAN.get_or_init(|| {
        let spec = std::env::var("WOW_KART_MARSHAL").ok()?;
        let mut parts = spec.split(':');
        let role = match parts.next()? {
            "host" => Role::Host,
            "join" => Role::Join,
            other => {
                error!("kart autotest: unknown role {other:?} in WOW_KART_MARSHAL={spec}");
                return None;
            }
        };
        let key = parts.next().unwrap_or("sprint");
        let Some(course) = COURSES.iter().find(|c| c.key == key) else {
            error!("kart autotest: no course {key:?}");
            return None;
        };
        let rest: Vec<&str> = parts.collect();
        let option = |key: &str| {
            rest.iter()
                .find_map(|p| p.strip_prefix(key)?.strip_prefix('='))
                .and_then(|s| s.parse().ok())
        };
        Some(Plan {
            role,
            course,
            leave_after: option("leave"),
            warp_after: option("warp"),
        })
    })
    .as_ref()
}

/// Whether the hook runs: the kart autopilot then holds still until GO.
pub(crate) fn active() -> bool {
    plan().is_some()
}

#[allow(clippy::too_many_arguments)]
fn drive(
    time: Res<Time>,
    mut test: ResMut<Autotest>,
    me: Query<(), With<SelfPlayer>>,
    player: Res<Player>,
    gossip: Res<GossipState>,
    races: Res<Races>,
    names: Option<Res<NameCache>>,
    units: Query<(&Guid, &NetEntity, &ObjectStore, &Transform), Without<SelfPlayer>>,
    net: Option<Res<NetCommands>>,
) {
    let (Some(plan), Some(net)) = (plan(), net) else {
        return;
    };
    if me.single().is_err() {
        return;
    }
    let now = time.elapsed_secs_f64();
    let send = |c: ClientCommand| {
        let _ = net.0.send(c);
    };
    let test = &mut *test;
    log_race(test, &races, now);

    match &mut test.step {
        Step::Boot => {
            if now < 8.0 {
                return;
            }
            if std::env::var_os("WOW_KART_MARSHAL_NOGO").is_some() {
                test.step = ask(plan);
                return;
            }
            if !hop(plan, &send) {
                test.step = Step::Done;
                return;
            }
            test.hops += 1;
            test.step = Step::Hopped(now);
        }
        Step::Hopped(at) => {
            if now - *at > 5.0 {
                test.step = ask(plan);
            }
        }
        Step::Asking { want, sent, tries } => {
            // Already in a race here (one left waiting from before is still ours): on to it.
            if plan.role == Role::Host && races.status().is_some() && want.starts_with("Host") {
                info!("kart autotest: already in a race — not hosting another");
                test.step = Step::Waiting;
                return;
            }
            let marshal = nearest_marshal(&player, names.as_deref(), &units, &net);
            let Some(marshal) = marshal else {
                if sent.is_none_or(|s| now - s > RETRY) {
                    warn!("kart autotest: no {MARSHAL} within {SEARCH} yd yet");
                    *sent = Some(now);
                }
                return;
            };
            let (marshal, away) = marshal;
            test.marshal = Some(marshal);
            // Gossip answers only within reach (the server ignores a hello from further).
            if away > REACH && sent.is_none_or(|s| now - s > RETRY) {
                if test.hops >= 5 || std::env::var_os("WOW_KART_MARSHAL_NOGO").is_some() {
                    error!(
                        "kart autotest: FAILURE — {away:.1} yd from the {MARSHAL}, out of reach"
                    );
                    test.step = Step::Done;
                    return;
                }
                info!("kart autotest: {away:.1} yd from the {MARSHAL} — hopping again");
                hop(plan, &send);
                test.hops += 1;
                *sent = Some(now);
                return;
            }
            let menu = (gossip.npc == Some(marshal) && !gossip.options.is_empty())
                .then_some(&gossip.options);
            if let (Some(options), Some(s)) = (menu, *sent) {
                if let Some(o) = options.iter().find(|o| o.message.contains(want.as_str())) {
                    let labels: Vec<&str> = options.iter().map(|o| o.message.as_str()).collect();
                    info!("kart autotest: menu {labels:?}");
                    info!(
                        "kart autotest: clicking {:?} (option {})",
                        o.message, o.index
                    );
                    send(ClientCommand::GossipSelectOption {
                        guid: marshal,
                        option: o.index,
                    });
                    test.step = Step::Waiting;
                    return;
                }
                if now - s < RETRY {
                    return;
                }
                let labels: Vec<&str> = options.iter().map(|o| o.message.as_str()).collect();
                info!("kart autotest: no {want:?} on the menu yet: {labels:?}");
            }
            if sent.is_none_or(|s| now - s > RETRY) {
                if *tries >= 30 {
                    error!("kart autotest: FAILURE — never offered {want:?}");
                    test.step = Step::Done;
                    return;
                }
                *tries += 1;
                info!("kart autotest: GossipHello to {marshal:#x} (try {tries}) for {want:?}");
                send(ClientCommand::GossipHello { guid: marshal });
                *sent = Some(now);
            }
        }
        Step::Waiting => {
            if let Some((_, roster, go, _)) = races.status() {
                if go {
                    test.step = Step::Racing;
                } else if plan.role == Role::Host && roster.len() >= 2 && !test.start_asked {
                    test.start_asked = true;
                    test.step = Step::Asking {
                        want: "Start the race".into(),
                        sent: None,
                        tries: 0,
                    };
                }
            }
        }
        Step::Racing => {
            let go = *test.go_at.get_or_insert(now);
            if let Some(after) = plan.leave_after.filter(|_| !test.left) {
                if now - go >= after {
                    info!(
                        "kart autotest: sending /leaverace's R\\tX {after} s after the countdown"
                    );
                    send(ClientCommand::AddonMessage {
                        distribution: benilla_ui::script::AddonDistribution::Party,
                        text: "KARTCRAFT\tR\tX".into(),
                    });
                    test.left = true;
                }
            }
            if let Some(after) = plan.warp_after {
                let next = races
                    .next_target(time.elapsed_secs())
                    .map(bevy_to_wow)
                    .map(|w| [w[0], w[1]]);
                match (next, test.target) {
                    (Some(p), Some((q, since))) if p == q => {
                        if now - since >= after {
                            info!(
                                "kart autotest: {after} s short of the checkpoint at {p:?} — \
                                 hopping to it"
                            );
                            send(ClientCommand::Chat {
                                kind: ChatKind::Say,
                                target: None,
                                // Our own height, a little up: the server drops us there.
                                text: format!(
                                    ".go xyz {:.1} {:.1} {:.1}",
                                    p[0],
                                    p[1],
                                    bevy_to_wow(player.pos)[2] + 3.0
                                ),
                                language: None,
                            });
                            test.target = Some((p, now));
                        }
                    }
                    (Some(p), _) => test.target = Some((p, now)),
                    (None, _) => test.target = None,
                }
            }
            if races.status().is_none() {
                info!("kart autotest: out of the race — done");
                test.step = Step::Done;
            }
        }
        Step::Done => {}
    }
}

fn ask(plan: &Plan) -> Step {
    let want = match plan.role {
        Role::Host => format!("Host a race: {}", plan.course.name),
        Role::Join => format!("race on {}", plan.course.name),
    };
    Step::Asking {
        want,
        sent: None,
        tries: 0,
    }
}

/// The nearest unit named "Race Marshal" in [`SEARCH`] yards; asks for names it lacks.
fn nearest_marshal(
    player: &Player,
    names: Option<&NameCache>,
    units: &Query<(&Guid, &NetEntity, &ObjectStore, &Transform), Without<SelfPlayer>>,
    net: &NetCommands,
) -> Option<(u64, f32)> {
    let names = names?;
    units
        .iter()
        .filter(|(_, e, ..)| e.kind == benilla_protocol::EntityKind::Unit)
        .filter(|(g, _, s, _)| names.resolve_unit(g.0, Some(s), net) == Some(MARSHAL))
        .map(|(g, _, _, t)| (g.0, t.translation.distance(player.pos)))
        .filter(|(_, d)| *d <= SEARCH)
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

/// `.go xyz` a step from the course's marshal towards its start; `false` with no marshal.
fn hop(plan: &Plan, send: &impl Fn(ClientCommand)) -> bool {
    let Some(m) = plan.course.marshal else {
        error!("kart autotest: {} has no marshal", plan.course.key);
        return false;
    };
    // Host and joiner either side of her facing, apart, so their karts do not bump.
    let a = m.o
        + match plan.role {
            Role::Host => 0.9,
            Role::Join => -0.9,
        };
    let (x, y) = (m.x + 2.5 * a.cos(), m.y + 2.5 * a.sin());
    info!(
        "kart autotest: {:?} on {} — hopping beside its marshal at ({:.1}, {:.1}, {:.1})",
        plan.role, plan.course.key, m.x, m.y, m.z
    );
    send(ClientCommand::Chat {
        kind: ChatKind::Say,
        target: None,
        text: format!(".go xyz {x:.1} {y:.1} {:.1} {}", m.z + 1.0, plan.course.map),
        language: None,
    });
    true
}

/// Logs each change of the race we are in, and leaving it.
fn log_race(test: &mut Autotest, races: &Races, now: f64) {
    let state = races
        .status()
        .map(|(id, roster, go, done)| (id, roster.to_vec(), go, done));
    if state == test.seen {
        return;
    }
    match &state {
        Some((id, roster, go, done)) => info!(
            "kart autotest: race {id} at {now:.1} s — roster {roster:?}, countdown {go}, finished {done}"
        ),
        None => info!("kart autotest: no race at {now:.1} s"),
    }
    test.seen = state;
}

pub(crate) fn plugin(app: &mut App) {
    if !active() {
        return;
    }
    app.init_resource::<Autotest>()
        .add_systems(Update, drive.in_set(crate::char_select::InWorldGated));
}
