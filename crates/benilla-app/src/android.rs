//! The Android entry: the APK's NativeActivity loads `libbenilla.so` and calls its `android_main`
//! (`crates/benilla-android`), which lands in [`main`]. A phone launches with no environment, so
//! the app's own folder on shared storage stands in for the launcher: the install in `WoW/Data`,
//! the disc's output in `kart-data`, the state folder in `benilla-config`, and `env.txt` for any
//! other variable (`WOW_HOST=…`, one per line), the way `kartcraft-launcher` passes them.

use std::collections::HashMap;
use std::path::Path;

pub use bevy::android::android_activity::AndroidApp;
use bevy::input::gamepad::{
    GamepadAxis, GamepadButton, GamepadConnection, GamepadConnectionEvent,
    RawGamepadAxisChangedEvent, RawGamepadButtonChangedEvent, RawGamepadEvent,
};
use bevy::input::InputSystems;
use bevy::prelude::*;
use winit::platform::android::gamepad::Event;

use crate::{AppExit, BuildId};

/// Runs the client in the activity `app`.
pub fn main(build: BuildId, app: AndroidApp) -> AppExit {
    logcat::redirect();
    if let Some(dir) = app.external_data_path() {
        environment(&dir);
    }
    let _ = bevy::android::ANDROID_APP.set(app);
    crate::run(build)
}

/// `env.txt`'s variables, then the launcher's three paths where it names none.
fn environment(dir: &Path) {
    println!("benilla: the app's folder is {}", dir.display());
    if let Ok(text) = std::fs::read_to_string(dir.join("env.txt")) {
        for (key, value) in text.lines().filter_map(|l| l.trim().split_once('=')) {
            if !key.starts_with('#') {
                std::env::set_var(key.trim(), value.trim());
            }
        }
    }
    for (key, sub) in [
        ("WOW_DATA", "WoW/Data"),
        ("WOW_KART_DATA", "kart-data"),
        ("BENILLA_HOME", "benilla-config"),
    ] {
        if std::env::var_os(key).is_none() {
            std::env::set_var(key, dir.join(sub));
        }
    }
    // The kart data's relative lookups (`kart-data`, `../kart-data`) and any other relative path.
    let _ = std::env::set_current_dir(dir);
}

/// Android's game controllers as Bevy gamepads. gilrs has no Android backend, so the patched
/// winit queues a controller's raw events (`third_party/winit/BENILLA.md`) and this turns them into
/// the raw gamepad events `bevy_gilrs` writes elsewhere: a pad appears with its first event, its
/// hat is the D-pad, and its sticks' Y points up as gilrs's does.
pub struct AndroidPlugin;

impl Plugin for AndroidPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Pads>()
            .add_systems(PreUpdate, read_pads.before(InputSystems));
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
