//! Showcase runs (`/kartshow <name>`, GM only): a short choreographed drive at a famous place,
//! authored offline over the same world (`course-gen showcase`) and kept as an input tape in
//! `showcase/showcase.json` ([`kart_driver::showcase`]), played back for recording.
//!
//! The layer's `/kartshow <name>` asks the server (`R\tShow\t<name>`), which checks the account is
//! a GM, teleports us to the run's start and puts us back in the kart, then answers
//! `R\tShow\t<name>`; `/kartshow stop` is answered `R\tShow-`, and a bare `/kartshow` is listed by
//! the server. Here, once we are in the kart at the start, the kart is put on the run's start pose
//! exactly (the session's origin is the start, as the tape was driven), held there through a 3, 2,
//! 1 on the screen while the camera settles, put there afresh at GO and driven by the tape tick by
//! tick, with a gentle pull back onto the recorded line (`Playback`). It stops where the tape
//! does, braking to a standstill, and hands the kart back.
//!
//! The camera is the showcase's own ([`kartcraft::camera::CineCamera`]): the run's best first, and
//! the camera binding (CTR's L2, `C`) steps through the rest. The kart drives as the tape was
//! driven: in the run's class, with no Wumpa, CTR's ground types off and no turbo pads.
//!
//! `WOW_KART_SHOWCASE=<name>` plays a run unattended: it hops to the start itself (`.go xyz`, a GM
//! account) and needs no answer from the server; `WOW_KART_SHOWCASE_CUT=<s>` steps to the next
//! camera every so many seconds. Every step logs `kart showcase:`.

use std::cell::RefCell;
use std::sync::LazyLock;

use benilla_assets::coords::wow_to_bevy;
use benilla_protocol::SessionEvent;
use benilla_world::collision::WorldCollision;
use bevy::prelude::*;
use kart_driver::showcase::{Playback, Run, ShowcaseFile};
use kartcraft::camera::{CineCamera, CineMode, CineSubject};
use kartcraft::{Event, Input, KartView, Modifiers, Session};

use crate::net::{ChatKind, ClientCommand, NetCommands, SelfPlayer};

/// The runs, `showcase/showcase.json` (our own data: tapes and poses, nothing of the install).
pub(crate) static RUNS: LazyLock<ShowcaseFile> = LazyLock::new(|| {
    ShowcaseFile::from_json(include_str!("../../../../showcase/showcase.json"))
        .expect("showcase/showcase.json")
});

/// The 3, 2, 1 before GO, seconds.
const COUNTDOWN: f32 = 3.0;
/// How long the kart is waited for at the start, seconds, before giving up.
const ARRIVE_TIMEOUT: f32 = 45.0;
/// How near the start (yards) we must be, in the kart, before the run is set up there.
const ARRIVE_NEAR: f32 = 30.0;
/// How long the camera holds on the stopped kart after the run, seconds.
const HOLD_AFTER: f32 = 3.0;

#[derive(Default)]
enum Phase {
    #[default]
    Idle,
    /// Asked for `run`; on our way to its start.
    Arriving {
        run: usize,
        since: f32,
    },
    /// At the start, held there until GO.
    Countdown {
        run: usize,
        go_at: f32,
    },
    Playing {
        run: usize,
        play: Playback,
    },
    /// The tape is over; the camera holds on the kart until then.
    Done {
        until: f32,
    },
}

#[derive(Resource, Default)]
pub(crate) struct Showcase {
    phase: Phase,
    /// The server's lines.
    inbox: Vec<Vec<String>>,
    camera: Option<CineCamera>,
    /// The cameras in the order the binding steps through them, the run's best first.
    order: Vec<CineMode>,
    shown: usize,
    /// The unattended hook has asked.
    hooked: bool,
    /// Seconds on the camera showing, for the unattended hook's cuts.
    on_camera: f32,
}

/// The showcase's lines from the server.
pub(crate) fn on_chat(In(ev): In<SessionEvent>, mut show: ResMut<Showcase>) {
    let SessionEvent::Chat(m) = ev else {
        return;
    };
    if !m.is_addon() {
        return;
    }
    if let Some(body) = m.text.strip_prefix("KARTCRAFT\tR\t") {
        if body == "Show-" || body.starts_with("Show\t") {
            show.inbox
                .push(body.split('\t').map(str::to_string).collect());
        }
    }
}

/// The run named `name`, any case.
fn find(name: &str) -> Option<usize> {
    RUNS.runs
        .iter()
        .position(|r| r.name.eq_ignore_ascii_case(name))
}

/// A run's start, Bevy world.
fn start_of(run: &Run) -> Vec3 {
    wow_to_bevy(run.start)
}

/// The cameras of a run in the binding's order: its own list, then any it leaves out.
fn camera_order(run: &Run) -> Vec<CineMode> {
    let mut order: Vec<CineMode> = run
        .cameras
        .iter()
        .filter_map(|c| CineMode::named(c))
        .collect();
    for m in CineMode::ALL {
        if !order.contains(&m) {
            order.push(m);
        }
    }
    order
}

impl Showcase {
    /// A run under way: the kart is the tape's, not the player's.
    pub(crate) fn driving(&self) -> bool {
        matches!(
            self.phase,
            Phase::Countdown { .. } | Phase::Playing { .. } | Phase::Done { .. }
        )
    }

    fn arm(&mut self, run: usize, now: f32) {
        let r = &RUNS.runs[run];
        info!("kart showcase: {} ({}) asked for", r.name, r.title);
        self.phase = Phase::Arriving { run, since: now };
        self.order = camera_order(r);
        self.shown = 0;
        self.camera = None;
    }

    fn stop(&mut self, why: &str) {
        if !matches!(self.phase, Phase::Idle) {
            info!("kart showcase: stopped ({why})");
        }
        self.phase = Phase::Idle;
        self.camera = None;
    }

    /// The server's answers: a run to play, or stop; and a way to the start that never ends.
    pub(crate) fn hear(&mut self, now: f32) {
        for f in std::mem::take(&mut self.inbox) {
            match f.as_slice() {
                [verb] if verb == "Show-" => self.stop("asked"),
                [_, name, ..] => match find(name) {
                    Some(run) => self.arm(run, now),
                    None => warn!("kart showcase: no run named {name:?}"),
                },
                _ => {}
            }
        }
        if let Phase::Arriving { since, .. } = self.phase {
            if now - since > ARRIVE_TIMEOUT {
                warn!("kart showcase: never got to the start in the kart");
                self.stop("timed out");
            }
        }
    }

    /// Before the frame's ticks, once in the kart at `at` (Bevy world): on the way to the start,
    /// the run set up on its start pose once we are there and the world under it has streamed in.
    pub(crate) fn arrive(
        &mut self,
        session: &mut Session,
        collide: &WorldCollision,
        at: Vec3,
        now: f32,
    ) {
        let Phase::Arriving { run, .. } = self.phase else {
            return;
        };
        let r = &RUNS.runs[run];
        let start = start_of(r);
        let flat = Vec3::new(at.x - start.x, 0.0, at.z - start.z).length();
        if flat > ARRIVE_NEAR {
            return;
        }
        if put_on_start(session, collide, r, now) {
            session.pin(true);
            self.camera = Some(CineCamera::new(
                self.order.first().copied().unwrap_or(CineMode::Chase),
                wow_to_bevy(r.tripod),
            ));
            self.phase = Phase::Countdown {
                run,
                go_at: now + COUNTDOWN,
            };
            info!("kart showcase: {} at its start, 3, 2, 1", r.name);
        }
    }

    /// The frame's ticks while a run is under way, in place of the player's: the countdown held
    /// on the start, the tape, then the brakes. Returns what happened in them.
    pub(crate) fn step(
        &mut self,
        session: &mut Session,
        collide: &WorldCollision,
        ground: kartcraft::Ground,
        now: f32,
        dt: f32,
    ) -> Vec<Event> {
        // As the tape was driven: CTR's ground types off, no turbo pads, no Wumpa.
        session.set_ground(ground, false);
        session.pads_off(true);
        let mods = Modifiers::default();
        let brake = Input {
            brake: true,
            ..Default::default()
        };
        match &mut self.phase {
            Phase::Countdown { run, go_at } => {
                let run = *run;
                if now < *go_at {
                    return session.step(collide, now, dt, &Input::default(), &mods, &[]);
                }
                // GO: the kart put on the start afresh, as the tape starts.
                let r = &RUNS.runs[run];
                if !put_on_start(session, collide, r, now) {
                    return Vec::new();
                }
                session.pin(false);
                match Playback::new(r) {
                    Ok(play) => {
                        info!(
                            "kart showcase: {} GO ({} ticks, {} boosts)",
                            r.name,
                            play.len(),
                            r.boosts
                        );
                        self.phase = Phase::Playing { run, play };
                    }
                    Err(e) => {
                        warn!("kart showcase: {}: {e}", r.name);
                        self.stop("a bad tape");
                    }
                }
                Vec::new()
            }
            Phase::Playing { run, play } => {
                let run = *run;
                let cell = RefCell::new(play);
                let events = session.step_driven(
                    collide,
                    now,
                    dt,
                    |_| cell.borrow().input().unwrap_or(brake),
                    &mods,
                    &[],
                    |_, kart, _| {
                        let mut p = cell.borrow_mut();
                        if !p.done() {
                            p.after_tick(kart);
                        }
                    },
                );
                let play = cell.into_inner();
                let boosts = events
                    .iter()
                    .filter(|e| matches!(e, Event::Boost { .. }))
                    .count();
                if boosts > 0 {
                    info!(
                        "kart showcase: boost at tick {} (gap to the line {:.1} lu)",
                        play.tick(),
                        play.gap
                    );
                }
                if play.done() {
                    info!(
                        "kart showcase: {} done: {} ticks, the most the kart strayed from the \
                         line {:.1} lu ({:.2} yd)",
                        RUNS.runs[run].name,
                        play.tick(),
                        play.max_gap,
                        play.max_gap / kartcraft::lu_per_yard()
                    );
                    self.phase = Phase::Done {
                        until: now + HOLD_AFTER,
                    };
                    // Eased back to the chase camera while the kart stands, so handing the kart
                    // back is no cut.
                    if let Some(cam) = self.camera.as_mut() {
                        cam.set_mode(CineMode::Chase);
                    }
                }
                events
            }
            Phase::Done { until } => {
                if now >= *until {
                    session.pads_off(false);
                    self.stop("the run is over");
                }
                session.step(collide, now, dt, &brake, &mods, &[])
            }
            _ => Vec::new(),
        }
    }

    /// The showcase camera's shot of the kart this frame (and the binding's press steps to the
    /// next camera); `None` with no run under way.
    pub(crate) fn frame(
        &mut self,
        view: &KartView,
        next: bool,
        dt: f32,
    ) -> Option<(Transform, f32)> {
        if !self.driving() {
            return None;
        }
        let cam = self.camera.as_mut()?;
        // `WOW_KART_SHOWCASE_CUT=<s>`: the unattended hook steps to the next camera that often.
        static CUT: LazyLock<Option<f32>> = LazyLock::new(|| {
            std::env::var("WOW_KART_SHOWCASE_CUT")
                .ok()
                .and_then(|v| v.parse().ok())
        });
        self.on_camera += dt;
        let cut = CUT.is_some_and(|every| self.on_camera >= every);
        if (next || cut) && !self.order.is_empty() {
            self.on_camera = 0.0;
            self.shown = (self.shown + 1) % self.order.len();
            cam.set_mode(self.order[self.shown]);
            info!("kart showcase: camera {}", self.order[self.shown].name());
        }
        let forward = Vec3::new(-view.yaw.sin(), 0.0, -view.yaw.cos());
        let subject = CineSubject {
            position: view.transform.translation,
            forward,
            velocity: forward * view.speed,
        };
        let chase = view
            .camera
            .map(|c| (c.translation, c.translation + c.forward() * 10.0));
        let shot = cam.shot(&subject, chase, view.fov, dt);
        let t = Transform::from_translation(shot.eye).looking_at(shot.look, Vec3::Y);
        Some((t, shot.fov))
    }

    /// The banner: the 3, 2, 1 before GO, then none (a course's "starts here" would be in the
    /// recording); `None` leaves the race's.
    pub(crate) fn banner(&self, now: f32) -> Option<String> {
        match &self.phase {
            Phase::Idle => None,
            Phase::Arriving { run, .. } => Some(format!("Showcase: {}", RUNS.runs[*run].title)),
            Phase::Countdown { go_at, .. } => {
                Some(((go_at - now).ceil().clamp(1.0, 3.0) as u32).to_string())
            }
            Phase::Playing { .. } | Phase::Done { .. } => Some(String::new()),
        }
    }
}

/// Puts the kart on `run`'s start pose, as the tape was driven from: the session's origin the
/// start, the kart on the ground under it at the run's heading, in the run's class.
fn put_on_start(session: &mut Session, collide: &WorldCollision, run: &Run, now: f32) -> bool {
    let at = start_of(run);
    match session.start(
        collide,
        at,
        kartcraft::yaw_of(run.heading),
        run.class(),
        now,
    ) {
        Ok(0) => false,
        Ok(_) => true,
        Err(e) => {
            warn!("kart showcase: {e}");
            false
        }
    }
}

/// The 3, 2, 1 over the race's banner, after the race has written its own.
pub(crate) fn banner(
    time: Res<Time>,
    show: Res<Showcase>,
    mut text: ResMut<super::race::RaceText>,
) {
    if let Some(b) = show.banner(time.elapsed_secs()) {
        text.banner = b;
    }
}

/// The unattended hook, `WOW_KART_SHOWCASE=<name>`: once in the world, hops to the run's start
/// (`.go xyz`, a GM account) and plays it, with no word from the server.
pub(crate) fn hook(
    time: Res<Time>,
    me: Query<(), With<SelfPlayer>>,
    net: Option<Res<NetCommands>>,
    mut show: ResMut<Showcase>,
) {
    static NAME: LazyLock<Option<String>> =
        LazyLock::new(|| std::env::var("WOW_KART_SHOWCASE").ok());
    let (Some(name), Some(net)) = (NAME.as_deref(), net) else {
        return;
    };
    if show.hooked || me.single().is_err() || time.elapsed_secs() < 10.0 {
        return;
    }
    show.hooked = true;
    let Some(run) = find(name) else {
        error!("kart showcase: no run named {name:?}");
        return;
    };
    let r = &RUNS.runs[run];
    let [x, y, z] = r.start;
    info!(
        "kart showcase: hook hops to {} at {x:.1} {y:.1} {z:.1}",
        r.name
    );
    let _ = net.0.send(ClientCommand::Chat {
        kind: ChatKind::Say,
        target: None,
        text: format!(".go xyz {x:.2} {y:.2} {:.2} {}", z + 0.5, r.map),
        language: None,
    });
    let now = time.elapsed_secs();
    show.arm(run, now);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_run_reads_and_names_its_cameras() {
        assert!(!RUNS.runs.is_empty());
        for r in &RUNS.runs {
            assert!(r.ticks().is_ok(), "{}", r.name);
            let order = camera_order(r);
            assert_eq!(order.len(), 4, "{}: every camera once", r.name);
            assert_eq!(
                Some(order[0].name()),
                r.cameras.first().map(String::as_str),
                "{}: its best first",
                r.name
            );
            assert!(r.map <= 1);
            assert_eq!(find(&r.name.to_uppercase()), Some(find(&r.name).unwrap()));
        }
    }

    #[test]
    fn the_server_s_answers_arm_and_stop_a_run() {
        let mut show = Showcase::default();
        let name = RUNS.runs[0].name.clone();
        show.inbox.push(vec!["Show".into(), name]);
        show.hear(1.0);
        assert!(matches!(show.phase, Phase::Arriving { run: 0, .. }));
        assert_eq!(
            show.banner(1.0),
            Some(format!("Showcase: {}", RUNS.runs[0].title))
        );
        show.inbox.push(vec!["Show-".into()]);
        show.hear(2.0);
        assert!(matches!(show.phase, Phase::Idle));
        // A name the client does not know arms nothing.
        show.inbox.push(vec!["Show".into(), "atlantis".into()]);
        show.hear(3.0);
        assert!(matches!(show.phase, Phase::Idle));
    }

    #[test]
    fn the_countdown_counts_three_two_one() {
        let show = Showcase {
            phase: Phase::Countdown {
                run: 0,
                go_at: 10.0,
            },
            ..Default::default()
        };
        assert_eq!(show.banner(7.1).as_deref(), Some("3"));
        assert_eq!(show.banner(8.5).as_deref(), Some("2"));
        assert_eq!(show.banner(9.9).as_deref(), Some("1"));
        assert!(show.driving());
        // Under way, the screen is the run's: no banner at all.
        let done = Showcase {
            phase: Phase::Done { until: 20.0 },
            ..Default::default()
        };
        assert_eq!(done.banner(15.0).as_deref(), Some(""));
        assert_eq!(Showcase::default().banner(15.0), None);
    }
}
