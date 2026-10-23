//! The Race Marshal test hook, `WOW_KART_MARSHAL=<role>:<course>[:leave=<s>]`: races through the
//! marshal's gossip as a player does. It hops beside the course's marshal (`courses.json`'s
//! `marshal`) with `.go xyz` (a GM account), finds the nearest unit named "Race Marshal", opens its
//! menu (`CMSG_GOSSIP_HELLO`) and clicks an option by its text (`CMSG_GOSSIP_SELECT_OPTION`).
//!
//! - `host:<course>` clicks "Host a race: <name>", then on the page that asks for the race's
//!   ground types "Host <name>: ground types off" (`:ground` for "...: ground types on"), then
//!   "Start the race" once another racer is in.
//! - `trial:<course>` clicks "Time trial: <name>": a race of one, under way at once.
//! - `join:<course>` clicks "Join ...'s race on <name>", asking again until a race is offered.
//! - `queue:<course>` hops to Northshire's marshal instead, queues for the course in the Race
//!   Finder (its Lua, as the window's button does), accepts the ready check on its popup, races,
//!   and once out of the race clicks "Return" on the popup that offers the way back.
//! - `:leave=<s>` sends `/leaverace`'s line (`R\tX`) that long after GO.
//! - `:warp=<s>` hops to the next checkpoint when the autopilot has not reached it in that long
//!   (it steers straight at it, and courses bend round buildings).
//! - `host:<course>:alone` starts the race with us alone in it (the server takes a race of one;
//!   its roll is then any item, `Kartcraft.cpp` `Roll`'s fallback); `:racers=<n>` once n are in.
//!
//! `WOW_KART_MARSHAL_NOGO=1` skips the hop. Every step logs `kart autotest:`.
//!
//! The crate test, `WOW_KART_CRATE_TEST=1` with the autopilot (`WOW_KART_AUTOPILOT=1`) in a race
//! with weapons: it breaks crates with our shots, the `W K` path. Empty-handed it steers for the
//! nearest weapon crate ahead, to take it; holding a bomb, missile, warp orb or shield it steers
//! for the nearest full-grown crate ahead and fires (the shield's second press, as a shield bomb)
//! once lined up within range; any other item it fires at once, to roll again. The break logs
//! `kart weapons: our <item> (shot <n>) broke crate <i>`, the server's `X` for it `kart weapons:
//! the server took crate <i> ... our shot <n> broke`. Its own steps log `kart autotest: crate`.

use benilla_assets::coords::bevy_to_wow;
use bevy::prelude::*;

use super::race::{Races, COURSES};
use crate::names::NameCache;
use crate::net::{ChatKind, ClientCommand, Guid, NetCommands, NetEntity, ObjectStore, SelfPlayer};
use crate::player::Player;
use crate::ui_gossip::GossipState;
use benilla_ui::script::UiScript;

/// The marshal's name (`creature_template` 190000).
const MARSHAL: &str = "Race Marshal";
/// How far from the hop a marshal is looked for (yards).
const SEARCH: f32 = 40.0;
/// How near the marshal a hello is sent (yards; the server's reach is 5 plus her size).
const REACH: f32 = 4.5;
/// Seconds before asking the marshal again when the option is not (yet) on the menu.
const RETRY: f64 = 4.0;
/// Where `queue:` waits for its race (Northshire's marshal, WoW x, y, z): far from the course, so
/// the way back is seen to go somewhere.
const QUEUE_SPOT: [f32; 3] = [-8930.5, -127.4, 83.6];

#[derive(Clone, Copy, Debug, PartialEq)]
enum Role {
    Host,
    Join,
    Queue,
    Trial,
}

/// What the hook does next.
#[derive(Debug, Default, PartialEq)]
enum Step {
    #[default]
    Boot,
    Hopped(f64),
    /// Asking the marshal for the options containing the texts, one menu page after another
    /// (`at` the page we are on); `sent` is when the hello, or the last click, went.
    Asking {
        wants: Vec<String>,
        at: usize,
        sent: Option<f64>,
        tries: u32,
    },
    /// Hosted or joined: waiting for the roster (host: for a second racer).
    Waiting,
    Racing,
    /// `queue:` queued at this time; the ready check is accepted on its popup.
    Queued(f64),
    /// `queue:` out of its race since then: "Return" is clicked when the popup offers it, and
    /// where we are is logged when it is.
    Returning {
        since: f64,
        clicked: Option<f64>,
    },
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

#[derive(Debug)]
struct Plan {
    role: Role,
    course: &'static super::race::Course,
    leave_after: Option<f64>,
    warp_after: Option<f64>,
    /// `host:` hosts with ground types on (`:ground` or `:ground=1`).
    ground: bool,
    /// `host:` starts the race with us alone in it (`:alone`).
    alone: bool,
    /// `host:` starts it once this many are in (`:racers=<n>`), else two.
    racers: Option<usize>,
}

fn plan() -> Option<&'static Plan> {
    static PLAN: std::sync::OnceLock<Option<Plan>> = std::sync::OnceLock::new();
    PLAN.get_or_init(|| parse(&std::env::var("WOW_KART_MARSHAL").ok()?))
        .as_ref()
}

/// `WOW_KART_MARSHAL`'s value, `<role>:<course>[:<option>...]`, as a plan.
fn parse(spec: &str) -> Option<Plan> {
    let mut parts = spec.split(':');
    let role = match parts.next()? {
        "host" => Role::Host,
        "join" => Role::Join,
        "queue" => Role::Queue,
        "trial" => Role::Trial,
        other => {
            error!("kart autotest: unknown role {other:?} in WOW_KART_MARSHAL={spec}");
            return None;
        }
    };
    let key = parts.next().unwrap_or("gp-azora");
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
    let ground = rest.iter().any(|p| *p == "ground" || *p == "ground=1");
    let alone = rest.contains(&"alone");
    let racers = rest
        .iter()
        .find_map(|p| p.strip_prefix("racers="))
        .and_then(|n| n.parse().ok());
    Some(Plan {
        role,
        course,
        leave_after: option("leave"),
        warp_after: option("warp"),
        ground,
        alone,
        racers,
    })
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
    script: Option<NonSend<UiScript>>,
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
            if plan.role == Role::Queue {
                let [x, y, z] = QUEUE_SPOT;
                info!(
                    "kart autotest: queue on {} — hopping to Northshire",
                    plan.course.key
                );
                send(ClientCommand::Chat {
                    kind: ChatKind::Say,
                    target: None,
                    text: format!(".go xyz {x:.1} {y:.1} {z:.1} 0"),
                    language: None,
                });
            } else if !hop(plan, &send) {
                test.step = Step::Done;
                return;
            }
            test.hops += 1;
            test.step = Step::Hopped(now);
        }
        Step::Hopped(at) => {
            if now - *at > 5.0 && plan.role == Role::Queue {
                let Some(script) = script.as_deref() else {
                    return;
                };
                let w = bevy_to_wow(player.pos);
                info!(
                    "kart autotest: at ({:.1}, {:.1}, {:.1}) — queueing for {}",
                    w[0], w[1], w[2], plan.course.key
                );
                // As the window's Find race does, for one course.
                let queued = script.run(&format!(
                    "local f = BenillaKartFinder \
                     f.queued, f.waiting, f.waited, f.noticeUntil = '{0}', nil, nil, 0 \
                     BenillaKart_Send('Q+\\t{0}') \
                     f.beat = GetTime() + BENILLA_KART_HEARTBEAT",
                    plan.course.key
                ));
                if let Err(e) = queued {
                    error!("kart autotest: FAILURE — queueing: {e}");
                    test.step = Step::Done;
                    return;
                }
                test.step = Step::Queued(now);
            } else if now - *at > 5.0 {
                test.step = ask(plan);
            }
        }
        Step::Queued(at) => {
            if races.status().is_some() {
                test.step = Step::Waiting;
                return;
            }
            if click_popup(script.as_deref(), "BENILLA_KART_READY", 1) {
                info!("kart autotest: ready check — clicked Accept");
            }
            if now - *at > 180.0 {
                error!("kart autotest: FAILURE — no race in 180 s of queueing");
                test.step = Step::Done;
            }
        }
        Step::Returning { since, clicked } => match clicked {
            None => {
                if click_popup(script.as_deref(), "BENILLA_KART_RETURN", 1) {
                    info!("kart autotest: the way back offered — clicked Return");
                    *clicked = Some(now);
                } else if now - *since > 30.0 {
                    error!("kart autotest: FAILURE — no way back offered in 30 s");
                    test.step = Step::Done;
                }
            }
            Some(at) if now - *at > 8.0 => {
                let w = bevy_to_wow(player.pos);
                info!(
                    "kart autotest: back at ({:.1}, {:.1}, {:.1}) — done",
                    w[0], w[1], w[2]
                );
                test.step = Step::Done;
            }
            Some(_) => {}
        },
        Step::Asking {
            wants,
            at,
            sent,
            tries,
        } => {
            // Already in a race here (one left waiting from before is still ours): on to it.
            if matches!(plan.role, Role::Host | Role::Trial)
                && !test.start_asked
                && races.status().is_some()
            {
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
                let want = wants[*at].as_str();
                if let Some(o) = options.iter().find(|o| o.message.contains(want)) {
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
                    // The next page comes as the answer to the click: no hello for it.
                    if *at + 1 < wants.len() {
                        *at += 1;
                        *sent = Some(now);
                    } else {
                        test.step = Step::Waiting;
                    }
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
                    error!("kart autotest: FAILURE — never offered {:?}", wants[*at]);
                    test.step = Step::Done;
                    return;
                }
                *tries += 1;
                // A hello opens the first page again.
                *at = 0;
                info!(
                    "kart autotest: GossipHello to {marshal:#x} (try {tries}) for {:?}",
                    wants[0]
                );
                send(ClientCommand::GossipHello { guid: marshal });
                *sent = Some(now);
            }
        }
        Step::Waiting => {
            if let Some((_, roster, go, _)) = races.status() {
                if go {
                    test.step = Step::Racing;
                } else if plan.role == Role::Host
                    && roster.len()
                        >= if plan.alone {
                            1
                        } else {
                            plan.racers.unwrap_or(2)
                        }
                    && !test.start_asked
                {
                    test.start_asked = true;
                    test.step = Step::Asking {
                        wants: vec!["Start the race".into()],
                        at: 0,
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
                                // `.go xy`: the ground's height there (vmangos `HandleGoHelper`).
                                // Our own, a checkpoint uphill of us, put us inside the hill.
                                text: format!(".go xy {:.1} {:.1}", p[0], p[1]),
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
                if plan.role == Role::Queue {
                    info!("kart autotest: out of the race — waiting for the way back");
                    test.step = Step::Returning {
                        since: now,
                        clicked: None,
                    };
                } else {
                    info!("kart autotest: out of the race — done");
                    test.step = Step::Done;
                }
            }
        }
        Step::Done => {}
    }
}

/// Clicks button `button` of the stock popup `which` when it is up; whether it was.
fn click_popup(script: Option<&UiScript>, which: &str, button: u8) -> bool {
    let Some(script) = script else {
        return false;
    };
    script
        .eval::<bool>(&format!(
            "local n = StaticPopup_Visible and StaticPopup_Visible('{which}') \
             if n then getglobal(n .. 'Button{button}'):Click() return true end \
             return false"
        ))
        .unwrap_or(false)
}

fn ask(plan: &Plan) -> Step {
    Step::Asking {
        wants: wants(plan.role, &plan.course.name, plan.ground),
        at: 0,
        sent: None,
        tries: 0,
    }
}

/// The marshal's options a role clicks, a menu page each: hosting asks for the race's ground
/// types on a page of its own (`Kartcraft.cpp` `MARSHAL_HOST_PLAIN`, `MARSHAL_HOST_GROUND`).
fn wants(role: Role, course: &str, ground: bool) -> Vec<String> {
    match role {
        Role::Host | Role::Queue => vec![
            format!("Host a race: {course}"),
            format!(
                "Host {course}: ground types {}",
                if ground { "on" } else { "off" }
            ),
        ],
        Role::Join => vec![format!("race on {course}")],
        Role::Trial => vec![format!("Time trial: {course}")],
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
            Role::Host | Role::Queue | Role::Trial => 0.9,
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

/// `WOW_KART_CRATE_TEST`: the autopilot breaks crates with our shots.
pub(crate) fn crate_test() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("WOW_KART_CRATE_TEST").is_some())
}

/// Crates nearer than this (yards) are not aimed for: the kart would reach one before its shot.
const CRATE_NEAR: f32 = 8.0;
/// Crates farther than this (yards) are not aimed for.
const CRATE_FAR: f32 = 70.0;
/// A crate this far off our heading (radians) is not ahead.
const CRATE_AHEAD: f32 = 1.2;
/// A shot goes when the crate is within this many yards and this far off our heading.
const FIRE_RANGE: f32 = 40.0;
const FIRE_LINED_UP: f32 = 0.04;
/// Seconds between the crate test's presses of Fire.
const FIRE_EVERY: f32 = 1.0;

/// The crate test's memory: the crate it aims for, logged once each, and its last press.
#[derive(Default)]
pub(crate) struct CrateTest {
    aimed: Option<(usize, bool)>,
    fired_at: Option<f32>,
}

/// What the crate test asks of the autopilot this frame: a place to steer for instead of the
/// next checkpoint, and whether to press Fire.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct CrateAim {
    pub(crate) target: Option<Vec3>,
    pub(crate) fire: bool,
}

/// The crate test's steering and Fire for our kart at `pos`, facing `yaw` (the autopilot's yaw:
/// forward is `-sin, -cos`), `now` seconds in, in a race with weapons.
pub(crate) fn crate_aim(
    test: &mut CrateTest,
    arms: &kartcraft::weapons::Weapons,
    pos: Vec3,
    yaw: f32,
    now: f32,
) -> CrateAim {
    use kartcraft::weapons::{wire::CrateKind, HeldView, Item};
    let shield_up = arms.our_look().shield.is_some();
    // A rolling bomb is ours to leave alone: a press sets it off.
    if arms.fires_again() && !shield_up {
        return CrateAim::default();
    }
    let (shooting, fire_now) = match arms.held() {
        _ if shield_up => (true, false),
        HeldView::Item(item) => match item.single() {
            Item::Bomb | Item::Missile | Item::WarpOrb => (true, false),
            // Up first; its second press is the shield bomb.
            Item::Shield => (false, true),
            _ => (false, true),
        },
        HeldView::Spinning | HeldView::Fading(_) => return CrateAim::default(),
        HeldView::Empty => (false, false),
    };
    let rested = test.fired_at.is_none_or(|t| now - t >= FIRE_EVERY);
    if fire_now {
        test.aimed = None;
        if !rested {
            return CrateAim::default();
        }
        test.fired_at = Some(now);
        info!(
            "kart autotest: crate — firing {:?} away to roll again",
            arms.held()
        );
        return CrateAim {
            target: None,
            fire: true,
        };
    }
    let off_of = |at: Vec3| {
        let to = at - pos;
        let want = f32::atan2(-to.x, -to.z);
        let off = (want - yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
            - std::f32::consts::PI;
        (Vec2::new(to.x, to.z).length(), off)
    };
    let pick = arms
        .crates()
        .iter()
        .enumerate()
        .filter(|(_, c)| c.up && arms.crate_scale(c) >= 1.0)
        .filter(|(_, c)| {
            if shooting {
                c.kind != CrateKind::Wumpa
            } else {
                c.kind == CrateKind::Weapon
            }
        })
        .map(|(i, c)| (i, c.at, off_of(c.at)))
        .filter(|(_, _, (d, off))| {
            *d <= CRATE_FAR && off.abs() <= CRATE_AHEAD && (!shooting || *d >= CRATE_NEAR)
        })
        .min_by(|a, b| a.2 .0.total_cmp(&b.2 .0));
    let Some((i, at, (d, off))) = pick else {
        test.aimed = None;
        return CrateAim::default();
    };
    if test.aimed != Some((i, shooting)) {
        info!(
            "kart autotest: crate — {} crate {i}, {d:.1} yd ahead, holding {:?}",
            if shooting { "aiming at" } else { "driving for" },
            arms.held()
        );
        test.aimed = Some((i, shooting));
    }
    let fire = rested && shooting && d <= FIRE_RANGE && off.abs() <= FIRE_LINED_UP;
    if fire {
        info!(
            "kart autotest: crate — firing {:?} at crate {i}, {d:.1} yd, {off:.3} rad off",
            arms.held()
        );
        test.fired_at = Some(now);
    }
    CrateAim {
        target: Some(at),
        fire,
    }
}

pub(crate) fn plugin(app: &mut App) {
    if !active() {
        return;
    }
    app.init_resource::<Autotest>()
        .add_systems(Update, drive.in_set(crate::char_select::InWorldGated));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hook_reads_its_role_course_and_options() {
        let course = &COURSES[0];
        let plan = parse(&format!("trial:{}:warp=4", course.key)).unwrap();
        assert_eq!(plan.role, Role::Trial);
        assert_eq!(plan.course.key, course.key);
        assert_eq!((plan.warp_after, plan.leave_after), (Some(4.0), None));
        assert!(!plan.ground);
        for spec in [
            "host:{}:ground",
            "host:{}:ground=1",
            "host:{}:leave=9:ground",
        ] {
            let plan = parse(&spec.replace("{}", &course.key)).unwrap();
            assert!(plan.role == Role::Host && plan.ground, "{spec}");
        }
        assert!(
            !parse(&format!("host:{}:ground=0", course.key))
                .unwrap()
                .ground
        );
        let plan = parse(&format!("host:{}:alone:warp=3", course.key)).unwrap();
        assert!(plan.alone && !plan.ground && plan.warp_after == Some(3.0));
        assert!(parse("drive:sprint").is_none(), "an unknown role");
        assert!(parse("host:no-such-course").is_none());
    }

    #[test]
    fn hosting_clicks_through_the_ground_types_page_and_a_trial_is_one_click() {
        // The menu texts the server writes (`Kartcraft.cpp` MarshalHello and the host page).
        let name = "Tower of Azora & Jasperlode Mine";
        let main = format!("Host a race: {name} (3 laps)");
        let plain = format!("Host {name}: ground types off");
        let ground = format!("Host {name}: ground types on (off-road slows)");
        let trial = format!("Time trial: {name} (3 laps)");
        let pick = |wants: &[String], menus: &[&[&String]]| -> Vec<String> {
            wants
                .iter()
                .zip(menus)
                .map(|(want, menu)| {
                    let hits: Vec<&&String> = menu.iter().filter(|o| o.contains(want)).collect();
                    assert_eq!(hits.len(), 1, "{want:?} picks one of {menu:?}");
                    hits[0].to_string()
                })
                .collect()
        };
        let pages: [&[&String]; 2] = [&[&main, &trial], &[&plain, &ground]];
        assert_eq!(
            pick(&wants(Role::Host, name, false), &pages),
            [main.clone(), plain.clone()]
        );
        assert_eq!(
            pick(&wants(Role::Host, name, true), &pages),
            [main.clone(), ground.clone()]
        );
        assert_eq!(
            pick(&wants(Role::Trial, name, false), &pages),
            std::slice::from_ref(&trial)
        );
    }
}
