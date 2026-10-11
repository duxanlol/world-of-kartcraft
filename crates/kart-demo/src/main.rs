//! `kartcraft-demo`: Crash Team Racing's kart over the WoW world with no server, login or
//! character: benilla's world viewer with its free-fly camera swapped for the kart and its chase
//! camera. It opens at Northshire Abbey, as the viewer does (`WOW_WORLDVIEW_AT=x,y` moves it).
//!
//! Controls: a PS5/PS4 or Xbox pad (✕/A gas, □/X brake, R1/RB hop and powerslide, L1/LB boost, stick
//! steer), or the
//! keyboard (W/↑ gas, S/↓ brake and reverse, A/D or ←/→ steer, Space hop and powerslide, Shift
//! boost). In a powerslide the turbo meter by the speedometer fills; press the other shoulder (Shift) when
//! it turns red for a turbo, up to three per slide. R puts the kart back at the start, K swaps the
//! chase camera for a far one, C (Select) swaps CTR's near and far chase cameras, X (R3) held looks
//! back, 1–4 toggle Wumpa Juice, fire a Turbo, toggle Super Engine and
//! toggle Aku Aku.
//!
//! A weapons sandbox stands around the start: a ring of "?" crates, fruit crates and Wumpa fruit
//! with a row of "?" crates ahead, and two dummy karts to shoot at. E (○/B) fires the held item,
//! backwards with S/↓ (the stick back) held. A local [`Referee`] rules as the Kartcraft server
//! does, so the weapons run as they do online. `WOW_KART_WEAPON_TEST=<item id>` hands that item
//! over and over and fires it, for unattended looks.

mod audio;

use benilla_assets::coords::bevy_to_wow;
use benilla_world::build_id::BuildId;
use benilla_world::collision::WorldCollision;
use bevy::camera::{PerspectiveProjection, Projection};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::input::gamepad::Gamepad;
use bevy::prelude::*;
use bevy::render::view::Hdr;
use kartcraft::chain::{Chain, Outcome};
use kartcraft::model::{KartSpawner, KartStatus, KartWheels};
use kartcraft::sound::{KartAudio, Sounds};
use kartcraft::weapons::referee::{Addressed, Referee};
use kartcraft::weapons::wire::{CrateKind, CrateSpot};
use kartcraft::weapons::{HeldView, Item, Weapons};
use kartcraft::{effects, hud, input, model, weapons, Event, KartView, Modifiers, Session, State};

/// Our racer in the sandbox, and the dummies: id, racer, and where they stand off the start
/// (yards right, yards ahead).
const ME: u32 = 1;
const DUMMIES: [(u32, &str, f32, f32); 2] = [(2, "cortex", -2.0, 16.0), (3, "tiny", 6.0, 26.0)];
/// The crate ring round the start (yards, spots), and the row of four "?" crates 4 yards apart
/// across the way off to the left ahead, as the server lays a row.
const RING: f32 = 34.0;
const RING_SPOTS: usize = 24;
const ROW_AHEAD: f32 = 10.0;

fn main() -> benilla_world::AppExit {
    benilla_world::worldview::run_with(
        BuildId {
            version: env!("CARGO_PKG_VERSION"),
            describe: env!("BENILLA_GIT_DESCRIBE"),
            sha: env!("BENILLA_GIT_SHA"),
            short: env!("BENILLA_GIT_SHORT"),
            date: env!("BENILLA_GIT_DATE"),
            profile: env!("BENILLA_PROFILE"),
            project_dir: env!("BENILLA_PROJECT_DIR"),
            ..Default::default()
        },
        "World of Kartcraft: kart demo",
        scene,
    )
}

/// Where the kart starts, raw WoW coords: `WOW_WORLDVIEW_AT=x,y`, else the road out of
/// Northshire Abbey.
fn start_xy() -> (f32, f32) {
    std::env::var("WOW_WORLDVIEW_AT")
        .ok()
        .and_then(|v| {
            let c: Vec<f32> = v.split(',').filter_map(|p| p.trim().parse().ok()).collect();
            (c.len() >= 2).then(|| (c[0], c[1]))
        })
        .unwrap_or(benilla_world::terrain_stream::SPAWN_XY)
}

/// Facing at the start, Bevy yaw: `WOW_KART_YAW` in degrees.
fn start_yaw() -> f32 {
    std::env::var("WOW_KART_YAW")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .unwrap_or(0.0)
        .to_radians()
}

#[derive(Component)]
struct DemoCam;

#[derive(Component)]
struct Hud;

/// Chain text: in progress, banked or lost.
#[derive(Component)]
struct ChainText;

#[derive(Resource, Default)]
struct Demo {
    session: Session,
    chain: Chain,
    /// The start, Bevy world, once the ground under it has streamed in.
    start: Option<Vec3>,
    far_camera: bool,
    /// CTR's far zoom table for the chase camera (C, Select).
    ctr_far: bool,
    juice: bool,
    engine: bool,
    aku: bool,
    /// The last chain shown: its text, colour and when.
    shown: Option<(String, Color, f32)>,
    best: f32,
    /// `WOW_KART_AUTOPILOT`: a scripted drive for unattended runs.
    autopilot: bool,
    /// The sandbox's referee, and the dummies: id, racer, origin (Bevy world) and yaw.
    referee: Option<Referee>,
    dummies: Vec<Dummy>,
}

/// `WOW_KART_WEAPON_TEST=<CTR item id>`: the item handed over and over.
fn weapon_test() -> Option<Item> {
    static TEST: std::sync::OnceLock<Option<Item>> = std::sync::OnceLock::new();
    *TEST.get_or_init(|| {
        let id: u8 = std::env::var("WOW_KART_WEAPON_TEST").ok()?.parse().ok()?;
        Item::from_id(id).filter(|i| *i != Item::None)
    })
}

/// The ground under `p` (Bevy world), else `p`.
fn ground(collide: &WorldCollision, p: Vec3) -> Vec3 {
    collide
        .ray_body(Vec3::new(p.x, p.y + 60.0, p.z), Dir3::NEG_Y, 160.0)
        .map_or(p, |hit| Vec3::new(p.x, p.y + 60.0 - hit.distance, p.z))
}

/// A dummy: id, racer, origin (Bevy world) and yaw.
type Dummy = (u32, &'static str, Vec3, f32);

/// The sandbox round the start `at` facing `yaw`: a referee over its crates, their layout, and
/// the dummies.
fn sandbox(collide: &WorldCollision, at: Vec3, yaw: f32) -> (Referee, Vec<Addressed>, Vec<Dummy>) {
    let ahead = Vec3::new(-yaw.sin(), 0.0, -yaw.cos());
    let right = Vec3::new(-ahead.z, 0.0, ahead.x);
    let mut spots = Vec::new();
    let mut spot = |kind, p: Vec3| {
        spots.push(CrateSpot {
            kind,
            at: bevy_to_wow(ground(collide, p)),
        });
    };
    // The row stands off to the left, leaving the line to the dummies clear.
    for i in 0..4 {
        spot(
            CrateKind::Weapon,
            at + ahead * ROW_AHEAD + right * (i as f32 * 4.0 - 22.0),
        );
    }
    for i in 0..RING_SPOTS {
        let a = (i as f32 + 0.5) / RING_SPOTS as f32 * std::f32::consts::TAU;
        let kind = [CrateKind::Weapon, CrateKind::Fruit, CrateKind::Wumpa][i % 3];
        spot(kind, at + (ahead * a.cos() + right * a.sin()) * RING);
    }
    let (referee, layout) = Referee::new(spots, 100);
    let dummies = DUMMIES
        .iter()
        .map(|&(id, racer, r, f)| (id, racer, ground(collide, at + right * r + ahead * f), yaw))
        .collect();
    (referee, layout, dummies)
}

/// The referee's lines for us (or for everyone) into the weapons.
fn deliver(weapons: &mut Weapons, lines: Vec<Addressed>) {
    for (to, down) in lines {
        if to.is_none_or(|to| to == ME) {
            weapons.receive(down);
        }
    }
}

/// The dummies' karts: spawned once, placed each frame with their hits' look.
#[derive(Resource, Default)]
struct DummyKarts(Vec<(u32, Entity)>);

fn dummies(
    demo: Res<Demo>,
    weapons: Res<Weapons>,
    mut karts: ResMut<DummyKarts>,
    mut spawner: KartSpawner,
    mut parts: Query<(
        &mut Transform,
        &mut Visibility,
        &mut KartWheels,
        &mut KartStatus,
    )>,
) {
    for &(id, racer, _, _) in &demo.dummies {
        if !karts.0.iter().any(|k| k.0 == id) {
            if let Some(e) = spawner.spawn(racer) {
                karts.0.push((id, e));
            }
        }
    }
    for &(id, kart) in &karts.0 {
        let Ok((mut t, mut vis, mut wheels, mut status)) = parts.get_mut(kart) else {
            continue;
        };
        let Some(&(_, _, at, yaw)) = demo.dummies.iter().find(|d| d.0 == id) else {
            *vis = Visibility::Hidden;
            continue;
        };
        let rotation = Quat::from_rotation_y(yaw + std::f32::consts::PI);
        let mut kart = Transform {
            translation: at,
            rotation,
            scale: Vec3::splat(kartcraft::model_scale()),
        };
        if let Some(r) = weapons.reaction(id) {
            kart.translation += rotation * r.offset;
            kart.rotation = rotation * r.rotation;
            kart.scale *= r.scale.max(Vec3::splat(0.01));
        }
        *t = kart;
        *vis = Visibility::Visible;
        *wheels = KartWheels::default();
        *status = KartStatus {
            grounded: true,
            ..Default::default()
        };
    }
}

fn scene(app: &mut App) {
    app.init_resource::<Demo>()
        .init_resource::<Weapons>()
        .init_resource::<weapons::look::WeaponLook>()
        .init_resource::<DummyKarts>()
        .init_resource::<KartView>()
        .init_resource::<KartAudio>()
        .init_resource::<effects::KartEffects>()
        .init_resource::<hud::KartHud>()
        .init_resource::<hud::HudExtras>()
        .init_resource::<model::KartModel>()
        .add_systems(
            Startup,
            (spawn_camera, spawn_hud, hud::spawn).after(benilla_assets::AssetSet::Open),
        )
        .add_systems(
            Startup,
            |mut demo: ResMut<Demo>, mut weapons: ResMut<Weapons>| {
                demo.autopilot = std::env::var_os("WOW_KART_AUTOPILOT").is_some();
                demo.far_camera = std::env::var("WOW_KART_CAM").is_ok_and(|v| v == "far");
                weapons.me = ME;
                model::preload();
                weapons::look::preload();
            },
        )
        .add_systems(
            Update,
            (
                input::log_pads,
                drive,
                model::update,
                dummies,
                model::animate_wheels,
                effects::attach,
                effects::tick,
                effects::draw,
                weapons::look::draw,
                hud,
                hud::draw,
                sound,
            )
                .chain()
                .in_set(benilla_world::schedule::WorldStage::Input),
        );
    if std::env::var("WOW_WORLDVIEW_SHOT").is_ok() {
        app.add_systems(Update, shoot_and_exit);
    }
}

fn spawn_camera(mut commands: Commands, msaa: Res<benilla_world::view::MsaaSetting>) {
    let (x, y) = start_xy();
    let start = benilla_assets::coords::wow_to_bevy([x, y, 100.0]);
    commands.spawn((
        Camera3d::default(),
        benilla_world::view::WorldCamera,
        msaa.level(),
        Projection::from(PerspectiveProjection {
            far: benilla_world::view::CAM_FAR,
            near: benilla_world::view::NEARCLIP_DEFAULT,
            fov: kartcraft::camera::FOV_Y,
            ..default()
        }),
        Hdr,
        Tonemapping::None,
        benilla_world::ffx_glow::FfxGlow::WORLD,
        Transform::from_translation(start + Vec3::new(0.0, 20.0, 20.0)).looking_at(start, Vec3::Y),
        DemoCam,
    ));
}

fn spawn_hud(mut commands: Commands) {
    let text = |size: f32| {
        (
            Text::new(String::new()),
            TextFont {
                font_size: size,
                ..default()
            },
            TextColor(Color::WHITE),
            TextShadow::default(),
        )
    };
    commands.spawn((
        Hud,
        text(16.0),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(14.0),
            top: Val::Px(10.0),
            ..default()
        },
    ));
    commands.spawn((
        ChainText,
        text(34.0),
        TextLayout::new_with_justify(Justify::Center),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Percent(9.0),
            width: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            ..default()
        },
    ));
}

#[allow(clippy::too_many_arguments)]
fn drive(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    collide: WorldCollision,
    mut demo: ResMut<Demo>,
    mut view: ResMut<KartView>,
    mut focus: ResMut<benilla_world::terrain_stream::ViewFocus>,
    mut audio: ResMut<KartAudio>,
    mut extras: ResMut<hud::HudExtras>,
    mut camera: Query<&mut Transform, With<DemoCam>>,
    mut weapons: ResMut<Weapons>,
) {
    let now = time.elapsed_secs();
    let dt = time.delta_secs();
    let demo = &mut *demo;
    let weapons = &mut *weapons;

    if !demo.session.active() || keys.just_pressed(KeyCode::KeyR) {
        // Wait for the ground at the start to stream in, then put the kart on it.
        let (x, y) = start_xy();
        let column = benilla_assets::coords::wow_to_bevy([x, y, 0.0]);
        let ground = collide
            .ray_body(Vec3::new(column.x, 2000.0, column.z), Dir3::NEG_Y, 4000.0)
            .map(|hit| Vec3::new(column.x, 2000.0 - hit.distance, column.z));
        let Some(ground) = ground.or(demo.start) else {
            return;
        };
        demo.start = Some(ground);
        match demo
            .session
            .start(&collide, ground, start_yaw(), kartcraft::class(), now)
        {
            Ok(faces) => info!(
                "kart demo: a {} kart on {faces} faces at {ground:?}",
                kartcraft::class().name()
            ),
            Err(e) => {
                error!("kart demo: {e}");
                return;
            }
        }
        demo.chain = Chain::default();
        // The sandbox round the start.
        let (referee, layout, dummies) = sandbox(&collide, ground, start_yaw());
        weapons.clear();
        demo.referee = Some(referee);
        demo.dummies = dummies;
        if let Some(r) = demo.referee.as_mut() {
            r.order = demo.dummies.iter().map(|d| d.0).chain([ME]).collect();
            r.force = weapon_test();
        }
        deliver(weapons, layout);
    }

    let mut controls = if demo.autopilot {
        input::autopilot(now, view.meter)
    } else if weapon_test().is_some() {
        // Unattended: the kart stands at the start, facing the dummies, whatever pad is about.
        input::Controls::default()
    } else {
        input::read(&input::Keys::from_keyboard(&keys), &pads)
    };
    // The autopilot fires now and then while it holds an item (a shield or a bomb it keeps).
    let holding = matches!(weapons.held(), HeldView::Item(_) | HeldView::Spinning);
    if (demo.autopilot || weapon_test().is_some()) && holding && input::autofire(now, dt) {
        controls.fire = true;
    }
    weapons.press(controls.fire, controls.reverse);
    weapons.camera = view.camera;
    // The dummies, and the referee's own frame: crates back, and the weapon test's item.
    weapons.begin_others();
    for (i, &(id, racer, at, yaw)) in demo.dummies.iter().enumerate() {
        weapons.other(id, at, yaw, i as u8, racer, true);
    }
    weapons.end_others();
    weapons.rank = demo.dummies.len() as u8;
    if let Some(referee) = demo.referee.as_mut() {
        let mut lines = referee.tick(now);
        if let Some(item) = weapon_test() {
            let idle = weapons.held() == HeldView::Empty && weapons.our_look().shield.is_none();
            if idle && !referee.holds(ME) {
                lines.extend(referee.grant(ME, item, now));
            }
        }
        deliver(weapons, lines);
    }
    // The autopilot fires a turbo item early in each lap of its loop.
    let cycle = now % 8.0;
    let auto_turbo = demo.autopilot && cycle >= 1.0 && cycle - dt < 1.0;
    let fire_turbo = controls.dpad[1] || keys.just_pressed(KeyCode::Digit2) || auto_turbo;
    if controls.dpad[0] {
        demo.juice = !demo.juice;
    }
    if controls.dpad[2] {
        demo.engine = !demo.engine;
    }
    if controls.dpad[3] {
        demo.aku = !demo.aku;
    }
    if controls.camera {
        demo.ctr_far = !demo.ctr_far;
    }
    demo.session.set_camera(demo.ctr_far, controls.look_back);
    // Kartcraft's camera in CTR's own frame: CTR's frustum (the demo's projection) and its aim.
    demo.session.tune_camera(kartcraft::camera::CameraTuning {
        fov: kartcraft::camera::FOV_Y.to_degrees(),
        aim_drop: 0.0,
        ..Default::default()
    });
    if keys.just_pressed(KeyCode::KeyK) {
        demo.far_camera = !demo.far_camera;
    }
    let wumpa = if demo.juice { 10 } else { weapons.wumpa() };
    extras.wumpa = wumpa;
    extras.item = weapons.hud_item();
    let mods = Modifiers {
        wumpa,
        mask: demo.aku,
        super_engine: demo.engine,
    };
    if fire_turbo {
        if let Some(kart) = demo.session.kart_mut() {
            kart.fire_turbo(demo.juice);
        }
    }
    let events = demo.session.step_with(
        &collide,
        now,
        dt,
        &kartcraft::kart_input(&controls),
        &mods,
        &[],
        |world, kart, origin| weapons.tick(world, kart, origin),
    );
    weapons.blend = demo.session.tick_fraction();
    // Our lines (and the dummies') to the referee, its answers back.
    if let Some(referee) = demo.referee.as_mut() {
        let lines: Vec<Addressed> = std::mem::take(&mut weapons.outbox)
            .into_iter()
            .flat_map(|(from, up)| {
                let at = demo
                    .dummies
                    .iter()
                    .find(|d| d.0 == from)
                    .map_or(view.transform.translation, |d| d.2);
                referee.handle(from, up, bevy_to_wow(at), now)
            })
            .collect();
        deliver(weapons, lines);
    }
    weapons.feed(&mut audio);
    for e in &events {
        demo.chain.observe(now, e);
        audio.observe(e);
        if !matches!(e, Event::Land { .. }) {
            debug!("kart demo: {e:?}");
        }
    }
    let drifting = demo
        .session
        .kart()
        .is_some_and(|k| k.state == State::Drifting);
    match demo.chain.idle(now, drifting) {
        Some(Outcome::Banked(line)) => {
            info!("kart demo: banked {} for {}", line.label, line.points);
            demo.best = demo.best.max(line.points);
            demo.shown = Some((
                format!("{}\n+{}", line.label, line.points),
                Color::srgb(0.35, 1.0, 0.35),
                now,
            ));
        }
        Some(Outcome::Lost(points)) => {
            demo.shown = Some((
                format!("CRASH!\n{points}"),
                Color::srgb(1.0, 0.25, 0.2),
                now,
            ));
        }
        None => {}
    }

    if let Some(kart) = demo.session.kart() {
        audio.speed = kart.speed().max(0) as f32 * 30.0 / 256.0;
        audio.drifting = kart.state == State::Drifting;
        audio.grounded = kart.grounded();
        audio.turbo = kart.reserves() > 0;
        audio.engine = kart.engine_sound();
    }
    *view = demo.session.view(dt);
    // `WOW_KART_FX_TEST=<size>`: a flame of that size burns throughout, to look at.
    if let Some(size) = std::env::var("WOW_KART_FX_TEST")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        view.fire = size;
    }
    if !view.active {
        return;
    }
    focus.set_if_neq_body(bevy_to_wow(view.transform.translation));
    if let (Ok(mut cam), Some(chase)) = (camera.single_mut(), view.camera) {
        *cam = if demo.far_camera {
            let at = view.transform.translation;
            let back = view.transform.rotation * Vec3::NEG_Z;
            Transform::from_translation(at + back * 18.0 + Vec3::Y * 9.0).looking_at(at, Vec3::Y)
        } else {
            chase
        };
    }
}

trait FocusExt {
    fn set_if_neq_body(&mut self, wow: [f32; 3]);
}

impl FocusExt for ResMut<'_, benilla_world::terrain_stream::ViewFocus> {
    fn set_if_neq_body(&mut self, wow: [f32; 3]) {
        **self = benilla_world::terrain_stream::ViewFocus::body(wow, true);
    }
}

#[allow(clippy::type_complexity)]
fn hud(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mut help: Local<Option<bool>>,
    demo: Res<Demo>,
    view: Res<KartView>,
    mut texts: ParamSet<(
        Query<&mut Text, With<Hud>>,
        Query<(&mut Text, &mut TextColor), With<ChainText>>,
    )>,
) {
    let now = time.elapsed_secs();
    // The help shows for the first 20 seconds, then H toggles it.
    let shown = help.get_or_insert(true);
    if keys.just_pressed(KeyCode::KeyH) {
        *shown = !*shown;
    } else if *shown && now > 20.0 && now - time.delta_secs() <= 20.0 {
        *shown = false;
    }
    let shown = *shown;
    if let Ok(mut text) = texts.p0().single_mut() {
        if !shown {
            text.0 = format!(
                "{} class, best chain {}   (H: help)",
                kartcraft::class().name(),
                demo.best
            );
        } else {
            let state = match view.state {
                State::Normal if !view.grounded => "airborne",
                State::Normal => "driving",
                State::Drifting => "POWERSLIDE",
                State::Crashing(_) => "crashed",
                State::Spinning(_) => "spun out",
            };
            let on = |b: bool| if b { "on" } else { "off" };
            text.0 = format!(
                "World of Kartcraft: kart demo ({} class)\n\
             {:>5.1} yd/s   {:>5.0} km/h   {state}\n\
             turbo {:.1} s   best chain {}\n\
             Wumpa Juice {}   Super Engine {}   Aku Aku {}\n\n\
             W/S gas, brake/reverse   A/D steer\n\
             Space hop, hold to powerslide   Shift boost (meter red)\n\
             R restart   K camera   1 Juice  2 Turbo  3 Engine  4 Aku Aku\n\
             E fire the item (hold S: backwards; again: detonate the bomb, fire the shield)\n\
             ? crates give an item, fruit crates and fruit Wumpa; two dummy karts stand ahead\n\
             Pad (PS5 or Xbox): Cross/A gas, Square/X brake, R1 slide, L1 boost,
             Circle/B fire, D-pad: up Juice, right Turbo, down Engine, left Aku Aku
             H hides this",
                kartcraft::class().name(),
                view.speed,
                view.speed * kartcraft::METERS_PER_YARD * 3.6,
                view.reserves,
                demo.best,
                on(demo.juice),
                on(demo.engine),
                on(demo.aku),
            );
        }
    }
    if let Ok((mut text, mut color)) = texts.p1().single_mut() {
        let live = demo.chain.live();
        let (value, tint, alpha) = match (&live, &demo.shown) {
            (Some(line), _) => (
                format!("{}\n{}", line.label, line.points),
                Color::srgb(1.0, 0.82, 0.0),
                1.0,
            ),
            (None, Some((s, c, at))) if now - at < 3.0 => {
                (s.clone(), *c, (3.0 - (now - at)).clamp(0.0, 1.0))
            }
            _ => (String::new(), Color::WHITE, 0.0),
        };
        if text.0 != value {
            text.0 = value;
        }
        color.0 = tint.with_alpha(alpha);
    }
}

/// `WOW_WORLDVIEW_SHOT=<png>` at `WOW_WORLDVIEW_SHOT_AT` seconds (default 20): one frame, then
/// exit, for unattended checks.
fn shoot_and_exit(
    time: Res<Time>,
    mut commands: Commands,
    mut fired_at: Local<Option<f32>>,
    mut exit: MessageWriter<AppExit>,
) {
    let at = std::env::var("WOW_WORLDVIEW_SHOT_AT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20.0);
    match *fired_at {
        None if time.elapsed_secs() >= at => {
            let path = std::env::var("WOW_WORLDVIEW_SHOT").unwrap_or_default();
            commands
                .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
                .observe(bevy::render::view::screenshot::save_to_disk(path));
            *fired_at = Some(time.elapsed_secs());
        }
        Some(t) if time.elapsed_secs() >= t + 2.0 => {
            exit.write(AppExit::Success);
        }
        _ => {}
    }
}

/// The kart's sounds through the demo's own output ([`audio`]; the client mixes through its own).
/// `WOW_KART_MUTE` keeps it quiet.
fn sound(
    view: Res<KartView>,
    mut audio: ResMut<KartAudio>,
    mut sounds: Local<Sounds>,
    mut manager: Local<Option<Option<kira::AudioManager<audio::CpalOut>>>>,
) {
    let manager = manager.get_or_insert_with(|| {
        if std::env::var_os("WOW_KART_MUTE").is_some() {
            return None;
        }
        kira::AudioManager::<audio::CpalOut>::new(kira::AudioManagerSettings::default())
            .map_err(|e| warn!("kart demo: no audio output ({e}); driving silent"))
            .ok()
    });
    let Some(manager) = manager.as_mut() else {
        audio.cues.clear();
        return;
    };
    audio.music = 0.7;
    sounds.update(&mut audio, view.active, 1.0, |data| manager.play(data).ok());
}
