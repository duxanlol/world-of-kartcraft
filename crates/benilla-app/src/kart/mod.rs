//! Kart mode (World of Kartcraft): `J` seats our character in a Crash Team Racing kart driven by
//! CTR's own driving model (`ctr-kart`, ported from the decompilation) through a
//! [`kartcraft::Session`]. While we drive, the kart's pose is our position, facing and camera
//! (read by `player::controller`), our character holds the saddle pose in the kart's seat
//! ([`seat`]) and the kart is drawn under it (`kartcraft::model`). Banked turbo chains feed the
//! Karting profession ([`profession`]) and the score on screen ([`score`]). In a race CTR's
//! weapons are live ([`weapons`]): crates, items, shots and hits through the server.

mod profession;
mod race;
mod remote;
mod score;
mod sound;
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
    /// Drive with the chase camera rather than our follow camera (`K` toggles).
    pub(crate) engine_camera: bool,
    /// The gas is down (a race's countdown holds the kart but times the start boost by it).
    pub(crate) revving: bool,
}

impl KartDrive {
    pub(crate) fn moving(&self) -> bool {
        self.speed > MOVING_SPEED
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
    /// When our kart's state was last relayed to the players around us.
    state_sent: f32,
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
) -> bool {
    let class = kartcraft::class();
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
            true
        }
        Err(e) => {
            warn!("kart: {e}");
            false
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn update(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
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
    (remotes, mut extras, races, mut arms): (
        Res<remote::RemoteKarts>,
        ResMut<kartcraft::hud::HudExtras>,
        Res<race::Races>,
        ResMut<kartcraft::weapons::Weapons>,
    ),
) {
    let now = time.elapsed_secs();
    let host = &mut *host;
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
    if keys.just_pressed(KeyCode::KeyK) && !ui.typing && drive.active {
        drive.engine_camera = !drive.engine_camera;
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
    if (keys.just_pressed(KeyCode::KeyJ) && !ui.typing) || autostart {
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
            started = start(&time, &collide, &player, host);
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
            started = start(&time, &collide, &player, host);
        }
    }
    if started {
        drive.active = true;
        // `WOW_KART_CAM=wow` starts on our own follow camera; the chase camera is the default.
        drive.engine_camera = std::env::var("WOW_KART_CAM").map_or(true, |v| v != "wow");
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

    if !drive.active || !host.session.active() {
        score.clear_live();
        return;
    }

    let controls = if input::autopilot_on() {
        // In a race the autopilot drives the course: full gas, steering for the next checkpoint.
        match races.next_target(now) {
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
            None if std::env::var_os("WOW_KART_RACE").is_some() => input::Controls::default(),
            None => input::autopilot(now, view.meter),
        }
    } else if ui.typing {
        input::Controls::default()
    } else {
        input::read(&keys, &pads)
    };
    if server_kart {
        profession.dpad(controls.dpad);
    }
    // ○ fires the held item; the autopilot fires now and then while it holds one.
    let holding = matches!(
        arms.held(),
        kartcraft::weapons::HeldView::Item(_) | kartcraft::weapons::HeldView::Spinning
    );
    let fire = controls.fire
        || (input::autopilot_on() && holding && input::autofire(now, time.delta_secs()));
    arms.press(fire, controls.reverse);
    arms.camera = view.camera;
    // Wumpa from the race's fruit, or all ten under Wumpa Juice.
    let wumpa = if auras.juice { 10 } else { arms.wumpa() };
    extras.wumpa = wumpa;
    drive.revving = controls.accelerate;
    // A race's countdown holds the kart on the line (the gas only revs it).
    let controls = if races.holding(now) {
        input::Controls {
            brake: true,
            ..Default::default()
        }
    } else {
        controls
    };
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
    if let Some(big) = race::START_BOOST.lock().unwrap().take() {
        if let Some(kart) = host.session.kart_mut() {
            kart.start_boost(big);
        }
    }
    let dt = time.delta_secs();
    let events = host.session.step_with(
        &collide,
        now,
        dt,
        &kartcraft::kart_input(&controls),
        &mods,
        &remotes.nearby(drive.pos, BUMP_RANGE),
        |world, kart, origin| arms.tick(world, kart, origin),
    );
    arms.blend = host.session.tick_fraction();
    arms.feed(&mut audio);
    for event in &events {
        audio.observe(event);
        host.chain.observe(now, event);
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
    audio.turbo = kart.reserves() > 0;
    audio.engine = kart.engine_sound();

    *view = host.session.view(dt);
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
}

/// Our character in the kart's seat: the body takes the kart's tilt and sits at its seat anchor.
/// After the controller, which places the body at the kart's origin facing its heading.
fn seat(view: Res<KartView>, mut body: Query<&mut Transform, With<crate::net::Embodied>>) {
    if !view.active {
        return;
    }
    let Ok(mut t) = body.single_mut() else {
        return;
    };
    let kart = view.transform;
    t.translation = kart.transform_point(model::seat() + seat_nudge());
    t.rotation = kart.rotation * driver_lean(view.steer);
}

/// The driver's lean in the seat, as CTR's racers throw their weight into a turn: up to
/// [`LEAN`] radians toward the steer, + left. The body's model faces −Z at rest, the kart's +Z.
pub(crate) fn driver_lean(steer: f32) -> Quat {
    // About the kart's forward axis, toward the inside of the turn (its left is +X).
    Quat::from_rotation_z(-steer.clamp(-1.0, 1.0) * LEAN)
        * Quat::from_rotation_y(std::f32::consts::PI)
}

/// The most a driver leans into a turn, radians.
const LEAN: f32 = 0.2;

/// `WOW_KART_SEAT=x,y,z` (meters, the kart's frame) moves the driver off CTR's seat anchor, for
/// races whose saddle pose sits higher or lower.
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

pub(crate) fn plugin(app: &mut App) {
    // A WoW character in the kart, and WoW's action bars along the bottom of the screen.
    kartcraft::set_framing(kartcraft::Framing {
        model_size: 1.25,
        look_drop: 70.0,
        hud_bottom_inset: 0.075,
        // A WoW character's head stands above a CTR racer's in the seat.
        head_lift: 0.5,
    });
    app.init_resource::<Host>()
        .init_resource::<KartDrive>()
        .init_resource::<KartView>()
        .init_resource::<model::KartModel>()
        .init_resource::<sound::KartAudio>()
        .init_resource::<score::KartScore>()
        .init_resource::<remote::RemoteKarts>()
        .init_resource::<kartcraft::hud::KartHud>()
        .init_resource::<kartcraft::hud::HudExtras>()
        .init_resource::<kartcraft::effects::KartEffects>()
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
        .init_resource::<race::Races>()
        .init_resource::<race::RaceText>()
        .add_systems(
            Startup,
            (
                score::spawn,
                kartcraft::hud::spawn,
                race::spawn_text,
                |_: Commands| {
                    model::preload();
                    kartcraft::weapons::look::preload();
                },
            ),
        )
        .add_systems(
            Update,
            (
                input::log_pads,
                race_name,
                update,
                race::update,
                weapons::update,
                race::beams,
                race::draw_text,
                model::update,
                sound::update,
                score::draw,
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

/// Our character's name, for the race rosters.
fn race_name(
    guid: Res<crate::net::SelfGuid>,
    names: Option<Res<crate::names::NameCache>>,
    mut races: ResMut<race::Races>,
) {
    if races.me.is_empty() {
        if let (Some(g), Some(names)) = (guid.0, names) {
            if let Some(name) = names.peek(g) {
                races.me = name.to_string();
            }
        }
    }
}
