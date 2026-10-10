//! `kartcraft-demo`: Crash Team Racing's kart over the WoW world with no server, login or
//! character: benilla's world viewer with its free-fly camera swapped for the kart and its chase
//! camera. It opens at Northshire Abbey, as the viewer does (`WOW_WORLDVIEW_AT=x,y` moves it).
//!
//! Controls: a PS5/PS4 or Xbox pad (✕/A gas, □/X brake, R1/RB hop and powerslide, L1/LB boost, stick
//! steer), or the
//! keyboard (W/↑ gas, S/↓ brake and reverse, A/D or ←/→ steer, Space hop and powerslide, Shift
//! boost). In a powerslide the turbo meter by the speedometer fills; press the other shoulder (Shift) when
//! it turns red for a turbo, up to three per slide. R puts the kart back at the start, K swaps the
//! chase camera for a far one, 1–4 toggle Wumpa Juice, fire a Turbo, toggle Super Engine and
//! toggle Aku Aku.

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
use kartcraft::sound::{KartAudio, Sounds};
use kartcraft::{effects, hud, input, model, Event, KartView, Modifiers, Session, State};

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
    juice: bool,
    engine: bool,
    aku: bool,
    /// The last chain shown: its text, colour and when.
    shown: Option<(String, Color, f32)>,
    best: f32,
    /// `WOW_KART_AUTOPILOT`: a scripted drive for unattended runs.
    autopilot: bool,
}

fn scene(app: &mut App) {
    app.init_resource::<Demo>()
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
        .add_systems(Startup, |mut demo: ResMut<Demo>| {
            demo.autopilot = std::env::var_os("WOW_KART_AUTOPILOT").is_some();
            demo.far_camera = std::env::var("WOW_KART_CAM").is_ok_and(|v| v == "far");
            model::preload();
        })
        .add_systems(
            Update,
            (
                input::log_pads,
                drive,
                model::update,
                model::animate_wheels,
                effects::attach,
                effects::tick,
                effects::draw,
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
) {
    let now = time.elapsed_secs();
    let dt = time.delta_secs();
    let demo = &mut *demo;

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
    }

    let controls = if demo.autopilot {
        input::autopilot(now, view.meter)
    } else {
        input::read(&keys, &pads)
    };
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
    if keys.just_pressed(KeyCode::KeyK) {
        demo.far_camera = !demo.far_camera;
    }
    extras.wumpa = if demo.juice { 10 } else { 0 };
    let mods = Modifiers {
        wumpa: if demo.juice { 10 } else { 0 },
        mask: demo.aku,
        super_engine: demo.engine,
    };
    if fire_turbo {
        if let Some(kart) = demo.session.kart_mut() {
            kart.fire_turbo(demo.juice);
        }
    }
    let events = demo.session.step(
        &collide,
        now,
        dt,
        &kartcraft::kart_input(&controls),
        &mods,
        &[],
    );
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
             Pad (PS5 or Xbox): Cross/A gas, Square/X brake, R1 slide, L1 boost,
             D-pad: up Juice, right Turbo, down Engine, left Aku Aku
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
