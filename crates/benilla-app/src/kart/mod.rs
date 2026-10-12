//! Kart mode (World of Kartcraft): Summon Kart's key (`J`) seats our character in a Crash Team
//! Racing kart driven by CTR's own driving model (`ctr-kart`, ported from the decompilation)
//! through a [`kartcraft::Session`]. While we drive, the kart's pose is our position, facing and
//! camera (read by `player::controller`), our character holds the saddle pose in the kart's seat
//! ([`seat`]) and the kart is drawn under it (`kartcraft::model`). Banked turbo chains feed the
//! Karting profession ([`profession`]) and the score on screen ([`score`]). In a race CTR's
//! weapons are live ([`weapons`]): crates, items, shots and hits through the server; in a time
//! trial there are none, and the ghost of our best run on the course races us ([`ghost`]). A GM's
//! `/kartshow` plays a recorded showcase run through cinematic cameras ([`showcase`]). Its
//! settings
//! are CVars the layer's settings window writes ([`settings`]); its controls are key bindings
//! ([`bindings`]).

mod autotest;
mod bindings;
pub(crate) mod freecam;
mod garage;
mod ghost;
mod ground;
pub(crate) mod marshals;
mod pads;
mod profession;
mod race;
mod remote;
mod score;
mod seat;
pub(crate) mod settings;
mod showcase;
mod sound;
mod ui;
pub(crate) use garage::{class_list, racer_list};
pub(crate) use race::{RaceMap, COURSES, ZONES};
pub(crate) use seat::SEAT_ANIMS;
pub(crate) use ui::{KartOverlay, RacerDot, ScoreLines};
mod weapons;

use benilla_world::collision::WorldCollision;
use benilla_world::schedule::WorldStage;
use bevy::input::gamepad::Gamepad;
use bevy::prelude::*;
use kartcraft::chain::{Chain, Outcome};
use kartcraft::{input, model, KartView, Modifiers, Session, State};

use crate::net::NetHandlerApp;

/// How often our kart's state is relayed (s).
const STATE_PERIOD: f32 = 0.1;
/// Other karts this close (yards) are checked for bumps.
const BUMP_RANGE: f32 = 20.0;
/// Below this speed (yd/s) the kart counts as standing still on the wire.
const MOVING_SPEED: f32 = 0.5;

/// What the controller reads while we drive: it drives the body and the wire from this instead of
/// the mover.
#[derive(Resource, Default)]
pub(crate) struct KartDrive {
    pub(crate) active: bool,
    /// The kart's origin (its bottom centre), Bevy world.
    pub(crate) pos: Vec3,
    /// Facing (Bevy yaw): the kart's heading.
    pub(crate) yaw: f32,
    /// Speed, yd/s.
    pub(crate) speed: f32,
    /// The chase camera, Bevy world.
    pub(crate) camera: Option<Transform>,
    /// The chase camera's vertical field of view, radians (`KartCamFov`).
    pub(crate) fov: f32,
    /// Drive with the chase camera rather than our follow camera (`KartChaseCamera`).
    pub(crate) engine_camera: bool,
}

/// The kart's drive for the frame, which writes [`KartDrive`].
#[derive(bevy::ecs::schedule::SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct KartDriveSet;

impl KartDrive {
    pub(crate) fn moving(&self) -> bool {
        self.speed > MOVING_SPEED
    }

    /// The field of view the world camera takes while the chase camera drives it.
    pub(crate) fn camera_fov(&self) -> Option<f32> {
        (self.active && self.engine_camera && self.camera.is_some() && self.fov > 0.0)
            .then_some(self.fov)
    }
}

#[derive(Resource, Default)]
struct Host {
    session: Session,
    chain: Chain,
    /// The `WOW_KART_AUTOSTART` hook: when we became free to drive, and whether it has fired.
    ready_since: Option<f32>,
    autostarted: bool,
    /// We cancelled Summon Kart and its aura has not gone yet: do not drive on it.
    dismissing: bool,
    /// Turbo Boost's aura was on us last frame.
    turbo_on: bool,
    /// The race autopilot: since when it has been stalled, and until when it backs away.
    stalled_since: Option<f32>,
    backing_until: f32,
    /// The `WOW_KART_CRATE_TEST` hook's aim (`autotest`).
    crate_test: autotest::CrateTest,
    /// When our kart's state was last relayed to the players around us.
    state_sent: f32,
    /// The body's last teleport settle we saw begin (`Player::settle_since`).
    settle_seen: f32,
    /// The engine class the kart drives with.
    class: kartcraft::Class,
}

fn autostart_after() -> Option<f32> {
    static AFTER: std::sync::OnceLock<Option<f32>> = std::sync::OnceLock::new();
    *AFTER.get_or_init(|| std::env::var("WOW_KART_AUTOSTART").ok()?.parse().ok())
}

fn stop(host: &mut Host, drive: &mut KartDrive, view: &mut KartView) {
    if drive.active {
        info!("kart: out of the kart");
    }
    host.session.stop();
    drive.active = false;
    drive.camera = None;
    *view = KartView::default();
}

fn start(
    time: &Time,
    collide: &WorldCollision,
    player: &crate::player::Player,
    host: &mut Host,
    class: kartcraft::Class,
) -> bool {
    match host.session.start(
        collide,
        player.pos,
        player.face_yaw(),
        class,
        time.elapsed_secs(),
    ) {
        // The world under us has not streamed in yet: ask again next frame.
        Ok(0) => false,
        Ok(faces) => {
            info!(
                "kart: in the kart ({} class, {faces} collision faces)",
                class.name()
            );
            host.chain = Chain::default();
            host.class = class;
            true
        }
        Err(e) => {
            warn!("kart: {e}");
            false
        }
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update(
    time: Res<Time>,
    (mut overlay, binds, mut cvars): (
        ResMut<crate::bindings::BindingOverlay>,
        Res<crate::bindings::BindingsState>,
        ResMut<crate::cvars::Cvars>,
    ),
    pads: Query<&Gamepad>,
    ui: Res<crate::ui_script::UiKeyboardCapture>,
    player: Res<crate::player::Player>,
    collide: WorldCollision,
    body: Query<Entity, With<crate::net::Embodied>>,
    mut sheath: MessageWriter<crate::creature_anim::SheathRequest>,
    mut host: ResMut<Host>,
    mut drive: ResMut<KartDrive>,
    mut view: ResMut<KartView>,
    mut audio: ResMut<sound::KartAudio>,
    profession: profession::Profession,
    mut score: ResMut<score::KartScore>,
    settings: Res<settings::KartSettings>,
    (remotes, mut extras, races, mut arms, mut rumble, mut show, mut free, mouse, buttons): (
        Res<remote::RemoteKarts>,
        ResMut<kartcraft::hud::HudExtras>,
        Res<race::Races>,
        ResMut<kartcraft::weapons::Weapons>,
        ResMut<kartcraft::rumble::Rumble>,
        ResMut<showcase::Showcase>,
        ResMut<freecam::FreeCam>,
        Res<bevy::input::mouse::AccumulatedMouseMotion>,
        Res<ButtonInput<MouseButton>>,
    ),
) {
    let now = time.elapsed_secs();
    let host = &mut *host;
    show.hear(now);
    // On a Kartcraft server the Summon Kart aura is the ride.
    let server_kart = profession.server_kart();
    let auras = profession.auras();
    // Something else has the body (a teleport's settle, a taxi, a possession): the ride pauses.
    // The Summon Kart aura stays, so the kart comes back when the body is ours again; a taxi or
    // a charm ends the aura on the server's side anyway.
    if drive.active && !player.may_drive() {
        info!("kart: out of the kart for now: {}", player.drive_refusal());
        stop(host, &mut drive, &mut view);
    }
    // A teleport (a race's grid, a `.go`) whose settle ended in its own frame, as a short hop
    // over loaded ground does, never refused the drive: the kart starts again where it put us,
    // or its old pose would carry us back.
    let teleported =
        std::mem::replace(&mut host.settle_seen, player.settle_since) != player.settle_since;
    if drive.active && teleported {
        info!("kart: out of the kart for now: teleported");
        stop(host, &mut drive, &mut view);
    }
    drive.engine_camera = settings.chase_camera();
    // The garage's class as our Karting skill allows it; a new pick or a new tier applies at once.
    let class = garage::effective(settings.class, score.skill());
    if drive.active && host.class != class {
        if let Some(kart) = host.session.kart_mut() {
            kart.set_class(class);
            host.class = class;
            info!("kart: now in the {} class", class.name());
        }
    }
    // Test hook: `WOW_KART_AUTOSTART=<seconds>` presses `J` once, that long after we may drive.
    let autostart = autostart_after().is_some_and(|after| {
        if !player.may_drive() || host.autostarted {
            host.ready_since = None;
            return false;
        }
        let since = *host.ready_since.get_or_insert(now);
        let due = now - since >= after;
        host.autostarted |= due;
        due
    });
    let mut started = false;
    // The test hook only ever climbs in: a kart aura kept over a logout already has us in it.
    let autostart = autostart && !drive.active && !auras.kart;
    // Summon Kart's binding (`J` by default); the spellbook's Summon Kart is the same toggle.
    if binds.ran(bindings::SUMMON) || autostart {
        if server_kart {
            if drive.active || auras.kart {
                profession.dismiss();
                host.dismissing = true;
                stop(host, &mut drive, &mut view);
            } else if player.may_drive() {
                profession.summon();
            }
        } else if drive.active {
            stop(host, &mut drive, &mut view);
        } else if player.may_drive() {
            started = start(&time, &collide, &player, host, class);
        }
    }
    if server_kart {
        if !auras.kart {
            host.dismissing = false;
            if drive.active {
                info!("kart: out of the kart: the Summon Kart aura is gone");
                stop(host, &mut drive, &mut view);
            }
        } else if !host.dismissing && !drive.active && player.may_drive() {
            started = start(&time, &collide, &player, host, class);
        }
    }
    if started {
        drive.active = true;
        // Weapons stow on the back, out of the seat's way.
        if let Ok(entity) = body.single() {
            sheath.write(crate::creature_anim::SheathRequest {
                entity,
                state: 0,
                ceremony: false,
            });
        }
        audio.reset();
    }
    let turbo = auras.turbo && !host.turbo_on;
    host.turbo_on = auras.turbo;

    // The driving controls' keys are the kart's while it drives (`bindings`).
    overlay.active = drive.active && host.session.active();
    if !drive.active || !host.session.active() {
        score.clear_live();
        return;
    }
    // A showcase run drives the kart from its start, once we are in it there.
    show.arrive(&mut host.session, &collide, drive.pos, now);
    let showing = show.driving();
    if !showing {
        host.session.pads_off(false);
    }

    // The crate test hook: in a race with weapons, steer for a crate and fire at it.
    let crate_aim = if input::autopilot_on()
        && autotest::crate_test()
        && races.weapons_race().is_some()
        && races.next_target(now).is_some()
    {
        autotest::crate_aim(&mut host.crate_test, &arms, drive.pos, drive.yaw, now)
    } else {
        autotest::CrateAim::default()
    };
    let controls = if input::autopilot_on() {
        // In a race the autopilot drives the course: full gas, steering for the next checkpoint.
        match crate_aim.target.or_else(|| races.next_target(now)) {
            Some(target) => {
                let to = target - drive.pos;
                let want = f32::atan2(-to.x, -to.z);
                let off = (want - drive.yaw + std::f32::consts::PI)
                    .rem_euclid(std::f32::consts::TAU)
                    - std::f32::consts::PI;
                // Stalled against something: back away with the wheel turned, then go again.
                if drive.speed < 3.0 {
                    let since = *host.stalled_since.get_or_insert(now);
                    if now - since > 1.0 && now > host.backing_until {
                        host.backing_until = now + 1.3;
                        host.stalled_since = None;
                    }
                } else {
                    host.stalled_since = None;
                }
                if now < host.backing_until {
                    input::Controls {
                        brake: true,
                        reverse: true,
                        steer: if off > 0.0 { 1.0 } else { -1.0 },
                        ..Default::default()
                    }
                } else {
                    input::Controls {
                        accelerate: true,
                        steer: (-off * 2.5).clamp(-1.0, 1.0),
                        ..Default::default()
                    }
                }
            }
            // Waiting for a race to start (the race test hook): stay on the line.
            None if std::env::var_os("WOW_KART_RACE").is_some() || autotest::active() => {
                input::Controls {
                    brake: true,
                    ..Default::default()
                }
            }
            None => input::autopilot(now, view.meter),
        }
    } else if ui.typing {
        input::Controls::default()
    } else {
        input::read(&bindings::keys(&overlay), &pads)
    };
    // A showcase run's tape drives; the camera binding steps through its cameras.
    let cycle = showing && controls.camera;
    // The free camera flying takes the controls; the kart holds still meanwhile.
    let fly = controls;
    let controls = if showing || free.flying() {
        input::Controls::default()
    } else {
        controls
    };
    if server_kart {
        profession.dpad(controls.dpad);
    }
    // CTR's L2 swaps the chase camera's distance, here the setting, so it persists; its R2 held
    // looks back (`kartcraft::camera`).
    let far = settings.far_camera ^ controls.camera;
    if controls.camera {
        cvars.set("KartFarCamera", if far { "1" } else { "0" });
    }
    host.session.set_camera(far, controls.look_back);
    // The camera window's tuning, live.
    host.session.tune_camera(settings.camera);
    // ○ fires the held item; the autopilot fires now and then while it holds one, and again
    // with its bomb rolling or its shield up.
    let holding = matches!(
        arms.held(),
        kartcraft::weapons::HeldView::Item(_) | kartcraft::weapons::HeldView::Spinning
    ) || arms.fires_again();
    let fire = controls.fire
        || crate_aim.fire
        || (input::autopilot_on()
            && !autotest::crate_test()
            && holding
            && input::autofire(now, time.delta_secs()));
    arms.press(fire, controls.reverse);
    arms.camera = view.camera;
    arms.camera_fov = view.fov;
    // Wumpa from the race's fruit, or all ten under Wumpa Juice.
    let wumpa = if auras.juice { 10 } else { arms.wumpa() };
    extras.wumpa = wumpa;
    // A race's countdown holds the kart on the line, revving with the gas as CTR's does until
    // GO, which gives the revs their start boost (`ctr_kart::rev`); pinned there so a start on
    // a slope does not roll it back.
    let on_grid = races.holding(now);
    host.session.pin(on_grid);
    if let Some(kart) = host.session.kart_mut() {
        if let Some(boost) = kart.hold_on_grid(on_grid) {
            info!("kart race: start boost {boost:?}");
        }
    }
    let mods = Modifiers {
        wumpa,
        mask: auras.aku,
        super_engine: auras.engine,
    };
    if turbo {
        if let Some(kart) = host.session.kart_mut() {
            kart.fire_turbo(auras.juice);
        }
    }
    let dt = time.delta_secs();
    // What each nearby kart rams with, as the race's weapons know it.
    let mut nearby = remotes.nearby(drive.pos, BUMP_RANGE);
    arms.arm(&mut nearby);
    let events = if showing {
        show.step(&mut host.session, &collide, view.ground, now, dt)
    } else {
        host.session.step_with(
            &collide,
            now,
            dt,
            &kartcraft::kart_input(&controls),
            &mods,
            &nearby,
            |world, kart, origin| arms.tick(world, kart, origin),
        )
    };
    arms.blend = host.session.tick_fraction();
    arms.feed(&mut audio);
    for event in &events {
        audio.observe(event);
        rumble.observe(event);
        // A showcase run earns no chain: it is a recording, not driving.
        if !showing {
            host.chain.observe(now, event);
        }
    }
    let Some(kart) = host.session.kart() else {
        return;
    };
    match host.chain.idle(now, kart.state == State::Drifting) {
        Some(Outcome::Banked(line)) => {
            score.bank(now, line.points, &line.label);
            if server_kart {
                profession.landed(f64::from(line.points), &line.label);
            }
        }
        Some(Outcome::Lost(points)) => score.lose(now, points),
        None => {}
    }
    score.live(host.chain.live());
    audio.speed = kart.speed().max(0) as f32 * 30.0 / 256.0;
    audio.drifting = kart.state == State::Drifting;
    audio.grounded = kart.grounded();
    audio.burn(kart.reserves() > 0, dt);
    audio.engine = kart.engine_sound();

    *view = host.session.view(dt);
    audio.skidding = view.skids.0 || view.skids.1;
    audio.ground = view.ground;
    // The scrape, the voices, the engine's class, the mask theme and the pad's shakes.
    if let Some(kart) = host.session.kart() {
        audio.engine_class = host.class;
        audio.mask_theme = kart
            .masked()
            .then(|| kartcraft::weapons::good_guy(kartcraft::model::our_kart()));
        audio.tick(&view.feedback, kart.clocked().is_some());
        rumble.feedback(&view.feedback, kart.speed());
    }
    rumble.off = !settings.rumble;
    // `WOW_KART_DEBUG`: the kart's pose twice a second.
    if std::env::var_os("WOW_KART_DEBUG").is_some()
        && (now * 2.0) as i32 != ((now - dt) * 2.0) as i32
    {
        info!(
            "kart debug: at {:?} grounded {} speed {:.1} state {:?} player {:?}",
            view.transform.translation, view.grounded, view.speed, view.state, player.pos
        );
    }
    // Our kart's state for the players around us (a Kartcraft server relays it).
    if server_kart && now - host.state_sent >= STATE_PERIOD {
        host.state_sent = now;
        profession.send(remote::state_line(&view));
    }
    drive.pos = view.transform.translation;
    drive.yaw = view.yaw;
    drive.speed = view.speed;
    drive.camera = view.camera;
    drive.fov = view.fov;
    // A showcase run is filmed by its own cameras.
    if let Some((camera, fov)) = show.frame(&view, cycle, dt) {
        drive.camera = Some(camera);
        drive.fov = fov;
        drive.engine_camera = true;
    } else {
        // The free camera (`/kartfree`): flown with the right mouse held to look, or parked.
        let look = if buttons.pressed(MouseButton::Right) {
            mouse.delta
        } else {
            Vec2::ZERO
        };
        if let Some((camera, fov)) = free.frame(&view, &fly, look, dt) {
            drive.camera = Some(camera);
            drive.fov = fov;
            drive.engine_camera = true;
        }
    }
}

/// Our character in the kart's seat: the body takes the kart's tilt and sits its chair pose's
/// pelvis on the seat anchor ([`seat::place`]). After the controller, which places the body at
/// the kart's origin facing its heading.
#[allow(clippy::type_complexity)]
fn seat(
    view: Res<KartView>,
    mut fits: ResMut<seat::SeatFits>,
    anim_data: Option<Res<crate::creature_anim::AnimData>>,
    mut body: Query<
        (
            &mut Transform,
            Option<&benilla_assets::ModelAnimations>,
            Option<&benilla_world::rig_anim::RigPose>,
        ),
        With<crate::net::Embodied>,
    >,
) {
    if !view.active {
        return;
    }
    let Ok((mut t, anims, rig)) = body.single_mut() else {
        return;
    };
    let catalog = anim_data.as_deref().map(|d| &d.0);
    let fit = anims
        .zip(rig)
        .and_then(|(anims, rig)| fits.get(anims, rig, catalog));
    (t.translation, t.rotation) = seat::place(
        &view.transform,
        model::seat() + seat_nudge(),
        driver_turn(),
        driver_lean(view.steer),
        fit,
        t.scale.x,
    );
}

/// The driver's facing in the kart: the body's model faces −Z at rest, the kart's +Z.
pub(crate) fn driver_turn() -> Quat {
    Quat::from_rotation_y(std::f32::consts::PI)
}

/// The driver's lean in the seat, as CTR's racers throw their weight into a turn: up to
/// [`LEAN`] radians toward the steer, + left, about the kart's forward axis (its left is +X).
pub(crate) fn driver_lean(steer: f32) -> Quat {
    Quat::from_rotation_z(-steer.clamp(-1.0, 1.0) * LEAN)
}

/// The most a driver leans into a turn, radians.
const LEAN: f32 = 0.2;

/// `WOW_KART_SEAT=x,y,z` (meters, the kart's frame) moves the seat anchor our driver's pelvis
/// rests on, to try a seat by eye.
fn seat_nudge() -> Vec3 {
    static NUDGE: std::sync::OnceLock<Vec3> = std::sync::OnceLock::new();
    *NUDGE.get_or_init(|| {
        std::env::var("WOW_KART_SEAT")
            .ok()
            .and_then(|v| {
                let c: Vec<f32> = v.split(',').filter_map(|p| p.trim().parse().ok()).collect();
                (c.len() == 3).then(|| Vec3::new(c[0], c[1], c[2]))
            })
            .unwrap_or(Vec3::ZERO)
    })
}

/// How much bigger than its physics the kart is drawn: a WoW character is taller than the CTR
/// racer the kart was built for.
const MODEL_SIZE: f32 = 1.25;

pub(crate) fn plugin(app: &mut App) {
    // A WoW character in the kart, and WoW's action bars along the bottom of the screen (the
    // chase camera's aim over them is its tuning's, `settings::KartSettings::camera`).
    kartcraft::set_framing(kartcraft::Framing {
        model_size: MODEL_SIZE,
        hud_bottom_inset: 0.075,
        // A WoW character's head stands above a CTR racer's in the seat.
        head_lift: 0.5,
    });
    autotest::plugin(app);
    marshals::plugin(app);
    bindings::plugin(app);
    app.init_resource::<Host>()
        .init_resource::<KartDrive>()
        .init_resource::<KartView>()
        .init_resource::<model::KartModel>()
        .init_resource::<sound::KartAudio>()
        .init_resource::<kartcraft::rumble::Rumble>()
        .init_resource::<score::KartScore>()
        .init_resource::<remote::RemoteKarts>()
        .init_resource::<seat::SeatFits>()
        .init_resource::<kartcraft::hud::KartHud>()
        .init_resource::<kartcraft::hud::HudExtras>()
        .init_resource::<kartcraft::effects::KartEffects>()
        .init_resource::<kartcraft::pads::PadField>()
        .init_resource::<kartcraft::weapons::Weapons>()
        .init_resource::<kartcraft::weapons::look::WeaponLook>()
        .init_resource::<weapons::WeaponLink>()
        .net_handler(
            benilla_protocol::SessionEventKind::XpGain,
            score::on_xp_gain,
        )
        .net_handler(benilla_protocol::SessionEventKind::Chat, remote::on_chat)
        .net_handler(benilla_protocol::SessionEventKind::Chat, race::on_chat)
        .net_handler(benilla_protocol::SessionEventKind::Chat, weapons::on_chat)
        .net_handler(benilla_protocol::SessionEventKind::Chat, showcase::on_chat)
        .init_resource::<showcase::Showcase>()
        .init_resource::<freecam::FreeCam>()
        .init_resource::<kartcraft::hud::HudHidden>()
        .init_resource::<race::Races>()
        .init_resource::<race::RaceText>()
        .init_resource::<ghost::Ghosts>()
        .init_resource::<ui::KartOverlay>()
        // Before `CvarLoad`'s boot fold, which fires their observer.
        .init_resource::<race::GuideHud>()
        .init_resource::<settings::KartSettings>()
        .add_observer(settings::on_cvar)
        .add_systems(
            Startup,
            (kartcraft::hud::spawn, |_: Commands| {
                model::preload();
                kartcraft::weapons::look::preload();
            }),
        )
        .add_systems(
            Update,
            (
                input::log_pads,
                race_name,
                pads::feed,
                showcase::hook,
                // The ground under the pads is read through the mover's trace, after its doors.
                kartcraft::pads::lay.after(crate::go_anim::publish_ghost_door_exclusions),
                update.in_set(KartDriveSet),
                race::update,
                showcase::banner,
                sound::overtakes,
                ghost::update,
                weapons::update,
                race::beams,
                race::guide,
                model::update,
                sound::update,
                kartcraft::rumble::send,
                score::track_skill,
                ui::publish,
                // Into the VM only once the interface is up, or the first frame's events are lost.
                crate::ui_kart::feed.run_if(crate::ui_script::ingame_ui_up),
                kartcraft::hud::draw,
            )
                .chain()
                .in_set(WorldStage::Input)
                .before(crate::player::PlayerControlSet)
                .in_set(crate::char_select::InWorldGated),
        )
        .add_systems(
            PostUpdate,
            (
                // Off `Update`, where the terrain streamer's writers run unordered: the ground
                // under the kart as drawn this frame, for the next frame's ticks.
                ground::sample,
                remote::update,
                model::animate_wheels,
                kartcraft::effects::attach,
                kartcraft::effects::tick,
                kartcraft::effects::draw,
                kartcraft::weapons::look::draw,
                seat,
                sound::remote_engines,
            )
                .chain()
                .before(bevy::transform::TransformSystems::Propagate)
                .after(crate::entities::conform_units),
        );
}

/// Whether a unit drives a kart: it wears Summon Kart (on a Kartcraft server a mount aura, so
/// its mount field names the stock mechanostrider every client knows).
pub(crate) fn drives_kart(store: &crate::net::ObjectStore) -> bool {
    store
        .0
        .unit_auras()
        .any(|a| a.spell_id == benilla_formats::kartcraft::SPELL_SUMMON_KART)
}

/// The mount display to build a model for: none for a kart driver, whose kart is drawn instead
/// (`remote` and `kartcraft::model`); the seat pose still follows the real field.
pub(crate) fn mount_model_display(store: &crate::net::ObjectStore) -> u32 {
    if drives_kart(store) {
        0
    } else {
        store.0.unit_mount_display_id()
    }
}

/// Our character's name, for the race rosters: the one logged in now, a relog onto another
/// character included, or no roster naming it is taken for ours.
fn race_name(
    guid: Res<crate::net::SelfGuid>,
    names: Option<Res<crate::names::NameCache>>,
    mut races: ResMut<race::Races>,
) {
    if let (Some(g), Some(names)) = (guid.0, names) {
        if let Some(name) = names.peek(g) {
            if races.me != name {
                races.me = name.to_string();
            }
        }
    }
}

#[cfg(test)]
mod race_name_tests {
    use super::*;

    #[test]
    fn a_relog_onto_another_character_races_under_its_name() {
        let mut app = App::new();
        let mut names = crate::names::NameCache::default();
        names.insert_player(1, "Vozaccamezu".into(), None);
        names.insert_player(2, "Asdasd".into(), None);
        app.insert_resource(names)
            .insert_resource(crate::net::SelfGuid(Some(1)))
            .init_resource::<race::Races>()
            .add_systems(Update, race_name);
        app.update();
        assert_eq!(app.world().resource::<race::Races>().me, "Vozaccamezu");
        // Back to character select and in as another: the rosters name the new one.
        app.world_mut().resource_mut::<crate::net::SelfGuid>().0 = Some(2);
        app.update();
        assert_eq!(app.world().resource::<race::Races>().me, "Asdasd");
    }
}
