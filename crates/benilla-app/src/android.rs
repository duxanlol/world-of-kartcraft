//! The Android entry: the APK's NativeActivity loads `libbenilla.so` and calls its `android_main`
//! (`crates/benilla-android`), which lands in [`main`]. A phone launches with no environment, so
//! the data folders stand in for the launcher: the install in `WoW/Data` and the disc's output in
//! `kart-data`, in the app's own folder (filled over adb) or in `Internal storage/WorldOfKartcraft`
//! (filled by a player from their PC); the state folder `benilla-config` in the app's own; and
//! `env.txt` in either for any other variable (`WOW_HOST=…`, one per line), the way
//! `kartcraft-launcher` passes them. Without the data a screen says what to copy where.

use std::collections::HashMap;
use std::path::Path;

pub use bevy::android::android_activity::AndroidApp;
use bevy::ecs::system::NonSendMarker;
use bevy::input::gamepad::{
    GamepadAxis, GamepadButton, GamepadConnection, GamepadConnectionEvent,
    RawGamepadAxisChangedEvent, RawGamepadButtonChangedEvent, RawGamepadEvent,
};
use bevy::input::touch::{TouchInput, TouchPhase};
use bevy::input::InputSystems;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use winit::platform::android::gamepad::Event;

use crate::char_select::ClientState;
use crate::glue::dialog::GlueDialog;
use crate::ui_script::{EditBoxFocused, EngineTyping};
use crate::{AppExit, BuildId};

/// Runs the client in the activity `app`.
pub fn main(build: BuildId, app: AndroidApp) -> AppExit {
    logcat::redirect();
    if let Some(dir) = app.external_data_path() {
        environment(&app, &dir);
        update::start(app.clone(), dir);
    }
    let _ = bevy::android::ANDROID_APP.set(app);
    crate::run(build)
}

/// The folder on the phone's shared storage a player copies their own data into
/// (`Internal storage/WorldOfKartcraft`), reached through "All files access".
pub const SHARED: &str = "WorldOfKartcraft";

/// The server the APK was built for (`scripts/android-apk.sh`, from the publish config's
/// `SERVER`), the way the launcher builds its preset in.
const BUILT_SERVER: Option<&str> = option_env!("KARTCRAFT_SERVER");

/// `env.txt`'s variables, then the launcher's paths where it names none: the data from the app's
/// own folder when it holds an install (`adb push`, `scripts/android-apk.sh --data`), else from
/// [`SHARED`], which a player fills from their PC; the state folder always in the app's own.
fn environment(app: &AndroidApp, own: &Path) {
    println!("benilla: the app's folder is {}", own.display());
    // `<storage>/Android/data/<package>/files` → `<storage>`.
    let shared = own.ancestors().nth(4).map(|s| s.join(SHARED));
    for dir in std::iter::once(own).chain(shared.as_deref()) {
        if let Ok(text) = std::fs::read_to_string(dir.join("env.txt")) {
            for (key, value) in text.lines().filter_map(|l| l.trim().split_once('=')) {
                if !key.starts_with('#') && std::env::var_os(key.trim()).is_none() {
                    std::env::set_var(key.trim(), value.trim());
                }
            }
        }
    }
    let data = if own.join("WoW/Data").is_dir() {
        own.to_path_buf()
    } else {
        let shared = shared.unwrap_or_else(|| own.to_path_buf());
        if !storage::all_files_access(app) {
            println!(
                "benilla: asking for all files access, to read {}",
                shared.display()
            );
            let _ = MISSING.set(Missing::Access);
        }
        shared
    };
    if MISSING.get().is_none() && std::fs::File::open(data.join("WoW/Data/dbc.MPQ")).is_err() {
        let _ = MISSING.set(Missing::Install);
    }
    // The kart data: the player's disc converted into the app's own folder, as the launcher does
    // on a PC (again when this build's converter is another than the one that made it); else a
    // `kart-data` copied from a PC; else what an earlier disc made, the disc since removed.
    let converted = own.join("kart-data");
    let kart = match convert::disc(&data) {
        Some(disc) if MISSING.get().is_none() => {
            if convert::needed(&converted) {
                println!(
                    "benilla: converting {} into {}",
                    disc.display(),
                    converted.display()
                );
                let _ = MISSING.set(Missing::Converting);
                convert::start(app.clone(), disc, converted.clone());
            }
            converted
        }
        _ if data.join("kart-data/kart.json").is_file() => data.join("kart-data"),
        _ if converted.join("kart.json").is_file() => converted,
        _ => {
            let _ = MISSING.set(Missing::KartData);
            converted
        }
    };
    for (key, path) in [
        ("WOW_DATA", data.join("WoW/Data")),
        ("WOW_KART_DATA", kart),
        ("BENILLA_HOME", own.join("benilla-config")),
    ] {
        if std::env::var_os(key).is_none() {
            std::env::set_var(key, path);
        }
    }
    if let (None, Some(server)) = (std::env::var_os("WOW_HOST"), BUILT_SERVER) {
        std::env::set_var("WOW_HOST", server);
    }
    // The kart data's relative lookups (`kart-data`, `../kart-data`) and any other relative path.
    let _ = std::env::set_current_dir(own);
}

/// Why the client cannot start on this phone yet: the player's own data is not where it reads it.
#[derive(Debug, Clone, Copy)]
enum Missing {
    /// "All files access" not granted; the settings page that grants it was opened.
    Access,
    /// No readable install in `WoW/Data`.
    Install,
    /// No disc image and no `kart-data`.
    KartData,
    /// The disc is being converted ([`convert`]); the app restarts when it is done.
    Converting,
}

static MISSING: std::sync::OnceLock<Missing> = std::sync::OnceLock::new();

impl Missing {
    /// What the player reads, and does next.
    fn text(self) -> String {
        let copy = format!(
            "World of Kartcraft plays from your own game files. Copy them from your PC to this \
             phone (plug it in over USB and open it in the file explorer):\n\n\
             - your World of Warcraft 1.12 folder's Data folder, to Internal storage/{SHARED}/WoW/Data\n\
             - your Crash Team Racing disc image (the US version, a .bin and .cue, or an .iso), to \
             Internal storage/{SHARED}. The app turns it into the kart, its sounds and the music \
             itself.\n\n\
             Then open the app again."
        );
        match self {
            Missing::Access => format!(
                "Allow \"All files access\" for World of Kartcraft on the page that opened, so it \
                 can read your game files in Internal storage/{SHARED}. Then open the app \
                 again.\n\n{copy}"
            ),
            Missing::Install => format!("No World of Warcraft files found.\n\n{copy}"),
            Missing::KartData => format!("No Crash Team Racing disc image found.\n\n{copy}"),
            Missing::Converting => match convert::status() {
                convert::Status::Running(line) => format!(
                    "Converting your Crash Team Racing disc. This takes a minute or so, once.\n\n\
                     {line}"
                ),
                convert::Status::Done => {
                    "Your kart is ready. World of Kartcraft starts again...".to_string()
                }
                convert::Status::Failed(why) => format!(
                    "Your Crash Team Racing disc could not be converted.\n\n{why}\n\n\
                     Put the US version's disc image in Internal storage/{SHARED} and open the app \
                     again."
                ),
            },
        }
    }
}

/// The missing-data screen's text, following a conversion as it goes.
#[derive(Component)]
struct MissingText;

fn missing_progress(mut texts: Query<&mut Text, With<MissingText>>) {
    let Some(missing @ Missing::Converting) = MISSING.get().copied() else {
        return;
    };
    let line = missing.text();
    for mut text in &mut texts {
        if text.0 != line {
            text.0 = line.clone();
        }
    }
}

/// The missing-data screen over everything else, on its own camera, when [`MISSING`] says why.
fn missing_screen(mut commands: Commands) {
    let Some(missing) = MISSING.get() else {
        return;
    };
    println!("benilla: cannot start yet: {missing:?}");
    let camera = commands
        .spawn((
            Name::new("android missing-data camera"),
            Camera2d,
            // As the debug overlay's: bevy keys the 2D depth texture on the target alone, so a
            // camera on the window must match the player-UI camera's `Msaa::Off` (the default
            // `Sample4` failed validation and aborted the app).
            bevy::render::view::Msaa::Off,
            Camera {
                order: 1000,
                clear_color: ClearColorConfig::Custom(Color::BLACK),
                ..default()
            },
        ))
        .id();
    commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                padding: UiRect::all(Val::Percent(6.0)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::BLACK),
            GlobalZIndex(i32::MAX),
            UiTargetCamera(camera),
        ))
        .with_child((
            MissingText,
            Text::new(missing.text()),
            TextFont {
                // Logical points: a phone's screen is about 400 of them tall in landscape.
                font_size: 15.0,
                ..default()
            },
            TextColor(Color::srgb(1.0, 0.82, 0.0)),
        ));
}

/// The player's Crash Team Racing disc into the app's own `kart-data`, on the phone, by the very
/// converter the PC launcher runs (`ctr_assets::extract`). Kart data is stamped with the converter
/// that made it (`.converter`, as the launcher stamps it), and a build whose converter is another
/// converts the disc again; so the disc image stays on the phone. The game loads its kart data
/// at start, so when it is done the app starts again (`Phoenix.java`).
mod convert {
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use jni::objects::{JClass, JObject, JValue};
    use jni::{jni_sig, jni_str};

    use super::AndroidApp;

    /// This build's converter: the last commit to touch `ctr-assets` (`scripts/android-apk.sh`).
    /// A build without it converts only when there is no kart data.
    const CONVERTER: Option<&str> = option_env!("KARTCRAFT_CONVERTER");
    /// The disc image's extensions, as the PC launcher looks for them (`find.rs`).
    const DISC: [&str; 3] = ["bin", "iso", "img"];
    /// Folders of [`super::SHARED`] that hold no disc.
    const NOT_DISC: [&str; 3] = ["WoW", "kart-data", "benilla-config"];

    #[derive(Debug, Clone)]
    pub enum Status {
        Running(String),
        Done,
        Failed(String),
    }

    static STATUS: Mutex<Option<Status>> = Mutex::new(None);

    pub fn status() -> Status {
        STATUS
            .lock()
            .ok()
            .and_then(|s| s.clone())
            .unwrap_or(Status::Running(String::new()))
    }

    fn set(status: Status) {
        *STATUS.lock().unwrap_or_else(|e| e.into_inner()) = Some(status);
    }

    /// The biggest disc image in `dir` or a folder in it (a disc copied as its own folder).
    pub fn disc(dir: &Path) -> Option<PathBuf> {
        let images = |d: &Path| -> Vec<(u64, PathBuf)> {
            std::fs::read_dir(d)
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|e| {
                    let path = e.path();
                    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
                    let meta = e.metadata().ok()?;
                    (meta.is_file() && DISC.contains(&ext.as_str())).then_some((meta.len(), path))
                })
                .collect()
        };
        let mut found = images(dir);
        for sub in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let name = sub.file_name();
            if sub.path().is_dir() && !NOT_DISC.iter().any(|n| name == *n) {
                found.extend(images(&sub.path()));
            }
        }
        found
            .into_iter()
            .max_by_key(|(size, _)| *size)
            .map(|(_, p)| p)
    }

    /// Whether `out` needs the disc converted: no kart, or another converter's.
    pub fn needed(out: &Path) -> bool {
        if !out.join("kart.json").is_file() {
            return true;
        }
        let stamp = std::fs::read_to_string(out.join(".converter")).unwrap_or_default();
        CONVERTER.is_some_and(|c| stamp.trim() != c)
    }

    /// Converts `disc` into `out` on a thread of its own, then starts the app again.
    pub fn start(app: AndroidApp, disc: PathBuf, out: PathBuf) {
        std::thread::spawn(move || {
            set(Status::Running(String::new()));
            match convert(&disc, &out) {
                Ok(()) => {
                    println!("benilla: converted the disc into {}", out.display());
                    set(Status::Done);
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    if let Err(e) = restart(&app) {
                        println!("benilla: could not start again: {e}");
                        set(Status::Failed(
                            "Done, but the app could not start again by itself: open it again."
                                .into(),
                        ));
                    }
                }
                Err(why) => {
                    println!("benilla: the disc did not convert: {why}");
                    set(Status::Failed(why));
                }
            }
        });
    }

    /// Into a fresh folder beside `out`, swapped in once whole: a conversion that fails keeps the
    /// kart data there was.
    fn convert(disc: &Path, out: &Path) -> Result<(), String> {
        let fresh = out.with_file_name("kart-data.new");
        let old = out.with_file_name("kart-data.old");
        let _ = std::fs::remove_dir_all(&fresh);
        let _ = std::fs::remove_dir_all(&old);
        let parts = ctr_assets::extract::Parts {
            all: true,
            ..Default::default()
        };
        ctr_assets::extract::run(disc, &fresh, "crash", parts, &mut |line| {
            println!("benilla: convert: {line}");
            set(Status::Running(line));
        })?;
        if let Some(c) = CONVERTER {
            std::fs::write(fresh.join(".converter"), c).map_err(|e| e.to_string())?;
        }
        if out.exists() {
            std::fs::rename(out, &old).map_err(|e| e.to_string())?;
        }
        std::fs::rename(&fresh, out).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_dir_all(&old);
        Ok(())
    }

    /// `Phoenix.rebirth(activity)`, through the activity's class loader as `Updater` is reached.
    fn restart(app: &AndroidApp) -> Result<(), String> {
        // SAFETY: the activity's own VM and activity, valid while `app` lives.
        let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) };
        let raw_activity = app.activity_as_ptr() as jni::sys::jobject;
        vm.attach_current_thread(|env| -> jni::errors::Result<()> {
            // SAFETY: the activity's global reference, not owned here.
            let activity = unsafe { JObject::from_raw(env, raw_activity) };
            let loader = env
                .call_method(
                    &activity,
                    jni_str!("getClassLoader"),
                    jni_sig!("()Ljava/lang/ClassLoader;"),
                    &[],
                )?
                .l()?;
            let name = env.new_string("com.worldofkartcraft.game.Phoenix")?;
            let class = env
                .call_method(
                    &loader,
                    jni_str!("loadClass"),
                    jni_sig!("(Ljava/lang/String;)Ljava/lang/Class;"),
                    &[JValue::Object(&name)],
                )?
                .l()?;
            let class = env.cast_local::<JClass>(class)?;
            env.call_static_method(
                &class,
                jni_str!("rebirth"),
                jni_sig!("(Landroid/content/Context;)V"),
                &[JValue::Object(&activity)],
            )?;
            Ok(())
        })
        .map_err(|e| format!("{e:?}"))
    }
}
/// The app's own updates, as the PC launcher updates the game: the update host's `android/`
/// folder (`dist/android-upload.sh`) holds `version.json` and the APK it names. A newer build
/// downloads in the background, is checked against its SHA-256, and goes to Android's installer
/// (`Updater.java`), which asks the player to confirm; the game plays on meanwhile.
mod update {
    use std::io::{Read, Write};
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use jni::objects::{JClass, JObject, JValue};
    use jni::{jni_sig, jni_str};
    use sha2::{Digest, Sha256};

    use super::AndroidApp;

    /// The update host's Android folder and this build's version code, both from
    /// `scripts/android-apk.sh`; a build without them never updates.
    const BASE: Option<&str> = option_env!("KARTCRAFT_UPDATE_URL");
    const CODE: Option<&str> = option_env!("KARTCRAFT_VERSION_CODE");
    /// A staging build's wake: staging sleeps when nobody plays, and the players' site starts it
    /// (`kartcraft-launcher/src/wake.rs` does the same for the testers' PC launcher).
    const WAKE: Option<&str> = option_env!("KARTCRAFT_WAKE_URL");
    /// How long a woken server takes at most, and the login port that says it is up.
    const PATIENCE: std::time::Duration = std::time::Duration::from_secs(300);
    const LOGIN_PORT: u16 = 3724;

    /// Where the server and an update stand, for the banner.
    #[derive(Debug, Clone, PartialEq)]
    pub enum Status {
        None,
        Waking { secs: u64 },
        Downloading { got: u64, size: u64 },
        Confirm,
    }

    pub static STATUS: Mutex<Status> = Mutex::new(Status::None);

    fn set(status: Status) {
        *STATUS.lock().unwrap_or_else(|e| e.into_inner()) = status;
    }

    /// Wakes a sleeping staging server, then looks for an update, on a thread of its own.
    pub fn start(app: AndroidApp, own: PathBuf) {
        std::thread::spawn(move || {
            if let Some(url) = WAKE.filter(|u| !u.is_empty()) {
                wake(url);
            }
            if let Err(e) = run(&app, &own) {
                println!("benilla: update: {e}");
                set(Status::None);
            }
        });
    }

    /// Whether the server's login port answers.
    fn up() -> bool {
        use std::net::ToSocketAddrs;
        let Ok(host) = std::env::var("WOW_HOST") else {
            return true;
        };
        (host.as_str(), LOGIN_PORT)
            .to_socket_addrs()
            .into_iter()
            .flatten()
            .any(|a| {
                std::net::TcpStream::connect_timeout(&a, std::time::Duration::from_secs(3)).is_ok()
            })
    }

    /// Asks the site to start the server (at most once a minute, the site's own limit) and
    /// waits for its login port, the banner counting.
    fn wake(url: &str) {
        let start = std::time::Instant::now();
        let mut asked: Option<std::time::Instant> = None;
        while !up() && start.elapsed() < PATIENCE {
            if asked.is_none_or(|a| a.elapsed().as_secs() >= 60) {
                asked = Some(std::time::Instant::now());
                println!("benilla: the test server is asleep: waking it");
                if let Err(e) = ureq::post(url).call() {
                    println!("benilla: could not wake the test server: {e}");
                    break;
                }
            }
            set(Status::Waking {
                secs: start.elapsed().as_secs(),
            });
            std::thread::sleep(std::time::Duration::from_secs(5));
        }
        set(Status::None);
    }

    fn run(app: &AndroidApp, own: &Path) -> Result<(), String> {
        let base = BASE.filter(|b| !b.is_empty());
        let (Some(base), Some(ours)) = (base, CODE.and_then(|c| c.parse::<u64>().ok())) else {
            return Ok(());
        };
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(std::time::Duration::from_secs(8))
            .timeout_read(std::time::Duration::from_secs(60))
            .build();
        let text = agent
            .get(&format!("{base}/version.json"))
            .call()
            .map_err(|e| e.to_string())?
            .into_string()
            .map_err(|e| e.to_string())?;
        let doc: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let code = doc["versionCode"]
            .as_u64()
            .ok_or("version.json names no versionCode")?;
        if code <= ours {
            println!("benilla: update: build {ours} is current (the host has {code})");
            return Ok(());
        }
        let apk = doc["apk"].as_str().ok_or("version.json names no apk")?;
        let want = doc["sha256"]
            .as_str()
            .ok_or("version.json names no sha256")?;
        println!("benilla: update: build {code} replaces {ours}, downloading {apk}");

        let response = agent
            .get(&format!("{base}/{apk}"))
            .call()
            .map_err(|e| e.to_string())?;
        let size = response
            .header("Content-Length")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let part = own.join("update.apk.part");
        let mut file = std::fs::File::create(&part).map_err(|e| e.to_string())?;
        let mut body = response.into_reader();
        let mut hash = Sha256::new();
        let mut buf = vec![0u8; 1 << 16];
        let mut got = 0u64;
        set(Status::Downloading { got, size });
        loop {
            let n = body.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            hash.update(&buf[..n]);
            got += n as u64;
            set(Status::Downloading { got, size });
        }
        drop(file);
        let have = format!("{:x}", hash.finalize());
        if !have.eq_ignore_ascii_case(want) {
            let _ = std::fs::remove_file(&part);
            return Err(format!(
                "the download's SHA-256 is {have}, version.json says {want}"
            ));
        }
        let ready = own.join("update.apk");
        std::fs::rename(&part, &ready).map_err(|e| e.to_string())?;
        install(app, &ready)?;
        set(Status::Confirm);
        Ok(())
    }

    /// `Updater.install(activity, apk)`. The class is the APK's own, so it is found through the
    /// activity's class loader: JNI's `FindClass` on this thread sees only the system's.
    fn install(app: &AndroidApp, apk: &Path) -> Result<(), String> {
        // SAFETY: the activity's own VM and activity, valid while `app` lives.
        let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) };
        let raw_activity = app.activity_as_ptr() as jni::sys::jobject;
        vm.attach_current_thread(|env| -> jni::errors::Result<()> {
            // SAFETY: the activity's global reference, not owned here.
            let activity = unsafe { JObject::from_raw(env, raw_activity) };
            let loader = env
                .call_method(
                    &activity,
                    jni_str!("getClassLoader"),
                    jni_sig!("()Ljava/lang/ClassLoader;"),
                    &[],
                )?
                .l()?;
            let name = env.new_string("com.worldofkartcraft.game.Updater")?;
            let class = env
                .call_method(
                    &loader,
                    jni_str!("loadClass"),
                    jni_sig!("(Ljava/lang/String;)Ljava/lang/Class;"),
                    &[JValue::Object(&name)],
                )?
                .l()?;
            let class = env.cast_local::<JClass>(class)?;
            let path = env.new_string(apk.to_string_lossy())?;
            env.call_static_method(
                &class,
                jni_str!("install"),
                jni_sig!("(Landroid/content/Context;Ljava/lang/String;)V"),
                &[JValue::Object(&activity), JValue::Object(&path)],
            )?;
            Ok(())
        })
        .map_err(|e| format!("the installer refused the update: {e:?}"))
    }
}

/// The update banner: a line at the top of the screen while an update downloads, and once the
/// installer has it, while it waits for the player to confirm.
fn update_banner(
    mut commands: Commands,
    mut banner: Local<Option<(Entity, Entity, Entity)>>,
    mut texts: Query<&mut Text>,
) {
    let status = update::STATUS
        .lock()
        .map(|s| s.clone())
        .unwrap_or(update::Status::None);
    let line = match status {
        update::Status::None => None,
        update::Status::Waking { secs } => Some(format!(
            "Waking the test server... {}:{:02} (usually about two minutes)",
            secs / 60,
            secs % 60
        )),
        update::Status::Downloading { got, size } if size > 0 => Some(format!(
            "Downloading an update of World of Kartcraft: {}%",
            got * 100 / size
        )),
        update::Status::Downloading { got, .. } => Some(format!(
            "Downloading an update of World of Kartcraft: {} MB",
            got >> 20
        )),
        update::Status::Confirm => {
            Some("The update is ready: confirm it on the installer's screen.".to_string())
        }
    };
    match (line, *banner) {
        (Some(line), Some((_, _, text))) => {
            if let Ok(mut t) = texts.get_mut(text) {
                if t.0 != line {
                    t.0 = line;
                }
            }
        }
        (Some(line), None) => {
            let camera = commands
                .spawn((
                    Name::new("android update-banner camera"),
                    Camera2d,
                    // `Msaa::Off`, as the missing-data screen's camera and for the same reason.
                    bevy::render::view::Msaa::Off,
                    Camera {
                        order: 999,
                        clear_color: ClearColorConfig::None,
                        ..default()
                    },
                ))
                .id();
            let text = commands
                .spawn((
                    Text::new(line),
                    TextFont {
                        font_size: 14.0,
                        ..default()
                    },
                    TextColor(Color::srgb(1.0, 0.82, 0.0)),
                ))
                .id();
            let node = commands
                .spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        top: Val::Px(12.0),
                        width: Val::Percent(100.0),
                        justify_content: JustifyContent::Center,
                        ..default()
                    },
                    GlobalZIndex(i32::MAX - 1),
                    UiTargetCamera(camera),
                ))
                .add_child(text)
                .id();
            *banner = Some((camera, node, text));
        }
        (None, Some((camera, node, _))) => {
            commands.entity(node).despawn();
            commands.entity(camera).despawn();
            *banner = None;
        }
        (None, None) => {}
    }
}
/// Android's "All files access" (`MANAGE_EXTERNAL_STORAGE`), the one way an app with no Java code
/// reads a folder a player copied in from a PC; the system's settings page asks the player.
mod storage {
    use jni::objects::{JObject, JString, JValue};
    use jni::{jni_sig, jni_str};

    use super::AndroidApp;

    /// Whether the app may read shared storage; if not, opens the page that grants it.
    pub fn all_files_access(app: &AndroidApp) -> bool {
        // SAFETY: the activity's own VM and activity, valid while `app` lives.
        let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) };
        let raw_activity = app.activity_as_ptr() as jni::sys::jobject;
        let asked = vm.attach_current_thread(|env| -> jni::errors::Result<bool> {
            let granted = env
                .call_static_method(
                    jni_str!("android/os/Environment"),
                    jni_str!("isExternalStorageManager"),
                    jni_sig!("()Z"),
                    &[],
                )?
                .z()?;
            if granted {
                return Ok(true);
            }
            let action = JString::from_jni_str(
                env,
                jni_str!("android.settings.MANAGE_APP_ALL_FILES_ACCESS_PERMISSION"),
            )?;
            let package =
                JString::from_jni_str(env, jni_str!("package:com.worldofkartcraft.game"))?;
            let uri = env
                .call_static_method(
                    jni_str!("android/net/Uri"),
                    jni_str!("parse"),
                    jni_sig!("(Ljava/lang/String;)Landroid/net/Uri;"),
                    &[JValue::Object(&package)],
                )?
                .l()?;
            let intent = env.new_object(
                jni_str!("android/content/Intent"),
                jni_sig!("(Ljava/lang/String;Landroid/net/Uri;)V"),
                &[JValue::Object(&action), JValue::Object(&uri)],
            )?;
            // SAFETY: the activity's global reference, not owned here.
            let activity = unsafe { JObject::from_raw(env, raw_activity) };
            env.call_method(
                &activity,
                jni_str!("startActivity"),
                jni_sig!("(Landroid/content/Intent;)V"),
                &[JValue::Object(&intent)],
            )?;
            Ok(false)
        });
        asked.unwrap_or_else(|e| {
            println!("benilla: could not ask for all files access: {e:?}");
            false
        })
    }
}

/// What Android adds to the desktop build: the phone's keyboard for an edit box, and its game
/// controllers as Bevy gamepads. gilrs has no Android backend, so the patched
/// winit queues a controller's raw events (`third_party/winit/BENILLA.md`) and this turns them into
/// the raw gamepad events `bevy_gilrs` writes elsewhere: a pad appears with its first event, its
/// hat is the D-pad, and its sticks' Y points up as gilrs's does.
pub struct AndroidPlugin;

impl Plugin for AndroidPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Pads>()
            .add_systems(PreUpdate, read_pads.before(InputSystems))
            .add_systems(Update, soft_keyboard)
            .add_systems(Startup, missing_screen)
            .add_systems(Update, missing_progress)
            .add_systems(Update, update_banner);
    }
}

/// The phone's own keyboard while a text box takes typing, the way a phone app opens it for a text
/// field; winit shows it for a window that allows IME and hides it again. Android keeps it down
/// while a hardware keyboard is attached. The glue screens' boxes are Rust's: the login form
/// always has one focused, character creation its name, and a glue dialog may carry one; in the
/// world, the interface's focused edit box (chat, a search) and the track editor's fields.
fn soft_keyboard(
    (edit_box, engine): (Res<EditBoxFocused>, Res<EngineTyping>),
    state: Res<State<ClientState>>,
    dialog: Res<GlueDialog>,
    mut windows: Query<(Entity, &mut Window), With<PrimaryWindow>>,
    clicks: Res<ButtonInput<MouseButton>>,
    mut touches: MessageReader<TouchInput>,
    _main: NonSendMarker,
) {
    let dialog_edit = dialog.kind.map(|k| k.has_edit_box());
    let typing = match state.get() {
        ClientState::Login | ClientState::CharCreate => dialog_edit != Some(false),
        ClientState::CharSelect => dialog_edit == Some(true),
        ClientState::InWorld => false,
    } || edit_box.0
        || engine.0;
    // Over the missing-data screen nothing takes typing: the login form under it is out of reach.
    let typing = typing && MISSING.get().is_none();
    for (entity, mut window) in &mut windows {
        // Only once winit has the window, which on Android is at the first resume: a window
        // made with IME already allowed has no change for bevy to pass on, so it never opens.
        let made = bevy::winit::WINIT_WINDOWS.with_borrow(|w| w.get_window(entity).is_some());
        if made && window.ime_enabled != typing {
            window.ime_enabled = typing;
        }
        // A tap or click while typing asks again, as tapping a phone app's text field does: the
        // player may have put the keyboard away, and bevy passes on only a change.
        let tapped = clicks.get_just_pressed().next().is_some()
            || touches.read().any(|t| t.phase == TouchPhase::Started);
        if made && typing && tapped {
            bevy::winit::WINIT_WINDOWS.with_borrow(|w| {
                if let Some(w) = w.get_window(entity) {
                    w.set_ime_allowed(true);
                }
            });
        }
    }
}

/// Android's device id for each pad's entity, and the hat each last reported.
#[derive(Resource, Default)]
struct Pads(HashMap<i32, (Entity, [f32; 2])>);

fn read_pads(
    mut commands: Commands,
    mut pads: ResMut<Pads>,
    mut events: MessageWriter<RawGamepadEvent>,
    mut connections: MessageWriter<GamepadConnectionEvent>,
    mut buttons: MessageWriter<RawGamepadButtonChangedEvent>,
    mut axes: MessageWriter<RawGamepadAxisChangedEvent>,
) {
    for event in winit::platform::android::gamepad::take() {
        let (Event::Key { device, .. } | Event::Axis { device, .. }) = event;
        let (pad, hat) = pads.0.entry(device).or_insert_with(|| {
            let pad = commands.spawn_empty().id();
            let connected = GamepadConnectionEvent::new(
                pad,
                GamepadConnection::Connected {
                    name: format!("Android controller {device}"),
                    vendor_id: None,
                    product_id: None,
                },
            );
            events.write(connected.clone().into());
            connections.write(connected);
            (pad, [0.0; 2])
        });
        let mut button = |button, value: f32| {
            events.write(RawGamepadButtonChangedEvent::new(*pad, button, value).into());
            buttons.write(RawGamepadButtonChangedEvent::new(*pad, button, value));
        };
        match event {
            Event::Key {
                keycode, pressed, ..
            } => {
                if let Some(b) = pad_button(keycode) {
                    button(b, if pressed { 1.0 } else { 0.0 });
                }
            }
            Event::Axis { axis, value, .. } => match axis {
                HAT_X | HAT_Y => {
                    let i = usize::from(axis == HAT_Y);
                    let value = value.round();
                    if hat[i] != value {
                        hat[i] = value;
                        let (minus, plus) = if i == 0 {
                            (GamepadButton::DPadLeft, GamepadButton::DPadRight)
                        } else {
                            (GamepadButton::DPadUp, GamepadButton::DPadDown)
                        };
                        button(minus, f32::from(u8::from(value < 0.0)));
                        button(plus, f32::from(u8::from(value > 0.0)));
                    }
                }
                LTRIGGER | BRAKE => button(GamepadButton::LeftTrigger2, value),
                RTRIGGER | GAS => button(GamepadButton::RightTrigger2, value),
                _ => {
                    if let Some((a, sign)) = pad_axis(axis) {
                        events.write(RawGamepadAxisChangedEvent::new(*pad, a, sign * value).into());
                        axes.write(RawGamepadAxisChangedEvent::new(*pad, a, sign * value));
                    }
                }
            },
        }
    }
}

// Android's `AXIS_*` ids for a controller's hat and triggers.
const HAT_X: u32 = 15;
const HAT_Y: u32 = 16;
const LTRIGGER: u32 = 17;
const RTRIGGER: u32 = 18;
const GAS: u32 = 22;
const BRAKE: u32 = 23;

/// A stick axis (`AXIS_X`, `_Y`, `_Z`, `_RZ`: Android's standard layout) and its sign.
fn pad_axis(axis: u32) -> Option<(GamepadAxis, f32)> {
    Some(match axis {
        0 => (GamepadAxis::LeftStickX, 1.0),
        1 => (GamepadAxis::LeftStickY, -1.0),
        11 => (GamepadAxis::RightStickX, 1.0),
        14 => (GamepadAxis::RightStickY, -1.0),
        _ => return None,
    })
}

/// A controller's `KEYCODE_*` as a button: the named ones, then the generic `BUTTON_1`… that a
/// pad with no layout of its own (a USB SNES pad) reports.
fn pad_button(keycode: u32) -> Option<GamepadButton> {
    use GamepadButton::*;
    Some(match keycode {
        96 | 188 => South,
        97 | 189 => East,
        99 | 190 => West,
        100 | 191 => North,
        98 => C,
        101 => Z,
        102 | 192 => LeftTrigger,
        103 | 193 => RightTrigger,
        104 | 194 => LeftTrigger2,
        105 | 195 => RightTrigger2,
        106 => LeftThumb,
        107 => RightThumb,
        // `KEYCODE_BACK`: the select button of many pads.
        109 | 4 | 196 => Select,
        108 | 197 => Start,
        110 => Mode,
        19 => DPadUp,
        20 => DPadDown,
        21 => DPadLeft,
        22 => DPadRight,
        _ => return None,
    })
}

/// stdout and stderr into the system log: an app's own go nowhere on Android, and the client
/// reports through `println!` (`adb logcat -s benilla`).
mod logcat {
    use std::ffi::{c_char, c_int, CString};
    use std::io::{BufRead, BufReader};
    use std::os::fd::FromRawFd;

    #[link(name = "log")]
    extern "C" {
        fn __android_log_write(prio: c_int, tag: *const c_char, text: *const c_char) -> c_int;
    }

    /// `ANDROID_LOG_INFO`.
    const INFO: c_int = 4;

    pub fn redirect() {
        let mut fds = [0; 2];
        // SAFETY: a fresh pipe; its write end replaces fds 1 and 2, its read end moves to the File.
        let read = unsafe {
            if libc::pipe(fds.as_mut_ptr()) != 0 {
                return;
            }
            libc::dup2(fds[1], 1);
            libc::dup2(fds[1], 2);
            std::fs::File::from_raw_fd(fds[0])
        };
        std::thread::spawn(move || {
            let tag = c"benilla";
            let mut lines = BufReader::new(read);
            let mut line = Vec::new();
            while lines.read_until(b'\n', &mut line).is_ok_and(|n| n > 0) {
                line.retain(|&b| b != 0 && b != b'\n');
                if let Ok(text) = CString::new(std::mem::take(&mut line)) {
                    // SAFETY: two NUL-terminated strings that outlive the call.
                    unsafe { __android_log_write(INFO, tag.as_ptr(), text.as_ptr()) };
                }
            }
        });
    }
}
